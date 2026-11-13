//! Home Assistant plugin.
//!
//! State subscription: WebSocket API on its own thread (auth with the
//! long-lived token, `subscribe_events` for `state_changed`, `get_states`).
//! Service calls: REST `POST /api/services/<domain>/<service>` (simple and
//! synchronous).
//!
//! Ops: `toggle` / `turn_on` / `turn_off` (any entity; input_boolean and automation use their
//! own services, so a toggle on an automation enables/disables it), `trigger` (run an
//! automation now), `call_service`, `scene`, `script`, knob ops `brightness`, `volume`,
//! `temperature`, `number`, `select_next`.
//!
//! State names the deck can ask for:
//!   `ha:<entity_id>`            the raw state ("on", "off", "23.5", "playing")
//!   `ha:<entity_id>:text`       formatted: state plus unit, e.g. "23.5 °C"
//!   `ha:<entity_id>:attr:<a>`   an attribute as string
//!   `ha:<entity_id>:name`       friendly name

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Entity {
    pub state: String,
    pub attributes: Value,
}

pub struct Ha {
    url: String,
    token: String,
    entities: Arc<Mutex<HashMap<String, Entity>>>,
    changes_rx: Mutex<mpsc::Receiver<String>>,
    pub connected: Arc<Mutex<bool>>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

fn ws_url(http: &str) -> String {
    let base = http.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{base}")
    };
    format!("{ws}/api/websocket")
}

impl Ha {
    pub fn start(url: String, token: String, log: Arc<dyn Fn(&str) + Send + Sync>) -> Ha {
        let entities: Arc<Mutex<HashMap<String, Entity>>> = Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(Mutex::new(false));
        let (tx, rx) = mpsc::channel::<String>();
        {
            let (url2, token2, ents, conn, log2) = (url.clone(), token.clone(), entities.clone(), connected.clone(), log.clone());
            std::thread::Builder::new()
                .name("ha-ws".into())
                .spawn(move || loop {
                    *conn.lock().unwrap() = false;
                    if let Err(e) = ws_loop(&url2, &token2, &ents, &tx, &conn, &log2) {
                        log2(&format!("ha websocket: {e}; reconnecting in 5 s"));
                    }
                    *conn.lock().unwrap() = false;
                    std::thread::sleep(Duration::from_secs(5));
                })
                .expect("ha thread");
        }
        Ha { url, token, entities, changes_rx: Mutex::new(rx), connected, log }
    }

    /// Entities whose state changed since the last call.
    pub fn drain_changes(&self) -> Vec<String> {
        let mut v = Vec::new();
        while let Ok(e) = self.changes_rx.lock().unwrap().try_recv() {
            v.push(e);
        }
        v.sort();
        v.dedup();
        v
    }

    pub fn entity(&self, id: &str) -> Option<Entity> {
        self.entities.lock().unwrap().get(id).cloned()
    }

