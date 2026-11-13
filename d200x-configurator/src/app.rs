//! Application state and deck operations for the configurator.

use crate::ha;
use d200x_core::Deck;
use deck_config::{Action, Config, KeyDef, KnobDef, Look, Page};
use eframe::egui::{self, Color32};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sel {
    Key(usize),
    Knob(usize),
    SideLeft,
    SideRight,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Keyboard,
    Pages,
    Apps,
    System,
    Media,
    HomeAssistant,
    Obs,
    Spotify,
    Discord,
    AgentDeck,
    Minecraft,
}

pub const LIBRARY_TABS: [(LibraryTab, &str); 11] = [
    (LibraryTab::Keyboard, "Keyboard"),
    (LibraryTab::Pages, "Pages"),
    (LibraryTab::Apps, "Apps"),
    (LibraryTab::System, "System"),
    (LibraryTab::Media, "Media"),
    (LibraryTab::HomeAssistant, "Home Assistant"),
    (LibraryTab::Obs, "OBS"),
    (LibraryTab::Spotify, "Spotify"),
    (LibraryTab::Discord, "Discord"),
    (LibraryTab::AgentDeck, "AgentDeck"),
    (LibraryTab::Minecraft, "Minecraft"),
];

/// Frames of a full-screen animation are rendered at this size (scaled up by the deck): a 960x540
/// PNG per frame would not fit the 3.5 MiB icon store, 480x270 keeps a 40-frame GIF under 1 MiB.
pub const ANIM_W: u32 = 480;
pub const ANIM_H: u32 = 270;
pub const ANIM_MAX_FRAMES: usize = 40;

pub struct App {
    pub cfg: Config,
    pub page: String,
    pub sel: Sel,
    /// Open the inspector's action section once (the library assigned something that needs input).
    pub reveal_action: bool,
    pub lib_tab: LibraryTab,
    pub status: String,
    pub status_ok: bool,
    pub device_info: String,
    pub connected: bool,
    pub dirty: bool,
    // icons
    pub icon_dir: PathBuf,
    pub textures: HashMap<String, egui::TextureHandle>,
    pub icons_known: Vec<String>,
    pub fonts_known: Vec<String>,
    pub pending_icons: BTreeMap<String, Vec<u8>>,
    // windows
    pub show_log: bool,
    pub log_text: String,
    pub show_settings: bool,
    pub show_mdi: bool,
    pub mdi_target: MdiTarget,
    pub show_fw: bool,
    pub fw_log: Arc<Mutex<String>>,
    pub fw_running: Arc<AtomicBool>,
    pub fw_confirm: Option<&'static str>,
    pub calibrating: bool,
    // agent
    pub agent_cfg: ha::AgentConfig,
    pub agent_running: Arc<AtomicBool>,
    /// The agent starts at Windows sign-in (read once at start, see `set_autostart`).
    pub agent_autostart: bool,
    /// Running applications with a window (refreshed on demand in a background thread).
    pub running_apps: Arc<Mutex<Vec<String>>>,
    // plugin data
    pub ha_entities: Vec<ha::Entity>,
    pub ha_filter: String,
    pub ha_domain: String,
    pub obs_scenes: Vec<String>,
    pub obs_inputs: Vec<String>,
    pub spotify_login: Arc<Mutex<Option<String>>>,
    // editors
    pub macro_text: String,
    pub macro_for: Sel,
    pub recording_hotkey: bool,
    pub mdi_names: Vec<String>,
    pub mdi_filter: String,
    pub mdi_color: Color32,
    // Minecraft item picker (same target routing as the MDI picker)
    pub show_mc: bool,
    pub mc_names: Vec<String>,
    pub states_ui: crate::ui_states::StateExplorer,
    pub mc_filter: String,
    pub new_rule: String,
    pub text_buf: String,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    pub pending_connect: Option<Arc<Mutex<Option<Result<String, String>>>>>,
    /// The last session's configuration was restored at start (otherwise the deck's is loaded on connect).
    pub restored: bool,
    pub last_autosave: std::time::Instant,
    /// Copied key (Ctrl+C / context menu), pasted onto any slot of any page.
    pub clipboard: Option<KeyDef>,
    // ESP32 bridge window (ui_bridge.rs)
    pub show_bridge: bool,
    pub bridge_busy: Arc<AtomicBool>,
    pub bridge_log: Arc<Mutex<String>>,
    /// Last status from the bridge (or {"error":..}); None until checked.
    pub bridge_status: Arc<Mutex<Option<serde_json::Value>>>,
    /// Plugins handed over to the bridge (checkbox state).
    pub bridge_handoff: BTreeMap<String, bool>,
    pub bridge_ssid: String,
    /// Only in memory, sent once to the bridge.
    pub bridge_pass: String,
    /// COM port override for USB flashing ("" = find the XIAO automatically).
    pub bridge_port: String,
}

