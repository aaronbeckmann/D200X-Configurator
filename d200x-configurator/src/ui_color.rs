//! The colour field used throughout the configurator: egui's picker plus paste and recent colours.
//! Recent colours live in %APPDATA%\d200x\colors.json; a colour counts as picked when its popup closes.

use crate::app::color_to_string;
use egui::color_picker::{color_picker_color32, show_color_at, Alpha};
use egui::{Area, Color32, Context, Event, Frame, Id, Key, Order, Rect, Response, Sense, StrokeKind, Ui, UiKind, Vec2, ViewportCommand};
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_RECENT: usize = 16;

/// The popup showing right now.
#[derive(Clone, Copy)]
struct OpenPicker {
    id: Id,
    from: Color32,
    current: Color32,
    seen_pass: u64,
}

struct State {
    recent: Option<Vec<Color32>>,
    open: Option<OpenPicker>,
}

static STATE: Mutex<State> = Mutex::new(State { recent: None, open: None });

fn colors_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("d200x").join("colors.json")
}

fn load_recent() -> Vec<Color32> {
    let names: Vec<String> = std::fs::read(colors_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    names.iter().filter_map(|n| parse_color(n)).collect()
}

/// A colour as text: `#rrggbb`, `rrggbb` or `r, g, b` (what the picker's copy button gives).
fn parse_color(text: &str) -> Option<Color32> {
    let v = deck_config::parse_color_value(text)?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Newest first, each colour once.
fn push_recent(recent: &mut Vec<Color32>, c: Color32) {
    recent.retain(|x| *x != c);
    recent.insert(0, c);
    recent.truncate(MAX_RECENT);
}

fn remember(st: &mut State, c: Color32) {
    let recent = st.recent.get_or_insert_with(load_recent);
    push_recent(recent, c);
    let names: Vec<String> = recent.iter().map(|c| color_to_string(*c)).collect();
    if let Ok(b) = serde_json::to_vec(&names) {
        let _ = std::fs::write(colors_path(), b);
    }
}

/// Run once per frame, before the UI: records the colour of a popup that closed. A popup is also
/// over when its field disappeared, e.g. because another key was selected.
pub fn finish_closed_picker(ctx: &Context) {
    let mut st = STATE.lock().unwrap();
    let Some(open) = st.open else { return };
    let gone = open.seen_pass + 1 < ctx.cumulative_pass_nr();
    if gone || !ctx.memory(|m| m.is_popup_open(open.id)) {
        if gone {
            ctx.memory_mut(|m| m.close_popup());
        }
        st.open = None;
        if open.current != open.from {
            remember(&mut st, open.current);
        }
    }
}

/// A colour popup is open, so Ctrl+V belongs to it.
pub fn picker_open() -> bool {
    STATE.lock().unwrap().open.is_some()
}

fn swatch(ui: &mut Ui, size: Vec2, c: Color32) -> Response {
    let (rect, resp): (Rect, Response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&resp);
        let rect = rect.expand(v.expansion);
        show_color_at(ui.painter(), c, rect);
        ui.painter().rect_stroke(rect, v.corner_radius, v.bg_stroke, StrokeKind::Inside);
    }
    resp
}

/// Colour button with the popup. Returns true when the colour changed.
pub fn color_field(ui: &mut Ui, c: &mut Color32) -> bool {
    let popup_id = ui.auto_id_with("color-popup");
    let button = swatch(ui, ui.spacing().interact_size, *c);
    if button.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    if !ui.memory(|m| m.is_popup_open(popup_id)) {
        return false;
    }
    let mut changed = false;
    let recent = STATE.lock().unwrap().recent.get_or_insert_with(load_recent).clone();
    let bad_paste_id = popup_id.with("bad-paste");
    let area = Area::new(popup_id).kind(UiKind::Picker).order(Order::Foreground).fixed_pos(button.rect.max).show(ui.ctx(), |ui| {
        ui.spacing_mut().slider_width = 275.0;
        Frame::popup(ui.style()).show(ui, |ui| {
            changed |= color_picker_color32(ui, c, Alpha::Opaque);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(color_to_string(*c));
                if ui.button("paste").on_hover_text("colour from the clipboard: #rrggbb or r, g, b (also Ctrl+V)").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::RequestPaste);
                    ui.ctx().request_repaint();
                }
                if ui.data(|d| d.get_temp::<bool>(bad_paste_id)).unwrap_or(false) {
                    ui.colored_label(Color32::LIGHT_RED, "clipboard holds no colour");
                }
            });
            ui.label("Recent");
            if recent.is_empty() {
                ui.small("Colours you pick show up here.");
            }
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(290.0);
                for r in &recent {
                    if swatch(ui, Vec2::splat(26.0), *r).on_hover_text(color_to_string(*r)).clicked() {
                        *c = *r;
                        changed = true;
                    }
                }
            });
        });
    });
    let pasted: Option<String> = ui.input(|i| i.events.iter().rev().find_map(|e| if let Event::Paste(t) = e { Some(t.clone()) } else { None }));
    if let Some(text) = pasted {
        let color = parse_color(&text);
        ui.data_mut(|d| d.insert_temp(bad_paste_id, color.is_none()));
        ui.ctx().request_repaint();
        if let Some(p) = color {
            *c = p;
            changed = true;
        }
    }
    let pass = ui.ctx().cumulative_pass_nr();
    {
        let mut st = STATE.lock().unwrap();
        let from = st.open.filter(|o| o.id == popup_id).map(|o| o.from).unwrap_or(*c);
        st.open = Some(OpenPicker { id: popup_id, from, current: *c, seen_pass: pass });
    }
    if !button.clicked() && (ui.input(|i| i.key_pressed(Key::Escape)) || area.response.clicked_elsewhere()) {
        ui.memory_mut(|m| m.close_popup());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_the_copy_button_gives() {
        assert_eq!(parse_color("245, 180, 0"), Some(Color32::from_rgb(245, 180, 0)));
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(parse_color("#f5b400"), Some(Color32::from_rgb(0xf5, 0xb4, 0x00)));
        assert_eq!(parse_color(" f5b400\n"), Some(Color32::from_rgb(0xf5, 0xb4, 0x00)));
    }

    #[test]
    fn other_text_is_no_colour() {
        assert_eq!(parse_color("hello"), None);
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("1, 2"), None);
    }

    #[test]
    fn recent_is_newest_first_without_duplicates() {
        let (a, b) = (Color32::RED, Color32::BLUE);
        let mut r = vec![];
        push_recent(&mut r, a);
        push_recent(&mut r, b);
        push_recent(&mut r, a);
        assert_eq!(r, vec![a, b]);
    }

    #[test]
    fn recent_is_capped() {
        let mut r = vec![];
        for i in 0..40 {
            push_recent(&mut r, Color32::from_rgb(i, 0, 0));
        }
        assert_eq!(r.len(), MAX_RECENT);
        assert_eq!(r[0], Color32::from_rgb(39, 0, 0));
    }
}
