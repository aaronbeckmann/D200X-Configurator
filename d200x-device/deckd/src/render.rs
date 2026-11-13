//! Page rendering: key tiles with labels and PNG icons, knob strip in the wide slot.
//!
//! `build_page` turns config and states into plain data (`PageFrame`, one `TileLook` per slot) on
//! the input thread; `Renderer` paints that data on the render thread. Comparing two looks tells
//! which tiles changed, see compose.rs.

use crate::canvas::{over, Canvas, Rect, H, W};
use crate::config::{parse_color, Config, Layout, Page};
use crate::ring::Ring;
use std::collections::HashMap;
use std::sync::Arc;

/// Canvas colour behind the tiles.
pub const PAGE_COLOR: u32 = 0xff101418;

pub struct Renderer {
    font: Option<Arc<fontdue::Font>>,
    icons: HashMap<String, Arc<Icon>>,
    /// Per-key fonts from the icon store (`font-xxx.ttf`), loaded on demand; None = failed to load.
    fonts: HashMap<String, Option<Arc<fontdue::Font>>>,
    /// Rasterised glyphs by font, character and size.
    glyphs: HashMap<(usize, char, u32), Glyph>,
    /// The picture each animated or image tile shows now, so that repainting the tile (press
    /// animation, effect in front) does not decode it again.
    tile_images: Vec<Option<(String, Arc<Icon>)>>,
    /// The last full-screen animation frame: frames last longer than a screen refresh.
    full_frame: Option<(String, Arc<Icon>)>,
    /// Page background scaled to the canvas, by image name.
    background: Option<(String, Vec<u32>)>,
}

struct Glyph {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

/// How a key's label is drawn.
#[derive(Clone, Copy, Default)]
pub struct TextStyle<'a> {
    /// Fixed size in px; None = automatic (up to 26, shrinking to fit).
    pub size: Option<f32>,
    /// "left", "right" or anything else = centred.
    pub align: &'a str,
    /// Font file name in the icon store; None = the deck font.
    pub font: Option<&'a str>,
}

pub struct Icon {
    w: usize,
    h: usize,
    px: Vec<u32>, // ARGB
}

// Only small fonts: the deck has 33 MB of RAM for Linux and the 3 MB CJK fonts
// in the rootfs get the process OOM-killed while fontdue parses them.
const FONT_PATHS: [&str; 3] = ["/data/deckd/font.ttf", "/tmp/font.ttf", "/res/font/font.ttf"];

/// Parsed fonts are large (7 MB for DejaVu Sans): share them, a clone per label takes 130 ms on the
/// deck and can get deckd OOM-killed.
fn parse_font(bytes: Vec<u8>) -> Result<Arc<fontdue::Font>, &'static str> {
    fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map(Arc::new)
}

impl Renderer {
    pub fn new() -> (Renderer, String) {
        let mut log = String::new();
        let mut font = None;
        for p in FONT_PATHS {
            if let Ok(bytes) = std::fs::read(p) {
                match parse_font(bytes) {
                    Ok(f) => {
                        log = format!("font {p}");
                        font = Some(f);
                        break;
                    }
                    Err(e) => log = format!("font {p} failed: {e}"),
                }
            }
        }
        let r = Renderer { font, icons: HashMap::new(), fonts: HashMap::new(), glyphs: HashMap::new(), tile_images: vec![None; 14], full_frame: None, background: None };
        (r, log)
    }

    /// The font to use for a style: a per-key font from the icon store, else the deck font.
    fn font_for(&mut self, name: Option<&str>) -> Option<Arc<fontdue::Font>> {
        if let Some(n) = name {
            if !self.fonts.contains_key(n) {
                // fonts are a few hundred KB each: keep the cache small
                if self.fonts.len() >= 4 {
                    self.forget_fonts();
                }
                let f = std::fs::read(format!("{}/{n}", crate::config::ICON_DIR)).ok().and_then(|b| parse_font(b).ok());
                self.fonts.insert(n.to_string(), f);
            }
            if let Some(Some(f)) = self.fonts.get(n) {
                return Some(f.clone());
            }
        }
        self.font.clone()
    }

    pub fn forget_fonts(&mut self) {
        self.fonts.clear();
        self.glyphs.clear();
    }

    fn text_width_in(font: Option<&fontdue::Font>, text: &str, size: f32) -> f32 {
        let Some(font) = font else { return text.len() as f32 * size * 0.6 };
        text.chars().map(|c| font.metrics(c, size).advance_width).sum()
    }

