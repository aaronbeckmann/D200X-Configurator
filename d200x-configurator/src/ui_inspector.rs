//! Right-hand inspector. A header card says what is selected and what it does; below it,
//! collapsible sections in the order a key is built (what it does, what it shows, how it
//! follows states). Each section header carries a summary, so closed sections still say what
//! is set. Explanations live in hover texts; only warnings stay on screen.

use crate::app::{action_summary, color_to_string, parse_color, App, MdiTarget, Sel};
use crate::ui_states::{pick_button, PickRequest, StateField};
use deck_config::{Action, Effect, KeyDef, Look, Progress, Step};
use eframe::egui::{self, Color32, RichText};

/// Width of the field-name column in every section.
const LABEL_W: f32 = 92.0;
const WARN: Color32 = Color32::from_rgb(0xff, 0x80, 0x80);

pub fn show(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    match app.sel {
        Sel::None => settings(app, ui),
        Sel::Key(i) => key_inspector(app, ui, ctx, i),
        Sel::Knob(k) => knob_inspector(app, ui, k),
        Sel::SideLeft | Sel::SideRight => side_inspector(app, ui),
    }
}

// ---------------------------------------------------------------- building blocks

/// A collapsible section. The open state is kept per title, so it stays as the user left it
/// while they click through keys. `reveal` opens it (e.g. after the library assigned an action
/// that still needs input).
fn section(ui: &mut egui::Ui, title: &str, summary: &str, default_open: bool, reveal: bool, body: impl FnOnce(&mut egui::Ui)) {
    let id = ui.make_persistent_id(("inspector-section", title));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open)) || reveal;
    let header = ui
        .horizontal(|ui| {
            let (_, icon) = ui.allocate_exact_size(egui::vec2(12.0, 18.0), egui::Sense::hover());
            egui::collapsing_header::paint_default_icon(ui, if open { 1.0 } else { 0.0 }, &icon);
            ui.label(RichText::new(title).strong().size(14.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(summary).weak()).truncate());
            });
        })
        .response
        .interact(egui::Sense::click());
    if header.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if header.clicked() {
        open = !open;
    }
    ui.data_mut(|d| d.insert_persisted(id, open));
    if open {
        ui.add_space(2.0);
        ui.indent(id, body);
    }
    ui.add_space(2.0);
    ui.separator();
}

/// Two-column grid: field names on the left, widgets on the right.
fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([10.0, 7.0]).min_col_width(LABEL_W).show(ui, body);
}

/// Field name, muted so the values stand out; the help text appears on hover.
fn field(ui: &mut egui::Ui, name: &str, help: &str) {
    let r = ui.label(RichText::new(name).color(ui.visuals().weak_text_color()));
    if !help.is_empty() {
        r.on_hover_cursor(egui::CursorIcon::Help).on_hover_text(help);
    }
}

/// A file name short enough for a combo box; the full name is in the list.
fn fit(name: &str) -> String {
    let n = name.trim_end_matches(".png");
    if n.chars().count() > 22 {
        format!("{}…", n.chars().take(21).collect::<String>())
    } else {
        n.to_string()
    }
}

/// Header card: a preview of the tile, its name and what it does, actions on the right.
fn header(ui: &mut egui::Ui, title: &str, subtitle: &str, preview: impl FnOnce(&mut egui::Ui), buttons: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).fill(ui.visuals().faint_bg_color).inner_margin(8.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            preview(ui);
            ui.vertical(|ui| {
                ui.label(RichText::new(title).heading());
                ui.add(egui::Label::new(RichText::new(subtitle).weak()).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), buttons);
        });
    });
    ui.add_space(6.0);
}

/// The tile as it looks with its base appearance: colour, icon, label.
fn tile_preview(ui: &mut egui::Ui, key: &KeyDef, icon: Option<egui::TextureHandle>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
    let p = ui.painter();
    let transparent = key.color.as_deref().map(|c| c.eq_ignore_ascii_case("none")).unwrap_or(false);
    if transparent {
        p.rect_stroke(rect, 10.0, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
    } else {
        p.rect_filled(rect, 10.0, parse_color(&key.color, Color32::from_rgb(0x22, 0x2a, 0x33)));
    }
    let text = parse_color(&key.text_color, Color32::WHITE);
    match icon {
        Some(tex) => {
            let img = if key.label.is_empty() { rect.shrink(8.0) } else { egui::Rect::from_min_max(rect.min + egui::vec2(12.0, 6.0), rect.max - egui::vec2(12.0, 20.0)) };
            egui::Image::new(&tex).paint_at(ui, img);
            if !key.label.is_empty() {
                ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 10.0), egui::Align2::CENTER_CENTER, short(&key.label, 9), egui::FontId::proportional(10.0), text);
            }
        }
        None => {
            p.text(rect.center(), egui::Align2::CENTER_CENTER, short(&key.label, 9), egui::FontId::proportional(12.0), text);
        }
    }
}

fn short(s: &str, n: usize) -> String {
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() > n {
        format!("{}…", first.chars().take(n - 1).collect::<String>())
    } else {
        first.to_string()
    }
}

/// "nothing" for an empty action, otherwise its one-line summary.
fn what(a: &Action) -> String {
    let s = action_summary(a);
    if s.is_empty() {
        "nothing".into()
    } else {
        s
    }
}

/// A state name field with the explorer's "pick" button and a clear button.
fn state_field(ui: &mut egui::Ui, value: &mut Option<String>, hint: &str, width: f32, pick_as: StateField, fallback: Option<&str>, pick: &mut PickRequest) -> bool {
    let mut changed = false;
    let mut s = value.clone().unwrap_or_default();
    let width = width.min(ui.available_width() - 70.0).max(80.0);
    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(width).hint_text(hint)).changed() {
        *value = if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
        changed = true;
    }
    pick_button(ui, pick_as, Some(&s), fallback, pick);
    if value.is_some() && ui.small_button("✖").on_hover_text("clear").clicked() {
        *value = None;
        changed = true;
    }
    changed
}

/// Shared action-editor state that lives in the app between frames.
struct ActionCtx {
    pages: Vec<String>,
    macro_text: String,
    owner: bool,
    recording: bool,
}

impl ActionCtx {
    fn take(app: &mut App) -> Self {
        ActionCtx { pages: app.cfg.order(), macro_text: std::mem::take(&mut app.macro_text), owner: app.macro_for == app.sel, recording: app.recording_hotkey }
    }
    fn give_back(self, app: &mut App) {
        app.macro_text = self.macro_text;
        app.recording_hotkey = self.recording;
        if self.owner {
            app.macro_for = app.sel;
        }
    }
}

// ---------------------------------------------------------------- deck (nothing selected)