    /// Resolve one deck state name ("ha:...") to a value string.
    pub fn resolve(&self, name: &str) -> Option<String> {
        let rest = name.strip_prefix("ha:")?;
        let mut parts = rest.splitn(3, ':');
        let entity_id = parts.next()?;
        let kind = parts.next();
        let e = self.entity(entity_id)?;
        Some(match kind {
            None => e.state.clone(),
            Some("text") => {
                let unit = e.attributes.get("unit_of_measurement").and_then(|u| u.as_str()).unwrap_or("");
                if unit.is_empty() {
                    e.state.clone()
                } else {
                    format!("{} {}", e.state, unit)
                }
            }
            Some("name") => e.attributes.get("friendly_name").and_then(|n| n.as_str()).unwrap_or(entity_id).to_string(),
            Some("attr") => {
                let a = parts.next()?;
                match e.attributes.get(a)? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    other => other.to_string(),
                }
            }
            Some(_) => return None,
        })
    }

    /// Every state name the entities offer right now.
    pub fn state_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, e) in self.entities.lock().unwrap().iter() {
            out.extend([format!("ha:{id}"), format!("ha:{id}:text"), format!("ha:{id}:name")]);
            if let Some(attrs) = e.attributes.as_object() {
                out.extend(attrs.keys().map(|a| format!("ha:{id}:attr:{a}")));
            }
        }
        out
    }

    /// Entity id referenced by a deck state name.
    pub fn entity_of(name: &str) -> Option<&str> {
        name.strip_prefix("ha:").and_then(|r| r.split(':').next())
    }

    fn call_service(&self, domain: &str, service: &str, data: Value) -> Result<()> {
        let url = format!("{}/api/services/{}/{}", self.url.trim_end_matches('/'), domain, service);
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(10))
            .send_json(data)
            .map_err(|e| anyhow!("{domain}.{service}: {e}"))?;
        if resp.status() >= 300 {
            return Err(anyhow!("{domain}.{service}: HTTP {}", resp.status()));
        }
        Ok(())
    }

    /// Run a deck host action. `delta` is the knob step for turn events.
    pub fn run(&self, op: &str, args: &Value, event: &str, delta: i32) -> Result<()> {
        let entity = args.get("entity_id").and_then(|v| v.as_str());
        let domain_of = |e: &str| e.split('.').next().unwrap_or("homeassistant").to_string();
        match op {
            "toggle" | "turn_on" | "turn_off" => {
                let e = entity.ok_or_else(|| anyhow!("{op}: missing entity_id"))?;
                let mut data = json!({ "entity_id": e });
                if let Some(extra) = args.get("data").and_then(|d| d.as_object()) {
                    for (k, v) in extra {
                        data[k] = v.clone();
                    }
                }
                // input_boolean / automation (enable-disable) have their own services; everything else
                // goes through the generic homeassistant.* services.
                let domain = match domain_of(e).as_str() {
                    d @ ("input_boolean" | "automation") => d.to_string(),
                    _ => "homeassistant".to_string(),
                };
                self.call_service(&domain, op, data)
            }
            "trigger" => {
                // run an automation's actions now (skips its conditions unless skip_condition is false)
                let e = entity.ok_or_else(|| anyhow!("trigger: missing entity_id"))?;
                let skip = args.get("skip_condition").and_then(|v| v.as_bool()).unwrap_or(true);
                self.call_service("automation", "trigger", json!({ "entity_id": e, "skip_condition": skip }))
            }
            "call_service" => {
                let domain = args.get("domain").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("call_service: missing domain"))?;
                let service = args.get("service").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("call_service: missing service"))?;
                let mut data = args.get("data").cloned().unwrap_or_else(|| json!({}));
                if let Some(e) = entity {
                    data["entity_id"] = json!(e);
                }
                self.call_service(domain, service, data)
            }
            "scene" => {
                let e = entity.ok_or_else(|| anyhow!("scene: missing entity_id"))?;
                self.call_service("scene", "turn_on", json!({ "entity_id": e }))
            }
            "script" => {
                let e = entity.ok_or_else(|| anyhow!("script: missing entity_id"))?;
                self.call_service("script", "turn_on", json!({ "entity_id": e }))
            }
            "brightness" => {
                // knob: step brightness by `step` percent per detent; press toggles
                let e = entity.ok_or_else(|| anyhow!("brightness: missing entity_id"))?;
                if event == "press" {
                    return self.call_service("light", "toggle", json!({ "entity_id": e }));
                }
                let step = args.get("step").and_then(|v| v.as_i64()).unwrap_or(10) as i32;
                self.call_service("light", "turn_on", json!({ "entity_id": e, "brightness_step_pct": step * delta.signum() }))
            }
            "volume" => {
                let e = entity.ok_or_else(|| anyhow!("volume: missing entity_id"))?;
                if event == "press" {
                    return self.call_service("media_player", "media_play_pause", json!({ "entity_id": e }));
                }
                let svc = if delta > 0 { "volume_up" } else { "volume_down" };
                self.call_service("media_player", svc, json!({ "entity_id": e }))
            }
            "media" => {
                let e = entity.ok_or_else(|| anyhow!("media: missing entity_id"))?;
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("media_play_pause");
                self.call_service("media_player", cmd, json!({ "entity_id": e }))
            }
            "temperature" => {
                let e = entity.ok_or_else(|| anyhow!("temperature: missing entity_id"))?;
                let step = args.get("step").and_then(|v| v.as_f64()).unwrap_or(0.5);
                let cur = self.entity(e).and_then(|en| en.attributes.get("temperature").and_then(|t| t.as_f64())).ok_or_else(|| anyhow!("no current temperature"))?;
                let t = cur + step * delta.signum() as f64;
                self.call_service("climate", "set_temperature", json!({ "entity_id": e, "temperature": t }))
            }
            "number" => {
                let e = entity.ok_or_else(|| anyhow!("number: missing entity_id"))?;
                let step = args.get("step").and_then(|v| v.as_f64());
                let en = self.entity(e).ok_or_else(|| anyhow!("unknown entity"))?;
                let cur: f64 = en.state.parse().unwrap_or(0.0);
                let step = step.or_else(|| en.attributes.get("step").and_then(|s| s.as_f64())).unwrap_or(1.0);
                self.call_service(&domain_of(e), "set_value", json!({ "entity_id": e, "value": cur + step * delta.signum() as f64 }))
            }
            "select_next" => {
                let e = entity.ok_or_else(|| anyhow!("select_next: missing entity_id"))?;
                let svc = if delta >= 0 { "select_next" } else { "select_previous" };
                self.call_service(&domain_of(e), svc, json!({ "entity_id": e, "cycle": true }))
            }
            other => Err(anyhow!("homeassistant: unknown op {other}")),
        }
    }

    pub fn log(&self, s: &str) {
        (self.log)(s)
    }
}

