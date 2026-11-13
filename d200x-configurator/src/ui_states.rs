//! State explorer window: pick a plugin (and entity), see every state with its live value and what
//! it can be, and put it on the selected key.

use crate::states::{self, Row, Snapshot, HOME_ASSISTANT, PLUGINS};
use eframe::egui::{self, Color32, RichText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(2);
/// The bridge answers through the deck's 115200 baud console link: keep that link free for keys.
const BRIDGE_POLL: Duration = Duration::from_secs(10);

/// The state fields of a key.
#[derive(Clone, Copy, PartialEq)]
pub enum StateField {
    KeyState,
    Label,
    Background,
    Ring,
    RingColor,
}

impl StateField {
    fn title(self) -> &'static str {
        match self {
            StateField::KeyState => "State",
            StateField::Label => "Label from state",
            StateField::Background => "Background from state",
            StateField::Ring => "Ring state",
            StateField::RingColor => "Ring colour from state",
        }
    }
}

/// What to do with a state on the selected key.
pub enum Use {
    Set(StateField, String),
    /// Add a look for this value of the key's state.
    Look(String),
}

/// A field asking for the picker, with the state to start at.
pub type PickRequest = Option<(StateField, Option<String>)>;

/// The "pick" button next to a state field. `current` is the field's value, `fallback` where to
/// start while it is empty: rings and colours usually come from the key's own entity.
pub fn pick_button(ui: &mut egui::Ui, field: StateField, current: Option<&str>, fallback: Option<&str>, request: &mut PickRequest) {
    if ui.small_button("pick").on_hover_text("choose from every plugin's states, with live values").clicked() {
        *request = Some((field, current.filter(|s| !s.is_empty()).or(fallback).map(str::to_string)));
    }
}

/// The selected key, as far as the window needs to know it.
#[derive(Default)]
pub struct KeyTarget {
    pub label: Option<String>,
    pub state: Option<String>,
    pub has_ring: bool,
}

#[derive(Default)]
pub struct StateExplorer {
    pub open: bool,
    plugin: usize,
    filter: String,
    entity: Option<String>,
    selected: Option<String>,
    /// The field this window was opened for; choosing a state fills it and closes the window.
    pick: Option<StateField>,
    /// Both agents merged; what the window shows.
    snapshot: Snapshot,
    agent: Snapshot,
    error: Option<String>,
    inbox: Arc<Mutex<Option<Result<Snapshot, String>>>>,
    busy: Arc<AtomicBool>,
    last_poll: Option<Instant>,
    /// Ask the ESP32 bridge too (it is installed and the deck is connected). Set every frame.
    pub bridge_enabled: bool,
    bridge: Option<Snapshot>,
    bridge_error: Option<String>,
    bridge_inbox: Arc<Mutex<Option<Result<Snapshot, String>>>>,
    bridge_busy: Arc<AtomicBool>,
    last_bridge: Option<Instant>,
}

impl StateExplorer {
    /// Open the window, showing `state` when there is one.
    pub fn open_at(&mut self, state: Option<&str>) {
        self.open = true;
        let Some(state) = state.filter(|s| !s.is_empty()) else { return };
        if let Some(p) = states::plugin_of(state) {
            self.plugin = p;
            if p == HOME_ASSISTANT {
                self.entity = state.strip_prefix("ha:").map(|s| s.split(':').next().unwrap_or("").to_string());
            }
        }
        self.filter.clear();
        self.selected = Some(state.to_string());
    }