fn settings(app: &mut App, ui: &mut egui::Ui) {
    header(ui, "Deck", "select a key, knob or side button on the preview to edit it", |_| {}, |_| {});
    let mut ch = false;
    let summary = format!("{} %, dim after {} s", app.cfg.brightness, app.cfg.dim_after_s);
    section(ui, "Display", &summary, true, false, |ui| {
        grid(ui, "display", |ui| {
            field(ui, "Brightness", "");
            ch |= ui.add(egui::Slider::new(&mut app.cfg.brightness, 0..=100).suffix(" %")).changed();
            ui.end_row();
            field(ui, "Dim", "after this many seconds without input the backlight dims (0 = never)");
            ui.horizontal(|ui| {
                ch |= ui.add(egui::DragValue::new(&mut app.cfg.dim_after_s).suffix(" s")).changed();
                ui.label("to");
                ch |= ui.add(egui::Slider::new(&mut app.cfg.dim_brightness, 0..=100).suffix(" %")).changed();
            });
            ui.end_row();
            field(ui, "Screen off", "after this many seconds without input (0 = never)");
            ch |= ui.add(egui::DragValue::new(&mut app.cfg.off_after_s).suffix(" s")).changed();
            ui.end_row();
            field(ui, "Home page", "shown after start");
            let pages = app.cfg.order();
            egui::ComboBox::from_id_salt("home").selected_text(app.cfg.home.clone()).show_ui(ui, |ui| {
                for p in &pages {
                    ch |= ui.selectable_value(&mut app.cfg.home, p.clone(), p).changed();
                }
            });
            ui.end_row();
        });
    });

    let page = app.page_ref();
    let summary = [page.background.as_deref().map(|b| format!("background {b}")), page.app.as_deref().map(|a| format!("follows {a}"))].into_iter().flatten().collect::<Vec<_>>().join(", ");
    let title = format!("Page \"{}\"", app.page);
    section(ui, &title, &summary, true, false, |ui| {
        grid(ui, "page", |ui| {
            field(ui, "Background", "Drawn behind the tiles; transparent keys (Colours) let it show through. A key's look can replace it on every page while its state matches (States & looks).");
            ui.horizontal(|ui| {
                ui.label(page.background.clone().unwrap_or_else(|| "none".into()));
                if ui.small_button("Image...").clicked() {
                    if let Some(name) = app.pick_fullscreen_image("bg") {
                        app.current_page().background = Some(name);
                        ch = true;
                    }
                }
                if page.background.is_some() && ui.small_button("✖").on_hover_text("remove").clicked() {
                    app.current_page().background = None;
                    ch = true;
                }
            });
            ui.end_row();
            field(ui, "Auto-switch", "The deck shows this page while one of these programs is in front, and returns to the previous page afterwards. Several names: separate with |. Needs the agent.");
            ui.horizontal(|ui| {
                let mut pat = page.app.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut pat).desired_width(170.0).hint_text("photoshop.exe|lightroom.exe")).changed() {
                    app.current_page().app = if pat.trim().is_empty() { None } else { Some(pat.clone()) };
                    ch = true;
                }
                let running = app.running_apps.lock().unwrap().clone();
                let r = egui::ComboBox::from_id_salt("running-apps").selected_text("add").width(60.0).show_ui(ui, |ui| {
                    if running.is_empty() {
                        ui.label(RichText::new("loading...").weak());
                    }
                    for exe in &running {
                        if ui.selectable_label(false, exe).clicked() {
                            let mut parts: Vec<String> = pat.split('|').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
                            if !parts.iter().any(|x| x == exe) {
                                parts.push(exe.clone());
                            }
                            app.current_page().app = Some(parts.join("|"));
                            ch = true;
                        }
                    }
                });
                // the list of running programs is refreshed whenever the menu opens
                if r.response.clicked() {
                    app.refresh_running_apps();
                }
            });
            ui.end_row();
        });
        let agent_running = app.agent_running.load(std::sync::atomic::Ordering::Relaxed);
        if app.page_ref().app.is_some() && !agent_running {
            ui.label(RichText::new("Auto-switch needs the agent: it reports the program in front.").color(WARN).small());
        }
    });

    section(ui, "Tile layout", "calibrated", false, false, |ui| {
        let l = &mut app.cfg.layout;
        grid(ui, "layout", |ui| {
            field(ui, "Rows y / h", "");
            ui.horizontal(|ui| {
                for v in l.row_y.iter_mut() {
                    ch |= ui.add(egui::DragValue::new(v)).changed();
                }
                ch |= ui.add(egui::DragValue::new(&mut l.row_h)).changed();
            });
            ui.end_row();
            field(ui, "Columns x / w", "");
            ui.horizontal(|ui| {
                for v in l.col_x.iter_mut() {
                    ch |= ui.add(egui::DragValue::new(v)).changed();
                }
                ch |= ui.add(egui::DragValue::new(&mut l.col_w)).changed();
            });
            ui.end_row();
            field(ui, "Wide x y w h", "");
            ui.horizontal(|ui| {
                for v in l.wide.iter_mut() {
                    ch |= ui.add(egui::DragValue::new(v)).changed();
                }
            });
            ui.end_row();
            field(ui, "Radius / pad", "corner radius and text padding");
            ui.horizontal(|ui| {
                ch |= ui.add(egui::DragValue::new(&mut l.radius)).changed();
                ch |= ui.add(egui::DragValue::new(&mut l.text_pad)).changed();
            });
            ui.end_row();
        });
    });
    if ch {
        app.dirty = true;
    }
}

// ---------------------------------------------------------------- keys

fn key_inspector(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, i: usize) {
    let key = app.page_ref().keys.get(&i.to_string()).cloned().unwrap_or_default();
    let icon = key.icon.clone().and_then(|n| app.texture(ctx, &n));
    let title = if i == 13 { "Wide key".to_string() } else { format!("Key {}", i + 1) };
    let mut op: Option<&str> = None;
    header(ui, &title, &format!("does: {}", what(&key.action)), |ui| tile_preview(ui, &key, icon), |ui| {
        if ui.small_button("Clear").clicked() {
            op = Some("clear");
        }
        if ui.add_enabled(app.clipboard.is_some(), egui::Button::new("Paste").small()).on_hover_text("Ctrl+V on the preview").clicked() {
            op = Some("paste");
        }
        if ui.small_button("Copy").on_hover_text("Ctrl+C on the preview; right-click a tile for cut / paste").clicked() {
            op = Some("copy");
        }
    });
    match op {
        Some("clear") => {
            app.current_page().keys.insert(i.to_string(), KeyDef::default());
            app.dirty = true;
        }
        Some("paste") => app.paste_key(i),
        Some("copy") => app.copy_key(i),
        _ => {}
    }
    let agent_running = app.agent_running.load(std::sync::atomic::Ordering::Relaxed);
    let bridge = crate::ui_bridge::ready_plugins(app);
    if crate::ui_canvas::unserved(&key.action, agent_running, &bridge) {
        ui.label(RichText::new("Needs the agent (or the ESP32 bridge for this plugin); neither is running, so the key flashes red.").color(WARN).small());
        ui.add_space(4.0);
    }

    let reveal = std::mem::take(&mut app.reveal_action);
    section(ui, "Action", &what(&key.action), true, reveal, |ui| {
        let mut a = ActionCtx::take(app);
        let mut changed = false;
        if let Some(key) = app.selected_key() {
            changed = action_editor(ui, ctx, "key", &mut key.action, &a.pages, &mut a.macro_text, &mut a.owner, &mut a.recording);
        }
        if a.macro_text == "\u{1}pick-anim" {
            a.macro_text.clear();
            if let Some((base, frames, delay)) = app.pick_animation("fx") {
                if let Some(key) = app.selected_key() {
                    key.action = Action::Effect { effect: Effect::Frames { name: base, frames, delay_ms: delay } };
                }
                changed = true;
            }
        }
        a.give_back(app);
        if changed {
            app.dirty = true;
        }
    });

    let summary = match (key.label.is_empty(), &key.icon) {
        (true, None) => "empty".to_string(),
        (false, None) => format!("\"{}\"", short(&key.label, 24)),
        (true, Some(i)) => fit(i),
        (false, Some(i)) => format!("\"{}\" + {}", short(&key.label, 16), fit(i)),
    };
    section(ui, "Label & icon", &summary, true, false, |ui| label_icon(app, ui));

    let base = if key.color.as_deref().map(|c| c.eq_ignore_ascii_case("none")).unwrap_or(false) { "transparent".to_string() } else { key.color.clone().unwrap_or_else(|| "default".into()) };
    let summary = if key.color_state.is_some() { format!("{base}, follows a state") } else { base };
    section(ui, "Colours", &summary, false, false, |ui| colours(app, ui));

    let summary = match &key.state {
        None => "not linked".to_string(),
        Some(s) => format!("{s}, {} look{}", key.looks.len(), if key.looks.len() == 1 { "" } else { "s" }),
    };
    section(ui, "States & looks", &summary, key.state.is_some(), false, |ui| states_editor(app, ui, ctx));

    let summary = progress_summary(&key.progress);
    section(ui, "Progress", &summary, false, false, |ui| key_progress(app, ui));

    let summary = key.effect.as_ref().map(crate::app::effect_summary).unwrap_or_else(|| "none".into());
    section(ui, "Press effect", &summary, false, false, |ui| press_effect_editor(app, ui));
}

