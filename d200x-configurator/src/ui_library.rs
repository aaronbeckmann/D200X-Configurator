//! The action library: one tab per plugin, each with a purpose-built panel
//! that assigns ready-made keys/knobs (with icons and looks) to the selection.

use crate::app::{App, LibraryTab, Sel, LIBRARY_TABS};
use crate::{ha, integrations};
use deck_config::{Action, KeyDef, KnobDef};
use eframe::egui::{self, RichText};
use serde_json::json;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (tab, name) in LIBRARY_TABS {
            ui.selectable_value(&mut app.lib_tab, tab, name);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let target = match app.sel {
                Sel::Key(i) => format!("assigning to key {}", i + 1),
                Sel::Knob(k) => format!("assigning to knob {}", k + 1),
                Sel::SideLeft => "assigning to left side button".into(),
                Sel::SideRight => "assigning to right side button".into(),
                Sel::None => "select a key or knob on the deck first".into(),
            };
            ui.label(RichText::new(target).weak());
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("library").show(ui, |ui| match app.lib_tab {
        LibraryTab::Keyboard => keyboard(app, ui),
        LibraryTab::Pages => pages(app, ui),
        LibraryTab::Apps => apps(app, ui),
        LibraryTab::System => system(app, ui),
        LibraryTab::Media => media(app, ui),
        LibraryTab::HomeAssistant => home_assistant(app, ui),
        LibraryTab::Obs => obs(app, ui),
        LibraryTab::Spotify => spotify(app, ui),
        LibraryTab::Discord => discord(app, ui),
        LibraryTab::AgentDeck => agentdeck(app, ui),
        LibraryTab::Minecraft => minecraft(app, ui),
    });
}

fn assign_action(app: &mut App, label: &str, icon: &str, color: &str, action: Action) {
    match app.sel {
        Sel::Key(_) | Sel::None => {
            let k = app.action_key(label, icon, color, action);
            app.put_key(k);
        }
        Sel::Knob(_) => {
            if let Some(knob) = app.selected_knob() {
                knob.press = action;
                if knob.label.is_empty() {
                    knob.label = label.into();
                }
            }
            app.dirty = true;
        }
        Sel::SideLeft => {
            app.cfg.side_left = action;
            app.dirty = true;
        }
        Sel::SideRight => {
            app.cfg.side_right = action;
            app.dirty = true;
        }
    }
    app.set_status(format!("assigned: {label}"), true);
}

fn assign_toggle(app: &mut App, label: &str, state: &str, on_icon: &str, off_icon: &str, on_color: &str, action: Action) {
    let k = app.toggle_key(label, state, on_icon, off_icon, on_color, action);
    app.put_key(k);
    app.set_status(format!("assigned: {label} (with on/off looks)"), true);
}

fn assign_knob(app: &mut App, label: &str, press: Action, left: Action, right: Action) {
    app.put_knob(KnobDef { label: label.into(), press, left, right, progress: None });
    app.set_status(format!("knob assigned: {label}"), true);
}

fn preset_button(ui: &mut egui::Ui, title: &str, hint: &str) -> bool {
    let r = ui.add_sized([150.0, 44.0], egui::Button::new(RichText::new(title).size(13.0)));
    r.on_hover_text(hint).clicked()
}

