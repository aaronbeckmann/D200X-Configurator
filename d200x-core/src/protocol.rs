//! Wire protocol of the Ulanzi D200 family vendor HID interface.
//!
//! Every transfer is a 1024-byte interrupt report on interface 0:
//! `7c 7c | cmd:u16 BE | len:u32 LE | payload...`.
//! Reverse-engineered by the community (strmdck, kastro5/opendeck-ulanzi-d200,
//! the Bitfocus Companion port) and consistent with strings in the
//! on-device app (`libzkgui.so`, hardware rev 02 / SSD210).



pub const VID: u16 = 0x2207;
pub const PID: u16 = 0x0019;
pub const REPORT_LEN: usize = 1024;
pub const HEADER: [u8; 2] = [0x7c, 0x7c];
pub const HEADER_LEN: usize = 8;
pub const PAYLOAD_PER_REPORT: usize = REPORT_LEN - HEADER_LEN;

/// Host -> deck commands (big-endian u16 on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Cmd {
    /// ZIP bundle with `manifest.json` + icons, chunked over many reports.
    /// Manifest keys are `"<col>_<row>"` on a 5x4 grid; `"3_2"` is the wide
    /// slot and carries `SmallViewMode`.
    SetButtons = 0x0001,
    /// Empty payload. Deck answers with `IN_DEVICE_INFO` (0x0303) carrying JSON:
    /// `{"SerialNumber","Dversion","error","DeviceType","HardwareVersion"}`.
    GetDeviceInfo = 0x0003,
    /// Wide "small window": `mode|cpu|ram|HH:MM:SS|gpu|24H|` (mode 0 stats, 1 clock, 2 image).
    SetSmallWindow = 0x0006,
    /// ASCII "0".."100".
    SetBrightness = 0x000a,
    /// Label font/colour JSON (`Align`, `Color`, `FontName`, `ShowTitle`, `Size`, `Weight`).
    SetLabelStyle = 0x000b,
    /// Partial update: ZIP with a manifest holding only the changed keys plus their icons.
    UpdateKeys = 0x000d,
    /// One ASCII digit, sent right after brightness by Ulanzi Studio ("0" observed).
    /// Probably the online sleep minutes setting.
    SetSleep = 0x001a,
    /// Key state feedback after a press: ZIP with manifest
    /// `{"<col>_<row>_<state>":{"KeyType":1,"Progress":100,"State":1}}`.
    SetKeyState = 0x00e0,
    /// deckd (custom firmware) only: config JSON in, reply 0x0300 JSON.
    DeckdSetConfig = 0x0200,
    /// deckd only: `name\0` + PNG bytes, reply 0x0300.
    DeckdSetIcon = 0x0201,
    /// deckd only: reply 0x0302 config JSON.
    DeckdGetConfig = 0x0202,
    /// deckd only: reboot the deck.
    DeckdReboot = 0x0203,
    /// deckd only: reply 0x0304 with the tail of the device log.
    DeckdGetLog = 0x0204,
    /// deckd only: calibration pattern, payload "1" or "0", reply 0x0300.
    DeckdCalibrate = 0x0205,
    /// agent -> deckd heartbeat JSON `{"agent":..,"plugins":[..]}` every 2 s.
    AgentHeartbeat = 0x0210,
    /// agent -> deckd partial state map JSON `{"ha:light.x":"on", ...}`.
    AgentStates = 0x0211,
    /// agent -> deckd result of a host action `{"id":n,"ok":bool,"error":..}`.
    AgentResult = 0x0212,
    /// deckd forwards to the ESP32 bridge on the console UART: `{"req":n,"config":{..}}`.
    /// Reply 0x0321 `{"req":n,"ok":bool,"error":..}`.
    BridgeConfig = 0x0220,
    /// deckd forwards to the ESP32 bridge: `{"req":n}`. Reply 0x0323 with status JSON.
    BridgeStatus = 0x0222,
    /// deckd forwards to the ESP32 bridge: `{"req":n,"cmd":"ota","url":..}` or `{"cmd":"reboot"}`.
    /// Reply 0x0325 `{"req":n,"ok":bool,"error":..}`.
    BridgeCommand = 0x0224,
}

/// deckd -> host: reply of the ESP32 bridge to `Cmd::BridgeCommand`.
pub const IN_BRIDGE_COMMAND: u16 = 0x0325;

/// deckd -> host: reply of the ESP32 bridge to `Cmd::BridgeConfig`.
pub const IN_BRIDGE_CONFIG: u16 = 0x0321;
/// deckd -> host: reply of the ESP32 bridge to `Cmd::BridgeStatus`.
pub const IN_BRIDGE_STATUS: u16 = 0x0323;

