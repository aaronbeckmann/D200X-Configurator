//! `discord` plugin: Discord RPC over the local IPC pipe (\\.\pipe\discord-ipc-N).
//! Needs a Discord application (client id + secret) created by the user at
//! https://discord.com/developers; the first connection asks for approval in
//! Discord and stores the access token in agent.json.
//! ops: mute_toggle, deafen_toggle, mute {on}, deafen {on}, ptt_toggle
//! states: discord:connected, discord:mute (also true while deafened), discord:deaf, discord:ptt

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

pub struct Discord {
    states: Arc<Mutex<HashMap<String, String>>>,
    changed: Arc<Mutex<bool>>,
    tx: Mutex<Option<mpsc::Sender<Value>>>,
}

impl Discord {
    pub fn start(client_id: String, client_secret: String, access_token: String, log: Arc<dyn Fn(&str) + Send + Sync>) -> Discord {
        let states: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let changed = Arc::new(Mutex::new(false));
        let (tx, rx) = mpsc::channel::<Value>();
        let rx = Arc::new(Mutex::new(rx));
        {
            let (st, ch) = (states.clone(), changed.clone());
            std::thread::Builder::new()
                .name("discord".into())
                .spawn(move || {
                    let mut token = access_token;
                    loop {
                        let r = ipc_loop(&client_id, &client_secret, &mut token, &st, &ch, &rx, &log);
                        st.lock().unwrap().insert("discord:connected".into(), "false".into());
                        *ch.lock().unwrap() = true;
                        if let Err(e) = r {
                            log(&format!("discord: {e}; retry in 10 s"));
                        }
                        std::thread::sleep(Duration::from_secs(10));
                    }
                })
                .expect("discord thread");
        }
        Discord { states, changed, tx: Mutex::new(Some(tx)) }
    }

    pub fn take_changed(&self) -> bool {
        std::mem::replace(&mut *self.changed.lock().unwrap(), false)
    }

    pub fn resolve(&self, name: &str) -> Option<String> {
        self.states.lock().unwrap().get(name).cloned()
    }

    pub fn state_names(&self) -> Vec<String> {
        self.states.lock().unwrap().keys().cloned().collect()
    }

    pub fn run(&self, op: &str, args: &Value) -> Result<()> {
        let cur = |k: &str| self.resolve(k).as_deref() == Some("true");
        let settings = match op {
            // like Discord's own mic button: while deafened it clears both
            "mute_toggle" if cur("discord:deaf") => json!({ "mute": false, "deaf": false }),
            "mute_toggle" => json!({ "mute": !cur("discord:mute") }),
            "deafen_toggle" => json!({ "deaf": !cur("discord:deaf") }),
            "mute" => json!({ "mute": args.get("on").and_then(|b| b.as_bool()).unwrap_or(true) }),
            "deafen" => json!({ "deaf": args.get("on").and_then(|b| b.as_bool()).unwrap_or(true) }),
            "ptt_toggle" => json!({ "mode": { "type": if cur("discord:ptt") { "VOICE_ACTIVITY" } else { "PUSH_TO_TALK" } } }),
            other => return Err(anyhow!("discord: unknown op {other}")),
        };
        let tx = self.tx.lock().unwrap();
        tx.as_ref().ok_or_else(|| anyhow!("discord not started"))?.send(json!({ "cmd": "SET_VOICE_SETTINGS", "args": settings })).map_err(|_| anyhow!("discord: not connected"))
    }
}

struct Pipe(std::fs::File);

impl Pipe {
    fn open() -> Result<Pipe> {
        for i in 0..10 {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) {
                return Ok(Pipe(f));
            }
        }
        Err(anyhow!("Discord is not running (no IPC pipe)"))
    }
    /// Is a message waiting? Lets the loop send commands without sitting in a blocking read
    /// (a read and a write on the same pipe handle would block each other).
    fn has_data(&self) -> Result<bool> {
        let mut avail = 0u32;
        let ok = unsafe { PeekNamedPipe(self.0.as_raw_handle(), std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(anyhow!("Discord closed the connection"));
        }
        Ok(avail > 0)
    }
    fn send(&mut self, op: u32, v: &Value) -> Result<()> {
        let body = v.to_string();
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(body.as_bytes());
        self.0.write_all(&frame)?;
        Ok(())
    }
    fn recv(&mut self) -> Result<(u32, Value)> {
        let mut head = [0u8; 8];
        self.0.read_exact(&mut head)?;
        let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let mut body = vec![0u8; len];
        self.0.read_exact(&mut body)?;
        Ok((op, serde_json::from_slice(&body).unwrap_or(json!({}))))
    }
}

fn nonce() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{n:x}")
}

