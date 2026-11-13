//! Geometry of the progress ring: a band between two rounded rectangles. No firmware dependencies,
//! so it can be checked on the PC: `rustc --test src/ring.rs`.

use std::f32::consts::FRAC_PI_2;

pub struct Ring {
    half_w: f32,
    half_h: f32,
    radius: f32,
    thickness: f32,
    inner_radius: f32,
    /// Path along the middle of the band: half lengths of the straight parts, corner radius.
    straight_w: f32,
    straight_h: f32,
    path_radius: f32,
    arc: f32,
    perimeter: f32,
}

/// Signed distance to a centred rounded rectangle, negative inside.
fn rounded_distance(x: f32, y: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let dx = x.abs() - (half_w - r);
    let dy = y.abs() - (half_h - r);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r
}

impl Ring {
    /// `w` x `h` outer size, `radius` outer corner radius, `thickness` of the band.
    pub fn new(w: i32, h: i32, radius: i32, thickness: i32) -> Ring {
        let (half_w, half_h, radius, thickness) = (w as f32 / 2.0, h as f32 / 2.0, radius as f32, thickness as f32);
        let path_radius = (radius - thickness / 2.0).max(1.0);
        let straight_w = (half_w - thickness / 2.0 - path_radius).max(0.0);
        let straight_h = (half_h - thickness / 2.0 - path_radius).max(0.0);
        let arc = path_radius * FRAC_PI_2;
        Ring {
            half_w,
            half_h,
            radius,
            thickness,
            // concentric when the band is thin; a thick band keeps a visibly round inner corner
            inner_radius: (radius - thickness).max(radius / 2.0),
            straight_w,
            straight_h,
            path_radius,
            arc,
            perimeter: 4.0 * (straight_w + straight_h + arc),
        }
    }

    /// Rows from the top where a whole line may touch the band; the rest only touch it at the sides.
    pub fn band_rows(&self) -> i32 {
        (self.thickness + self.inner_radius).ceil() as i32 + 1
    }

    /// How much of pixel (`x`, `y`) lies in the band, 0.0..=1.0 (edges are antialiased).
    pub fn coverage(&self, x: i32, y: i32) -> f32 {
        let (qx, qy) = (x as f32 + 0.5 - self.half_w, y as f32 + 0.5 - self.half_h);
        let outer = rounded_distance(qx, qy, self.half_w, self.half_h, self.radius);
        let inner = rounded_distance(qx, qy, self.half_w - self.thickness, self.half_h - self.thickness, self.inner_radius);
        (0.5 - outer).clamp(0.0, 1.0) * (0.5 + inner).clamp(0.0, 1.0)
    }

    /// Where pixel (`x`, `y`) lies along the band, clockwise from top centre, 0.0..1.0.
    pub fn position(&self, x: i32, y: i32) -> f32 {
        let (qx, qy) = (x as f32 + 0.5 - self.half_w, y as f32 + 0.5 - self.half_h);
        let (sw, sh, rc, arc) = (self.straight_w, self.straight_h, self.path_radius, self.arc);
        let s = if qx.abs() <= sw {
            if qy < 0.0 {
                qx
            } else {
                sw + 2.0 * arc + 2.0 * sh + sw - qx
            }
        } else if qy.abs() <= sh {
            if qx > 0.0 {
                sw + arc + sh + qy
            } else {
                3.0 * sw + 3.0 * arc + 2.0 * sh + sh - qy
            }
        } else {
            let (dx, dy) = (qx.abs() - sw, qy.abs() - sh);
            match (qx > 0.0, qy > 0.0) {
                (true, false) => sw + rc * dx.atan2(dy),
                (true, true) => sw + arc + 2.0 * sh + rc * dy.atan2(dx),
                (false, true) => 3.0 * sw + 2.0 * arc + 2.0 * sh + rc * dx.atan2(dy),
                (false, false) => 3.0 * sw + 3.0 * arc + 4.0 * sh + rc * dy.atan2(dx),
            }
        };
        (s / self.perimeter).rem_euclid(1.0)
    }

    /// `position` of the middle of a side ("top", "right", "bottom", "left").
    pub fn start(&self, side: &str) -> f32 {
        let right = (self.straight_w + self.arc + self.straight_h) / self.perimeter;
        match side {
            "right" => right,
            "bottom" => 0.5,
            "left" => 1.0 - right,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile() -> Ring {
        Ring::new(156, 146, 12, 10)
    }

    #[test]
    fn band_follows_the_rounded_corners() {
        let r = tile();
        assert_eq!(r.coverage(0, 0), 0.0, "outer corner is cut off");
        assert_eq!(r.coverage(78, 0), 1.0, "top edge");
        assert_eq!(r.coverage(0, 73), 1.0, "left edge");
        assert_eq!(r.coverage(78, 73), 0.0, "centre stays free");
        assert_eq!(r.coverage(78, 10), 0.0, "just inside the band");
        // the corner of a square inner edge would be at (10, 10): rounded, it is still band
        assert_eq!(r.coverage(10, 10), 1.0, "inner corner is rounded");
    }

    #[test]
    fn inner_corner_stays_round_on_a_thick_band() {
        let r = Ring::new(156, 146, 12, 14);
        assert_eq!(r.coverage(14, 14), 1.0);
    }

    #[test]
    fn position_runs_clockwise_around_the_whole_band() {
        let r = tile();
        let (w, h) = (156, 146);
        let mut points: Vec<(i32, i32)> = Vec::new();
        points.extend((78..w - 8).map(|x| (x, 4)));
        points.extend((8..h - 8).map(|y| (w - 5, y)));
        points.extend((8..w - 8).rev().map(|x| (x, h - 5)));
        points.extend((8..h - 8).rev().map(|y| (4, y)));
        points.extend((8..78).map(|x| (x, 4)));
        let mut last = -1.0;
        for (x, y) in points {
            let p = r.position(x, y);
            if p > 0.5 || last < 0.9 {
                assert!(p >= last || (last > 0.99 && p < 0.01), "({x},{y}): {p} after {last}");
            }
            last = p;
        }
        assert!(r.position(78, 4) < 0.01);
    }

    #[test]
    fn starts_are_the_middle_of_their_side() {
        let r = tile();
        for (side, x, y) in [("right", 151, 73), ("bottom", 78, 141), ("left", 4, 73)] {
            assert!((r.position(x, y) - r.start(side)).abs() < 0.01, "{side}");
        }
    }
}
