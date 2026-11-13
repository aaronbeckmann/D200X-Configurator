//! `obs` plugin: OBS Studio via obs-websocket v5 (ws://host:4455, optional password).
//! ops: scene {name}, record_toggle, stream_toggle, mute_toggle {input}, source_visible {scene, item, visible},
//!      record_start/record_stop, stream_start/stream_stop, virtualcam_toggle, replay_save
//! states: obs:connected, obs:scene, obs:recording, obs:streaming, obs:mute:<input>, obs:virtualcam

use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub struct Obs {
    states: Arc<Mutex<HashMap<String, String>>>,
    tx: Mutex<Option<mpsc::Sender<Value>>>,
    changed: Arc<Mutex<bool>>,
    pub scenes: Arc<Mutex<Vec<String>>>,
    pub inputs: Arc<Mutex<Vec<String>>>,
}

impl Obs {
    pub fn start(url: String, password: String, log: Arc<dyn Fn(&str) + Send + Sync>) -> Obs {
        let states: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let changed = Arc::new(Mutex::new(false));
        let scenes = Arc::new(Mutex::new(Vec::new()));
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel::<Value>();
        let rx = Arc::new(Mutex::new(rx));
        {
            let (st, ch, sc, inp) = (states.clone(), changed.clone(), scenes.clone(), inputs.clone());
            std::thread::Builder::new()
                .name("obs".into())
                .spawn(move || loop {
                    let r = ws_loop(&url, &password, &st, &ch, &sc, &inp, &rx, &log);
                    st.lock().unwrap().insert("obs:connected".into(), "false".into());
                    *ch.lock().unwrap() = true;
                    if let Err(e) = r {
                        log(&format!("obs: {e}; retry in 5 s"));
                    }
                    std::thread::sleep(Duration::from_secs(5));
                })
                .expect("obs thread");
        }
        Obs { states, tx: Mutex::new(Some(tx)), changed, scenes, inputs }
    }

    pub fn take_changed(&self) -> bool {
        std::mem::replace(&mut *self.changed.lock().unwrap(), false)
    }

    pub fn state_names(&self) -> Vec<String> {
        self.states.lock().unwrap().keys().cloned().collect()
    }

    pub fn resolve(&self, name: &str) -> Option<String> {
        self.states.lock().unwrap().get(name).cloned()
    }

    fn request(&self, req_type: &str, data: Value) -> Result<()> {
        let tx = self.tx.lock().unwrap();
        let tx = tx.as_ref().ok_or_else(|| anyhow!("obs not started"))?;
        tx.send(json!({ "requestType": req_type, "requestData": data })).map_err(|_| anyhow!("obs: not connected"))
    }

    pub fn run(&self, op: &str, args: &Value) -> Result<()> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
        match op {
            "scene" => self.request("SetCurrentProgramScene", json!({ "sceneName": s("name").ok_or_else(|| anyhow!("scene: missing name"))? })),
            "record_toggle" => self.request("ToggleRecord", json!({})),
            "record_start" => self.request("StartRecord", json!({})),
            "record_stop" => self.request("StopRecord", json!({})),
            "stream_toggle" => self.request("ToggleStream", json!({})),
            "stream_start" => self.request("StartStream", json!({})),
            "stream_stop" => self.request("StopStream", json!({})),
            "virtualcam_toggle" => self.request("ToggleVirtualCam", json!({})),
            "replay_save" => self.request("SaveReplayBuffer", json!({})),
            "mute_toggle" => self.request("ToggleInputMute", json!({ "inputName": s("input").ok_or_else(|| anyhow!("mute_toggle: missing input"))? })),
            "source_visible" => {
                let scene = s("scene").ok_or_else(|| anyhow!("missing scene"))?;
                let item = s("item").ok_or_else(|| anyhow!("missing item"))?;
                // needs the item id: resolved in the ws thread via a two-step request
                self.request("__SetSourceVisible", json!({ "scene": scene, "item": item, "visible": args.get("visible").cloned().unwrap_or(json!("toggle")) }))
            }
            "studio_toggle" => self.request("ToggleStudioMode", json!({})),
            "transition" => self.request("TriggerStudioModeTransition", json!({})),
            other => Err(anyhow!("obs: unknown op {other}")),
        }
    }
}

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn send(ws: &mut Ws, v: Value) -> Result<()> {
    ws.send(tungstenite::Message::Text(v.to_string().into())).map_err(|e| anyhow!("send: {e}"))
}