fn keyboard(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Keyboard shortcuts run on the deck itself, with no PC software.").weak());
    ui.horizontal_wrapped(|ui| {
        for (title, keys, icon) in [
            ("Copy", "ctrl+c", "content-copy"), ("Paste", "ctrl+v", "content-paste"), ("Cut", "ctrl+x", "content-cut"), ("Undo", "ctrl+z", "undo"), ("Redo", "ctrl+y", "redo"),
            ("Save", "ctrl+s", "content-save"), ("Find", "ctrl+f", "magnify"), ("Select all", "ctrl+a", "select-all"), ("New tab", "ctrl+t", "tab-plus"), ("Close tab", "ctrl+w", "close-box"),
            ("Alt+Tab", "alt+tab", "swap-horizontal"), ("Desktop", "gui+d", "monitor"), ("Lock PC", "gui+l", "lock"), ("Screenshot", "gui+shift+s", "camera"), ("Explorer", "gui+e", "folder"),
            ("Mute", "mute", "volume-mute"), ("Vol +", "volumeup", "volume-plus"), ("Vol -", "volumedown", "volume-minus"), ("Enter", "enter", "keyboard-return"), ("Esc", "esc", "keyboard-esc"),
        ] {
            if preset_button(ui, title, keys) {
                assign_action(app, title, icon, "#2d6cdf", Action::Key { keys: keys.split('+').map(str::to_string).collect() });
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if preset_button(ui, "Custom hotkey", "then use Record in the inspector") {
            assign_action(app, "Hotkey", "keyboard", "#34495e", Action::Key { keys: vec![] });
            app.reveal_action = true;
        }
        if preset_button(ui, "Type text", "types a text, \\n = Enter") {
            assign_action(app, "Text", "keyboard-variant", "#34495e", Action::Text { text: "Hello\n".into() });
            app.reveal_action = true;
        }
        if preset_button(ui, "Macro", "sequence with delays") {
            assign_action(app, "Macro", "playlist-play", "#34495e", Action::Macro { steps: vec![] });
            app.reveal_action = true;
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Knobs: arrows / scroll / volume").strong());
    ui.horizontal(|ui| {
        if preset_button(ui, "Volume knob", "turn: volume, press: mute") {
            assign_knob(app, "Vol", Action::Key { keys: vec!["mute".into()] }, Action::Key { keys: vec!["volumedown".into()] }, Action::Key { keys: vec!["volumeup".into()] });
        }
        if preset_button(ui, "Arrows knob", "turn: left/right, press: Enter") {
            assign_knob(app, "Arrows", Action::Key { keys: vec!["enter".into()] }, Action::Key { keys: vec!["left".into()] }, Action::Key { keys: vec!["right".into()] });
        }
        if preset_button(ui, "Scroll knob", "turn: up/down, press: Home") {
            assign_knob(app, "Scroll", Action::Key { keys: vec!["home".into()] }, Action::Key { keys: vec!["up".into()] }, Action::Key { keys: vec!["down".into()] });
        }
        if preset_button(ui, "Tabs knob", "turn: switch tabs, press: close tab") {
            assign_knob(app, "Tabs", Action::Key { keys: vec!["ctrl".into(), "w".into()] }, Action::Key { keys: vec!["ctrl".into(), "shift".into(), "tab".into()] }, Action::Key { keys: vec!["ctrl".into(), "tab".into()] });
        }
    });
}

fn pages(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Pages are stored on the deck. Side buttons flip pages by default.").weak());
    let names = app.cfg.order();
    ui.horizontal_wrapped(|ui| {
        for p in names {
            if preset_button(ui, &format!("Go to {p}"), "switch page") {
                assign_action(app, &p, "view-grid", "#8e44ad", Action::Page { page: p.clone() });
            }
        }
        if preset_button(ui, "Next page", "") {
            assign_action(app, "Next", "arrow-right-bold", "#8e44ad", Action::PageNext);
        }
        if preset_button(ui, "Previous page", "") {
            assign_action(app, "Prev", "arrow-left-bold", "#8e44ad", Action::PagePrev);
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Full-screen image").strong());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Show image...", "pick a picture; shown across all keys until a key is pressed") {
            if let Some(name) = app.pick_fullscreen_image("screen") {
                assign_action(app, "Show", "image", "#1c2b3a", Action::Screen { image: name, seconds: 0, frames: 0, delay_ms: 80 });
            }
        }
        if preset_button(ui, "Play animation...", "pick a GIF; plays once across the whole deck (any key ends it)") {
            if let Some((base, frames, delay)) = app.pick_animation("screen") {
                assign_action(app, "Play", "movie-play", "#1c2b3a", Action::Screen { image: base, seconds: 0, frames, delay_ms: delay });
            }
        }
        if preset_button(ui, "Level-up effect", "Minecraft level-up wave over the current page") {
            assign_action(app, "Level up", "star-four-points", "#2f5f1f", Action::Effect { effect: deck_config::Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 } });
        }
        if preset_button(ui, "Subpage + back", "new page with a Back key, this key goes there") {
            let mut n = 1;
            while app.cfg.pages.contains_key(&format!("sub{n}")) {
                n += 1;
            }
            let name = format!("sub{n}");
            let mut page = deck_config::Page::default();
            page.keys.insert("13".into(), app.action_key("Back", "arrow-left", "#8e44ad", Action::Page { page: app.page.clone() }));
            app.cfg.pages.insert(name.clone(), page);
            let mut o = app.cfg.order();
            o.push(name.clone());
            app.cfg.page_order = o;
            assign_action(app, &name, "folder", "#8e44ad", Action::Page { page: name.clone() });
        }
    });
}

fn apps(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Application pages: a preset creates a page of hotkeys, and the deck switches to it automatically while that application is in front (agent required).").weak());
    if !app.agent_running.load(std::sync::atomic::Ordering::Relaxed) {
        ui.label(RichText::new("! agent not running: pages are created, but automatic switching only works with the agent").color(egui::Color32::from_rgb(230, 70, 70)).small());
    }
    ui.add_space(4.0);
    let mut chosen: Option<usize> = None;
    let mut chosen_with_key: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        for (i, p) in crate::presets::APP_PRESETS.iter().enumerate() {
            let exists = app.cfg.pages.contains_key(p.page);
            let title = if exists { format!("{} *", p.name) } else { p.name.to_string() };
            let hint = format!("{} keys, page \"{}\", focused app: {}{}", p.keys.len(), p.page, p.apps, if exists { "\n(* page exists: will be replaced)" } else { "" });
            let r = ui.add_sized([150.0, 44.0], egui::Button::new(RichText::new(title).size(13.0)));
            let r = r.on_hover_text(hint);
            if r.clicked() {
                chosen = Some(i);
            }
            if r.secondary_clicked() {
                chosen_with_key = Some(i);
            }
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Left click: create the page. Right click: create the page and put a \"Go to\" key on the selected slot of the current page.").weak().small());
    if let Some(i) = chosen.or(chosen_with_key) {
        let p = &crate::presets::APP_PRESETS[i];
        let from = app.page.clone();
        let name = app.apply_preset(p);
        if chosen_with_key.is_some() && from != name {
            app.page = from;
            assign_action(app, p.name, p.icon, p.color, Action::Page { page: name.clone() });
            app.page = name.clone();
            app.sel = Sel::None;
        }
        app.set_status(format!("page \"{name}\" created for {} (auto-switch: {})", p.name, p.apps), true);
    }
}

fn system(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("These run on the PC through the agent (needs the agent running).").weak());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Open program...", "pick an .exe") {
            if let Some(p) = rfd::FileDialog::new().add_filter("programs", &["exe", "lnk", "bat"]).pick_file() {
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "App".into());
                assign_action(app, &name, "application", "#1f6f8b", App::host("system", "open", json!({ "path": p.to_string_lossy() })));
            }
        }
        if preset_button(ui, "Open website", "edit the URL in the inspector") {
            assign_action(app, "Web", "web", "#1f6f8b", App::host("system", "url", json!({ "url": "https://" })));
            app.reveal_action = true;
        }
        if preset_button(ui, "Close program", "taskkill by image name") {
            assign_action(app, "Close", "close-circle", "#8b1f1f", App::host("system", "close", json!({ "process": "notepad.exe" })));
            app.reveal_action = true;
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Media keys via the PC").strong());
    ui.horizontal_wrapped(|ui| {
        for (title, key, icon) in [("Play/Pause", "play_pause", "play-pause"), ("Next", "next", "skip-next"), ("Previous", "prev", "skip-previous"), ("Stop", "stop", "stop")] {
            if preset_button(ui, title, "sends the media key on the PC") {
                assign_action(app, title, icon, "#2d6cdf", App::host("system", "media", json!({ "key": key })));
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Open Home Assistant", "in the browser") {
            let url = app.agent_cfg.homeassistant.url.clone();
            assign_action(app, "HA", "home-assistant", "#1f6f8b", App::host("system", "url", json!({ "url": url })));
        }
    });
}

fn media(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Windows media session: controls whatever is playing (Spotify, browser, players) and shows title, artist and album art. No login needed.").weak());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Play/Pause", "with playing/paused looks") {
            let k = app.toggle_key("Play", "media:status", "pause", "play", "#1f5f3a", App::host("media", "play_pause", json!({})));
            app.put_key(k);
        }
        if preset_button(ui, "Next", "") {
            assign_action(app, "Next", "skip-next", "#2d6cdf", App::host("media", "next", json!({})));
        }
        if preset_button(ui, "Previous", "") {
            assign_action(app, "Prev", "skip-previous", "#2d6cdf", App::host("media", "prev", json!({})));
        }
        if preset_button(ui, "Now playing tile", "album art + title") {
            let mut k = app.action_key("", "music", "#101418", App::host("media", "play_pause", json!({})));
            k.icon = Some("media-art.png".into());
            k.value = Some("media:line".into());
            app.put_key(k);
        }
        if preset_button(ui, "Title tile", "text only") {
            let mut k = app.action_key("", "music-note", "#101418", Action::None);
            k.value = Some("media:title".into());
            app.put_key(k);
        }
    });
    ui.horizontal(|ui| {
        if preset_button(ui, "Media knob", "turn: PC volume, press: play/pause") {
            assign_knob(app, "Media", App::host("media", "play_pause", json!({})), Action::Key { keys: vec!["volumedown".into()] }, Action::Key { keys: vec!["volumeup".into()] });
        }
    });
}