impl App {
    /// Copy the key in `slot` of the current page to the clipboard.
    pub fn copy_key(&mut self, slot: usize) {
        let k = self.page_ref().keys.get(&slot.to_string()).cloned().unwrap_or_default();
        self.clipboard = Some(k);
        self.set_status(format!("key {} copied", slot + 1), true);
    }

    /// Copy the key and clear its slot.
    pub fn cut_key(&mut self, slot: usize) {
        self.copy_key(slot);
        self.current_page().keys.insert(slot.to_string(), KeyDef::default());
        self.dirty = true;
        self.set_status(format!("key {} cut", slot + 1), true);
    }

    /// Paste the clipboard onto `slot` of the current page (icons are already in the store).
    pub fn paste_key(&mut self, slot: usize) {
        let Some(k) = self.clipboard.clone() else {
            self.set_status("nothing copied yet (select a key, Ctrl+C)", false);
            return;
        };
        self.current_page().keys.insert(slot.to_string(), k);
        self.sel = Sel::Key(slot);
        self.dirty = true;
        self.macro_for = Sel::None;
        self.set_status(format!("pasted onto key {}", slot + 1), true);
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum MdiTarget {
    KeyIcon,
    LookIcon(String),
    None,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::default_config();
        let page = cfg.home.clone();
        let icon_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("d200x").join("icons");
        let _ = std::fs::create_dir_all(&icon_dir);
        let mut app = App {
            cfg,
            page,
            sel: Sel::None,
            reveal_action: false,
            lib_tab: LibraryTab::Keyboard,
            status: "not connected".into(),
            status_ok: false,
            device_info: String::new(),
            connected: false,
            dirty: false,
            icon_dir,
            textures: HashMap::new(),
            icons_known: Vec::new(),
            fonts_known: Vec::new(),
            pending_icons: BTreeMap::new(),
            show_log: false,
            log_text: String::new(),
            show_settings: false,
            show_mdi: false,
            mdi_target: MdiTarget::None,
            show_fw: false,
            fw_log: Arc::new(Mutex::new(String::new())),
            fw_running: Arc::new(AtomicBool::new(false)),
            fw_confirm: None,
            calibrating: false,
            agent_cfg: ha::load_agent_config(),
            agent_running: Arc::new(AtomicBool::new(false)),
            agent_autostart: autostart_enabled(),
            running_apps: Arc::new(Mutex::new(Vec::new())),
            ha_entities: Vec::new(),
            ha_filter: String::new(),
            ha_domain: "all".into(),
            obs_scenes: Vec::new(),
            obs_inputs: Vec::new(),
            spotify_login: Arc::new(Mutex::new(None)),
            macro_text: String::new(),
            macro_for: Sel::None,
            recording_hotkey: false,
            mdi_names: deck_icons::mdi_names(),
            mdi_filter: String::new(),
            mdi_color: Color32::WHITE,
            show_mc: false,
            mc_names: deck_icons::minecraft_names(),
            states_ui: Default::default(),
            mc_filter: String::new(),
            new_rule: String::new(),
            text_buf: String::new(),
            thumbs: HashMap::new(),
            pending_connect: None,
            restored: false,
            last_autosave: std::time::Instant::now(),
            clipboard: None,
            show_bridge: false,
            bridge_busy: Arc::new(AtomicBool::new(false)),
            bridge_log: Arc::new(Mutex::new(String::new())),
            bridge_status: Arc::new(Mutex::new(None)),
            bridge_handoff: BTreeMap::new(),
            bridge_ssid: String::new(),
            bridge_pass: String::new(),
            bridge_port: String::new(),
        };
        app.collect_icons();
        app.restored = app.load_local();
        if app.restored {
            app.set_status("configuration restored from the last session", true);
        }
        // agent watcher
        let flag = app.agent_running.clone();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || loop {
            let running = agent_process_running();
            if flag.load(Ordering::Relaxed) != running {
                flag.store(running, Ordering::Relaxed);
                ctx.request_repaint();
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        });
        // connect in the background so the window never waits
        app.connect_async(cc.egui_ctx.clone());
        app
    }

    /// Local copy of the deck configuration, restored at the next start.
    pub fn local_config_path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("d200x").join("config.json")
    }

