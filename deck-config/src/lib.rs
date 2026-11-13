//! Shared configuration model: pages, keys, knobs, actions, layout.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Action {
    /// Press a key combination, e.g. {"type":"key","keys":["ctrl","shift","s"]}
    Key { keys: Vec<String> },
    /// Type ASCII text.
    Text { text: String },
    /// Switch to a named page.
    Page { page: String },
    /// Next / previous page.
    PageNext,
    PagePrev,
    /// Sequence of actions with optional delays (ms).
    Macro { steps: Vec<Step> },
    /// Executed by the PC agent: plugin operation with JSON arguments.
    /// `fallback` runs on the deck when no agent is connected.
    Host {
        plugin: String,
        op: String,
        #[serde(default)]
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<Box<Action>>,
    },
    /// Show an image full screen (960x540) until any key is pressed or `seconds` elapse (0 = until key).
    /// With `frames` > 0 the image is an animation: `image-f00.png` .. `image-fNN.png`, one frame every
    /// `delay_ms`; it plays once when `seconds` is 0, otherwise it loops for `seconds`.
    Screen {
        image: String,
        #[serde(default)]
        seconds: u32,
        #[serde(default, skip_serializing_if = "is_zero_u32")]
        frames: u32,
        #[serde(default = "default_frame_delay", skip_serializing_if = "is_default_frame_delay")]
        delay_ms: u64,
    },
    /// Play a full-screen effect over the current page (see `Effect`).
    Effect { effect: Effect },
    /// Do nothing.
    None,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn default_frame_delay() -> u64 {
    80
}
fn is_default_frame_delay(v: &u64) -> bool {
    *v == default_frame_delay()
}
fn default_effect_ms() -> u32 {
    700
}
fn default_wave_ms() -> u32 {
    700
}
fn default_every_ms() -> u32 {
    250
}
fn default_from() -> String {
    "left".into()
}

/// Full-screen effect drawn over the page, started by a key press (`KeyDef::effect`)
/// or by an `Action::Effect`. Runs on the deck, no PC involved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Effect {
    /// Minecraft level-up: blocky bright green rings roll outwards from the pressed key.
    /// `ms` total duration; `wave_ms` time one ring needs to cross the screen (speed);
    /// `every_ms` interval between rings (another ring starts whenever it still fits into `ms`).
    LevelUp {
        #[serde(default = "default_effect_ms")]
        ms: u32,
        #[serde(default = "default_wave_ms")]
        wave_ms: u32,
        #[serde(default = "default_every_ms")]
        every_ms: u32,
    },
    /// A glowing band in `color` sweeping across the screen from `from` (left/right/top/bottom).
    /// `blocky` quantises the band into 20 px cells (pixel-art look).
    Wave {
        #[serde(default)]
        color: Option<String>,
        #[serde(default = "default_effect_ms")]
        ms: u32,
        #[serde(default = "default_from")]
        from: String,
        #[serde(default)]
        blocky: bool,
    },
    /// Frame sequence from the icon store blended over the page: `name-f00.png` .. `name-fNN.png`
    /// (any size; scaled to 960x540, alpha respected), one frame every `delay_ms`, played once.
    Frames {
        name: String,
        frames: u32,
        #[serde(default = "default_frame_delay")]
        delay_ms: u64,
    },
}

impl Effect {
    /// Number of rings a level-up effect fires (one, plus one per `every_ms` that still fits).
    pub fn ring_count(ms: u32, wave_ms: u32, every_ms: u32) -> u32 {
        if ms < wave_ms {
            1
        } else {
            1 + (ms - wave_ms) / every_ms.max(16)
        }
    }

    /// Total duration of the effect in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        match self {
            Effect::LevelUp { ms, .. } => *ms as u64,
            Effect::Wave { ms, .. } => *ms as u64,
            Effect::Frames { frames, delay_ms, .. } => *frames as u64 * (*delay_ms).max(16),
        }
    }
}

/// Progress indicator on a tile driven by a numeric state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Progress {
    /// State name whose value is shown, e.g. "ha:light.x:attr:brightness": the first number in
    /// it, or an on/off word (see `Progress::fraction`).
    pub state: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default = "default_max")]
    pub max: f64,
    /// "border" (ring along the tile edge) or "fill" (fills the tile from a side).
    #[serde(default = "default_style")]
    pub style: String,
    /// border: where the ring starts ("top", "right", "bottom", "left"); fill: side it grows from.
    #[serde(default = "default_start")]
    pub start: String,
    #[serde(default = "default_pcolor")]
    pub color: String,
    /// State whose value replaces `color` while it parses as a colour (see `parse_color_value`),
    /// e.g. "ha:light.x:attr:rgb_color".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_state: Option<String>,
    /// border thickness in px
    #[serde(default = "default_width")]
    pub width: i32,
    /// States whose numbers replace `min` / `max` while they are known, for limits the device
    /// reports itself (a thermostat's `min_temp`, an input_number's `min` and `max`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_state: Option<String>,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { state: String::new(), min: 0.0, max: default_max(), style: default_style(), start: default_start(), color: default_pcolor(), color_state: None, width: default_width(), min_state: None, max_state: None }
    }
}