fn home_assistant(app: &mut App, ui: &mut egui::Ui) {
    if app.agent_cfg.homeassistant.url.is_empty() {
        ui.label("Set the Home Assistant URL and token in Agent settings first.");
        return;
    }
    ui.horizontal(|ui| {
        ui.label("search");
        ui.add(egui::TextEdit::singleline(&mut app.ha_filter).desired_width(220.0));
        ui.label("domain");
        let mut domains: Vec<String> = app.ha_entities.iter().map(|e| e.domain.clone()).collect();
        domains.sort();
        domains.dedup();
        egui::ComboBox::from_id_salt("ha-domain").selected_text(app.ha_domain.clone()).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.ha_domain, "all".into(), "all");
            for d in domains {
                ui.selectable_value(&mut app.ha_domain, d.clone(), d);
            }
        });
        if ui.button(if app.ha_entities.is_empty() { "Load entities" } else { "Reload" }).clicked() {
            match ha::fetch_entities(&app.agent_cfg.homeassistant) {
                Ok(v) => {
                    app.set_status(format!("{} entities", v.len()), true);
                    app.ha_entities = v;
                }
                Err(e) => app.set_status(format!("{e}"), false),
            }
        }
        ui.label(RichText::new(format!("{} entities", app.ha_entities.len())).weak());
    });
    if app.ha_entities.is_empty() {
        ui.label(RichText::new("Press Load entities. Then: toggle key = on/off with icons following the real state; run = trigger an automation or script; tile = live value; knob = brightness / volume / temperature.").weak());
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("quick filter:").weak().small());
        for (d, t) in [("automation", "automations"), ("input_boolean", "booleans (input_boolean)"), ("script", "scripts"), ("light", "lights"), ("switch", "switches")] {
            if ui.selectable_label(app.ha_domain == d, t).clicked() {
                app.ha_domain = if app.ha_domain == d { "all".into() } else { d.into() };
            }
        }
    });
    let filter = app.ha_filter.to_lowercase();
    let mut act: Option<(usize, &'static str)> = None;
    egui::Grid::new("ha-grid").striped(true).num_columns(4).min_col_width(60.0).show(ui, |ui| {
        for (i, e) in app.ha_entities.iter().enumerate() {
            if app.ha_domain != "all" && e.domain != app.ha_domain {
                continue;
            }
            if !filter.is_empty() && !e.id.to_lowercase().contains(&filter) && !e.name.to_lowercase().contains(&filter) {
                continue;
            }
            ui.label(&e.name);
            ui.label(RichText::new(&e.id).weak().small());
            ui.label(RichText::new(format!("{} {}", e.state, e.unit)).small());
            ui.horizontal(|ui| {
                if e.domain == "automation" && ui.small_button("run").on_hover_text("key that triggers the automation's actions now (automation.trigger)").clicked() {
                    act = Some((i, "run"));
                }
                if e.domain == "script" && ui.small_button("run").on_hover_text("key that runs the script").clicked() {
                    act = Some((i, "run"));
                }
                let toggle_title = match e.domain.as_str() {
                    "automation" => "on/off key",
                    "input_boolean" => "toggle key",
                    _ => "toggle key",
                };
                let toggle_hint = match e.domain.as_str() {
                    "automation" => "enables / disables the automation; the key shows whether it is enabled",
                    "input_boolean" => "flips the boolean; the key follows its state",
                    _ => "toggles the entity; the key follows its state",
                };
                if ha::is_togglable(e) && ui.small_button(toggle_title).on_hover_text(toggle_hint).clicked() {
                    act = Some((i, "toggle"));
                }
                if e.domain == "light" && ui.small_button("toggle + ring").on_hover_text("toggle key with a brightness ring").clicked() {
                    act = Some((i, "ring"));
                }
                if e.domain == "light" && ui.small_button("toggle + colour").on_hover_text("toggle key whose background shows the light's current colour").clicked() {
                    act = Some((i, "colour"));
                }
                if ui.small_button("tile").clicked() {
                    act = Some((i, "sensor"));
                }
                if ha::is_knobbable(e) && ui.small_button("knob").clicked() {
                    act = Some((i, "knob"));
                }
            });
            ui.end_row();
        }
    });
    if let Some((i, what)) = act {
        let e = app.ha_entities[i].clone();
        match what {
            "toggle" => match ha::toggle_key_local(app, &e) {
                Ok(k) => {
                    app.put_key(k);
                    app.set_status(format!("{}: toggle key with on/off looks", e.name), true);
                }
                Err(err) => app.set_status(format!("{err}"), false),
            },
            "run" => {
                let (icon, op, color) = if e.domain == "automation" { ("robot", "trigger", "#1f4f6f") } else { ("script-text-play", "script", "#1f4f6f") };
                let base = e.icon.as_deref().map(|i| i.trim_start_matches("mdi:").to_string()).unwrap_or_else(|| icon.to_string());
                let label = e.name.split(' ').take(2).collect::<Vec<_>>().join(" ");
                let k = app.action_key(&label, &base, color, App::host("homeassistant", op, json!({ "entity_id": e.id })));
                app.put_key(k);
                app.set_status(format!("{}: runs on press", e.name), true);
            }
            "colour" => match ha::toggle_key_local(app, &e) {
                Ok(mut k) => {
                    k.color_state = Some(format!("ha:{}:attr:rgb_color", e.id));
                    k.progress = Some(deck_config::Progress { state: format!("ha:{}:attr:brightness", e.id), min: 0.0, max: 255.0, style: "border".into(), start: "top".into(), color: "#ffffff".into(), width: 4, color_state: None, ..Default::default() });
                    app.put_key(k);
                    app.set_status(format!("{}: toggle key, background follows the light's colour", e.name), true);
                }
                Err(err) => app.set_status(format!("{err}"), false),
            },
            "ring" => match ha::toggle_key_local(app, &e) {
                Ok(mut k) => {
                    k.progress = Some(deck_config::Progress { state: format!("ha:{}:attr:brightness", e.id), min: 0.0, max: 255.0, style: "border".into(), start: "top".into(), color: "#f5b400".into(), width: 6, color_state: None, ..Default::default() });
                    app.put_key(k);
                    app.set_status(format!("{}: toggle key with brightness ring", e.name), true);
                }
                Err(err) => app.set_status(format!("{err}"), false),
            },
            "sensor" => match ha::sensor_key_local(app, &e) {
                Ok(k) => {
                    app.put_key(k);
                    app.set_status(format!("{}: live tile", e.name), true);
                }
                Err(err) => app.set_status(format!("{err}"), false),
            },
            _ => {
                let k = ha::knob(&e);
                app.put_knob(k);
                app.set_status(format!("{}: knob", e.name), true);
            }
        }
    }
}

