//! Text. The faces are baked by `build.rs` into 4-bit coverage tables. A glyph
//! the tables lack (CJK, mostly) is drawn from an outline font loaded the first
//! time one appears, with a small bounded cache.

use crate::canvas::{Canvas, Mask4};
use crate::color::Color;

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub cp: u32,
    pub w: u8,
    pub h: u8,
    pub left: i8,
    pub top: i8,
    /// Advance in 1/64 px.
    pub adv: u16,
    /// Nibble offset into `BakedFont::bits`.
    pub off: u32,
}

const fn g(cp: u32, w: u32, h: u32, left: i32, top: i32, adv: u32, off: u32) -> Glyph {
    Glyph {
        cp,
        w: w as u8,
        h: h as u8,
        left: left as i8,
        top: top as i8,
        adv: adv as u16,
        off,
    }
}

pub struct BakedFont {
    pub ascent: i32,
    pub descent: i32,
    pub line: i32,
    /// The em the face was rasterized at, after x-height snapping.
    pub em: f32,
    pub glyphs: &'static [Glyph],
    pub bits: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/fonts.rs"));

pub type Font = &'static BakedFont;

const ELLIPSIS: char = '\u{2026}';

impl BakedFont {
    fn find(&self, cp: u32) -> Option<&Glyph> {
        // ASCII is baked first and complete, so it indexes directly.
        if (0x20..0x7f).contains(&cp) {
            let g = &self.glyphs[(cp - 0x20) as usize];
            if g.cp == cp {
                return Some(g);
            }
        }
        self.glyphs
            .binary_search_by_key(&cp, |g| g.cp)
            .ok()
            .map(|i| &self.glyphs[i])
    }

    /// Advance of one char in 1/64 px.
    fn advance(&self, ch: char) -> u32 {
        match self.find(ch as u32) {
            Some(g) => g.adv as u32,
            None => fallback::advance(ch, self.em).unwrap_or(self.tofu_advance()),
        }
    }

    fn tofu_advance(&self) -> u32 {
        (self.em * 0.6 * 64.0) as u32
    }

    pub fn measure(&self, s: &str) -> i32 {
        let sum: u32 = s.chars().map(|c| self.advance(c)).sum();
        ((sum + 32) >> 6) as i32
    }

    /// Height of capitals, for centring a line in a box.
    pub fn cap(&self) -> i32 {
        self.find('H' as u32)
            .map(|g| -(g.top as i32))
            .unwrap_or(self.ascent)
    }

    /// Draws `s` with its baseline at `y`. Returns the x where the text ends.
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, s: &str, color: Color) -> i32 {
        let mut pen: u32 = 0;
        for ch in s.chars() {
            let px = x + ((pen + 32) >> 6) as i32;
            pen += self.draw_char(c, px, y, ch, color);
        }
        x + ((pen + 32) >> 6) as i32
    }

    fn draw_char(&self, c: &mut Canvas, x: i32, y: i32, ch: char, color: Color) -> u32 {
        if let Some(g) = self.find(ch as u32) {
            if g.w > 0 {
                let m = Mask4 {
                    w: g.w as i32,
                    h: g.h as i32,
                    bits: self.bits,
                    off: g.off as usize,
                };
                c.mask4(x + g.left as i32, y + g.top as i32, &m, color);
            }
            return g.adv as u32;
        }
        if let Some(adv) = fallback::draw(c, x, y, ch, self.em, color) {
            return adv;
        }
        let w = ((self.tofu_advance() >> 6) as i32 - 2).max(3);
        let h = self.cap();
        c.stroke_round_rect(crate::geom::Rect::new(x + 1, y - h, w, h), 0, color);
        self.tofu_advance()
    }

    /// Draws `s` cut to `max` px wide, ending in an ellipsis when cut.
    pub fn draw_elided(
        &self,
        c: &mut Canvas,
        x: i32,
        y: i32,
        s: &str,
        max: i32,
        color: Color,
    ) -> i32 {
        if self.measure(s) <= max {
            return self.draw(c, x, y, s, color);
        }
        let limit = ((max.max(0) as u32) << 6).saturating_sub(self.advance(ELLIPSIS));
        let mut pen = 0;
        let mut end = 0;
        for (i, ch) in s.char_indices() {
            let a = self.advance(ch);
            if pen + a > limit {
                break;
            }
            pen += a;
            end = i + ch.len_utf8();
        }
        let x = self.draw(c, x, y, s[..end].trim_end(), color);
        self.draw(c, x, y, "\u{2026}", color)
    }
}