/// deckd -> agent: a host action request (JSON).
pub const EV_HOST_ACTION: u16 = 0x0310;
/// deckd -> agent: hello `{"firmware","serial","states":[..],"icons":[..]}`.
pub const EV_HELLO: u16 = 0x0311;

/// Deck -> host input report commands.
pub const IN_BUTTON_A: u16 = 0x0101;
pub const IN_BUTTON_B: u16 = 0x0102;
/// Reply to `Cmd::GetDeviceInfo`; payload is NUL-padded JSON.
pub const IN_DEVICE_INFO: u16 = 0x0303;

/// Hardware input indices as reported by the deck.
pub mod index {
    pub const WIDE_KEY: u8 = 13;
    pub const SIDE_BUTTON_1: u8 = 15;
    pub const SIDE_BUTTON_2: u8 = 16;
    pub const DIAL_1: u8 = 17;
    pub const DIAL_2: u8 = 18;
    pub const DIAL_3: u8 = 19;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialEvent {
    Release,
    Press,
    Left,
    Right,
}

impl DialEvent {
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Release,
            1 => Self::Press,
            2 => Self::Left,
            3 => Self::Right,
            _ => return None,
        })
    }
}

/// Build the sequence of 1024-byte reports for one command + payload.
/// The first report carries the header and the first chunk; later reports
/// carry raw continuation bytes (the deck reassembles by `len`).
pub fn encode(cmd: Cmd, payload: &[u8]) -> Vec<[u8; REPORT_LEN]> {
    let mut reports = Vec::new();
    let mut first = [0u8; REPORT_LEN];
    first[0..2].copy_from_slice(&HEADER);
    first[2..4].copy_from_slice(&(cmd as u16).to_be_bytes());
    first[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    let n = payload.len().min(PAYLOAD_PER_REPORT);
    first[HEADER_LEN..HEADER_LEN + n].copy_from_slice(&payload[..n]);
    reports.push(first);
    for chunk in payload[n..].chunks(REPORT_LEN) {
        let mut r = [0u8; REPORT_LEN];
        r[..chunk.len()].copy_from_slice(chunk);
        reports.push(r);
    }
    reports
}

/// Firmware bug: a ZIP byte of 0x00 or 0x7c landing at offset
/// `1016 + 1024*N` (first byte of every continuation report) is
/// mis-parsed as a new header. Returns the first offending offset.
pub fn find_bad_boundary_byte(zip: &[u8]) -> Option<usize> {
    (0..)
        .map(|n| PAYLOAD_PER_REPORT + REPORT_LEN * n)
        .take_while(|&i| i < zip.len())
        .find(|&i| zip[i] == 0x00 || zip[i] == 0x7c)
}

/// A decoded input report from the deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputReport {
    pub cmd: u16,
    pub payload: Vec<u8>,
}

pub fn decode_input(report: &[u8]) -> Option<InputReport> {
    if report.len() < HEADER_LEN || report[0..2] != HEADER {
        return None;
    }
    let cmd = u16::from_be_bytes([report[2], report[3]]);
    let len = u32::from_le_bytes([report[4], report[5], report[6], report[7]]) as usize;
    let end = (HEADER_LEN + len).min(report.len());
    Some(InputReport { cmd, payload: report[HEADER_LEN..end].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_brightness() {
        let r = encode(Cmd::SetBrightness, b"50");
        assert_eq!(r.len(), 1);
        assert_eq!(&r[0][..8], &[0x7c, 0x7c, 0x00, 0x0a, 2, 0, 0, 0]);
        assert_eq!(&r[0][8..10], b"50");
    }

    #[test]
    fn splits_large_payload() {
        let payload = vec![1u8; PAYLOAD_PER_REPORT + REPORT_LEN + 5];
        let r = encode(Cmd::SetButtons, &payload);
        assert_eq!(r.len(), 3);
        assert_eq!(r[2][4], 1);
        assert_eq!(r[2][5], 0);
    }

    #[test]
    fn boundary_check() {
        let mut z = vec![1u8; 3000];
        assert_eq!(find_bad_boundary_byte(&z), None);
        z[1016 + 1024] = 0x7c;
        assert_eq!(find_bad_boundary_byte(&z), Some(2040));
    }

    #[test]
    fn decodes_input() {
        let mut r = [0u8; 1024];
        r[..8].copy_from_slice(&[0x7c, 0x7c, 0x01, 0x01, 2, 0, 0, 0]);
        r[8] = 17;
        r[9] = 3;
        let d = decode_input(&r).unwrap();
        assert_eq!(d.cmd, IN_BUTTON_A);
        assert_eq!(d.payload, vec![17, 3]);
    }
}