fn obs(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("obs-websocket");
        ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.obs.url).desired_width(180.0).hint_text("ws://127.0.0.1:4455"));
        ui.label("password");
        ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.obs.password).password(true).desired_width(120.0));
        if ui.button("Save & connect").clicked() {
            if app.agent_cfg.obs.url.is_empty() {
                app.agent_cfg.obs.url = "ws://127.0.0.1:4455".into();
            }
            let _ = ha::save_agent_config(&app.agent_cfg);
            match integrations::obs_lists(&app.agent_cfg.obs.url, &app.agent_cfg.obs.password) {
                Ok((s, i)) => {
                    crate::restart_agent(app, &format!("OBS: {} scenes, {} inputs", s.len(), i.len()));
                    app.obs_scenes = s;
                    app.obs_inputs = i;
                }
                Err(e) => app.set_status(format!("OBS: {e}"), false),
            }
        }
    });
    ui.label(RichText::new("OBS Studio 28+ with WebSocket server enabled (Tools > WebSocket Server Settings).").weak());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Record", "toggle, red while recording") {
            assign_toggle(app, "Rec", "obs:recording", "record", "record-circle-outline", "#7a1f1f", App::host("obs", "record_toggle", json!({})));
        }
        if preset_button(ui, "Stream", "toggle, with live look") {
            assign_toggle(app, "Live", "obs:streaming", "broadcast", "broadcast-off", "#7a1f1f", App::host("obs", "stream_toggle", json!({})));
        }
        if preset_button(ui, "Virtual cam", "toggle") {
            assign_toggle(app, "Cam", "obs:virtualcam", "webcam", "webcam-off", "#1f5f3a", App::host("obs", "virtualcam_toggle", json!({})));
        }
        if preset_button(ui, "Save replay", "replay buffer") {
            assign_action(app, "Replay", "content-save-move", "#34495e", App::host("obs", "replay_save", json!({})));
        }
        if preset_button(ui, "Studio mode", "toggle") {
            assign_action(app, "Studio", "monitor-multiple", "#34495e", App::host("obs", "studio_toggle", json!({})));
        }
        if preset_button(ui, "Transition", "studio mode transition") {
            assign_action(app, "Go", "transition", "#34495e", App::host("obs", "transition", json!({})));
        }
    });
    if !app.obs_scenes.is_empty() {
        ui.label(RichText::new("Scenes (key lights up while the scene is live)").strong());
        let scenes = app.obs_scenes.clone();
        ui.horizontal_wrapped(|ui| {
            for s in scenes {
                if preset_button(ui, &s, "switch to this scene") {
                    let mut k = app.action_key(&s, "television", "#1c2b3a", App::host("obs", "scene", json!({ "name": s })));
                    k.state = Some("obs:scene".into());
                    let on = app.mdi_icon("television-play", "#f5b400");
                    k.looks.insert(s.clone(), deck_config::Look { icon: on, color: Some("#4a3a10".into()), ..Default::default() });
                    app.put_key(k);
                }
            }
        });
    }
    if !app.obs_inputs.is_empty() {
        ui.label(RichText::new("Inputs: mute toggles").strong());
        let inputs = app.obs_inputs.clone();
        ui.horizontal_wrapped(|ui| {
            for inp in inputs {
                if preset_button(ui, &format!("Mute {inp}"), "toggle mute, red while muted") {
                    assign_toggle(app, &inp, &format!("obs:mute:{inp}"), "microphone-off", "microphone", "#7a1f1f", App::host("obs", "mute_toggle", json!({ "input": inp })));
                }
            }
        });
    }
}

