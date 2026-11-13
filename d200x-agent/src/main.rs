//! d200x-agent: PC-side agent for the deckd firmware.
//!
//! Connects to the deck's vendor HID interface, sends a heartbeat every 2 s,
//! answers host actions and feeds states. Plugins: system, media (Windows media
//! session), homeassistant, obs, spotify, discord.
//!
//! Usage:
//!   d200x-agent                 run (reconnects forever)
//!   d200x-agent icon <mdi-name> <#rrggbb> <out.png>
//!   d200x-agent entities [filter]      list Home Assistant entities
//!
//! 127.0.0.1:8898 answers every connection with a JSON snapshot of all states (no secrets) for
//! the configurator's state explorer.

mod agentdeck;
mod config;
mod discord;
mod ha;
mod media;
mod obs;
mod spotify;
mod system;

use d200x_core::{Deck, EV_HELLO, EV_HOST_ACTION};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AGENT_NAME: &str = concat!("d200x-agent ", env!("CARGO_PKG_VERSION"));
const STATES_ADDR: &str = "127.0.0.1:8898";

struct Logger {
    file: Mutex<Option<std::fs::File>>,
}

impl Logger {
    fn line(&self, s: &str) {
        let ts = now_hms();
        println!("{ts} {s}");
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{ts} {s}");
        }
    }
}

fn now_hms() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02}Z", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
}

/// Everything the deck loop needs from the plugins.
struct Plugins {
    fg: system::Foreground,
    agentdeck: Option<agentdeck::AgentDeck>,
    ha: Option<ha::Ha>,
    media: Option<media::Media>,
    obs: Option<obs::Obs>,
    spotify: Option<Arc<spotify::Spotify>>,
    discord: Option<discord::Discord>,
}

impl Plugins {
    fn names(&self) -> Vec<&'static str> {
        let mut v = vec!["system"];
        if self.media.is_some() {
            v.push("media");
        }
        if self.ha.is_some() {
            v.push("homeassistant");
        }
        if self.obs.is_some() {
            v.push("obs");
        }
        if self.spotify.is_some() {
            v.push("spotify");
        }
        if self.discord.is_some() {
            v.push("discord");
        }
        if self.agentdeck.is_some() {
            v.push("agentdeck");
        }
        v
    }

    fn resolve(&self, name: &str) -> Option<String> {
        if name.starts_with("system:") {
            return self.fg.resolve(name);
        }
        if name.starts_with("ha:") {
            return self.ha.as_ref().and_then(|h| h.resolve(name));
        }
        if name.starts_with("media:") {
            return self.media.as_ref().and_then(|m| m.resolve(name));
        }
        if name.starts_with("obs:") {
            return self.obs.as_ref().and_then(|o| o.resolve(name));
        }
        if name.starts_with("spotify:") {
            return self.spotify.as_ref().and_then(|s| s.resolve(name));
        }
        if name.starts_with("discord:") {
            return self.discord.as_ref().and_then(|d| d.resolve(name));
        }
        if name.starts_with("agentdeck:") {
            return self.agentdeck.as_ref().and_then(|a| a.resolve(name));
        }
        None
    }

    /// Every state known right now, for the state explorer.
    fn snapshot(&self) -> Value {
        let mut names: Vec<String> = ["system:app", "system:title"].iter().chain(system::INFO_STATES.iter()).map(|s| s.to_string()).collect();
        if self.media.is_some() {
            names.extend(media::STATES.map(str::to_string));
        }
        names.extend(self.ha.iter().flat_map(|h| h.state_names()));
        names.extend(self.obs.iter().flat_map(|o| o.state_names()));
        names.extend(self.spotify.iter().flat_map(|s| s.state_names()));
        names.extend(self.discord.iter().flat_map(|d| d.state_names()));
        names.extend(self.agentdeck.iter().flat_map(|a| a.state_names()));
        let states: serde_json::Map<String, Value> = names.into_iter().filter_map(|n| self.resolve(&n).map(|v| (n, Value::String(v)))).collect();
        json!({ "plugins": self.names(), "states": states })
    }

    /// Did anything change since the last call? (HA changes are tracked per entity.)
    fn changed(&self) -> (bool, Vec<String>) {
        let mut any = self.fg.take_changed();
        let ha_changed = self.ha.as_ref().map(|h| h.drain_changes()).unwrap_or_default();
        any |= !ha_changed.is_empty();
        any |= self.media.as_ref().map(|m| m.take_changed()).unwrap_or(false);
        any |= self.obs.as_ref().map(|o| o.take_changed()).unwrap_or(false);
        any |= self.spotify.as_ref().map(|s| s.take_changed()).unwrap_or(false);
        any |= self.discord.as_ref().map(|d| d.take_changed()).unwrap_or(false);
        any |= self.agentdeck.as_ref().map(|a| a.take_changed()).unwrap_or(false);
        (any, ha_changed)
    }

    fn run(&self, plugin: &str, op: &str, args: &Value, event: &str, delta: i32) -> anyhow::Result<()> {
        match plugin {
            "system" => system::run(op, args),
            "media" => self.media.as_ref().ok_or_else(|| anyhow::anyhow!("media plugin disabled"))?.run(op),
            "homeassistant" => self.ha.as_ref().ok_or_else(|| anyhow::anyhow!("homeassistant plugin not configured"))?.run(op, args, event, delta),
            "obs" => self.obs.as_ref().ok_or_else(|| anyhow::anyhow!("obs plugin not configured"))?.run(op, args),
            "spotify" => self.spotify.as_ref().ok_or_else(|| anyhow::anyhow!("spotify plugin not configured"))?.run(op, args, delta),
            "discord" => self.discord.as_ref().ok_or_else(|| anyhow::anyhow!("discord plugin not configured"))?.run(op, args),
            "agentdeck" => {
                let mut a = args.clone();
                if op == "key" {
                    a["event"] = Value::String(event.to_string());
                }
                self.agentdeck.as_ref().ok_or_else(|| anyhow::anyhow!("agentdeck plugin disabled"))?.run(op, &a)
            }
            other => Err(anyhow::anyhow!("unknown plugin {other}")),
        }
    }
}