fn ws_loop(
    url: &str,
    token: &str,
    entities: &Arc<Mutex<HashMap<String, Entity>>>,
    tx: &mpsc::Sender<String>,
    connected: &Arc<Mutex<bool>>,
    log: &Arc<dyn Fn(&str) + Send + Sync>,
) -> Result<()> {
    let (mut ws, _) = tungstenite::connect(ws_url(url)).context("connect")?;
    let mut next_id = 1u64;
    let mut send = |ws: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>, v: Value| -> Result<()> {
        ws.send(tungstenite::Message::Text(v.to_string().into())).map_err(|e| anyhow!("send: {e}"))
    };
    loop {
        let msg = ws.read().map_err(|e| anyhow!("read: {e}"))?;
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
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "auth_required" => send(&mut ws, json!({ "type": "auth", "access_token": token }))?,
            "auth_invalid" => return Err(anyhow!("auth invalid: {}", v.get("message").and_then(|m| m.as_str()).unwrap_or(""))),
            "auth_ok" => {
                log(&format!("ha connected: {}", v.get("ha_version").and_then(|h| h.as_str()).unwrap_or("?")));
                send(&mut ws, json!({ "id": next_id, "type": "subscribe_events", "event_type": "state_changed" }))?;
                next_id += 1;
                send(&mut ws, json!({ "id": next_id, "type": "get_states" }))?;
                next_id += 1;
            }
            "result" => {
                if let Some(arr) = v.get("result").and_then(|r| r.as_array()) {
                    let mut map = entities.lock().unwrap();
                    for s in arr {
                        if let (Some(id), Some(state)) = (s.get("entity_id").and_then(|i| i.as_str()), s.get("state").and_then(|st| st.as_str())) {
                            map.insert(id.to_string(), Entity { state: state.to_string(), attributes: s.get("attributes").cloned().unwrap_or(json!({})) });
                            let _ = tx.send(id.to_string());
                        }
                    }
                    log(&format!("ha: {} entities", map.len()));
                    *connected.lock().unwrap() = true;
                } else if v.get("success").and_then(|s| s.as_bool()) == Some(false) {
                    log(&format!("ha error: {}", v.get("error").map(|e| e.to_string()).unwrap_or_default()));
                }
            }
            "event" => {
                if let Some(new) = v.pointer("/event/data/new_state") {
                    if let (Some(id), Some(state)) = (new.get("entity_id").and_then(|i| i.as_str()), new.get("state").and_then(|st| st.as_str())) {
                        entities.lock().unwrap().insert(id.to_string(), Entity { state: state.to_string(), attributes: new.get("attributes").cloned().unwrap_or(json!({})) });
                        let _ = tx.send(id.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}