/// Runs every frame: the login may finish while another tab is open.
pub fn poll_spotify_login(app: &mut App) {
    let login_result = app.spotify_login.lock().unwrap().take();
    if let Some(r) = login_result {
        if let Some(e) = r.strip_prefix("ERROR ") {
            app.set_status(format!("Spotify login failed: {e}"), false);
        } else {
            app.agent_cfg.spotify.refresh_token = r;
            crate::restart_agent(app, "Spotify connected");
        }
    }
}

fn spotify(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Spotify Web API with your own app: create one at developer.spotify.com, add the redirect URI below, paste the Client ID, then Log in.").weak());
    ui.horizontal(|ui| {
        ui.label("Client ID");
        ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.spotify.client_id).desired_width(300.0));
        ui.label(RichText::new(format!("redirect URI: {}", integrations::SPOTIFY_REDIRECT)).weak().small());
    });
    ui.horizontal(|ui| {
        let logged = !app.agent_cfg.spotify.refresh_token.is_empty();
        ui.label(if logged { "Logged in" } else { "Not logged in" });
        if ui.button("Log in with Spotify").clicked() {
            let _ = ha::save_agent_config(&app.agent_cfg);
            let cid = app.agent_cfg.spotify.client_id.clone();
            let slot = app.spotify_login.clone();
            app.set_status("Spotify login: approve in the browser...", true);
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let r = match integrations::spotify_login(&cid) {
                    Ok(None) => return, // a newer login took over
                    Ok(Some(t)) => ha::save_spotify_token(&t).map(|_| t).unwrap_or_else(|e| format!("ERROR {e}")),
                    Err(e) => format!("ERROR {e}"),
                };
                *slot.lock().unwrap() = Some(r);
                ctx.request_repaint();
            });
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Play/Pause", "with playing look") {
            assign_toggle(app, "Spotify", "spotify:playing", "pause-circle", "play-circle", "#1f5f3a", App::host("spotify", "play_pause", json!({})));
        }
        if preset_button(ui, "Next", "") {
            assign_action(app, "Next", "skip-next", "#1d8f4c", App::host("spotify", "next", json!({})));
        }
        if preset_button(ui, "Previous", "") {
            assign_action(app, "Prev", "skip-previous", "#1d8f4c", App::host("spotify", "prev", json!({})));
        }
        if preset_button(ui, "Shuffle", "toggle with look") {
            assign_toggle(app, "Shuffle", "spotify:shuffle", "shuffle-variant", "shuffle-disabled", "#1f5f3a", App::host("spotify", "shuffle_toggle", json!({})));
        }
        if preset_button(ui, "Repeat", "off / context / track") {
            let mut k = app.action_key("Repeat", "repeat-off", "#1c2b3a", App::host("spotify", "repeat_cycle", json!({})));
            k.state = Some("spotify:repeat".into());
            let ctx_icon = app.mdi_icon("repeat", "#f5b400");
            let track_icon = app.mdi_icon("repeat-once", "#f5b400");
            k.looks.insert("context".into(), deck_config::Look { icon: ctx_icon, color: Some("#1f5f3a".into()), ..Default::default() });
            k.looks.insert("track".into(), deck_config::Look { icon: track_icon, color: Some("#1f5f3a".into()), ..Default::default() });
            app.put_key(k);
        }
        if preset_button(ui, "Like", "heart follows the current track") {
            assign_toggle(app, "Like", "spotify:liked", "heart", "heart-outline", "#5a1f3a", App::host("spotify", "like_toggle", json!({})));
        }
        if preset_button(ui, "Now playing tile", "title and artist") {
            let mut k = app.action_key("", "spotify", "#101418", App::host("spotify", "play_pause", json!({})));
            k.value = Some("spotify:line".into());
            app.put_key(k);
        }
        if preset_button(ui, "Volume knob", "turn: Spotify volume, press: play/pause") {
            assign_knob(app, "Spotify", App::host("spotify", "play_pause", json!({})), App::host("spotify", "volume", json!({ "step": 5 })), App::host("spotify", "volume", json!({ "step": 5 })));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Play a playlist/album URI:");
        ui.add(egui::TextEdit::singleline(&mut app.text_buf).desired_width(260.0).hint_text("spotify:playlist:..."));
        if ui.button("Assign").clicked() && !app.text_buf.is_empty() {
            let uri = app.text_buf.clone();
            assign_action(app, "Playlist", "playlist-music", "#1d8f4c", App::host("spotify", "play", json!({ "uri": uri })));
        }
    });
}

