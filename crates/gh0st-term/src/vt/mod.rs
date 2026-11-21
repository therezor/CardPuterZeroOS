//! The terminal: a `vte::Perform` that keeps the screen, the history and the
//! modes. PTY bytes go in through [`Vt::feed`]; what the terminal must send
//! back (device reports) collects in [`Terminal::replies`].

pub mod cell;
pub mod grid;
pub mod history;
#[cfg(test)]
mod tests;

use cell::{Cell, Clusters, Color, Pen, attr};
use grid::Grid;
use history::History;
use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

/// An OSC longer than this is cut. It bounds what a hostile host can make
/// the parser hold.
const OSC_MAX: usize = 4096;
const TITLE_MAX: usize = 256;
/// REP can't be told to fill more than a screen.
const REP_MAX: usize = 64 * 1024;

/// The parser and the terminal it drives.
pub struct Vt {
    parser: vte::Parser<OSC_MAX>,
    pub t: Terminal,
}

impl Vt {
    pub fn new(cols: usize, rows: usize, history: usize) -> Vt {
        Vt {
            parser: vte::Parser::<OSC_MAX>::new_with_size(),
            t: Terminal::new(cols, rows, history),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.t, bytes);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    Ascii,
    /// DEC special graphics: the line-drawing set.
    Line,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Copy, Default)]
struct Cursor {
    x: usize,
    y: usize,
    pen: Pen,
    /// The last column was written: the next char goes on the next line.
    wrap_next: bool,
}

#[derive(Clone, Copy)]
struct Saved {
    cur: Cursor,
    origin: bool,
    charsets: [Charset; 2],
    gl: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Modes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub autowrap: bool,
    pub origin: bool,
    pub insert: bool,
    /// LNM: LF also returns the carriage.
    pub newline: bool,
    pub show_cursor: bool,
    pub blink: bool,
    pub bracketed_paste: bool,
    pub focus: bool,
    /// Synchronized output (2026): hold the draw until it ends.
    pub sync: bool,
}

impl Default for Modes {
    fn default() -> Modes {
        Modes {
            app_cursor: false,
            app_keypad: false,
            autowrap: true,
            origin: false,
            insert: false,
            newline: false,
            show_cursor: true,
            blink: false,
            bracketed_paste: false,
            focus: false,
            sync: false,
        }
    }
}

/// What happened that the app may show: bits in [`Terminal::events`].
pub mod event {
    pub const TITLE: u8 = 1;
    pub const BELL: u8 = 2;
    pub const CWD: u8 = 4;
}

pub struct Terminal {
    pub cols: usize,
    pub rows: usize,
    main: Grid,
    alt: Grid,
    pub alt_active: bool,
    cur: Cursor,
    saved_main: Option<Saved>,
    saved_alt: Option<Saved>,
    top: usize,
    bot: usize,
    pub modes: Modes,
    charsets: [Charset; 2],
    gl: usize,
    tabs: Vec<bool>,
    pub title: String,
    /// The working directory the shell last reported (OSC 7).
    pub cwd: Option<String>,
    pub history: History,
    pub clusters: Clusters,
    /// Bytes for the PTY: answers to device queries.
    pub replies: Vec<u8>,
    pub shape: CursorShape,
    /// Bits from [`event`], cleared by the app.
    pub events: u8,
    last: Option<char>,
}

