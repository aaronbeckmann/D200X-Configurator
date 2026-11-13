//! "ESP32 bridge" window: the optional hardware mod (esp32-bridge/docs/WIRING.md).
//!
//! - switch the deck's console link on (deck config `agents.uart.enabled`),
//! - hand plugins over from the PC agent (Home Assistant, OBS, Spotify): their settings go to the
//!   bridge, the deck routes them to the bridge first and to the PC agent as fallback,
//! - build and flash the bridge firmware (USB for the first install, Wi-Fi through the deck after).
//!
//! Nothing here changes the PC agent: without the mod (or with the link off) every plugin keeps
//! running on the PC exactly as before.

use crate::app::{repo_root, App};
use crate::ha;
use d200x_core::Deck;
use deck_config::LinkKind;
use eframe::egui::{self, Color32, RichText};
use serde_json::{json, Value};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Plugins both the PC agent and the bridge can run.
pub const HANDOFF: [(&str, &str); 3] = [("homeassistant", "Home Assistant"), ("obs", "OBS Studio"), ("spotify", "Spotify")];

/// Plugins the bridge reports as ready, when the deck's link to it is on (for the "needs agent" hints).
pub fn ready_plugins(app: &App) -> Vec<String> {
    if !app.cfg.agents.uart.enabled {
        return Vec::new();
    }
    let st = app.bridge_status.lock().unwrap();
    st.as_ref()
        .and_then(|v| v.get("plugins"))
        .and_then(|p| p.as_object())
        .map(|m| m.iter().filter(|(_, v)| v.get("ready").and_then(|r| r.as_bool()) == Some(true)).map(|(k, _)| k.clone()).collect())
        .unwrap_or_default()
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    if !app.show_bridge {
        return;
    }
    let mut open = app.show_bridge;
    let busy = app.bridge_busy.load(Ordering::SeqCst);
    if app.bridge_handoff.is_empty() {
        // what the deck config already routes to the bridge first
        for (id, _) in HANDOFF {
            let first = app.cfg.agents.routes.get(id).and_then(|r| r.first().copied());
            app.bridge_handoff.insert(id.to_string(), first == Some(LinkKind::Uart));
        }
    }
    egui::Window::new("ESP32 bridge").open(&mut open).default_size([760.0, 620.0]).show(ctx, |ui| {
        ui.label(RichText::new("Optional hardware mod: a XIAO ESP32S3 on the deck's console UART runs network plugins with the PC off. Without it, or with the link off, everything keeps running on the PC agent as before.").weak());
        ui.separator();

        // 1. link
        let mut enabled = app.cfg.agents.uart.enabled;
        if ui.add_enabled(!busy, egui::Checkbox::new(&mut enabled, RichText::new("Bridge installed: the deck opens its console link").strong())).changed() {
            app.cfg.agents.uart.enabled = enabled;
            app.dirty = true;
            if app.connected {
                // the link only opens with the saved config: save right away
                app.save_to_deck();
                if !app.dirty {
                    log_line(&app.bridge_log, if enabled { "deck link on (saved to the deck)" } else { "deck link off (saved to the deck); everything runs on the PC agent" });
                    if enabled {
                        refresh_status(app);
                    }
                }
            }
        }
        ui.label(RichText::new("Saved to the deck at once. Needs deckd with bridge support (Firmware... > Install deckd). With the link on, the deck's serial root shell is off; ADB stays available.").small().weak());
        ui.add_space(6.0);

        // 2. status
        ui.horizontal(|ui| {
            ui.label(RichText::new("Status").strong());
            if ui.add_enabled(!busy && app.connected, egui::Button::new("Check")).clicked() {
                refresh_status(app);
            }
        });
        status_view(app, ui);
        ui.separator();

        // 3. hand-off
        ui.label(RichText::new("Run on the bridge").strong());
        ui.label(RichText::new("Checked plugins go to the bridge first; the PC agent stays their fallback (it keeps its settings and runs them while the bridge is missing or offline).").small().weak());
        for (id, label) in HANDOFF {
            let configured = agent_has(&app.agent_cfg, id);
            let mut on = app.bridge_handoff.get(id).copied().unwrap_or(false);
            ui.horizontal(|ui| {
                if ui.add_enabled(configured, egui::Checkbox::new(&mut on, label)).changed() {
                    app.bridge_handoff.insert(id.to_string(), on);
                }
                if !configured {
                    ui.label(RichText::new("set it up in Agent settings first").small().weak());
                }
            });
        }
        ui.label(RichText::new("Spotify: the bridge refreshes its own access tokens from the same login. If the PC agent later reports Spotify login errors, log in again there.").small().weak());
        ui.add_space(4.0);
        egui::Grid::new("bridge_wifi").num_columns(2).show(ui, |ui| {
            ui.label("Wi-Fi name");
            ui.add(egui::TextEdit::singleline(&mut app.bridge_ssid).desired_width(260.0));
            ui.end_row();
            ui.label("Wi-Fi password");
            ui.add(egui::TextEdit::singleline(&mut app.bridge_pass).password(true).desired_width(260.0).hint_text("empty: keep the stored one"));
            ui.end_row();
        });
        if ui.add_enabled(!busy && app.connected, egui::Button::new(RichText::new("Send settings to the bridge").strong())).clicked() {
            send_config(app);
        }
        ui.separator();

        // 4. firmware
        ui.label(RichText::new("Bridge firmware").strong());
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Build")).on_hover_text("idf.py build in esp32-bridge (ESP-IDF needed)").clicked() {
                build(app);
            }
            ui.label("USB port");
            ui.add(egui::TextEdit::singleline(&mut app.bridge_port).desired_width(70.0).hint_text("auto"));
            if ui.add_enabled(!busy, egui::Button::new("Flash over USB")).on_hover_text("first install or recovery: XIAO plugged into this PC").clicked() {
                flash_usb(app);
            }
            if ui.add_enabled(!busy && app.connected, egui::Button::new("Update over Wi-Fi")).on_hover_text("through the deck; the bridge downloads the image from this PC and rolls back if it fails").clicked() {
                update_wifi(app);
            }
            if busy {
                ui.spinner();
            }
        });
        let text = app.bridge_log.lock().unwrap().clone();
        egui::ScrollArea::vertical().stick_to_bottom(true).max_height(180.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut text.as_str()).desired_width(f32::INFINITY).desired_rows(8).font(egui::TextStyle::Monospace));
        });
    });
    app.show_bridge = open;
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }
}

