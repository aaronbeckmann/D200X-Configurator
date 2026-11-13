//! Home Assistant side of the configurator: agent settings, entity list over
//! REST, and key/knob generation with rendered MDI icons.

use anyhow::{anyhow, Result};
use d200x_core::Deck;
use deck_config::{Action, KeyDef, KnobDef, Look};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
    pub url: String,
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
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDeckConfig {
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub anim_frames: u16,
}

pub fn agent_config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("d200x").join("agent.json")
}

pub fn load_agent_config() -> AgentConfig {
    std::fs::read(agent_config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_agent_config(c: &AgentConfig) -> Result<()> {
    let mut c = c.clone();
    // the agent writes these tokens while it runs, so the copy in memory here may be stale
    let disk = load_agent_config();
    if disk.spotify.client_id == c.spotify.client_id {
        c.spotify.refresh_token = disk.spotify.refresh_token;
    }
    if disk.discord.client_id == c.discord.client_id {
        c.discord.access_token = disk.discord.access_token;
    }
    if c.plugins.is_empty() {
        c.plugins = vec!["system".into(), "media".into(), "homeassistant".into(), "obs".into(), "spotify".into(), "discord".into()];
    }
    write_agent_config(&c)
}

/// Store a fresh Spotify login right away, whatever the UI is showing.
pub fn save_spotify_token(token: &str) -> Result<()> {
    let mut c = load_agent_config();
    c.spotify.refresh_token = token.to_string();
    write_agent_config(&c)
}

fn write_agent_config(c: &AgentConfig) -> Result<()> {
    let p = agent_config_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, serde_json::to_vec_pretty(c)?)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: String,
    pub domain: String,
    pub name: String,
    pub state: String,
    pub unit: String,
    pub icon: Option<String>,
    pub device_class: Option<String>,
}

pub fn fetch_entities(cfg: &HaConfig) -> Result<Vec<Entity>> {
    if cfg.url.is_empty() || cfg.token.is_empty() {
        return Err(anyhow!("Home Assistant URL and token are not set (Agent settings)"));
    }
    let url = format!("{}/api/states", cfg.url.trim_end_matches('/'));
    let v: Value = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", cfg.token))
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| anyhow!("{e}"))?
        .into_json()
        .map_err(|e| anyhow!("{e}"))?;
    let mut out: Vec<Entity> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let id = e.get("entity_id")?.as_str()?.to_string();
                    let attrs = e.get("attributes").cloned().unwrap_or(json!({}));
                    Some(Entity {
                        domain: id.split('.').next().unwrap_or("").to_string(),
                        name: attrs.get("friendly_name").and_then(|n| n.as_str()).unwrap_or(&id).to_string(),
                        state: e.get("state").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                        unit: attrs.get("unit_of_measurement").and_then(|u| u.as_str()).unwrap_or("").to_string(),
                        icon: attrs.get("icon").and_then(|i| i.as_str()).map(str::to_string),
                        device_class: attrs.get("device_class").and_then(|d| d.as_str()).map(str::to_string),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Render an MDI icon in a colour and upload it; returns the icon file name.
pub fn upload_mdi(name: &str, color: &str) -> Result<String> {
    let file = deck_icons::mdi_file_name(name, color);
    let png = deck_icons::render_mdi(name, color, deck_icons::DEFAULT_SIZE)?;
    let reply = Deck::open()?.set_icon(&file, &png)?;
    if !reply.contains("\"ok\":true") {
        return Err(anyhow!("deck rejected icon: {reply}"));
    }
    Ok(file)
}

fn short_name(e: &Entity) -> String {
    let n = e.name.trim();
    if n.len() <= 16 {
        n.to_string()
    } else {
        n.split(' ').take(2).collect::<Vec<_>>().join(" ")
    }
}

const ON_COLOR: &str = "#4a3a10";
const ON_ICON_COLOR: &str = "#f5b400";
const OFF_COLOR: &str = "#1c2b3a";
const OFF_ICON_COLOR: &str = "#9aa5b1";
const SENSOR_COLOR: &str = "#15303a";
const SENSOR_ICON_COLOR: &str = "#7fd3ff";

/// A toggle key for a switchable entity (light, switch, fan, input_boolean, ...).
pub fn toggle_key(e: &Entity) -> Result<KeyDef> {
    let base = e.icon.as_deref().map(|i| i.trim_start_matches("mdi:").to_string());
    let on_icon = base.clone().unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), "on").to_string());
    let off_icon = base.unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), "off").to_string());
    let on_file = upload_mdi(&on_icon, ON_ICON_COLOR)?;
    let off_file = upload_mdi(&off_icon, OFF_ICON_COLOR)?;
    let mut looks = BTreeMap::new();
    looks.insert("on".into(), Look { icon: Some(on_file.clone()), color: Some(ON_COLOR.into()), ..Default::default() });
    looks.insert("off".into(), Look { icon: Some(off_file.clone()), color: Some(OFF_COLOR.into()), ..Default::default() });
    let op = match e.domain.as_str() {
        "scene" => ("scene", json!({ "entity_id": e.id })),
        "script" => ("script", json!({ "entity_id": e.id })),
        "button" => ("call_service", json!({ "domain": "button", "service": "press", "entity_id": e.id })),
        "lock" => ("call_service", json!({ "domain": "lock", "service": if e.state == "locked" { "unlock" } else { "lock" }, "entity_id": e.id })),
        _ => ("toggle", json!({ "entity_id": e.id })),
    };
    Ok(KeyDef {
        label: short_name(e),
        icon: Some(off_file),
        color: Some(OFF_COLOR.into()),
        text_color: None,
        action: Action::Host { plugin: "homeassistant".into(), op: op.0.into(), args: op.1, fallback: None },
        state: Some(format!("ha:{}", e.id)),
        looks,
        value: None,
        progress: None,
        tile: None,
        effect: None,
        color_state: None,
        font_size: None,
        align: None,
        valign: None,
        font: None,
    })
}