    /// Restore the last session's configuration; false when there is none.
    pub fn load_local(&mut self) -> bool {
        let Ok(bytes) = std::fs::read(Self::local_config_path()) else { return false };
        match serde_json::from_slice::<Config>(&bytes) {
            Ok(c) => {
                self.cfg = c;
                if !self.cfg.pages.contains_key(&self.page) {
                    self.page = self.cfg.home.clone();
                }
                self.collect_icons();
                true
            }
            Err(_) => false,
        }
    }

    pub fn save_local(&self) {
        let p = Self::local_config_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(json) = serde_json::to_vec_pretty(&self.cfg) {
            let _ = std::fs::write(p, json);
        }
    }

    pub fn set_status(&mut self, s: impl Into<String>, ok: bool) {
        self.status = s.into();
        self.status_ok = ok;
    }

    pub fn collect_icons(&mut self) {
        let mut set = std::collections::BTreeSet::new();
        for p in self.cfg.pages.values() {
            for k in p.keys.values() {
                if let Some(i) = &k.icon {
                    set.insert(i.clone());
                }
                for l in k.looks.values() {
                    if let Some(i) = &l.icon {
                        set.insert(i.clone());
                    }
                }
            }
        }
        for i in &self.icons_known {
            set.insert(i.clone());
        }
        if let Ok(rd) = std::fs::read_dir(&self.icon_dir) {
            for e in rd.flatten() {
                if let Some(n) = e.file_name().to_str() {
                    if n.ends_with(".png") {
                        set.insert(n.to_string());
                    }
                }
            }
        }
        self.icons_known = set.into_iter().collect();
        let mut fonts: std::collections::BTreeSet<String> = self.cfg.pages.values().flat_map(|p| p.keys.values()).filter_map(|k| k.font.clone()).collect();
        if let Ok(rd) = std::fs::read_dir(&self.icon_dir) {
            fonts.extend(rd.flatten().filter_map(|e| e.file_name().to_str().filter(|n| n.ends_with(".ttf")).map(str::to_string)));
        }
        self.fonts_known = fonts.into_iter().collect();
    }