fn status_view(app: &App, ui: &mut egui::Ui) {
    let st = app.bridge_status.lock().unwrap().clone();
    let Some(v) = st else {
        ui.label(RichText::new("not checked yet").weak());
        return;
    };
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        ui.label(RichText::new(format!("no bridge: {err}")).color(Color32::from_rgb(230, 120, 70)));
        return;
    }
    let s = |p: &str| v.pointer(p).map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).unwrap_or_default();
    let wifi = if v.pointer("/wifi/connected").and_then(|b| b.as_bool()) == Some(true) { format!("Wi-Fi {} ({} dBm), {}", s("/wifi/ssid"), s("/wifi/rssi"), s("/wifi/ip")) } else { format!("Wi-Fi not connected ({})", s("/wifi/ssid")) };
    ui.label(format!("firmware {} | up {} s | {}", s("/version"), s("/uptime_s"), wifi));
    let mut parts = Vec::new();
    if let Some(m) = v.get("plugins").and_then(|p| p.as_object()) {
        for (name, p) in m {
            if p.get("enabled").and_then(|b| b.as_bool()) == Some(true) {
                let ready = p.get("ready").and_then(|b| b.as_bool()) == Some(true);
                parts.push(format!("{name} {}", if ready { "ready" } else { "not ready" }));
            }
        }
    }
    ui.label(if parts.is_empty() { "no plugins configured".to_string() } else { parts.join(", ") });
}

fn agent_has(c: &ha::AgentConfig, id: &str) -> bool {
    match id {
        "homeassistant" => !c.homeassistant.url.is_empty() && !c.homeassistant.token.is_empty(),
        "obs" => !c.obs.url.is_empty(),
        "spotify" => !c.spotify.client_id.is_empty() && !ha::load_agent_config().spotify.refresh_token.is_empty(),
        _ => false,
    }
}