/// A tile showing the entity's state with unit (sensors, climate current temp...).
pub fn sensor_key(e: &Entity) -> Result<KeyDef> {
    let icon = e.icon.as_deref().map(|i| i.trim_start_matches("mdi:").to_string()).unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), &e.state).to_string());
    let file = upload_mdi(&icon, SENSOR_ICON_COLOR)?;
    Ok(KeyDef {
        label: short_name(e),
        icon: Some(file),
        color: Some(SENSOR_COLOR.into()),
        text_color: None,
        action: Action::None,
        state: None,
        looks: BTreeMap::new(),
        value: Some(format!("ha:{}:text", e.id)),
        progress: None,
        tile: None,
        effect: None,
        color_state: None,
        font_size: None,
        align: None,
        valign: None,
        font: None,
    })
}

/// A knob: light brightness, media volume, climate temperature, number, select.
pub fn knob(e: &Entity) -> KnobDef {
    let (op, press_op, extra) = match e.domain.as_str() {
        "light" => ("brightness", "brightness", json!({ "step": 10 })),
        "media_player" => ("volume", "volume", json!({})),
        "climate" => ("temperature", "temperature", json!({ "step": 0.5 })),
        "number" | "input_number" => ("number", "number", json!({})),
        "select" | "input_select" => ("select_next", "select_next", json!({})),
        _ => ("toggle", "toggle", json!({})),
    };
    let mk = |o: &str| {
        let mut args = json!({ "entity_id": e.id });
        if let Some(m) = extra.as_object() {
            for (k, v) in m {
                args[k] = v.clone();
            }
        }
        Action::Host { plugin: "homeassistant".into(), op: o.into(), args, fallback: None }
    };
    KnobDef { label: short_name(e), press: mk(press_op), left: mk(op), right: mk(op), progress: None }
}

pub fn is_togglable(e: &Entity) -> bool {
    matches!(e.domain.as_str(), "light" | "switch" | "fan" | "input_boolean" | "automation" | "scene" | "script" | "button" | "lock" | "cover" | "media_player" | "humidifier" | "siren" | "vacuum")
}

pub fn is_knobbable(e: &Entity) -> bool {
    matches!(e.domain.as_str(), "light" | "media_player" | "climate" | "number" | "input_number" | "select" | "input_select")
}

/// Toggle key built through the configurator's icon cache (uploaded on save).
pub fn toggle_key_local(app: &mut crate::app::App, e: &Entity) -> Result<KeyDef> {
    let base = e.icon.as_deref().map(|i| i.trim_start_matches("mdi:").to_string());
    let on_icon = base.clone().unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), "on").to_string());
    let off_icon = base.unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), "off").to_string());
    let on_file = app.mdi_icon(&on_icon, ON_ICON_COLOR).ok_or_else(|| anyhow!("icon render failed"))?;
    let off_file = app.mdi_icon(&off_icon, OFF_ICON_COLOR).ok_or_else(|| anyhow!("icon render failed"))?;
    let mut looks = BTreeMap::new();
    looks.insert("on|open|playing|locked|home|heat|cool".into(), Look { icon: Some(on_file.clone()), color: Some(ON_COLOR.into()), ..Default::default() });
    looks.insert("*".into(), Look { icon: Some(off_file.clone()), color: Some(OFF_COLOR.into()), ..Default::default() });
    let op = match e.domain.as_str() {
        "scene" => ("scene", json!({ "entity_id": e.id })),
        "script" => ("script", json!({ "entity_id": e.id })),
        "button" => ("call_service", json!({ "domain": "button", "service": "press", "entity_id": e.id })),
        "lock" => ("call_service", json!({ "domain": "lock", "service": if e.state == "locked" { "unlock" } else { "lock" }, "entity_id": e.id })),
        _ => ("toggle", json!({ "entity_id": e.id })),
    };
    Ok(KeyDef {
        label: short_name(e),
        icon: Some(off_file),
        color: Some(OFF_COLOR.into()),
        text_color: None,
        action: Action::Host { plugin: "homeassistant".into(), op: op.0.into(), args: op.1, fallback: None },
        state: Some(format!("ha:{}", e.id)),
        looks,
        value: None,
        progress: None,
        tile: None,
        effect: None,
        color_state: None,
        font_size: None,
        align: None,
        valign: None,
        font: None,
    })
}

pub fn sensor_key_local(app: &mut crate::app::App, e: &Entity) -> Result<KeyDef> {
    let icon = e.icon.as_deref().map(|i| i.trim_start_matches("mdi:").to_string()).unwrap_or_else(|| deck_icons::default_ha_icon(&e.id, e.device_class.as_deref(), &e.state).to_string());
    let file = app.mdi_icon(&icon, SENSOR_ICON_COLOR).ok_or_else(|| anyhow!("icon render failed"))?;
    Ok(KeyDef {
        label: short_name(e),
        icon: Some(file),
        color: Some(SENSOR_COLOR.into()),
        text_color: None,
        action: Action::None,
        state: None,
        looks: BTreeMap::new(),
        value: Some(format!("ha:{}:text", e.id)),
        progress: None,
        tile: None,
        effect: None,
        color_state: None,
        font_size: None,
        align: None,
        valign: None,
        font: None,
    })
}
