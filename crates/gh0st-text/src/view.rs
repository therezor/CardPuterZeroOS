//! A line's bytes as cells on screen. Tabs stop every [`TAB`] columns, wide
//! characters take two cells, and a byte that is not UTF-8 shows as one
//! cell of its own, so every byte can be seen and edited.

use unicode_width::UnicodeWidthChar;

pub const TAB: u32 = 4;

/// What one character is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Char(char),
    Tab,
    /// A byte that is not part of any UTF-8 character.
    Bad(u8),
    /// A control character other than tab.
    Control(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Where it starts in the line.
    pub byte: usize,
    /// Its bytes.
    pub len: usize,
    pub col: u32,
    pub width: u32,
    pub glyph: Glyph,
}

/// The cells of `line`, in order.
pub fn cells(line: &[u8]) -> Cells<'_> {
    Cells {
        line,
        at: 0,
        col: 0,
    }
}

pub struct Cells<'a> {
    line: &'a [u8],
    at: usize,
    col: u32,
}

impl Iterator for Cells<'_> {
    type Item = Cell;

    fn next(&mut self) -> Option<Cell> {
        let rest = self.line.get(self.at..).filter(|r| !r.is_empty())?;
        let (glyph, len) = decode(rest);
        let width = match glyph {
            Glyph::Tab => TAB - self.col % TAB,
            Glyph::Char(c) => c.width().unwrap_or(1).clamp(1, 2) as u32,
            Glyph::Bad(_) | Glyph::Control(_) => 1,
        };
        let cell = Cell {
            byte: self.at,
            len,
            col: self.col,
            width,
            glyph,
        };
        self.at += len;
        self.col += width;
        Some(cell)
    }
}

/// The first character of `b` and its length.
fn decode(b: &[u8]) -> (Glyph, usize) {
    let c = b[0];
    if c == b'\t' {
        return (Glyph::Tab, 1);
    }
    if c < 0x20 || c == 0x7f {
        return (Glyph::Control(c), 1);
    }
    if c < 0x80 {
        return (Glyph::Char(c as char), 1);
    }
    let n = match c {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return (Glyph::Bad(c), 1),
    };
    match b.get(..n).and_then(|s| std::str::from_utf8(s).ok()) {
        Some(s) => match s.chars().next() {
            Some(ch) => (Glyph::Char(ch), n),
            None => (Glyph::Bad(c), 1),
        },
        None => (Glyph::Bad(c), 1),
    }
}

/// The column the byte at `byte` starts in.
pub fn col_of(line: &[u8], byte: usize) -> u32 {
    let mut col = 0;
    for c in cells(line) {
        if c.byte >= byte {
            return c.col;
        }
        col = c.col + c.width;
    }
    col
}

/// The byte of the cell at column `col`, or the line's end past it.
pub fn byte_at(line: &[u8], col: u32) -> usize {
    for c in cells(line) {
        if c.col + c.width > col {
            return c.byte;
        }
    }
    line.len()
}

/// Where the character before `byte` starts, stepping over a whole UTF-8
/// character.
pub fn prev_char(line: &[u8], byte: usize) -> usize {
    let mut last = 0;
    for c in cells(line) {
        if c.byte >= byte {
            break;
        }
        last = c.byte;
    }
    last
}

/// Where the character at `byte` ends.
pub fn next_char(line: &[u8], byte: usize) -> usize {
    if byte >= line.len() {
        return line.len();
    }
    byte + decode(&line[byte..]).1
}

/// The leading blanks of a line: the indent a new line takes.
pub fn indent(line: &[u8]) -> &[u8] {
    let n = line
        .iter()
        .take_while(|&&c| c == b' ' || c == b'\t')
        .count();
    &line[..n]
}

/// Where a word starts or ends, from `byte`, for Ctrl and an arrow.
pub fn word_left(line: &[u8], byte: usize) -> usize {
    let mut i = byte.min(line.len());
    while i > 0 && !is_word(line[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(line[i - 1]) {
        i -= 1;
    }
    i
}

pub fn word_right(line: &[u8], byte: usize) -> usize {
    let mut i = byte.min(line.len());
    while i < line.len() && !is_word(line[i]) {
        i += 1;
    }
    while i < line.len() && is_word(line[i]) {
        i += 1;
    }
    i
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_wide_and_bad_bytes_take_their_cells() {
        let line = "a\tb\u{4e2d}c".as_bytes();
        let v: Vec<(u32, u32)> = cells(line).map(|c| (c.col, c.width)).collect();
        assert_eq!(v, [(0, 1), (1, 3), (4, 1), (5, 2), (7, 1)]);
        let bad = b"x\xff\xe2\x82y";
        let g: Vec<Glyph> = cells(bad).map(|c| c.glyph).collect();
        assert_eq!(
            g,
            [
                Glyph::Char('x'),
                Glyph::Bad(0xff),
                Glyph::Bad(0xe2),
                Glyph::Bad(0x82),
                Glyph::Char('y')
            ]
        );
    }

    #[test]
    fn columns_and_bytes_map_both_ways() {
        let line = "\tx\u{e9}y".as_bytes();
        assert_eq!(col_of(line, 0), 0);
        assert_eq!(col_of(line, 1), 4);
        assert_eq!(col_of(line, 2), 5);
        assert_eq!(col_of(line, 4), 6);
        assert_eq!(col_of(line, 5), 7);
        assert_eq!(byte_at(line, 2), 0);
        assert_eq!(byte_at(line, 4), 1);
        assert_eq!(byte_at(line, 5), 2);
        assert_eq!(byte_at(line, 99), 5);
        assert_eq!(prev_char(line, 4), 2);
        assert_eq!(next_char(line, 2), 4);
    }

    #[test]
    fn words_and_indents() {
        let line = b"    let some_name = 3;";
        assert_eq!(indent(line), b"    ");
        assert_eq!(word_left(line, 13), 8);
        assert_eq!(word_right(line, 8), 17);
    }
}