const AD_WAIT: &str = "awaiting_permission|awaiting_option|awaiting_diff";

/// Session tile: colour and icon follow the session state, label shows name + activity.
fn ad_session_key(app: &mut App, state_prefix: &str, action: Action) -> KeyDef {
    let mut k = app.action_key("", "robot-outline", "#1c2b3a", action);
    k.state = Some(format!("{state_prefix}state"));
    k.value = Some(format!("{state_prefix}line"));
    let idle = app.mdi_icon("robot-outline", "#9aa5b1");
    let busy = app.mdi_icon("robot", "#ffffff");
    let wait = app.mdi_icon("hand-back-left", "#ffffff");
    let off = app.mdi_icon("robot-off-outline", "#4a5560");
    k.looks.insert("idle".into(), deck_config::Look { icon: idle, color: Some("#1c2b3a".into()), ..Default::default() });
    k.looks.insert("processing".into(), deck_config::Look { icon: busy, color: Some("#1f6feb".into()), ..Default::default() });
    k.looks.insert(AD_WAIT.into(), deck_config::Look { icon: wait, color: Some("#d97706".into()), ..Default::default() });
    k.looks.insert("disconnected".into(), deck_config::Look { icon: off, color: Some("#14181c".into()), ..Default::default() });
    k.looks.insert("empty".into(), deck_config::Look { icon: None, color: Some("#101418".into()), ..Default::default() });
    k
}

fn ad_option_key(app: &mut App, i: usize) -> KeyDef {
    let mut k = app.action_key("", "numeric-1-box", "#2d3748", App::host("agentdeck", "option", json!({ "index": i })));
    k.icon = app.mdi_icon(&format!("numeric-{}-box", i + 1), "#ffffff");
    k.value = Some(format!("agentdeck:sel:opt:{i}"));
    k
}