fn log_line(log: &Arc<Mutex<String>>, s: impl AsRef<str>) {
    let mut l = log.lock().unwrap();
    l.push_str(s.as_ref());
    l.push('\n');
}

/// Run `f` on a worker thread with the busy flag set.
fn background(app: &mut App, f: impl FnOnce(&Arc<Mutex<String>>, &Arc<Mutex<Option<Value>>>) + Send + 'static) {
    let log = app.bridge_log.clone();
    let st = app.bridge_status.clone();
    let busy = app.bridge_busy.clone();
    busy.store(true, Ordering::SeqCst);
    std::thread::spawn(move || {
        f(&log, &st);
        busy.store(false, Ordering::SeqCst);
    });
}

pub fn refresh_status(app: &mut App) {
    background(app, |_, st| {
        let v = Deck::open().and_then(|d| status_waiting(&d)).unwrap_or_else(|e| json!({ "error": friendly(&e.to_string()) }));
        *st.lock().unwrap() = Some(v);
    });
}

/// Bridge status; right after the link opens (or the bridge restarts) it takes a few seconds
/// until the bridge has registered with deckd, so "no uart agent" is retried for up to 10 s.
fn status_waiting(deck: &Deck) -> anyhow::Result<Value> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match d200x_core::bridge::status(deck) {
            Err(e) if e.to_string().contains("no uart agent") && std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_secs(1)),
            r => return r,
        }
    }
}

fn friendly(e: &str) -> String {
    if e.contains("no uart agent") {
        "the deck hears no bridge on its console link (link off in the deck config, deckd without bridge support, or the ESP32 not powered/wired)".into()
    } else {
        e.to_string()
    }
}

// ---- hand-off -----------------------------------------------------------------------------------

/// A PC-local URL as the bridge must call it: localhost becomes this PC's LAN address, and
/// mDNS names (*.local), which the bridge cannot resolve, become their IPv4 address.
fn url_for_bridge(url: &str, pc_ip: Option<IpAddr>) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    let (hostport, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, String::new()));
    let (host, port) = hostport.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(h, p)| (h.to_string(), Some(p.to_string()))).unwrap_or((hostport.to_string(), None));
    let new_host = if host == "localhost" || host == "127.0.0.1" {
        pc_ip.map(|ip| ip.to_string()).unwrap_or(host)
    } else if host.ends_with(".local") {
        format!("{host}:{}", port.as_deref().unwrap_or("0"))
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.find(|x| x.is_ipv4()))
            .map(|a| a.ip().to_string())
            .unwrap_or(host)
    } else {
        host
    };
    match port {
        Some(p) => format!("{scheme}://{new_host}:{p}{path}"),
        None => format!("{scheme}://{new_host}{path}"),
    }
}

