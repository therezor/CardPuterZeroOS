//! Box drawing, blocks, braille and powerline arrows, drawn as rects in the
//! cell. A font's glyphs for these rarely fit the cell exactly, and lines
//! must join with the next cell at every size.

use gh0st_app::gfx::{Canvas, Rect};

/// Weights per direction: up, right, down, left. 1 light, 2 heavy, 3 double.
const LINES: [[u8; 4]; 0x80] = {
    let mut t = [[0u8; 4]; 0x80];
    let src: &[(usize, [u8; 4])] = &[
        (0x00, [0, 1, 0, 1]),
        (0x01, [0, 2, 0, 2]),
        (0x02, [1, 0, 1, 0]),
        (0x03, [2, 0, 2, 0]),
        (0x0c, [0, 1, 1, 0]),
        (0x0d, [0, 2, 1, 0]),
        (0x0e, [0, 1, 2, 0]),
        (0x0f, [0, 2, 2, 0]),
        (0x10, [0, 0, 1, 1]),
        (0x11, [0, 0, 1, 2]),
        (0x12, [0, 0, 2, 1]),
        (0x13, [0, 0, 2, 2]),
        (0x14, [1, 1, 0, 0]),
        (0x15, [1, 2, 0, 0]),
        (0x16, [2, 1, 0, 0]),
        (0x17, [2, 2, 0, 0]),
        (0x18, [1, 0, 0, 1]),
        (0x19, [1, 0, 0, 2]),
        (0x1a, [2, 0, 0, 1]),
        (0x1b, [2, 0, 0, 2]),
        (0x1c, [1, 1, 1, 0]),
        (0x1d, [1, 2, 1, 0]),
        (0x1e, [2, 1, 1, 0]),
        (0x1f, [1, 1, 2, 0]),
        (0x20, [2, 1, 2, 0]),
        (0x21, [2, 2, 1, 0]),
        (0x22, [1, 2, 2, 0]),
        (0x23, [2, 2, 2, 0]),
        (0x24, [1, 0, 1, 1]),
        (0x25, [1, 0, 1, 2]),
        (0x26, [2, 0, 1, 1]),
        (0x27, [1, 0, 2, 1]),
        (0x28, [2, 0, 2, 1]),
        (0x29, [2, 0, 1, 2]),
        (0x2a, [1, 0, 2, 2]),
        (0x2b, [2, 0, 2, 2]),
        (0x2c, [0, 1, 1, 1]),
        (0x2d, [0, 1, 1, 2]),
        (0x2e, [0, 2, 1, 1]),
        (0x2f, [0, 2, 1, 2]),
        (0x30, [0, 1, 2, 1]),
        (0x31, [0, 1, 2, 2]),
        (0x32, [0, 2, 2, 1]),
        (0x33, [0, 2, 2, 2]),
        (0x34, [1, 1, 0, 1]),
        (0x35, [1, 1, 0, 2]),
        (0x36, [1, 2, 0, 1]),
        (0x37, [1, 2, 0, 2]),
        (0x38, [2, 1, 0, 1]),
        (0x39, [2, 1, 0, 2]),
        (0x3a, [2, 2, 0, 1]),
        (0x3b, [2, 2, 0, 2]),
        (0x3c, [1, 1, 1, 1]),
        (0x3d, [1, 1, 1, 2]),
        (0x3e, [1, 2, 1, 1]),
        (0x3f, [1, 2, 1, 2]),
        (0x40, [2, 1, 1, 1]),
        (0x41, [1, 1, 2, 1]),
        (0x42, [2, 1, 2, 1]),
        (0x43, [2, 1, 1, 2]),
        (0x44, [2, 2, 1, 1]),
        (0x45, [1, 1, 2, 2]),
        (0x46, [1, 2, 2, 1]),
        (0x47, [2, 2, 1, 2]),
        (0x48, [1, 2, 2, 2]),
        (0x49, [2, 1, 2, 2]),
        (0x4a, [2, 2, 2, 1]),
        (0x4b, [2, 2, 2, 2]),
        (0x50, [0, 3, 0, 3]),
        (0x51, [3, 0, 3, 0]),
        (0x52, [0, 3, 1, 0]),
        (0x53, [0, 1, 3, 0]),
        (0x54, [0, 3, 3, 0]),
        (0x55, [0, 0, 1, 3]),
        (0x56, [0, 0, 3, 1]),
        (0x57, [0, 0, 3, 3]),
        (0x58, [1, 3, 0, 0]),
        (0x59, [3, 1, 0, 0]),
        (0x5a, [3, 3, 0, 0]),
        (0x5b, [1, 0, 0, 3]),
        (0x5c, [3, 0, 0, 1]),
        (0x5d, [3, 0, 0, 3]),
        (0x5e, [1, 3, 1, 0]),
        (0x5f, [3, 1, 3, 0]),
        (0x60, [3, 3, 3, 0]),
        (0x61, [1, 0, 1, 3]),
        (0x62, [3, 0, 3, 1]),
        (0x63, [3, 0, 3, 3]),
        (0x64, [0, 3, 1, 3]),
        (0x65, [0, 1, 3, 1]),
        (0x66, [0, 3, 3, 3]),
        (0x67, [1, 3, 0, 3]),
        (0x68, [3, 1, 0, 1]),
        (0x69, [3, 3, 0, 3]),
        (0x6a, [1, 3, 1, 3]),
        (0x6b, [3, 1, 3, 1]),
        (0x6c, [3, 3, 3, 3]),
        // Rounded corners, drawn square.
        (0x6d, [0, 1, 1, 0]),
        (0x6e, [0, 0, 1, 1]),
        (0x6f, [1, 0, 0, 1]),
        (0x70, [1, 1, 0, 0]),
        (0x74, [0, 0, 0, 1]),
        (0x75, [1, 0, 0, 0]),
        (0x76, [0, 1, 0, 0]),
        (0x77, [0, 0, 1, 0]),
        (0x78, [0, 0, 0, 2]),
        (0x79, [2, 0, 0, 0]),
        (0x7a, [0, 2, 0, 0]),
        (0x7b, [0, 0, 2, 0]),
        (0x7c, [0, 2, 0, 1]),
        (0x7d, [1, 0, 2, 0]),
        (0x7e, [0, 1, 0, 2]),
        (0x7f, [2, 0, 1, 0]),
    ];
    let mut i = 0;
    while i < src.len() {
        t[src[i].0] = src[i].1;
        i += 1;
    }
    t
};

