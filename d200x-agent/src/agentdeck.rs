//! `agentdeck` plugin: AgentDeck daemon (github.com/puritysb/AgentDeck) over its local
//! WebSocket (ws://127.0.0.1:<port>, port from ~/.agentdeck/daemon.json, default 9120).
//! Mirrors the coding-agent sessions (Claude Code, Codex, OpenCode, ...) onto deck tiles and
//! answers their prompts.
//!
//! states:
//!   agentdeck:connected                    true/false
//!   agentdeck:state                        offline | idle | processing | waiting
//!   agentdeck:count / agentdeck:waiting    alive sessions / sessions waiting for an answer
//!   agentdeck:slot:N:state                 empty | idle | processing | awaiting_permission | awaiting_option | awaiting_diff | disconnected
//!   agentdeck:slot:N:name / :line / :tool / :question   (N = 0..7, roster order)
//!   agentdeck:sel:state / :name / :line / :question / :tool / :model / :mode / :context / :slot
//!   agentdeck:sel:opt:I                    label of option I (0..5) of the selected session's prompt
//!   agentdeck:usage:5h / :7d / :tokens     Claude quota percent, session tokens
//! ops:
//!   select {slot}, select_next, select_prev, approve, deny, option {index}, stop, escape,
//!   prompt {text}, mode {mode}, review, voice {action}, refresh

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub const SLOTS: usize = 8;

#[derive(Default, Clone)]
struct Session {
    id: String,
    name: String,
    state: String,
    alive: bool,
    observed: bool,
    tool: String,
    question: String,
    options: Vec<(i64, String, String)>, // index, label, shortcut
    request_id: String,
    live_answerable: bool,
    model: String,
    mode: String,
    context: String,
}

#[derive(Default)]
struct Model {
    connected: bool,
    sessions: Vec<Session>,
    focused: String,
    selected: String,
    navigable: bool,
    usage_5h: String,
    usage_7d: String,
    tokens: String,
}

pub struct AgentDeck {
    states: Arc<Mutex<HashMap<String, String>>>,
    model: Arc<Mutex<Model>>,
    tx: Mutex<Option<mpsc::Sender<Value>>>,
    changed: Arc<Mutex<bool>>,
    bridge: Mutex<Option<Bridge>>,
    anim_frames: u16,
    port: u16,
    /// Rendered tiles waiting to be uploaded to the deck (from the bridge).
    uploads: Arc<Mutex<Vec<TileUpload>>>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// One rendered tile from the bridge: static image or animation frames.
pub struct TileUpload {
    pub key: u32,
    pub sig: String,
    pub png: Option<Vec<u8>>,
    pub frames: Vec<Vec<u8>>,
    pub delay: u64,
}

struct Bridge {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
}

/// Where the Node bridge lives: next to the exe, or in the source tree.
fn bridge_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("bridge"), dir.join("../../bridge"), dir.join("../../../d200x-agent/bridge")].into_iter().find(|p| p.join("bridge.mjs").exists())
}

fn short_state(s: &str) -> String {
    let s = s.to_lowercase();
    if s.is_empty() {
        "idle".into()
    } else {
        s
    }
}

fn awaiting(s: &str) -> bool {
    s.starts_with("awaiting")
}

impl AgentDeck {
    pub fn start(port: u16, anim_frames: u16, log: Arc<dyn Fn(&str) + Send + Sync>) -> AgentDeck {
        let states: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let model = Arc::new(Mutex::new(Model::default()));
        let changed = Arc::new(Mutex::new(false));
        let (tx, rx) = mpsc::channel::<Value>();
        let rx = Arc::new(Mutex::new(rx));
        {
            let (st, md, ch) = (states.clone(), model.clone(), changed.clone());
            let log = log.clone();
            std::thread::Builder::new()
                .name("agentdeck".into())
                .spawn(move || loop {
                    let p = if port != 0 { port } else { discover_port() };
                    let r = ws_loop(p, &st, &md, &ch, &rx, &log);
                    {
                        let mut m = md.lock().unwrap();
                        m.connected = false;
                        m.sessions.clear();
                        publish(&m, &st, &ch);
                    }
                    if let Err(e) = r {
                        log(&format!("agentdeck: {e}; retry in 5 s"));
                    }
                    std::thread::sleep(Duration::from_secs(5));
                })
                .expect("agentdeck thread");
        }
        let ad = AgentDeck { states, model, tx: Mutex::new(Some(tx)), changed, bridge: Mutex::new(None), anim_frames, port, uploads: Arc::new(Mutex::new(Vec::new())), log: log.clone() };
        ad.start_bridge();
        ad
    }