#[cfg(feature = "cjk")]
mod fallback {
    use super::*;
    use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont, point};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    const CACHE_MAX: usize = 256;
    const CANDIDATES: &[&str] = &[
        "/usr/share/APPLaunch/share/font/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];

    struct Outline {
        w: i32,
        h: i32,
        left: i32,
        top: i32,
        adv: u32,
        alpha: Vec<u8>,
    }

    #[derive(Default)]
    struct State {
        tried: bool,
        font: Option<FontRef<'static>>,
        cache: HashMap<(char, u32), Rc<Outline>>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::default();
    }

    fn glyph(ch: char, em: f32) -> Option<Rc<Outline>> {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let key = (ch, (em * 10.0) as u32);
            if let Some(o) = s.cache.get(&key) {
                return Some(o.clone());
            }
            if !s.tried {
                s.tried = true;
                s.font = load();
            }
            let font = s.font.as_ref()?;
            let id = font.glyph_id(ch);
            if id.0 == 0 {
                return None;
            }
            let upem = font.units_per_em().unwrap_or(1000.0);
            let scale = PxScale::from(em * font.height_unscaled() / upem);
            let adv = (font.as_scaled(scale).h_advance(id) * 64.0).round() as u32;
            let o = match font.outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0))) {
                Some(ol) => {
                    let b = ol.px_bounds();
                    let (w, h) = (b.width() as i32, b.height() as i32);
                    let mut alpha = vec![0u8; (w * h) as usize];
                    ol.draw(|x, y, c| {
                        alpha[(y as i32 * w + x as i32) as usize] = (c.min(1.0) * 255.0) as u8
                    });
                    Outline {
                        w,
                        h,
                        left: b.min.x as i32,
                        top: b.min.y as i32,
                        adv,
                        alpha,
                    }
                }
                None => Outline {
                    w: 0,
                    h: 0,
                    left: 0,
                    top: 0,
                    adv,
                    alpha: Vec::new(),
                },
            };
            if s.cache.len() >= CACHE_MAX {
                s.cache.clear();
            }
            let o = Rc::new(o);
            s.cache.insert(key, o.clone());
            Some(o)
        })
    }

    /// Maps the font file rather than reading it: a CJK face is 19 MB, and only
    /// the pages for the glyphs in use are ever read.
    fn load() -> Option<FontRef<'static>> {
        let env = std::env::var("ZER0_CJK_FONT").ok();
        let path = env
            .iter()
            .map(String::as_str)
            .chain(CANDIDATES.iter().copied())
            .find(|p| std::path::Path::new(p).exists())?;
        let data = map(path)?;
        FontRef::try_from_slice_and_index(data, 0).ok()
    }

    fn map(path: &str) -> Option<&'static [u8]> {
        use std::os::fd::AsRawFd;
        let f = std::fs::File::open(path).ok()?;
        let len = f.metadata().ok()?.len() as usize;
        // SAFETY: a read-only private mapping of a whole regular file. It is
        // never unmapped, so the slice lives for the rest of the process.
        unsafe {
            let p = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                f.as_raw_fd(),
                0,
            );
            if p == libc::MAP_FAILED {
                return None;
            }
            Some(std::slice::from_raw_parts(p as *const u8, len))
        }
    }

    pub fn advance(ch: char, em: f32) -> Option<u32> {
        glyph(ch, em).map(|o| o.adv)
    }

    pub fn draw(c: &mut Canvas, x: i32, y: i32, ch: char, em: f32, color: Color) -> Option<u32> {
        let o = glyph(ch, em)?;
        if o.w > 0 {
            c.mask8(x + o.left, y + o.top, o.w, o.h, &o.alpha, color);
        }
        Some(o.adv)
    }
}

#[cfg(not(feature = "cjk"))]
mod fallback {
    use super::*;
    pub fn advance(_: char, _: f32) -> Option<u32> {
        None
    }
    pub fn draw(_: &mut Canvas, _: i32, _: i32, _: char, _: f32, _: Color) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_complete_and_indexes_directly() {
        for f in [&SANS_11, &SANS_13_BOLD, &MONO_10] {
            for cp in 0x20..0x7f {
                assert_eq!(f.glyphs[(cp - 0x20) as usize].cp, cp);
            }
        }
    }

    #[test]
    fn mono_cells_are_six_px_wide() {
        assert_eq!(MONO_10.measure("abcdefghij"), 60);
    }

    #[test]
    fn elided_text_fits_its_box() {
        let mut c = Canvas::new(200, 20);
        let end = SANS_11.draw_elided(&mut c, 0, 14, "A rather long application name", 60, 0xffff);
        assert!(end <= 60, "{end}");
    }

    #[test]
    fn cyrillic_is_baked() {
        assert!(SANS_13.find('Ж' as u32).is_some());
        assert!(SANS_13.find('ї' as u32).is_some());
    }
}