fn tab_stops(cols: usize) -> Vec<bool> {
    (0..cols).map(|x| x % 8 == 0 && x > 0).collect()
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, history: usize) -> Terminal {
        let (cols, rows) = (cols.max(2), rows.max(1));
        Terminal {
            cols,
            rows,
            main: Grid::new(cols, rows),
            alt: Grid::new(cols, rows),
            alt_active: false,
            cur: Cursor::default(),
            saved_main: None,
            saved_alt: None,
            top: 0,
            bot: rows - 1,
            modes: Modes::default(),
            charsets: [Charset::Ascii; 2],
            gl: 0,
            tabs: tab_stops(cols),
            title: String::new(),
            cwd: None,
            history: History::new(history),
            clusters: Clusters::default(),
            replies: Vec::new(),
            shape: CursorShape::default(),
            events: 0,
            last: None,
        }
    }

    /// The screen in use: the alternate one while a full-screen program runs.
    pub fn grid(&self) -> &Grid {
        if self.alt_active {
            &self.alt
        } else {
            &self.main
        }
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.main
        }
    }

    /// Cursor column and row.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cur.x, self.cur.y)
    }

    /// New size. Rows that no longer fit above the cursor go to history.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if cols == self.cols && rows == self.rows {
            return;
        }
        if rows < self.rows {
            let over = (self.cur.y + 1).saturating_sub(rows);
            if over > 0 {
                for i in 0..over {
                    self.history.push(&self.main.rows[i], &self.clusters);
                }
                self.main.rows.rotate_left(over);
                if !self.alt_active {
                    self.cur.y -= over;
                }
            }
        }
        self.main.resize(cols, rows);
        self.alt.resize(cols, rows);
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bot = rows - 1;
        self.tabs = tab_stops(cols);
        self.cur.x = self.cur.x.min(cols - 1);
        self.cur.y = self.cur.y.min(rows - 1);
        self.cur.wrap_next = false;
        for s in [&mut self.saved_main, &mut self.saved_alt]
            .into_iter()
            .flatten()
        {
            s.cur.x = s.cur.x.min(cols - 1);
            s.cur.y = s.cur.y.min(rows - 1);
        }
    }

    fn mark_cursor(&mut self) {
        let y = self.cur.y;
        self.grid_mut().mark(y);
    }

    // Cursor moves.

    fn goto(&mut self, y: usize, x: usize) {
        self.mark_cursor();
        let (lo, hi) = if self.modes.origin {
            (self.top, self.bot)
        } else {
            (0, self.rows - 1)
        };
        self.cur.y = (lo + y).clamp(lo, hi);
        self.cur.x = x.min(self.cols - 1);
        self.cur.wrap_next = false;
        self.mark_cursor();
    }

    fn set_x(&mut self, x: usize) {
        self.cur.x = x.min(self.cols - 1);
        self.cur.wrap_next = false;
        self.mark_cursor();
    }

    fn set_y(&mut self, y: usize) {
        self.mark_cursor();
        self.cur.y = y.min(self.rows - 1);
        self.cur.wrap_next = false;
        self.mark_cursor();
    }

    fn up(&mut self, n: usize) {
        let lo = if self.cur.y >= self.top { self.top } else { 0 };
        self.set_y(self.cur.y.saturating_sub(n).max(lo));
    }

    fn down(&mut self, n: usize) {
        let hi = if self.cur.y <= self.bot {
            self.bot
        } else {
            self.rows - 1
        };
        self.set_y((self.cur.y + n).min(hi));
    }

    /// LF: down one row, scrolling at the bottom margin.
    fn index(&mut self) {
        if self.cur.y == self.bot {
            self.scroll_up(1);
        } else if self.cur.y + 1 < self.rows {
            self.set_y(self.cur.y + 1);
        }
        self.cur.wrap_next = false;
    }

    fn reverse_index(&mut self) {
        if self.cur.y == self.top {
            self.scroll_down(1);
        } else if self.cur.y > 0 {
            self.set_y(self.cur.y - 1);
        }
        self.cur.wrap_next = false;
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bot) = (self.top, self.bot);
        if top == 0 && !self.alt_active {
            for i in 0..n.min(bot + 1) {
                self.history.push(&self.main.rows[i], &self.clusters);
            }
        }
        let pen = self.cur.pen;
        self.grid_mut().scroll_up(top, bot, n, &pen);
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bot) = (self.top, self.bot);
        let pen = self.cur.pen;
        self.grid_mut().scroll_down(top, bot, n, &pen);
    }

    fn tab(&mut self, n: usize) {
        let mut x = self.cur.x;
        for _ in 0..n {
            x = (x + 1..self.cols)
                .find(|&i| self.tabs[i])
                .unwrap_or(self.cols - 1);
        }
        self.set_x(x);
    }

    fn back_tab(&mut self, n: usize) {
        let mut x = self.cur.x;
        for _ in 0..n {
            x = (0..x).rev().find(|&i| self.tabs[i]).unwrap_or(0);
        }
        self.set_x(x);
    }

    fn wrap(&mut self) {
        let y = self.cur.y;
        self.grid_mut().rows[y].wrapped = true;
        self.cur.x = 0;
        self.index();
    }

    // Writing.

    fn put(&mut self, c: char) {
        let width = match c.width() {
            Some(w) => w,
            None => return,
        };
        if width == 0 {
            self.combine(c);
            return;
        }
        let cols = self.cols;
        if self.cur.wrap_next && self.modes.autowrap {
            self.wrap();
        }
        if width == 2 && self.cur.x + 1 >= cols {
            if self.modes.autowrap {
                let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
                self.grid_mut().erase(y, x, x + 1, &pen);
                self.wrap();
            } else {
                self.cur.x = cols - 2;
            }
        }
        let pen = self.cur.pen;
        let (x, y) = (self.cur.x, self.cur.y);
        let insert = self.modes.insert;
        let g = self.grid_mut();
        if insert {
            g.insert_cells(y, x, width, &pen);
        }
        g.unsplit(y, x);
        if width == 2 {
            g.unsplit(y, x + 1);
            g.rows[y].cells[x] = Cell::wide(c, &pen);
            g.rows[y].cells[x + 1] = Cell::spacer(&pen);
        } else {
            g.rows[y].cells[x] = Cell::new(c, &pen);
        }
        g.mark(y);
        self.last = Some(c);
        if x + width >= cols {
            self.cur.x = cols - 1;
            self.cur.wrap_next = true;
        } else {
            self.cur.x = x + width;
        }
    }

    /// A zero-width char joins the cell before the cursor.
    fn combine(&mut self, c: char) {
        let y = self.cur.y;
        let mut x = self.cur.x;
        if !self.cur.wrap_next {
            if x == 0 {
                return;
            }
            x -= 1;
        }
        if self.grid().rows[y].cells[x].is_spacer() && x > 0 {
            x -= 1;
        }
        let cell = self.grid().rows[y].cells[x];
        let mut s = String::new();
        self.clusters.push_text(&cell, &mut s);
        s.push(c);
        let id = match self.clusters.intern(&s) {
            Some(id) => Some(id),
            None => {
                self.collect_clusters();
                self.clusters.intern(&s)
            }
        };
        if let Some(id) = id {
            let g = self.grid_mut();
            g.rows[y].cells[x] = cell.with_cluster(id);
            g.mark(y);
        }
    }

    /// Rebuilds the cluster table from the cells that still use it. History
    /// holds text, so only the two screens count.
    fn collect_clusters(&mut self) {
        let old = std::mem::take(&mut self.clusters);
        for g in [&mut self.main, &mut self.alt] {
            for r in &mut g.rows {
                for c in r.cells.iter_mut().filter(|c| c.is_cluster()) {
                    *c = match self.clusters.intern(old.get(c.code())) {
                        Some(id) => c.with_cluster(id),
                        None => {
                            Cell::new(old.get(c.code()).chars().next().unwrap_or(' '), &c.pen())
                        }
                    };
                }
            }
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
        let (cols, rows) = (self.cols, self.rows);
        let g = self.grid_mut();
        match mode {
            0 => {
                g.erase(y, x, cols, &pen);
                for r in y + 1..rows {
                    g.erase(r, 0, cols, &pen);
                }
                g.rows[y].wrapped = false;
            }
            1 => {
                for r in 0..y {
                    g.erase(r, 0, cols, &pen);
                }
                g.erase(y, 0, x + 1, &pen);
            }
            2 => g.clear(&pen),
            3 => self.history.clear(),
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
        let cols = self.cols;
        let g = self.grid_mut();
        match mode {
            0 => {
                g.erase(y, x, cols, &pen);
                g.rows[y].wrapped = false;
            }
            1 => g.erase(y, 0, x + 1, &pen),
            2 => {
                g.erase(y, 0, cols, &pen);
                g.rows[y].wrapped = false;
            }
            _ => {}
        }
    }

    fn insert_lines(&mut self, n: usize) {
        let y = self.cur.y;
        if y < self.top || y > self.bot {
            return;
        }
        let (bot, pen) = (self.bot, self.cur.pen);
        self.grid_mut().scroll_down(y, bot, n, &pen);
        self.set_x(0);
    }

    fn delete_lines(&mut self, n: usize) {
        let y = self.cur.y;
        if y < self.top || y > self.bot {
            return;
        }
        let (bot, pen) = (self.bot, self.cur.pen);
        self.grid_mut().scroll_up(y, bot, n, &pen);
        self.set_x(0);
    }

    fn save_cursor(&mut self) {
        let s = Saved {
            cur: self.cur,
            origin: self.modes.origin,
            charsets: self.charsets,
            gl: self.gl,
        };
        if self.alt_active {
            self.saved_alt = Some(s);
        } else {
            self.saved_main = Some(s);
        }
    }

    fn restore_cursor(&mut self) {
        let s = if self.alt_active {
            self.saved_alt
        } else {
            self.saved_main
        };
        self.mark_cursor();
        match s {
            Some(s) => {
                self.cur = s.cur;
                self.modes.origin = s.origin;
                self.charsets = s.charsets;
                self.gl = s.gl;
            }
            None => {
                self.cur = Cursor::default();
                self.modes.origin = false;
            }
        }
        self.cur.x = self.cur.x.min(self.cols - 1);
        self.cur.y = self.cur.y.min(self.rows - 1);
        self.mark_cursor();
    }

    fn set_alt(&mut self, on: bool, clear: bool) {
        if on == self.alt_active {
            return;
        }
        if on && clear {
            let pen = self.cur.pen;
            self.alt.clear(&pen);
        }
        self.alt_active = on;
        self.grid_mut().mark_all();
    }

    fn reset(&mut self) {
        let history = std::mem::replace(&mut self.history, History::new(0));
        *self = Terminal {
            history,
            ..Terminal::new(self.cols, self.rows, 0)
        };
    }

    fn soft_reset(&mut self) {
        self.modes = Modes::default();
        self.cur.pen = Pen::DEFAULT;
        self.cur.wrap_next = false;
        self.top = 0;
        self.bot = self.rows - 1;
        self.charsets = [Charset::Ascii; 2];
        self.gl = 0;
        self.saved_main = None;
        self.saved_alt = None;
    }

    fn reply(&mut self, s: &str) {
        self.replies.extend_from_slice(s.as_bytes());
    }

    fn sgr(&mut self, params: &Params) {
        let pen = &mut self.cur.pen;
        let mut it = params.iter();
        if params.is_empty() {
            *pen = Pen::DEFAULT;
            return;
        }
        while let Some(p) = it.next() {
            match p.first().copied().unwrap_or(0) {
                0 => *pen = Pen::DEFAULT,
                1 => pen.set(attr::BOLD, true),
                2 => pen.set(attr::DIM, true),
                3 => pen.set(attr::ITALIC, true),
                4 => pen.set(attr::UNDERLINE, p.get(1).is_none_or(|&s| s != 0)),
                7 => pen.set(attr::INVERSE, true),
                9 => pen.set(attr::STRIKE, true),
                21 => pen.set(attr::UNDERLINE, true),
                22 => pen.set(attr::BOLD | attr::DIM, false),
                23 => pen.set(attr::ITALIC, false),
                24 => pen.set(attr::UNDERLINE, false),
                27 => pen.set(attr::INVERSE, false),
                29 => pen.set(attr::STRIKE, false),
                n @ 30..=37 => pen.set_fg(Color::Index(n as u8 - 30)),
                38 => {
                    if let Some(c) = extended(p, &mut it) {
                        pen.set_fg(c);
                    }
                }
                39 => pen.set_fg(Color::DEFAULT),
                n @ 40..=47 => pen.set_bg(Color::Index(n as u8 - 40)),
                48 => {
                    if let Some(c) = extended(p, &mut it) {
                        pen.set_bg(c);
                    }
                }
                49 => pen.set_bg(Color::DEFAULT),
                // Underline colour: read past it, we don't draw it.
                58 => {
                    extended(p, &mut it);
                }
                n @ 90..=97 => pen.set_fg(Color::Index(n as u8 - 90 + 8)),
                n @ 100..=107 => pen.set_bg(Color::Index(n as u8 - 100 + 8)),
                _ => {}
            }
        }
    }

    fn private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.app_cursor = on,
            6 => {
                self.modes.origin = on;
                self.goto(0, 0);
            }
            7 => self.modes.autowrap = on,
            12 => self.modes.blink = on,
            25 => {
                self.modes.show_cursor = on;
                self.mark_cursor();
            }
            47 => self.set_alt(on, false),
            66 => self.modes.app_keypad = on,
            1004 => self.modes.focus = on,
            1047 => {
                if !on && self.alt_active {
                    let pen = self.cur.pen;
                    self.alt.clear(&pen);
                }
                self.set_alt(on, false);
            }
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    self.save_cursor();
                    self.set_alt(true, true);
                    // The alternate screen starts with the main screen's
                    // saved cursor.
                    self.saved_alt = self.saved_main;
                } else {
                    self.set_alt(false, false);
                    self.restore_cursor();
                }
            }
            2004 => self.modes.bracketed_paste = on,
            2026 => self.modes.sync = on,
            _ => {}
        }
    }

    /// DECRQM: 1 set, 2 reset, 0 unknown.
    fn mode_state(&self, mode: u16) -> u8 {
        let m = &self.modes;
        let v = match mode {
            1 => m.app_cursor,
            6 => m.origin,
            7 => m.autowrap,
            12 => m.blink,
            25 => m.show_cursor,
            47 | 1047 | 1049 => self.alt_active,
            66 => m.app_keypad,
            1004 => m.focus,
            2004 => m.bracketed_paste,
            2026 => m.sync,
            _ => return 0,
        };
        if v { 1 } else { 2 }
    }

    fn osc(&mut self, params: &[&[u8]]) {
        let Some(&cmd) = params.first() else {
            return;
        };
        let rest = || params[1..].join(&b';');
        match cmd {
            b"0" | b"2" => {
                let t = String::from_utf8_lossy(&rest()).into_owned();
                self.title = t
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(TITLE_MAX)
                    .collect();
                self.events |= event::TITLE;
            }
            b"7" => {
                if let Some(path) = file_url_path(&rest()) {
                    self.cwd = Some(path);
                    self.events |= event::CWD;
                }
            }
            _ => {}
        }
    }
}