fn send_config(app: &mut App) {
    if !app.cfg.agents.uart.enabled {
        log_line(&app.bridge_log, "tick \"Bridge installed\" first: the deck only talks to the bridge with its console link on");
        return;
    }
    let agent = ha::load_agent_config(); // fresh from disk: the agent rotates tokens
    let handoff = app.bridge_handoff.clone();
    let ssid = app.bridge_ssid.trim().to_string();
    let pass = app.bridge_pass.clone();
    // deck side: route the chosen plugins to the bridge first, the rest to the PC first
    for (id, _) in HANDOFF {
        let on = handoff.get(id).copied().unwrap_or(false);
        app.cfg.agents.routes.insert(id.to_string(), if on { vec![LinkKind::Uart, LinkKind::Usb] } else { vec![LinkKind::Usb, LinkKind::Uart] });
    }
    app.dirty = true;
    app.save_to_deck(); // routes take effect on the deck right away
    background(app, move |log, st| {
        let deck = match Deck::open() {
            Ok(d) => d,
            Err(e) => return log_line(log, format!("deck: {e}")),
        };
        // start from what the bridge holds (secrets come back as "***" and are kept by the bridge)
        let current = match status_waiting(&deck) {
            Ok(v) => Some(v),
            Err(e) => return log_line(log, format!("no bridge: {}", friendly(&e.to_string()))),
        };
        let mut doc = current.as_ref().and_then(|v| v.get("config").cloned()).filter(|c| c.is_object()).unwrap_or_else(|| json!({}));
        let pc_ip = current.as_ref().and_then(|v| v.pointer("/wifi/ip")).and_then(|i| i.as_str()).and_then(|i| i.parse().ok()).and_then(|ip| d200x_core::bridge::local_ip_towards(ip).ok());
        if !ssid.is_empty() {
            doc["wifi"] = json!({ "ssid": ssid, "password": if pass.is_empty() { "***".to_string() } else { pass } });
        }
        if doc.get("wifi").is_none() {
            return log_line(log, "enter the Wi-Fi name first");
        }
        let obj = doc.as_object_mut().unwrap();
        for (id, _) in HANDOFF {
            if !handoff.get(id).copied().unwrap_or(false) {
                obj.remove(id);
                continue;
            }
            let section = match id {
                "homeassistant" => json!({ "url": url_for_bridge(&agent.homeassistant.url, pc_ip), "token": agent.homeassistant.token }),
                "obs" => json!({ "url": url_for_bridge(&agent.obs.url, pc_ip), "password": agent.obs.password }),
                _ => json!({ "client_id": agent.spotify.client_id, "refresh_token": agent.spotify.refresh_token }),
            };
            if id == "obs" && pc_ip.is_none() && (agent.obs.url.contains("localhost") || agent.obs.url.contains("127.0.0.1")) {
                log_line(log, "OBS runs on this PC: send again once the bridge is on Wi-Fi, so its address can be filled in");
            }
            obj.insert(id.to_string(), section);
        }
        let shown: Vec<&str> = HANDOFF.iter().filter(|(id, _)| obj.contains_key(*id)).map(|(_, l)| *l).collect();
        match deck.bridge_config(&doc.to_string()) {
            Ok(r) if !r.contains("\"ok\":false") => {
                log_line(log, format!("bridge settings stored ({}); it restarts, routes saved to the deck", if shown.is_empty() { "Wi-Fi only".to_string() } else { shown.join(", ") }));
                std::thread::sleep(std::time::Duration::from_secs(8));
                *st.lock().unwrap() = Some(d200x_core::bridge::status(&deck).unwrap_or_else(|e| json!({ "error": e.to_string() })));
            }
            Ok(r) => log_line(log, format!("bridge refused: {r}")),
            Err(e) => log_line(log, format!("no answer from the bridge: {}", friendly(&e.to_string()))),
        }
    });
}

// ---- firmware -----------------------------------------------------------------------------------

fn bridge_dir() -> Option<PathBuf> {
    repo_root().map(|r| r.join("esp32-bridge")).filter(|p| p.join("CMakeLists.txt").exists())
}

/// ESP-IDF's export script: $IDF_PATH or the newest C:\esp\*\esp-idf.
fn idf_export() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("IDF_PATH") {
        let e = PathBuf::from(p).join("export.ps1");
        if e.exists() {
            return Some(e);
        }
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir("C:\\esp").ok()?.filter_map(|e| e.ok()).map(|e| e.path().join("esp-idf").join("export.ps1")).filter(|p| p.exists()).collect();
    found.sort();
    found.pop()
}

/// A Python that has esptool: ESP-IDF's environment, else the one on PATH.
fn esptool_python() -> String {
    let base = dirs::home_dir().map(|h| h.join(".espressif").join("python_env"));
    let mut envs: Vec<PathBuf> = base.and_then(|b| std::fs::read_dir(b).ok()).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path().join("Scripts").join("python.exe")).filter(|p| p.exists()).collect()).unwrap_or_default();
    envs.sort();
    envs.pop().map(|p| p.display().to_string()).unwrap_or_else(|| "python".into())
}

/// COM port of a XIAO ESP32S3 on USB (Espressif USB serial, 303A:1001 interface 0).
fn find_port() -> Option<String> {
    let out = hidden(std::process::Command::new("powershell").args(["-NoProfile", "-Command", "Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_303A&PID_1001&MI_00' } | ForEach-Object { $_.FriendlyName }"])).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let i = s.find("(COM")?;
    Some(s[i + 1..].split(')').next()?.to_string())
}