/// Draws `cp` into cell `r` if it is one of ours. False: use the font.
pub fn draw(c: &mut Canvas, r: Rect, cp: u32, fg: u16, bg: u16) -> bool {
    match cp {
        0x2500..=0x257f => lines(c, r, cp - 0x2500, fg),
        0x2580..=0x259f => block(c, r, cp, fg, bg),
        0x2800..=0x28ff => braille(c, r, cp - 0x2800, fg),
        0xe0b0..=0xe0b3 => arrow(c, r, cp, fg),
        _ => false,
    }
}

fn lines(c: &mut Canvas, r: Rect, i: u32, fg: u16) -> bool {
    let (cx, cy) = (r.x + (r.w - 1) / 2, r.y + (r.h - 1) / 2);
    // Dashed lines: (horizontal, heavy, dashes).
    let dash = match i {
        0x04 => Some((true, 1, 3)),
        0x05 => Some((true, 2, 3)),
        0x06 => Some((false, 1, 3)),
        0x07 => Some((false, 2, 3)),
        0x08 => Some((true, 1, 4)),
        0x09 => Some((true, 2, 4)),
        0x0a => Some((false, 1, 4)),
        0x0b => Some((false, 2, 4)),
        0x4c => Some((true, 1, 2)),
        0x4d => Some((true, 2, 2)),
        0x4e => Some((false, 1, 2)),
        0x4f => Some((false, 2, 2)),
        _ => None,
    };
    if let Some((horiz, t, n)) = dash {
        let len = if horiz { r.w } else { r.h };
        for k in 0..n {
            let a = len * k / n;
            let b = len * (k + 1) / n - 1;
            let seg = if horiz {
                Rect::new(r.x + a, cy, (b - a).max(1), t)
            } else {
                Rect::new(cx, r.y + a, t, (b - a).max(1))
            };
            c.fill(seg.intersect(&r), fg);
        }
        return true;
    }
    if (0x71..=0x73).contains(&i) {
        diagonal(c, r, i, fg);
        return true;
    }
    let [up, right, down, left] = LINES[i as usize];
    if up | right | down | left == 0 {
        return false;
    }
    // One stroke along a direction: offsets across it for its weight.
    let across = |w: u8| -> &'static [i32] {
        match w {
            1 => &[0],
            2 => &[0, 1],
            3 => &[-1, 1],
            _ => &[],
        }
    };
    // Extend each arm to cover the widest stroke across the centre.
    let reach = |a: u8, b: u8| {
        if a.max(b) == 3 {
            1
        } else {
            a.max(b) as i32 - 1
        }
    };
    let vr = reach(up, down);
    let hr = reach(left, right);
    for &d in across(left) {
        c.fill(
            Rect::new(r.x, cy + d, cx - r.x + vr.max(0) + 1, 1).intersect(&r),
            fg,
        );
    }
    for &d in across(right) {
        let x0 = cx - vr.max(0);
        c.fill(Rect::new(x0, cy + d, r.right() - x0, 1).intersect(&r), fg);
    }
    for &d in across(up) {
        c.fill(
            Rect::new(cx + d, r.y, 1, cy - r.y + hr.max(0) + 1).intersect(&r),
            fg,
        );
    }
    for &d in across(down) {
        let y0 = cy - hr.max(0);
        c.fill(Rect::new(cx + d, y0, 1, r.bottom() - y0).intersect(&r), fg);
    }
    true
}