    /// Start the Node tile bridge (AgentDeck's own renderers). Without Node the plain states still work.
    fn start_bridge(&self) {
        let Some(dir) = bridge_dir() else {
            (self.log)("agentdeck: bridge not found (d200x-agent/bridge); official tiles disabled");
            return;
        };
        let mut cmd = std::process::Command::new("node");
        cmd.arg(dir.join("bridge.mjs")).current_dir(&dir);
        if self.anim_frames > 0 {
            cmd.env("AGENTDECK_ANIM_FRAMES", self.anim_frames.to_string());
        }
        if self.port > 0 {
            cmd.env("AGENTDECK_PORT", self.port.to_string());
        }
        cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let stdin = child.stdin.take().expect("stdin");
                let stdout = child.stdout.take().expect("stdout");
                let stderr = child.stderr.take();
                let (uploads, log) = (self.uploads.clone(), self.log.clone());
                std::thread::Builder::new()
                    .name("agentdeck-bridge".into())
                    .spawn(move || {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                            match v.get("ev").and_then(|e| e.as_str()).unwrap_or("") {
                                "log" => log(&format!("bridge: {}", v.get("msg").and_then(|m| m.as_str()).unwrap_or(""))),
                                "tile" | "anim" => {
                                    use base64::Engine;
                                    let b64 = base64::engine::general_purpose::STANDARD;
                                    let key = v.get("key").and_then(|k| k.as_u64()).unwrap_or(99) as u32;
                                    let sig = v.get("sig").and_then(|s| s.as_str()).unwrap_or("").to_string();
                                    let png = v.get("png").and_then(|p| p.as_str()).and_then(|p| b64.decode(p).ok());
                                    let frames: Vec<Vec<u8>> = v.get("frames").and_then(|f| f.as_array()).map(|a| a.iter().filter_map(|f| f.as_str()).filter_map(|f| b64.decode(f).ok()).collect()).unwrap_or_default();
                                    let delay = v.get("delay").and_then(|d| d.as_u64()).unwrap_or(145);
                                    if key < 14 && !sig.is_empty() {
                                        let mut q = uploads.lock().unwrap();
                                        // a newer static tile for the same key supersedes queued ones
                                        if png.is_some() {
                                            q.retain(|u| u.key != key);
                                        }
                                        q.push(TileUpload { key, sig, png, frames, delay });
                                    }
                                }
                                _ => {}
                            }
                        }
                        log("agentdeck: bridge exited");
                    })
                    .expect("bridge thread");
                if let Some(e) = stderr {
                    let log = self.log.clone();
                    std::thread::spawn(move || {
                        use std::io::BufRead;
                        for line in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                            log(&format!("bridge! {line}"));
                        }
                    });
                }
                *self.bridge.lock().unwrap() = Some(Bridge { child, stdin });
                (self.log)(&format!("agentdeck: bridge started from {}", dir.display()));
            }
            Err(e) => (self.log)(&format!("agentdeck: cannot start node bridge: {e} (install Node.js 22+)")),
        }
    }

    fn bridge_send(&self, v: Value) -> Result<()> {
        use std::io::Write;
        let mut b = self.bridge.lock().unwrap();
        let Some(br) = b.as_mut() else { return Err(anyhow!("tile bridge not running (Node.js needed)")) };
        writeln!(br.stdin, "{v}").map_err(|e| anyhow!("bridge: {e}"))?;
        br.stdin.flush().map_err(|e| anyhow!("bridge: {e}"))
    }

    /// The deck told us which states it wants: tell the bridge which keys carry tiles.
    pub fn set_wanted(&self, wanted: &std::collections::HashSet<String>) {
        let mut keys: Vec<u32> = wanted.iter().filter_map(|w| w.strip_prefix("agentdeck:k")).filter_map(|k| k.parse().ok()).collect();
        keys.sort();
        self.uploads.lock().unwrap().clear();
        let _ = self.bridge_send(json!({ "cmd": "positions", "keys": keys }));
    }

    /// Rendered tiles waiting for upload.
    pub fn take_uploads(&self) -> Vec<TileUpload> {
        std::mem::take(&mut *self.uploads.lock().unwrap())
    }

    /// Record the tile image now on the deck for a key (called after the upload).
    pub fn set_tile(&self, key: u32, spec: String) {
        let mut st = self.states.lock().unwrap();
        let name = format!("agentdeck:k{key}");
        if st.get(&name) != Some(&spec) {
            st.insert(name, spec);
            *self.changed.lock().unwrap() = true;
        }
    }

    pub fn take_changed(&self) -> bool {
        std::mem::replace(&mut *self.changed.lock().unwrap(), false)
    }

    pub fn resolve(&self, name: &str) -> Option<String> {
        self.states.lock().unwrap().get(name).cloned()
    }

    /// Without `agentdeck:k<n>`, the tile images: those only mean something to the deck.
    pub fn state_names(&self) -> Vec<String> {
        self.states.lock().unwrap().keys().filter(|k| !k.starts_with("agentdeck:k")).cloned().collect()
    }

    fn send(&self, v: Value) -> Result<()> {
        let tx = self.tx.lock().unwrap();
        let tx = tx.as_ref().ok_or_else(|| anyhow!("agentdeck not started"))?;
        if !self.model.lock().unwrap().connected {
            return Err(anyhow!("AgentDeck daemon not connected"));
        }
        tx.send(v).map_err(|_| anyhow!("agentdeck: send failed"))
    }

    fn selected(&self) -> Result<Session> {
        let m = self.model.lock().unwrap();
        m.sessions.iter().find(|s| s.id == m.selected).cloned().ok_or_else(|| anyhow!("no session selected"))
    }

    /// A command for the selected session: observed sessions take it wrapped in
    /// `session_command`; managed (PTY) sessions take it after `focus_session`.
    fn session_cmd(&self, s: &Session, cmd: Value) -> Result<()> {
        if s.observed {
            self.send(json!({ "type": "session_command", "sessionId": s.id, "command": cmd }))
        } else {
            if self.model.lock().unwrap().focused != s.id {
                self.send(json!({ "type": "focus_session", "sessionId": s.id }))?;
            }
            self.send(cmd)
        }
    }

    fn select_slot(&self, slot: usize) -> Result<()> {
        let id = {
            let mut m = self.model.lock().unwrap();
            let id = m.sessions.get(slot).map(|s| s.id.clone()).ok_or_else(|| anyhow!("slot {} is empty", slot + 1))?;
            m.selected = id.clone();
            publish(&m, &self.states, &self.changed);
            id
        };
        let _ = self.send(json!({ "type": "focus_session", "sessionId": id }));
        Ok(())
    }

    fn select_step(&self, delta: i32) -> Result<()> {
        let (slot, n) = {
            let m = self.model.lock().unwrap();
            let n = m.sessions.len();
            if n == 0 {
                return Err(anyhow!("no sessions"));
            }
            let cur = m.sessions.iter().position(|s| s.id == m.selected).unwrap_or(0) as i32;
            ((cur + delta).rem_euclid(n as i32) as usize, n)
        };
        let _ = n;
        self.select_slot(slot)
    }

    /// Answer option `i` of the selected session's prompt (index into its option list).
    fn answer(&self, s: &Session, i: usize) -> Result<()> {
        let opt = s.options.get(i).cloned();
        let navigable = self.model.lock().unwrap().navigable;
        match opt {
            Some((index, _label, shortcut)) => {
                if s.observed && !s.live_answerable {
                    return Err(anyhow!("this prompt must be answered in the terminal"));
                }
                if navigable || s.live_answerable || s.observed {
                    let mut cmd = json!({ "type": "select_option", "index": index, "sessionId": s.id });
                    if !s.question.is_empty() {
                        cmd["question"] = json!(s.question);
                    }
                    self.send(cmd)
                } else {
                    let v = if shortcut.is_empty() { (i + 1).to_string() } else { shortcut };
                    self.session_cmd(s, json!({ "type": "respond", "value": v }))
                }
            }
            None => Err(anyhow!("no option {}", i + 1)),
        }
    }

    pub fn run(&self, op: &str, args: &Value) -> Result<()> {
        let num = |k: &str| args.get(k).and_then(|v| v.as_i64());
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
        match op {
            "key" => {
                let k = num("key").unwrap_or(0);
                let ev = args.get("event").and_then(|e| e.as_str()).unwrap_or("press");
                if ev == "release" {
                    self.bridge_send(json!({ "cmd": "release", "key": k }))
                } else {
                    self.bridge_send(json!({ "cmd": "press", "key": k }))
                }
            }
            "select" => self.select_slot(num("slot").unwrap_or(0).max(0) as usize),
            "select_next" => self.select_step(1),
            "select_prev" => self.select_step(-1),
            "refresh" => self.send(json!({ "type": "query_usage" })),
            "approve" | "deny" => {
                let s = self.selected()?;
                let allow = op == "approve";
                if !s.request_id.is_empty() {
                    return self.send(json!({ "type": "permission_decision", "requestId": s.request_id, "decision": if allow { "allow" } else { "deny" } }));
                }
                if !s.options.is_empty() {
                    // yes = first option, no = last option (Claude's prompts list "No" last)
                    let i = if allow { 0 } else { s.options.len() - 1 };
                    return self.answer(&s, i);
                }
                if awaiting(&s.state) && !s.observed {
                    return self.session_cmd(&s, json!({ "type": "respond", "value": if allow { "y" } else { "n" } }));
                }
                Err(anyhow!("{} is not waiting for an answer", s.name))
            }
            "option" => {
                let s = self.selected()?;
                self.answer(&s, num("index").unwrap_or(0).max(0) as usize)
            }
            "stop" => {
                let s = self.selected()?;
                self.session_cmd(&s, json!({ "type": "interrupt" }))
            }
            "escape" => {
                let s = self.selected()?;
                if s.observed {
                    return Err(anyhow!("escape cannot reach an observed session"));
                }
                self.session_cmd(&s, json!({ "type": "escape" }))
            }
            "prompt" => {
                let s = self.selected()?;
                let t = text("text").ok_or_else(|| anyhow!("prompt: missing text"))?;
                self.session_cmd(&s, json!({ "type": "send_prompt", "text": t }))
            }
            "mode" => {
                let s = self.selected()?;
                let mut cmd = json!({ "type": "switch_mode" });
                if let Some(m) = text("mode") {
                    cmd["mode"] = json!(m);
                }
                self.session_cmd(&s, cmd)
            }
            "review" => {
                let s = self.selected()?;
                self.send(json!({ "type": "review_run", "sessionId": s.id }))
            }
            "voice" => {
                let s = self.selected()?;
                self.send(json!({ "type": "voice", "action": text("action").unwrap_or_else(|| "start".into()), "sessionId": s.id }))
            }
            other => Err(anyhow!("agentdeck: unknown op {other}")),
        }
    }
}

