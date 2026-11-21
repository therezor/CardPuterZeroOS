//! Cells to pixels. Each cell clips its own glyph, so a glyph wider than
//! its cell never spills into the next one.

use crate::vt::CursorShape;
use crate::vt::cell::{Cell, Clusters, Color, attr};
use gh0st_app::gfx::color::mix;
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::{Canvas, Rect, Theme};

/// A face and the cell it sets in.
#[derive(Clone, Copy)]
pub struct Look {
    pub font: Font,
    pub bold: Font,
    /// Cell width and height.
    pub cw: i32,
    pub ch: i32,
    /// Baseline from the cell top.
    pub base: i32,
}

impl Look {
    pub fn mono(t: &Theme) -> Look {
        let font = t.f.mono;
        let cw = font.measure("M").max(1);
        let ch = font.line.max(1);
        Look {
            font,
            bold: t.f.mono_bold,
            cw,
            ch,
            base: font.ascent + (ch - font.ascent + font.descent) / 2,
        }
    }
}

/// The 16 ANSI colours and the defaults, resolved from the theme.
#[derive(Clone, Copy)]
pub struct Colors {
    pub fg: u16,
    pub bg: u16,
    pub ansi: [u16; 16],
}

impl Colors {
    pub fn of(t: &Theme) -> Colors {
        Colors {
            fg: t.c.fg,
            bg: t.c.bg,
            ansi: t.c.ansi,
        }
    }

    fn get(&self, c: Color, default: u16) -> u16 {
        match c {
            Color::Index(i) if (i as usize) < 16 => self.ansi[i as usize],
            Color::Index(_) => default,
            Color::Rgb565(v) => v,
        }
    }

    /// Foreground and background of a cell, after inverse and dim.
    fn pair(&self, cell: &Cell) -> (u16, u16) {
        let mut fg = self.get(cell.fg(), self.fg);
        let mut bg = self.get(cell.bg(), self.bg);
        if cell.attrs() & attr::INVERSE != 0 {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.attrs() & attr::DIM != 0 {
            fg = mix(bg, fg, 140);
        }
        (fg, bg)
    }
}

pub struct Cursor {
    pub col: usize,
    pub shape: CursorShape,
}

/// Draws columns `first..first + n` of one row with its top-left at
/// (`x`, `y`), clipped to `area`. The background is already the default.
#[allow(clippy::too_many_arguments)]
pub fn row(
    c: &mut Canvas,
    area: Rect,
    look: &Look,
    pal: &Colors,
    x: i32,
    y: i32,
    cells: &[Cell],
    clusters: &Clusters,
    first: usize,
    n: usize,
    cursor: Option<Cursor>,
) {
    let end = (first + n).min(cells.len());
    let mut buf = [0u8; 4];
    for (col, cell) in cells.iter().enumerate().take(end).skip(first) {
        let cx = x + (col - first) as i32 * look.cw;
        let wide = cell.is_wide() && col + 1 < end;
        let w = if wide { look.cw * 2 } else { look.cw };
        let rect = Rect::new(cx, y, w, look.ch).intersect(&area);
        // A spacer in the first column is the half of a char we can't see.
        if cell.is_spacer() && col != first {
            continue;
        }
        let (mut fg, mut bg) = pal.pair(cell);
        let at_cursor = cursor
            .as_ref()
            .is_some_and(|k| k.col == col || (wide && k.col == col + 1));
        let shape = cursor.as_ref().map(|k| k.shape);
        if at_cursor && shape == Some(CursorShape::Block) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if bg != pal.bg {
            c.fill(rect, bg);
        }
        if !cell.is_spacer() && (cell.is_cluster() || cell.code() != ' ' as u32) {
            let font = if cell.attrs() & attr::BOLD != 0 {
                look.bold
            } else {
                look.font
            };
            let text = if cell.is_cluster() {
                clusters.get(cell.code())
            } else {
                cell.ch().encode_utf8(&mut buf)
            };
            let old = c.set_clip(rect);
            font.draw(c, cx, y + look.base, text, fg);
            c.set_clip(old);
        }
        if cell.attrs() & attr::UNDERLINE != 0 {
            c.fill(Rect::new(cx, y + look.base + 1, w, 1).intersect(&area), fg);
        }
        if cell.attrs() & attr::STRIKE != 0 {
            c.fill(Rect::new(cx, y + look.ch / 2, w, 1).intersect(&area), fg);
        }
        if at_cursor {
            let mark = match shape {
                Some(CursorShape::Underline) => Rect::new(cx, y + look.ch - 2, w, 2),
                Some(CursorShape::Bar) => Rect::new(cx, y, 2, look.ch),
                _ => continue,
            };
            c.fill(mark.intersect(&area), pal.fg);
        }
    }
    // A cursor past the last cell drawn: the row was shorter.
    if let Some(k) = cursor.filter(|k| k.col >= end && k.col < first + n) {
        let cx = x + (k.col - first) as i32 * look.cw;
        let r = match k.shape {
            CursorShape::Block => Rect::new(cx, y, look.cw, look.ch),
            CursorShape::Underline => Rect::new(cx, y + look.ch - 2, look.cw, 2),
            CursorShape::Bar => Rect::new(cx, y, 2, look.ch),
        };
        c.fill(r.intersect(&area), pal.fg);
    }
}