fn diagonal(c: &mut Canvas, r: Rect, i: u32, fg: u16) {
    for y in 0..r.h {
        let x = y * (r.w - 1) / (r.h - 1).max(1);
        if i != 0x72 {
            c.fill(Rect::new(r.right() - 1 - x, r.y + y, 1, 1), fg);
        }
        if i != 0x71 {
            c.fill(Rect::new(r.x + x, r.y + y, 1, 1), fg);
        }
    }
}

fn block(c: &mut Canvas, r: Rect, cp: u32, fg: u16, bg: u16) -> bool {
    let eighth_h = |n: i32| r.h * n / 8;
    let eighth_w = |n: i32| r.w * n / 8;
    let (hw, hh) = (r.w / 2, r.h / 2);
    let quads = |c: &mut Canvas, ul: bool, ur: bool, ll: bool, lr: bool| {
        let q = [
            (ul, Rect::new(r.x, r.y, hw, hh)),
            (ur, Rect::new(r.x + hw, r.y, r.w - hw, hh)),
            (ll, Rect::new(r.x, r.y + hh, hw, r.h - hh)),
            (lr, Rect::new(r.x + hw, r.y + hh, r.w - hw, r.h - hh)),
        ];
        for (on, q) in q {
            if on {
                c.fill(q, fg);
            }
        }
    };
    match cp {
        0x2580 => c.fill(Rect::new(r.x, r.y, r.w, hh), fg),
        0x2581..=0x2588 => {
            let h = eighth_h((cp - 0x2580) as i32);
            c.fill(Rect::new(r.x, r.bottom() - h, r.w, h), fg);
        }
        0x2589..=0x258f => {
            let w = eighth_w((0x2590 - cp) as i32);
            c.fill(Rect::new(r.x, r.y, w, r.h), fg);
        }
        0x2590 => c.fill(Rect::new(r.x + hw, r.y, r.w - hw, r.h), fg),
        0x2591..=0x2593 => {
            c.fill(r, bg);
            c.fill_alpha(r, fg, [64, 128, 192][(cp - 0x2591) as usize]);
        }
        0x2594 => c.fill(Rect::new(r.x, r.y, r.w, eighth_h(1).max(1)), fg),
        0x2595 => {
            let w = eighth_w(1).max(1);
            c.fill(Rect::new(r.right() - w, r.y, w, r.h), fg);
        }
        0x2596 => quads(c, false, false, true, false),
        0x2597 => quads(c, false, false, false, true),
        0x2598 => quads(c, true, false, false, false),
        0x2599 => quads(c, true, false, true, true),
        0x259a => quads(c, true, false, false, true),
        0x259b => quads(c, true, true, true, false),
        0x259c => quads(c, true, true, false, true),
        0x259d => quads(c, false, true, false, false),
        0x259e => quads(c, false, true, true, false),
        0x259f => quads(c, false, true, true, true),
        _ => return false,
    }
    true
}

