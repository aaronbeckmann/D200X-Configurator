//! Turns frames into pixels and works out what changed. The page stays composed on `base` between
//! frames and only tiles whose look changed are repainted; effects are drawn over a copy of it.

use crate::canvas::{intersects, Canvas, Rect, H, W};
use crate::config::Layout;
use crate::fb::Dirty;
use crate::render::{self, PageFrame, Renderer, TileLook, PAGE_COLOR};

pub enum Frame {
    /// All black: the last thing shown before a test run reboots.
    Black,
    Calibrate(Layout),
    /// Full-screen image; `animated` frames are never cached.
    Screen { name: String, animated: bool },
    Page(PageFrame),
}

/// Something in the image stores changed: what was drawn from it is out of date.
pub enum Stale {
    Icon(String),
    AllIcons,
    Fonts,
}

/// What `base` shows at the moment.
struct Shown {
    background: Option<String>,
    tiles: Vec<TileLook>,
    status: (String, u32),
    /// Tiles to repaint although their look did not change (bitmask by slot).
    stale: u16,
}

enum Base {
    Unknown,
    Page(Shown),
    Screen(String),
}

pub struct Compositor {
    r: Renderer,
    base: Canvas,
    /// The page with effects on it, while any run.
    scene: Option<Canvas>,
    shown: Base,
}

impl Compositor {
    pub fn new(r: Renderer) -> Compositor {
        Compositor { r, base: Canvas::new(), scene: None, shown: Base::Unknown }
    }

    pub fn stale(&mut self, s: Stale) {
        match s {
            Stale::Icon(name) => {
                self.r.forget_icon(&name);
                match &mut self.shown {
                    Base::Page(p) if p.background.as_deref() == Some(&name) => self.shown = Base::Unknown,
                    Base::Page(p) => p.stale |= p.tiles.iter().filter(|t| t.uses(&name)).fold(0, |m, t| m | 1 << t.slot),
                    Base::Screen(n) if *n == name => self.shown = Base::Unknown,
                    _ => {}
                }
            }
            Stale::AllIcons => {
                self.r.clear_icons();
                self.shown = Base::Unknown;
            }
            Stale::Fonts => {
                self.r.forget_fonts();
                self.shown = Base::Unknown;
            }
        }
    }

    /// Draw a frame: the finished canvas and what changed since the last one.
    pub fn compose(&mut self, f: &Frame) -> (&Canvas, Dirty) {
        match f {
            Frame::Black => {
                self.base.clear(0xff000000);
                self.shown = Base::Unknown;
                (&self.base, Dirty::Full)
            }
            Frame::Calibrate(layout) => {
                render::draw_calibration(&mut self.r, &mut self.base, layout);
                self.shown = Base::Unknown;
                (&self.base, Dirty::Full)
            }
            Frame::Screen { name, animated } => {
                let same = matches!(&self.shown, Base::Screen(n) if n == name);
                if same && self.scene.is_none() {
                    return (&self.base, Dirty::None);
                }
                self.scene = None;
                self.r.draw_screen(&mut self.base, name, *animated);
                self.shown = Base::Screen(name.clone());
                (&self.base, Dirty::Full)
            }
            Frame::Page(p) => self.compose_page(p),
        }
    }

    fn compose_page(&mut self, p: &PageFrame) -> (&Canvas, Dirty) {
        let mut dirty = self.update_base(p);
        if p.effects.is_empty() {
            if self.scene.take().is_some() {
                dirty = Dirty::Full; // the effect's pixels are still on screen
            }
            return (&self.base, dirty);
        }
        let scene = self.scene.get_or_insert_with(Canvas::new);
        scene.px.copy_from_slice(&self.base.px);
        for e in &p.effects {
            render::draw_effect(&mut self.r, scene, &e.effect, e.ms, e.origin);
            // the key that fired the effect stays in front of it, drawn in full
            if let Some(t) = e.slot.and_then(|s| p.tiles.get(s)) {
                self.r.paint_tile(scene, t);
            }
        }
        (&*scene, Dirty::Full)
    }

    /// Bring `base` up to date with the page, as cheaply as the differences allow.
    fn update_base(&mut self, p: &PageFrame) -> Dirty {
        let status = (p.status.clone(), p.status_color);
        let Base::Page(old) = &self.shown else { return self.repaint_page(p, status) };
        if old.background != p.background || old.tiles.len() != p.tiles.len() {
            return self.repaint_page(p, status);
        }
        let changed: Vec<usize> = (0..p.tiles.len()).filter(|i| old.stale & (1 << i) != 0 || old.tiles[*i] != p.tiles[*i]).collect();
        let status_changed = old.status != status;
        if changed.is_empty() && !status_changed {
            return Dirty::None;
        }
        // Tiles are repainted on their own only when nothing else shares their pixels: the order
        // of painting decides what is on top in a layout with overlaps.
        let moved = changed.iter().any(|i| old.tiles[*i].area != p.tiles[*i].area);
        let areas: Vec<Rect> = p.tiles.iter().map(|t| t.area).chain([p.status_area]).collect();
        let overlaps = areas.iter().enumerate().any(|(i, a)| areas[i + 1..].iter().any(|b| intersects(*a, *b)));
        if moved || overlaps {
            return self.repaint_page(p, status);
        }
        let mut rects = Vec::with_capacity(changed.len() + 1);
        for i in changed {
            let t = &p.tiles[i];
            self.restore(p.background.as_deref(), t.area);
            self.r.paint_tile(&mut self.base, t);
            rects.push(t.area);
        }
        if status_changed {
            self.restore(p.background.as_deref(), p.status_area);
            self.paint_status(p);
            rects.push(p.status_area);
        }
        if let Base::Page(old) = &mut self.shown {
            old.tiles.clone_from(&p.tiles);
            old.status = status;
            old.stale = 0;
        }
        Dirty::Rects(rects)
    }

    fn repaint_page(&mut self, p: &PageFrame, status: (String, u32)) -> Dirty {
        let full = (0, 0, W as i32, H as i32);
        self.restore(p.background.as_deref(), full);
        for t in &p.tiles {
            self.r.paint_tile(&mut self.base, t);
        }
        self.paint_status(p);
        self.shown = Base::Page(Shown { background: p.background.clone(), tiles: p.tiles.clone(), status, stale: 0 });
        Dirty::Full
    }

    /// The page without its tiles, in one box of `base`. The background is an image name or a
    /// colour (`#rrggbb`).
    fn restore(&mut self, background: Option<&str>, r: Rect) {
        if let Some(c) = background.filter(|b| b.starts_with('#')).and_then(deck_config::parse_color_value) {
            self.base.rect(r.0, r.1, r.2, r.3, c);
            return;
        }
        match background.and_then(|b| self.r.background(b)) {
            Some(px) => self.base.copy_rect(px, r),
            None => self.base.rect(r.0, r.1, r.2, r.3, PAGE_COLOR),
        }
    }

    // status line in the gap below the middle row
    fn paint_status(&mut self, p: &PageFrame) {
        self.base.set_clip(p.status_area);
        self.r.draw_text(&mut self.base, &p.status, 14.0, (p.status_area.1 + 14) as f32, 13.0, p.status_color);
        self.base.reset_clip();
    }
}
