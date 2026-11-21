//! Built-in symbols: Lucide outlines rasterized by `build.rs` into 8-bit alpha
//! masks at five sizes. They take any colour at draw time.

use crate::canvas::Canvas;
use crate::color::Color;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSize {
    /// A top-bar slot.
    Tiny = 12,
    Small = 14,
    /// The skull in the top bar.
    Bar = 16,
    Medium = 20,
    Large = 36,
}

#[derive(Clone, Copy)]
pub struct Mask {
    pub size: i32,
    data: &'static [u8],
}

pub fn icon(name: &str, size: IconSize) -> Option<Mask> {
    let (off, s) = icon_slot(name, size as u16)?;
    let n = s as usize * s as usize;
    Some(Mask {
        size: s as i32,
        data: &ICON_BITS[off..off + n],
    })
}

impl Mask {
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, color: Color) {
        c.mask8(x, y, self.size, self.size, self.data, color);
    }
}

/// Draws a named icon, or nothing if it is not baked. Returns its size.
pub fn draw(c: &mut Canvas, name: &str, size: IconSize, x: i32, y: i32, color: Color) -> i32 {
    match icon(name, size) {
        Some(m) => {
            m.draw(c, x, y, color);
            m.size
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_baked_at_every_size() {
        for name in ICON_NAMES {
            for s in [
                IconSize::Tiny,
                IconSize::Small,
                IconSize::Bar,
                IconSize::Medium,
                IconSize::Large,
            ] {
                let m = icon(name, s).unwrap();
                assert!(m.data.iter().any(|&a| a > 0), "{name} at {s:?} is blank");
            }
        }
    }
}