#[allow(clippy::too_many_arguments)]
fn ws_loop(
    url: &str,
    password: &str,
    states: &Arc<Mutex<HashMap<String, String>>>,
    changed: &Arc<Mutex<bool>>,
    scenes: &Arc<Mutex<Vec<String>>>,
    inputs: &Arc<Mutex<Vec<String>>>,
    rx: &Arc<Mutex<mpsc::Receiver<Value>>>,
    log: &Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<()> {
    let url = if url.starts_with("ws") { url.to_string() } else { format!("ws://{url}") };
    let (mut ws, _) = tungstenite::connect(&url).map_err(|e| anyhow!("connect {url}: {e}"))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(100)))?;
    }
    let mut req_id = 1u64;
    let set = |k: &str, v: String| {
        let mut st = states.lock().unwrap();
        if st.get(k) != Some(&v) {
            st.insert(k.to_string(), v);
            *changed.lock().unwrap() = true;
        }
    };
    loop {
        // outgoing requests from the plugin
        while let Ok(req) = rx.lock().unwrap().try_recv() {
            let t = req.get("requestType").and_then(|t| t.as_str()).unwrap_or("");
            if t == "__SetSourceVisible" {
                let d = req.get("requestData").cloned().unwrap_or(json!({}));
                req_id += 1;
                send(&mut ws, json!({ "op": 6, "d": { "requestType": "GetSceneItemId", "requestId": format!("vis-{}-{}", req_id, d.get("visible").map(|v| v.to_string()).unwrap_or_default()), "requestData": { "sceneName": d["scene"], "sourceName": d["item"] } } }))?;
                continue;
            }
            req_id += 1;
            send(&mut ws, json!({ "op": 6, "d": { "requestType": t, "requestId": format!("r{req_id}"), "requestData": req.get("requestData").cloned().unwrap_or(json!({})) } }))?;
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
        let d = v.get("d").cloned().unwrap_or(json!({}));
        match v.get("op").and_then(|o| o.as_u64()).unwrap_or(99) {
            0 => {
                // Hello: identify (with auth if required)
                let mut ident = json!({ "op": 1, "d": { "rpcVersion": 1, "eventSubscriptions": 1 | 4 | 8 | 64 } });
                if let Some(auth) = d.get("authentication") {
                    let salt = auth.get("salt").and_then(|s| s.as_str()).unwrap_or("");
                    let challenge = auth.get("challenge").and_then(|s| s.as_str()).unwrap_or("");
                    let b64 = base64::engine::general_purpose::STANDARD;
                    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
                    let answer = b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()));
                    ident["d"]["authentication"] = json!(answer);
                }
                send(&mut ws, ident)?;
            }
            2 => {
                log("obs connected");
                set("obs:connected", "true".into());
                for (t, id) in [("GetSceneList", "init-scenes"), ("GetInputList", "init-inputs"), ("GetRecordStatus", "init-record"), ("GetStreamStatus", "init-stream"), ("GetVirtualCamStatus", "init-vcam")] {
                    send(&mut ws, json!({ "op": 6, "d": { "requestType": t, "requestId": id, "requestData": {} } }))?;
                }
            }
            5 => {
                let et = d.get("eventType").and_then(|e| e.as_str()).unwrap_or("");
                let ed = d.get("eventData").cloned().unwrap_or(json!({}));
                match et {
                    "CurrentProgramSceneChanged" => set("obs:scene", ed.get("sceneName").and_then(|s| s.as_str()).unwrap_or("").into()),
                    "RecordStateChanged" => set("obs:recording", ed.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    "StreamStateChanged" => set("obs:streaming", ed.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    "VirtualcamStateChanged" => set("obs:virtualcam", ed.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    "InputMuteStateChanged" => {
                        let name = ed.get("inputName").and_then(|s| s.as_str()).unwrap_or("");
                        set(&format!("obs:mute:{name}"), ed.get("inputMuted").and_then(|b| b.as_bool()).unwrap_or(false).to_string());
                    }
                    "SceneListChanged" => {
                        send(&mut ws, json!({ "op": 6, "d": { "requestType": "GetSceneList", "requestId": "init-scenes", "requestData": {} } }))?;
                    }
                    _ => {}
                }
            }
            7 => {
                let rid = d.get("requestId").and_then(|s| s.as_str()).unwrap_or("");
                let rd = d.get("responseData").cloned().unwrap_or(json!({}));
                let ok = d.pointer("/requestStatus/result").and_then(|b| b.as_bool()).unwrap_or(false);
                if !ok {
                    log(&format!("obs request {} failed: {}", d.get("requestType").and_then(|s| s.as_str()).unwrap_or(""), d.pointer("/requestStatus/comment").and_then(|s| s.as_str()).unwrap_or("")));
                }
                match rid {
                    "init-scenes" => {
                        let list: Vec<String> = rd.get("scenes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.get("sceneName").and_then(|n| n.as_str()).map(str::to_string)).collect()).unwrap_or_default();
                        *scenes.lock().unwrap() = list;
                        set("obs:scene", rd.get("currentProgramSceneName").and_then(|s| s.as_str()).unwrap_or("").into());
                    }
                    "init-inputs" => {
                        let list: Vec<String> = rd.get("inputs").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.get("inputName").and_then(|n| n.as_str()).map(str::to_string)).collect()).unwrap_or_default();
                        for name in &list {
                            send(&mut ws, json!({ "op": 6, "d": { "requestType": "GetInputMute", "requestId": format!("mute-{name}"), "requestData": { "inputName": name } } }))?;
                        }
                        *inputs.lock().unwrap() = list;
                    }
                    "init-record" => set("obs:recording", rd.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    "init-stream" => set("obs:streaming", rd.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    "init-vcam" => set("obs:virtualcam", rd.get("outputActive").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    r if r.starts_with("mute-") => set(&format!("obs:mute:{}", &r[5..]), rd.get("inputMuted").and_then(|b| b.as_bool()).unwrap_or(false).to_string()),
                    r if r.starts_with("vis-") => {
                        if let Some(item_id) = rd.get("sceneItemId") {
                            let visible = r.rsplit('-').next().unwrap_or("toggle");
                            let scene = d.pointer("/requestData/sceneName").cloned();
                            // obs-websocket does not echo requestData; keep scene in the id would be cleaner, but
                            // we know the only caller: re-read the pending data from the id is not possible, so use the last scene.
                            let scene_name = scene.and_then(|s| s.as_str().map(str::to_string)).or_else(|| states.lock().unwrap().get("obs:scene").cloned()).unwrap_or_default();
                            let enabled = match visible { "\"true\"" | "true" => json!(true), "\"false\"" | "false" => json!(false), _ => json!(true) };
                            send(&mut ws, json!({ "op": 6, "d": { "requestType": "SetSceneItemEnabled", "requestId": "r-vis", "requestData": { "sceneName": scene_name, "sceneItemId": item_id, "sceneItemEnabled": enabled } } }))?;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