fn agentdeck(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("AgentDeck (github.com/puritysb/AgentDeck) watches Claude Code, Codex and other coding agents. The agent talks to its local daemon; tiles show sessions and answer their prompts.").weak());
    ui.horizontal(|ui| {
        ui.label("daemon port");
        let mut port = app.agent_cfg.agentdeck.port as u32;
        if ui.add(egui::DragValue::new(&mut port).range(0..=65535)).on_hover_text("0 = read ~/.agentdeck/daemon.json (default 9120)").changed() {
            app.agent_cfg.agentdeck.port = port as u16;
        }
        ui.label("animation frames");
        let mut fr = app.agent_cfg.agentdeck.anim_frames as u32;
        if ui.add(egui::DragValue::new(&mut fr).range(0..=96)).on_hover_text("frames per orbit cycle (3.5 s): 0 = 48 (14 fps). 24 = like the official plugin, 72 = 21 fps; more frames = longer uploads").changed() {
            app.agent_cfg.agentdeck.anim_frames = fr as u16;
        }
        if ui.button("Save").clicked() {
            let _ = ha::save_agent_config(&app.agent_cfg);
            crate::restart_agent(app, "AgentDeck settings saved");
        }
        ui.label(RichText::new("install: npx @agentdeck/setup, then agentdeck daemon install").weak().small());
    });
    ui.label(RichText::new("Whole page").strong());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "AgentDeck (official)", "page drawn by AgentDeck's own renderer: session cards with animations, detail view with options, usage gauges. Needs Node.js on this PC.") {
            let name = "agentdeck".to_string();
            let mut page = deck_config::Page::default();
            for i in 0..14 {
                let mut k = KeyDef { label: String::new(), icon: None, color: Some("#000000".into()), text_color: None, action: App::host("agentdeck", "key", json!({ "key": i })), state: None, looks: std::collections::BTreeMap::new(), value: None, progress: None, tile: Some(format!("agentdeck:k{i}")), effect: None, color_state: None, font_size: None, align: None, valign: None, font: None };
                k.label = String::new();
                page.keys.insert(i.to_string(), k);
            }
            page.knobs = vec![KnobDef::default(), KnobDef::default(), KnobDef::default()];
            app.cfg.pages.insert(name.clone(), page);
            if !app.cfg.page_order.contains(&name) {
                let mut o = app.cfg.order();
                o.push(name.clone());
                app.cfg.page_order = o;
            }
            app.page = name;
            app.sel = Sel::None;
            app.dirty = true;
            app.set_status("page \"agentdeck\" created (official AgentDeck tiles; the agent renders them)", true);
        }
        if preset_button(ui, "Simple tiles page", "deckd-native tiles: 8 session tiles, approve / deny / option, stop, usage ring") {
            let name = "agentdeck".to_string();
            let mut page = deck_config::Page::default();
            for i in 0..8 {
                let k = ad_session_key(app, &format!("agentdeck:slot:{i}:"), App::host("agentdeck", "select", json!({ "slot": i })));
                page.keys.insert(i.to_string(), k);
            }
            page.keys.insert("8".into(), app.action_key("Yes", "check-bold", "#15803d", App::host("agentdeck", "approve", json!({}))));
            page.keys.insert("9".into(), app.action_key("No", "close-thick", "#b91c1c", App::host("agentdeck", "deny", json!({}))));
            page.keys.insert("10".into(), ad_option_key(app, 1));
            page.keys.insert("11".into(), app.action_key("Stop", "stop", "#7a1f1f", App::host("agentdeck", "stop", json!({}))));
            let mut ring = ad_session_key(app, "agentdeck:sel:", App::host("agentdeck", "select_next", json!({})));
            ring.progress = Some(deck_config::Progress { state: "agentdeck:usage:5h".into(), min: 0.0, max: 100.0, style: "border".into(), start: "top".into(), color: "#f5b400".into(), width: 6, color_state: None, ..Default::default() });
            page.keys.insert("12".into(), ring);
            page.knobs = vec![
                KnobDef { label: "Session".into(), press: App::host("agentdeck", "approve", json!({})), left: App::host("agentdeck", "select_prev", json!({})), right: App::host("agentdeck", "select_next", json!({})), progress: None },
                KnobDef::default(),
                KnobDef::default(),
            ];
            app.cfg.pages.insert(name.clone(), page);
            if !app.cfg.page_order.contains(&name) {
                let mut o = app.cfg.order();
                o.push(name.clone());
                app.cfg.page_order = o;
            }
            app.page = name;
            app.sel = Sel::None;
            app.dirty = true;
            app.set_status("page \"agentdeck\" created", true);
        }
    });
    ui.label(RichText::new("Sessions").strong());
    ui.horizontal_wrapped(|ui| {
        for i in 0..8 {
            if preset_button(ui, &format!("Session {}", i + 1), "tile follows the session; press selects it") {
                let k = ad_session_key(app, &format!("agentdeck:slot:{i}:"), App::host("agentdeck", "select", json!({ "slot": i })));
                app.put_key(k);
            }
        }
        if preset_button(ui, "Selected session", "tile of the selected session; press cycles") {
            let k = ad_session_key(app, "agentdeck:sel:", App::host("agentdeck", "select_next", json!({})));
            app.put_key(k);
        }
        if preset_button(ui, "Overview", "idle / busy / waiting across all sessions, count") {
            let mut k = ad_session_key(app, "agentdeck:", App::host("agentdeck", "select_next", json!({})));
            k.value = Some("agentdeck:waiting".into());
            k.looks.insert("waiting".into(), deck_config::Look { icon: app.mdi_icon("hand-back-left", "#ffffff"), color: Some("#d97706".into()), ..Default::default() });
            k.looks.insert("offline".into(), deck_config::Look { icon: app.mdi_icon("robot-off-outline", "#4a5560"), color: Some("#14181c".into()), ..Default::default() });
            app.put_key(k);
        }
        if preset_button(ui, "Sessions knob", "turn: select session, press: approve") {
            assign_knob(app, "Session", App::host("agentdeck", "approve", json!({})), App::host("agentdeck", "select_prev", json!({})), App::host("agentdeck", "select_next", json!({})));
        }
    });
    ui.label(RichText::new("Answer the selected session").strong());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Approve / Yes", "permission gate: allow; prompt: first option") {
            assign_action(app, "Yes", "check-bold", "#15803d", App::host("agentdeck", "approve", json!({})));
        }
        if preset_button(ui, "Deny / No", "permission gate: deny; prompt: last option") {
            assign_action(app, "No", "close-thick", "#b91c1c", App::host("agentdeck", "deny", json!({})));
        }
        for i in 0..4 {
            if preset_button(ui, &format!("Option {}", i + 1), "label follows the prompt's option") {
                let k = ad_option_key(app, i);
                app.put_key(k);
            }
        }
        if preset_button(ui, "Stop", "interrupt (Ctrl+C / soft stop)") {
            assign_action(app, "Stop", "stop", "#7a1f1f", App::host("agentdeck", "stop", json!({})));
        }
        if preset_button(ui, "Escape", "Esc in the agent's terminal") {
            assign_action(app, "Esc", "keyboard-esc", "#3a3f44", App::host("agentdeck", "escape", json!({})));
        }
        if preset_button(ui, "Plan mode", "Shift+Tab cycle / plan") {
            assign_action(app, "Plan", "map-outline", "#4b53bc", App::host("agentdeck", "mode", json!({ "mode": "plan" })));
        }
        if preset_button(ui, "Review", "run AgentDeck's independent review") {
            assign_action(app, "Review", "magnify-scan", "#2d3748", App::host("agentdeck", "review", json!({})));
        }
        if preset_button(ui, "Usage ring", "Claude 5-hour quota around the tile") {
            let mut k = app.action_key("5h", "gauge", "#1c2b3a", App::host("agentdeck", "refresh", json!({})));
            k.value = Some("agentdeck:usage:5h".into());
            k.progress = Some(deck_config::Progress { state: "agentdeck:usage:5h".into(), min: 0.0, max: 100.0, style: "border".into(), start: "top".into(), color: "#f5b400".into(), width: 6, color_state: None, ..Default::default() });
            app.put_key(k);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Send a prompt:");
        ui.add(egui::TextEdit::singleline(&mut app.text_buf).desired_width(300.0).hint_text("continue / commit the changes / run the tests"));
        if ui.button("Assign").clicked() && !app.text_buf.is_empty() {
            let t = app.text_buf.clone();
            let label: String = t.chars().take(10).collect();
            assign_action(app, &label, "send", "#2d3748", App::host("agentdeck", "prompt", json!({ "text": t })));
        }
    });
}