    fn draw_text_in(c: &mut Canvas, font: Option<&fontdue::Font>, glyphs: &mut HashMap<(usize, char, u32), Glyph>, text: &str, x: f32, y_baseline: f32, size: f32, argb: u32) {
        let Some(font) = font else { return };
        let id = font as *const fontdue::Font as usize;
        let mut pen = x;
        for ch in text.chars() {
            if glyphs.len() >= 512 {
                glyphs.clear();
            }
            let g = glyphs.entry((id, ch, size.to_bits())).or_insert_with(|| {
                let (metrics, bitmap) = font.rasterize(ch, size);
                Glyph { metrics, bitmap }
            });
            let m = g.metrics;
            let gx = pen as i32 + m.xmin;
            let gy = y_baseline as i32 - m.ymin - m.height as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    c.blend(gx + col as i32, gy + row as i32, argb, g.bitmap[row * m.width + col]);
                }
            }
            pen += m.advance_width;
        }
    }

    /// Label inside the box [bx, bx+bw] centred vertically on `cy`, with a style: fixed or
    /// automatic size, left/centre/right alignment, optional font. Up to two lines (explicit
    /// newline, or a split at the last space when the text does not fit).
    pub fn draw_label_styled(&mut self, c: &mut Canvas, text: &str, bx: i32, cy: i32, bw: i32, argb: u32, st: TextStyle) {
        if text.is_empty() {
            return;
        }
        let font = self.font_for(st.font);
        let font = font.as_deref();
        let glyphs = &mut self.glyphs;
        let mut place = |c: &mut Canvas, line: &str, size: f32, cy: i32| {
            let w = Self::text_width_in(font, line, size);
            let x = match st.align {
                "left" => bx as f32,
                "right" => (bx + bw) as f32 - w,
                _ => bx as f32 + (bw as f32 - w) / 2.0,
            };
            Self::draw_text_in(c, font, glyphs, line, x, cy as f32 + size * 0.35, size, argb);
        };
        let gap = st.size.map(|s| (s * 0.55).round() as i32).unwrap_or(14);
        if let Some((a, b)) = text.split_once('\n') {
            let b = b.lines().next().unwrap_or("").trim();
            let size = st.size.unwrap_or_else(|| Self::fit_size(font, a.trim(), bw).min(Self::fit_size(font, b, bw)));
            place(c, a.trim(), size, cy - gap);
            place(c, b, size, cy + gap);
            return;
        }
        let size = st.size.unwrap_or_else(|| Self::fit_size(font, text, bw));
        if Self::text_width_in(font, text, size) > bw as f32 {
            if let Some(split) = text.rfind(' ') {
                let (a, b) = (&text[..split], &text[split + 1..]);
                place(c, a, size, cy - gap);
                place(c, b, size, cy + gap);
                return;
            }
        }
        place(c, text, size, cy);
    }

    /// Automatic size: 26 px, shrinking in 2 px steps down to 12 until the text fits.
    fn fit_size(font: Option<&fontdue::Font>, text: &str, max_w: i32) -> f32 {
        let mut size = 26.0;
        while size > 12.0 && Self::text_width_in(font, text, size) > max_w as f32 {
            size -= 2.0;
        }
        size
    }

    pub fn icon(&mut self, name: &str) -> Option<Arc<Icon>> {
        if let Some(i) = self.icons.get(name) {
            return Some(i.clone());
        }
        let dir = if name.starts_with("ram-") { crate::config::RAM_ICON_DIR } else { crate::config::ICON_DIR };
        let icon = Arc::new(load_png(&format!("{dir}/{name}"))?);
        // keep the cache small
        if self.icons.len() >= 48 {
            self.icons.clear();
        }
        self.icons.insert(name.to_string(), icon.clone());
        Some(icon)
    }

    pub fn clear_icons(&mut self) {
        self.icons.clear();
        self.tile_images.iter_mut().for_each(|t| *t = None);
        self.full_frame = None;
        self.background = None;
    }

    pub fn forget_icon(&mut self, name: &str) {
        self.icons.remove(name);
        for t in self.tile_images.iter_mut() {
            if t.as_ref().map(|t| t.0 == name).unwrap_or(false) {
                *t = None;
            }
        }
        if self.background.as_ref().map(|b| b.0 == name).unwrap_or(false) {
            self.background = None;
        }
    }

    /// The picture of an image tile: frames are decoded on the fly and only the current one per
    /// key is kept, dozens of 150x150 images per key would exhaust the RAM.
    fn tile_picture(&mut self, slot: usize, img: &TileImage) -> Option<Arc<Icon>> {
        if let Some((name, icon)) = &self.tile_images[slot] {
            if *name == img.file {
                return Some(icon.clone());
            }
        }
        let icon = if img.file == img.fallback {
            self.icon(&img.file)?
        } else {
            load_icon_file(&img.file).map(Arc::new).or_else(|| self.icon(&img.fallback))?
        };
        self.tile_images[slot] = Some((img.file.clone(), icon.clone()));
        Some(icon)
    }

    /// Full-tile image: scaled to fit, centred on a dark rounded tile.
    fn draw_tile_image(&mut self, c: &mut Canvas, slot: usize, img: &TileImage, x: i32, y: i32, w: i32, h: i32, radius: i32) -> bool {
        let Some(icon) = self.tile_picture(slot, img) else { return false };
        c.rounded_rect(x, y, w, h, radius, 0xff000000);
        let scale = (w as f32 / icon.w as f32).min(h as f32 / icon.h as f32);
        let (dw, dh) = ((icon.w as f32 * scale) as i32, (icon.h as f32 * scale) as i32);
        blit_icon(c, &icon, x + (w - dw) / 2, y + (h - dh) / 2, dw, dh);
        true
    }

    pub fn text_width(&self, text: &str, size: f32) -> f32 {
        Self::text_width_in(self.font.as_deref(), text, size)
    }

    pub fn draw_text(&mut self, c: &mut Canvas, text: &str, x: f32, y_baseline: f32, size: f32, argb: u32) {
        let font = self.font.clone();
        Self::draw_text_in(c, font.as_deref(), &mut self.glyphs, text, x, y_baseline, size, argb);
    }

    /// Centered text, shrinking to fit the width, up to two lines split on space.
    pub fn draw_label(&mut self, c: &mut Canvas, text: &str, cx: i32, cy: i32, max_w: i32, argb: u32) {
        if text.is_empty() {
            return;
        }
        // explicit line break: two lines, each fitted on its own
        if let Some((a, b)) = text.split_once('\n') {
            self.draw_label(c, a.trim(), cx, cy - 14, max_w, argb);
            self.draw_label(c, b.trim().lines().next().unwrap_or(""), cx, cy + 14, max_w, argb);
            return;
        }
        let mut size = 26.0;
        while size > 12.0 && self.text_width(text, size) > max_w as f32 {
            size -= 2.0;
        }
        if self.text_width(text, size) > max_w as f32 {
            if let Some(split) = text.rfind(' ') {
                let (a, b) = (&text[..split], &text[split + 1..]);
                self.draw_label(c, a, cx, cy - 14, max_w, argb);
                self.draw_label(c, b, cx, cy + 14, max_w, argb);
                return;
            }
        }
        let w = self.text_width(text, size);
        self.draw_text(c, text, cx as f32 - w / 2.0, cy as f32 + size * 0.35, size, argb);
    }
}

