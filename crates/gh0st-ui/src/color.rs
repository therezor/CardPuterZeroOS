//! RGB565, the panel's own format. Drawing in it means a flush is a copy,
//! with no conversion pass over 54,400 pixels.

/// One RGB565 pixel, red in the top five bits.
pub type Color = u16;

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// `#rrggbb` or `rrggbb`. Anything else is `None`.
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Compile-time `#rrggbb`. Panics at build time on a bad literal.
pub const fn hex(s: &str) -> Color {
    let b = s.as_bytes();
    assert!(b.len() == 7 && b[0] == b'#');
    rgb(
        nib(b[1]) << 4 | nib(b[2]),
        nib(b[3]) << 4 | nib(b[4]),
        nib(b[5]) << 4 | nib(b[6]),
    )
}

const fn nib(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("bad hex digit"),
    }
}

/// Expand to 8-bit channels by bit replication, so white stays 255.
pub const fn to_rgb8(c: Color) -> [u8; 3] {
    let r = (c >> 11) as u8 & 0x1f;
    let g = (c >> 5) as u8 & 0x3f;
    let b = c as u8 & 0x1f;
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
}

/// `src` over `dst` with alpha 0-255. Uses the classic 565 spread so the three
/// channels blend in one multiply.
#[inline]
pub fn blend(dst: Color, src: Color, alpha: u8) -> Color {
    if alpha == 0 {
        return dst;
    }
    if alpha == 255 {
        return src;
    }
    let a = (alpha as u32 + 4) >> 3;
    let s = (src as u32 | (src as u32) << 16) & 0x07e0_f81f;
    let d = (dst as u32 | (dst as u32) << 16) & 0x07e0_f81f;
    let r = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(a) >> 5) & 0x07e0_f81f;
    (r | r >> 16) as Color
}

/// A tint of `c` at `alpha` over `base`, as a solid colour.
pub fn mix(base: Color, c: Color, alpha: u8) -> Color {
    blend(base, c, alpha)
}

/// WCAG relative luminance, 0.0-1.0.
pub fn luminance(c: Color) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = to_rgb8(c);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black_survive_the_round_trip() {
        assert_eq!(to_rgb8(rgb(255, 255, 255)), [255, 255, 255]);
        assert_eq!(to_rgb8(rgb(0, 0, 0)), [0, 0, 0]);
        assert_eq!(hex("#ffffff"), 0xffff);
        assert_eq!(parse_hex("3ddc97"), Some(hex("#3ddc97")));
    }

    #[test]
    fn blend_ends_are_exact_and_the_middle_is_between() {
        let (a, b) = (rgb(0, 0, 0), rgb(255, 255, 255));
        assert_eq!(blend(a, b, 0), a);
        assert_eq!(blend(a, b, 255), b);
        let [r, g, bl] = to_rgb8(blend(a, b, 128));
        for v in [r, g, bl] {
            assert!((120..=136).contains(&v), "{v}");
        }
    }
}