fn minecraft(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Minecraft item icons come from the game's own client jar on this PC (nothing is downloaded); they are drawn as crisp pixel art on the deck. The level-up effect runs on the deck itself.").weak());
    ui.horizontal(|ui| {
        if app.mc_names.is_empty() {
            ui.label(RichText::new("no item icons imported yet").color(egui::Color32::from_rgb(230, 170, 70)));
        } else {
            ui.label(RichText::new(format!("{} item icons", app.mc_names.len())).strong());
        }
        if ui.button(if app.mc_names.is_empty() { "Import from installed Minecraft" } else { "Re-import" }).clicked() {
            app.import_minecraft(None);
        }
        if ui.button("Pick client .jar...").on_hover_text("%APPDATA%\\.minecraft\\versions\\<version>\\<version>.jar").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Minecraft client", &["jar"]).pick_file() {
                app.import_minecraft(Some(p));
            }
        }
        if ui.button("Browse items...").clicked() {
            app.mdi_target = crate::app::MdiTarget::KeyIcon;
            app.show_mc = true;
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Level-up effect").strong());
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Add to selected key", "the selected key keeps its action and plays the level-up wave across the screen on every press") {
            if let Some(key) = app.selected_key() {
                key.effect = Some(deck_config::Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 });
                app.dirty = true;
                app.set_status("level-up wave added to the key (Appearance > Press effect to change it)", true);
            } else {
                app.set_status("select a key on the deck first", false);
            }
        }
        if preset_button(ui, "Level-up key", "a key whose only job is the wave (diamond icon)") {
            let k = app.mc_key("Level up", "experience_bottle", "#2f5f1f", Action::Effect { effect: deck_config::Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 } });
            app.put_key(k);
            app.set_status("level-up key assigned", true);
        }
        if preset_button(ui, "Wave on whole page", "every key on this page plays the wave when pressed") {
            let page = app.current_page();
            let mut n = 0;
            for k in page.keys.values_mut() {
                if !(k.label.is_empty() && matches!(k.action, Action::None)) {
                    k.effect = Some(deck_config::Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 });
                    n += 1;
                }
            }
            app.dirty = true;
            app.set_status(format!("level-up wave on {n} keys"), true);
        }
        if preset_button(ui, "Remove from page", "clear the press effect of every key on this page") {
            let page = app.current_page();
            for k in page.keys.values_mut() {
                k.effect = None;
            }
            app.dirty = true;
            app.set_status("press effects cleared on this page", true);
        }
    });
    if app.mc_names.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Starter keys (icon only; set the action in the inspector)").strong());
    ui.horizontal_wrapped(|ui| {
        for (label, item) in [
            ("Sword", "diamond_sword"), ("Pickaxe", "diamond_pickaxe"), ("Axe", "diamond_axe"), ("Bow", "bow"), ("Shield", "shield"),
            ("Apple", "golden_apple"), ("Potion", "potion"), ("Ender pearl", "ender_pearl"), ("Compass", "compass"), ("Clock", "clock"),
            ("Diamond", "diamond"), ("Emerald", "emerald"), ("Redstone", "redstone"), ("Book", "enchanted_book"), ("Totem", "totem_of_undying"),
        ] {
            if app.mc_names.iter().any(|n| n == item) && preset_button(ui, label, item) {
                let k = app.mc_key(label, item, "#2b2b2b", Action::None);
                app.put_key(k);
                app.reveal_action = true;
                app.set_status(format!("{label} key assigned; choose its action"), true);
            }
        }
    });
}

fn discord(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Discord RPC: create an application at discord.com/developers (OAuth2 tab: add redirect http://127.0.0.1), paste its Client ID and Secret. Save restarts the agent; the first time, Discord then asks you to approve the connection.").weak());
    ui.horizontal(|ui| {
        ui.label("Client ID");
        ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.discord.client_id).desired_width(200.0));
        ui.label("Client secret");
        ui.add(egui::TextEdit::singleline(&mut app.agent_cfg.discord.client_secret).password(true).desired_width(200.0));
        if ui.button("Save").clicked() {
            let _ = ha::save_agent_config(&app.agent_cfg);
            crate::restart_agent(app, "Discord settings saved");
        }
    });
    ui.horizontal_wrapped(|ui| {
        if preset_button(ui, "Mute", "toggle, red while muted") {
            assign_toggle(app, "Mute", "discord:mute", "microphone-off", "microphone", "#7a1f1f", App::host("discord", "mute_toggle", json!({})));
        }
        if preset_button(ui, "Deafen", "toggle, red while deafened") {
            assign_toggle(app, "Deafen", "discord:deaf", "headphones-off", "headphones", "#7a1f1f", App::host("discord", "deafen_toggle", json!({})));
        }
        if preset_button(ui, "Push-to-talk", "toggle voice mode") {
            assign_toggle(app, "PTT", "discord:ptt", "radio-handheld", "account-voice", "#1f5f3a", App::host("discord", "ptt_toggle", json!({})));
        }
    });
    let _ = KeyDef::default();
}
