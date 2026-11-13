//! Agent configuration: %APPDATA%\d200x\agent.json (user-only permissions).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HaConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObsConfig {
    #[serde(default)]
    pub url: String, // ws://127.0.0.1:4455
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpotifyConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub refresh_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiscordConfig {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default)]
    pub access_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDeckConfig {
    /// 0 = read ~/.agentdeck/daemon.json, else 9120.
    #[serde(default)]
    pub port: u16,
    /// Animation frames per orbit cycle (0 = default 48; the official plugin uses 24).
    #[serde(default)]
    pub anim_frames: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub agentdeck: AgentDeckConfig,
    #[serde(default)]
    pub homeassistant: HaConfig,
    #[serde(default)]
    pub obs: ObsConfig,
    #[serde(default)]
    pub spotify: SpotifyConfig,
    #[serde(default)]
    pub discord: DiscordConfig,
    #[serde(default = "default_plugins")]
    pub plugins: Vec<String>,
}

fn default_plugins() -> Vec<String> {
    vec!["system".into(), "media".into(), "homeassistant".into(), "obs".into(), "spotify".into(), "discord".into(), "agentdeck".into()]
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("d200x")
}

pub fn config_path() -> PathBuf {
    config_dir().join("agent.json")
}

pub fn log_path() -> PathBuf {
    config_dir().join("agent.log")
}

pub fn load() -> AgentConfig {
    std::fs::read(config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(cfg: &AgentConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_path(), serde_json::to_vec_pretty(cfg).unwrap_or_default())
}

/// Read-modify-write (used by plugins to persist refreshed tokens).
pub fn update(f: impl FnOnce(&mut AgentConfig)) -> std::io::Result<()> {
    let mut c = load();
    f(&mut c);
    save(&c)
}
