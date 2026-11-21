//! Lines that scrolled off the top, kept compact under a byte budget.
//!
//! A line is stored once, as a logical line: UTF-8 text plus style runs,
//! not as screen rows. The view wraps it at its own width, so a zoom or a
//! resize rewrites nothing here. All records sit in one buffer; a line costs
//! its text, 7 bytes of header, 8 bytes per style run and 4 of index.

use super::cell::{Cell, Clusters};
use super::grid::Row;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

const HEADER: usize = 7;
const RUN: usize = 8;
/// Text past this is cut into another record, so a huge line can't hold
/// the whole budget hostage.
pub const LINE_MAX: usize = 32 * 1024;
const GROW: usize = 16 * 1024;

pub mod flag {
    /// The logical line goes on in the next record.
    pub const CONT: u8 = 1;
    /// A shell prompt starts on this line (OSC 133).
    pub const PROMPT: u8 = 2;
}

type Style = (u8, u16, u16);

/// One line of history.
pub struct Line<'a> {
    pub text: &'a str,
    runs: &'a [u8],
    pub flags: u8,
}

impl<'a> Line<'a> {
    fn run(&self, i: usize) -> (usize, Style) {
        let r = &self.runs[i * RUN..i * RUN + RUN];
        let u = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
        (u(0) as usize, (r[2], u(4), u(6)))
    }

    fn nruns(&self) -> usize {
        self.runs.len() / RUN
    }

    /// The units of the line: a base char with its marks, and its width.
    fn units(&self) -> Units<'a> {
        Units { s: self.text }
    }

    /// How many rows the line takes at `width` columns.
    pub fn rows(&self, width: usize) -> usize {
        let mut rows = 1;
        let mut col = 0;
        for (_, w) in self.units() {
            if col + w > width {
                rows += 1;
                col = 0;
            }
            col += w;
        }
        rows
    }

    /// Fills `out` with the cells of row `sub` at `width` columns. Text with
    /// combining marks goes to `clusters`, which the caller clears.
    pub fn cells(&self, width: usize, sub: usize, out: &mut Vec<Cell>, clusters: &mut Clusters) {
        out.clear();
        let mut row = 0;
        let mut col = 0;
        let mut run = 0;
        let mut style = (0xc0, 16, 16);
        for (unit, (s, w)) in self.units().enumerate() {
            while run < self.nruns() && self.run(run).0 <= unit {
                style = self.run(run).1;
                run += 1;
            }
            if col + w > width {
                row += 1;
                col = 0;
                if row > sub {
                    break;
                }
            }
            col += w;
            if row < sub {
                continue;
            }
            let mut chars = s.chars();
            let base = chars.next().unwrap_or(' ');
            let pen = Cell::from_style_key(' ', style).pen();
            let mut c = if w == 2 {
                Cell::wide(base, &pen)
            } else {
                Cell::new(base, &pen)
            };
            if chars.next().is_some() {
                if let Some(id) = clusters.intern(s) {
                    c = c.with_cluster(id);
                }
            }
            out.push(c);
            if w == 2 {
                out.push(Cell::spacer(&pen));
            }
        }
    }
}

struct Units<'a> {
    s: &'a str,
}

impl<'a> Iterator for Units<'a> {
    type Item = (&'a str, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let mut it = self.s.char_indices();
        let (_, base) = it.next()?;
        let w = base.width().unwrap_or(1).max(1);
        let end = it
            .find(|(_, c)| c.width() != Some(0))
            .map_or(self.s.len(), |(i, _)| i);
        let (unit, rest) = self.s.split_at(end);
        self.s = rest;
        Some((unit, w))
    }
}

#[derive(Default)]
struct Open {
    /// UTF-8, kept as bytes so ASCII runs go in without a char at a time.
    text: Vec<u8>,
    runs: Vec<u8>,
    units: usize,
    last: Option<Style>,
    flags: u8,
    active: bool,
}

impl Open {
    fn style(&mut self, cell: &Cell) {
        let key = cell.style_key();
        if self.last != Some(key) {
            self.last = Some(key);
            let col = self.units.min(u16::MAX as usize) as u16;
            self.runs.extend_from_slice(&col.to_le_bytes());
            self.runs.extend_from_slice(&[key.0, 0]);
            self.runs.extend_from_slice(&key.1.to_le_bytes());
            self.runs.extend_from_slice(&key.2.to_le_bytes());
        }
    }

    /// From the first cell on, the ASCII cells in its style, in one pass.
    /// Returns how many went in; 0 if the first isn't such a cell.
    fn push_ascii(&mut self, cells: &[Cell]) -> usize {
        if !cells[0].is_ascii() {
            return 0;
        }
        self.style(&cells[0]);
        let look = cells[0].bits() & Cell::LOOK;
        self.text.reserve(cells.len());
        let mut n = 0;
        for c in cells {
            let b = c.bits();
            let cp = b as u32 & 0x1f_ffff;
            if b & Cell::LOOK != look || cp.wrapping_sub(0x20) >= 0x5f {
                break;
            }
            self.text.push(cp as u8);
            n += 1;
        }
        self.units += n;
        n
    }