fn label_icon(app: &mut App, ui: &mut egui::Ui) {
    let icons = app.icons_known.clone();
    let fonts = app.fonts_known.clone();
    let (mut changed, mut want_mdi, mut want_mc, mut want_file, mut want_font, mut clear_icon) = (false, false, false, false, false, false);
    let Some(key) = app.selected_key() else { return };
    grid(ui, "label-icon", |ui| {
        field(ui, "Label", "Text on the tile; Enter for a second line. Templates: {} = the state picked under \"Label from\" (States & looks), {ha:sensor.x|1} = any state; |0 |1 round, |int, |upper. Missing states show as -.");
        changed |= ui.add(egui::TextEdit::multiline(&mut key.label).desired_rows(1).desired_width(f32::INFINITY)).changed();
        ui.end_row();

        field(ui, "Text", "size (auto: up to 26 px, shrinking to fit), horizontal and vertical alignment");
        ui.horizontal(|ui| {
            let mut size = key.font_size.unwrap_or(0.0);
            if ui.add(egui::DragValue::new(&mut size).range(0.0..=80.0).speed(0.5).custom_formatter(|v, _| if v < 6.0 { "auto".to_string() } else { format!("{v:.0} px") })).changed() {
                key.font_size = if size < 6.0 { None } else { Some(size.round()) };
                changed = true;
            }
            let cur = key.align.clone().unwrap_or_else(|| "center".into());
            egui::ComboBox::from_id_salt("align").selected_text(cur.clone()).width(70.0).show_ui(ui, |ui| {
                for a in ["left", "center", "right"] {
                    if ui.selectable_label(cur == a, a).clicked() {
                        key.align = if a == "center" { None } else { Some(a.to_string()) };
                        changed = true;
                    }
                }
            });
            let vcur = key.valign.clone().unwrap_or_else(|| "auto".into());
            egui::ComboBox::from_id_salt("valign").selected_text(vcur.clone()).width(70.0).show_ui(ui, |ui| {
                for a in ["auto", "top", "middle", "bottom"] {
                    if ui.selectable_label(vcur == a, a).on_hover_text("auto: middle without an icon, under the icon with one. top: above the icon. middle: over the icon.").clicked() {
                        key.valign = if a == "auto" { None } else { Some(a.to_string()) };
                        changed = true;
                    }
                }
            });
        });
        ui.end_row();

        field(ui, "Font", "any TrueType font, e.g. from C:\\Windows\\Fonts; uploaded to the deck with the icons");
        ui.horizontal(|ui| {
            let cur = key.font.as_deref().map(fit).unwrap_or_else(|| "deck font".into());
            egui::ComboBox::from_id_salt("font").selected_text(cur).width(170.0).show_ui(ui, |ui| {
                if ui.selectable_label(key.font.is_none(), "deck font").clicked() {
                    key.font = None;
                    changed = true;
                }
                for f in &fonts {
                    if ui.selectable_label(key.font.as_deref() == Some(f), f).clicked() {
                        key.font = Some(f.clone());
                        changed = true;
                    }
                }
            });
            want_font = ui.small_button("TTF...").clicked();
        });
        ui.end_row();

        field(ui, "Icon", "");
        ui.horizontal(|ui| {
            let current = key.icon.as_deref().map(fit).unwrap_or_else(|| "none".into());
            egui::ComboBox::from_id_salt("icon").selected_text(current).width(170.0).show_ui(ui, |ui| {
                if ui.selectable_label(key.icon.is_none(), "none").clicked() {
                    key.icon = None;
                    changed = true;
                }
                for n in &icons {
                    if ui.selectable_label(key.icon.as_deref() == Some(n), n).clicked() {
                        key.icon = Some(n.clone());
                        changed = true;
                    }
                }
            });
            if key.icon.is_some() && ui.small_button("✖").on_hover_text("remove the icon").clicked() {
                clear_icon = true;
            }
        });
        ui.end_row();

        ui.label("");
        ui.horizontal(|ui| {
            want_mdi = ui.small_button("Material...").clicked();
            want_mc = ui.small_button("Minecraft...").clicked();
            want_file = ui.small_button("Image file...").clicked();
        });
        ui.end_row();
    });
    if clear_icon {
        key.icon = None;
        changed = true;
    }
    if want_mdi {
        app.mdi_target = MdiTarget::KeyIcon;
        app.show_mdi = true;
    }
    if want_mc {
        app.mdi_target = MdiTarget::KeyIcon;
        app.show_mc = true;
    }
    if want_font {
        if let Some(name) = app.pick_font() {
            if let Some(key) = app.selected_key() {
                key.font = Some(name);
            }
            changed = true;
        }
    }
    if want_file {
        if let Some(path) = rfd::FileDialog::new().add_filter("images", &["png", "jpg", "jpeg", "svg"]).pick_file() {
            let name = path.file_stem().map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>()).unwrap_or_else(|| "icon".into()) + ".png";
            match deck_icons::render_file(&path, 124, 110) {
                Ok(png) => {
                    app.add_icon(&name, png);
                    if let Some(key) = app.selected_key() {
                        key.icon = Some(name.clone());
                    }
                    app.set_status(format!("icon {name} added (uploaded on save)"), true);
                    changed = true;
                }
                Err(e) => app.set_status(format!("{e}"), false),
            }
        }
    }
    if changed {
        app.dirty = true;
    }
}

fn colours(app: &mut App, ui: &mut egui::Ui) {
    let mut changed = false;
    let mut pick: PickRequest = None;
    let Some(key) = app.selected_key() else { return };
    grid(ui, "colours", |ui| {
        field(ui, "Background", "transparent lets the page background show through");
        ui.horizontal(|ui| {
            let mut tp = key.color.as_deref().map(|s| s.eq_ignore_ascii_case("none")).unwrap_or(false);
            if !tp {
                let mut c = parse_color(&key.color, Color32::from_rgb(0x22, 0x2a, 0x33));
                if crate::ui_color::color_field(ui, &mut c) {
                    key.color = Some(color_to_string(c));
                    changed = true;
                }
            }
            if ui.checkbox(&mut tp, "transparent").changed() {
                key.color = Some(if tp { "none" } else { "#222a33" }.into());
                changed = true;
            }
        });
        ui.end_row();
        field(ui, "Text", "");
        let mut t = parse_color(&key.text_color, Color32::WHITE);
        if crate::ui_color::color_field(ui, &mut t) {
            key.text_color = Some(color_to_string(t));
            changed = true;
        }
        ui.end_row();
        field(ui, "From state", "The background follows a state that holds a colour (#rrggbb, r,g,b or [r, g, b]), e.g. a light's rgb_color. It overrides the colours above while the value parses as a colour. For fixed colours per value (on/off) use States & looks.");
        ui.horizontal(|ui| {
            changed |= state_field(ui, &mut key.color_state, "ha:light.desk:attr:rgb_color", 160.0, StateField::Background, key.state.as_deref(), &mut pick);
            let light = key.state.as_deref().and_then(|s| s.strip_prefix("ha:")).map(|s| s.split(':').next().unwrap_or("").to_string()).filter(|e| e.starts_with("light."));
            if let Some(e) = light {
                if key.color_state.is_none() && ui.small_button("light colour").on_hover_text("the tile takes the light's current colour").clicked() {
                    key.color_state = Some(format!("ha:{e}:attr:rgb_color"));
                    changed = true;
                }
            }
        });
        ui.end_row();
    });
    if let Some((field, start)) = pick {
        app.states_ui.pick_for(field, start.as_deref());
    }
    if changed {
        app.dirty = true;
    }
}