/// Port from ~/.agentdeck/daemon.json, else 9120.
fn discover_port() -> u16 {
    let file = dirs::home_dir().map(|h| h.join(".agentdeck").join("daemon.json"));
    file.and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.get("port").and_then(|p| p.as_u64())).map(|p| p as u16).unwrap_or(9120)
}

fn s_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn parse_options(v: &Value) -> Vec<(i64, String, String)> {
    v.get("options")
        .and_then(|o| o.as_array())
        .map(|a| a.iter().enumerate().map(|(i, o)| (o.get("index").and_then(|x| x.as_i64()).unwrap_or(i as i64), s_of(o, "label"), s_of(o, "shortcut"))).collect())
        .unwrap_or_default()
}

fn trim_line(s: &str, n: usize) -> String {
    let first = s.lines().next().unwrap_or("").trim();
    if first.chars().count() > n {
        format!("{}…", first.chars().take(n - 1).collect::<String>())
    } else {
        first.to_string()
    }
}

/// Derive the deck states from the model.
fn publish(m: &Model, states: &Arc<Mutex<HashMap<String, String>>>, changed: &Arc<Mutex<bool>>) {
    let mut out: HashMap<String, String> = HashMap::new();
    out.insert("agentdeck:connected".into(), m.connected.to_string());
    let alive = m.sessions.iter().filter(|s| s.alive).count();
    let waiting = m.sessions.iter().filter(|s| awaiting(&s.state)).count();
    out.insert("agentdeck:count".into(), alive.to_string());
    out.insert("agentdeck:waiting".into(), waiting.to_string());
    let agg = if !m.connected {
        "offline"
    } else if waiting > 0 {
        "waiting"
    } else if m.sessions.iter().any(|s| s.state == "processing") {
        "processing"
    } else {
        "idle"
    };
    out.insert("agentdeck:state".into(), agg.into());
    let line_of = |s: &Session| {
        let second = if awaiting(&s.state) {
            if s.question.is_empty() { "waiting".to_string() } else { trim_line(&s.question, 18) }
        } else if s.state == "processing" {
            if s.tool.is_empty() { "working".into() } else { s.tool.clone() }
        } else {
            s.state.clone()
        };
        format!("{}\n{}", trim_line(&s.name, 14), second)
    };
    for i in 0..SLOTS {
        let p = format!("agentdeck:slot:{i}:");
        match m.sessions.get(i) {
            Some(s) => {
                out.insert(format!("{p}state"), if s.alive { s.state.clone() } else { "disconnected".into() });
                out.insert(format!("{p}name"), s.name.clone());
                out.insert(format!("{p}line"), line_of(s));
                out.insert(format!("{p}tool"), s.tool.clone());
                out.insert(format!("{p}question"), s.question.clone());
            }
            None => {
                out.insert(format!("{p}state"), "empty".into());
                out.insert(format!("{p}name"), String::new());
                out.insert(format!("{p}line"), String::new());
                out.insert(format!("{p}tool"), String::new());
                out.insert(format!("{p}question"), String::new());
            }
        }
    }
    let sel = m.sessions.iter().position(|s| s.id == m.selected);
    let empty = Session::default();
    let s = sel.map(|i| &m.sessions[i]).unwrap_or(&empty);
    out.insert("agentdeck:sel:slot".into(), sel.map(|i| (i + 1).to_string()).unwrap_or_default());
    out.insert("agentdeck:sel:state".into(), if sel.is_none() { "empty".into() } else if s.alive { s.state.clone() } else { "disconnected".into() });
    out.insert("agentdeck:sel:name".into(), s.name.clone());
    out.insert("agentdeck:sel:line".into(), if sel.is_none() { String::new() } else { line_of(s) });
    out.insert("agentdeck:sel:question".into(), s.question.clone());
    out.insert("agentdeck:sel:tool".into(), s.tool.clone());
    out.insert("agentdeck:sel:model".into(), s.model.clone());
    out.insert("agentdeck:sel:mode".into(), s.mode.clone());
    out.insert("agentdeck:sel:context".into(), s.context.clone());
    for i in 0..6 {
        out.insert(format!("agentdeck:sel:opt:{i}"), s.options.get(i).map(|o| trim_line(&o.1, 24)).unwrap_or_default());
    }
    out.insert("agentdeck:usage:5h".into(), m.usage_5h.clone());
    out.insert("agentdeck:usage:7d".into(), m.usage_7d.clone());
    out.insert("agentdeck:usage:tokens".into(), m.tokens.clone());
    let mut st = states.lock().unwrap();
    for (k, v) in st.iter() {
        if k.starts_with("agentdeck:k") {
            out.insert(k.clone(), v.clone());
        }
    }
    if *st != out {
        *st = out;
        *changed.lock().unwrap() = true;
    }
}

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn send_ws(ws: &mut Ws, v: Value) -> Result<()> {
    ws.send(tungstenite::Message::Text(v.to_string().into())).map_err(|e| anyhow!("send: {e}"))
}