/// Reads a 38/48/58 colour: from its own subparams (`38:2::r:g:b`) or from
/// the params after it (`38;5;n`).
fn extended<'a>(p: &[u16], it: &mut impl Iterator<Item = &'a [u16]>) -> Option<Color> {
    let to8 = |v: u16| v.min(255) as u8;
    if p.len() > 1 {
        return match p[1] {
            5 => p.get(2).map(|&n| Color::xterm(to8(n))),
            2 => {
                // With a colour space id the colour starts one later.
                let c = if p.len() >= 6 { &p[3..6] } else { p.get(2..5)? };
                Some(Color::Rgb565(cell::rgb565(to8(c[0]), to8(c[1]), to8(c[2]))))
            }
            _ => None,
        };
    }
    match it.next()?.first()? {
        5 => Some(Color::xterm(to8(*it.next()?.first()?))),
        2 => {
            let r = to8(*it.next()?.first()?);
            let g = to8(*it.next()?.first()?);
            let b = to8(*it.next()?.first()?);
            Some(Color::Rgb565(cell::rgb565(r, g, b)))
        }
        _ => None,
    }
}

/// The path of a `file://host/path` URL, percent-decoded.
fn file_url_path(url: &[u8]) -> Option<String> {
    let rest = url.strip_prefix(b"file://")?;
    let slash = rest.iter().position(|&b| b == b'/')?;
    let mut out = Vec::with_capacity(rest.len());
    let mut i = slash;
    let p = rest;
    while i < p.len() {
        if p[i] == b'%' && i + 2 < p.len() {
            let hex = std::str::from_utf8(&p[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(p[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn dec_graphics(c: char) -> char {
    const TABLE: &[char] = &[
        ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼', '⎺', '⎻',
        '─', '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
    ];
    match c {
        '_'..='~' => TABLE[c as usize - '_' as usize],
        _ => c,
    }
}

/// Param `i`, with 0 or a missing one meaning `default`.
fn arg(params: &Params, i: usize, default: usize) -> usize {
    match params.iter().nth(i).and_then(|p| p.first()) {
        Some(&0) | None => default,
        Some(&n) => n as usize,
    }
}

impl Perform for Terminal {
    fn print(&mut self, c: char) {
        let c = if self.charsets[self.gl] == Charset::Line {
            dec_graphics(c)
        } else {
            c
        };
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.events |= event::BELL,
            0x08 => {
                if self.cur.x > 0 {
                    self.set_x(self.cur.x - 1);
                }
            }
            0x09 => self.tab(1),
            0x0a..=0x0c => {
                if self.modes.newline {
                    self.cur.x = 0;
                }
                self.index();
            }
            0x0d => self.set_x(0),
            0x0e => self.gl = 1,
            0x0f => self.gl = 0,
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, inter: &[u8], _ignore: bool, action: char) {
        let n = |i| arg(params, i, 1);
        match (action, inter) {
            ('@', []) => {
                let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
                self.grid_mut().insert_cells(y, x, n(0), &pen);
            }
            ('A', []) => self.up(n(0)),
            ('B' | 'e', []) => self.down(n(0)),
            ('C' | 'a', []) => self.set_x(self.cur.x + n(0)),
            ('D', []) => self.set_x(self.cur.x.saturating_sub(n(0))),
            ('E', []) => {
                self.down(n(0));
                self.set_x(0);
            }
            ('F', []) => {
                self.up(n(0));
                self.set_x(0);
            }
            ('G' | '`', []) => self.set_x(n(0) - 1),
            ('H' | 'f', []) => self.goto(n(0) - 1, n(1) - 1),
            ('I', []) => self.tab(n(0)),
            ('J', []) => self.erase_display(arg(params, 0, 0) as u16),
            ('K', []) => self.erase_line(arg(params, 0, 0) as u16),
            ('L', []) => self.insert_lines(n(0)),
            ('M', []) => self.delete_lines(n(0)),
            ('P', []) => {
                let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
                self.grid_mut().delete_cells(y, x, n(0), &pen);
            }
            ('S', []) => self.scroll_up(n(0)),
            ('T', []) if params.len() <= 1 => self.scroll_down(n(0)),
            ('X', []) => {
                let (x, y, pen) = (self.cur.x, self.cur.y, self.cur.pen);
                self.grid_mut().erase(y, x, x + n(0), &pen);
            }
            ('Z', []) => self.back_tab(n(0)),
            ('b', []) => {
                if let Some(c) = self.last {
                    for _ in 0..n(0).min(REP_MAX) {
                        self.put(c);
                    }
                }
            }
            ('c', []) => {
                if arg(params, 0, 0) == 0 {
                    self.reply("\x1b[?62;22c");
                }
            }
            ('c', [b'>']) => self.reply("\x1b[>1;10;0c"),
            ('d', []) => self.goto(n(0) - 1, self.cur.x),
            ('g', []) => match arg(params, 0, 0) {
                0 => self.tabs[self.cur.x] = false,
                3 => self.tabs.fill(false),
                _ => {}
            },
            ('h' | 'l', []) => {
                for p in params.iter() {
                    match p.first().copied().unwrap_or(0) {
                        4 => self.modes.insert = action == 'h',
                        20 => self.modes.newline = action == 'h',
                        _ => {}
                    }
                }
            }
            ('h' | 'l', [b'?']) => {
                for p in params.iter() {
                    self.private_mode(p.first().copied().unwrap_or(0), action == 'h');
                }
            }
            ('m', []) => self.sgr(params),
            ('n', []) => match arg(params, 0, 0) {
                5 => self.reply("\x1b[0n"),
                6 => {
                    let y = self.cur.y - if self.modes.origin { self.top } else { 0 };
                    let s = format!("\x1b[{};{}R", y + 1, self.cur.x + 1);
                    self.reply(&s);
                }
                _ => {}
            },
            ('p', [b'!']) => self.soft_reset(),
            ('p', [b'?', b'$']) => {
                let m = arg(params, 0, 0) as u16;
                let s = format!("\x1b[?{};{}$y", m, self.mode_state(m));
                self.reply(&s);
            }
            ('q', [b' ']) => {
                let s = arg(params, 0, 0);
                self.shape = match s {
                    3 | 4 => CursorShape::Underline,
                    5 | 6 => CursorShape::Bar,
                    _ => CursorShape::Block,
                };
                self.modes.blink = s == 0 || s % 2 == 1;
                self.mark_cursor();
            }
            ('r', []) => {
                let top = n(0) - 1;
                let bot = arg(params, 1, self.rows).min(self.rows) - 1;
                if top < bot {
                    self.top = top;
                    self.bot = bot;
                    self.goto(0, 0);
                }
            }
            ('s', []) => self.save_cursor(),
            ('u', []) => self.restore_cursor(),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, inter: &[u8], _ignore: bool, byte: u8) {
        match (inter, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.cur.x = 0;
                self.index();
            }
            ([], b'H') => self.tabs[self.cur.x] = true,
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([], b'=') => self.modes.app_keypad = true,
            ([], b'>') => self.modes.app_keypad = false,
            ([b'('], b) => self.charsets[0] = charset(b),
            ([b')'], b) => self.charsets[1] = charset(b),
            ([b'#'], b'8') => {
                let pen = Pen::DEFAULT;
                let g = self.grid_mut();
                for r in &mut g.rows {
                    r.cells.fill(Cell::new('E', &pen));
                }
                g.mark_all();
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell: bool) {
        self.osc(params);
    }
}

fn charset(b: u8) -> Charset {
    if b == b'0' {
        Charset::Line
    } else {
        Charset::Ascii
    }
}