/// Per-key full-screen effect played on every press (runs on the deck).
fn press_effect_editor(app: &mut App, ui: &mut egui::Ui) {
    let mut want_anim = false;
    let mut changed = false;
    if let Some(key) = app.selected_key() {
        let mut eff = key.effect.clone();
        changed |= effect_editor(ui, "key-fx", &mut eff, &mut want_anim, true);
        if changed {
            key.effect = eff;
        }
    }
    if want_anim {
        if let Some((base, frames, delay)) = app.pick_animation("fx") {
            if let Some(key) = app.selected_key() {
                key.effect = Some(Effect::Frames { name: base, frames, delay_ms: delay });
            }
            changed = true;
        }
    }
    if changed {
        app.dirty = true;
    }
}

// ---------------------------------------------------------------- progress (keys and knobs)

fn progress_summary(p: &Option<Progress>) -> String {
    match p {
        None => "off".into(),
        Some(p) => format!("{} on {}", if p.style == "fill" { "fill" } else { "ring" }, if p.state.is_empty() { "?" } else { &p.state }),
    }
}

fn key_progress(app: &mut App, ui: &mut egui::Ui) {
    let mut pick: PickRequest = None;
    let mut changed = false;
    let Some(key) = app.selected_key() else { return };
    let hint = key.state.clone();
    let mut presets: Vec<(String, Progress)> = Vec::new();
    let p = |state: String, min: f64, max: f64, style: &str, start: &str, color: &str| Progress { state, min, max, style: style.into(), start: start.into(), color: color.into(), ..Default::default() };
    if let Some(e) = hint.as_deref().and_then(|s| s.strip_prefix("ha:")).map(|s| s.split(':').next().unwrap_or("").to_string()) {
        if e.starts_with("light.") {
            presets.push(("Brightness ring".into(), p(format!("ha:{e}:attr:brightness"), 0.0, 255.0, "border", "top", "#f5b400")));
        }
        if e.starts_with("media_player.") {
            presets.push(("Volume ring".into(), p(format!("ha:{e}:attr:volume_level"), 0.0, 1.0, "border", "top", "#7fd3ff")));
        }
        if e.starts_with("climate.") {
            presets.push(("Temperature fill".into(), p(format!("ha:{e}:attr:current_temperature"), 15.0, 30.0, "fill", "bottom", "#ff8040")));
        }
    }
    let s = hint.clone().unwrap_or_default();
    presets.push(("Ring".into(), p(s.clone(), 0.0, 100.0, "border", "top", "#f5b400")));
    presets.push(("Fill from bottom".into(), p(s.clone(), 0.0, 100.0, "fill", "bottom", "#f5b400")));
    presets.push(("Fill from left".into(), p(s, 0.0, 100.0, "fill", "left", "#7fd3ff")));
    changed |= progress_fields(ui, "key", &mut key.progress, &presets, hint.as_deref(), &mut pick);
    if let Some((field, start)) = pick {
        app.states_ui.pick_for(field, start.as_deref());
    }
    if changed {
        app.dirty = true;
    }
}