    /// Pick a TTF and register it as a deck font (`font-<stem>.ttf`, uploaded like an icon).
    pub fn pick_font(&mut self) -> Option<String> {
        let path = rfd::FileDialog::new().add_filter("TrueType font", &["ttf"]).set_directory("C:\\Windows\\Fonts").pick_file()?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()).unwrap_or_else(|| "font".into());
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.set_status(format!("{e}"), false);
                return None;
            }
        };
        if bytes.len() > 1_500_000 {
            self.set_status(format!("font is {} KB: the deck has little RAM, use a font under 1.5 MB (no CJK fonts)", bytes.len() / 1024), false);
            return None;
        }
        let name = format!("font-{stem}.ttf");
        self.add_icon(&name, bytes);
        if !self.fonts_known.contains(&name) {
            self.fonts_known.push(name.clone());
        }
        self.set_status(format!("{name} ready (uploaded on save)"), true);
        Some(name)
    }

    /// Register a rendered icon: cache locally, queue for upload.
    pub fn add_icon(&mut self, name: &str, png: Vec<u8>) {
        let _ = std::fs::write(self.icon_dir.join(name), &png);
        self.textures.remove(name);
        self.pending_icons.insert(name.to_string(), png);
        if !self.icons_known.contains(&name.to_string()) {
            self.icons_known.push(name.to_string());
        }
        self.dirty = true;
    }

    /// Texture for an icon (from the local cache), loaded lazily.
    pub fn texture(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(name) {
            return Some(t.clone());
        }
        let bytes = std::fs::read(self.icon_dir.join(name)).ok()?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let t = ctx.load_texture(name.to_string(), ci, egui::TextureOptions::LINEAR);
        self.textures.insert(name.to_string(), t.clone());
        Some(t)
    }

    /// Small rendered preview of an MDI icon for the picker.
    pub fn mdi_thumb(&mut self, ctx: &egui::Context, name: &str, color: &str) -> Option<egui::TextureHandle> {
        let key = format!("{name}|{color}");
        if let Some(t) = self.thumbs.get(&key) {
            return Some(t.clone());
        }
        let png = deck_icons::render_mdi(name, color, 36).ok()?;
        let img = image::load_from_memory(&png).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let t = ctx.load_texture(key.clone(), ci, egui::TextureOptions::LINEAR);
        if self.thumbs.len() > 1500 {
            self.thumbs.clear();
        }
        self.thumbs.insert(key, t.clone());
        Some(t)
    }

    /// Small pixel-art preview of a Minecraft item for the picker.
    pub fn mc_thumb(&mut self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        let key = format!("mc:{name}");
        if let Some(t) = self.thumbs.get(&key) {
            return Some(t.clone());
        }
        let png = deck_icons::render_minecraft(name, 32).ok()?;
        let img = image::load_from_memory(&png).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let t = ctx.load_texture(key.clone(), ci, egui::TextureOptions::NEAREST);
        if self.thumbs.len() > 1500 {
            self.thumbs.clear();
        }
        self.thumbs.insert(key, t.clone());
        Some(t)
    }

    /// Render a Minecraft item icon into the local cache + pending uploads; returns the file name.
    pub fn mc_icon(&mut self, name: &str) -> Option<String> {
        let file = deck_icons::minecraft_file_name(name);
        if self.icon_dir.join(&file).exists() && !self.pending_icons.contains_key(&file) {
            if let Ok(b) = std::fs::read(self.icon_dir.join(&file)) {
                self.pending_icons.insert(file.clone(), b);
            }
            return Some(file);
        }
        match deck_icons::render_minecraft(name, deck_icons::DEFAULT_SIZE) {
            Ok(png) => {
                self.add_icon(&file, png);
                Some(file)
            }
            Err(e) => {
                self.set_status(format!("{e}"), false);
                None
            }
        }
    }

    /// Import the item textures from the local Minecraft installation (or a chosen jar).
    pub fn import_minecraft(&mut self, jar: Option<PathBuf>) {
        let Some(jar) = jar.or_else(deck_icons::find_minecraft_jar) else {
            self.set_status("no Minecraft installation found: pick the client .jar (versions/<v>/<v>.jar)", false);
            return;
        };
        let out = deck_icons::minecraft_default_dir();
        match deck_icons::import_minecraft(&jar, &out) {
            Ok(n) => {
                self.mc_names = deck_icons::minecraft_names();
                self.thumbs.retain(|k, _| !k.starts_with("mc:"));
                self.set_status(format!("{n} Minecraft item icons imported from {}", jar.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()), true);
            }
            Err(e) => self.set_status(format!("Minecraft import failed: {e:#}"), false),
        }
    }

    /// Pick a GIF (or still image) and register its frames as a full-screen animation
    /// `<prefix>-<stem>-f00.png` ...; returns (base name, frame count, delay in ms).
    pub fn pick_animation(&mut self, prefix: &str) -> Option<(String, u32, u64)> {
        let path = rfd::FileDialog::new().add_filter("animation", &["gif", "png", "jpg", "jpeg"]).pick_file()?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()).unwrap_or_else(|| "anim".into());
        let base = format!("{prefix}-{stem}");
        match deck_icons::render_animation(&path, ANIM_W, ANIM_H, ANIM_MAX_FRAMES) {
            Ok(frames) if !frames.is_empty() => {
                let n = frames.len();
                let delay = (frames.iter().map(|f| f.delay_ms).sum::<u64>() / n as u64).clamp(20, 2000);
                let bytes: usize = frames.iter().map(|f| f.png.len()).sum();
                for (i, f) in frames.into_iter().enumerate() {
                    self.add_icon(&deck_icons::frame_file_name(&base, i), f.png);
                }
                self.set_status(format!("{base}: {n} frames, {} ms/frame, {} KB (uploaded on save)", delay, bytes / 1024), true);
                Some((base, n as u32, delay))
            }
            Ok(_) => {
                self.set_status("no frames in that file", false);
                None
            }
            Err(e) => {
                self.set_status(format!("{e:#}"), false);
                None
            }
        }
    }

    /// Pick an image file and register it as a 960x540 background / screen image.
    pub fn pick_fullscreen_image(&mut self, prefix: &str) -> Option<String> {
        let path = rfd::FileDialog::new().add_filter("images", &["png", "jpg", "jpeg", "svg"]).pick_file()?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()).unwrap_or_else(|| "image".into());
        let name = format!("{prefix}-{stem}.png");
        match deck_icons::render_file(&path, 960, 540) {
            Ok(png) => {
                self.add_icon(&name, png);
                self.set_status(format!("{name} ready (uploaded on save)"), true);
                Some(name)
            }
            Err(e) => {
                self.set_status(format!("{e}"), false);
                None
            }
        }
    }

    pub fn connect_async(&mut self, ctx: egui::Context) {
        self.set_status("connecting...", false);
        let result = Arc::new(Mutex::new(None));
        let r2 = result.clone();
        std::thread::spawn(move || {
            let r = Deck::open().and_then(|d| d.get_device_info());
            *r2.lock().unwrap() = Some(r.map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
        self.pending_connect = Some(result);
    }

    pub fn poll_connect(&mut self) {
        let Some(p) = self.pending_connect.clone() else { return };
        let Some(r) = p.lock().unwrap().take() else { return };
        self.pending_connect = None;
        match r {
            Ok(info) => {
                self.connected = true;
                let deckd = info.contains("\"firmware\":\"deckd\"");
                self.device_info = info;
                self.set_status(if deckd { "connected to deckd" } else { "connected: stock Ulanzi firmware (install deckd via Firmware)" }, deckd);
                // nothing restored locally: the deck's configuration is the starting point
                if deckd && !self.restored && !self.dirty {
                    self.load_from_deck();
                    self.restored = true;
                }
            }
            Err(e) => {
                self.connected = false;
                self.device_info.clear();
                self.set_status(format!("not connected: {e}"), false);
            }
        }
    }

    pub fn load_from_deck(&mut self) {
        match Deck::open().and_then(|d| d.get_config()) {
            Ok(json) => match serde_json::from_str::<Config>(&json) {
                Ok(c) => {
                    self.cfg = c;
                    if !self.cfg.pages.contains_key(&self.page) {
                        self.page = self.cfg.home.clone();
                    }
                    self.collect_icons();
                    self.dirty = false;
                    self.save_local();
                    self.set_status("configuration loaded from the deck", true);
                }
                Err(e) => self.set_status(format!("deck config not understood: {e}"), false),
            },
            Err(e) => self.set_status(format!("load failed: {e}"), false),
        }
    }

    pub fn save_to_deck(&mut self) {
        let deck = match Deck::open() {
            Ok(d) => d,
            Err(e) => {
                self.set_status(format!("save failed: {e}"), false);
                return;
            }
        };
        // upload pending icons first
        let pending: Vec<(String, Vec<u8>)> = self.pending_icons.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (name, png) in pending {
            match deck.set_icon(&name, &png) {
                Ok(r) if r.contains("\"ok\":true") => {
                    self.pending_icons.remove(&name);
                }
                Ok(r) => {
                    self.set_status(format!("icon {name} rejected: {r}"), false);
                    return;
                }
                Err(e) => {
                    self.set_status(format!("icon {name} upload failed: {e}"), false);
                    return;
                }
            }
        }
        let json = match serde_json::to_vec_pretty(&self.cfg) {
            Ok(j) => j,
            Err(e) => {
                self.set_status(format!("serialize failed: {e}"), false);
                return;
            }
        };
        match deck.set_config(&json) {
            Ok(reply) if reply.contains("\"ok\":true") => {
                self.dirty = false;
                self.save_local();
                self.set_status("saved to the deck", true);
            }
            Ok(reply) => self.set_status(format!("deck rejected the config: {reply}"), false),
            Err(e) => self.set_status(format!("save failed: {e}"), false),
        }
    }

    pub fn fetch_log(&mut self) {
        match Deck::open().and_then(|d| d.get_log()) {
            Ok(t) => {
                self.log_text = t;
                self.show_log = true;
            }
            Err(e) => self.set_status(format!("log failed: {e}"), false),
        }
    }

    pub fn toggle_calibrate(&mut self) {
        let on = !self.calibrating;
        match Deck::open().and_then(|d| d.calibrate(on)) {
            Ok(_) => {
                self.calibrating = on;
                self.set_status(if on { "centering rings shown on the deck" } else { "rings off" }, true);
            }
            Err(e) => self.set_status(format!("calibrate failed: {e}"), false),
        }
    }

    pub fn current_page(&mut self) -> &mut Page {
        self.cfg.pages.entry(self.page.clone()).or_default()
    }

    pub fn page_ref(&self) -> Page {
        self.cfg.pages.get(&self.page).cloned().unwrap_or_default()
    }

    pub fn selected_key(&mut self) -> Option<&mut KeyDef> {
        if let Sel::Key(i) = self.sel {
            Some(self.current_page().keys.entry(i.to_string()).or_insert_with(KeyDef::default))
        } else {
            None
        }
    }

    pub fn selected_knob(&mut self) -> Option<&mut KnobDef> {
        if let Sel::Knob(k) = self.sel {
            let page = self.current_page();
            while page.knobs.len() < 3 {
                page.knobs.push(KnobDef::default());
            }
            Some(&mut page.knobs[k])
        } else {
            None
        }
    }

    /// Slot for a new key: selected key, else first empty slot.
    pub fn target_slot(&mut self) -> usize {
        if let Sel::Key(i) = self.sel {
            return i;
        }
        let page = self.page_ref();
        (0..13).find(|i| page.keys.get(&i.to_string()).map(|k| k.label.is_empty() && matches!(k.action, Action::None)).unwrap_or(true)).unwrap_or(0)
    }

    pub fn put_key(&mut self, key: KeyDef) {
        let slot = self.target_slot();
        self.current_page().keys.insert(slot.to_string(), key);
        self.sel = Sel::Key(slot);
        self.dirty = true;
        self.macro_for = Sel::None;
    }

    pub fn put_knob(&mut self, knob: KnobDef) {
        let idx = if let Sel::Knob(i) = self.sel { i } else { 0 };
        let page = self.current_page();
        while page.knobs.len() < 3 {
            page.knobs.push(KnobDef::default());
        }
        page.knobs[idx] = knob;
        self.sel = Sel::Knob(idx);
        self.dirty = true;
    }

    /// Render an MDI icon into the local cache + pending uploads; returns the file name.
    pub fn mdi_icon(&mut self, name: &str, color: &str) -> Option<String> {
        let file = deck_icons::mdi_file_name(name, color);
        if self.icon_dir.join(&file).exists() && !self.pending_icons.contains_key(&file) {
            // already cached locally: still queue so a fresh deck gets it
            if let Ok(b) = std::fs::read(self.icon_dir.join(&file)) {
                self.pending_icons.insert(file.clone(), b);
            }
            return Some(file);
        }
        match deck_icons::render_mdi(name, color, deck_icons::DEFAULT_SIZE) {
            Ok(png) => {
                self.add_icon(&file, png);
                Some(file)
            }
            Err(e) => {
                self.set_status(format!("{e}"), false);
                None
            }
        }
    }

    /// A key with two looks (on/off) driven by a boolean-like state.
    pub fn toggle_key(&mut self, label: &str, state: &str, on_icon: &str, off_icon: &str, on_color: &str, action: Action) -> KeyDef {
        let on = self.mdi_icon(on_icon, "#f5b400");
        let off = self.mdi_icon(off_icon, "#9aa5b1");
        let mut looks = BTreeMap::new();
        looks.insert("true|on|playing|recording".into(), Look { icon: on, color: Some(on_color.into()), ..Default::default() });
        looks.insert("*".into(), Look { icon: off.clone(), color: Some("#1c2b3a".into()), ..Default::default() });
        KeyDef { label: label.into(), icon: off, color: Some("#1c2b3a".into()), text_color: None, action, state: Some(state.into()), looks, value: None, progress: None, tile: None, effect: None, color_state: None, font_size: None, align: None, valign: None, font: None }
    }

    /// A simple action key with one icon.
    pub fn action_key(&mut self, label: &str, icon: &str, color: &str, action: Action) -> KeyDef {
        let file = self.mdi_icon(icon, "#ffffff");
        KeyDef { label: label.into(), icon: file, color: Some(color.into()), text_color: None, action, state: None, looks: BTreeMap::new(), value: None, progress: None, tile: None, effect: None, color_state: None, font_size: None, align: None, valign: None, font: None }
    }

    /// A key with a Minecraft item icon.
    pub fn mc_key(&mut self, label: &str, item: &str, color: &str, action: Action) -> KeyDef {
        let file = self.mc_icon(item);
        KeyDef { label: label.into(), icon: file, color: Some(color.into()), text_color: None, action, state: None, looks: BTreeMap::new(), value: None, progress: None, tile: None, effect: None, color_state: None, font_size: None, align: None, valign: None, font: None }
    }

    pub fn host(plugin: &str, op: &str, args: serde_json::Value) -> Action {
        Action::Host { plugin: plugin.into(), op: op.into(), args, fallback: None }
    }

    /// Refresh the list of running applications in the background.
    pub fn refresh_running_apps(&self) {
        let out = self.running_apps.clone();
        std::thread::spawn(move || {
            let v = crate::presets::running_apps();
            *out.lock().unwrap() = v;
        });
    }

    /// Create (or replace) the page of an application preset and select it.
    /// Returns the page name.
    pub fn apply_preset(&mut self, p: &crate::presets::AppPreset) -> String {
        let name = p.page.to_string();
        let mut page = Page::default();
        for (i, k) in p.keys.iter().take(13).enumerate() {
            let key = self.action_key(k.label, k.icon, p.color, Action::Key { keys: k.keys.iter().map(|s| s.to_string()).collect() });
            page.keys.insert(i.to_string(), key);
        }
        page.app = Some(p.apps.to_string());
        self.cfg.pages.insert(name.clone(), page);
        if !self.cfg.page_order.contains(&name) {
            let mut o = self.cfg.order();
            o.push(name.clone());
            self.cfg.page_order = o;
        }
        self.page = name.clone();
        self.sel = Sel::None;
        self.dirty = true;
        name
    }
}

pub fn agent_process_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq d200x-agent.exe", "/NH"]).creation_flags(0x0800_0000).output();
        return out.map(|o| String::from_utf8_lossy(&o.stdout).contains("d200x-agent.exe")).unwrap_or(false);
    }
    #[allow(unreachable_code)]
    false
}

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "D200X Agent";