    pub fn pick_for(&mut self, field: StateField, start: Option<&str>) {
        self.open_at(start);
        self.pick = Some(field);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.inbox.lock().unwrap().take() {
            match r {
                Ok(s) => {
                    self.agent = s;
                    self.error = None;
                }
                Err(e) => {
                    self.agent = Snapshot::default();
                    self.error = Some(e);
                }
            }
        }
        if let Some(r) = self.bridge_inbox.lock().unwrap().take() {
            match r {
                Ok(s) => {
                    self.bridge = Some(s);
                    self.bridge_error = None;
                }
                Err(e) => self.bridge_error = Some(e),
            }
        }
        if !self.bridge_enabled {
            self.bridge = None;
            self.bridge_error = None;
        }
        self.snapshot = states::merge(&self.agent, self.bridge.as_ref());
        if self.bridge_enabled && self.last_bridge.map(|t| t.elapsed() >= BRIDGE_POLL).unwrap_or(true) && !self.bridge_busy.swap(true, Ordering::SeqCst) {
            self.last_bridge = Some(Instant::now());
            let (inbox, busy, ctx2) = (self.bridge_inbox.clone(), self.bridge_busy.clone(), ctx.clone());
            std::thread::spawn(move || {
                *inbox.lock().unwrap() = Some(states::fetch_bridge());
                busy.store(false, Ordering::SeqCst);
                ctx2.request_repaint();
            });
        }
        if self.last_poll.map(|t| t.elapsed() >= POLL).unwrap_or(true) && !self.busy.swap(true, Ordering::SeqCst) {
            self.last_poll = Some(Instant::now());
            let (inbox, busy, ctx2) = (self.inbox.clone(), self.busy.clone(), ctx.clone());
            std::thread::spawn(move || {
                *inbox.lock().unwrap() = Some(states::fetch_snapshot());
                busy.store(false, Ordering::SeqCst);
                ctx2.request_repaint();
            });
        }
        ctx.request_repaint_after(POLL);
    }

    fn matches(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.filter.trim().to_lowercase())
    }

    fn rows(&self) -> Vec<Row> {
        if self.plugin == HOME_ASSISTANT {
            return self.entity.as_deref().map(|e| states::ha_rows(&self.snapshot, e)).unwrap_or_default();
        }
        let rows = match PLUGINS.get(self.plugin) {
            Some(p) => states::plugin_rows(p, &self.snapshot),
            None => states::extra_plugins(&self.snapshot).get(self.plugin - PLUGINS.len()).map(|n| states::extra_rows(n, &self.snapshot)).unwrap_or_default(),
        };
        rows.into_iter().filter(|r| self.matches(&r.name) || self.matches(&r.note)).collect()
    }

    /// Where a plugin runs, for the tab hint.
    fn runs_on(&self, id: &str) -> &'static str {
        match (self.snapshot.plugins.iter().any(|n| n == id), self.snapshot.bridge_plugins.iter().any(|n| n == id)) {
            (true, true) => "running in the agent and on the ESP32 bridge",
            (true, false) => "running in the agent",
            (false, true) => "running on the ESP32 bridge",
            (false, false) => "not running",
        }
    }
}

fn short(v: &str) -> String {
    let line = v.replace('\n', " ");
    if line.chars().count() > 34 {
        format!("{}...", line.chars().take(32).collect::<String>())
    } else {
        line
    }
}

/// Returns what the user wants done with the selected key this frame.
pub fn window(ctx: &egui::Context, ex: &mut StateExplorer, target: &KeyTarget) -> Option<Use> {
    if !ex.open {
        ex.pick = None;
        return None;
    }
    ex.poll(ctx);
    let mut open = true;
    let mut used = None;
    let title = ex.pick.map(|f| format!("Pick: {}", f.title())).unwrap_or_else(|| "States".to_string());
    egui::Window::new(title).id(egui::Id::new("states-window")).open(&mut open).default_size([820.0, 560.0]).min_width(620.0).min_height(380.0).show(ctx, |ui| {
        header(ui, ex);
        let rows = ex.rows();
        egui::TopBottomPanel::bottom("states-detail").resizable(false).min_height(150.0).show_inside(ui, |ui| {
            let row = rows.iter().find(|r| Some(&r.name) == ex.selected.as_ref());
            used = detail(ui, row, target, ex.pick);
        });
        if ex.plugin == HOME_ASSISTANT {
            egui::SidePanel::left("states-entities").resizable(true).default_width(280.0).show_inside(ui, |ui| entity_list(ui, ex));
        }
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let (Some(field), Some(name)) = (ex.pick, state_table(ui, ex, &rows)) {
                used = Some(Use::Set(field, name));
            }
        });
    });
    ex.open = open && !(ex.pick.is_some() && matches!(used, Some(Use::Set(..))));
    used
}