/// Progress editor shared by keys and knobs. While none is set, `presets` are offered.
fn progress_fields(ui: &mut egui::Ui, id: &str, progress: &mut Option<Progress>, presets: &[(String, Progress)], state_hint: Option<&str>, pick: &mut PickRequest) -> bool {
    let mut changed = false;
    let Some(p) = progress.as_mut() else {
        ui.label(RichText::new("Shows a number (brightness, volume, battery, temperature) along the tile.").weak().small());
        ui.horizontal_wrapped(|ui| {
            for (name, preset) in presets {
                if ui.button(name).clicked() {
                    *progress = Some(preset.clone());
                    changed = true;
                }
            }
        });
        return changed;
    };
    let mut remove = false;
    grid(ui, &format!("{id}-progress"), |ui| {
        field(ui, "State", "The first number in the value is used (\"45 %\" -> 45). On/off words (on, off, true, false, open, closed, ...) show full or empty.");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::TextEdit::singleline(&mut p.state).desired_width(200.0).hint_text("ha:light.x:attr:brightness")).changed();
            pick_button(ui, StateField::Ring, Some(&p.state), state_hint, pick);
        });
        ui.end_row();
        field(ui, "Range", "the values shown as empty and full");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut p.min).speed(0.1)).changed();
            ui.label("to");
            changed |= ui.add(egui::DragValue::new(&mut p.max).speed(0.1)).changed();
        });
        ui.end_row();
        if p.min_state.is_some() || p.max_state.is_some() {
            field(ui, "Live range", "The device reports its own limits; these states replace the range above while they are known.");
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} .. {}", p.min_state.as_deref().unwrap_or("-"), p.max_state.as_deref().unwrap_or("-"))).weak().small());
                if ui.small_button("✖").on_hover_text("use the fixed range only").clicked() {
                    p.min_state = None;
                    p.max_state = None;
                    changed = true;
                }
            });
            ui.end_row();
        }
        field(ui, "Style", "ring along the edge, or a fill growing from one side");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(format!("{id}-pstyle")).selected_text(if p.style == "fill" { "fill" } else { "ring" }).width(60.0).show_ui(ui, |ui| {
                changed |= ui.selectable_value(&mut p.style, "border".to_string(), "ring").changed();
                changed |= ui.selectable_value(&mut p.style, "fill".to_string(), "fill").changed();
            });
            ui.label(if p.style == "fill" { "from" } else { "starts at" });
            egui::ComboBox::from_id_salt(format!("{id}-pstart")).selected_text(p.start.clone()).width(70.0).show_ui(ui, |ui| {
                for s in ["top", "right", "bottom", "left"] {
                    changed |= ui.selectable_value(&mut p.start, s.to_string(), s).changed();
                }
            });
            if p.style != "fill" {
                ui.label("width");
                changed |= ui.add(egui::DragValue::new(&mut p.width).range(2..=14)).changed();
            }
        });
        ui.end_row();
        field(ui, "Colour", "");
        let mut c = parse_color(&Some(p.color.clone()), Color32::from_rgb(0xf5, 0xb4, 0x00));
        if crate::ui_color::color_field(ui, &mut c) {
            p.color = color_to_string(c);
            changed = true;
        }
        ui.end_row();
        field(ui, "Colour from", "A state holding a colour (#rrggbb, r,g,b or [r, g, b]), e.g. a light's rgb_color. The colour above is used while it has none (a light that is off).");
        ui.horizontal(|ui| {
            changed |= state_field(ui, &mut p.color_state, "ha:light.x:attr:rgb_color", 160.0, StateField::RingColor, Some(&p.state), pick);
            let light = [p.state.as_str(), state_hint.unwrap_or("")].iter().filter_map(|s| s.strip_prefix("ha:")).map(|s| s.split(':').next().unwrap_or("")).find(|e| e.starts_with("light.")).map(str::to_string);
            if let Some(e) = light {
                if p.color_state.is_none() && ui.small_button("light colour").clicked() {
                    p.color_state = Some(format!("ha:{e}:attr:rgb_color"));
                    changed = true;
                }
            }
        });
        ui.end_row();
        ui.label("");
        remove = ui.small_button("Remove indicator").clicked();
        ui.end_row();
    });
    if remove {
        *progress = None;
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------- knobs and side buttons

fn knob_inspector(app: &mut App, ui: &mut egui::Ui, k: usize) {
    let knob = {
        let page = app.page_ref();
        page.knobs.get(k).cloned().unwrap_or_default()
    };
    let subtitle = format!("turn: {} / {}   press: {}", what(&knob.left), what(&knob.right), what(&knob.press));
    let preview = |ui: &mut egui::Ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 28.0, Color32::from_rgb(0x2b, 0x3b, 0x4b));
        ui.painter().circle_stroke(rect.center(), 28.0, egui::Stroke::new(2.0, Color32::from_rgb(0x4a, 0x6a, 0x8a)));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, short(&knob.label, 7), egui::FontId::proportional(11.0), Color32::from_rgb(0xd0, 0xe0, 0xf0));
    };
    header(ui, &format!("Knob {}", k + 1), &subtitle, preview, |_| {});

    let mut changed = false;
    if let Some(knob) = app.selected_knob() {
        grid(ui, "knob-label", |ui| {
            field(ui, "Label", "shown in the knob's box on the wide key");
            changed |= ui.add(egui::TextEdit::singleline(&mut knob.label).desired_width(f32::INFINITY)).changed();
            ui.end_row();
        });
        ui.add_space(4.0);
    }
    let reveal = std::mem::take(&mut app.reveal_action);
    let turn = if what(&knob.left) == what(&knob.right) { what(&knob.left) } else { format!("{} / {}", what(&knob.left), what(&knob.right)) };
    section(ui, "Turn", &turn, true, reveal, |ui| {
        let mut a = ActionCtx::take(app);
        if let Some(knob) = app.selected_knob() {
            for (title, act, id) in [("Left", &mut knob.left, "kl"), ("Right", &mut knob.right, "kr")] {
                ui.label(RichText::new(title).weak());
                let (mut o, mut r) = (false, false);
                changed |= action_editor_simple(ui, id, act, &a.pages, &mut a.macro_text, &mut o, &mut r);
                ui.add_space(4.0);
            }
        }
        a.give_back(app);
    });
    section(ui, "Press", &what(&knob.press), true, false, |ui| {
        let mut a = ActionCtx::take(app);
        if let Some(knob) = app.selected_knob() {
            changed |= action_editor_simple(ui, "kp", &mut knob.press, &a.pages, &mut a.macro_text, &mut a.owner, &mut a.recording);
        }
        a.give_back(app);
    });
    let summary = match (&knob.progress, knob.indicator()) {
        (Some(p), _) if p.style == "none" => "off".to_string(),
        (None, None) => "automatic: no known range".to_string(),
        (None, shown) => format!("automatic: {}", progress_summary(&shown)),
        (Some(_), shown) => progress_summary(&shown),
    };
    section(ui, "Indicator", &summary, true, false, |ui| {
        let mut pick: PickRequest = None;
        if let Some(knob) = app.selected_knob() {
            const AUTO: usize = 0;
            const CUSTOM: usize = 1;
            const OFF: usize = 2;
            let auto = deck_config::knob_progress_preset(knob);
            let mode = match &knob.progress {
                None => AUTO,
                Some(p) if p.style == "none" => OFF,
                Some(_) => CUSTOM,
            };
            let mut m = mode;
            grid(ui, "knob-indicator", |ui| {
                field(ui, "Show", "Automatic: a fill in the knob's box whenever its turn action has a known range (PC volume keys, Home Assistant brightness, volume, thermostat and number limits, Spotify volume). Custom: any state and style. Off: nothing.");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut m, AUTO, "automatic");
                    ui.selectable_value(&mut m, CUSTOM, "custom");
                    ui.selectable_value(&mut m, OFF, "off");
                });
                ui.end_row();
                if m == AUTO {
                    field(ui, "Now", "");
                    match &auto {
                        Some(p) => ui.add(egui::Label::new(format!("{} from {} to {}", p.state, p.min, p.max)).wrap()),
                        None => ui.add(egui::Label::new(RichText::new("the turn action has no known range, so nothing is shown").weak()).wrap()),
                    };
                    ui.end_row();
                }
            });
            if m != mode {
                knob.progress = match m {
                    AUTO => None,
                    OFF => Some(Progress { style: "none".into(), ..Default::default() }),
                    // start from what automatic shows, or an empty fill
                    _ => Some(auto.clone().unwrap_or(Progress { style: "fill".into(), start: "left".into(), ..Default::default() })),
                };
                changed = true;
            }
            if m == CUSTOM {
                ui.add_space(4.0);
                changed |= progress_fields(ui, "knob", &mut knob.progress, &[], None, &mut pick);
                if knob.progress.is_none() {
                    // "Remove indicator" in custom mode means off, not automatic
                    knob.progress = Some(Progress { style: "none".into(), ..Default::default() });
                }
            }
        }
        if let Some((field, start)) = pick {
            app.states_ui.pick_for(field, start.as_deref());
        }
    });
    if changed {
        app.dirty = true;
    }
}

fn side_inspector(app: &mut App, ui: &mut egui::Ui) {
    let left = app.sel == Sel::SideLeft;
    let act = if left { app.cfg.side_left.clone() } else { app.cfg.side_right.clone() };
    header(ui, if left { "Left side button" } else { "Right side button" }, &format!("does: {}", what(&act)), |_| {}, |_| {});
    section(ui, "Action", &what(&act), true, false, |ui| {
        let mut a = ActionCtx::take(app);
        let act = if left { &mut app.cfg.side_left } else { &mut app.cfg.side_right };
        let changed = action_editor_simple(ui, "side", act, &a.pages, &mut a.macro_text, &mut a.owner, &mut a.recording);
        a.give_back(app);
        if changed {
            app.dirty = true;
        }
    });
}

const EFFECT_KINDS: [&str; 4] = ["none", "Minecraft level up", "colour wave", "animation (GIF frames)"];

fn effect_kind(e: &Option<Effect>) -> usize {
    match e {
        None => 0,
        Some(Effect::LevelUp { .. }) => 1,
        Some(Effect::Wave { .. }) => 2,
        Some(Effect::Frames { .. }) => 3,
    }
}