fn braille(c: &mut Canvas, r: Rect, bits: u32, fg: u16) -> bool {
    // Dot n sits at (column, row) in a 2x4 grid.
    const AT: [(i32, i32); 8] = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];
    let (dx, dy) = (r.w / 2, r.h / 4);
    let s = (dx.min(dy) / 2).max(1);
    for (n, &(col, row)) in AT.iter().enumerate() {
        if bits & (1 << n) != 0 {
            let x = r.x + col * dx + (dx - s) / 2;
            let y = r.y + row * dy + (dy - s) / 2;
            c.fill(Rect::new(x, y, s, s), fg);
        }
    }
    true
}

/// Powerline's arrows: solid E0B0 and E0B2, thin E0B1 and E0B3.
fn arrow(c: &mut Canvas, r: Rect, cp: u32, fg: u16) -> bool {
    let left = cp >= 0xe0b2;
    let solid = cp % 2 == 0;
    let mid = (r.h - 1) as f32 / 2.0;
    for y in 0..r.h {
        let d = 1.0 - ((y as f32 - mid).abs() / (mid + 0.5));
        let w = ((r.w as f32) * d).round().max(1.0) as i32;
        let (x0, x1) = if left {
            (r.right() - w, r.right())
        } else {
            (r.x, r.x + w)
        };
        if solid {
            c.fill(Rect::new(x0, r.y + y, x1 - x0, 1), fg);
        } else {
            let x = if left { x0 } else { x1 - 1 };
            c.fill(Rect::new(x, r.y + y, 1, 1), fg);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(cp: u32) -> Canvas {
        let mut c = Canvas::new(6, 12);
        assert!(draw(&mut c, Rect::new(0, 0, 6, 12), cp, 0xffff, 0));
        c
    }

    #[test]
    fn a_horizontal_line_spans_the_cell_and_a_vertical_its_height() {
        let h = cell(0x2500);
        assert!((0..6).all(|x| h.get(x, 5) == 0xffff));
        let v = cell(0x2502);
        assert!((0..12).all(|y| v.get(2, y) == 0xffff));
    }

    #[test]
    fn a_cross_meets_every_edge() {
        let c = cell(0x253c);
        assert_eq!(c.get(0, 5), 0xffff);
        assert_eq!(c.get(5, 5), 0xffff);
        assert_eq!(c.get(2, 0), 0xffff);
        assert_eq!(c.get(2, 11), 0xffff);
    }

    #[test]
    fn full_block_fills_and_half_blocks_split() {
        let f = cell(0x2588);
        assert!(f.pixels().iter().all(|&p| p == 0xffff));
        let top = cell(0x2580);
        assert_eq!(top.get(0, 0), 0xffff);
        assert_eq!(top.get(0, 11), 0);
    }

    #[test]
    fn braille_dots_land_in_their_quarter() {
        let c = cell(0x2801);
        assert!(c.pixels().contains(&0xffff));
        assert_eq!(c.get(5, 11), 0);
    }
}
