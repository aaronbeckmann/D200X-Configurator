//! `spotify` plugin: Spotify Web API with the user's own Client ID (PKCE, no secret).
//! The configurator performs the login and stores `refresh_token` in agent.json.
//! ops: play_pause, next, prev, shuffle_toggle, repeat_cycle, like_toggle, volume {step} (knob),
//!      volume_set {percent}, play {uri} (playlist/album/track URI), seek {delta_ms}
//! states: spotify:connected, spotify:playing, spotify:title, spotify:artist, spotify:album,
//!         spotify:line, spotify:shuffle, spotify:repeat, spotify:liked, spotify:volume, spotify:device

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read user-library-modify playlist-read-private";

#[derive(Default)]
struct Token {
    access: String,
    expires: Option<Instant>,
}

pub struct Spotify {
    client_id: String,
    refresh_token: Mutex<String>,
    token: Mutex<Token>,
    states: Arc<Mutex<HashMap<String, String>>>,
    changed: Arc<Mutex<bool>>,
    track_id: Mutex<String>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Spotify {
    pub fn start(client_id: String, refresh_token: String, log: Arc<dyn Fn(&str) + Send + Sync>) -> Arc<Spotify> {
        let sp = Arc::new(Spotify {
            client_id,
            refresh_token: Mutex::new(refresh_token),
            token: Mutex::new(Token::default()),
            states: Arc::new(Mutex::new(HashMap::new())),
            changed: Arc::new(Mutex::new(false)),
            track_id: Mutex::new(String::new()),
            log,
        });
        let sp2 = sp.clone();
        std::thread::Builder::new()
            .name("spotify".into())
            .spawn(move || loop {
                if let Err(e) = sp2.poll() {
                    sp2.set("spotify:connected", "false".into());
                    (sp2.log)(&format!("spotify: {e}"));
                    std::thread::sleep(Duration::from_secs(10));
                } else {
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("spotify thread");
        sp
    }

    fn set(&self, k: &str, v: String) {
        let mut st = self.states.lock().unwrap();
        if st.get(k) != Some(&v) {
            st.insert(k.to_string(), v);
            *self.changed.lock().unwrap() = true;
        }
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

    fn access_token(&self) -> Result<String> {
        {
            let t = self.token.lock().unwrap();
            if let Some(exp) = t.expires {
                if Instant::now() < exp && !t.access.is_empty() {
                    return Ok(t.access.clone());
                }
            }
        }
        let refresh = self.refresh_token.lock().unwrap().clone();
        if refresh.is_empty() || self.client_id.is_empty() {
            return Err(anyhow!("not logged in (set Client ID and log in from the configurator)"));
        }
        let resp: Value = ureq::post("https://accounts.spotify.com/api/token")
            .send_form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &self.client_id)])
            .map_err(|e| anyhow!("token refresh: {e}"))?
            .into_json()?;
        let access = resp.get("access_token").and_then(|a| a.as_str()).ok_or_else(|| anyhow!("no access token in refresh response"))?.to_string();
        if let Some(new_refresh) = resp.get("refresh_token").and_then(|r| r.as_str()) {
            *self.refresh_token.lock().unwrap() = new_refresh.to_string();
            let _ = crate::config::update(|c| c.spotify.refresh_token = new_refresh.to_string());
        }
        let secs = resp.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(3600);
        *self.token.lock().unwrap() = Token { access: access.clone(), expires: Some(Instant::now() + Duration::from_secs(secs.saturating_sub(60))) };
        Ok(access)
    }

    fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Option<Value>> {
        let token = self.access_token()?;
        let url = format!("https://api.spotify.com/v1{path}");
        let req = match method {
            "GET" => ureq::get(&url),
            "PUT" => ureq::put(&url),
            "POST" => ureq::post(&url),
            "DELETE" => ureq::delete(&url),
            _ => return Err(anyhow!("bad method")),
        }
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(10));
        let resp = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match resp {
            Ok(r) => {
                if r.status() == 204 {
                    return Ok(None);
                }
                let text = r.into_string().unwrap_or_default();
                if text.trim().is_empty() {
                    return Ok(None);
                }
                Ok(serde_json::from_str(&text).ok())
            }
            Err(ureq::Error::Status(404, _)) => Err(anyhow!("no active Spotify device (start playback in Spotify once)")),
            Err(ureq::Error::Status(code, r)) => Err(anyhow!("spotify {method} {path}: HTTP {code} {}", r.into_string().unwrap_or_default())),
            Err(e) => Err(anyhow!("spotify {method} {path}: {e}")),
        }
    }

    fn poll(&self) -> Result<()> {
        let v = self.api("GET", "/me/player", None)?;
        self.set("spotify:connected", "true".into());
        let Some(v) = v else {
            self.set("spotify:playing", "false".into());
            return Ok(());
        };
        let playing = v.get("is_playing").and_then(|b| b.as_bool()).unwrap_or(false);
        self.set("spotify:playing", playing.to_string());
        self.set("spotify:shuffle", v.get("shuffle_state").and_then(|b| b.as_bool()).unwrap_or(false).to_string());
        self.set("spotify:repeat", v.get("repeat_state").and_then(|s| s.as_str()).unwrap_or("off").to_string());
        self.set("spotify:volume", v.pointer("/device/volume_percent").and_then(|n| n.as_u64()).unwrap_or(0).to_string());
        self.set("spotify:device", v.pointer("/device/name").and_then(|s| s.as_str()).unwrap_or("").to_string());
        if let Some(item) = v.get("item") {
            let title = item.get("name").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let artist = item.get("artists").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str())).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let album = item.pointer("/album/name").and_then(|s| s.as_str()).unwrap_or("").to_string();
            self.set("spotify:title", title.clone());
            self.set("spotify:artist", artist.clone());
            self.set("spotify:album", album);
            self.set("spotify:line", if artist.is_empty() { title } else { format!("{artist} - {}", self.resolve("spotify:title").unwrap_or_default()) });
            let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let mut cur = self.track_id.lock().unwrap();
            if *cur != id && !id.is_empty() {
                *cur = id.clone();
                drop(cur);
                if let Ok(Some(liked)) = self.api("GET", &format!("/me/tracks/contains?ids={id}"), None) {
                    let l = liked.as_array().and_then(|a| a.first()).and_then(|b| b.as_bool()).unwrap_or(false);
                    self.set("spotify:liked", l.to_string());
                }
            }
        }
        Ok(())
    }

    pub fn run(&self, op: &str, args: &Value, delta: i32) -> Result<()> {
        match op {
            "play_pause" => {
                let playing = self.resolve("spotify:playing").as_deref() == Some("true");
                self.api("PUT", if playing { "/me/player/pause" } else { "/me/player/play" }, None).map(|_| ())
            }
            "play" => {
                let uri = args.get("uri").and_then(|u| u.as_str());
                let body = uri.map(|u| if u.contains(":track:") { json!({ "uris": [u] }) } else { json!({ "context_uri": u }) });
                self.api("PUT", "/me/player/play", body).map(|_| ())
            }
            "next" => self.api("POST", "/me/player/next", None).map(|_| ()),
            "prev" => self.api("POST", "/me/player/previous", None).map(|_| ()),
            "shuffle_toggle" => {
                let on = self.resolve("spotify:shuffle").as_deref() == Some("true");
                self.api("PUT", &format!("/me/player/shuffle?state={}", !on), None).map(|_| ())
            }
            "repeat_cycle" => {
                let cur = self.resolve("spotify:repeat").unwrap_or_else(|| "off".into());
                let next = match cur.as_str() { "off" => "context", "context" => "track", _ => "off" };
                self.api("PUT", &format!("/me/player/repeat?state={next}"), None).map(|_| ())
            }
            "like_toggle" => {
                let id = self.track_id.lock().unwrap().clone();
                if id.is_empty() {
                    return Err(anyhow!("no current track"));
                }
                let liked = self.resolve("spotify:liked").as_deref() == Some("true");
                self.api(if liked { "DELETE" } else { "PUT" }, &format!("/me/tracks?ids={id}"), None)?;
                self.set("spotify:liked", (!liked).to_string());
                Ok(())
            }
            "volume" => {
                let step = args.get("step").and_then(|s| s.as_i64()).unwrap_or(5) as i32;
                let cur: i32 = self.resolve("spotify:volume").and_then(|v| v.parse().ok()).unwrap_or(50);
                let v = (cur + step * delta.signum()).clamp(0, 100);
                self.api("PUT", &format!("/me/player/volume?volume_percent={v}"), None).map(|_| ())
            }
            "volume_set" => {
                let v = args.get("percent").and_then(|p| p.as_u64()).unwrap_or(50).min(100);
                self.api("PUT", &format!("/me/player/volume?volume_percent={v}"), None).map(|_| ())
            }
            other => Err(anyhow!("spotify: unknown op {other}")),
        }
    }
}