fn ipc_loop(
    client_id: &str,
    client_secret: &str,
    token: &mut String,
    states: &Arc<Mutex<HashMap<String, String>>>,
    changed: &Arc<Mutex<bool>>,
    rx: &Arc<Mutex<mpsc::Receiver<Value>>>,
    log: &Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<()> {
    if client_id.is_empty() {
        return Err(anyhow!("no Discord client id configured"));
    }
    let mut pipe = Pipe::open()?;
    pipe.send(0, &json!({ "v": 1, "client_id": client_id }))?;
    let (_, ready) = pipe.recv()?;
    if ready.get("evt").and_then(|e| e.as_str()) != Some("READY") {
        return Err(anyhow!("unexpected handshake reply: {ready}"));
    }
    let set = |k: &str, v: String| {
        let mut st = states.lock().unwrap();
        if st.get(k) != Some(&v) {
            st.insert(k.to_string(), v);
            *changed.lock().unwrap() = true;
        }
    };
    // authenticate; on failure run the authorize flow once (user approves in Discord)
    let mut authenticated = false;
    for attempt in 0..2 {
        if token.is_empty() && attempt == 0 {
            // skip straight to authorize
        } else if !token.is_empty() {
            pipe.send(1, &json!({ "cmd": "AUTHENTICATE", "args": { "access_token": token }, "nonce": nonce() }))?;
            let (_, r) = pipe.recv()?;
            if r.get("evt").and_then(|e| e.as_str()) != Some("ERROR") {
                authenticated = true;
                break;
            }
            log("discord: stored token rejected, re-authorizing");
        }
        if client_secret.is_empty() {
            return Err(anyhow!("no Discord client secret configured (needed once for authorization)"));
        }
        pipe.send(1, &json!({ "cmd": "AUTHORIZE", "args": { "client_id": client_id, "scopes": ["rpc", "rpc.voice.read", "rpc.voice.write"] }, "nonce": nonce() }))?;
        let (_, r) = pipe.recv()?;
        let code = r.pointer("/data/code").and_then(|c| c.as_str()).ok_or_else(|| anyhow!("authorization denied or failed: {r}"))?.to_string();
        let resp: Value = ureq::post("https://discord.com/api/oauth2/token")
            .send_form(&[("client_id", client_id), ("client_secret", client_secret), ("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", "http://127.0.0.1")])
            .map_err(|e| anyhow!("token exchange: {e}"))?
            .into_json()?;
        let t = resp.get("access_token").and_then(|a| a.as_str()).ok_or_else(|| anyhow!("token exchange: {resp}"))?.to_string();
        *token = t.clone();
        let _ = crate::config::update(|c| c.discord.access_token = t.clone());
        log("discord: authorized");
    }
    if !authenticated {
        pipe.send(1, &json!({ "cmd": "AUTHENTICATE", "args": { "access_token": token }, "nonce": nonce() }))?;
        let (_, r) = pipe.recv()?;
        if r.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
            return Err(anyhow!("authenticate failed: {r}"));
        }
    }
    log("discord connected");
    set("discord:connected", "true".into());
    pipe.send(1, &json!({ "cmd": "SUBSCRIBE", "evt": "VOICE_SETTINGS_UPDATE", "args": {}, "nonce": nonce() }))?;
    pipe.send(1, &json!({ "cmd": "GET_VOICE_SETTINGS", "args": {}, "nonce": nonce() }))?;
    let apply = |d: &Value| {
        let deaf = d.get("deaf").and_then(|b| b.as_bool());
        // Discord keeps `mute` false while deafened, but the mic is off then too
        if let Some(m) = d.get("mute").and_then(|b| b.as_bool()) {
            set("discord:mute", (m || deaf == Some(true)).to_string());
        }
        if let Some(m) = deaf {
            set("discord:deaf", m.to_string());
        }
        if let Some(t) = d.pointer("/mode/type").and_then(|s| s.as_str()) {
            set("discord:ptt", (t == "PUSH_TO_TALK").to_string());
        }
    };
    loop {
        // outgoing
        while let Ok(cmd) = rx.lock().unwrap().try_recv() {
            let mut c = cmd;
            c["nonce"] = json!(nonce());
            pipe.send(1, &c)?;
        }
        if !pipe.has_data()? {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        let (_, msg) = pipe.recv()?;
        match (msg.get("cmd").and_then(|c| c.as_str()).unwrap_or(""), msg.get("evt").and_then(|e| e.as_str()).unwrap_or("")) {
            ("GET_VOICE_SETTINGS", _) | ("SET_VOICE_SETTINGS", _) => {
                if let Some(d) = msg.get("data") {
                    apply(d);
                }
            }
            ("DISPATCH", "VOICE_SETTINGS_UPDATE") => {
                if let Some(d) = msg.get("data") {
                    apply(d);
                }
            }
            (_, "ERROR") => log(&format!("discord error: {}", msg.pointer("/data/message").and_then(|s| s.as_str()).unwrap_or(""))),
            _ => {}
        }
    }
}
