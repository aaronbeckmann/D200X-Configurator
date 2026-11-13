//! MCU link on /dev/ttyS1 (115200): keys, side buttons, knobs.
//! Frame: FF 55 | cmd u16 LE | len u8 | payload | sum u16 BE.

use std::fs::File;
use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// MCU firmware version string (reply to `query_version`).
    Version(String),
    /// Any other frame, for logging.
    Other { cmd: u16, payload: Vec<u8> },
    /// Key index 0..13 (13 = wide key) pressed / released.
    Key { index: u8, pressed: bool },
    /// Side button 0 or 1.
    Side { index: u8, pressed: bool },
    /// Knob 0..2 press / release.
    KnobPress { knob: u8, pressed: bool },
    /// Knob 0..2 turned one detent; dir +1 or -1.
    KnobTurn { knob: u8, dir: i8 },
}

pub struct Mcu {
    port: File,
    buf: Vec<u8>,
    matrix: u16,
    discrete: u8,
    acc: [i32; 3],
}

impl Mcu {
    pub fn open() -> Result<Mcu, String> {
        let port = std::fs::OpenOptions::new().read(true).write(true).open("/dev/ttyS1").map_err(|e| e.to_string())?;
        let fd = port.as_raw_fd();
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            libc::tcgetattr(fd, &mut t);
            libc::cfmakeraw(&mut t);
            t.c_cflag |= libc::CLOCAL | libc::CREAD;
            t.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CRTSCTS);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            libc::cfsetispeed(&mut t, libc::B115200);
            libc::cfsetospeed(&mut t, libc::B115200);
            libc::tcsetattr(fd, libc::TCSANOW, &t);
            libc::tcflush(fd, libc::TCIOFLUSH);
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        Ok(Mcu { port, buf: Vec::new(), matrix: 0, discrete: 0, acc: [0; 3] })
    }

    pub fn fd(&self) -> RawFd {
        self.port.as_raw_fd()
    }

    /// Send a frame: FF 55 | cmd u16 LE | len | payload | sum16 BE.
    pub fn send(&mut self, cmd: u16, payload: &[u8]) {
        use std::io::Write;
        let mut f = vec![0xff, 0x55, (cmd & 0xff) as u8, (cmd >> 8) as u8, payload.len() as u8];
        f.extend_from_slice(payload);
        let sum: u16 = f.iter().map(|&b| b as u16).fold(0u16, |a, b| a.wrapping_add(b));
        f.push((sum >> 8) as u8);
        f.push((sum & 0xff) as u8);
        let _ = self.port.write_all(&f);
    }

    /// Ask the MCU to report the current key state (vendor `queryKeyStatus`, frame FF 55 01 00 00 01 55).
    pub fn query_key_status(&mut self) {
        self.send(0x0001, &[]);
    }

    /// Ask for the MCU firmware version (vendor `queryMcuVersion`, frame FF 55 10 00 00 01 64).
    /// The reply arrives as cmd 0x0010 with an ASCII payload and is logged by `poll`.
    pub fn query_version(&mut self) {
        self.send(0x0010, &[]);
    }

    /// Read whatever is available and return decoded events.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut tmp = [0u8; 512];
        while let Ok(n) = self.port.read(&mut tmp) {
            if n == 0 {
                break;
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
        let mut events = Vec::new();
        loop {
            if self.buf.len() < 7 {
                break;
            }
            if self.buf[0] != 0xff || self.buf[1] != 0x55 {
                self.buf.remove(0);
                continue;
            }
            let cmd = self.buf[2] as u16 | ((self.buf[3] as u16) << 8);
            let len = self.buf[4] as usize;
            let total = 5 + len + 2;
            if len > 32 {
                self.buf.drain(..2);
                continue;
            }
            if self.buf.len() < total {
                break;
            }
            let sum: u16 = self.buf[..total - 2].iter().map(|&b| b as u16).fold(0u16, |a, b| a.wrapping_add(b));
            let cs = ((self.buf[total - 2] as u16) << 8) | self.buf[total - 1] as u16;
            if sum != cs {
                self.buf.drain(..2);
                continue;
            }
            let payload: Vec<u8> = self.buf[5..5 + len].to_vec();
            self.buf.drain(..total);
            match (cmd, len) {
                (0x0001, 3) => {
                    let d = payload[0];
                    let m = ((payload[1] as u16) << 8) | payload[2] as u16;
                    for i in 0..14u8 {
                        let bit = if i == 13 { 1 } else { 0x4000 >> i };
                        let was = self.matrix & bit != 0;
                        let now = m & bit != 0;
                        if was != now {
                            events.push(Event::Key { index: i, pressed: now });
                        }
                    }
                    for (bit, ev) in [(0x01u8, 0u8), (0x02, 1)] {
                        let was = self.discrete & bit != 0;
                        let now = d & bit != 0;
                        if was != now {
                            events.push(Event::Side { index: ev, pressed: now });
                        }
                    }
                    for k in 0..3u8 {
                        let bit = 0x04 << k;
                        let was = self.discrete & bit != 0;
                        let now = d & bit != 0;
                        if was != now {
                            events.push(Event::KnobPress { knob: k, pressed: now });
                        }
                    }
                    self.matrix = m;
                    self.discrete = d;
                }
                (0x0002..=0x0004, 1) => {
                    let k = (cmd - 2) as usize;
                    let dir = match payload[0] {
                        0x01 | 0x07 | 0x0e | 0x08 => 1,
                        0x0d | 0x04 | 0x02 | 0x0b => -1,
                        _ => 0,
                    };
                    self.acc[k] += dir;
                    if self.acc[k] >= 4 {
                        self.acc[k] = 0;
                        events.push(Event::KnobTurn { knob: k as u8, dir: 1 });
                    } else if self.acc[k] <= -4 {
                        self.acc[k] = 0;
                        events.push(Event::KnobTurn { knob: k as u8, dir: -1 });
                    }
                }
                (0x0010, _) => {
                    let end = payload.iter().position(|&b| b == 0).unwrap_or(payload.len());
                    events.push(Event::Version(String::from_utf8_lossy(&payload[..end]).into_owned()));
                }
                _ => events.push(Event::Other { cmd, payload }),
            }
        }
        events
    }
}