/// Editor for an optional effect. `allow_none` offers "none" (for key press effects); an
/// `Action::Effect` always has one. Sets `want_anim` when the user wants to pick a GIF.
pub fn effect_editor(ui: &mut egui::Ui, id: &str, e: &mut Option<Effect>, want_anim: &mut bool, allow_none: bool) -> bool {
    let mut changed = false;
    let mut kind = effect_kind(e);
    ui.horizontal(|ui| {
        ui.label("Effect");
        egui::ComboBox::from_id_salt(format!("{id}-kind")).selected_text(EFFECT_KINDS[kind]).show_ui(ui, |ui| {
            for (i, k) in EFFECT_KINDS.iter().enumerate() {
                if i == 0 && !allow_none {
                    continue;
                }
                changed |= ui.selectable_value(&mut kind, i, *k).changed();
            }
        });
    });
    if kind != effect_kind(e) {
        *e = match kind {
            0 => None,
            1 => Some(Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 }),
            2 => Some(Effect::Wave { color: Some("#7fd3ff".into()), ms: 600, from: "left".into(), blocky: false }),
            _ => {
                *want_anim = true;
                Some(Effect::Frames { name: String::new(), frames: 0, delay_ms: 80 })
            }
        };
        changed = true;
    }
    match e {
        None => {
            ui.small("Plays across the whole screen when the key is pressed, on top of the tiles.");
        }
        Some(Effect::LevelUp { ms, wave_ms, every_ms }) => {
            ui.horizontal(|ui| {
                ui.label("duration");
                changed |= ui.add(egui::Slider::new(ms, 200..=4000).suffix(" ms")).changed();
                let waves = Effect::ring_count(*ms, *wave_ms, *every_ms);
                ui.label(RichText::new(format!("{waves} wave{}", if waves == 1 { "" } else { "s" })).weak());
            });
            ui.horizontal(|ui| {
                ui.label("wave speed");
                changed |= ui.add(egui::Slider::new(wave_ms, 150..=2500).suffix(" ms").logarithmic(true)).on_hover_text("time one ring needs to cross the screen: lower = faster").changed();
            });
            ui.horizontal(|ui| {
                ui.label("new wave every");
                changed |= ui.add(egui::Slider::new(every_ms, 50..=1500).suffix(" ms")).changed();
            });
            ui.small("Blocky XP-green rings rolling outwards from the pressed key, which stays drawn in front of them. Another ring starts every interval while it still fits into the duration.");
        }
        Some(Effect::Wave { color, ms, from, blocky }) => {
            ui.horizontal(|ui| {
                ui.label("colour");
                let mut c = parse_color(color, Color32::from_rgb(0x7f, 0xd3, 0xff));
                if crate::ui_color::color_field(ui, &mut c) {
                    *color = Some(color_to_string(c));
                    changed = true;
                }
                ui.label("from");
                egui::ComboBox::from_id_salt(format!("{id}-from")).selected_text(from.clone()).show_ui(ui, |ui| {
                    for s in ["left", "right", "top", "bottom"] {
                        changed |= ui.selectable_value(from, s.to_string(), s).changed();
                    }
                });
                changed |= ui.checkbox(blocky, "blocky").changed();
            });
            ui.horizontal(|ui| {
                ui.label("duration");
                changed |= ui.add(egui::Slider::new(ms, 200..=3000).suffix(" ms")).changed();
            });
        }
        Some(Effect::Frames { name, frames, delay_ms }) => {
            ui.horizontal(|ui| {
                ui.label(if name.is_empty() { "(no animation picked)".to_string() } else { format!("{name}: {frames} frames") });
                if ui.small_button("Pick GIF...").clicked() {
                    *want_anim = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("frame delay");
                changed |= ui.add(egui::Slider::new(delay_ms, 20..=500).suffix(" ms")).changed();
            });
            ui.small(format!("Frames are stored at {}x{} and blended over the tiles (transparent GIFs work). Up to {} frames.", crate::app::ANIM_W, crate::app::ANIM_H, crate::app::ANIM_MAX_FRAMES));
        }
    }
    changed
}

const KINDS: [&str; 10] = ["nothing", "key combination", "type text", "go to page", "next page", "previous page", "macro", "agent (PC) action", "show full-screen image / animation", "full-screen effect"];

fn kind_of(a: &Action) -> usize {
    match a {
        Action::None => 0,
        Action::Key { .. } => 1,
        Action::Text { .. } => 2,
        Action::Page { .. } => 3,
        Action::PageNext => 4,
        Action::PagePrev => 5,
        Action::Macro { .. } => 6,
        Action::Host { .. } => 7,
        Action::Screen { .. } => 8,
        Action::Effect { .. } => 9,
    }
}

pub fn macro_to_text(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|s| match s {
            Step::Delay { delay_ms } => format!("delay {delay_ms}"),
            Step::Act(Action::Key { keys }) => format!("key {}", keys.join("+")),
            Step::Act(Action::Text { text }) => format!("text {}", text.replace('\n', "\\n")),
            Step::Act(Action::Page { page }) => format!("page {page}"),
            Step::Act(Action::PageNext) => "next".into(),
            Step::Act(Action::PagePrev) => "prev".into(),
            Step::Act(Action::Host { plugin, op, args, .. }) => format!("host {plugin} {op} {args}"),
            Step::Act(_) => "none".into(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn text_to_macro(t: &str) -> Vec<Step> {
    t.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (cmd, arg) = l.split_once(' ').unwrap_or((l, ""));
            Some(match cmd {
                "delay" => Step::Delay { delay_ms: arg.trim().parse().ok()? },
                "key" => Step::Act(Action::Key { keys: arg.split('+').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect() }),
                "text" => Step::Act(Action::Text { text: arg.replace("\\n", "\n") }),
                "page" => Step::Act(Action::Page { page: arg.trim().to_string() }),
                "next" => Step::Act(Action::PageNext),
                "prev" => Step::Act(Action::PagePrev),
                "host" => {
                    let mut it = arg.splitn(3, ' ');
                    let plugin = it.next()?.to_string();
                    let op = it.next()?.to_string();
                    let args = it.next().and_then(|j| serde_json::from_str(j).ok()).unwrap_or(serde_json::json!({}));
                    Step::Act(Action::Host { plugin, op, args, fallback: None })
                }
                _ => return None,
            })
        })
        .collect()
}

/// Key names from an egui key event.
fn key_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match k {
        A => "a", B => "b", C => "c", D => "d", E => "e", F => "f", G => "g", H => "h", I => "i", J => "j", K => "k", L => "l", M => "m",
        N => "n", O => "o", P => "p", Q => "q", R => "r", S => "s", T => "t", U => "u", V => "v", W => "w", X => "x", Y => "y", Z => "z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4", Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "f1", F2 => "f2", F3 => "f3", F4 => "f4", F5 => "f5", F6 => "f6", F7 => "f7", F8 => "f8", F9 => "f9", F10 => "f10", F11 => "f11", F12 => "f12",
        Enter => "enter", Escape => "esc", Backspace => "backspace", Tab => "tab", Space => "space", Insert => "insert", Delete => "delete",
        Home => "home", End => "end", PageUp => "pageup", PageDown => "pagedown", ArrowLeft => "left", ArrowRight => "right", ArrowUp => "up", ArrowDown => "down",
        Minus => "-", Plus | Equals => "=", Comma => ",", Period => ".", Slash => "/", Backslash => "\\", Semicolon => ";", Quote => "'", OpenBracket => "[", CloseBracket => "]", Backtick => "`",
        _ => return None,
    })
}

/// Hotkey recorder: while active, the next key press (with modifiers) becomes the combination.
fn record_hotkey(ctx: &egui::Context) -> Option<Vec<String>> {
    ctx.input(|i| {
        for ev in &i.events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                if let Some(name) = key_name(*key) {
                    let mut v = Vec::new();
                    if modifiers.ctrl {
                        v.push("ctrl".to_string());
                    }
                    if modifiers.shift {
                        v.push("shift".to_string());
                    }
                    if modifiers.alt {
                        v.push("alt".to_string());
                    }
                    if modifiers.mac_cmd || modifiers.command && !modifiers.ctrl {
                        v.push("gui".to_string());
                    }
                    v.push(name.to_string());
                    return Some(v);
                }
            }
        }
        None
    })
}