fn reg(args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut c = std::process::Command::new("reg");
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output()
}

/// Is the agent started at Windows sign-in (the Run entry exists)?
pub fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", RUN_VALUE]).map(|o| o.status.success()).unwrap_or(false)
}

/// Add or remove the sign-in entry. It runs this configurator with `--start-agent`, which starts
/// the agent without a console window and exits (the agent itself is a console program).
pub fn set_autostart(on: bool) -> Result<(), String> {
    let out = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" --start-agent", exe.display());
        reg(&["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &cmd, "/f"])
    } else {
        reg(&["delete", RUN_KEY, "/v", RUN_VALUE, "/f"])
    }
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn agent_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("d200x-agent.exe"), dir.join("../../../d200x-agent/target/release/d200x-agent.exe"), dir.join("../../d200x-agent/target/release/d200x-agent.exe")]
        .into_iter()
        .find(|p| p.exists())
}

pub fn repo_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut d = exe.parent()?.to_path_buf();
    for _ in 0..4 {
        if d.join("research").join("adb-install.ps1").exists() {
            return Some(d);
        }
        d = d.parent()?.to_path_buf();
    }
    None
}

pub fn parse_color(s: &Option<String>, fallback: Color32) -> Color32 {
    let Some(s) = s else { return fallback };
    let h = s.trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        }
    }
    fallback
}

