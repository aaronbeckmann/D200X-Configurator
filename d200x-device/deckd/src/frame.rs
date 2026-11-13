//! Agent frames on a byte stream (the console UART to the ESP32 bridge):
//! `7c 7c | cmd u16 BE | len u32 LE | payload | crc u16 BE`, CRC-16/CCITT-FALSE over cmd..payload.
//! The header is the vendor HID channel's, so command numbers are shared. Bytes outside frames
//! (boot loader text, noise) are skipped. Spec: esp32-bridge/docs/INTERFACE.md section 2.
//!
//! No libc here, so the module tests on the PC: `rustc --test src/frame.rs`.

use std::time::{Duration, Instant};

pub const MAX_PAYLOAD: usize = 16384;
const HEADER: usize = 8;
/// A started frame whose bytes stop arriving for this long is given up.
const STALL: Duration = Duration::from_millis(1000);

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

pub fn encode(cmd: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(HEADER + payload.len() + 2);
    f.extend_from_slice(&[0x7c, 0x7c]);
    f.extend_from_slice(&cmd.to_be_bytes());
    f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    f.extend_from_slice(payload);
    let crc = crc16(&f[2..]);
    f.extend_from_slice(&crc.to_be_bytes());
    f
}

/// Does `b` start with a complete frame whose CRC matches?
fn complete_frame_at(b: &[u8]) -> bool {
    if b.len() < HEADER + 2 || b[0] != 0x7c || b[1] != 0x7c {
        return false;
    }
    let len = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
    len <= MAX_PAYLOAD && b.len() >= HEADER + len + 2 && crc16(&b[2..HEADER + len]) == u16::from_be_bytes([b[HEADER + len], b[HEADER + len + 1]])
}

#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    last: Option<Instant>,
    /// Frames dropped for a bad CRC or length (for the log).
    pub errors: u64,
}

impl Parser {
    /// Feed received bytes; returns the complete frames as (cmd, payload).
    pub fn push(&mut self, data: &[u8], now: Instant) -> Vec<(u16, Vec<u8>)> {
        if let Some(t) = self.last {
            if !self.buf.is_empty() && now.duration_since(t) > STALL {
                // a frame that never completed: skip its first byte and look again
                self.buf.drain(..1);
                self.errors += 1;
            }
        }
        if !data.is_empty() {
            self.last = Some(now);
        }
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            match self.buf.windows(2).position(|w| w == [0x7c, 0x7c]) {
                Some(i) => {
                    self.buf.drain(..i);
                }
                None => {
                    // keep a trailing 7c: it may start the next frame
                    let keep = self.buf.last() == Some(&0x7c);
                    self.buf.clear();
                    if keep {
                        self.buf.push(0x7c);
                    }
                    break;
                }
            }
            if self.buf.len() < HEADER {
                break;
            }
            let len = u32::from_le_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]) as usize;
            if len > MAX_PAYLOAD {
                self.buf.drain(..1);
                self.errors += 1;
                continue;
            }
            let total = HEADER + len + 2;
            if self.buf.len() < total {
                // A false start (boot text "|||" is 7c 7c 7c) may announce a plausible length
                // and hide a real frame behind it: a complete valid frame further on wins.
                match (1..self.buf.len()).find(|&j| complete_frame_at(&self.buf[j..])) {
                    Some(j) => {
                        self.buf.drain(..j);
                        self.errors += 1;
                        continue;
                    }
                    None => break,
                }
            }
            let crc = u16::from_be_bytes([self.buf[HEADER + len], self.buf[HEADER + len + 1]]);
            if crc16(&self.buf[2..HEADER + len]) != crc {
                self.buf.drain(..1);
                self.errors += 1;
                continue;
            }
            let cmd = u16::from_be_bytes([self.buf[2], self.buf[3]]);
            out.push((cmd, self.buf[HEADER..HEADER + len].to_vec()));
            self.buf.drain(..total);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
    }

    #[test]
    fn round_trip() {
        let mut p = Parser::default();
        let f = encode(0x0210, b"{}");
        assert_eq!(&f[..8], &[0x7c, 0x7c, 0x02, 0x10, 0x02, 0, 0, 0]);
        assert_eq!(p.push(&f, Instant::now()), vec![(0x0210, b"{}".to_vec())]);
    }

    #[test]
    fn byte_by_byte_with_boot_text_around() {
        let mut p = Parser::default();
        let mut stream = b"IPL 0x1234\r\nU-Boot 2015.01 |||\r\n".to_vec();
        stream.extend(encode(0x0211, br#"{"ha:x":"on"}"#));
        stream.extend(b"\x7cnoise\x7c");
        stream.extend(encode(0x0212, br#"{"id":1,"ok":true}"#));
        let now = Instant::now();
        let mut got = Vec::new();
        for b in stream {
            got.extend(p.push(&[b], now));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, 0x0211);
        assert_eq!(got[1], (0x0212, br#"{"id":1,"ok":true}"#.to_vec()));
    }

    #[test]
    fn bad_crc_is_dropped_and_the_next_frame_survives() {
        let mut p = Parser::default();
        let mut bad = encode(0x0210, b"{\"agent\":\"x\"}");
        let n = bad.len();
        bad[n - 1] ^= 0xff;
        let mut stream = bad;
        stream.extend(encode(0x0210, b"{}"));
        assert_eq!(p.push(&stream, Instant::now()), vec![(0x0210, b"{}".to_vec())]);
        assert_eq!(p.errors, 1);
    }

    #[test]
    fn oversize_length_resyncs() {
        let mut p = Parser::default();
        let mut stream = vec![0x7c, 0x7c, 0x02, 0x10, 0xff, 0xff, 0xff, 0x7f];
        stream.extend(encode(0x0213, b"{}"));
        assert_eq!(p.push(&stream, Instant::now()), vec![(0x0213, b"{}".to_vec())]);
    }

    #[test]
    fn a_stalled_false_start_does_not_block_later_frames() {
        let mut p = Parser::default();
        let t0 = Instant::now();
        // a fake header announcing 100 bytes that never come
        assert!(p.push(&[0x7c, 0x7c, 0x02, 0x10, 100, 0, 0, 0], t0).is_empty());
        let later = t0 + Duration::from_millis(1500);
        assert_eq!(p.push(&encode(0x0210, b"{}"), later), vec![(0x0210, b"{}".to_vec())]);
    }

    #[test]
    fn payload_containing_the_sof_bytes() {
        let mut p = Parser::default();
        let payload = b"a\x7c\x7cb".to_vec();
        assert_eq!(p.push(&encode(0x0211, &payload), Instant::now()), vec![(0x0211, payload)]);
    }
}