    fn push(&mut self, cell: &Cell, clusters: &Clusters) {
        self.style(cell);
        let mut s = String::new();
        clusters.push_text(cell, &mut s);
        self.text.extend_from_slice(s.as_bytes());
        self.units += 1;
    }

    fn line(&self) -> Line<'_> {
        Line {
            text: std::str::from_utf8(&self.text).unwrap_or(""),
            runs: &self.runs,
            flags: self.flags,
        }
    }
}

pub struct History {
    data: Vec<u8>,
    /// Where each record starts, counted from the first byte ever stored, so
    /// dropping old records moves no index.
    starts: VecDeque<u32>,
    /// The count of `data[0]`.
    base: u32,
    max: usize,
    /// Lines dropped from the front, so a line keeps its id.
    dropped: u64,
    open: Open,
}

impl History {
    pub fn new(max: usize) -> History {
        History {
            data: Vec::new(),
            starts: VecDeque::new(),
            base: 0,
            max,
            dropped: 0,
            open: Open::default(),
        }
    }

    pub fn set_max(&mut self, max: usize) {
        self.max = max;
        self.trim();
    }

    /// Lines, the one still growing included.
    pub fn len(&self) -> usize {
        self.starts.len() + self.open.active as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes held, for the budget and the gate.
    pub fn bytes(&self) -> usize {
        self.data.capacity() + self.starts.capacity() * 4 + self.open.text.capacity()
    }

    /// The id of line `i`, stable while it is kept.
    pub fn id(&self, i: usize) -> u64 {
        self.dropped + i as u64
    }

    pub fn line(&self, i: usize) -> Line<'_> {
        if i == self.starts.len() {
            return self.open.line();
        }
        let off = self.starts[i].wrapping_sub(self.base) as usize;
        let d = &self.data[off..];
        let len = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
        let nruns = u16::from_le_bytes([d[4], d[5]]) as usize;
        let text = &d[HEADER..HEADER + len];
        Line {
            text: std::str::from_utf8(text).unwrap_or(""),
            runs: &d[HEADER + len..HEADER + len + nruns * RUN],
            flags: d[6],
        }
    }

    /// A row leaves the top of the screen.
    pub fn push(&mut self, row: &Row, clusters: &Clusters) {
        self.open.active = true;
        if row.prompt {
            self.open.flags |= flag::PROMPT;
        }
        let used = if row.wrapped {
            row.cells.len()
        } else {
            row.used()
        };
        let cells = &row.cells[..used];
        let mut i = 0;
        while i < cells.len() {
            let first = &cells[i];
            let n = self.open.push_ascii(&cells[i..]);
            if n > 0 {
                i += n;
            } else {
                if !first.is_spacer() {
                    self.open.push(first, clusters);
                }
                i += 1;
            }
            if self.open.text.len() >= LINE_MAX {
                self.open.flags |= flag::CONT;
                self.close();
                self.open.active = true;
            }
        }
        if !row.wrapped {
            self.close();
        }
    }

    /// Marks the line being written as holding a prompt.
    pub fn mark_open(&mut self, f: u8) {
        self.open.flags |= f;
    }

    fn close(&mut self) {
        let o = &mut self.open;
        let need = HEADER + o.text.len() + o.runs.len();
        if self.data.len() + need > self.data.capacity() {
            let room = self.max.saturating_sub(self.data.capacity());
            self.data.reserve_exact(need.max(GROW.min(room)));
        }
        self.starts
            .push_back(self.base.wrapping_add(self.data.len() as u32));
        self.data
            .extend_from_slice(&(o.text.len() as u32).to_le_bytes());
        self.data
            .extend_from_slice(&((o.runs.len() / RUN) as u16).to_le_bytes());
        self.data.push(o.flags);
        self.data.extend_from_slice(&o.text);
        self.data.extend_from_slice(&o.runs);
        // Keep the buffers: the next line reuses them.
        o.text.clear();
        o.runs.clear();
        o.units = 0;
        o.last = None;
        o.flags = 0;
        o.active = false;
        self.trim();
    }

    /// Drops old lines once over budget, down to three quarters of it, so
    /// the move happens rarely.
    fn trim(&mut self) {
        let held = |h: &History| h.data.len() + h.starts.len() * 4;
        if held(self) <= self.max {
            return;
        }
        let goal = self.max / 4 * 3;
        let mut k = 0;
        while k < self.starts.len() {
            let cut = self.starts[k].wrapping_sub(self.base) as usize;
            if held(self) - cut - k * 4 <= goal {
                break;
            }
            k += 1;
        }
        let cut = match self.starts.get(k) {
            Some(s) => s.wrapping_sub(self.base) as usize,
            None => self.data.len(),
        };
        self.data.drain(..cut);
        self.base = self.base.wrapping_add(cut as u32);
        self.starts.drain(..k);
        self.dropped += k as u64;
        if self.data.capacity() > self.max {
            self.data.shrink_to(self.max);
        }
    }

