//! Pixel-art marks drawn from a bitmap: crisp at any whole scale. The skull
//! is pt35's, so both devices carry the same mark.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geom::Rect;

/// pt35's menu skull, 15x16. The switcher's Home card draws it at 2x.
pub const SKULL: &[&str] = &[
    "....#######....",
    "..###########..",
    ".#############.",
    "###############",
    "###############",
    "###...###...###",
    "##....###....##",
    "##....###....##",
    "##....###....##",
    "###..##.##..###",
    "######...######",
    ".#####...#####.",
    "..###########..",
    "...#.#.#.#.#...",
    "...#.#.#.#.#...",
    "...#########...",
];

/// The same skull cut down to 11x12 for the 18 px top bar.
pub const SKULL_SMALL: &[&str] = &[
    "...#####...",
    ".#########.",
    "###########",
    "###########",
    "#...###...#",
    "#...###...#",
    "##..###..##",
    "#####.#####",
    ".####.####.",
    "..#######..",
    "..#.#.#.#..",
    "..#######..",
];

/// Width and height at `scale`.
pub fn size(bitmap: &[&str], scale: i32) -> (i32, i32) {
    let w = bitmap.first().map_or(0, |r| r.len()) as i32;
    (w * scale, bitmap.len() as i32 * scale)
}

pub fn draw(c: &mut Canvas, bitmap: &[&str], x: i32, y: i32, scale: i32, color: Color) {
    for (row, line) in bitmap.iter().enumerate() {
        for (col, cell) in line.bytes().enumerate() {
            if cell == b'#' {
                let r = Rect::new(x + col as i32 * scale, y + row as i32 * scale, scale, scale);
                c.fill(r, color);
            }
        }
    }
}

/// Draws `bitmap` centred in `r`.
pub fn draw_in(c: &mut Canvas, bitmap: &[&str], r: Rect, scale: i32, color: Color) {
    let (w, h) = size(bitmap, scale);
    let (x, y) = r.centre(w, h);
    draw(c, bitmap, x, y, scale, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_as_wide_as_the_first() {
        for b in [SKULL, SKULL_SMALL] {
            let w = b[0].len();
            assert!(b.iter().all(|r| r.len() == w));
        }
        assert_eq!(size(SKULL, 2), (30, 32));
        assert_eq!(size(SKULL_SMALL, 1), (11, 12));
    }

    #[test]
    fn ink_lands_only_where_the_bitmap_says() {
        let mut c = Canvas::new(15, 16);
        draw(&mut c, SKULL, 0, 0, 1, 0xffff);
        assert_eq!(c.get(4, 0), 0xffff, "top of the dome");
        assert_eq!(c.get(4, 6), 0, "an eye is a hole");
    }
}
