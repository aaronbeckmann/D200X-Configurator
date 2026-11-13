//! What the state explorer knows: the states each plugin offers (names, meaning, possible values)
//! merged with the live snapshots from the PC agent and the ESP32 bridge. Names here follow what
//! the agents really set.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

const AGENT_STATES: &str = "127.0.0.1:8898";

#[derive(Debug, Default, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub states: BTreeMap<String, String>,
    /// Plugins running on the ESP32 bridge (filled by `merge`).
    #[serde(default)]
    pub bridge_plugins: Vec<String>,
}

/// The bridge's states, through the deck. Slow (the console UART runs at 115200): poll rarely.
pub fn fetch_bridge() -> Result<Snapshot, String> {
    let deck = d200x_core::Deck::open().map_err(|e| e.to_string())?;
    let (plugins, states) = d200x_core::bridge::states(&deck).map_err(|e| e.to_string())?;
    Ok(Snapshot { plugins: Vec::new(), states, bridge_plugins: plugins })
}

/// One view of both agents. The PC agent's value wins where both know a state.
pub fn merge(agent: &Snapshot, bridge: Option<&Snapshot>) -> Snapshot {
    let mut out = Snapshot { plugins: agent.plugins.clone(), states: agent.states.clone(), bridge_plugins: Vec::new() };
    if let Some(b) = bridge {
        out.bridge_plugins = b.bridge_plugins.clone();
        for (k, v) in &b.states {
            out.states.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    out
}

/// Bridge plugins the catalogue does not describe (mqtt, net, plugins from the bridge config).
pub fn extra_plugins(snap: &Snapshot) -> Vec<String> {
    snap.bridge_plugins.iter().filter(|p| !PLUGINS.iter().any(|k| k.id == p.as_str())).cloned().collect()
}

/// Live states of a plugin without a catalogue entry: everything under `<name>:`.
pub fn extra_rows(name: &str, snap: &Snapshot) -> Vec<Row> {
    with_prefix(snap, &format!("{name}:")).map(|(k, v)| Row { name: k.clone(), value: Some(v.clone()), note: String::new(), values: Vec::new(), template: false }).collect()
}

pub fn fetch_snapshot() -> Result<Snapshot, String> {
    let addr = AGENT_STATES.parse().map_err(|e| format!("{e}"))?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map_err(|e| format!("{e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| format!("{e}"))?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| format!("{e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("{e}"))
}

pub struct Doc {
    /// `<x>` marks a part that depends on the setup, e.g. "obs:mute:<input>".
    pub name: &'static str,
    pub note: &'static str,
    pub values: &'static [&'static str],
}

pub struct Plugin {
    pub id: &'static str,
    pub title: &'static str,
    pub prefix: &'static str,
    pub docs: &'static [Doc],
}

const fn doc(name: &'static str, note: &'static str, values: &'static [&'static str]) -> Doc {
    Doc { name, note, values }
}

const BOOL: &[&str] = &["true", "false"];
const SESSION: &[&str] = &["empty", "idle", "processing", "awaiting_permission", "awaiting_option", "awaiting_diff", "disconnected"];

pub const HOME_ASSISTANT: usize = 2;

pub const PLUGINS: [Plugin; 7] = [
    Plugin {
        id: "system",
        title: "System",
        prefix: "system:",
        docs: &[
            doc("system:app", "program in the foreground", &[]),
            doc("system:title", "title of the foreground window", &[]),
            doc("system:cpu", "CPU load, percent", &[]),
            doc("system:ram", "memory in use, percent", &[]),
            doc("system:ram_used", "memory in use, GB", &[]),
            doc("system:ram_total", "installed memory, GB", &[]),
            doc("system:disk", "drive C: used, percent", &[]),
            doc("system:disk_free", "drive C: free, GB", &[]),
            doc("system:volume", "master volume, 0 to 100", &[]),
            doc("system:mute", "master volume muted", BOOL),
            doc("system:time", "local time, HH:MM", &[]),
            doc("system:date", "local date, YYYY-MM-DD", &[]),
            doc("system:weekday", "day of the week", &[]),
            doc("system:uptime", "time since Windows started", &[]),
            doc("system:uptime_h", "time since Windows started, hours", &[]),
            doc("system:host", "computer name", &[]),
            doc("system:battery", "battery charge, percent (laptops)", &[]),
            doc("system:charging", "on mains power (laptops)", BOOL),
        ],
    },
    Plugin {
        id: "media",
        title: "Media",
        prefix: "media:",
        docs: &[
            doc("media:status", "Windows media session", &["playing", "paused", "stopped", "none"]),
            doc("media:title", "current title", &[]),
            doc("media:artist", "current artist", &[]),
            doc("media:line", "artist and title in one line", &[]),
            doc("media:app", "app playing the media", &[]),
        ],
    },
    Plugin { id: "homeassistant", title: "Home Assistant", prefix: "ha:", docs: &[] },
    Plugin {
        id: "obs",
        title: "OBS",
        prefix: "obs:",
        docs: &[
            doc("obs:connected", "connected to OBS", BOOL),
            doc("obs:scene", "current program scene", &[]),
            doc("obs:recording", "recording is running", BOOL),
            doc("obs:streaming", "stream is running", BOOL),
            doc("obs:virtualcam", "virtual camera is on", BOOL),
            doc("obs:mute:<input>", "input is muted", BOOL),
        ],
    },
    Plugin {
        id: "spotify",
        title: "Spotify",
        prefix: "spotify:",
        docs: &[
            doc("spotify:connected", "logged in and reachable", BOOL),
            doc("spotify:playing", "playback is running", BOOL),
            doc("spotify:title", "current track", &[]),
            doc("spotify:artist", "current artist", &[]),
            doc("spotify:album", "current album", &[]),
            doc("spotify:line", "artist and title in one line", &[]),
            doc("spotify:shuffle", "shuffle is on", BOOL),
            doc("spotify:repeat", "repeat mode", &["off", "context", "track"]),
            doc("spotify:liked", "current track is liked", BOOL),
            doc("spotify:volume", "volume, 0 to 100", &[]),
            doc("spotify:device", "device playing", &[]),
        ],
    },
    Plugin {
        id: "discord",
        title: "Discord",
        prefix: "discord:",
        docs: &[
            doc("discord:connected", "connected and approved", BOOL),
            doc("discord:mute", "mic is off (also true while deafened)", BOOL),
            doc("discord:deaf", "deafened", BOOL),
            doc("discord:ptt", "push to talk is the voice mode", BOOL),
        ],
    },
    Plugin {
        id: "agentdeck",
        title: "AgentDeck",
        prefix: "agentdeck:",
        docs: &[
            doc("agentdeck:connected", "connected to the AgentDeck daemon", BOOL),
            doc("agentdeck:state", "all sessions together", &["offline", "idle", "processing", "waiting"]),
            doc("agentdeck:count", "alive sessions", &[]),
            doc("agentdeck:waiting", "sessions waiting for an answer", &[]),
            doc("agentdeck:usage:5h", "Claude 5-hour quota, percent", &[]),
            doc("agentdeck:usage:7d", "Claude 7-day quota, percent", &[]),
            doc("agentdeck:usage:tokens", "session tokens", &[]),
            doc("agentdeck:sel:slot", "selected session, counted from 1", &[]),
            doc("agentdeck:sel:state", "selected session", SESSION),
            doc("agentdeck:sel:name", "selected session", &[]),
            doc("agentdeck:sel:line", "selected session, short status", &[]),
            doc("agentdeck:sel:question", "selected session's open question", &[]),
            doc("agentdeck:sel:tool", "selected session's current tool", &[]),
            doc("agentdeck:sel:model", "selected session's model", &[]),
            doc("agentdeck:sel:mode", "selected session's mode", &[]),
            doc("agentdeck:sel:context", "selected session's context", &[]),
            doc("agentdeck:sel:opt:<I>", "label of option I (0 to 5) of the open prompt", &[]),
            doc("agentdeck:slot:<N>:state", "session N (0 to 7)", SESSION),
            doc("agentdeck:slot:<N>:name", "session N (0 to 7)", &[]),
            doc("agentdeck:slot:<N>:line", "session N (0 to 7), short status", &[]),
            doc("agentdeck:slot:<N>:tool", "session N (0 to 7), current tool", &[]),
            doc("agentdeck:slot:<N>:question", "session N (0 to 7), open question", &[]),
        ],
    },
];

pub fn plugin_of(state: &str) -> Option<usize> {
    PLUGINS.iter().position(|p| state.starts_with(p.prefix))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub value: Option<String>,
    pub note: String,
    pub values: Vec<String>,
    /// Still has a `<x>` part, so it cannot be used as it is.
    pub template: bool,
}

fn matches_doc(pattern: &str, name: &str) -> bool {
    let (Some((head, _)), Some((_, tail))) = (pattern.split_once('<'), pattern.split_once('>')) else { return false };
    name.len() > head.len() + tail.len() && name.starts_with(head) && name.ends_with(tail)
}

fn with_prefix<'a>(snap: &'a Snapshot, prefix: &str) -> impl Iterator<Item = (&'a String, &'a String)> {
    let p = prefix.to_string();
    snap.states.range(p.clone()..).take_while(move |(k, _)| k.starts_with(&p))
}

/// States of a plugin without entities: the catalogue, then what only exists live.
pub fn plugin_rows(p: &Plugin, snap: &Snapshot) -> Vec<Row> {
    let row = |name: &str, value: Option<String>, d: &Doc, template: bool| Row { name: name.to_string(), value, note: d.note.to_string(), values: d.values.iter().map(|v| v.to_string()).collect(), template };
    let mut out: Vec<Row> = p.docs.iter().filter(|d| !d.name.contains('<')).map(|d| row(d.name, snap.states.get(d.name).cloned(), d, false)).collect();
    for (name, value) in with_prefix(snap, p.prefix) {
        if !out.iter().any(|r| r.name == *name) {
            match p.docs.iter().find(|d| matches_doc(d.name, name)) {
                Some(d) => out.push(row(name, Some(value.clone()), d, false)),
                None => out.push(Row { name: name.clone(), value: Some(value.clone()), note: String::new(), values: Vec::new(), template: false }),
            }
        }
    }
    for d in p.docs.iter().filter(|d| d.name.contains('<')) {
        if !out.iter().any(|r| matches_doc(d.name, &r.name)) {
            out.push(row(d.name, None, d, true));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct HaEntity {
    pub id: String,
    pub name: String,
    pub state: String,
}

pub fn ha_entities(snap: &Snapshot) -> Vec<HaEntity> {
    with_prefix(snap, "ha:")
        .filter_map(|(k, state)| {
            let id = k.strip_prefix("ha:")?;
            (!id.contains(':')).then(|| HaEntity { id: id.to_string(), name: snap.states.get(&format!("{k}:name")).cloned().unwrap_or_else(|| id.to_string()), state: state.clone() })
        })
        .collect()
}

fn list_attr(snap: &Snapshot, entity: &str, attr: &str) -> Vec<String> {
    snap.states.get(&format!("ha:{entity}:attr:{attr}")).and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// What the state of an entity can be, by domain; a few domains list it on the entity itself.
fn ha_state_values(snap: &Snapshot, entity: &str) -> Vec<String> {
    let fixed: &[&str] = match entity.split('.').next().unwrap_or("") {
        "light" | "switch" | "fan" | "input_boolean" | "automation" | "binary_sensor" | "remote" | "siren" | "humidifier" | "script" => &["on", "off"],
        "media_player" => &["off", "on", "idle", "playing", "paused", "standby", "buffering"],
        "cover" => &["open", "closed", "opening", "closing"],
        "lock" => &["locked", "unlocked", "locking", "unlocking", "jammed"],
        "person" | "device_tracker" => &["home", "not_home"],
        "sun" => &["above_horizon", "below_horizon"],
        "vacuum" => &["cleaning", "docked", "paused", "returning", "idle", "error"],
        "alarm_control_panel" => &["disarmed", "armed_home", "armed_away", "armed_night", "pending", "arming", "triggered"],
        _ => &[],
    };
    let mut out: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
    out.extend(list_attr(snap, entity, "options"));
    out.extend(list_attr(snap, entity, "hvac_modes"));
    if !out.is_empty() {
        out.extend(["unavailable".to_string(), "unknown".to_string()]);
    }
    out
}

fn ha_attr_info(snap: &Snapshot, entity: &str, attr: &str) -> (&'static str, Vec<String>) {
    let list = |a: &str| list_attr(snap, entity, a);
    match attr {
        "brightness" => ("0 to 255", vec![]),
        "rgb_color" => ("colour", vec![]),
        "volume_level" => ("0 to 1", vec![]),
        "effect" => ("", list("effect_list")),
        "preset_mode" => ("", list("preset_modes")),
        "fan_mode" => ("", list("fan_modes")),
        "swing_mode" => ("", list("swing_modes")),
        "source" => ("", list("source_list")),
        "sound_mode" => ("", list("sound_mode_list")),
        "color_mode" => ("", list("supported_color_modes")),
        _ => ("", vec![]),
    }
}

/// All states of one entity: its state first, then text and name, then the attributes.
pub fn ha_rows(snap: &Snapshot, entity: &str) -> Vec<Row> {
    let base = format!("ha:{entity}");
    let row = |name: String, note: &str, values: Vec<String>| {
        let value = snap.states.get(&name).cloned();
        Row { name, value, note: note.to_string(), values, template: false }
    };
    let mut out = vec![row(base.clone(), "state", ha_state_values(snap, entity)), row(format!("{base}:text"), "state with its unit", vec![]), row(format!("{base}:name"), "friendly name", vec![])];
    let attr_prefix = format!("{base}:attr:");
    for (name, _) in with_prefix(snap, &attr_prefix) {
        let (note, values) = ha_attr_info(snap, entity, &name[attr_prefix.len()..]);
        out.push(row(name.clone(), note, values));
    }
    out
}

/// What a value can drive on a key, for the explorer's hint.
pub fn value_fits(value: &str) -> Vec<&'static str> {
    let mut out = vec![];
    if deck_config::switch_value(value).is_some() || deck_config::first_number(value).is_some() {
        out.push("ring");
    }
    if deck_config::parse_color_value(value).is_some() {
        out.push("colour");
    }
    out
}

/// Sensible top of the scale for a new ring on this state.
pub fn ring_max(state: &str) -> f64 {
    if state.ends_with(":attr:brightness") {
        255.0
    } else if state.ends_with(":attr:volume_level") {
        1.0
    } else {
        100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(states: &[(&str, &str)]) -> Snapshot {
        Snapshot { plugins: vec![], states: states.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), bridge_plugins: vec![] }
    }

    fn plugin(id: &str) -> &'static Plugin {
        PLUGINS.iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn home_assistant_index_points_at_home_assistant() {
        assert_eq!(PLUGINS[HOME_ASSISTANT].id, "homeassistant");
    }

    #[test]
    fn catalogue_rows_show_without_the_agent() {
        let rows = plugin_rows(plugin("discord"), &Snapshot::default());
        assert!(rows.iter().any(|r| r.name == "discord:mute" && r.value.is_none() && r.values == ["true", "false"]));
    }

    #[test]
    fn live_value_lands_on_its_catalogue_row() {
        let rows = plugin_rows(plugin("discord"), &snap(&[("discord:mute", "true")]));
        assert_eq!(rows.iter().filter(|r| r.name == "discord:mute").count(), 1);
        assert_eq!(rows.iter().find(|r| r.name == "discord:mute").unwrap().value.as_deref(), Some("true"));
    }

    #[test]
    fn live_instances_replace_their_template() {
        let rows = plugin_rows(plugin("obs"), &snap(&[("obs:mute:Mic", "false")]));
        let mic = rows.iter().find(|r| r.name == "obs:mute:Mic").unwrap();
        assert_eq!((mic.note.as_str(), mic.template), ("input is muted", false));
        assert!(!rows.iter().any(|r| r.template));
    }

    #[test]
    fn template_stays_until_something_is_live() {
        let rows = plugin_rows(plugin("obs"), &Snapshot::default());
        assert!(rows.iter().any(|r| r.name == "obs:mute:<input>" && r.template));
    }

    #[test]
    fn states_missing_from_the_catalogue_still_show() {
        let rows = plugin_rows(plugin("media"), &snap(&[("media:new_thing", "x")]));
        assert!(rows.iter().any(|r| r.name == "media:new_thing" && r.value.as_deref() == Some("x")));
    }

    #[test]
    fn entities_come_with_friendly_names() {
        let s = snap(&[("ha:light.a", "on"), ("ha:light.a:name", "Wand"), ("ha:light.a:attr:brightness", "12"), ("ha:sensor.b", "3")]);
        let e = ha_entities(&s);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], HaEntity { id: "light.a".into(), name: "Wand".into(), state: "on".into() });
        assert_eq!(e[1].name, "sensor.b");
    }

    #[test]
    fn entity_rows_do_not_mix_up_similar_ids() {
        let s = snap(&[("ha:light.a", "on"), ("ha:light.ab", "off"), ("ha:light.a:attr:brightness", "12"), ("ha:light.ab:attr:effect", "x")]);
        let names: Vec<String> = ha_rows(&s, "light.a").into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["ha:light.a", "ha:light.a:text", "ha:light.a:name", "ha:light.a:attr:brightness"]);
    }

    #[test]
    fn possible_values_by_domain_and_from_the_entity() {
        let s = snap(&[("ha:switch.x", "on"), ("ha:select.y", "a"), ("ha:select.y:attr:options", r#"["a","b"]"#), ("ha:climate.z", "heat"), ("ha:climate.z:attr:hvac_modes", r#"["off","heat"]"#)]);
        assert_eq!(ha_rows(&s, "switch.x")[0].values, ["on", "off", "unavailable", "unknown"]);
        assert_eq!(ha_rows(&s, "select.y")[0].values, ["a", "b", "unavailable", "unknown"]);
        assert_eq!(ha_rows(&s, "climate.z")[0].values, ["off", "heat", "unavailable", "unknown"]);
        assert!(ha_rows(&snap(&[("ha:sensor.t", "21")]), "sensor.t")[0].values.is_empty());
    }

    #[test]
    fn attribute_values_come_from_their_list() {
        let s = snap(&[("ha:light.x", "on"), ("ha:light.x:attr:effect", "rainbow"), ("ha:light.x:attr:effect_list", r#"["rainbow","solid"]"#)]);
        let effect = ha_rows(&s, "light.x").into_iter().find(|r| r.name.ends_with(":effect")).unwrap();
        assert_eq!(effect.values, ["rainbow", "solid"]);
    }

    #[test]
    fn fit_hints() {
        assert_eq!(value_fits("on"), ["ring"]);
        assert_eq!(value_fits("128"), ["ring"]);
        assert_eq!(value_fits("[255, 0, 10]"), ["ring", "colour"]);
        assert!(value_fits("hello").is_empty());
    }

    #[test]
    fn bridge_fills_in_and_brings_its_own_plugins() {
        let agent = snap(&[("spotify:volume", "40")]);
        let mut bridge = snap(&[("spotify:volume", "41"), ("mqtt:door", "open"), ("wled:on", "true")]);
        bridge.bridge_plugins = vec!["spotify".into(), "mqtt".into(), "wled".into()];
        let m = merge(&agent, Some(&bridge));
        assert_eq!(m.states["spotify:volume"], "40");
        assert_eq!(m.states["mqtt:door"], "open");
        assert_eq!(extra_plugins(&m), ["mqtt", "wled"]);
        assert_eq!(extra_rows("wled", &m)[0].name, "wled:on");
    }

    #[test]
    fn ring_scale() {
        assert_eq!(ring_max("ha:light.x:attr:brightness"), 255.0);
        assert_eq!(ring_max("ha:media_player.x:attr:volume_level"), 1.0);
        assert_eq!(ring_max("agentdeck:usage:5h"), 100.0);
    }
}
