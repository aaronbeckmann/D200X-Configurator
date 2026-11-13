//! HID transport for the deck's vendor interface (interface 0).

use crate::protocol::*;
use anyhow::{anyhow, Context, Result};
use hidapi::{HidApi, HidDevice};

pub struct Deck {
    dev: HidDevice,
    pub serial: Option<String>,
    pub product: Option<String>,
    pub path: String,
}

impl Deck {
    /// Open the vendor HID interface. On Windows interface 1 is the boot
    /// keyboard and is locked by the OS; we want interface 0.
    pub fn open() -> Result<Self> {
        let api = HidApi::new().context("hidapi init")?;
        let info = api
            .device_list()
            .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
            .find(|d| d.interface_number() == 0)
            .ok_or_else(|| anyhow!("no Ulanzi deck (2207:0019 interface 0) found"))?;
        let dev = info
            .open_device(&api)
            .context("open HID interface 0 (close Ulanzi Studio first)")?;
        Ok(Self {
            serial: info.serial_number().map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            path: info.path().to_string_lossy().into_owned(),
            dev,
        })
    }

    /// Report descriptors of every interface of the deck, as hex, reconstructed
    /// by hidapi from Windows' preparsed data.
    pub fn descriptors() -> Result<Vec<(i32, String)>> {
        let api = HidApi::new()?;
        let mut out = Vec::new();
        for info in api.device_list().filter(|d| d.vendor_id() == VID && d.product_id() == PID) {
            let dev = match info.open_device(&api) {
                Ok(d) => d,
                Err(e) => {
                    out.push((info.interface_number(), format!("open failed: {e}")));
                    continue;
                }
            };
            let mut buf = vec![0u8; 4096];
            match dev.get_report_descriptor(&mut buf) {
                Ok(n) => out.push((
                    info.interface_number(),
                    buf[..n].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
                )),
                Err(e) => out.push((info.interface_number(), format!("descriptor failed: {e}"))),
            }
        }
        Ok(out)
    }

    pub fn list() -> Result<Vec<String>> {
        let api = HidApi::new()?;
        Ok(api
            .device_list()
            .filter(|d| d.vendor_id() == VID && d.product_id() == PID)
            .map(|d| {
                format!(
                    "if{} usage_page=0x{:04x} usage=0x{:04x} product={:?} serial={:?} path={}",
                    d.interface_number(),
                    d.usage_page(),
                    d.usage(),
                    d.product_string().unwrap_or(""),
                    d.serial_number().unwrap_or(""),
                    d.path().to_string_lossy()
                )
            })
            .collect())
    }

    /// Write one 1024-byte report. hidapi expects a leading report-ID byte
    /// (0x00 here, the deck uses unnumbered reports), so the buffer is 1025 bytes.
    fn write_report(&self, report: &[u8; REPORT_LEN]) -> Result<()> {
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(report);
        let n = self.dev.write(&buf).context("hid write")?;
        if n != buf.len() && n != REPORT_LEN {
            return Err(anyhow!("short write: {n}"));
        }
        Ok(())
    }

    pub fn send(&self, cmd: Cmd, payload: &[u8]) -> Result<usize> {
        let reports = encode(cmd, payload);
        for r in &reports {
            self.write_report(r)?;
        }
        Ok(reports.len())
    }

    pub fn set_brightness(&self, percent: u8) -> Result<()> {
        let p = percent.min(100).to_string();
        self.send(Cmd::SetBrightness, p.as_bytes()).map(|_| ())
    }

    /// Ask for serial, firmware and hardware version. Returns the raw JSON string.
    pub fn get_device_info(&self) -> Result<String> {
        self.send(Cmd::GetDeviceInfo, &[])?;
        for _ in 0..20 {
            if let Some(r) = self.read_input(500)? {
                if r.cmd == IN_DEVICE_INFO {
                    let end = r.payload.iter().position(|&b| b == 0).unwrap_or(r.payload.len());
                    return Ok(String::from_utf8_lossy(&r.payload[..end]).into_owned());
                }
            }
        }
        Err(anyhow!("no device-info reply"))
    }