pub fn load_png(path: &str) -> Option<Icon> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let mut px = Vec::with_capacity(w * h);
    match info.color_type {
        png::ColorType::Rgba => {
            for p in bytes.chunks_exact(4) {
                px.push(((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32);
            }
        }
        png::ColorType::Rgb => {
            for p in bytes.chunks_exact(3) {
                px.push(0xff000000 | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32);
            }
        }
        png::ColorType::Grayscale => {
            for p in bytes.chunks_exact(1) {
                let v = p[0] as u32;
                px.push(0xff000000 | (v << 16) | (v << 8) | v);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for p in bytes.chunks_exact(2) {
                let v = p[0] as u32;
                px.push(((p[1] as u32) << 24) | (v << 16) | (v << 8) | v);
            }
        }
        _ => return None,
    }
    Some(Icon { w, h, px })
}

pub fn blit_icon(c: &mut Canvas, icon: &Icon, x: i32, y: i32, max_w: i32, max_h: i32) {
    // nearest-neighbour scale to fit
    let scale = (max_w as f32 / icon.w as f32).min(max_h as f32 / icon.h as f32).min(1.0);
    let dw = (icon.w as f32 * scale) as i32;
    let dh = (icon.h as f32 * scale) as i32;
    let ox = x + (max_w - dw) / 2;
    let oy = y + (max_h - dh) / 2;
    let columns: Vec<usize> = (0..dw).map(|dx| ((dx as f32 / scale) as usize).min(icon.w - 1)).collect();
    for dy in 0..dh {
        let sy = ((dy as f32 / scale) as usize).min(icon.h - 1);
        let row = &icon.px[sy * icon.w..(sy + 1) * icon.w];
        for (dx, sx) in columns.iter().enumerate() {
            let p = row[*sx];
            c.blend(ox + dx as i32, oy + dy, p & 0x00ffffff, (p >> 24) as u8);
        }
    }
}

pub struct ViewState<'a> {
    pub page_name: &'a str,
    pub pressed: u16,
    pub knob_pressed: u8,
    pub status: &'a str,
    /// Agent-fed states: name -> value.
    pub states: &'a std::collections::HashMap<String, String>,
    /// Keys currently flashing red (bitmask like `pressed`).
    pub error_flash: u16,
    pub agent_alive: bool,
    /// Which agents are connected: "pc", "esp", "pc+esp".
    pub agent_label: &'a str,
    /// Press animation progress per key (0.0 = just pressed, 1.0 = done).
    pub anim: &'a [(u16, f32)],
    /// Monotonic clock for animated tiles.
    pub now_ms: u64,
}

/// A full-tile image: the file to show, and the file to fall back on for a missing frame.
#[derive(Clone, PartialEq)]
pub struct TileImage {
    file: String,
    fallback: String,
}

#[derive(Clone, PartialEq)]
pub struct ProgressLook {
    frac: f32,
    color: u32,
    fill: bool,
    start: String,
    width: i32,
}

/// Everything that decides how a tile looks: tiles with equal looks have equal pixels.
#[derive(Clone, PartialEq)]
pub struct TileLook {
    pub slot: usize,
    /// The slot on the canvas; nothing of the tile is drawn outside it.
    pub area: Rect,
    /// The tile in its slot: smaller while the press animation runs.
    rect: Rect,
    radius: i32,
    text_pad: i32,
    /// Background colour and opacity.
    fill: Option<(u32, u8)>,
    progress: Option<ProgressLook>,
    /// False for an empty slot: nothing more than the background.
    has_key: bool,
    image: Option<TileImage>,
    icon: Option<String>,
    label: String,
    text_color: u32,
    font_size: Option<f32>,
    align: String,
    font: Option<String>,
    valign: Option<String>,
    /// Wide slot: knob labels, their pressed state and their indicators.
    knobs: Vec<(String, bool, Option<ProgressLook>)>,
}

impl TileLook {
    pub fn uses(&self, name: &str) -> bool {
        self.icon.as_deref() == Some(name) || self.image.as_ref().map(|i| i.file == name || i.fallback == name).unwrap_or(false)
    }
}

/// A full-screen effect at `ms` after its start; `slot` stays in front of it.
#[derive(Clone)]
pub struct EffectLook {
    pub effect: deck_config::Effect,
    pub ms: u64,
    pub origin: (i32, i32),
    pub slot: Option<usize>,
}

pub struct PageFrame {
    pub background: Option<String>,
    pub tiles: Vec<TileLook>,
    pub status: String,
    pub status_color: u32,
    /// The status line's box; it runs along the bottom of the gap above the last row.
    pub status_area: Rect,
    pub effects: Vec<EffectLook>,
}

/// Effective icon / colour / text colour / label of a key under the current states.
fn resolve_look(key: &deck_config::KeyDef, states: &std::collections::HashMap<String, String>) -> (Option<String>, Option<String>, Option<String>, String) {
    let mut icon = key.icon.clone();
    let mut color = key.color.clone();
    let mut text_color = key.text_color.clone();
    let mut label = key.label.clone();
    if let Some(state) = key.state.as_deref().and_then(|s| states.get(s)) {
        if let Some(look) = deck_config::pick_look(&key.looks, state) {
            if look.icon.is_some() {
                icon = look.icon.clone();
            }
            if look.color.is_some() {
                color = look.color.clone();
            }
            if look.text_color.is_some() {
                text_color = look.text_color.clone();
            }
            if let Some(l) = &look.label {
                label = l.clone();
            }
        }
    }
    // label templates: `{}` = the value state, `{name|fmt}` = any state; a plain label is
    // replaced by the value state when one is set
    let value = key.value.as_deref().and_then(|s| states.get(s)).cloned();
    label = deck_config::render_label(&label, value.as_deref(), &|n| states.get(n).cloned());
    // background from a state value (a light's rgb_color, a "#rrggbb" sensor...), when it parses
    if let Some(c) = key.color_state.as_deref().and_then(|s| states.get(s)).and_then(|v| deck_config::parse_color_value(v)) {
        color = Some(format!("#{:06x}", c & 0xffffff));
    }
    (icon, color, text_color, label)
}

/// A progress indicator under the current states; None while its state is unknown.
fn progress_look(p: &deck_config::Progress, states: &std::collections::HashMap<String, String>) -> Option<ProgressLook> {
    let frac = p.fraction_in(states)?;
    let color = p.color_state.as_deref().and_then(|s| states.get(s)).and_then(|s| deck_config::parse_color_value(s));
    Some(ProgressLook { frac: frac as f32, color: color.unwrap_or_else(|| parse_color(&p.color, 0xfff5b400)), fill: p.style == "fill", start: p.start.clone(), width: p.width })
}

/// The whole page as plain data.
pub fn build_page(cfg: &Config, page: &Page, v: &ViewState, effects: Vec<EffectLook>) -> PageFrame {
    let layout = &cfg.layout;
    let dot = if v.agent_alive { v.agent_label } else { "no agent" };
    PageFrame {
        // a key's active look can set the background of every page
        background: cfg.state_background(v.page_name, v.states).or_else(|| page.background.clone()),
        tiles: (0..14).map(|i| tile_look(cfg, page, v, i)).collect(),
        status: format!("{}  |  {}  |  {}", dot, v.page_name, v.status),
        status_color: if v.agent_alive { 0xff60c080 } else { 0xff6a7a8a },
        status_area: (0, layout.row_y[2] - 20, W as i32, 20),
        effects,
    }
}

/// One tile of the page: background (with press animation, error flash), progress, tile image
/// or icon + label; the wide slot also carries the knob labels.
fn tile_look(cfg: &Config, page: &Page, v: &ViewState, i: usize) -> TileLook {
    let layout = &cfg.layout;
    let area = layout.slot_rect(i);
    let (mut x, mut y, mut w, mut h) = area;
    let key = page.keys.get(&i.to_string());
    let bit = if i == 13 { 1 } else { 0x4000 >> i };
    let pressed = v.pressed & bit != 0;
    // press animation: the tile shrinks while pressed and springs back on release
    let inset = if pressed {
        6
    } else if let Some((_, t)) = v.anim.iter().find(|(b, _)| *b == bit) {
        (6.0 * (1.0 - t)).round() as i32
    } else {
        0
    };
    if inset > 0 {
        x += inset;
        y += inset;
        w -= 2 * inset;
        h -= 2 * inset;
    }
    let resolved = key.map(|k| resolve_look(k, v.states));
    let transparent = resolved.as_ref().and_then(|r| r.1.as_deref()).map(|s| s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent")).unwrap_or(false);
    let base = resolved.as_ref().and_then(|r| r.1.as_deref()).map(|s| parse_color(s, 0xff2a3440)).unwrap_or(0xff222a33);
    let color = if v.error_flash & bit != 0 {
        0xffc0392b
    } else if pressed {
        lighten(base)
    } else {
        base
    };
    let fill = if !transparent {
        Some((color, 255))
    } else if pressed || v.error_flash & bit != 0 {
        Some((color, 90))
    } else {
        None
    };
    let progress = key.and_then(|k| k.progress.as_ref()).and_then(|p| progress_look(p, v.states));
    let image = key.and_then(|k| k.tile.as_deref()).and_then(|t| v.states.get(t)).and_then(|spec| {
        let (base, frames, delay) = deck_config::parse_tile(spec)?;
        let fallback = format!("{base}.png");
        // the tiles step one after the other: 14 frames decoded at the same moment would stall
        // the whole screen for 100 ms on every step
        let frame = (v.now_ms + i as u64 * delay / 14) / delay;
        Some(TileImage { file: if frames > 0 { format!("{base}-f{:02}.png", frame % frames as u64) } else { fallback.clone() }, fallback })
    });
    let (icon, _, text_color, label) = resolved.unwrap_or_default();
    TileLook {
        slot: i,
        area,
        rect: (x, y, w, h),
        radius: layout.radius,
        text_pad: layout.text_pad,
        fill,
        progress,
        has_key: key.is_some(),
        image,
        icon,
        label,
        text_color: text_color.as_deref().map(|s| parse_color(s, 0xffffffff)).unwrap_or(0xffffffff),
        font_size: key.and_then(|k| k.font_size).filter(|s| *s >= 6.0),
        align: key.and_then(|k| k.align.clone()).unwrap_or_else(|| "center".into()),
        font: key.and_then(|k| k.font.clone()),
        valign: key.and_then(|k| k.valign.clone()),
        knobs: if i == 13 && key.is_some() { page.knobs.iter().take(3).enumerate().map(|(k, knob)| (knob.label.clone(), v.knob_pressed & (1 << k) != 0, knob.indicator().and_then(|p| progress_look(&p, v.states)))).collect() } else { Vec::new() },
    }
}

impl Renderer {
    /// Paint a tile; nothing leaves its slot.
    pub fn paint_tile(&mut self, c: &mut Canvas, t: &TileLook) {
        c.set_clip(t.area);
        self.tile_content(c, t);
        c.reset_clip();
    }

    fn tile_content(&mut self, c: &mut Canvas, t: &TileLook) {
        let (x, y, w, h) = t.rect;
        match t.fill {
            Some((color, 255)) => c.rounded_rect(x, y, w, h, t.radius, color),
            Some((color, a)) => c.rounded_rect_alpha(x, y, w, h, t.radius, color, a),
            None => {}
        }
        if let Some(p) = &t.progress {
            draw_progress(c, x, y, w, h, t.radius, p);
        }
        if !t.has_key {
            return;
        }
        if let Some(img) = &t.image {
            if self.draw_tile_image(c, t.slot, img, x, y, w, h, t.radius) {
                return;
            }
        }
        let style = TextStyle { size: t.font_size, align: &t.align, font: t.font.as_deref() };
        let (text_color, label) = (t.text_color, t.label.as_str());
        let pad = t.text_pad;
        // content box inside the tile
        let (bx, by, bw, bh) = (x + pad, y + pad, w - 2 * pad, h - 2 * pad);
        if t.slot == 13 {
            // wide slot: label in the area above the knob strip (top by default), knob labels below
            let area_h = bh - 48;
            let half = (style.size.unwrap_or(26.0) * 0.4) as i32; // glyphs span ~0.35 of the size from the line centre
            let cy = match t.valign.as_deref() {
                Some("middle") => by + area_h / 2,
                Some("bottom") => by + area_h - half,
                _ => by + half,
            };
            self.draw_label_styled(c, label, bx, cy, bw, text_color, style);
            let kw = bw / 3;
            for (k, (knob, kp, progress)) in t.knobs.iter().enumerate() {
                let kx = bx + k as i32 * kw;
                let (cx, cy, cw, ch) = (kx + 4, by + bh - 48, kw - 8, 44);
                c.rounded_rect(cx, cy, cw, ch, 10, if *kp { 0xff4a6a8a } else { 0xff2b3b4b });
                if let Some(p) = progress {
                    draw_knob_progress(c, cx, cy, cw, ch, 10, p);
                }
                self.draw_label(c, knob, kx + kw / 2, by + bh - 26, kw - 16, 0xffd0e0f0);
            }
            return;
        }
        if let Some(icon) = t.icon.as_deref().and_then(|n| self.icon(n)) {
            if label.is_empty() {
                blit_icon(c, &icon, bx, by, bw, bh);
            } else {
                // the text strip next to the icon grows with a fixed font size
                let strip = style.size.map(|s| (s * 1.3) as i32).unwrap_or(34).min(bh - 20);
                match t.valign.as_deref() {
                    Some("top") => {
                        blit_icon(c, &icon, bx, by + strip, bw, bh - strip);
                        self.draw_label_styled(c, label, bx, by + strip / 2 - 3, bw, text_color, style);
                    }
                    Some("middle") => {
                        blit_icon(c, &icon, bx, by, bw, bh);
                        self.draw_label_styled(c, label, bx, by + bh / 2, bw, text_color, style);
                    }
                    _ => {
                        blit_icon(c, &icon, bx, by, bw, bh - strip);
                        self.draw_label_styled(c, label, bx, by + bh - strip / 2 - 3, bw, text_color, style);
                    }
                }
            }
            return;
        }
        // text only: the line height decides how far from the edge top/bottom text sits
        let half = (style.size.unwrap_or(26.0) * 0.4) as i32 + if label.contains('\n') { 14 } else { 0 };
        let cy = match t.valign.as_deref() {
            Some("top") => by + half,
            Some("bottom") => by + bh - half,
            _ => by + bh / 2,
        };
        self.draw_label_styled(c, label, bx, cy, bw, text_color, style);
    }
}

/// Centering test: every tile gets concentric inset rectangles (10 px steps,
/// alternating colours), a crosshair through its centre and its slot number.
/// A centred key window shows the same number of rings on all four sides.
pub fn draw_calibration(r: &mut Renderer, c: &mut Canvas, layout: &Layout) {
    c.clear(0xff000000);
    let colors = [0xffffffff, 0xffffd400, 0xff00e0ff, 0xffff40c0, 0xff40ff60, 0xffff8000];
    for i in 0..14usize {
        let (x, y, w, h) = layout.slot_rect(i);
        c.rect(x, y, w, h, 0xff202020);
        for (k, inset) in (0..).map(|k| k * 10).take_while(|&ins| ins * 2 < w.min(h)).enumerate() {
            let col = colors[k % colors.len()];
            let (ix, iy, iw, ih) = (x + inset, y + inset, w - 2 * inset, h - 2 * inset);
            c.rect(ix, iy, iw, 2, col);
            c.rect(ix, iy + ih - 2, iw, 2, col);
            c.rect(ix, iy, 2, ih, col);
            c.rect(ix + iw - 2, iy, 2, ih, col);
        }
        let (cx, cy) = (x + w / 2, y + h / 2);
        c.rect(x, cy, w, 1, 0xffff0000);
        c.rect(cx, y, 1, h, 0xffff0000);
        r.draw_label(c, &format!("{i}"), cx, cy - 2, 60, 0xffffffff);
    }
    r.draw_text(c, "calibration: rings every 10 px; red = tile centre", 14.0, (layout.row_y[2] - 6) as f32, 13.0, 0xff8090a0);
}

/// A knob's box: the fill keeps the box's rounded ends (a slider), the border is the tile ring.
fn draw_knob_progress(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, radius: i32, p: &ProgressLook) {
    if !p.fill {
        draw_progress(c, x, y, w, h, radius, &ProgressLook { width: p.width.min(4), ..p.clone() });
        return;
    }
    if p.frac <= 0.0 {
        return;
    }
    let (fw, fh) = ((w as f32 * p.frac).round() as i32, (h as f32 * p.frac).round() as i32);
    let (fx, fy, fw, fh) = match p.start.as_str() {
        "right" => (x + w - fw, y, fw, h),
        "top" => (x, y, w, fh),
        "bottom" => (x, y + h - fh, w, fh),
        _ => (x, y, fw, h),
    };
    c.rounded_rect_alpha(fx, fy, fw, fh, radius.min(fw / 2).min(fh / 2), p.color, 150);
}

/// Progress: ring along the tile edge (clockwise from `start`) or fill from a side.
fn draw_progress(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, radius: i32, p: &ProgressLook) {
    let (frac, color) = (p.frac, p.color);
    if frac <= 0.0 {
        return; // empty means nothing at all, not even the dim track
    }
    if p.fill {
        let a = 110u8;
        match p.start.as_str() {
            "left" => c.rect_alpha(x, y, (w as f32 * frac) as i32, h, color, a),
            "right" => {
                let fw = (w as f32 * frac) as i32;
                c.rect_alpha(x + w - fw, y, fw, h, color, a)
            }
            "top" => c.rect_alpha(x, y, w, (h as f32 * frac) as i32, color, a),
            _ => {
                let fh = (h as f32 * frac) as i32;
                c.rect_alpha(x, y + h - fh, w, fh, color, a)
            }
        }
        return;
    }
    let t = p.width.max(2);
    let inset = 2;
    let (rx, ry, rw, rh) = (x + inset, y + inset, w - 2 * inset, h - 2 * inset);
    if rw <= 0 || rh <= 0 {
        return;
    }
    let ring = Ring::new(rw, rh, (radius - inset).max(0), t);
    let start = ring.start(&p.start);
    let band_rows = ring.band_rows();
    for yy in 0..rh {
        let sides_only = yy >= band_rows && yy < rh - band_rows;
        let mut xx = 0;
        while xx < rw {
            if sides_only && xx == t + 1 {
                xx = (rw - t - 1).max(xx); // the middle of the tile is never part of the ring
            }
            let cov = ring.coverage(xx, yy);
            if cov > 0.0 {
                let lit = frac >= 1.0 || (ring.position(xx, yy) - start).rem_euclid(1.0) < frac;
                c.blend(rx + xx, ry + yy, color, (cov * if lit { 255.0 } else { 40.0 }) as u8);
            }
            xx += 1;
        }
    }
}

impl Renderer {
    /// The page's background image scaled to the canvas. Kept apart from the icon cache: tiles
    /// are repainted over it all the time and the decoded image is not needed again.
    pub fn background(&mut self, name: &str) -> Option<&[u32]> {
        if self.background.as_ref().map(|b| b.0 != name).unwrap_or(true) {
            let icon = self.icon(name)?;
            let mut c = Canvas::new();
            c.clear(PAGE_COLOR);
            blit_fullscreen(&mut c, &icon);
            self.icons.remove(name);
            self.background = Some((name.to_string(), c.px));
        }
        self.background.as_ref().map(|b| b.1.as_slice())
    }

    /// A full-screen animation frame. Never kept in the icon cache: one 960x540 frame is 2 MB and
    /// the deck has little RAM to spare; only the frame on screen stays, it is shown several times.
    fn full_frame(&mut self, name: &str) -> Option<Arc<Icon>> {
        if let Some((n, icon)) = &self.full_frame {
            if n == name {
                return Some(icon.clone());
            }
        }
        let icon = Arc::new(load_icon_file(name)?);
        self.full_frame = Some((name.to_string(), icon.clone()));
        Some(icon)
    }

    /// Full-screen image (Screen action).
    pub fn draw_screen(&mut self, c: &mut Canvas, name: &str, animated: bool) {
        c.clear(0xff000000);
        let icon = if animated { self.full_frame(name) } else { self.icon(name) };
        if let Some(icon) = icon {
            blit_fullscreen(c, &icon);
        }
    }
}

/// Load an icon from either store without caching it.
fn load_icon_file(name: &str) -> Option<Icon> {
    let dir = if name.starts_with("ram-") { crate::config::RAM_ICON_DIR } else { crate::config::ICON_DIR };
    load_png(&format!("{dir}/{name}"))
}

/// Scale an image onto the whole canvas (nearest neighbour), honouring its alpha.
fn blit_fullscreen(c: &mut Canvas, icon: &Icon) {
    let (iw, ih) = (icon.w, icon.h);
    if iw == 0 || ih == 0 {
        return;
    }
    let columns: Vec<usize> = (0..W).map(|x| x * iw / W).collect();
    for y in 0..H {
        let sy = y * ih / H;
        let row = &icon.px[sy * iw..sy * iw + iw];
        for (d, sx) in c.span(0, W as i32, y as i32).iter_mut().zip(&columns) {
            let p = row[*sx];
            let a = p >> 24;
            if a == 255 {
                *d = p;
            } else if a > 0 {
                *d = over(*d, p, a);
            }
        }
    }
}

/// Draw a full-screen effect over the page at `ms` milliseconds after its start.
/// `origin` is the centre of the pressed key (the level-up ring rolls outwards from it).
pub fn draw_effect(r: &mut Renderer, c: &mut Canvas, e: &deck_config::Effect, ms: u64, origin: (i32, i32)) {
    use deck_config::Effect;
    match e {
        // Minecraft XP green, blocky ring rolling outwards from the key, with a short flash
        Effect::LevelUp { ms: total, wave_ms, every_ms } => draw_ring(c, 0xff80ff20, *total, *wave_ms, *every_ms, origin, ms),
        Effect::Wave { color, ms: total, from, blocky } => {
            let col = color.as_deref().map(|s| parse_color(s, 0xff80ff20)).unwrap_or(0xff80ff20);
            draw_wave(c, col, *total as u64, from, *blocky, ms, false)
        }
        Effect::Frames { name, frames, delay_ms } => {
            if *frames == 0 {
                return;
            }
            let i = (ms / (*delay_ms).max(16)).min(*frames as u64 - 1);
            let file = format!("{name}-f{i:02}.png");
            if let Some(icon) = r.full_frame(&file) {
                blit_fullscreen(c, &icon);
            }
        }
    }
}

/// A glowing band sweeping across the screen. `t` in 0..1 moves the band's leading edge from
/// before the first pixel to past the last one; the band is brightest at the front and fades
/// behind. `blocky` quantises it into 20 px cells (pixel-art look); `flash` adds a brief tint.
fn draw_wave(c: &mut Canvas, color: u32, total_ms: u64, from: &str, blocky: bool, ms: u64, flash: bool) {
    let total = total_ms.max(100) as f32;
    let t = (ms as f32 / total).clamp(0.0, 1.0);
    let vertical = matches!(from, "top" | "bottom");
    let reversed = matches!(from, "right" | "bottom");
    let span = if vertical { H as f32 } else { W as f32 };
    let band = span * 0.28;
    // leading edge position along the sweep axis (0 = start edge)
    let lead = t * (span + band);
    let cell = if blocky { 20.0 } else { 1.0 };
    if flash {
        // short whole-screen tint at the start, gone by a third of the way
        let f = (1.0 - t * 3.0).max(0.0);
        if f > 0.0 {
            let a = (f * 70.0) as u8;
            c.rect_alpha(0, 0, W as i32, H as i32, color, a);
        }
    }
    let cells = (span / cell).ceil() as i32;
    for i in 0..cells {
        let pos = i as f32 * cell + cell / 2.0; // centre of this cell along the axis
        let p = if reversed { span - pos } else { pos };
        let d = lead - p; // distance behind the leading edge
        if d < 0.0 || d > band {
            continue;
        }
        // intensity: sharp bright front, then a fade
        let u = d / band;
        let mut a = if u < 0.12 { 1.0 } else { (1.0 - (u - 0.12) / 0.88).powf(1.6) };
        a *= 0.85;
        // white-hot front line
        let front = u < 0.06;
        let col = if front { 0x00ffffff } else { color & 0x00ffffff };
        let alpha = (a * 255.0) as u8;
        let (x0, y0, w, h) = if vertical { (0, (i as f32 * cell) as i32, W as i32, cell as i32) } else { ((i as f32 * cell) as i32, 0, cell as i32, H as i32) };
        if blocky {
            // stagger the fade per 20 px cell across the other axis for a pixelated edge
            let other = if vertical { W as i32 } else { H as i32 };
            let mut j = 0;
            while j < other {
                let jitter = ((i * 7 + j / 20 * 13) % 5) as f32 * 0.06;
                let aa = ((a - jitter).max(0.0) * 255.0) as u8;
                if aa > 0 {
                    let (cx, cy, cw, ch) = if vertical { (j, y0, 20.min(other - j), h) } else { (x0, j, w, 20.min(other - j)) };
                    c.rect_alpha(cx, cy, cw, ch, col, aa);
                }
                j += 20;
            }
        } else if alpha > 0 {
            c.rect_alpha(x0, y0, w, h, col, alpha);
        }
    }
}

/// Blocky rings (20 px cells) expanding from `origin` until they have left the screen: a white-hot
/// front, a bright band behind it that fades out, and a brief glow around the origin at the start.
/// One ring takes `wave_ms` to cross the screen; another starts every `every_ms` while it still
/// fits into `total_ms`.
fn draw_ring(c: &mut Canvas, color: u32, total_ms: u32, wave_ms: u32, every_ms: u32, origin: (i32, i32), ms: u64) {
    const CELL: i32 = 20;
    let wave = wave_ms.max(50) as u64;
    let every = every_ms.max(16) as u64;
    let (ox, oy) = (origin.0 as f32, origin.1 as f32);
    // farthest corner: a ring must reach it before it ends
    let max_r = [(0.0, 0.0), (W as f32, 0.0), (0.0, H as f32), (W as f32, H as f32)].iter().map(|(x, y)| ((x - ox).powi(2) + (y - oy).powi(2)).sqrt()).fold(0.0, f32::max);
    let band = 190.0;
    let leads: Vec<f32> = (0..deck_config::Effect::ring_count(total_ms, wave_ms, every_ms) as u64)
        .map(|k| k * every)
        .filter(|&start| ms >= start && ms < start + wave)
        .map(|start| (ms - start) as f32 / wave as f32 * (max_r + band))
        .collect();
    let flash = (1.0 - ms as f32 / (wave as f32 / 3.0)).max(0.0);
    if leads.is_empty() && flash <= 0.0 {
        return;
    }
    let rgb = color & 0x00ffffff;
    let mut cy = 0;
    while cy < H as i32 {
        let mut cx = 0;
        while cx < W as i32 {
            let (mx, my) = (cx as f32 + CELL as f32 / 2.0, cy as f32 + CELL as f32 / 2.0);
            let d = ((mx - ox).powi(2) + (my - oy).powi(2)).sqrt();
            let mut a = 0.0f32;
            let mut col = rgb;
            for lead in &leads {
                let behind = lead - d;
                if behind >= 0.0 && behind <= band {
                    let u = behind / band;
                    let mut ra = if u < 0.12 { 1.0 } else { (1.0 - (u - 0.12) / 0.88).powf(1.6) };
                    ra *= 0.85;
                    // pixel-art stagger on the fade
                    ra -= ((cx / CELL * 7 + cy / CELL * 13) % 5) as f32 * 0.06;
                    if ra > a {
                        a = ra;
                        col = if u < 0.06 { 0x00ffffff } else { rgb };
                    }
                }
            }
            // start flash: a soft glow around the key that fades within the first third of a ring
            if flash > 0.0 {
                let g = flash * (1.0 - (d / 320.0).min(1.0)) * 0.5;
                if g > a {
                    a = g;
                    col = rgb;
                }
            }
            if a > 0.0 {
                c.rect_alpha(cx, cy, CELL.min(W as i32 - cx), CELL.min(H as i32 - cy), col, (a.min(1.0) * 255.0) as u8);
            }
            cx += CELL;
        }
        cy += CELL;
    }
}

fn lighten(argb: u32) -> u32 {
    let ch = |v: u32| (v + (255 - v) / 2) & 0xff;
    0xff000000 | (ch((argb >> 16) & 0xff) << 16) | (ch((argb >> 8) & 0xff) << 8) | ch(argb & 0xff)
}
