//! The deck canvas: a scaled picture of the deck with real icons, labels,
//! selection, knobs and side buttons.

use crate::app::{action_summary, parse_color, App, Sel};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Rounding, Stroke, StrokeKind, Vec2};

pub const OUTER: f32 = 36.0;
pub const BEZEL: f32 = 40.0;
/// Size of the deck preview at full scale (for panel sizing).
pub const FULL_W: f32 = 960.0 + 2.0 * BEZEL + 2.0 * OUTER;
pub const FULL_H: f32 = 540.0 + 90.0 + 2.0 * BEZEL + OUTER;

pub fn show(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let page = app.page_ref();
    let agent_running = app.agent_running.load(std::sync::atomic::Ordering::Relaxed);
    let bridge = crate::ui_bridge::ready_plugins(app);
    // generous padding: outer margin around the deck body, and a wide bezel inside it
    // fit the deck into the available width AND height, never above full size
    let avail_w = ui.available_width();
    let avail_h = ui.available_height();
    let scale_w = (avail_w - 2.0 * OUTER - 2.0 * BEZEL) / 960.0;
    let scale_h = (avail_h - OUTER - 2.0 * BEZEL) / (540.0 + 90.0);
    let scale = scale_w.min(scale_h).min(1.0).max(0.25);
    let bezel = BEZEL;
    let total_w = 960.0 * scale + 2.0 * bezel;
    let total_h = 540.0 * scale + 2.0 * bezel + 90.0 * scale;
    let top_left = ui.cursor().min + Vec2::new((avail_w - total_w) / 2.0, ((avail_h - total_h) / 2.0).max(OUTER * 0.5));
    let origin = top_left + Vec2::new(bezel, bezel);
    let (full, _) = ui.allocate_exact_size(Vec2::new(avail_w, avail_h), egui::Sense::hover());
    let painter = ui.painter_at(full);
    // deck body
    let body = Rect::from_min_size(top_left, Vec2::new(total_w, total_h));
    painter.rect_filled(body, 22.0 * scale.max(0.5), Color32::from_rgb(0x12, 0x14, 0x18));
    painter.rect_stroke(body, 22.0 * scale.max(0.5), Stroke::new(1.0, Color32::from_gray(50)), StrokeKind::Inside);
    let layout = app.cfg.layout.clone();
    let mut clicked: Option<Sel> = None;
    let mut menu: Option<(usize, &str)> = None;
    for i in 0..14usize {
        let (x, y, w, h) = layout.slot_rect(i);
        let r = Rect::from_min_size(origin + Vec2::new(x as f32 * scale, y as f32 * scale), Vec2::new(w as f32 * scale, h as f32 * scale));
        let key = page.keys.get(&i.to_string());
        let bg = parse_color(&key.and_then(|k| k.color.clone()), Color32::from_rgb(0x22, 0x2a, 0x33));
        let resp = ui.interact(r, ui.id().with(("tile", i)), egui::Sense::click());
        let hovered = resp.hovered();
        painter.rect_filled(r, layout.radius as f32 * scale, if hovered { lighten(bg) } else { bg });
        if app.sel == Sel::Key(i) {
            painter.rect_stroke(r, layout.radius as f32 * scale, Stroke::new(3.0, Color32::from_rgb(0xff, 0xa0, 0x00)), StrokeKind::Outside);
        }
        let pad = layout.text_pad as f32 * scale;
        let inner = r.shrink(pad);
        let label = key.map(|k| tile_placeholder(k).unwrap_or_else(|| k.label.clone())).unwrap_or_default();
        let text_col = parse_color(&key.and_then(|k| k.text_color.clone()), Color32::WHITE);
        if i == 13 {
            // label area = content box minus the knob strip at the bottom; alignment as on the deck
            let area_h = inner.height() - 44.0 * scale;
            let size = key.and_then(|k| k.font_size).map(|s| s * 0.75).unwrap_or(15.0);
            let half = size * 0.45 * scale;
            let y = match key.and_then(|k| k.valign.as_deref()) {
                Some("middle") => inner.top() + area_h / 2.0,
                Some("bottom") => inner.top() + area_h - half,
                _ => inner.top() + half,
            };
            let (anchor, x) = match key.and_then(|k| k.align.as_deref()) {
                Some("left") => (Align2::LEFT_CENTER, inner.left()),
                Some("right") => (Align2::RIGHT_CENTER, inner.right()),
                _ => (Align2::CENTER_CENTER, inner.center().x),
            };
            painter.text(Pos2::new(x, y), anchor, if label.is_empty() { "wide key".to_string() } else { label.clone() }, FontId::proportional(size * scale.max(0.6)), text_col);
            let kw = inner.width() / 3.0;
            for (k, knob) in page.knobs.iter().take(3).enumerate() {
                let kr = Rect::from_min_size(Pos2::new(inner.left() + k as f32 * kw + 3.0, inner.bottom() - 40.0 * scale), Vec2::new(kw - 6.0, 36.0 * scale));
                let kresp = ui.interact(kr, ui.id().with(("knobtile", k)), egui::Sense::click());
                painter.rect_filled(kr, 8.0 * scale, if app.sel == Sel::Knob(k) { Color32::from_rgb(0x4a, 0x6a, 0x8a) } else { Color32::from_rgb(0x2b, 0x3b, 0x4b) });
                painter.text(kr.center(), Align2::CENTER_CENTER, if knob.label.is_empty() { format!("knob {}", k + 1) } else { knob.label.clone() }, FontId::proportional(12.0 * scale.max(0.6)), Color32::from_rgb(0xd0, 0xe0, 0xf0));
                if kresp.clicked() {
                    clicked = Some(Sel::Knob(k));
                }
            }
        } else {
            let icon = key.and_then(|k| k.icon.clone());
            let mut drew_icon = false;
            if let Some(name) = icon.as_deref() {
                if let Some(tex) = app.texture(ctx, name) {
                    let size = tex.size_vec2();
                    let valign = key.and_then(|k| k.valign.as_deref()).unwrap_or("auto");
                    let strip = if label.is_empty() || valign == "middle" { 0.0 } else { 22.0 * scale };
                    let box_h = inner.height() - strip;
                    let s = (inner.width() / size.x).min(box_h / size.y).min(1.0 * scale.max(0.5));
                    let d = size * s;
                    let top = if valign == "top" { inner.top() + strip } else { inner.top() };
                    let pos = Pos2::new(inner.center().x - d.x / 2.0, top + (box_h - d.y) / 2.0);
                    painter.image(tex.id(), Rect::from_min_size(pos, d), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    drew_icon = true;
                } else {
                    painter.text(inner.left_top(), Align2::LEFT_TOP, "icon", FontId::proportional(10.0), Color32::from_gray(140));
                }
            }
            if !label.is_empty() {
                let size = key.and_then(|k| k.font_size).map(|s| s * 0.75).unwrap_or(if drew_icon { 13.0 } else { 16.0 });
                let y = match key.and_then(|k| k.valign.as_deref()) {
                    Some("top") => inner.top() + size * 0.45 * scale,
                    Some("middle") => inner.center().y,
                    Some("bottom") => inner.bottom() - size * 0.45 * scale,
                    _ => if drew_icon { inner.bottom() - 9.0 * scale } else { inner.center().y },
                };
                let (anchor, x) = match key.and_then(|k| k.align.as_deref()) {
                    Some("left") => (Align2::LEFT_CENTER, inner.left()),
                    Some("right") => (Align2::RIGHT_CENTER, inner.right()),
                    _ => (Align2::CENTER_CENTER, inner.center().x),
                };
                painter.text(Pos2::new(x, y), anchor, label.clone(), FontId::proportional(size * scale.max(0.6)), text_col);
            } else if !drew_icon {
                painter.text(inner.center(), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(14.0 * scale), Color32::from_gray(110));
            }
            if let Some(k) = key {
                let sum = action_summary(&k.action);
                if !sum.is_empty() && hovered {
                    painter.rect_filled(Rect::from_min_size(r.left_bottom() - Vec2::new(0.0, 16.0), Vec2::new(r.width(), 16.0)), 0.0, Color32::from_black_alpha(160));
                    painter.text(Pos2::new(r.center().x, r.bottom() - 8.0), Align2::CENTER_CENTER, sum, FontId::proportional(10.0), Color32::from_gray(230));
                }
                if k.state.is_some() || k.value.is_some() {
                    painter.circle_filled(r.right_top() + Vec2::new(-8.0, 8.0), 4.0, Color32::from_rgb(0x60, 0xc0, 0x80));
                }
                if k.effect.is_some() {
                    painter.text(r.right_bottom() + Vec2::new(-6.0, -6.0), Align2::RIGHT_BOTTOM, "fx", FontId::proportional(10.0), Color32::from_rgb(0x80, 0xff, 0x20));
                }
                if unserved(&k.action, agent_running, &bridge) {
                    let c = r.left_top() + Vec2::new(9.0, 9.0);
                    painter.circle_filled(c, 6.5, Color32::from_rgb(0xd0, 0x30, 0x30));
                    painter.text(c, Align2::CENTER_CENTER, "!", FontId::proportional(10.0), Color32::WHITE);
                    if hovered {
                        painter.rect_filled(Rect::from_min_size(r.left_top(), Vec2::new(r.width(), 16.0)), 0.0, Color32::from_black_alpha(160));
                        painter.text(Pos2::new(r.center().x, r.top() + 8.0), Align2::CENTER_CENTER, "needs the agent (not running, no bridge)", FontId::proportional(10.0), Color32::from_rgb(0xff, 0x90, 0x90));
                    }
                }
                if let Some(p) = &k.progress {
                    let pc = parse_color(&Some(p.color.clone()), Color32::GOLD);
                    if p.style == "fill" {
                        let fr = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - r.height() * 0.6), r.right_bottom());
                        painter.rect_filled(fr, 0.0, Color32::from_rgba_unmultiplied(pc.r(), pc.g(), pc.b(), 70));
                    } else {
                        painter.rect_stroke(r.shrink(2.0), layout.radius as f32 * scale, Stroke::new(3.0, pc), StrokeKind::Inside);
                    }
                }
            }
        }
        if resp.clicked() {
            clicked = Some(Sel::Key(i));
        }
        resp.context_menu(|ui| {
            if ui.button("Copy").clicked() {
                menu = Some((i, "copy"));
                ui.close_menu();
            }
            if ui.button("Cut").clicked() {
                menu = Some((i, "cut"));
                ui.close_menu();
            }
            if ui.add_enabled(app.clipboard.is_some(), egui::Button::new("Paste")).clicked() {
                menu = Some((i, "paste"));
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Clear").clicked() {
                menu = Some((i, "clear"));
                ui.close_menu();
            }
        });
    }
    if let Some((i, what)) = menu {
        match what {
            "copy" => app.copy_key(i),
            "cut" => app.cut_key(i),
            "paste" => app.paste_key(i),
            _ => {
                app.current_page().keys.insert(i.to_string(), deck_config::KeyDef::default());
                app.dirty = true;
            }
        }
    }
    // keyboard shortcuts while a key is selected and no text field has the focus
    if let Sel::Key(i) = app.sel {
        if !ctx.wants_keyboard_input() && !crate::ui_color::picker_open() {
            let (copy, cut, paste) = ctx.input(|inp| (inp.modifiers.command && inp.key_pressed(egui::Key::C), inp.modifiers.command && inp.key_pressed(egui::Key::X), inp.modifiers.command && inp.key_pressed(egui::Key::V)));
            if copy {
                app.copy_key(i);
            } else if cut {
                app.cut_key(i);
            } else if paste {
                app.paste_key(i);
            }
        }
    }
    // bottom row, as on the device: left button, right button, knob 1, 2, 3, space-evenly
    let row_y = origin.y + 540.0 * scale + bezel * 0.5 + 45.0 * scale;
    let slot_w = 960.0 * scale / 5.0;
    for (idx, item) in [Sel::SideLeft, Sel::SideRight, Sel::Knob(0), Sel::Knob(1), Sel::Knob(2)].into_iter().enumerate() {
        let cx = origin.x + (idx as f32 + 0.5) * slot_w;
        let c = Pos2::new(cx, row_y);
        let selected = app.sel == item;
        match item {
            Sel::Knob(k) => {
                let r = Rect::from_center_size(c, Vec2::splat(44.0 * scale));
                let resp = ui.interact(r, ui.id().with(("knob", k)), egui::Sense::click());
                painter.circle_filled(c, 22.0 * scale, if selected { Color32::from_rgb(0xff, 0xa0, 0x00) } else if resp.hovered() { Color32::from_gray(120) } else { Color32::from_gray(70) });
                painter.circle_filled(c, 16.0 * scale, Color32::from_rgb(0x30, 0x34, 0x3a));
                if page.knobs.get(k).map(|kn| unserved(&kn.press, agent_running, &bridge) || unserved(&kn.left, agent_running, &bridge) || unserved(&kn.right, agent_running, &bridge)).unwrap_or(false) {
                    painter.circle_filled(c + Vec2::new(16.0 * scale, -16.0 * scale), 6.0, Color32::from_rgb(0xd0, 0x30, 0x30));
                }
                let label = page.knobs.get(k).map(|kn| kn.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| format!("knob {}", k + 1));
                painter.text(c + Vec2::new(0.0, 30.0 * scale), Align2::CENTER_CENTER, label, FontId::proportional(11.0), Color32::from_gray(170));
                if resp.clicked() {
                    clicked = Some(item);
                }
            }
            _ => {
                let r = Rect::from_center_size(c, Vec2::new(40.0 * scale, 26.0 * scale));
                let resp = ui.interact(r, ui.id().with(("side", idx)), egui::Sense::click());
                painter.rect_filled(r, 6.0 * scale, if selected { Color32::from_rgb(0xff, 0xa0, 0x00) } else if resp.hovered() { Color32::from_gray(120) } else { Color32::from_gray(70) });
                let act = if item == Sel::SideLeft { &app.cfg.side_left } else { &app.cfg.side_right };
                if unserved(act, agent_running, &bridge) {
                    painter.circle_filled(r.right_top() + Vec2::new(2.0, -2.0), 6.0, Color32::from_rgb(0xd0, 0x30, 0x30));
                }
                let label = match act { deck_config::Action::PagePrev => "prev page".to_string(), deck_config::Action::PageNext => "next page".to_string(), a => action_summary(a) };
                painter.text(c + Vec2::new(0.0, 24.0 * scale), Align2::CENTER_CENTER, if label.is_empty() { if idx == 0 { "button 1".into() } else { "button 2".into() } } else { label }, FontId::proportional(11.0), Color32::from_gray(170));
                if resp.clicked() {
                    clicked = Some(item);
                }
            }
        }
    }
    painter.text(Pos2::new(origin.x, origin.y - bezel * 0.5), Align2::LEFT_CENTER, format!("page: {}", app.page), FontId::proportional(12.0), Color32::from_gray(140));
    if let Some(s) = clicked {
        if s != app.sel {
            app.sel = s;
            app.macro_for = Sel::None;
            app.macro_text.clear();
            app.recording_hotkey = false;
        }
    }
    let _ = Rounding::default();
}

fn lighten(c: Color32) -> Color32 {
    let f = |v: u8| (v as u16 + (255 - v as u16) / 4) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Does the action (or any macro step) need the PC agent?
/// Label shown in the preview for a key whose image comes from a state (AgentDeck tiles).
pub fn tile_placeholder(k: &deck_config::KeyDef) -> Option<String> {
    k.tile.as_deref().map(|t| if t.starts_with("agentdeck:") { "AgentDeck".to_string() } else { "live image".to_string() })
}

/// A host action that nothing would run: the PC agent is stopped and the ESP32 bridge does not
/// report its plugin ready (`bridge`: ready plugins, empty without the mod).
pub fn unserved(a: &deck_config::Action, agent_running: bool, bridge: &[String]) -> bool {
    if agent_running {
        return false;
    }
    let host_unserved = |x: &deck_config::Action| matches!(x, deck_config::Action::Host { plugin, .. } if !bridge.iter().any(|b| b == plugin));
    match a {
        deck_config::Action::Macro { steps } => steps.iter().any(|st| matches!(st, deck_config::Step::Act(x) if host_unserved(x))),
        x => host_unserved(x),
    }
}