fn header(ui: &mut egui::Ui, ex: &mut StateExplorer) {
    let extras = states::extra_plugins(&ex.snapshot);
    if ex.plugin >= PLUGINS.len() + extras.len() {
        ex.plugin = 0;
    }
    ui.horizontal_wrapped(|ui| {
        let tabs = PLUGINS.iter().map(|p| (p.id.to_string(), p.title.to_string())).chain(extras.iter().map(|e| (e.clone(), e.clone())));
        for (i, (id, title)) in tabs.enumerate() {
            let runs = ex.runs_on(&id);
            let text = if runs != "not running" { RichText::new(&title) } else { RichText::new(&title).weak() };
            if ui.selectable_label(ex.plugin == i, text).on_hover_text(runs).clicked() {
                ex.plugin = i;
                ex.selected = None;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("filter");
        ui.add(egui::TextEdit::singleline(&mut ex.filter).desired_width(240.0).hint_text(if ex.plugin == HOME_ASSISTANT { "entity id or name" } else { "state name" }));
        if !ex.filter.is_empty() && ui.small_button("x").clicked() {
            ex.filter.clear();
        }
        let id = PLUGINS.get(ex.plugin).map(|p| p.id.to_string()).or_else(|| extras.get(ex.plugin - PLUGINS.len()).cloned()).unwrap_or_default();
        let runs = ex.runs_on(&id);
        if runs == "not running" {
            ui.label(RichText::new(format!("{id} is not running in the agent{}", if ex.bridge.is_some() { " or on the bridge" } else { "" })).weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(e) = &ex.bridge_error {
                ui.colored_label(Color32::LIGHT_RED, "bridge: no states").on_hover_text(e);
            } else if ex.bridge.is_some() {
                ui.label(RichText::new("bridge").color(Color32::LIGHT_GREEN)).on_hover_text("live values from the ESP32 bridge, every 10 s");
            } else if ex.bridge_enabled {
                ui.label(RichText::new("asking the bridge...").weak());
            }
            if ex.error.is_some() {
                ui.colored_label(Color32::LIGHT_RED, "agent not reachable");
            } else {
                ui.label(RichText::new("agent").color(Color32::LIGHT_GREEN)).on_hover_text("live values from the PC agent, every 2 s");
            }
        });
    });
    ui.separator();
}

fn entity_list(ui: &mut egui::Ui, ex: &mut StateExplorer) {
    let entities: Vec<states::HaEntity> = states::ha_entities(&ex.snapshot).into_iter().filter(|e| ex.matches(&e.id) || ex.matches(&e.name)).collect();
    ui.label(RichText::new(format!("{} entities", entities.len())).weak());
    let height = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y * 2.0;
    egui::ScrollArea::vertical().id_salt("states-entity-scroll").auto_shrink([false, false]).show_rows(ui, height, entities.len(), |ui, range| {
        for e in &entities[range] {
            let picked = ex.entity.as_deref() == Some(e.id.as_str());
            if ui.selectable_label(picked, short(&format!("{} · {} · {}", e.name, e.id.split('.').next().unwrap_or(""), e.state))).on_hover_text(&e.id).clicked() {
                ex.entity = Some(e.id.clone());
                ex.selected = Some(format!("ha:{}", e.id));
            }
        }
    });
}

/// Returns the state that was double-clicked.
fn state_table(ui: &mut egui::Ui, ex: &mut StateExplorer, rows: &[Row]) -> Option<String> {
    if ex.plugin == HOME_ASSISTANT && ex.entity.is_none() {
        ui.label(RichText::new("Pick an entity to see its states.").weak());
        return None;
    }
    let mut chosen = None;
    egui::ScrollArea::vertical().id_salt("states-table-scroll").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("states-table").num_columns(3).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new("state").strong());
            ui.label(RichText::new("now").strong());
            ui.label(RichText::new("about").strong());
            ui.end_row();
            for r in rows {
                let picked = ex.selected.as_ref() == Some(&r.name);
                // "ha:<id>" repeats on every line of an entity: show what differs
                let shown = ex.entity.as_ref().filter(|_| ex.plugin == HOME_ASSISTANT).and_then(|e| r.name.strip_prefix(&format!("ha:{e}"))).filter(|s| !s.is_empty()).unwrap_or(&r.name);
                let label = ui.selectable_label(picked, RichText::new(shown).monospace()).on_hover_text(&r.name);
                if label.clicked() {
                    ex.selected = Some(r.name.clone());
                }
                if label.double_clicked() && !r.template {
                    chosen = Some(r.name.clone());
                }
                match &r.value {
                    Some(v) if v.is_empty() => ui.label(RichText::new("(empty)").weak()),
                    Some(v) if v == "null" => ui.label(RichText::new("(no value)").weak()), // HA attribute, e.g. brightness of a light that is off
                    Some(v) => ui.label(short(v)).on_hover_text(v),
                    None => ui.label(RichText::new("-").weak()),
                };
                ui.label(RichText::new(&r.note).weak());
                ui.end_row();
            }
        });
    });
    chosen
}

