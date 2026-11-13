//! Configurator-side helpers for plugins: OBS scene/input listing and Spotify login.

use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Ask OBS (obs-websocket v5) for scenes and inputs.
pub fn obs_lists(url: &str, password: &str) -> Result<(Vec<String>, Vec<String>)> {
    let url = if url.starts_with("ws") { url.to_string() } else { format!("ws://{url}") };
    let (mut ws, _) = tungstenite::connect(&url).map_err(|e| anyhow!("connect {url}: {e}"))?;
    let mut scenes = Vec::new();
    let mut inputs = Vec::new();
    let b64 = base64::engine::general_purpose::STANDARD;
    loop {
        let msg = ws.read().map_err(|e| anyhow!("{e}"))?;
        let text = match msg {
            tungstenite::Message::Text(t) => t.to_string(),
            tungstenite::Message::Close(_) => return Err(anyhow!("closed")),
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text)?;
        let d = v.get("d").cloned().unwrap_or(json!({}));
        match v.get("op").and_then(|o| o.as_u64()).unwrap_or(99) {
            0 => {
                let mut ident = json!({ "op": 1, "d": { "rpcVersion": 1 } });
                if let Some(auth) = d.get("authentication") {
                    let salt = auth.get("salt").and_then(|s| s.as_str()).unwrap_or("");
                    let challenge = auth.get("challenge").and_then(|s| s.as_str()).unwrap_or("");
                    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
                    ident["d"]["authentication"] = json!(b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes())));
                }
                ws.send(tungstenite::Message::Text(ident.to_string().into()))?;
            }
            2 => {
                ws.send(tungstenite::Message::Text(json!({ "op": 6, "d": { "requestType": "GetSceneList", "requestId": "s", "requestData": {} } }).to_string().into()))?;
                ws.send(tungstenite::Message::Text(json!({ "op": 6, "d": { "requestType": "GetInputList", "requestId": "i", "requestData": {} } }).to_string().into()))?;
            }
            7 => {
                let rd = d.get("responseData").cloned().unwrap_or(json!({}));
                match d.get("requestId").and_then(|s| s.as_str()).unwrap_or("") {
                    "s" => scenes = rd.get("scenes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.get("sceneName").and_then(|n| n.as_str()).map(str::to_string)).collect()).unwrap_or_default(),
                    "i" => {
                        inputs = rd.get("inputs").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|s| s.get("inputName").and_then(|n| n.as_str()).map(str::to_string)).collect()).unwrap_or_default();
                        return Ok((scenes, inputs));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

pub const SPOTIFY_SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read user-library-modify playlist-read-private";
pub const SPOTIFY_REDIRECT: &str = "http://127.0.0.1:8899/callback";

fn random_string(n: usize) -> String {
    let mut h = Sha256::new();
    h.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0).to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let mut out = String::new();
    let mut seed = h.finalize().to_vec();
    while out.len() < n {
        seed = Sha256::digest(&seed).to_vec();
        for b in &seed {
            let c = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"[(*b as usize) % 66] as char;
            out.push(c);
            if out.len() >= n {
                break;
            }
        }
    }
    out
}

/// Bumped by every login; a login still waiting for its callback gives up when it changes.
static LOGIN_ATTEMPT: AtomicU32 = AtomicU32::new(0);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

/// PKCE login: opens the browser, waits for the callback on 127.0.0.1:8899, exchanges the code.
/// Returns the refresh token, None when a newer login replaced this one. Blocks up to 3 minutes;
/// run on a thread.
pub fn spotify_login(client_id: &str) -> Result<Option<String>> {
    if client_id.trim().is_empty() {
        return Err(anyhow!("enter your Spotify Client ID first"));
    }
    let attempt = LOGIN_ATTEMPT.fetch_add(1, Ordering::SeqCst) + 1;
    let verifier = random_string(64);
    let state = random_string(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let listener = bind_callback()?;
    listener.set_nonblocking(true)?;
    let url = format!(
        "https://accounts.spotify.com/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge_method=S256&code_challenge={}&state={}",
        client_id,
        urlencode(SPOTIFY_REDIRECT),
        urlencode(SPOTIFY_SCOPES),
        challenge,
        state
    );
    open::that_detached(&url).map_err(|e| anyhow!("open browser: {e}"))?;
    let Some(code) = wait_for_callback(&listener, &state, attempt)? else { return Ok(None) };
    let resp: Value = ureq::post("https://accounts.spotify.com/api/token")
        .send_form(&[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", SPOTIFY_REDIRECT), ("client_id", client_id), ("code_verifier", &verifier)])
        .map_err(|e| anyhow!("token exchange: {e}"))?
        .into_json()?;
    resp.get("refresh_token").and_then(|r| r.as_str()).map(|t| Some(t.to_string())).ok_or_else(|| anyhow!("no refresh token: {resp}"))
}

/// The previous login may hold the port until it sees it was replaced (one 50 ms poll).
fn bind_callback() -> Result<TcpListener> {
    let mut last = None;
    for _ in 0..20 {
        match TcpListener::bind("127.0.0.1:8899") {
            Ok(l) => return Ok(l),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(anyhow!("cannot listen on 127.0.0.1:8899: {}", last.map(|e| e.to_string()).unwrap_or_default()))
}

/// Serve the port until the callback for this login arrives. Browsers also open idle connections
/// and ask for /favicon.ico, and an old tab may still redirect here: none of that ends the wait.
fn wait_for_callback(listener: &TcpListener, state: &str, attempt: u32) -> Result<Option<String>> {
    let deadline = Instant::now() + CALLBACK_TIMEOUT;
    loop {
        if LOGIN_ATTEMPT.load(Ordering::SeqCst) != attempt {
            return Ok(None);
        }
        if Instant::now() > deadline {
            return Err(anyhow!("no redirect from the browser within 3 minutes (is {SPOTIFY_REDIRECT} on the app's Redirect URIs?)"));
        }
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        match parse_callback(req.lines().next().unwrap_or(""), state) {
            Some(Ok(code)) => {
                respond(&mut stream, "200 OK", "<h2>Spotify connected.</h2>You can close this tab and return to the D200X Configurator.");
                return Ok(Some(code));
            }
            Some(Err(e)) => {
                respond(&mut stream, "200 OK", "<h2>Login failed or denied.</h2>");
                return Err(anyhow!("Spotify refused: {e}"));
            }
            None => respond(&mut stream, "404 Not Found", ""),
        }
    }
}

/// Read a request line: Some(Ok(code)) or Some(Err(reason)) for this login's callback, None for anything else.
fn parse_callback(request_line: &str, state: &str) -> Option<Result<String, String>> {
    let target = request_line.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?')?;
    if path != "/callback" {
        return None;
    }
    let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=')).map(|v| v.split('#').next().unwrap_or(v).to_string());
    if param("state").as_deref() != Some(state) {
        return None;
    }
    match (param("code"), param("error")) {
        (Some(code), _) => Some(Ok(code)),
        (None, e) => Some(Err(e.unwrap_or_else(|| "no authorization code".into()))),
    }
}

fn respond(stream: &mut TcpStream, status: &str, html: &str) {
    let body = format!("<html><body style='font-family:sans-serif'>{html}</body></html>");
    let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes());
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_with_code() {
        assert_eq!(parse_callback("GET /callback?code=abc&state=s1 HTTP/1.1", "s1"), Some(Ok("abc".into())));
        assert_eq!(parse_callback("GET /callback?state=s1&code=abc HTTP/1.1", "s1"), Some(Ok("abc".into())));
    }

    #[test]
    fn callback_denied() {
        assert_eq!(parse_callback("GET /callback?error=access_denied&state=s1 HTTP/1.1", "s1"), Some(Err("access_denied".into())));
    }

    #[test]
    fn other_requests_keep_waiting() {
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1", "s1"), None);
        assert_eq!(parse_callback("", "s1"), None);
        assert_eq!(parse_callback("GET /callback?code=abc&state=old HTTP/1.1", "s1"), None);
        assert_eq!(parse_callback("GET /callback?code=abc HTTP/1.1", "s1"), None);
    }

    #[test]
    fn listener_survives_noise_then_takes_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let attempt = LOGIN_ATTEMPT.fetch_add(1, Ordering::SeqCst) + 1;
        let client = std::thread::spawn(move || {
            let _idle = TcpStream::connect(addr).unwrap(); // browser preconnect: never sends anything
            let mut favicon = TcpStream::connect(addr).unwrap();
            favicon.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut real = TcpStream::connect(addr).unwrap();
            real.write_all(b"GET /callback?code=abc&state=s1 HTTP/1.1\r\n\r\n").unwrap();
            let mut page = String::new();
            let _ = real.read_to_string(&mut page);
            page
        });
        assert_eq!(wait_for_callback(&listener, "s1", attempt).unwrap(), Some("abc".to_string()));
        assert!(client.join().unwrap().contains("Spotify connected"));
    }
}
