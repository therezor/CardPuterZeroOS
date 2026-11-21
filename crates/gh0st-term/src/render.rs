//! Cells to pixels. Each cell clips its own glyph, so a glyph wider than
//! its cell never spills into the next one. Box drawing, blocks and
//! braille are drawn, not taken from the font. A Nerd Font icon is the one
//! exception: it takes the blank after it too, as in kitty and WezTerm.

use crate::boxes;
use crate::vt::CursorShape;
use crate::vt::cell::{Cell, Clusters, Color, attr};
use gh0st_app::gfx::color::mix;
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::theme::{self, TermFace};

const CW: i32 = theme::TERM_CURSOR_W;
use gh0st_app::gfx::{Canvas, Rect, Theme};

/// A face and the cell it sets in.
#[derive(Clone, Copy)]
pub struct Look {
    pub font: Font,
    /// None: bold strikes the regular glyph twice, a pixel apart.
    pub bold: Option<Font>,
    /// Nerd Font icons, two cells wide.
    pub icons: Font,
    pub cw: i32,
    pub ch: i32,
    /// Baseline from the cell top.
    pub base: i32,
}

impl Look {
    pub fn of(face: &TermFace) -> Look {
        let font = face.regular;
        let ch = font.line.max(1);
        Look {
            font,
            bold: face.bold,
            icons: face.icons,
            cw: font.measure("M").max(1),
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
    pub accent: u16,
}

impl Colors {
    pub fn of(t: &Theme) -> Colors {
        Colors {
            fg: t.c.fg,
            bg: t.c.bg,
            ansi: t.c.ansi,
            accent: t.c.accent,
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
            fg = mix(bg, fg, theme::TERM_DIM);
        }
        (fg, bg)
    }
}

#[derive(Clone, Copy)]
pub struct Cursor {
    pub col: usize,
    pub shape: CursorShape,
}

/// Where a row goes and which of its cells show.
pub struct Span {
    pub x: i32,
    pub y: i32,
    pub first: usize,
    pub n: usize,
}

/// Draws one row of cells, clipped to `area`. The background is already
/// the default colour. `marked`: columns drawn as selected or found.
#[allow(clippy::too_many_arguments)]
pub fn row(
    c: &mut Canvas,
    area: Rect,
    look: &Look,
    pal: &Colors,
    span: &Span,
    cells: &[Cell],
    clusters: &Clusters,
    cursor: Option<Cursor>,
    marked: Option<(usize, usize)>,
) {
    let end = (span.first + span.n).min(cells.len());
    let mut buf = [0u8; 4];
    let mut taken = false;
    for (col, cell) in cells.iter().enumerate().take(end).skip(span.first) {
        if std::mem::take(&mut taken) {
            continue;
        }
        let cx = span.x + (col - span.first) as i32 * look.cw;
        let icon = !cell.is_cluster() && private_use(cell.code()) && look.icons.has(cell.ch());
        taken = icon && col + 1 < end && spills(pal, cell, &cells[col + 1], col, cursor, marked);
        let wide = (cell.is_wide() && col + 1 < end) || taken;
        let w = if wide { look.cw * 2 } else { look.cw };
        let rect = Rect::new(cx, span.y, w, look.ch).intersect(&area);
        // A spacer in the first column is the half of a char we can't see.
        if cell.is_spacer() && col != span.first {
            continue;
        }
        let (mut fg, mut bg) = pal.pair(cell);
        if marked.is_some_and(|(a, b)| col >= a && col < b) {
            (fg, bg) = (pal.bg, pal.accent);
        }
        let at_cursor = cursor
            .as_ref()
            .is_some_and(|k| k.col == col || (wide && k.col == col + 1));
        let shape = cursor.map(|k| k.shape);
        if at_cursor && shape == Some(CursorShape::Block) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if bg != pal.bg {
            c.fill(rect, bg);
        }
        if !cell.is_spacer() && (cell.is_cluster() || cell.code() != ' ' as u32) {
            let drawn = !cell.is_cluster() && boxes::draw(c, rect, cell.code(), fg, bg);
            if icon {
                // Without the blank after it, the middle of the icon shows.
                let x = if taken { cx } else { cx - look.cw / 2 };
                let old = c.set_clip(rect);
                look.icons.draw(
                    c,
                    x,
                    span.y + look.base,
                    cell.ch().encode_utf8(&mut buf),
                    fg,
                );
                c.set_clip(old);
            } else if !drawn {
                glyph(c, look, rect, cx, span.y, cell, clusters, fg, &mut buf);
            }
        }
        if cell.attrs() & attr::UNDERLINE != 0 {
            let y = (span.y + look.base + 1).min(span.y + look.ch - 1);
            c.fill(Rect::new(cx, y, w, 1).intersect(&area), fg);
        }
        if cell.attrs() & attr::STRIKE != 0 {
            c.fill(
                Rect::new(cx, span.y + look.ch / 2, w, 1).intersect(&area),
                fg,
            );
        }
        if at_cursor {
            let mark = match shape {
                Some(CursorShape::Underline) => Rect::new(cx, span.y + look.ch - CW, w, CW),
                Some(CursorShape::Bar) => Rect::new(cx, span.y, CW, look.ch),
                _ => continue,
            };
            c.fill(mark.intersect(&area), pal.fg);
        }
    }
    // A cursor past the cells drawn: the row was shorter.
    if let Some(k) = cursor.filter(|k| k.col >= end && k.col < span.first + span.n) {
        let cx = span.x + (k.col - span.first) as i32 * look.cw;
        let r = match k.shape {
            CursorShape::Block => Rect::new(cx, span.y, look.cw, look.ch),
            CursorShape::Underline => Rect::new(cx, span.y + look.ch - CW, look.cw, CW),
            CursorShape::Bar => Rect::new(cx, span.y, CW, look.ch),
        };
        c.fill(r.intersect(&area), pal.fg);
    }
}

/// Nerd Fonts puts its icons in the Private Use Areas. Powerline's are
/// drawn by `boxes`.
fn private_use(cp: u32) -> bool {
    matches!(cp, 0xe000..=0xe09f | 0xe0d8..=0xf8ff | 0xf0000..=0x10fffd)
}

/// Whether an icon at `col` may draw over `next`: a plain blank that looks
/// the same, with no cursor on it and no mark that starts or ends between.
fn spills(
    pal: &Colors,
    cell: &Cell,
    next: &Cell,
    col: usize,
    cursor: Option<Cursor>,
    marked: Option<(usize, usize)>,
) -> bool {
    const LINES: u32 = attr::UNDERLINE | attr::STRIKE;
    let in_mark = |x: usize| marked.is_some_and(|(a, b)| x >= a && x < b);
    !next.is_cluster()
        && !next.is_spacer()
        && !next.is_wide()
        && next.code() == ' ' as u32
        && pal.pair(next).1 == pal.pair(cell).1
        && next.attrs() & LINES == cell.attrs() & LINES
        && cursor.is_none_or(|k| k.col != col + 1)
        && in_mark(col) == in_mark(col + 1)
}

#[allow(clippy::too_many_arguments)]
fn glyph(
    c: &mut Canvas,
    look: &Look,
    rect: Rect,
    x: i32,
    y: i32,
    cell: &Cell,
    clusters: &Clusters,
    fg: u16,
    buf: &mut [u8; 4],
) {
    let text = if cell.is_cluster() {
        clusters.get(cell.code())
    } else {
        cell.ch().encode_utf8(buf)
    };
    let bold = cell.attrs() & attr::BOLD != 0;
    let font = match (bold, look.bold) {
        (true, Some(b)) => b,
        _ => look.font,
    };
    let old = c.set_clip(rect);
    font.draw(c, x, y + look.base, text, fg);
    if bold && look.bold.is_none() {
        font.draw(c, x + 1, y + look.base, text, fg);
    }
    c.set_clip(old);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::cell::Pen;

    // Yazi's folder.
    const FOLDER: char = '\u{e5ff}';

    /// Draws `text` in one row at text size `zoom` and returns which cells
    /// got ink.
    fn inked(zoom: usize, text: &str) -> Vec<bool> {
        let th = Theme::default();
        let pal = Colors::of(&th);
        let look = Look::of(&theme::TERM_TERMINUS[zoom]);
        let pen = Pen::default();
        let cells: Vec<Cell> = text.chars().map(|c| Cell::new(c, &pen)).collect();
        let n = cells.len();
        let mut c = Canvas::new(n as i32 * look.cw, look.ch);
        c.fill(c.bounds(), pal.bg);
        let span = Span {
            x: 0,
            y: 0,
            first: 0,
            n,
        };
        let area = c.bounds();
        row(
            &mut c,
            area,
            &look,
            &pal,
            &span,
            &cells,
            &Clusters::default(),
            None,
            None,
        );
        (0..n as i32)
            .map(|i| {
                (i * look.cw..(i + 1) * look.cw)
                    .any(|x| (0..look.ch).any(|y| c.get(x, y) != pal.bg))
            })
            .collect()
    }

    #[test]
    fn an_icon_takes_the_blank_after_it() {
        for z in 0..theme::TERM_TERMINUS.len() {
            assert_eq!(inked(z, &format!("{FOLDER}  ")), [true, true, false]);
            assert_eq!(inked(z, &format!(" {FOLDER} a")), [false, true, true, true]);
        }
    }

    #[test]
    fn an_icon_stays_in_its_cell_when_text_follows() {
        for z in 0..theme::TERM_TERMINUS.len() {
            assert_eq!(inked(z, &format!("{FOLDER}a ")), [true, true, false]);
            // The last column has no blank after it on screen.
            assert_eq!(inked(z, &format!(" {FOLDER}")), [false, true]);
        }
    }
}