fn hidden(c: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Run a command, stream its output into the log.
fn run_logged(log: &Arc<Mutex<String>>, mut cmd: std::process::Command) -> bool {
    use std::io::BufRead;
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = match hidden(&mut cmd).spawn() {
        Ok(c) => c,
        Err(e) => {
            log_line(log, format!("cannot start: {e}"));
            return false;
        }
    };
    let err = child.stderr.take();
    let log2 = log.clone();
    let t = std::thread::spawn(move || {
        if let Some(e) = err {
            for line in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                if !line.trim().is_empty() && !line.contains("CategoryInfo") && !line.contains("FullyQualifiedErrorId") {
                    log_line(&log2, line);
                }
            }
        }
    });
    if let Some(o) = child.stdout.take() {
        for line in std::io::BufReader::new(o).lines().map_while(Result::ok) {
            log_line(log, line);
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    let _ = t.join();
    ok
}

fn build(app: &mut App) {
    let (Some(dir), Some(export)) = (bridge_dir(), idf_export()) else {
        log_line(&app.bridge_log, "esp32-bridge or ESP-IDF (C:\\esp\\...\\esp-idf) not found");
        return;
    };
    background(app, move |log, _| {
        log_line(log, format!("> idf.py build in {}", dir.display()));
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &format!(". '{}' *> $null; idf.py build", export.display())]).current_dir(&dir);
        let ok = run_logged(log, c);
        log_line(log, if ok { "[build finished]" } else { "[build failed]" });
    });
}

fn flash_usb(app: &mut App) {
    let Some(dir) = bridge_dir() else {
        log_line(&app.bridge_log, "esp32-bridge not found next to the configurator");
        return;
    };
    let build = dir.join("build");
    if !build.join("flash_args").exists() {
        log_line(&app.bridge_log, "no firmware built yet: press Build first");
        return;
    }
    let manual = app.bridge_port.trim().to_string();
    background(app, move |log, _| {
        let Some(port) = (if manual.is_empty() { find_port() } else { Some(manual) }) else {
            return log_line(log, "no XIAO ESP32S3 found on USB (plug it into this PC, or enter its COM port)");
        };
        log_line(log, format!("> esptool write-flash on {port} (bootloader, partitions, firmware; the bridge config in NVS is kept)"));
        let mut c = std::process::Command::new(esptool_python());
        c.args(["-m", "esptool", "--chip", "esp32s3", "-p", &port, "-b", "460800", "--before", "default-reset", "--after", "hard-reset", "write-flash", "@flash_args"]).current_dir(&build);
        let ok = run_logged(log, c);
        log_line(log, if ok { "[flashed; the bridge restarts]" } else { "[flash failed: if it cannot connect, hold BOOT, tap RESET, release BOOT and try again]" });
    });
}

fn update_wifi(app: &mut App) {
    let Some(bin) = bridge_dir().map(|d| d.join("build").join("esp32-bridge.bin")).filter(|p| p.exists()) else {
        log_line(&app.bridge_log, "no firmware built yet: press Build first");
        return;
    };
    background(app, move |log, st| {
        let image = match std::fs::read(&bin) {
            Ok(b) => b,
            Err(e) => return log_line(log, format!("{}: {e}", bin.display())),
        };
        let deck = match Deck::open() {
            Ok(d) => d,
            Err(e) => return log_line(log, format!("deck: {e}")),
        };
        match d200x_core::bridge::ota(&deck, &image, &mut |l| log_line(log, l)) {
            Ok(_) => *st.lock().unwrap() = d200x_core::bridge::status(&deck).ok(),
            Err(e) => log_line(log, format!("update failed: {e}")),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localhost_becomes_the_pc_address() {
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        assert_eq!(url_for_bridge("ws://127.0.0.1:4455", Some(ip)), "ws://192.168.1.20:4455");
        assert_eq!(url_for_bridge("ws://localhost:4455/x", Some(ip)), "ws://192.168.1.20:4455/x");
        assert_eq!(url_for_bridge("http://192.168.1.5:8123", Some(ip)), "http://192.168.1.5:8123");
        assert_eq!(url_for_bridge("https://ha.example.org", None), "https://ha.example.org");
        assert_eq!(url_for_bridge("ws://localhost:4455", None), "ws://localhost:4455");
    }
}