fn ws_loop(port: u16, states: &Arc<Mutex<HashMap<String, String>>>, model: &Arc<Mutex<Model>>, changed: &Arc<Mutex<bool>>, rx: &Arc<Mutex<mpsc::Receiver<Value>>>, log: &Arc<dyn Fn(&str) + Send + Sync>) -> Result<()> {
    let url = format!("ws://127.0.0.1:{port}");
    let (mut ws, _) = tungstenite::connect(&url).map_err(|e| anyhow!("connect {url}: {e}"))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(100)))?;
    }
    send_ws(&mut ws, json!({
        "type": "client_register",
        "clientType": "ulanzi-plugin",
        "clientLabel": "Ulanzi D200X (deckd)",
        "devices": [{ "id": "d200h", "name": "Ulanzi D200X (deckd)", "family": "d200h", "columns": 5, "rows": 3 }]
    }))?;
    send_ws(&mut ws, json!({ "type": "query_usage" }))?;
    log(&format!("agentdeck connected on {port}"));
    {
        let mut m = model.lock().unwrap();
        m.connected = true;
        publish(&m, states, changed);
    }
    let mut last_ping = std::time::Instant::now();
    loop {
        while let Ok(cmd) = rx.lock().unwrap().try_recv() {
            send_ws(&mut ws, cmd)?;
        }
        if last_ping.elapsed() > Duration::from_secs(20) {
            last_ping = std::time::Instant::now();
            ws.send(tungstenite::Message::Ping(vec![].into())).map_err(|e| anyhow!("ping: {e}"))?;
        }
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(anyhow!("read: {e}")),
        };
        let text = match msg {
            tungstenite::Message::Text(t) => t.to_string(),
            tungstenite::Message::Ping(p) => {
                let _ = ws.send(tungstenite::Message::Pong(p));
                continue;
            }
            tungstenite::Message::Close(_) => return Err(anyhow!("closed")),
            _ => continue,
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mut m = model.lock().unwrap();
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "sessions_list" => {
                let rows = v.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                let mut sessions: Vec<Session> = Vec::new();
                for r in &rows {
                    let id = s_of(r, "id");
                    if id.is_empty() || sessions.iter().any(|s| s.id == id) {
                        continue;
                    }
                    let mut name = s_of(r, "displayName");
                    if name.is_empty() {
                        name = s_of(r, "projectName");
                    }
                    let alive = r.get("alive").and_then(|a| a.as_bool()).unwrap_or(true);
                    sessions.push(Session {
                        id,
                        name,
                        state: if alive { short_state(&s_of(r, "state")) } else { "disconnected".into() },
                        alive,
                        observed: s_of(r, "controlMode") == "observed",
                        tool: s_of(r, "currentTool"),
                        question: s_of(r, "question"),
                        options: parse_options(r),
                        request_id: s_of(r, "requestId"),
                        live_answerable: r.get("liveAnswerable").and_then(|a| a.as_bool()).unwrap_or(false),
                        model: s_of(r, "modelName"),
                        mode: s_of(r, "permissionMode"),
                        context: r.get("contextPercent").and_then(|c| c.as_f64()).map(|c| format!("{c:.0}")).unwrap_or_default(),
                    });
                }
                m.sessions = sessions;
                // keep an explicit selection while it exists; otherwise prefer a waiting session,
                // then the daemon's focused one, then the first alive one
                let keep = m.sessions.iter().any(|s| s.id == m.selected);
                if !keep {
                    let pick = m.sessions.iter().find(|s| awaiting(&s.state)).or_else(|| m.sessions.iter().find(|s| s.id == m.focused)).or_else(|| m.sessions.iter().find(|s| s.alive)).map(|s| s.id.clone());
                    m.selected = pick.unwrap_or_default();
                }
                publish(&m, states, changed);
            }
            "state_update" => {
                let focused = s_of(&v, "focusedSessionId");
                let sid = if focused.is_empty() { s_of(&v, "sessionId") } else { focused };
                if !sid.is_empty() {
                    m.focused = sid.clone();
                    m.navigable = v.get("navigable").and_then(|n| n.as_bool()).unwrap_or(false);
                    // managed sessions: the live snapshot carries the prompt details
                    if let Some(s) = m.sessions.iter_mut().find(|s| s.id == sid && !s.observed) {
                        s.state = short_state(&s_of(&v, "state"));
                        s.tool = s_of(&v, "currentTool");
                        if v.get("question").is_some() {
                            s.question = s_of(&v, "question");
                        }
                        if v.get("options").is_some() {
                            s.options = parse_options(&v);
                        }
                    }
                    publish(&m, states, changed);
                }
            }
            "prompt_options" => {
                let focused = s_of(&v, "focusedSessionId");
                let sid = if focused.is_empty() { s_of(&v, "sessionId") } else { focused };
                if let Some(s) = m.sessions.iter_mut().find(|s| s.id == sid) {
                    s.options = parse_options(&v);
                    s.question = s_of(&v, "question");
                    publish(&m, states, changed);
                }
            }
            "usage_update" => {
                let pct = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| format!("{x:.0}")).unwrap_or_default();
                m.usage_5h = pct("fiveHourPercent");
                m.usage_7d = pct("sevenDayPercent");
                let total = v.get("totalTokens").and_then(|x| x.as_f64()).or_else(|| Some(v.get("inputTokens").and_then(|x| x.as_f64()).unwrap_or(0.0) + v.get("outputTokens").and_then(|x| x.as_f64()).unwrap_or(0.0)));
                m.tokens = total.map(|t| if t >= 1_000_000.0 { format!("{:.1}M", t / 1e6) } else if t >= 1000.0 { format!("{:.0}k", t / 1e3) } else { format!("{t:.0}") }).unwrap_or_default();
                publish(&m, states, changed);
            }
            "review_status" | "session_settings" | "voice_state" => {}
            _ => {}
        }
    }
}