/// Full action editor (with hotkey recorder, macro text and host action editor).
#[allow(clippy::too_many_arguments)]
pub fn action_editor(ui: &mut egui::Ui, ctx: &egui::Context, id: &str, a: &mut Action, pages: &[String], macro_text: &mut String, macro_owner: &mut bool, recording: &mut bool) -> bool {
    let mut changed = false;
    let mut kind = kind_of(a);
    ui.horizontal(|ui| {
        ui.label("Type");
        egui::ComboBox::from_id_salt(format!("{id}-kind")).selected_text(KINDS[kind]).show_ui(ui, |ui| {
            for (i, k) in KINDS.iter().enumerate() {
                if ui.selectable_value(&mut kind, i, *k).changed() {
                    changed = true;
                }
            }
        });
    });
    if kind != kind_of(a) {
        *a = match kind {
            0 => Action::None,
            1 => Action::Key { keys: vec![] },
            2 => Action::Text { text: String::new() },
            3 => Action::Page { page: pages.first().cloned().unwrap_or_default() },
            4 => Action::PageNext,
            5 => Action::PagePrev,
            6 => Action::Macro { steps: vec![] },
            7 => Action::Host { plugin: "system".into(), op: "url".into(), args: serde_json::json!({"url": "https://"}), fallback: None },
            8 => Action::Screen { image: String::new(), seconds: 0, frames: 0, delay_ms: 80 },
            _ => Action::Effect { effect: Effect::LevelUp { ms: 700, wave_ms: 700, every_ms: 250 } },
        };
        changed = true;
        if matches!(a, Action::Macro { .. }) {
            macro_text.clear();
            *macro_owner = true;
        }
    }
    ui.add_space(4.0);
    match a {
        Action::Key { keys } => {
            ui.horizontal(|ui| {
                let mut s = keys.join("+");
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(220.0)).changed() {
                    *keys = s.split('+').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
                    changed = true;
                }
                let btn = ui.selectable_label(*recording, if *recording { "press keys..." } else { "Record" }).on_hover_text("ctrl+shift+s, alt+tab, f5, gui+d, mute, volumeup, volumedown, enter, esc, tab, space, arrows");
                if btn.clicked() {
                    *recording = !*recording;
                }
            });
            if *recording {
                if let Some(v) = record_hotkey(ctx) {
                    *keys = v;
                    *recording = false;
                    changed = true;
                }
                ctx.request_repaint();
            }
        }
        Action::Text { text } => {
            let mut s = text.replace('\n', "\\n");
            if ui.add(egui::TextEdit::multiline(&mut s).desired_rows(3).desired_width(f32::INFINITY)).changed() {
                *text = s.replace("\\n", "\n");
                changed = true;
            }
            ui.small("\\n = Enter. Typed with a US layout by the deck itself.");
        }
        Action::Page { page } => {
            egui::ComboBox::from_id_salt(format!("{id}-page")).selected_text(page.clone()).show_ui(ui, |ui| {
                for p in pages {
                    if ui.selectable_value(page, p.clone(), p).changed() {
                        changed = true;
                    }
                }
            });
        }
        Action::Macro { steps } => {
            if !*macro_owner {
                *macro_text = macro_to_text(steps);
                *macro_owner = true;
            }
            ui.small("one step per line: key ctrl+c | text hello | delay 300 | page main | next | prev | host system url {\"url\":\"...\"}");
            if ui.add(egui::TextEdit::multiline(macro_text).desired_rows(6).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace)).changed() {
                *steps = text_to_macro(macro_text);
                changed = true;
            }
        }
        Action::Host { plugin, op, args, .. } => {
            ui.horizontal(|ui| {
                ui.label("Plugin");
                egui::ComboBox::from_id_salt(format!("{id}-plugin")).selected_text(plugin.clone()).show_ui(ui, |ui| {
                    for p in ["system", "media", "homeassistant", "obs", "spotify", "discord"] {
                        if ui.selectable_value(plugin, p.to_string(), p).changed() {
                            changed = true;
                        }
                    }
                });
                ui.label("Op");
                let ops: &[&str] = match plugin.as_str() {
                    "system" => &["url", "open", "close", "media"],
                    "media" => &["play_pause", "next", "prev", "stop"],
                    "obs" => &["scene", "record_toggle", "stream_toggle", "mute_toggle", "source_visible", "virtualcam_toggle", "replay_save", "studio_toggle", "transition"],
                    "spotify" => &["play_pause", "next", "prev", "shuffle_toggle", "repeat_cycle", "like_toggle", "volume", "volume_set", "play"],
                    "discord" => &["mute_toggle", "deafen_toggle", "mute", "deafen", "ptt_toggle"],
                    _ => &["toggle", "turn_on", "turn_off", "trigger", "call_service", "scene", "script", "brightness", "volume", "media", "temperature", "number", "select_next"],
                };
                egui::ComboBox::from_id_salt(format!("{id}-op")).selected_text(op.clone()).show_ui(ui, |ui| {
                    for o in ops {
                        if ui.selectable_value(op, o.to_string(), *o).changed() {
                            changed = true;
                        }
                    }
                });
            });
            let help = if plugin == "homeassistant" { "toggle on an automation enables or disables it; trigger runs it now. input_boolean: toggle / turn_on / turn_off." } else { "The library tabs below fill this in for you." };
            ui.label("Arguments (JSON)").on_hover_cursor(egui::CursorIcon::Help).on_hover_text(help);
            let mut s = serde_json::to_string(args).unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace)).changed() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    *args = v;
                    changed = true;
                }
            }
        }
        Action::Screen { image, seconds, frames, delay_ms } => {
            ui.horizontal(|ui| {
                ui.label(if *frames > 0 { "animation" } else { "image" });
                changed |= ui.add(egui::TextEdit::singleline(image).desired_width(180.0).hint_text("screen-....png")).changed();
                ui.label("seconds");
                changed |= ui.add(egui::DragValue::new(seconds)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("frames");
                changed |= ui.add(egui::DragValue::new(frames).range(0..=999)).changed();
                if *frames > 0 {
                    ui.label("delay");
                    changed |= ui.add(egui::DragValue::new(delay_ms).range(16..=5000).suffix(" ms")).changed();
                }
            });
            ui.small(if *frames > 0 {
                "Animation: <name>-f00.png ... fills the whole deck. Plays once when seconds is 0, otherwise loops that long. Any key press ends it. Pick a GIF in the library's Pages tab."
            } else {
                "Shown across the whole deck until any key is pressed (or after the timeout). Pick the file with the button in the library's Pages tab, or type a name from the icon store."
            });
        }
        Action::Effect { effect } => {
            let mut e = Some(effect.clone());
            let mut want_anim = false;
            if effect_editor(ui, &format!("{id}-fx"), &mut e, &mut want_anim, false) {
                if let Some(e) = e {
                    *effect = e;
                }
                changed = true;
            }
            if want_anim {
                // the GIF is picked by the caller (needs the app's icon store): flag it through the macro text buffer
                *macro_text = "\u{1}pick-anim".into();
            }
            ui.small("Also available per key as a press effect (Appearance tab), which keeps the key's own action.");
        }
        _ => {}
    }
    changed
}

/// Same editor without the recorder context (knob/side editors call the full one via a dummy ctx).
#[allow(clippy::too_many_arguments)]
pub fn action_editor_simple(ui: &mut egui::Ui, id: &str, a: &mut Action, pages: &[String], macro_text: &mut String, macro_owner: &mut bool, recording: &mut bool) -> bool {
    let ctx = ui.ctx().clone();
    action_editor(ui, &ctx, id, a, pages, macro_text, macro_owner, recording)
}