    /// Read one complete message (reassembling continuation reports) with an
    /// overall timeout. Input events (0x0101/0x0102) are skipped.
    pub fn read_message(&self, timeout_ms: u64) -> Result<Option<InputReport>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let mut buf = [0u8; REPORT_LEN + 1];
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as i32;
            if left <= 0 {
                return Ok(None);
            }
            let n = self.dev.read_timeout(&mut buf, left)?;
            if n == 0 {
                continue;
            }
            let Some(head) = decode_input(&buf[..n]) else { continue };
            if head.cmd == IN_BUTTON_A || head.cmd == IN_BUTTON_B {
                continue;
            }
            let need = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
            let mut payload = head.payload;
            while payload.len() < need {
                let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as i32;
                if left <= 0 {
                    return Ok(None);
                }
                let n = self.dev.read_timeout(&mut buf, left)?;
                if n == 0 {
                    continue;
                }
                let take = (need - payload.len()).min(n);
                payload.extend_from_slice(&buf[..take]);
            }
            payload.truncate(need);
            return Ok(Some(InputReport { cmd: head.cmd, payload }));
        }
    }

    /// deckd: push a config JSON, returns the reply JSON.
    pub fn set_config(&self, json: &[u8]) -> Result<String> {
        self.send(Cmd::DeckdSetConfig, json)?;
        self.expect_reply(0x0300, 5000)
    }

    pub fn get_config(&self) -> Result<String> {
        self.send(Cmd::DeckdGetConfig, &[])?;
        self.expect_reply(0x0302, 5000)
    }

    pub fn set_icon(&self, name: &str, png: &[u8]) -> Result<String> {
        let mut payload = name.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(png);
        self.send(Cmd::DeckdSetIcon, &payload)?;
        self.expect_reply(0x0300, 5000)
    }

    pub fn calibrate(&self, on: bool) -> Result<String> {
        self.send(Cmd::DeckdCalibrate, if on { b"1" } else { b"0" })?;
        self.expect_reply(0x0300, 3000)
    }

    pub fn get_log(&self) -> Result<String> {
        self.send(Cmd::DeckdGetLog, &[])?;
        self.expect_reply(0x0304, 8000)
    }

    pub fn reboot(&self) -> Result<()> {
        self.send(Cmd::DeckdReboot, &[]).map(|_| ())
    }

    /// ESP32 bridge (through deckd): store a config (Wi-Fi, connections, polls). The deck only
    /// relays it; the bridge keeps it in its own flash. Returns the reply JSON.
    pub fn bridge_config(&self, config_json: &str) -> Result<String> {
        let payload = format!("{{\"req\":1,\"config\":{config_json}}}");
        self.send(Cmd::BridgeConfig, payload.as_bytes())?;
        self.expect_reply(crate::protocol::IN_BRIDGE_CONFIG, 10000)
    }

    /// ESP32 bridge (through deckd): status JSON (Wi-Fi, plugins, memory, redacted config).
    pub fn bridge_status(&self) -> Result<String> {
        self.send(Cmd::BridgeStatus, b"{\"req\":2}")?;
        self.expect_reply(crate::protocol::IN_BRIDGE_STATUS, 5000)
    }

    /// ESP32 bridge (through deckd): a command object such as `{"cmd":"ota","url":"http://.."}`
    /// or `{"cmd":"reboot"}`. Returns the reply JSON.
    pub fn bridge_command(&self, command_json: &str) -> Result<String> {
        let body = command_json.trim().trim_start_matches('{');
        let payload = format!("{{\"req\":3,{body}");
        self.send(Cmd::BridgeCommand, payload.as_bytes())?;
        self.expect_reply(crate::protocol::IN_BRIDGE_COMMAND, 5000)
    }

    fn expect_reply(&self, cmd: u16, timeout_ms: u64) -> Result<String> {
        loop {
            match self.read_message(timeout_ms)? {
                Some(m) if m.cmd == cmd => {
                    let end = m.payload.iter().position(|&b| b == 0).unwrap_or(m.payload.len());
                    return Ok(String::from_utf8_lossy(&m.payload[..end]).into_owned());
                }
                Some(_) => continue,
                None => return Err(anyhow!("timeout waiting for reply 0x{cmd:04x}")),
            }
        }
    }

    // ---- agent side -------------------------------------------------------

    pub fn send_heartbeat(&self, agent: &str, plugins: &[&str]) -> Result<()> {
        let json = format!(
            "{{\"agent\":\"{}\",\"plugins\":[{}]}}",
            agent,
            plugins.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(",")
        );
        self.send(Cmd::AgentHeartbeat, json.as_bytes()).map(|_| ())
    }

    pub fn send_states_json(&self, json: &str) -> Result<()> {
        self.send(Cmd::AgentStates, json.as_bytes()).map(|_| ())
    }

    pub fn send_result(&self, id: u64, ok: bool, error: &str) -> Result<()> {
        let json = format!("{{\"id\":{id},\"ok\":{ok},\"error\":\"{}\"}}", error.replace('"', "'"));
        self.send(Cmd::AgentResult, json.as_bytes()).map(|_| ())
    }

    /// Read the next complete message of any kind (events included), or None on timeout.
    pub fn read_any(&self, timeout_ms: u64) -> Result<Option<InputReport>> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let mut buf = [0u8; REPORT_LEN + 1];
        let left = timeout_ms as i32;
        let n = self.dev.read_timeout(&mut buf, left)?;
        if n == 0 {
            return Ok(None);
        }
        let Some(head) = decode_input(&buf[..n]) else { return Ok(None) };
        let need = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let mut payload = head.payload;
        while payload.len() < need {
            let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis().max(50) as i32;
            let n = self.dev.read_timeout(&mut buf, left)?;
            if n == 0 {
                break;
            }
            let take = (need - payload.len()).min(n);
            payload.extend_from_slice(&buf[..take]);
        }
        payload.truncate(need);
        Ok(Some(InputReport { cmd: head.cmd, payload }))
    }

    /// Blocking read with timeout; returns None on timeout.
    pub fn read_input(&self, timeout_ms: i32) -> Result<Option<InputReport>> {
        let mut buf = [0u8; REPORT_LEN + 1];
        let n = self.dev.read_timeout(&mut buf, timeout_ms).context("hid read")?;
        if n == 0 {
            return Ok(None);
        }
        Ok(decode_input(&buf[..n]))
    }
}