fn detail(ui: &mut egui::Ui, row: Option<&Row>, target: &KeyTarget, pick: Option<StateField>) -> Option<Use> {
    let Some(r) = row else {
        ui.add_space(8.0);
        ui.label(RichText::new("Select a state to see what it can be and to use it on a key.").weak());
        return None;
    };
    let mut used = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&r.name).monospace().strong());
        if ui.add_enabled(!r.template, egui::Button::new("copy").small()).clicked() {
            ui.ctx().copy_text(r.name.clone());
        }
    });
    if r.template {
        ui.label(RichText::new("Shows up here with its real name once the plugin reports one.").weak());
    }
    if let Some(v) = r.value.as_ref().filter(|v| *v != "null") {
        let fits = states::value_fits(v);
        let hint = if fits.is_empty() { String::new() } else { format!("   (works for: {})", fits.join(", ")) };
        ui.label(format!("now: {}{hint}", short(v)));
    }
    let key_uses_state = target.state.as_deref() == Some(r.name.as_str());
    if !r.values.is_empty() {
        // a list can be long (a WLED light has 200 effects)
        egui::ScrollArea::vertical().id_salt("states-values-scroll").max_height(64.0).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("can be:");
                for v in &r.values {
                    let tip = if key_uses_state { "add a look for this value to the key" } else { "set this state on a key to add looks here" };
                    if ui.add_enabled(key_uses_state, egui::Button::new(v).small()).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                        used = Some(Use::Look(v.clone()));
                    }
                }
            });
        });
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if let Some(field) = pick {
            if ui.add_enabled(!r.template, egui::Button::new(RichText::new(format!("Use for {}", field.title())).strong())).clicked() {
                used = Some(Use::Set(field, r.name.clone()));
            }
            ui.label(RichText::new("or double-click a state").weak());
            return;
        }
        let Some(label) = &target.label else {
            ui.label(RichText::new("Select a key on the deck preview to use this state.").weak());
            return;
        };
        ui.label(if label.is_empty() { "use on the selected key:".to_string() } else { format!("use on \"{label}\":") });
        let mut button = |ui: &mut egui::Ui, text: &str, tip: &str, enabled: bool, field: StateField| {
            if ui.add_enabled(enabled && !r.template, egui::Button::new(text)).on_hover_text(tip).clicked() {
                used = Some(Use::Set(field, r.name.clone()));
            }
        };
        button(ui, "State", "picks the key's look", true, StateField::KeyState);
        button(ui, "Label", "shows the value as the key's label", true, StateField::Label);
        button(ui, "Background", "tile colour from this state", true, StateField::Background);
        button(ui, "Ring", "progress ring from this state", true, StateField::Ring);
        button(ui, "Ring colour", "needs a ring on the key", target.has_ring, StateField::RingColor);
    });
    used
}