/// One agent per user session: the autostart entry and the configurator's start button may both
/// launch one. The named mutex lives as long as the process.
fn first_instance() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\d200x-agent".encode_utf16().chain(std::iter::once(0)).collect();
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        return h.is_null() || GetLastError() != ERROR_ALREADY_EXISTS;
    }
    #[allow(unreachable_code)]
    true
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cfg = config::load();
    let _ = std::fs::create_dir_all(config::config_dir());
    let logger = Arc::new(Logger { file: Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(config::log_path()).ok()) });

    match args.get(1).map(String::as_str) {
        Some("icon") => {
            let name = args.get(2).expect("mdi name");
            let color = args.get(3).map(String::as_str).unwrap_or("#ffffff");
            let out = args.get(4).expect("output path");
            let png = deck_icons::render_mdi(name, color, deck_icons::DEFAULT_SIZE).expect("render");
            std::fs::write(out, &png).expect("write");
            println!("wrote {out} ({} bytes)", png.len());
            return;
        }
        Some("entities") => {
            let filter = args.get(2).cloned().unwrap_or_default();
            let url = format!("{}/api/states", cfg.homeassistant.url.trim_end_matches('/'));
            let resp: Value = ureq::get(&url).set("Authorization", &format!("Bearer {}", cfg.homeassistant.token)).call().expect("HA states").into_json().expect("json");
            for e in resp.as_array().unwrap_or(&vec![]) {
                let id = e.get("entity_id").and_then(|i| i.as_str()).unwrap_or("");
                if id.contains(&filter) {
                    println!("{id}  state={}  name={}", e.get("state").and_then(|s| s.as_str()).unwrap_or(""), e.pointer("/attributes/friendly_name").and_then(|n| n.as_str()).unwrap_or(""));
                }
            }
            return;
        }
        Some(other) if other.starts_with('-') || other == "run" => {}
        Some(other) => {
            eprintln!("unknown command {other}");
            return;
        }
        None => {}
    }

    if !first_instance() {
        logger.line(&format!("{AGENT_NAME}: another agent already runs, exiting"));
        return;
    }
    logger.line(&format!("{AGENT_NAME} starting; config {}", config::config_path().display()));
    let log_fn: Arc<dyn Fn(&str) + Send + Sync> = {
        let l = logger.clone();
        Arc::new(move |s: &str| l.line(s))
    };
    let enabled = |p: &str| cfg.plugins.iter().any(|x| x == p);
    let plugins = Arc::new(Plugins {
        fg: system::Foreground::start(),
        agentdeck: if enabled("agentdeck") { Some(agentdeck::AgentDeck::start(cfg.agentdeck.port, cfg.agentdeck.anim_frames, log_fn.clone())) } else { None },
        ha: if enabled("homeassistant") && !cfg.homeassistant.url.is_empty() { Some(ha::Ha::start(cfg.homeassistant.url.clone(), cfg.homeassistant.token.clone(), log_fn.clone())) } else { None },
        media: if enabled("media") { Some(media::Media::start(log_fn.clone())) } else { None },
        obs: if enabled("obs") && !cfg.obs.url.is_empty() { Some(obs::Obs::start(cfg.obs.url.clone(), cfg.obs.password.clone(), log_fn.clone())) } else { None },
        spotify: if enabled("spotify") && !cfg.spotify.client_id.is_empty() { Some(spotify::Spotify::start(cfg.spotify.client_id.clone(), cfg.spotify.refresh_token.clone(), log_fn.clone())) } else { None },
        discord: if enabled("discord") && !cfg.discord.client_id.is_empty() { Some(discord::Discord::start(cfg.discord.client_id.clone(), cfg.discord.client_secret.clone(), cfg.discord.access_token.clone(), log_fn.clone())) } else { None },
    });
    logger.line(&format!("plugins: {:?}", plugins.names()));
    serve_states(plugins.clone(), log_fn.clone());

    loop {
        let deck = match Deck::open() {
            Ok(d) => d,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        logger.line(&format!("deck connected: {:?}", deck.serial));
        if let Err(e) = session(&deck, &plugins, &logger) {
            logger.line(&format!("deck session ended: {e}"));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Answer the configurator's state explorer. Without the port the agent works as before.
fn serve_states(plugins: Arc<Plugins>, log: Arc<dyn Fn(&str) + Send + Sync>) {
    let listener = match std::net::TcpListener::bind(STATES_ADDR) {
        Ok(l) => l,
        Err(e) => return log(&format!("state explorer disabled, cannot listen on {STATES_ADDR}: {e}")),
    };
    std::thread::Builder::new()
        .name("states".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let _ = stream.write_all(plugins.snapshot().to_string().as_bytes());
            }
        })
        .expect("states thread");
}

/// One connection to the deck. Returns when the deck goes away.
fn session(deck: &Deck, plugins: &Plugins, log: &Logger) -> anyhow::Result<()> {
    let names = plugins.names();
    let mut last_hb = Instant::now() - Duration::from_secs(10);
    let mut wanted: HashSet<String> = HashSet::new();
    let mut sent: HashMap<String, String> = HashMap::new();
    let mut hello_seen = false;
    let mut last_full_push = Instant::now();
    let mut deck_icons: HashSet<String> = HashSet::new();
    loop {
        if last_hb.elapsed() >= Duration::from_secs(2) {
            deck.send_heartbeat(AGENT_NAME, &names)?;
            last_hb = Instant::now();
        }
        if let Some(m) = deck.read_any(40)? {
            match m.cmd {
                EV_HELLO => {
                    if let Ok(v) = serde_json::from_slice::<Value>(&m.payload) {
                        wanted = v.get("states").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        deck_icons = v.get("icons").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        log.line(&format!("hello from {} {} serial {}; {} states wanted", v.get("firmware").and_then(|f| f.as_str()).unwrap_or("?"), v.get("version").and_then(|f| f.as_str()).unwrap_or(""), v.get("serial").and_then(|f| f.as_str()).unwrap_or(""), wanted.len()));
                        hello_seen = true;
                        sent.clear();
                        if let Some(a) = plugins.agentdeck.as_ref() {
                            a.set_wanted(&wanted);
                        }
                    }
                }
                EV_HOST_ACTION => {
                    let v: Value = match serde_json::from_slice(&m.payload) {
                        Ok(v) => v,
                        Err(e) => {
                            log.line(&format!("bad host action: {e}"));
                            continue;
                        }
                    };
                    let id = v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                    let plugin = v.get("plugin").and_then(|p| p.as_str()).unwrap_or("");
                    let op = v.get("op").and_then(|p| p.as_str()).unwrap_or("");
                    let args = v.get("args").cloned().unwrap_or(json!({}));
                    let event = v.get("event").and_then(|p| p.as_str()).unwrap_or("press");
                    let delta = v.get("delta").and_then(|d| d.as_i64()).unwrap_or(0) as i32;
                    let result = plugins.run(plugin, op, &args, event, delta);
                    match &result {
                        Ok(()) => log.line(&format!("action {id} {plugin}.{op} {args} ok")),
                        Err(e) => log.line(&format!("action {id} {plugin}.{op} failed: {e}")),
                    }
                    deck.send_result(id, result.is_ok(), &result.err().map(|e| e.to_string()).unwrap_or_default())?;
                }
                _ => {}
            }
        }
        // AgentDeck tiles rendered by the bridge: upload to the deck's RAM icon store, then point the key at them
        if let Some(a) = plugins.agentdeck.as_ref() {
            for up in a.take_uploads() {
                let base = format!("ram-ad{}-{}", up.key, up.sig);
                if let Some(png) = &up.png {
                    let name = format!("{base}.png");
                    if !deck_icons.contains(&name) {
                        deck.send(d200x_core::Cmd::DeckdSetIcon, &[name.as_bytes(), b"\0", png].concat())?;
                        deck_icons.insert(name);
                    }
                    a.set_tile(up.key, base.clone());
                }
                if !up.frames.is_empty() {
                    let n = up.frames.len();
                    for (i, f) in up.frames.iter().enumerate() {
                        let name = format!("{base}-f{i:02}.png");
                        if !deck_icons.contains(&name) {
                            deck.send(d200x_core::Cmd::DeckdSetIcon, &[name.as_bytes(), b"\0", f].concat())?;
                            deck_icons.insert(name);
                        }
                        // keep heartbeats and replies flowing during a long burst
                        while let Some(_) = deck.read_any(0)? {}
                    }
                    a.set_tile(up.key, format!("{base}|{n}|{}", up.delay));
                }
            }
        }
        // album art for keys using media-art.png
        if let Some(m) = plugins.media.as_ref() {
            if let Some(png) = m.take_art() {
                if wanted.iter().any(|w| w.starts_with("media:")) || deck_icons.contains("media-art.png") {
                    let _ = deck.send(d200x_core::Cmd::DeckdSetIcon, &[b"media-art.png\0".as_slice(), &png].concat());
                    // the deck answers 0x0300; it is read and ignored above
                }
            }
        }
        // states -> deck
        let (any, ha_changed) = plugins.changed();
        let full = hello_seen && (sent.is_empty() || last_full_push.elapsed() > Duration::from_secs(60));
        if !wanted.is_empty() && (any || full) {
            let mut map = serde_json::Map::new();
            for name in &wanted {
                if !full && name.starts_with("ha:") {
                    let Some(ent) = ha::Ha::entity_of(name) else { continue };
                    if !ha_changed.iter().any(|c| c == ent) {
                        continue;
                    }
                }
                if let Some(val) = plugins.resolve(name) {
                    if sent.get(name) != Some(&val) {
                        sent.insert(name.clone(), val.clone());
                        map.insert(name.clone(), Value::String(val));
                    }
                }
            }
            if !map.is_empty() {
                deck.send_states_json(&Value::Object(map).to_string())?;
            }
            if full {
                last_full_push = Instant::now();
            }
        }
    }
}