fn default_max() -> f64 {
    100.0
}
fn default_style() -> String {
    "border".into()
}
fn default_start() -> String {
    "top".into()
}
fn default_pcolor() -> String {
    "#f5b400".into()
}
fn default_width() -> i32 {
    6
}

/// Appearance of a key for one state value (see `KeyDef::looks`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Look {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Deck background while this look is active, on every page: an image in the icon store
    /// (960x540, like `Page::background`) or a colour `#rrggbb`. See `Config::state_background`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

impl Default for Action {
    fn default() -> Self {
        Action::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    Delay { delay_ms: u64 },
    Act(Action),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyDef {
    #[serde(default)]
    pub label: String,
    /// Icon file name under /data/deckd/icons (PNG), optional.
    #[serde(default)]
    pub icon: Option<String>,
    /// Background colour as "#rrggbb".
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub text_color: Option<String>,
    #[serde(default = "default_none")]
    pub action: Action,
    /// Name of a state fed by the agent, e.g. "ha:light.kitchen". Its value
    /// selects an entry of `looks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Looks by state value ("on", "off", "playing", ...). Fields not set fall
    /// back to the key's base icon/colour/label.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub looks: BTreeMap<String, Look>,
    /// Name of a state whose value is shown as the label (sensor readings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Progress ring / fill driven by a numeric state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// Full-tile image fed by a state: the value names an uploaded image ("name" -> name.png) or an
    /// animation ("name|frames|delay_ms" -> name-f00.png ...). Replaces icon, label and colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile: Option<String>,
    /// Full-screen effect played on every press of this key (besides its action).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    /// State whose value is used as the background colour when it parses as one
    /// (`#rrggbb`, `r,g,b` or `[r, g, b]`, e.g. "ha:light.desk:attr:rgb_color"). Overrides the
    /// key's and the look's colour; an unparsable or missing value leaves them untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_state: Option<String>,
    /// Label font size in px (None: automatic, up to 26, shrinking to fit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Horizontal label alignment: "left", "center" (default) or "right".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    /// Vertical label alignment: "top", "middle" or "bottom". Default: middle without an icon,
    /// bottom (under the icon) with one; "top" puts the text above the icon, "middle" over it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<String>,
    /// Font file in the icon store (`font-xxx.ttf`, uploaded like an icon); None: the deck's font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

/// State names referenced by `{...}` placeholders in a label template (`{ha:sensor.x|1}` -> `ha:sensor.x`).
pub fn label_placeholders(label: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = label;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let inner = &after[..end];
        let name = inner.split('|').next().unwrap_or("").trim();
        if !name.is_empty() {
            v.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    v
}

/// Format one placeholder value: `|0`/`|1`/`|2` round to that many decimals (first number in the
/// value, unit dropped), `|int` = `|0`, `|upper`, `|lower`; no spec: the value as is.
fn format_placeholder(value: &str, spec: Option<&str>) -> String {
    match spec.map(str::trim) {
        None | Some("") => value.to_string(),
        Some("upper") => value.to_uppercase(),
        Some("lower") => value.to_lowercase(),
        Some(d) => {
            let decimals: usize = if d == "int" { 0 } else { d.parse().unwrap_or(0) };
            match first_number(value) {
                Some(n) => format!("{n:.decimals$}"),
                None => value.to_string(),
            }
        }
    }
}

/// Fill a label template: `{}` = the key's `value` state, `{name}` / `{name|fmt}` = any state.
/// Missing states render as "-". Without placeholders the `value` (if any) replaces the label.
pub fn render_label(template: &str, value: Option<&str>, states: &dyn Fn(&str) -> Option<String>) -> String {
    if !template.contains('{') {
        return value.map(str::to_string).unwrap_or_else(|| template.to_string());
    }
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (name, spec) = match inner.split_once('|') {
            Some((n, f)) => (n.trim(), Some(f)),
            None => (inner.trim(), None),
        };
        let v = if name.is_empty() { value.map(str::to_string) } else { states(name) };
        out.push_str(&v.map(|v| format_placeholder(&v, spec)).unwrap_or_else(|| "-".into()));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Parse a state value as a colour: `#rrggbb`, `rrggbb`, `r,g,b`, `[r, g, b]` or `(r, g, b)`.
pub fn parse_color_value(v: &str) -> Option<u32> {
    let t = v.trim();
    let h = t.trim_start_matches('#');
    if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        return u32::from_str_radix(h, 16).ok().map(|c| 0xff000000 | c);
    }
    let inner = t.trim_matches(|c| c == '[' || c == ']' || c == '(' || c == ')');
    let parts: Vec<u32> = inner.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).map(|x| x.round().clamp(0.0, 255.0) as u32).collect();
    if parts.len() >= 3 {
        return Some(0xff000000 | (parts[0] << 16) | (parts[1] << 8) | parts[2]);
    }
    None
}

/// Parse a tile state value: (base name, frames, delay ms).
pub fn parse_tile(v: &str) -> Option<(String, u32, u64)> {
    let mut it = v.split('|');
    let base = it.next()?.trim();
    if base.is_empty() {
        return None;
    }
    let frames = it.next().and_then(|f| f.trim().parse().ok()).unwrap_or(0);
    let delay = it.next().and_then(|d| d.trim().parse().ok()).unwrap_or(150).max(30);
    Some((base.to_string(), frames, delay))
}

fn default_none() -> Action {
    Action::None
}

/// First number found in a state value ("45 %" -> 45.0).
pub fn first_number(v: &str) -> Option<f64> {
    v.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).find_map(|t| t.parse::<f64>().ok())
}

/// On/off style state value as 1.0 / 0.0 ("on", "true", "open", ...), any case.
pub fn switch_value(v: &str) -> Option<f64> {
    match v.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "open" | "unlocked" | "home" | "playing" | "active" | "enabled" | "connected" => Some(1.0),
        "off" | "false" | "no" | "closed" | "locked" | "away" | "not_home" | "paused" | "idle" | "stopped" | "inactive" | "disabled" | "disconnected" => Some(0.0),
        _ => None,
    }
}

impl Progress {
    /// How full the indicator is for a state value, 0.0..=1.0. On/off words are simply full or
    /// empty whatever min and max are; a value that is neither gives None (nothing drawn).
    pub fn fraction(&self, value: &str) -> Option<f64> {
        if let Some(on) = switch_value(value) {
            return Some(on);
        }
        let n = first_number(value)?;
        Some(if self.max > self.min { ((n - self.min) / (self.max - self.min)).clamp(0.0, 1.0) } else { 0.0 })
    }

