//! The offscreen 960x540 ARGB canvas and its drawing primitives. No firmware dependencies, so it
//! can be checked on the PC: `rustc --edition 2021 --test src/canvas.rs`.

pub const W: usize = 960;
pub const H: usize = 540;

/// x, y, w, h.
pub type Rect = (i32, i32, i32, i32);

pub fn intersects(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

pub struct Canvas {
    pub px: Vec<u32>,
    /// Drawing is limited to this box: x0, y0, x1, y1 (exclusive).
    clip: (i32, i32, i32, i32),
}

/// Pixels of row `yy` of a `w` x `h` rounded rectangle, as a range of columns.
fn rounded_span(w: i32, h: i32, r: i32, yy: i32) -> (i32, i32) {
    if yy >= r && yy < h - r {
        return (0, w);
    }
    let dy = if yy < r { r - yy } else { yy - (h - 1 - r) };
    let s = ((r * r - dy * dy) as f32).sqrt() as i32;
    (r - s, w - r + s)
}

/// `argb` at opacity `a` (1..255) over the pixel `d`.
#[inline]
pub fn over(d: u32, argb: u32, a: u32) -> u32 {
    let mix = |s: u32, d: u32| ((s * a + d * (255 - a)) / 255) & 0xff;
    0xff000000 | (mix((argb >> 16) & 0xff, (d >> 16) & 0xff) << 16) | (mix((argb >> 8) & 0xff, (d >> 8) & 0xff) << 8) | mix(argb & 0xff, d & 0xff)
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas { px: vec![0xff000000; W * H], clip: (0, 0, W as i32, H as i32) }
    }
    pub fn clear(&mut self, argb: u32) {
        self.px.fill(argb);
    }
    pub fn set_clip(&mut self, r: Rect) {
        self.clip = (r.0.max(0), r.1.max(0), (r.0 + r.2).min(W as i32), (r.1 + r.3).min(H as i32));
    }
    pub fn reset_clip(&mut self) {
        self.clip = (0, 0, W as i32, H as i32);
    }
    #[inline]
    fn inside(&self, x: i32, y: i32) -> bool {
        x >= self.clip.0 && y >= self.clip.1 && x < self.clip.2 && y < self.clip.3
    }
    /// The clipped part of a horizontal run of pixels.
    #[inline]
    pub fn span(&mut self, x0: i32, x1: i32, y: i32) -> &mut [u32] {
        let (x0, x1) = (x0.max(self.clip.0), x1.min(self.clip.2));
        if y < self.clip.1 || y >= self.clip.3 || x0 >= x1 {
            return &mut [];
        }
        let row = y as usize * W;
        &mut self.px[row + x0 as usize..row + x1 as usize]
    }
    /// Alpha blend a coverage value (0..255) of colour onto the canvas.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, argb: u32, cov: u8) {
        if cov == 0 || !self.inside(x, y) {
            return;
        }
        let i = y as usize * W + x as usize;
        self.px[i] = over(self.px[i], argb, cov as u32);
    }
    /// `blend` of one colour over a run of pixels.
    fn blend_span(&mut self, x0: i32, x1: i32, y: i32, argb: u32, cov: u8) {
        let a = cov as u32;
        let ia = 255 - a;
        let (sr, sg, sb) = (((argb >> 16) & 0xff) * a, ((argb >> 8) & 0xff) * a, (argb & 0xff) * a);
        for p in self.span(x0, x1, y) {
            let d = *p;
            let r = (sr + ((d >> 16) & 0xff) * ia) / 255;
            let g = (sg + ((d >> 8) & 0xff) * ia) / 255;
            let b = (sb + (d & 0xff) * ia) / 255;
            *p = 0xff000000 | (r << 16) | (g << 8) | b;
        }
    }
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, argb: u32) {
        for yy in y..y + h {
            self.span(x, x + w, yy).fill(argb);
        }
    }
    pub fn rect_alpha(&mut self, x: i32, y: i32, w: i32, h: i32, argb: u32, a: u8) {
        if a == 0 {
            return;
        }
        for yy in y..y + h {
            self.blend_span(x, x + w, yy, argb, a);
        }
    }
    pub fn rounded_rect_alpha(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32, a: u8) {
        let r = r.min(w / 2).min(h / 2).max(0);
        for yy in 0..h {
            let (x0, x1) = rounded_span(w, h, r, yy);
            self.blend_span(x + x0, x + x1, y + yy, argb, a);
        }
    }
    pub fn rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32) {
        let r = r.min(w / 2).min(h / 2).max(0);
        for yy in 0..h {
            let (x0, x1) = rounded_span(w, h, r, yy);
            self.span(x + x0, x + x1, y + yy).fill(argb);
        }
    }
    /// Copy a box out of a whole-screen pixel buffer.
    pub fn copy_rect(&mut self, from: &[u32], r: Rect) {
        for yy in r.1.max(0)..(r.1 + r.3).min(H as i32) {
            let (x0, x1) = (r.0.max(0), (r.0 + r.2).min(W as i32));
            if x0 < x1 {
                let row = yy as usize * W;
                self.span(x0, x1, yy).copy_from_slice(&from[row + x0 as usize..row + x1 as usize]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-pixel rule `rounded_rect` used before it filled whole rows.
    fn inside_rounded(w: i32, h: i32, r: i32, xx: i32, yy: i32) -> bool {
        let corner = |cx: i32, cy: i32| (xx - cx) * (xx - cx) + (yy - cy) * (yy - cy) <= r * r;
        (xx >= r && xx < w - r) || (yy >= r && yy < h - r) || corner(r, r) || corner(w - 1 - r, r) || corner(r, h - 1 - r) || corner(w - 1 - r, h - 1 - r)
    }

    #[test]
    fn rounded_rows_match_the_pixel_rule() {
        for (w, h, r) in [(160, 150, 14), (148, 138, 14), (356, 150, 14), (102, 44, 10), (31, 40, 15), (20, 20, 0)] {
            for yy in 0..h {
                let (x0, x1) = rounded_span(w, h, r, yy);
                for xx in 0..w {
                    assert_eq!(xx >= x0 && xx < x1, inside_rounded(w, h, r, xx, yy), "{w}x{h} r{r} at ({xx},{yy})");
                }
            }
        }
    }

    #[test]
    fn blend_span_matches_blend() {
        for a in [1u8, 90, 110, 200, 254, 255] {
            let (mut one, mut run) = (Canvas::new(), Canvas::new());
            for (i, p) in one.px.iter_mut().enumerate() {
                *p = 0xff000000 | (i as u32).wrapping_mul(2654435761) >> 8;
            }
            run.px.copy_from_slice(&one.px);
            for x in 0..W as i32 {
                one.blend(x, 7, 0x80ff20, a);
            }
            run.blend_span(0, W as i32, 7, 0x80ff20, a);
            assert!(one.px == run.px);
        }
    }

    #[test]
    fn clip_limits_everything() {
        let mut c = Canvas::new();
        c.set_clip((10, 10, 20, 20));
        c.rect(0, 0, 960, 540, 0xffffffff);
        c.rect_alpha(0, 0, 960, 540, 0xff0000, 255);
        c.rounded_rect(0, 0, 100, 100, 5, 0xff00ff00);
        c.blend(5, 5, 0xffffff, 255);
        let touched = c.px.iter().filter(|p| **p != 0xff000000).count();
        assert_eq!(touched, 400);
        assert_eq!(c.px[5 * W + 5], 0xff000000);
    }

    #[test]
    fn rects_off_the_canvas_are_harmless() {
        let mut c = Canvas::new();
        c.rect(-5, -5, 2, 2, 0xffffffff);
        c.rect(955, 535, 50, 50, 0xffffffff);
        c.rounded_rect(950, 530, 40, 40, 6, 0xffffffff);
        c.rect_alpha(-20, 530, 40, 40, 0xffffff, 40);
        let from = vec![1u32; W * H];
        c.copy_rect(&from, (-3, -3, 10, 10));
        c.copy_rect(&from, (955, 535, 50, 50));
        assert!(!intersects((0, 0, 10, 10), (10, 0, 10, 10)));
        assert!(intersects((0, 0, 11, 10), (10, 0, 10, 10)));
    }
}
