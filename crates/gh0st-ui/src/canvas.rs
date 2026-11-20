//! The back buffer. Every primitive clips to `clip`, so a redraw of one dirty
//! rect touches only the pixels inside it.

use crate::color::{Color, blend};
use crate::geom::Rect;
use crate::image::Image;

/// A `w` x `h` 4-bit coverage mask starting at nibble `off` of `bits`.
pub struct Mask4<'a> {
    pub w: i32,
    pub h: i32,
    pub bits: &'a [u8],
    pub off: usize,
}

#[derive(Clone)]
pub struct Canvas {
    w: i32,
    h: i32,
    px: Vec<Color>,
    clip: Rect,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Canvas {
            w,
            h,
            px: vec![0; (w * h) as usize],
            clip: Rect::new(0, 0, w, h),
        }
    }

    pub fn width(&self) -> i32 {
        self.w
    }

    pub fn height(&self) -> i32 {
        self.h
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    pub fn pixels(&self) -> &[Color] {
        &self.px
    }

    pub fn row(&self, y: i32) -> &[Color] {
        let s = (y * self.w) as usize;
        &self.px[s..s + self.w as usize]
    }

    /// Copies the first `w` pixels of row `y` from `src`.
    pub fn copy_row_from(&mut self, src: &Canvas, y: i32, w: i32) {
        let n = w.min(self.w).min(src.w) as usize;
        let d = (y * self.w) as usize;
        self.px[d..d + n].copy_from_slice(&src.row(y)[..n]);
    }

    /// Copies `w` pixels of row `y` from `src`, starting at `x`.
    pub fn copy_span_from(&mut self, src: &Canvas, y: i32, x: i32, w: i32) {
        let d = (y * self.w + x) as usize;
        let s = (y * src.w + x) as usize;
        self.px[d..d + w as usize].copy_from_slice(&src.px[s..s + w as usize]);
    }

    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// Narrows drawing to `r` (inside the canvas). Returns the old clip so a
    /// widget can put it back.
    pub fn set_clip(&mut self, r: Rect) -> Rect {
        let old = self.clip;
        self.clip = r.intersect(&self.bounds());
        old
    }

    pub fn get(&self, x: i32, y: i32) -> Color {
        self.px[(y * self.w + x) as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, c: Color) {
        self.px[(y * self.w + x) as usize] = c;
    }

    #[inline]
    pub fn blend_px(&mut self, x: i32, y: i32, c: Color, a: u8) {
        if self.clip.contains(x, y) {
            let i = (y * self.w + x) as usize;
            self.px[i] = blend(self.px[i], c, a);
        }
    }

    pub fn fill(&mut self, r: Rect, c: Color) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            let s = (y * self.w + r.x) as usize;
            self.px[s..s + r.w as usize].fill(c);
        }
    }

    pub fn fill_alpha(&mut self, r: Rect, c: Color, a: u8) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            let s = (y * self.w + r.x) as usize;
            for p in &mut self.px[s..s + r.w as usize] {
                *p = blend(*p, c, a);
            }
        }
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, c: Color) {
        self.fill(Rect::new(x0, y, x1 - x0, 1), c);
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, c: Color) {
        self.fill(Rect::new(x, y0, 1, y1 - y0), c);
    }

    /// Filled rectangle with anti-aliased round corners.
    pub fn round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        let radius = radius.min(r.w / 2).min(r.h / 2).max(0);
        if radius == 0 {
            return self.fill(r, c);
        }
        self.fill(Rect::new(r.x, r.y + radius, r.w, r.h - 2 * radius), c);
        self.fill(Rect::new(r.x + radius, r.y, r.w - 2 * radius, radius), c);
        self.fill(
            Rect::new(r.x + radius, r.bottom() - radius, r.w - 2 * radius, radius),
            c,
        );
        self.corners(r, radius, |d, rf| (rf + 0.5 - d).clamp(0.0, 1.0), c);
    }

    /// One-pixel outline with anti-aliased round corners.
    pub fn stroke_round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        let radius = radius.min(r.w / 2).min(r.h / 2).max(0);
        self.hline(r.x + radius, r.right() - radius, r.y, c);
        self.hline(r.x + radius, r.right() - radius, r.bottom() - 1, c);
        self.vline(r.x, r.y + radius, r.bottom() - radius, c);
        self.vline(r.right() - 1, r.y + radius, r.bottom() - radius, c);
        if radius > 0 {
            self.corners(
                r,
                radius,
                |d, rf| {
                    let outer = (rf + 0.5 - d).clamp(0.0, 1.0);
                    let inner = (rf - 0.5 - d).clamp(0.0, 1.0);
                    outer - inner
                },
                c,
            );
        }
    }

    /// Runs `cov` over the four corner squares. `cov` maps a pixel's distance
    /// from the corner's circle centre to its coverage.
    fn corners(&mut self, r: Rect, radius: i32, cov: impl Fn(f32, f32) -> f32, c: Color) {
        let rf = radius as f32;
        for dy in 0..radius {
            for dx in 0..radius {
                let fx = rf - (dx as f32 + 0.5);
                let fy = rf - (dy as f32 + 0.5);
                let d = (fx * fx + fy * fy).sqrt();
                let a = (cov(d, rf - 0.5) * 255.0) as u8;
                if a == 0 {
                    continue;
                }
                self.blend_px(r.x + dx, r.y + dy, c, a);
                self.blend_px(r.right() - 1 - dx, r.y + dy, c, a);
                self.blend_px(r.x + dx, r.bottom() - 1 - dy, c, a);
                self.blend_px(r.right() - 1 - dx, r.bottom() - 1 - dy, c, a);
            }
        }
    }

    /// Anti-aliased filled circle centred on (cx, cy).
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, c: Color) {
        let x0 = (cx - radius - 1.0).floor() as i32;
        let y0 = (cy - radius - 1.0).floor() as i32;
        let x1 = (cx + radius + 1.0).ceil() as i32;
        let y1 = (cy + radius + 1.0).ceil() as i32;
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let a = ((radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0) * 255.0) as u8;
                if a > 0 {
                    self.blend_px(x, y, c, a);
                }
            }
        }
    }

    /// Paints `c` through an 8-bit alpha mask.
    pub fn mask8(&mut self, x: i32, y: i32, w: i32, h: i32, data: &[u8], c: Color) {
        let r = Rect::new(x, y, w, h).intersect(&self.clip);
        for py in r.y..r.bottom() {
            let src = ((py - y) * w) as usize;
            let dst = (py * self.w) as usize;
            for px in r.x..r.right() {
                let a = data[src + (px - x) as usize];
                if a != 0 {
                    let p = &mut self.px[dst + px as usize];
                    *p = blend(*p, c, a);
                }
            }
        }
    }

    /// Paints `c` through a 4-bit mask packed two pixels per byte, high nibble
    /// first.
    pub fn mask4(&mut self, x: i32, y: i32, m: &Mask4, c: Color) {
        let r = Rect::new(x, y, m.w, m.h).intersect(&self.clip);
        for py in r.y..r.bottom() {
            let row = m.off + ((py - y) * m.w) as usize;
            let dst = (py * self.w) as usize;
            for px in r.x..r.right() {
                let n = row + (px - x) as usize;
                let v = (m.bits[n >> 1] >> if n & 1 == 0 { 4 } else { 0 }) & 0x0f;
                if v != 0 {
                    let p = &mut self.px[dst + px as usize];
                    *p = blend(*p, c, v * 17);
                }
            }
        }
    }

    pub fn image(&mut self, x: i32, y: i32, img: &Image) {
        let r = Rect::new(x, y, img.w, img.h).intersect(&self.clip);
        for py in r.y..r.bottom() {
            let src = ((py - y) * img.w) as usize;
            let dst = (py * self.w) as usize;
            for px in r.x..r.right() {
                let i = src + (px - x) as usize;
                let a = img.alpha[i];
                if a != 0 {
                    let p = &mut self.px[dst + px as usize];
                    *p = blend(*p, img.px[i], a);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_stays_inside_the_clip() {
        let mut c = Canvas::new(10, 10);
        c.set_clip(Rect::new(2, 2, 3, 3));
        c.fill(Rect::new(0, 0, 10, 10), 0xffff);
        let lit = c.pixels().iter().filter(|&&p| p == 0xffff).count();
        assert_eq!(lit, 9);
        assert_eq!(c.get(1, 1), 0);
        assert_eq!(c.get(2, 2), 0xffff);
    }

    #[test]
    fn round_rect_leaves_the_corner_pixel_mostly_empty() {
        let mut c = Canvas::new(20, 20);
        c.round_rect(Rect::new(0, 0, 20, 20), 6, 0xffff);
        assert_eq!(c.get(0, 0), 0);
        assert_eq!(c.get(10, 10), 0xffff);
        assert_eq!(c.get(10, 0), 0xffff);
    }
}
