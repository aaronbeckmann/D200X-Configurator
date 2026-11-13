//! ESP32 bridge (the optional hardware mod on the deck's console UART), reached through deckd.
//! Shared by the CLI and the configurator.

use crate::Deck;
use anyhow::{anyhow, bail, Result};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, UdpSocket};
use std::time::{Duration, Instant};

/// The bridge's status as JSON, or an error when no bridge answers (mod not installed, deckd
/// without the link, bridge not powered).
pub fn status(deck: &Deck) -> Result<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&deck.bridge_status()?)?;
    if v.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        bail!("{}", v.get("error").and_then(|e| e.as_str()).unwrap_or("bridge not reachable"));
    }
    Ok(v)
}

/// Everything the bridge's plugins know right now (not only what the deck shows), and its
/// enabled plugins. For the configurator's state explorer.
pub fn states(deck: &Deck) -> Result<(Vec<String>, std::collections::BTreeMap<String, String>)> {
    let v: serde_json::Value = serde_json::from_str(&deck.bridge_command(r#"{"cmd":"states"}"#)?)?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        let e = v.get("error").and_then(|e| e.as_str()).unwrap_or("bridge not reachable");
        if e.contains("unknown command") {
            bail!("the bridge firmware is too old to list its states (update it in the ESP32 bridge window)");
        }
        bail!("{e}");
    }
    let plugins = v.get("plugins").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let states = v.get("states").and_then(|s| s.as_object()).map(|o| o.iter().map(|(k, x)| (k.clone(), x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()))).collect()).unwrap_or_default();
    Ok((plugins, states))
}

/// The local address this PC uses to reach `ip` (for URLs the bridge must call back).
pub fn local_ip_towards(ip: IpAddr) -> Result<IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0")?;
    probe.connect((ip, 80))?;
    Ok(probe.local_addr()?.ip())
}

/// Install firmware over Wi-Fi: serve `image` over HTTP on the interface that reaches the
/// bridge, ask the bridge (through deckd) to fetch it, wait until it runs again. Returns the
/// new version. `log` receives progress lines.
pub fn ota(deck: &Deck, image: &[u8], log: &mut dyn FnMut(String)) -> Result<String> {
    if image.first() != Some(&0xe9) {
        bail!("not an ESP32 app image (no 0xE9 magic byte)");
    }
    let st = status(deck)?;
    let old = st.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let esp_ip: IpAddr = st.pointer("/wifi/ip").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("the bridge has no Wi-Fi connection"))?.parse()?;
    let local = local_ip_towards(esp_ip)?;
    let listener = TcpListener::bind((local, 0))?;
    let url = format!("http://{}:{}/esp32-bridge.bin", local, listener.local_addr()?.port());
    listener.set_nonblocking(true)?;
    log(format!("bridge {old} at {esp_ip}; serving {} bytes at {url}", image.len()));

    let reply = deck.bridge_command(&serde_json::json!({ "cmd": "ota", "url": url }).to_string())?;
    if reply.contains("\"ok\":false") {
        bail!("bridge refused the update: {reply}");
    }
    log("bridge accepted; waiting for the download (allow the firewall prompt if one appears)".into());

    let deadline = Instant::now() + Duration::from_secs(180);
    let mut served = false;
    while !served && Instant::now() < deadline {
        match listener.accept() {
            Ok((mut s, peer)) => {
                s.set_nonblocking(false)?;
                s.set_read_timeout(Some(Duration::from_secs(10)))?;
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = s.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", image.len());
                s.write_all(head.as_bytes())?;
                s.write_all(image)?;
                s.flush()?;
                log(format!("sent {} bytes to {peer}", image.len()));
                served = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    }
    if !served {
        bail!("the bridge never fetched the image (firewall? different network?)");
    }
    log("installing; the bridge restarts".into());
    let installing = Instant::now();
    std::thread::sleep(Duration::from_secs(8));
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(v) = status(deck) {
            // the old firmware answers until it has written the image: only a bridge that
            // started after the download counts
            let restarted = v.get("uptime_s").and_then(|u| u.as_u64()).map(|u| u <= installing.elapsed().as_secs()).unwrap_or(false);
            if let Some(ver) = v.get("version").and_then(|v| v.as_str()).filter(|_| restarted) {
                log(format!("bridge runs {ver} (was {old})"));
                return Ok(ver.to_string());
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    bail!("the bridge did not report back within 60 s (an image that fails is rolled back on its next reset)")
}