    pub fn clear(&mut self) {
        self.dropped += self.len() as u64;
        *self = History {
            dropped: self.dropped,
            ..History::new(self.max)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::cell::{Color, Pen};

    fn row(s: &str, wrapped: bool, cols: usize) -> Row {
        let mut r = Row::new(cols, Cell::BLANK);
        let mut x = 0;
        for ch in s.chars() {
            if ch.width() == Some(2) {
                r.cells[x] = Cell::wide(ch, &Pen::DEFAULT);
                r.cells[x + 1] = Cell::spacer(&Pen::DEFAULT);
                x += 2;
            } else {
                r.cells[x] = Cell::new(ch, &Pen::DEFAULT);
                x += 1;
            }
        }
        r.wrapped = wrapped;
        r
    }

    fn texts(h: &History, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut cells = Vec::new();
        let mut cl = Clusters::default();
        for i in 0..h.len() {
            let l = h.line(i);
            for sub in 0..l.rows(width) {
                l.cells(width, sub, &mut cells, &mut cl);
                out.push(
                    cells
                        .iter()
                        .filter(|c| !c.is_spacer())
                        .map(|c| c.ch())
                        .collect(),
                );
            }
        }
        out
    }

    #[test]
    fn wrapped_rows_join_into_one_line() {
        let mut h = History::new(1 << 20);
        let cl = Clusters::default();
        h.push(&row("abcd", true, 4), &cl);
        assert_eq!(h.len(), 1);
        h.push(&row("ef", false, 4), &cl);
        h.push(&row("x", false, 4), &cl);
        assert_eq!(h.len(), 2);
        assert_eq!(h.line(0).text, "abcdef");
        assert_eq!(texts(&h, 3), ["abc", "def", "x"]);
        assert_eq!(texts(&h, 8), ["abcdef", "x"]);
    }

    #[test]
    fn wide_chars_wrap_whole() {
        let mut h = History::new(1 << 20);
        h.push(&row("a漢字", false, 6), &Clusters::default());
        assert_eq!(h.line(0).rows(2), 3);
        assert_eq!(texts(&h, 2), ["a", "漢", "字"]);
    }

    #[test]
    fn styles_come_back() {
        let mut h = History::new(1 << 20);
        let mut r = row("ab", false, 4);
        let mut pen = Pen::DEFAULT;
        pen.set_fg(Color::Index(2));
        r.cells[1] = Cell::new('b', &pen);
        h.push(&r, &Clusters::default());
        let mut cells = Vec::new();
        h.line(0).cells(4, 0, &mut cells, &mut Clusters::default());
        assert_eq!(cells[0].fg(), Color::DEFAULT);
        assert_eq!(cells[1].fg(), Color::Index(2));
    }

    #[test]
    fn combining_marks_stay_with_their_base() {
        let mut cl = Clusters::default();
        let id = cl.intern("e\u{301}").unwrap();
        let mut r = row("xe", false, 4);
        r.cells[1] = r.cells[1].with_cluster(id);
        let mut h = History::new(1 << 20);
        h.push(&r, &cl);
        assert_eq!(h.line(0).text, "xe\u{301}");
        let mut cells = Vec::new();
        let mut out = Clusters::default();
        h.line(0).cells(4, 0, &mut cells, &mut out);
        assert_eq!(cells.len(), 2);
        assert_eq!(out.get(cells[1].code()), "e\u{301}");
    }

    #[test]
    fn the_budget_drops_old_lines_and_ids_hold() {
        let mut h = History::new(2000);
        let cl = Clusters::default();
        for i in 0..200 {
            h.push(&row(&format!("line {i:03} xxxxxxxxxx"), false, 30), &cl);
        }
        assert!(h.data.len() + h.starts.len() * 4 <= 2000);
        let last = h.len() - 1;
        assert_eq!(h.line(last).text, "line 199 xxxxxxxxxx");
        assert_eq!(h.id(last), 199);
        let first: u64 = h.line(0).text[5..8].parse().unwrap();
        assert_eq!(h.id(0), first);
    }

    #[test]
    fn a_plain_line_is_compact() {
        let mut h = History::new(1 << 20);
        let cl = Clusters::default();
        let text = "drwxr-xr-x  2 rezor staff   64 Nov 21 10:00 x";
        for _ in 0..1000 {
            h.push(&row(text, false, 53), &cl);
        }
        let per_line = (h.data.len() + h.starts.len() * 4) / 1000;
        assert!(per_line <= text.len() + 20, "{per_line}");
    }
}