    /// `fraction` with the live limits from `min_state` / `max_state` where those are known.
    pub fn fraction_in(&self, states: &std::collections::HashMap<String, String>) -> Option<f64> {
        let value = states.get(&self.state)?;
        let live = |s: &Option<String>| s.as_deref().and_then(|n| states.get(n)).and_then(|v| first_number(v));
        let p = Progress { min: live(&self.min_state).unwrap_or(self.min), max: live(&self.max_state).unwrap_or(self.max), ..self.clone() };
        p.fraction(value)
    }

    /// Every state this indicator reads.
    pub fn states(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.state).chain(self.color_state.iter()).chain(self.min_state.iter()).chain(self.max_state.iter())
    }
}

/// A progress indicator for a knob whose turn action has known limits: PC volume (volume keys),
/// Home Assistant light brightness, media player volume, thermostat and number limits (reported
/// live), Spotify volume.
pub fn knob_progress_preset(knob: &KnobDef) -> Option<Progress> {
    let fill = |state: String, min: f64, max: f64, color: &str| Progress { state, min, max, style: "fill".into(), start: "left".into(), color: color.into(), ..Default::default() };
    // volume keys: the PC agent reports the master volume
    if let Action::Key { keys } = &knob.right {
        return (keys.len() == 1 && keys[0] == "volumeup").then(|| fill("system:volume".into(), 0.0, 100.0, "#7fd3ff"));
    }
    let Action::Host { plugin, op, args, .. } = &knob.right else { return None };
    match plugin.as_str() {
        "spotify" if op == "volume" || op == "volume_set" => Some(fill("spotify:volume".into(), 0.0, 100.0, "#1db954")),
        "homeassistant" => {
            let e = args.get("entity_id").and_then(|v| v.as_str())?;
            let domain = e.split('.').next().unwrap_or("");
            match (op.as_str(), domain) {
                ("brightness", "light") => Some(fill(format!("ha:{e}:attr:brightness"), 0.0, 255.0, "#f5b400")),
                ("volume", "media_player") => Some(fill(format!("ha:{e}:attr:volume_level"), 0.0, 1.0, "#7fd3ff")),
                ("temperature", "climate") => Some(Progress { min_state: Some(format!("ha:{e}:attr:min_temp")), max_state: Some(format!("ha:{e}:attr:max_temp")), ..fill(format!("ha:{e}:attr:temperature"), 7.0, 35.0, "#ff8040") }),
                ("number", "number" | "input_number") => Some(Progress { min_state: Some(format!("ha:{e}:attr:min")), max_state: Some(format!("ha:{e}:attr:max")), ..fill(format!("ha:{e}"), 0.0, 100.0, "#7fd3ff") }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Does a look rule key match a state value?
/// Rules: exact string, `*` default, `>x` `>=x` `<x` `<=x` numeric, `a..b` inclusive range,
/// `a|b|c` alternatives, `~text` substring (case-insensitive).
pub fn rule_matches(rule: &str, value: &str) -> bool {
    let rule = rule.trim();
    if rule == "*" {
        return true;
    }
    if rule.contains('|') {
        return rule.split('|').any(|r| rule_matches(r, value));
    }
    if let Some(sub) = rule.strip_prefix('~') {
        return value.to_lowercase().contains(&sub.trim().to_lowercase());
    }
    let num = value.trim().split_whitespace().next().and_then(|v| v.parse::<f64>().ok());
    if let Some((lo, hi)) = rule.split_once("..") {
        if let (Ok(lo), Ok(hi), Some(n)) = (lo.trim().parse::<f64>(), hi.trim().parse::<f64>(), num) {
            return n >= lo && n <= hi;
        }
        return false;
    }
    for (op, f) in [(">=", (|n: f64, x: f64| n >= x) as fn(f64, f64) -> bool), ("<=", |n, x| n <= x), (">", |n, x| n > x), ("<", |n, x| n < x), ("=", |n, x| (n - x).abs() < 1e-9)] {
        if let Some(rest) = rule.strip_prefix(op) {
            if let (Ok(x), Some(n)) = (rest.trim().parse::<f64>(), num) {
                return f(n, x);
            }
            return false;
        }
    }
    rule.eq_ignore_ascii_case(value.trim())
}

/// Pick the look for a state value: exact rules first, then comparisons, `*` last.
pub fn pick_look<'a>(looks: &'a BTreeMap<String, Look>, value: &str) -> Option<&'a Look> {
    if let Some(l) = looks.iter().find(|(k, _)| !k.starts_with(['>', '<', '=', '~', '*']) && !k.contains("..") && !k.contains('|') && rule_matches(k, value)) {
        return Some(l.1);
    }
    if let Some(l) = looks.iter().find(|(k, _)| k.as_str() != "*" && rule_matches(k, value)) {
        return Some(l.1);
    }
    looks.get("*")
}

impl Config {
    /// The background a key's active look sets for the whole deck, if any. Keys on the shown
    /// page come first, then the other pages in page order; within a page, slot order.
    pub fn state_background(&self, shown: &str, states: &std::collections::HashMap<String, String>) -> Option<String> {
        let order = self.order();
        let pages = std::iter::once(shown).chain(order.iter().map(String::as_str).filter(|p| *p != shown));
        for page in pages.filter_map(|p| self.pages.get(p)) {
            let mut keys: Vec<(&String, &KeyDef)> = page.keys.iter().collect();
            keys.sort_by_key(|(slot, _)| slot.parse::<u32>().unwrap_or(u32::MAX));
            for (_, k) in keys {
                let Some(value) = k.state.as_deref().and_then(|s| states.get(s)) else { continue };
                if let Some(bg) = pick_look(&k.looks, value).and_then(|l| l.background.as_deref()).filter(|b| !b.trim().is_empty()) {
                    return Some(bg.to_string());
                }
            }
        }
        None
    }

    /// All state names the agent should feed (states and values of every key).
    pub fn wanted_states(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .pages
            .values()
            .flat_map(|p| p.keys.values())
            .flat_map(|k| {
                let mut v: Vec<String> = k.state.iter().chain(k.value.iter()).chain(k.tile.iter()).chain(k.color_state.iter()).chain(k.progress.iter().flat_map(Progress::states)).cloned().collect();
                v.extend(label_placeholders(&k.label));
                v.extend(k.looks.values().filter_map(|l| l.label.as_deref()).flat_map(label_placeholders));
                v
            })
            .collect();
        let knobs: Vec<Progress> = self.pages.values().flat_map(|p| p.knobs.iter()).filter_map(KnobDef::indicator).collect();
        v.extend(knobs.iter().flat_map(Progress::states).cloned());
        if self.pages.values().any(|p| p.app.as_deref().map(|a| !a.trim().is_empty()).unwrap_or(false)) {
            v.push("system:app".into());
        }
        v.sort();
        v.dedup();
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnobDef {
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_none")]
    pub press: Action,
    #[serde(default = "default_none")]
    pub left: Action,
    #[serde(default = "default_none")]
    pub right: Action,
    /// Indicator in the knob's box on the wide key (fill or border), e.g. the brightness it turns.
    /// None: automatic (`knob_progress_preset`), style "none": hidden. See `KnobDef::indicator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

impl KnobDef {
    /// The indicator shown: the custom one, or the automatic one when the turn action has a
    /// known range; nothing when switched off.
    pub fn indicator(&self) -> Option<Progress> {
        match &self.progress {
            Some(p) if p.style == "none" => None,
            Some(p) => Some(p.clone()),
            None => knob_progress_preset(self),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Page {
    /// Keys by slot "0".."13" (13 = wide key).
    #[serde(default)]
    pub keys: BTreeMap<String, KeyDef>,
    #[serde(default)]
    pub knobs: Vec<KnobDef>,
    /// Full-screen background image (960x540 PNG/JPEG in the icon store), drawn behind the tiles.
    /// Tiles with colour "none" are transparent so the image shows through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Auto-switch: process names (e.g. "photoshop.exe|lightroom.exe", case-insensitive substring)
    /// of the foreground application that brings this page up. Needs the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// Does a page's `app` pattern match a foreground process name?
pub fn app_matches(pattern: &str, app: &str) -> bool {
    let app = app.to_lowercase();
    pattern.split('|').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).any(|p| app.contains(&p))
}

/// Tile geometry on the 960x540 canvas. Defaults follow the physical key
/// windows (D200 panel notes scaled 0.75): rows are centred at y 73/270/467,
/// columns every 192 px. Adjustable at runtime through the config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default = "default_row_y")]
    pub row_y: [i32; 3],
    #[serde(default = "default_row_h")]
    pub row_h: i32,
    #[serde(default = "default_col_x")]
    pub col_x: [i32; 5],
    #[serde(default = "default_col_w")]
    pub col_w: i32,
    /// Wide slot (index 13): x, y, w, h.
    #[serde(default = "default_wide")]
    pub wide: [i32; 4],
    #[serde(default = "default_radius")]
    pub radius: i32,
    /// Inset for labels and icons from the tile edge, so content stays clear of
    /// the keycap bevel when the deck is viewed at an angle.
    #[serde(default = "default_text_pad")]
    pub text_pad: i32,
}

// Calibrated on the real deck 2026-10-30 with the ring pattern (`calibrate on`).
fn default_row_y() -> [i32; 3] {
    [0, 195, 390]
}
fn default_row_h() -> i32 {
    150
}
fn default_col_x() -> [i32; 5] {
    [3, 199, 395, 591, 787]
}
fn default_col_w() -> i32 {
    160
}
fn default_wide() -> [i32; 4] {
    [591, 390, 356, 150]
}
fn default_radius() -> i32 {
    14
}
fn default_text_pad() -> i32 {
    18
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            row_y: default_row_y(),
            row_h: default_row_h(),
            col_x: default_col_x(),
            col_w: default_col_w(),
            wide: default_wide(),
            radius: default_radius(),
            text_pad: default_text_pad(),
        }
    }
}

impl Layout {
    pub fn slot_rect(&self, i: usize) -> (i32, i32, i32, i32) {
        if i == 13 {
            return (self.wide[0], self.wide[1], self.wide[2], self.wide[3]);
        }
        let (col, row) = (i % 5, i / 5);
        (self.col_x[col.min(4)], self.row_y[row.min(2)], self.col_w, self.row_h)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub layout: Layout,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_home")]
    pub home: String,
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Seconds of inactivity before the backlight dims to `dim_brightness` (0 = never).
    #[serde(default = "default_dim_after")]
    pub dim_after_s: u32,
    #[serde(default = "default_dim_brightness")]
    pub dim_brightness: u8,
    /// Seconds of inactivity before the screen turns off (0 = never).
    #[serde(default = "default_off_after")]
    pub off_after_s: u32,
    /// Side buttons: default page prev/next.
    #[serde(default = "default_side_prev")]
    pub side_left: Action,
    #[serde(default = "default_side_next")]
    pub side_right: Action,
    /// Page order for next/prev.
    #[serde(default)]
    pub page_order: Vec<String>,
    pub pages: BTreeMap<String, Page>,
    /// Agent links: the PC over USB, an ESP32 bridge on the console UART.
    #[serde(default, skip_serializing_if = "Agents::is_default")]
    pub agents: Agents,
}

/// A link an agent can sit on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Usb,
    Uart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Agents {
    #[serde(default)]
    pub uart: UartLink,
    /// Plugin -> links in order of preference. Plugins not listed use `["uart", "usb"]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, Vec<LinkKind>>,
}

impl Agents {
    fn is_default(&self) -> bool {
        *self == Agents::default()
    }
    /// Links to try for a plugin's actions and states, best first.
    pub fn route(&self, plugin: &str) -> Vec<LinkKind> {
        self.routes.get(plugin).cloned().unwrap_or_else(|| vec![LinkKind::Uart, LinkKind::Usb])
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UartLink {
    /// Console UART (/dev/ttyS0) carries an agent (the ESP32 bridge). Off: the console stays a shell.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_uart_baud")]
    pub baud: u32,
}

impl Default for UartLink {
    fn default() -> Self {
        UartLink { enabled: false, baud: default_uart_baud() }
    }
}

fn default_uart_baud() -> u32 {
    115200
}

/// Plugin that feeds a state, from the state name's prefix (`ha:light.x` -> `homeassistant`).
/// Agents may announce other prefixes in their heartbeat; this is the built-in table.
pub fn plugin_of_state(state: &str) -> &str {
    let prefix = state.split(':').next().unwrap_or("");
    match prefix {
        "ha" => "homeassistant",
        other => other,
    }
}

fn default_version() -> u32 {
    1
}
fn default_home() -> String {
    "main".into()
}
fn default_brightness() -> u8 {
    80
}
fn default_dim_after() -> u32 {
    120
}
fn default_dim_brightness() -> u8 {
    15
}
fn default_off_after() -> u32 {
    900
}
fn default_side_prev() -> Action {
    Action::PagePrev
}
fn default_side_next() -> Action {
    Action::PageNext
}

impl Config {
    pub fn default_config() -> Config {
        let mut keys = BTreeMap::new();
        let defs: [(&str, &str, &str, Action); 14] = [
            ("0", "Copy", "#2d6cdf", Action::Key { keys: vec!["ctrl".into(), "c".into()] }),
            ("1", "Paste", "#2d6cdf", Action::Key { keys: vec!["ctrl".into(), "v".into()] }),
            ("2", "Cut", "#2d6cdf", Action::Key { keys: vec!["ctrl".into(), "x".into()] }),
            ("3", "Undo", "#7a4fd8", Action::Key { keys: vec!["ctrl".into(), "z".into()] }),
            ("4", "Redo", "#7a4fd8", Action::Key { keys: vec!["ctrl".into(), "y".into()] }),
            ("5", "Save", "#1f9d55", Action::Key { keys: vec!["ctrl".into(), "s".into()] }),
            ("6", "Find", "#1f9d55", Action::Key { keys: vec!["ctrl".into(), "f".into()] }),
            ("7", "Hello", "#c0392b", Action::Text { text: "Hello from deckd\n".into() }),
            ("8", "Desktop", "#d35400", Action::Key { keys: vec!["gui".into(), "d".into()] }),
            ("9", "Lock", "#d35400", Action::Key { keys: vec!["gui".into(), "l".into()] }),
            ("10", "Alt+Tab", "#555555", Action::Key { keys: vec!["alt".into(), "tab".into()] }),
            ("11", "Mute", "#555555", Action::Key { keys: vec!["mute".into()] }),
            ("12", "Page 2", "#8e44ad", Action::Page { page: "two".into() }),
            ("13", "deckd 0.1  standalone", "#1c2b3a", Action::None),
        ];
        for (slot, label, color, action) in defs {
            keys.insert(slot.into(), KeyDef { label: label.into(), icon: None, color: Some(color.into()), text_color: None, action, state: None, looks: BTreeMap::new(), value: None, progress: None, tile: None, effect: None, color_state: None, font_size: None, align: None, valign: None, font: None });
        }
        let knobs = vec![
            KnobDef {
                label: "Vol".into(),
                press: Action::Key { keys: vec!["mute".into()] },
                left: Action::Key { keys: vec!["volumedown".into()] },
                right: Action::Key { keys: vec!["volumeup".into()] },
                progress: None,
            },
            KnobDef {
                label: "Scroll".into(),
                press: Action::Key { keys: vec!["home".into()] },
                left: Action::Key { keys: vec!["up".into()] },
                right: Action::Key { keys: vec!["down".into()] },
                progress: None,
            },
            KnobDef {
                label: "Tabs".into(),
                press: Action::Key { keys: vec!["ctrl".into(), "w".into()] },
                left: Action::Key { keys: vec!["ctrl".into(), "shift".into(), "tab".into()] },
                right: Action::Key { keys: vec!["ctrl".into(), "tab".into()] },
                progress: None,
            },
        ];
        let mut pages = BTreeMap::new();
        pages.insert("main".into(), Page { keys, knobs: knobs.clone(), background: None, app: None });
        let mut keys2 = BTreeMap::new();
        for i in 0..13 {
            keys2.insert(
                i.to_string(),
                KeyDef {
                    label: format!("F{}", i + 1),
                    icon: None,
                    color: Some("#34495e".into()),
                    text_color: None,
                    action: Action::Key { keys: vec![format!("f{}", i + 1)] },
                    state: None,
                    looks: BTreeMap::new(),
                    value: None,
                    progress: None,
                    tile: None,
                    effect: None,
                    color_state: None,
                    font_size: None,
                    align: None,
                    valign: None,
                    font: None,
                },
            );
        }
        keys2.insert("13".into(), KeyDef { label: "Back".into(), icon: None, color: Some("#8e44ad".into()), text_color: None, action: Action::Page { page: "main".into() }, state: None, looks: BTreeMap::new(), value: None, progress: None, tile: None, effect: None, color_state: None, font_size: None, align: None, valign: None, font: None });
        pages.insert("two".into(), Page { keys: keys2, knobs, background: None, app: None });
        Config {
            layout: Layout::default(),
            version: 1,
            home: "main".into(),
            brightness: 80,
            dim_after_s: 120,
            dim_brightness: 15,
            off_after_s: 900,
            side_left: Action::PagePrev,
            side_right: Action::PageNext,
            page_order: vec!["main".into(), "two".into()],
            pages,
            agents: Agents::default(),
        }
    }

    pub fn order(&self) -> Vec<String> {
        if self.page_order.is_empty() {
            self.pages.keys().cloned().collect()
        } else {
            self.page_order.clone()
        }
    }
}

pub fn parse_color(s: &str, fallback: u32) -> u32 {
    let h = s.trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return 0xff000000 | v;
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rules() {
        assert!(rule_matches("on", "on"));
        assert!(rule_matches("ON", "on"));
        assert!(!rule_matches("on", "off"));
        assert!(rule_matches("*", "anything"));
        assert!(rule_matches(">20", "21.5 °C"));
        assert!(!rule_matches(">20", "19"));
        assert!(rule_matches("18..22", "20"));
        assert!(rule_matches("playing|paused", "paused"));
        assert!(rule_matches("~heat", "Heating"));
        let mut looks = BTreeMap::new();
        looks.insert("*".to_string(), Look { label: Some("default".into()), ..Default::default() });
        looks.insert(">25".to_string(), Look { label: Some("hot".into()), ..Default::default() });
        looks.insert("on".to_string(), Look { label: Some("on".into()), ..Default::default() });
        assert_eq!(pick_look(&looks, "on").unwrap().label.as_deref(), Some("on"));
        assert_eq!(pick_look(&looks, "30").unwrap().label.as_deref(), Some("hot"));
        assert_eq!(pick_look(&looks, "10").unwrap().label.as_deref(), Some("default"));
    }

    #[test]
    fn effects_roundtrip() {
        let a = Action::Screen { image: "x".into(), seconds: 0, frames: 12, delay_ms: 50 };
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains("\"frames\":12"));
        let b: Action = serde_json::from_str(&j).unwrap();
        assert!(matches!(b, Action::Screen { frames: 12, delay_ms: 50, .. }));
        // old configs without the new fields still load
        let old: Action = serde_json::from_str(r#"{"type":"screen","image":"a.png"}"#).unwrap();
        assert!(matches!(old, Action::Screen { frames: 0, delay_ms: 80, .. }));
        let k: KeyDef = serde_json::from_str(r#"{"label":"x","effect":{"type":"level-up"}}"#).unwrap();
        assert_eq!(k.effect, Some(Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 }));
        assert_eq!(Effect::ring_count(1200, 700, 250), 3);
        assert_eq!(Effect::ring_count(500, 700, 250), 1);
        assert_eq!(Effect::Frames { name: "f".into(), frames: 10, delay_ms: 50 }.duration_ms(), 500);
        assert_eq!(parse_color_value("#ff8000"), Some(0xffff8000));
        assert_eq!(parse_color_value("[255, 128, 0]"), Some(0xffff8000));
        assert_eq!(parse_color_value("255,128,0"), Some(0xffff8000));
        assert_eq!(parse_color_value("on"), None);
    }

    #[test]
    fn label_templates() {
        let states = |n: &str| match n {
            "ha:input_number.done" => Some("4.0".to_string()),
            "ha:sensor.t" => Some("21.46 °C".to_string()),
            _ => None,
        };
        assert_eq!(label_placeholders("{ha:input_number.done|0}/26 {ha:sensor.t}"), vec!["ha:input_number.done", "ha:sensor.t"]);
        assert_eq!(render_label("{ha:input_number.done|0}/26", None, &states), "4/26");
        assert_eq!(render_label("{ha:sensor.t|1}", None, &states), "21.5");
        assert_eq!(render_label("{}/26", Some("4.0"), &states), "4.0/26");
        assert_eq!(render_label("{|int}/26", Some("4.0"), &states), "4/26");
        assert_eq!(render_label("{ha:none}", None, &states), "-");
        assert_eq!(render_label("plain", Some("v"), &states), "v");
        assert_eq!(render_label("plain", None, &states), "plain");
    }

    #[test]
    fn progress_fraction_numbers_and_switch_words() {
        let p = Progress { state: "s".into(), min: 0.0, max: 200.0, style: "border".into(), start: "top".into(), color: "#fff".into(), width: 4, color_state: None, ..Default::default() };
        assert_eq!(p.fraction("50"), Some(0.25));
        assert_eq!(p.fraction("100 %"), Some(0.5));
        assert_eq!(p.fraction("900"), Some(1.0));
        for v in ["on", "ON", "true", " True ", "yes", "open", "playing"] {
            assert_eq!(p.fraction(v), Some(1.0), "{v}");
        }
        for v in ["off", "Off", "false", "no", "closed", "paused"] {
            assert_eq!(p.fraction(v), Some(0.0), "{v}");
        }
        assert_eq!(p.fraction("unavailable"), None);
        assert_eq!(p.fraction(""), None);
    }

    #[test]
    fn wanted_states_include_the_ring_colour_state() {
        let mut cfg = Config::default_config();
        let page = cfg.pages.values_mut().next().unwrap();
        let key = page.keys.values_mut().next().unwrap();
        key.progress = Some(Progress { state: "ha:light.x:attr:brightness".into(), min: 0.0, max: 255.0, style: "border".into(), start: "top".into(), color: "#fff".into(), width: 4, color_state: Some("ha:light.x:attr:rgb_color".into()), ..Default::default() });
        assert!(cfg.wanted_states().iter().any(|s| s == "ha:light.x:attr:rgb_color"));
    }

    #[test]
    fn old_configs_load_without_a_ring_colour_state() {
        let p: Progress = serde_json::from_str(r#"{"state":"s"}"#).unwrap();
        assert_eq!(p.color_state, None);
    }

    #[test]
    fn configs_without_agents_keep_the_uart_off_and_do_not_grow() {
        let cfg: Config = serde_json::from_str(r#"{"pages":{}}"#).unwrap();
        assert!(!cfg.agents.uart.enabled);
        assert_eq!(cfg.agents.uart.baud, 115200);
        assert!(!serde_json::to_string(&cfg).unwrap().contains("agents"));
    }

    #[test]
    fn routes_default_to_uart_then_usb() {
        let cfg: Config = serde_json::from_str(r#"{"pages":{},"agents":{"uart":{"enabled":true},"routes":{"agentdeck":["usb"]}}}"#).unwrap();
        assert!(cfg.agents.uart.enabled);
        assert_eq!(cfg.agents.route("homeassistant"), vec![LinkKind::Uart, LinkKind::Usb]);
        assert_eq!(cfg.agents.route("agentdeck"), vec![LinkKind::Usb]);
        assert_eq!(plugin_of_state("ha:light.x:attr:rgb_color"), "homeassistant");
        assert_eq!(plugin_of_state("obs:scene"), "obs");
    }

    #[test]
    fn state_background() {
        let key = |state: &str, bg: &str| {
            let mut looks = BTreeMap::new();
            looks.insert("on".to_string(), Look { background: Some(bg.into()), ..Default::default() });
            looks.insert("*".to_string(), Look::default());
            KeyDef { state: Some(state.into()), looks, ..Default::default() }
        };
        let mut a = Page::default();
        a.keys.insert("0".into(), key("x", "bg-x.png"));
        let mut b = Page::default();
        b.keys.insert("3".into(), key("y", "#203040"));
        let mut pages = BTreeMap::new();
        pages.insert("a".to_string(), a);
        pages.insert("b".to_string(), b);
        let cfg: Config = serde_json::from_value(serde_json::json!({ "pages": {} })).unwrap();
        let cfg = Config { pages, page_order: vec!["a".into(), "b".into()], ..cfg };
        let mut st = std::collections::HashMap::new();
        assert_eq!(cfg.state_background("a", &st), None);
        st.insert("y".to_string(), "on".to_string());
        assert_eq!(cfg.state_background("a", &st).as_deref(), Some("#203040"));
        st.insert("x".to_string(), "on".to_string());
        assert_eq!(cfg.state_background("a", &st).as_deref(), Some("bg-x.png"));
        assert_eq!(cfg.state_background("b", &st).as_deref(), Some("#203040"));
        st.insert("y".to_string(), "off".to_string());
        assert_eq!(cfg.state_background("b", &st).as_deref(), Some("bg-x.png"));
    }

    #[test]
    fn knob_progress() {
        let ha = |op: &str, e: &str| Action::Host { plugin: "homeassistant".into(), op: op.into(), args: serde_json::json!({ "entity_id": e }), fallback: None };
        let knob = |a: Action| KnobDef { right: a, ..Default::default() };
        let p = knob_progress_preset(&knob(ha("brightness", "light.desk"))).unwrap();
        assert_eq!((p.state.as_str(), p.max, p.style.as_str()), ("ha:light.desk:attr:brightness", 255.0, "fill"));
        assert_eq!(knob_progress_preset(&knob(Action::Key { keys: vec!["volumeup".into()] })).unwrap().state, "system:volume");
        assert!(knob_progress_preset(&knob(Action::Key { keys: vec!["right".into()] })).is_none());
        assert!(knob_progress_preset(&knob(ha("toggle", "light.desk"))).is_none());
        // live limits replace the fallback ones
        let p = knob_progress_preset(&knob(ha("temperature", "climate.lr"))).unwrap();
        let mut st = std::collections::HashMap::new();
        st.insert("ha:climate.lr:attr:temperature".to_string(), "20".to_string());
        assert_eq!(p.fraction_in(&st), Some(13.0 / 28.0));
        st.insert("ha:climate.lr:attr:min_temp".to_string(), "10".to_string());
        st.insert("ha:climate.lr:attr:max_temp".to_string(), "30".to_string());
        assert_eq!(p.fraction_in(&st), Some(0.5));
        let mut page = Page::default();
        // automatic: no progress set, the turn action decides
        page.knobs.push(knob(ha("temperature", "climate.lr")));
        assert!(page.knobs[0].indicator().is_some());
        assert!(KnobDef { progress: Some(Progress { style: "none".into(), ..p.clone() }), ..page.knobs[0].clone() }.indicator().is_none());
        let mut pages = BTreeMap::new();
        pages.insert("main".to_string(), page);
        let cfg: Config = serde_json::from_value(serde_json::json!({ "pages": {} })).unwrap();
        let cfg = Config { pages, ..cfg };
        assert!(cfg.wanted_states().contains(&"ha:climate.lr:attr:max_temp".to_string()));
    }
}