pub fn color_to_string(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

pub fn effect_summary(e: &deck_config::Effect) -> String {
    match e {
        deck_config::Effect::LevelUp { .. } => "level up".into(),
        deck_config::Effect::Wave { color, from, .. } => format!("wave {} from {from}", color.as_deref().unwrap_or("#80ff20")),
        deck_config::Effect::Frames { name, frames, .. } => format!("{name} ({frames} frames)"),
    }
}

pub fn action_summary(a: &Action) -> String {
    match a {
        Action::None => String::new(),
        Action::Key { keys } => keys.join("+"),
        Action::Text { text } => format!("\"{}\"", text.trim_end().chars().take(18).collect::<String>()),
        Action::Page { page } => format!("page {page}"),
        Action::PageNext => "next page".into(),
        Action::PagePrev => "previous page".into(),
        Action::Macro { steps } => format!("macro ({} steps)", steps.len()),
        Action::Screen { image, frames, .. } => if *frames > 0 { format!("play {image} ({frames} frames)") } else { format!("show {image}") },
        Action::Effect { effect } => format!("effect: {}", effect_summary(effect)),
        Action::Host { plugin, op, args, .. } => {
            let e = args.get("entity_id").and_then(|v| v.as_str()).or_else(|| args.get("name").and_then(|v| v.as_str())).or_else(|| args.get("url").and_then(|v| v.as_str())).or_else(|| args.get("key").and_then(|v| v.as_str())).unwrap_or("");
            format!("{plugin}: {op} {e}").trim().to_string()
        }
    }
}