/// States & looks: which state drives the key, and one look per rule on its value.
fn states_editor(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let icons = app.icons_known.clone();
    // thumbnails first: the textures need the app, the editor below holds the key
    let names: Vec<String> = app.page_ref().keys.values().flat_map(|k| k.looks.values().filter_map(|l| l.icon.clone())).collect();
    let thumbs: std::collections::HashMap<String, egui::TextureHandle> = names.into_iter().filter_map(|n| app.texture(ctx, &n).map(|t| (n, t))).collect();
    let mut changed = false;
    let mut icon_for: Option<(String, bool)> = None; // (rule, minecraft)
    let mut bg_image_for: Option<String> = None;
    let mut new_rule = app.new_rule.clone();
    let mut pick: PickRequest = None;
    let Some(key) = app.selected_key() else { return };
    grid(ui, "state", |ui| {
        field(ui, "State", "The agent or the ESP32 bridge feeds this state; the look whose rule matches its value is shown.");
        ui.horizontal(|ui| {
            changed |= state_field(ui, &mut key.state, "ha:light.kitchen or obs:recording", 200.0, StateField::KeyState, None, &mut pick);
        });
        ui.end_row();
        field(ui, "Label from", "A state shown as the label. If the label contains {} the value goes there (e.g. \"{|int}/26\"), otherwise it replaces the label.");
        ui.horizontal(|ui| {
            changed |= state_field(ui, &mut key.value, "ha:sensor.temp:text", 200.0, StateField::Label, key.state.as_deref(), &mut pick);
        });
        ui.end_row();
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Looks").strong());
        ui.label(RichText::new("(?)").weak()).on_hover_cursor(egui::CursorIcon::Help).on_hover_text("One look per rule on the state's value. Rules: an exact value (on, playing), a|b|c, >20, <=5, 18..22, ~text (contains), * (anything else). Exact values win over comparisons, * comes last. Fields left empty keep the key's own.");
    });
    if key.state.is_none() && !key.looks.is_empty() {
        ui.label(RichText::new("Set a state above; looks follow its value.").color(WARN).small());
    }
    let rules: Vec<String> = key.looks.keys().cloned().collect();
    let mut remove: Option<String> = None;
    for rule in rules {
        let look = key.looks.get_mut(&rule).unwrap();
        egui::Frame::group(ui.style()).inner_margin(6.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(if rule == "*" { "otherwise".to_string() } else { format!("when {rule}") }).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✖").on_hover_text("remove this look").clicked() {
                        remove = Some(rule.clone());
                    }
                });
            });
            grid(ui, &format!("look-{rule}"), |ui| {
                field(ui, "Icon", "");
                ui.horizontal(|ui| {
                    if let Some(t) = look.icon.as_ref().and_then(|n| thumbs.get(n)) {
                        ui.add(egui::Image::new(t).max_size(egui::Vec2::splat(22.0)));
                    }
                    let cur = look.icon.as_deref().map(fit).unwrap_or_else(|| "key's icon".into());
                    egui::ComboBox::from_id_salt(format!("look-icon-{rule}")).selected_text(cur).width(150.0).show_ui(ui, |ui| {
                        if ui.selectable_label(look.icon.is_none(), "key's icon").clicked() {
                            look.icon = None;
                            changed = true;
                        }
                        for n in &icons {
                            if ui.selectable_label(look.icon.as_deref() == Some(n), n).clicked() {
                                look.icon = Some(n.clone());
                                changed = true;
                            }
                        }
                    });
                    if ui.small_button("Material...").clicked() {
                        icon_for = Some((rule.clone(), false));
                    }
                    if ui.small_button("Minecraft...").clicked() {
                        icon_for = Some((rule.clone(), true));
                    }
                });
                ui.end_row();
                field(ui, "Colours", "background and text; ✖ goes back to the key's own");
                ui.horizontal(|ui| {
                    for (slot, fallback, what) in [(&mut look.color, Color32::from_rgb(0x22, 0x2a, 0x33), "background"), (&mut look.text_color, Color32::WHITE, "text")] {
                        let mut c = parse_color(slot, fallback);
                        if crate::ui_color::color_field(ui, &mut c) {
                            *slot = Some(color_to_string(c));
                            changed = true;
                        }
                        if slot.is_some() {
                            if ui.small_button("✖").on_hover_text(format!("key's own {what}")).clicked() {
                                *slot = None;
                                changed = true;
                            }
                        } else {
                            ui.label(RichText::new(format!("key's {what}")).weak().small());
                        }
                        ui.add_space(6.0);
                    }
                });
                ui.end_row();
                field(ui, "Label", "empty keeps the key's label; templates work as in the label");
                let mut lbl = look.label.clone().unwrap_or_default();
                if ui.add(egui::TextEdit::singleline(&mut lbl).desired_width(f32::INFINITY).hint_text("key's label")).changed() {
                    look.label = if lbl.is_empty() { None } else { Some(lbl) };
                    changed = true;
                }
                ui.end_row();
                field(ui, "Deck background", "While this look is active the whole deck shows this background, on every page. Keys on the shown page win over other pages, then page order. Transparent keys let it show through.");
                ui.horizontal(|ui| {
                    let is_color = look.background.as_deref().map(|b| b.starts_with('#')).unwrap_or(false);
                    let cur = match &look.background {
                        None => "page's own".to_string(),
                        Some(_) if is_color => "colour".to_string(),
                        Some(b) => fit(b),
                    };
                    egui::ComboBox::from_id_salt(format!("look-bg-{rule}")).selected_text(cur).width(130.0).show_ui(ui, |ui| {
                        if ui.selectable_label(look.background.is_none(), "page's own").clicked() {
                            look.background = None;
                            changed = true;
                        }
                        if ui.selectable_label(is_color, "colour").clicked() && !is_color {
                            look.background = Some("#202830".into());
                            changed = true;
                        }
                        if ui.selectable_label(false, "image...").clicked() {
                            bg_image_for = Some(rule.clone());
                        }
                    });
                    if is_color {
                        let mut c = parse_color(&look.background, Color32::from_rgb(0x20, 0x28, 0x30));
                        if crate::ui_color::color_field(ui, &mut c) {
                            look.background = Some(color_to_string(c));
                            changed = true;
                        }
                    }
                });
                ui.end_row();
            });
        });
        ui.add_space(2.0);
    }
    if let Some(r) = remove {
        key.looks.remove(&r);
        changed = true;
    }
    let mut add: Vec<&str> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut new_rule).desired_width(90.0).hint_text("rule, e.g. >20"));
        if ui.button("Add look").clicked() && !new_rule.trim().is_empty() {
            key.looks.entry(new_rule.trim().to_string()).or_insert_with(Look::default);
            new_rule.clear();
            changed = true;
        }
        ui.label(RichText::new("or").weak());
        if ui.small_button("on / off").clicked() {
            add.extend(["on", "off"]);
        }
        if ui.small_button("true / false").clicked() {
            add.extend(["true", "false"]);
        }
        if ui.small_button("otherwise (*)").clicked() {
            add.push("*");
        }
    });
    for r in add {
        key.looks.entry(r.into()).or_insert_with(Look::default);
        changed = true;
    }
    app.new_rule = new_rule;
    if let Some((field, start)) = pick {
        app.states_ui.pick_for(field, start.as_deref());
    }
    if let Some((rule, minecraft)) = icon_for {
        app.mdi_target = MdiTarget::LookIcon(rule);
        if minecraft {
            app.show_mc = true;
        } else {
            app.show_mdi = true;
        }
    }
    if let Some(rule) = bg_image_for {
        if let Some(name) = app.pick_fullscreen_image("bg") {
            if let Some(look) = app.selected_key().and_then(|k| k.looks.get_mut(&rule)) {
                look.background = Some(name);
                changed = true;
            }
        }
    }
    if changed {
        app.dirty = true;
    }
}
