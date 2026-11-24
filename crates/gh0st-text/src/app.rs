//! The editor page. It takes the keyboard raw, as the terminal does, and
//! draws only the rows on screen.
//!
//! Two key models, chosen in the menu:
//! - Modeless: always typing. Ctrl and a letter run commands, Shift and an
//!   arrow select. No action bar: every row is text.
//! - View and edit: a file opens in View, where letters move and chips
//!   find, go to a line and save. Enter or `i` edits; Esc goes back.

use crate::buf::save::{self, Saver, Way};
use crate::buf::{Buffer, Lines};
use crate::find::{Finder, Found};
use crate::lex::{self, Cache, Lang, Role, Span};
use crate::view::{self, Glyph};
use crate::warm;
use crate::watch::{Change, FileWatch};
use gh0st_app::RawKey;
use gh0st_app::gfx::color::mix;
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::theme::{self, TermFace};
use gh0st_app::gfx::{Rect, Theme};
use gh0st_app::prelude::*;
use gh0st_core::keys::{Nav, State, code};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Settings, in the shared config.
mod key {
    pub const KEYS: &str = "gh0st.text.keys";
    pub const SIZE: &str = "gh0st.text.size";
    pub const NUMBERS: &str = "gh0st.text.numbers";
}

/// Command ids.
mod cmd {
    pub const SAVE: u32 = 1;
    pub const UNDO: u32 = 2;
    pub const REDO: u32 = 3;
    pub const FIND: u32 = 4;
    pub const GOTO: u32 = 5;
    pub const COPY: u32 = 6;
    pub const CUT: u32 = 7;
    pub const PASTE: u32 = 8;
    pub const ALL: u32 = 9;
    pub const LINE_START: u32 = 10;
    pub const LINE_END: u32 = 11;
    pub const SIZE: u32 = 12;
    pub const NUMBERS: u32 = 13;
    pub const KEYS: u32 = 14;
    pub const DISCARD: u32 = 15;
    pub const EDIT: u32 = 16;
    pub const NEXT: u32 = 17;
    pub const TOP: u32 = 18;
    pub const BOTTOM: u32 = 19;
    pub const REPLACE: u32 = 20;
    pub const UP_DOWN: u32 = 21;
    pub const LEFT_RIGHT: u32 = 22;
    pub const PAGE: u32 = 23;
}

/// Dialog ids.
mod ask {
    pub const QUIT: u32 = 1;
    pub const RESTORE: u32 = 2;
    pub const REPLACE: u32 = 3;
    pub const CHANGED: u32 = 4;
    pub const IN_PLACE: u32 = 5;
}

/// Mode names on the Keys screen.
const EDITING: &str = "Editing";
const VIEWING: &str = "Viewing";

/// Work per slice of the index, find and save: the loop stays quick to
/// keys between slices.
const SLICE_MS: u64 = 8;
/// How long a slice waits for the card to read ahead before trying again.
const DISK_WAIT_MS: u64 = 15;
/// A file under this is saved in one go.
const SAVE_AT_ONCE: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyModel {
    /// Always typing; Ctrl runs commands.
    Modeless,
    /// View first, Enter edits.
    Modal,
}

impl KeyModel {
    fn word(self) -> &'static str {
        match self {
            KeyModel::Modeless => "Always type",
            KeyModel::Modal => "View, then edit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Edit,
    View,
    Find,
    Goto,
}

/// How a page starts, for tests and mockups. The defaults are the app's.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The key model, instead of the setting.
    pub keys: Option<KeyModel>,
    /// The text size, an index into the terminal's faces.
    pub size: Option<usize>,
    /// Bytes per index slice, instead of timing them.
    pub index_budget: Option<u64>,
    /// Asks to restore this many unsaved edits on open.
    pub restore: Option<usize>,
}

struct Doc {
    buf: Buffer,
    cache: Cache,
    spans: Vec<Span>,
    line: Vec<u8>,
}

pub struct Editor {
    opts: Options,
    path: Option<PathBuf>,
    title: String,
    doc: RefCell<Doc>,
    error: Option<String>,
    binary: bool,
    read_only: bool,
    model: KeyModel,
    mode: Mode,
    cursor: u64,
    anchor: Option<u64>,
    /// The column up and down keep to.
    want: Option<u32>,
    /// The first line on screen, by offset.
    top: u64,
    /// The first column on screen.
    pan: u32,
    size: usize,
    numbers: bool,
    /// The undo state the file on disk has.
    saved: u64,
    area: Rect,
    index: Option<Timer>,
    budget: u64,
    finder: Option<(Finder, Timer)>,
    saving: Option<(Saver, Timer)>,
    search: Search,
    /// The query of the last find, for find next.
    needle: Vec<u8>,
    /// The match on screen.
    hit: Option<(u64, u64)>,
    clip: Vec<u8>,
    watch: Option<(FileWatch, Watch)>,
    context: String,
    /// The reader bringing a big file into memory (`warm`).
    warm: Option<i32>,
}

impl Editor {
    /// The file at `path`, or a new one there if it doesn't exist.
    pub fn open(path: Option<&Path>, opts: Options) -> Editor {
        let (buf, error) = match path {
            Some(p) => match Buffer::open(p) {
                Ok(b) => (b, None),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    (Buffer::from_bytes(b""), None)
                }
                Err(e) => (Buffer::from_bytes(b""), Some(e.to_string())),
            },
            None => (Buffer::from_bytes(b""), None),
        };
        let mut buf = buf;
        let binary = buf.looks_binary().unwrap_or(false);
        let first = {
            let end = buf.line_end(0).unwrap_or(0).min(256);
            buf.bytes(0..end).unwrap_or_default()
        };
        let lang = path.map_or(Lang::Plain, |p| lex::detect(p, &first));
        let read_only = binary
            || path.is_some_and(|p| p.exists() && save::way(p).is_ok_and(|w| w == Way::ReadOnly));
        let title = path
            .and_then(|p| p.file_name())
            .map_or("Text".into(), |n| n.to_string_lossy().into_owned());
        Editor {
            opts,
            path: path.map(Path::to_path_buf),
            title,
            doc: RefCell::new(Doc {
                buf,
                cache: Cache::new(lang),
                spans: Vec::new(),
                line: Vec::new(),
            }),
            error,
            binary,
            read_only,
            model: KeyModel::Modeless,
            mode: Mode::Edit,
            cursor: 0,
            anchor: None,
            want: None,
            top: 0,
            pan: 0,
            size: 0,
            numbers: true,
            saved: 0,
            area: Rect::new(0, 18, 320, 152),
            index: None,
            budget: 1 << 20,
            finder: None,
            saving: None,
            search: Search::new(),
            needle: Vec::new(),
            hit: None,
            clip: Vec::new(),
            watch: None,
            context: String::new(),
            warm: None,
        }
    }

    /// Starts a reader of the file from `from`, for big files: the index,
    /// find and save then never wait for the card.
    fn warm_from(&mut self, from: u64, cx: &mut Cx) {
        let Some(p) = self.path.clone() else {
            return;
        };
        let len = self.doc.borrow().buf.len();
        if self.warm.is_some() || len.saturating_sub(from) < warm::MIN || self.binary {
            return;
        }
        if let Ok(pid) = warm::spawn(&p, from) {
            cx.adopt(pid);
            self.warm = Some(pid);
            self.doc.borrow_mut().buf.set_warming(true);
        }
    }

    fn stop_warm(&mut self) {
        if let Some(pid) = self.warm.take() {
            warm::stop(pid);
            self.doc.borrow_mut().buf.set_warming(false);
        }
    }

    fn dirty(&self) -> bool {
        self.doc.borrow().buf.state() != self.saved
    }

    fn look(&self) -> Look {
        Look::of(&theme::TERM_TERMINUS[self.size.min(theme::TERM_TERMINUS.len() - 1)])
    }

    fn rows(&self) -> usize {
        (self.area.h / self.look().ch).max(1) as usize
    }

    fn lines(&self) -> Lines {
        self.doc.borrow().buf.lines()
    }

    fn gutter_cells(&self) -> u32 {
        if !self.numbers || self.binary {
            return 0;
        }
        let n = self.lines().count.max(999);
        n.ilog10() + 2
    }

    fn cols(&self) -> u32 {
        let look = self.look();
        let w = self.area.w - self.gutter_cells() as i32 * look.cw - theme::TEXT_GUTTER_GAP;
        (w / look.cw).max(1) as u32
    }

    /// The selection, start to end, if any.
    fn selection(&self) -> Option<(u64, u64)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }

    /// The line holding `off`: its start, end and bytes up to the lex limit.
    fn line_at(&self, off: u64) -> (u64, u64, Vec<u8>) {
        let mut d = self.doc.borrow_mut();
        let start = d.buf.line_start(off).unwrap_or(0);
        let end = d.buf.line_end(off).unwrap_or(start);
        let take = (end - start).min(lex::LINE_MAX as u64 * 4);
        let bytes = d.buf.bytes(start..start + take).unwrap_or_default();
        (start, end, bytes)
    }

    fn col_of(&self, off: u64) -> u32 {
        let (start, _, line) = self.line_at(off);
        view::col_of(&line, (off - start) as usize)
    }

    // Moving.

    fn go(&mut self, to: u64, select: bool) {
        if select {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.doc.borrow().buf.len());
        self.doc.borrow_mut().buf.seal();
    }

    fn left(&mut self, select: bool, word: bool) {
        if let (false, Some((a, _))) = (select, self.selection()) {
            self.go(a, false);
            return;
        }
        let (start, _, line) = self.line_at(self.cursor);
        let at = (self.cursor - start) as usize;
        let to = if at == 0 {
            self.cursor.saturating_sub(1)
        } else if word {
            start + view::word_left(&line, at) as u64
        } else {
            start + view::prev_char(&line, at) as u64
        };
        self.want = None;
        self.go(to, select);
    }

    fn right(&mut self, select: bool, word: bool) {
        if let (false, Some((_, b))) = (select, self.selection()) {
            self.go(b, false);
            return;
        }
        let (start, end, line) = self.line_at(self.cursor);
        let at = (self.cursor - start) as usize;
        let to = if self.cursor >= end {
            self.cursor + 1
        } else if word {
            start + view::word_right(&line, at) as u64
        } else {
            start + view::next_char(&line, at) as u64
        };
        self.want = None;
        self.go(to, select);
    }

    fn vertical(&mut self, lines: i64, select: bool) {
        let col = match self.want {
            Some(c) => c,
            None => self.col_of(self.cursor),
        };
        let mut at = self.cursor;
        let len = self.doc.borrow().buf.len();
        for _ in 0..lines.unsigned_abs() {
            let (start, end, _) = self.line_at(at);
            if lines < 0 {
                if start == 0 {
                    break;
                }
                at = start - 1;
            } else {
                if end >= len {
                    break;
                }
                at = end + 1;
            }
        }
        let (start, _, line) = self.line_at(at);
        let to = start + view::byte_at(&line, col) as u64;
        self.go(to, select);
        self.want = Some(col);
    }

    fn line_edge(&mut self, end: bool, select: bool) {
        let (start, stop, line) = self.line_at(self.cursor);
        let to = if end {
            stop
        } else {
            // First to the indent, then to the very start.
            let ind = start + view::indent(&line).len() as u64;
            if self.cursor == ind { start } else { ind }
        };
        self.want = None;
        self.go(to, select);
    }

    /// Keeps the cursor on screen.
    fn reveal(&mut self) {
        let rows = self.rows() as u64;
        if self.binary {
            let per = self.hex_per();
            let row = self.cursor / per;
            let top = self.top / per;
            if row < top {
                self.top = row * per;
            } else if row >= top + rows {
                self.top = (row + 1 - rows) * per;
            }
            return;
        }
        let mut d = self.doc.borrow_mut();
        let cur = d.buf.line_start(self.cursor).unwrap_or(0);
        if cur < self.top {
            self.top = cur;
        } else {
            // Is it within `rows` lines of the top?
            let mut at = self.top;
            let mut n = 0;
            let len = d.buf.len();
            while at < cur && n < rows {
                at = d.buf.line_end(at).unwrap_or(len) + 1;
                n += 1;
            }
            if at < cur || n >= rows {
                // Put it on the last row.
                let mut top = cur;
                for _ in 0..rows - 1 {
                    if top == 0 {
                        break;
                    }
                    top = d.buf.line_start(top - 1).unwrap_or(0);
                }
                self.top = top;
            }
        }
        drop(d);
        let col = self.col_of(self.cursor);
        let cols = self.cols();
        if col < self.pan {
            self.pan = col.saturating_sub(cols / 4);
        } else if col >= self.pan + cols {
            self.pan = col + cols / 4 - cols + 1;
        }
    }

    // Editing.

    fn type_bytes(&mut self, bytes: &[u8]) {
        if self.read_only {
            return;
        }
        let mut d = self.doc.borrow_mut();
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        if d.buf.replace(a..b, bytes).is_ok() {
            d.cache.edited(a);
            self.cursor = a + bytes.len() as u64;
        }
        self.anchor = None;
        self.want = None;
        self.hit = None;
    }

    fn newline(&mut self) {
        let (start, _, line) = self.line_at(self.cursor);
        let at = (self.cursor - start) as usize;
        let mut ins = b"\n".to_vec();
        ins.extend_from_slice(view::indent(&line[..at.min(line.len())]));
        self.doc.borrow_mut().buf.seal();
        self.type_bytes(&ins);
    }

    fn backspace(&mut self, forward: bool) {
        if self.read_only {
            return;
        }
        let (a, b) = match self.selection() {
            Some(s) => s,
            None => {
                let (start, end, line) = self.line_at(self.cursor);
                let at = (self.cursor - start) as usize;
                if forward {
                    let to = if self.cursor >= end {
                        self.cursor + 1
                    } else {
                        start + view::next_char(&line, at) as u64
                    };
                    (self.cursor, to.min(self.doc.borrow().buf.len()))
                } else if at == 0 {
                    (self.cursor.saturating_sub(1), self.cursor)
                } else {
                    (start + view::prev_char(&line, at) as u64, self.cursor)
                }
            }
        };
        if a == b {
            return;
        }
        let mut d = self.doc.borrow_mut();
        if d.buf.delete(a..b).is_ok() {
            d.cache.edited(a);
            self.cursor = a;
        }
        self.anchor = None;
        self.hit = None;
    }

    fn copy(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            return false;
        };
        let bytes = self.doc.borrow_mut().buf.bytes(a..b.min(a + (16 << 20)));
        self.clip = bytes.unwrap_or_default();
        true
    }

    fn undo(&mut self, redo: bool) {
        let mut d = self.doc.borrow_mut();
        let at = if redo { d.buf.redo() } else { d.buf.undo() };
        if let Some(at) = at {
            let lang = d.cache.lang();
            d.cache.clear(lang);
            self.cursor = at.min(d.buf.len());
        }
        self.anchor = None;
        self.hit = None;
    }

    // Find.

    fn find_next(&mut self, cx: &mut Cx) {
        if self.needle.is_empty() {
            return;
        }
        let from = self.hit.map_or(self.cursor, |(a, _)| a + 1);
        let f = Finder::new(&self.needle, from, true);
        self.warm_from(from, cx);
        if let Some((_, t)) = self.finder.take() {
            cx.cancel(t);
        }
        self.finder = Some((f, cx.after(0)));
    }

    fn find_step(&mut self, cx: &mut Cx) {
        let Some((mut f, _)) = self.finder.take() else {
            return;
        };
        let t0 = Instant::now();
        loop {
            let r = f.step(&mut self.doc.borrow_mut().buf, 1 << 20);
            match r {
                Ok(Found::Hit(at)) => {
                    let n = f.needle().len() as u64;
                    self.hit = Some((at, at + n));
                    self.cursor = at;
                    self.anchor = None;
                    self.reveal();
                    cx.idle();
                    return;
                }
                Ok(Found::Miss) => {
                    self.hit = None;
                    cx.idle();
                    cx.toast("Not found");
                    return;
                }
                Ok(Found::More) if (t0.elapsed().as_millis() as u64) < SLICE_MS => {}
                Ok(Found::More) => {
                    cx.busy("Finding");
                    self.finder = Some((f, cx.after(0)));
                    return;
                }
                Ok(Found::Wait) => {
                    cx.busy("Finding");
                    self.finder = Some((f, cx.after(DISK_WAIT_MS)));
                    return;
                }
                Err(e) => {
                    cx.idle();
                    cx.toast_tone(Tone::Critical, &e.to_string());
                    return;
                }
            }
        }
    }

    fn goto_line(&mut self, n: u64) -> bool {
        let at = self.doc.borrow_mut().buf.line_offset(n.saturating_sub(1));
        match at {
            Ok(Some(at)) => {
                self.go(at, false);
                true
            }
            _ => false,
        }
    }

    // Files.

    fn save(&mut self, cx: &mut Cx) {
        let Some(path) = self.path.clone() else {
            cx.toast("Save as comes with the file picker");
            return;
        };
        if self.read_only {
            cx.toast_tone(Tone::Warning, "Read only");
            return;
        }
        match save::way(&path) {
            Ok(Way::Replace) => self.start_save(&path, Way::Replace, cx),
            Ok(Way::InPlace) => cx.ask(
                ask::IN_PLACE,
                Dialog::confirm(
                    "Save over the file?",
                    "It has other links or its folder is read only. A crash while saving loses it.",
                )
                .yes("save"),
            ),
            Ok(Way::ReadOnly) => cx.toast_tone(Tone::Warning, "Read only"),
            Err(e) => cx.toast_tone(Tone::Critical, &e.to_string()),
        }
    }

    fn start_save(&mut self, path: &Path, how: Way, cx: &mut Cx) {
        let started = Saver::start(&mut self.doc.borrow_mut().buf, path, how);
        match started {
            Ok(s) => {
                let big = self.doc.borrow().buf.len() > SAVE_AT_ONCE;
                if big {
                    cx.busy("Saving");
                    self.warm_from(0, cx);
                }
                self.saving = Some((s, cx.after(0)));
            }
            Err(e) => cx.toast_tone(Tone::Critical, &e.to_string()),
        }
    }

    fn save_step(&mut self, cx: &mut Cx) {
        let Some((mut s, _)) = self.saving.take() else {
            return;
        };
        let t0 = Instant::now();
        loop {
            let r = s.step(&mut self.doc.borrow_mut().buf, 1 << 20);
            match r {
                Ok(true) => {
                    self.saved = self.doc.borrow().buf.state();
                    if let Some((w, _)) = &mut self.watch {
                        w.saved();
                    }
                    cx.idle();
                    cx.toast_tone(Tone::Ok, "Saved");
                    return;
                }
                Ok(false) if (t0.elapsed().as_millis() as u64) < SLICE_MS => {}
                Ok(false) => {
                    self.saving = Some((s, cx.after(0)));
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.saving = Some((s, cx.after(DISK_WAIT_MS)));
                    return;
                }
                Err(e) => {
                    cx.idle();
                    cx.toast_tone(Tone::Critical, &e.to_string());
                    return;
                }
            }
        }
    }

    fn index_step(&mut self, cx: &mut Cx) {
        self.index = None;
        let budget = self.opts.index_budget.unwrap_or(self.budget);
        let t0 = Instant::now();
        let done = self.doc.borrow_mut().buf.index_step(budget);
        let ms = t0.elapsed().as_micros().max(1) as u64;
        let waiting = matches!(&done, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock);
        if self.opts.index_budget.is_none() && !waiting {
            // Aim each slice at SLICE_MS.
            self.budget = (self.budget * SLICE_MS * 1000 / ms).clamp(64 << 10, 256 << 20);
        }
        match done {
            // A millisecond apart, so keys and frames come between slices.
            Ok(false) => self.index = Some(cx.after(1)),
            Ok(true) => {}
            // The card is reading ahead: come back when it has had time.
            Err(_) if waiting => self.index = Some(cx.after(DISK_WAIT_MS)),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The top bar's position: line and column, and the index's progress.
    fn refresh(&mut self) {
        let lines = self.lines();
        let len = self.doc.borrow().buf.len();
        let line = self
            .doc
            .borrow_mut()
            .buf
            .line_of(self.cursor)
            .ok()
            .flatten();
        let col = self.col_of(self.cursor) + 1;
        let mut s = match (self.binary, line) {
            (true, _) => format!("{:x}", self.cursor),
            (false, Some(l)) => format!("{}:{col}", l + 1),
            (false, None) => format!("?:{col}"),
        };
        if !lines.exact && len > 0 {
            let done = self.doc.borrow().buf.indexed();
            s.push_str(&format!(" {}%", done * 100 / len));
        }
        self.context = s;
    }

    /// After every event: the bars may have changed, the cursor moved.
    fn settle(&mut self, cx: &mut Cx) {
        let t = *cx.theme();
        self.area = self.bars().content(&t);
        self.reveal();
        self.refresh();
    }

    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        if m != Mode::Find && m != Mode::Goto {
            self.search.cancel();
        }
    }

    fn leave_query(&mut self) {
        self.set_mode(if self.model == KeyModel::Modal {
            Mode::View
        } else {
            Mode::Edit
        });
    }

    fn raw_key(&mut self, k: &RawKey, cx: &mut Cx) {
        if k.state == State::Release {
            return;
        }
        if self.binary && !matches!(self.mode, Mode::Find | Mode::Goto) {
            return self.hex_key(k);
        }
        match self.mode {
            Mode::Find | Mode::Goto => self.query_key(k, cx),
            Mode::View => {}
            Mode::Edit => self.edit_key(k),
        }
    }

    fn query_key(&mut self, k: &RawKey, cx: &mut Cx) {
        match k.code {
            code::ENTER => {
                let q = self.search.query().to_string();
                if self.mode == Mode::Goto {
                    match q.trim().parse::<u64>() {
                        Ok(n) if self.goto_line(n) => self.leave_query(),
                        _ => cx.toast("No such line yet"),
                    }
                } else {
                    // A new query starts at the cursor; the same one goes on.
                    if self.needle != q.as_bytes() {
                        self.needle = q.into_bytes();
                        self.hit = None;
                    }
                    self.find_next(cx);
                }
            }
            code::BACKSPACE => {
                self.search.key(Key::Backspace);
            }
            _ => {
                if let Some(c) = k.ch.filter(|c| !c.is_control()) {
                    self.search.key(Key::Char(c));
                }
            }
        }
    }

    /// Bytes on a row of the hex view.
    fn hex_per(&self) -> u64 {
        hex_per((self.area.w / self.look().cw) as usize) as u64
    }

    fn hex_key(&mut self, k: &RawKey) {
        let per = self.hex_per() as i64;
        let page = per * self.rows() as i64;
        let by = match (k.nav(true), layer(k)) {
            (Some(Nav::Up), _) => -per,
            (Some(Nav::Down), _) => per,
            (Some(Nav::Left), _) => -1,
            (Some(Nav::Right), _) => 1,
            (_, code::PAGEUP) => -page,
            (_, code::PAGEDOWN) => page,
            _ => 0,
        };
        self.hex_move(by);
    }

    fn hex_move(&mut self, by: i64) {
        let last = self.doc.borrow().buf.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(by).min(last);
    }

    /// A move from View's keys: up and down by `lines`, or across by one.
    fn view_move(&mut self, id: u32, step: Step) {
        let back = step == Step::Prev;
        let rows = self.rows() as i64 - 1;
        if self.binary {
            let per = self.hex_per() as i64;
            let by = match id {
                cmd::UP_DOWN => per,
                cmd::PAGE => per * rows,
                _ => 1,
            };
            return self.hex_move(if back { -by } else { by });
        }
        match id {
            cmd::UP_DOWN => self.vertical(if back { -1 } else { 1 }, false),
            cmd::PAGE => self.vertical(if back { -rows } else { rows }, false),
            _ if back => self.left(false, false),
            _ => self.right(false, false),
        }
    }

    fn edit_key(&mut self, k: &RawKey) {
        let sel = k.shift;
        match k.nav(false) {
            Some(Nav::Up) => return self.vertical(-1, sel),
            Some(Nav::Down) => return self.vertical(1, sel),
            Some(Nav::Left) => return self.left(sel, k.ctrl),
            Some(Nav::Right) => return self.right(sel, k.ctrl),
            None => {}
        }
        match layer(k) {
            code::PAGEUP => self.vertical(-(self.rows() as i64 - 1), sel),
            code::PAGEDOWN => self.vertical(self.rows() as i64 - 1, sel),
            code::END => self.line_edge(true, sel),
            _ if self.read_only => {}
            code::ENTER => self.newline(),
            code::BACKSPACE => self.backspace(false),
            code::DELETE => self.backspace(true),
            code::TAB => self.type_bytes(b"    "),
            // Fn and a key that means nothing here types nothing.
            _ if k.fn_ => {}
            _ => {
                if let Some(c) = k.ch.filter(|c| !c.is_control()) {
                    let mut b = [0u8; 4];
                    self.type_bytes(c.encode_utf8(&mut b).as_bytes());
                }
            }
        }
    }

    fn run(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let edit_ok = !self.read_only;
        match id {
            cmd::SAVE => self.save(cx),
            cmd::UNDO if edit_ok => self.undo(false),
            cmd::REDO if edit_ok => self.undo(true),
            cmd::FIND | cmd::REPLACE => {
                self.set_mode(Mode::Find);
                self.search.start();
                if id == cmd::REPLACE && !self.needle.is_empty() {
                    cx.ask(
                        ask::REPLACE,
                        Dialog::text(
                            &format!(
                                "Replace \u{201c}{}\u{201d}",
                                String::from_utf8_lossy(&self.needle)
                            ),
                            "with",
                        )
                        .yes("replace"),
                    );
                }
            }
            cmd::NEXT => self.find_next(cx),
            cmd::GOTO => {
                self.set_mode(Mode::Goto);
                self.search.start();
            }
            cmd::COPY => {
                if self.copy() {
                    cx.toast("Copied");
                }
            }
            cmd::CUT if edit_ok => {
                if self.copy() {
                    self.backspace(false);
                }
            }
            cmd::PASTE if edit_ok => {
                let clip = self.clip.clone();
                self.doc.borrow_mut().buf.seal();
                self.type_bytes(&clip);
                self.doc.borrow_mut().buf.seal();
            }
            cmd::ALL => {
                self.anchor = Some(0);
                self.cursor = self.doc.borrow().buf.len();
            }
            cmd::LINE_START => self.line_edge(false, false),
            cmd::LINE_END => self.line_edge(true, false),
            cmd::TOP => self.go(0, false),
            cmd::BOTTOM => {
                let len = self.doc.borrow().buf.len();
                self.go(len, false);
            }
            cmd::SIZE => {
                let n = theme::TERM_TERMINUS.len();
                self.size = match step {
                    Step::Prev => self.size.saturating_sub(1),
                    _ => (self.size + 1).min(n - 1),
                };
                cx.settings().set_int(key::SIZE, self.size as i64);
            }
            cmd::NUMBERS => {
                self.numbers = !self.numbers;
                cx.settings().set_bool(key::NUMBERS, self.numbers);
            }
            cmd::KEYS => {
                self.model = match self.model {
                    KeyModel::Modeless => KeyModel::Modal,
                    KeyModel::Modal => KeyModel::Modeless,
                };
                cx.settings().set_str(
                    key::KEYS,
                    if self.model == KeyModel::Modal {
                        "modal"
                    } else {
                        "modeless"
                    },
                );
                self.leave_query();
            }
            cmd::UP_DOWN | cmd::LEFT_RIGHT | cmd::PAGE => self.view_move(id, step),
            cmd::DISCARD => cx.quit(),
            cmd::EDIT if edit_ok => self.set_mode(Mode::Edit),
            _ => {}
        }
    }
}

/// The key a press means, with Fn and a letter as the key it stands for:
/// the device sends Fn+L as PgUp, a script sends Fn and L.
fn layer(k: &RawKey) -> u16 {
    if !k.fn_ {
        return k.code;
    }
    match gh0st_core::keys::base_char(k.code) {
        Some('l') => code::PAGEUP,
        Some('m') => code::PAGEDOWN,
        Some('n') => code::END,
        _ if k.code == code::BACKSPACE => code::DELETE,
        _ => k.code,
    }
}

/// A face and the cell it sets in.
#[derive(Clone, Copy)]
struct Look {
    font: Font,
    bold: Font,
    cw: i32,
    ch: i32,
    base: i32,
}

impl Look {
    fn of(face: &TermFace) -> Look {
        let font = face.regular;
        let ch = font.line.max(1);
        Look {
            font,
            bold: face.bold.unwrap_or(font),
            cw: font.measure("M").max(1),
            ch,
            base: font.ascent + (ch - font.ascent + font.descent) / 2,
        }
    }
}

impl Page for Editor {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> &str {
        "file-code"
    }

    fn context(&self) -> Option<&str> {
        Some(&self.context)
    }

    fn status(&self) -> Option<(&str, Tone)> {
        if self.binary {
            Some(("Hex", Tone::Info))
        } else if self.read_only {
            Some(("Read only", Tone::Info))
        } else if self.dirty() {
            Some(("Edited", Tone::Warning))
        } else {
            None
        }
    }

    fn about(&self) -> &str {
        "Text opens files of any size: only what is on screen is read. \
         Changes stay in memory until you save."
    }

    fn bars(&self) -> Bars {
        match (self.model, self.mode) {
            (_, Mode::Find | Mode::Goto) => Bars::top().search(&self.search),
            (KeyModel::Modeless, _) => Bars::top_only(),
            (KeyModel::Modal, Mode::View) => Bars::top(),
            (KeyModel::Modal, _) => {
                Bars::top().hints([("Esc", "view"), ("Ctrl S", "save"), ("Fn H", "keys")])
            }
        }
    }

    /// A file another app opened here while this one runs.
    fn opened(&mut self, paths: &[PathBuf], cx: &mut Cx) {
        let Some(p) = paths.first() else {
            return;
        };
        if self.path.as_deref() == Some(p.as_path()) {
            return;
        }
        if self.dirty() || self.saving.is_some() {
            cx.toast("Save this file first, then open the other again");
            return;
        }
        // Let go of this file: its index, find, watch.
        if let Some(t) = self.index.take() {
            cx.cancel(t);
        }
        if let Some((_, t)) = self.finder.take() {
            cx.cancel(t);
        }
        if let Some((_, w)) = self.watch.take() {
            cx.unwatch(w);
        }
        *self = Editor::open(Some(p), self.opts.clone());
        self.start(cx);
        cx.redraw();
    }

    /// The reader ended: the file is in memory, or it gave up. Reads wait
    /// for the disk again, so whatever is left still gets read.
    fn exited(&mut self, pid: i32, _code: Option<i32>, cx: &mut Cx) {
        if self.warm == Some(pid) {
            self.warm = None;
            self.doc.borrow_mut().buf.set_warming(false);
        }
        let _ = cx;
    }

    fn stop(&mut self, _cx: &mut Cx) {
        self.stop_warm();
    }

    fn start(&mut self, cx: &mut Cx) {
        let s = cx.settings();
        self.model = match self.opts.keys {
            Some(m) => m,
            None if s.get_str(key::KEYS) == Some("modal") => KeyModel::Modal,
            None => KeyModel::Modeless,
        };
        self.size = self
            .opts
            .size
            .or_else(|| s.get_int(key::SIZE).map(|n| n.max(0) as usize))
            .unwrap_or(0)
            .min(theme::TERM_TERMINUS.len() - 1);
        self.numbers = s.get_bool(key::NUMBERS).unwrap_or(true);
        self.mode = if self.model == KeyModel::Modal {
            Mode::View
        } else {
            Mode::Edit
        };
        if !self.doc.borrow().buf.lines().exact {
            self.warm_from(0, cx);
            self.index = Some(cx.after(1));
        }
        if let Some(p) = &self.path {
            let w = FileWatch::new(p);
            if let Some(fd) = w.fd() {
                let id = cx.watch(fd);
                self.watch = Some((w, id));
            }
        }
        if let Some(e) = self.error.clone() {
            cx.toast_tone(Tone::Critical, &e);
        }
        if let Some(n) = self.opts.restore {
            cx.ask(
                ask::RESTORE,
                Dialog::confirm(
                    "Restore unsaved edits?",
                    &format!("{} closed with {n} edits not saved.", self.title),
                )
                .yes("restore")
                .no("discard"),
            );
        }
        self.settle(cx);
    }

    fn timer(&mut self, t: Timer, cx: &mut Cx) {
        if self.index == Some(t) {
            self.index_step(cx);
        } else if self.finder.as_ref().is_some_and(|f| f.1 == t) {
            self.find_step(cx);
        } else if self.saving.as_ref().is_some_and(|s| s.1 == t) {
            self.save_step(cx);
        }
        self.settle(cx);
    }

    fn ready(&mut self, w: Watch, cx: &mut Cx) {
        let Some((fw, id)) = &mut self.watch else {
            return;
        };
        if *id != w {
            return;
        }
        match fw.check() {
            Some(Change::Gone) => cx.toast_tone(Tone::Warning, "The file was deleted or moved"),
            Some(Change::Modified) if !self.dirty() => {
                if let Some(p) = self.path.clone() {
                    let mut fresh = Editor::open(Some(&p), self.opts.clone());
                    fresh.model = self.model;
                    fresh.size = self.size;
                    fresh.numbers = self.numbers;
                    fresh.mode = self.mode;
                    fresh.cursor = self.cursor;
                    fresh.top = self.top;
                    fresh.watch = self.watch.take();
                    *self = fresh;
                    if !self.doc.borrow().buf.lines().exact {
                        self.index = Some(cx.after(0));
                    }
                    cx.toast("Reloaded: changed on disk");
                }
            }
            Some(Change::Modified) => cx.ask(
                ask::CHANGED,
                Dialog::confirm(
                    "Changed on disk",
                    "Another program changed this file. Your edits stay until you save over it.",
                )
                .yes("ok"),
            ),
            None => return,
        }
        self.settle(cx);
        cx.redraw();
    }

    /// Raw while typing. View takes keys as a list does, so its chips and
    /// the menu on 8 work.
    fn raw_keys(&self) -> bool {
        self.mode != Mode::View
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter && !self.read_only {
            self.set_mode(Mode::Edit);
        }
        self.settle(cx);
    }

    fn typing(&self) -> bool {
        matches!(self.mode, Mode::Edit | Mode::Find | Mode::Goto)
    }

    fn raw(&mut self, k: &RawKey, cx: &mut Cx) {
        self.raw_key(k, cx);
        self.settle(cx);
        cx.redraw();
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        if matches!(self.mode, Mode::Find | Mode::Goto) {
            if let Some((_, t)) = self.finder.take() {
                cx.cancel(t);
                cx.idle();
            }
            self.leave_query();
        } else if self.anchor.is_some() {
            self.anchor = None;
        } else if self.model == KeyModel::Modal && self.mode == Mode::Edit {
            self.set_mode(Mode::View);
        } else if self.dirty() {
            cx.ask(
                ask::QUIT,
                Dialog::confirm(
                    &format!("Save {}?", self.title),
                    "Esc keeps editing. To close without saving, use the menu.",
                )
                .yes("save"),
            );
        } else {
            return true;
        }
        self.settle(cx);
        false
    }

    fn answer(&mut self, id: u32, a: Answer, cx: &mut Cx) {
        match (id, a) {
            (ask::QUIT, Answer::Yes(_)) => {
                self.save(cx);
                if self.saving.is_some() {
                    while self.saving.is_some() {
                        self.save_step(cx);
                    }
                    if !self.dirty() {
                        cx.quit();
                    }
                }
            }
            (ask::IN_PLACE, Answer::Yes(_)) => {
                if let Some(p) = self.path.clone() {
                    self.start_save(&p, Way::InPlace, cx);
                }
            }
            (ask::REPLACE, Answer::Yes(with)) => {
                if let Some((a, b)) = self.hit {
                    self.anchor = Some(a);
                    self.cursor = b;
                    self.type_bytes(with.as_bytes());
                    self.find_next(cx);
                }
            }
            _ => {}
        }
        self.settle(cx);
    }

    fn commands(&self, c: &mut Commands) {
        let modal = self.model == KeyModel::Modal;
        let view = self.mode == Mode::View;
        let edit = self.mode == Mode::Edit;
        let can_edit = !self.read_only;
        let mode = if view { VIEWING } else { EDITING };
        let here = |c: Command| c.mode(mode, view || edit);
        c.group("File");
        c.add(here(
            Command::action(cmd::SAVE, "Save")
                .keys("Ctrl+s")
                .enabled(can_edit)
                .chip(7, "file", "Save"),
        ));
        c.add(here(
            Command::action(cmd::FIND, "Find")
                .keys(if modal && view { "/ Ctrl+f" } else { "Ctrl+f" })
                .chip(5, "search", "Find"),
        ));
        c.add(here(
            Command::action(cmd::REPLACE, "Replace")
                .keys("Ctrl+r")
                .enabled(can_edit),
        ));
        c.add(here(
            Command::action(cmd::GOTO, "Go to line")
                .keys("Ctrl+g")
                .chip(6, "list", "Line"),
        ));
        c.add(here(Command::action(cmd::NEXT, "Find next").keys("Ctrl+n")));
        c.group("Edit");
        c.add(here(
            Command::action(cmd::UNDO, "Undo")
                .keys("Ctrl+z")
                .enabled(can_edit),
        ));
        c.add(here(
            Command::action(cmd::REDO, "Redo")
                .keys("Ctrl+y")
                .enabled(can_edit),
        ));
        c.add(here(Command::action(cmd::COPY, "Copy").keys("Ctrl+c")));
        c.add(here(
            Command::action(cmd::CUT, "Cut")
                .keys("Ctrl+x")
                .enabled(can_edit),
        ));
        c.add(here(
            Command::action(cmd::PASTE, "Paste")
                .keys("Ctrl+v")
                .enabled(can_edit && !self.clip.is_empty()),
        ));
        c.add(here(Command::action(cmd::ALL, "Select all")));
        c.add(here(
            Command::action(cmd::LINE_START, "Line start")
                .keys("Ctrl+a")
                .keys_only(),
        ));
        c.add(here(
            Command::action(cmd::LINE_END, "Line end")
                .keys("Ctrl+e End")
                .keys_only(),
        ));
        c.add(here(
            Command::action(cmd::TOP, "Top of file")
                .keys(if view { "g" } else { "Ctrl+Up" })
                .keys_only(),
        ));
        c.add(here(
            Command::action(cmd::BOTTOM, "End of file")
                .keys(if view { "G" } else { "Ctrl+Down" })
                .keys_only(),
        ));
        c.add(
            Command::action(cmd::EDIT, "Edit")
                .keys("i")
                .keys_only()
                .mode(VIEWING, view && can_edit),
        );
        let moves = |c: Command| c.keys_only().repeats().mode(VIEWING, view);
        c.add(moves(
            Command::choice(cmd::UP_DOWN, "Up, down", "").keys("f x"),
        ));
        c.add(moves(
            Command::choice(cmd::LEFT_RIGHT, "Left, right", "").keys("z c"),
        ));
        c.add(moves(
            Command::choice(cmd::PAGE, "Page up, down", "").keys("PgUp PgDn"),
        ));
        c.group("View");
        let face = &theme::TERM_TERMINUS[self.size.min(theme::TERM_TERMINUS.len() - 1)];
        let l = Look::of(face);
        c.add(
            Command::choice(cmd::SIZE, "Text size", format!("{}x{}", l.cw, l.ch))
                .keys("Fn+r Fn+t")
                .repeats(),
        );
        c.add(Command::toggle(cmd::NUMBERS, "Line numbers", self.numbers));
        c.add(Command::choice(cmd::KEYS, "Keys", self.model.word()));
        c.group("Close");
        c.add(
            Command::action(cmd::DISCARD, "Close without saving")
                .confirm()
                .enabled(self.dirty()),
        );
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        self.run(id, step, cx);
        self.settle(cx);
    }

    fn frame_ms(&self) -> u64 {
        if self.index.is_some() { 20 } else { 0 }
    }

    fn draw(&self, ui: &mut Ui) {
        let t: Theme = *ui.theme();
        let area = ui.area();
        let look = self.look();
        let gutter = self.gutter_cells() as i32;
        let cols = self.cols();
        let rows = self.rows();
        let mut d = self.doc.borrow_mut();
        let d = &mut *d;
        if self.binary {
            draw_hex(ui, &t, area, look, &mut d.buf, self.top, self.cursor);
            return;
        }
        let x0 = area.x
            + gutter * look.cw
            + if gutter > 0 {
                theme::TEXT_GUTTER_GAP
            } else {
                0
            };
        let len = d.buf.len();
        let first_line = d.buf.line_of(self.top).ok().flatten();
        let mut state = d.cache.state_at(&mut d.buf, self.top).unwrap_or(0);
        let lang = d.cache.lang();
        let sel = self.selection();
        let line_bg = mix(t.c.bg, t.c.surface, theme::TEXT_LINE);
        let sel_bg = mix(t.c.bg, t.c.accent, theme::TEXT_SELECT);
        let hit_bg = mix(t.c.bg, t.c.warning, theme::TEXT_MATCH);
        let c = ui.canvas();
        let mut start = self.top;
        let mut run = String::new();
        for r in 0..rows {
            if start > len {
                break;
            }
            let y = area.y + r as i32 * look.ch;
            let end = d.buf.line_end(start).unwrap_or(len);
            let take = (end - start).min(lex::LINE_MAX as u64);
            d.line.clear();
            let line = &mut d.line;
            let _ = d.buf.for_each(start..start + take, |ch| {
                line.extend_from_slice(ch);
                true
            });
            if d.line.last() == Some(&b'\r') {
                d.line.pop();
            }
            state = lex::lex(lang, state, &d.line, &mut d.spans);
            let on_line = self.cursor >= start && self.cursor <= end;
            if on_line && self.mode != Mode::View {
                c.fill(Rect::new(area.x, y, area.w, look.ch), line_bg);
            }
            if gutter > 0 {
                let n = first_line.map_or("?".to_string(), |f| (f + r as u64 + 1).to_string());
                let w = look.font.measure(&n);
                let ink = if on_line { t.c.soft() } else { t.c.muted };
                look.font.draw(
                    c,
                    x0 - theme::TEXT_GUTTER_GAP - w - look.cw / 2,
                    y + look.base,
                    &n,
                    ink,
                );
            }
            // Cells on screen, with their colour from the spans.
            let mut si = 0;
            let mut run_x = 0;
            let mut run_color = 0;
            let flush = |c: &mut gh0st_app::gfx::Canvas, run: &mut String, x: i32, color| {
                if !run.is_empty() {
                    look.font.draw(c, x, y + look.base, run, color);
                    run.clear();
                }
            };
            for cell in view::cells(&d.line) {
                if cell.col + cell.width <= self.pan {
                    continue;
                }
                if cell.col >= self.pan + cols {
                    break;
                }
                while si < d.spans.len() && d.spans[si].end as usize <= cell.byte {
                    si += 1;
                }
                let role = d.spans.get(si).map_or(Role::Plain, |s| s.role);
                let x = x0 + (cell.col - self.pan.min(cell.col)) as i32 * look.cw;
                let off = start + cell.byte as u64;
                let w = cell.width as i32 * look.cw;
                if sel.is_some_and(|(a, b)| off >= a && off < b) {
                    flush(c, &mut run, run_x, run_color);
                    c.fill(Rect::new(x, y, w, look.ch), sel_bg);
                } else if self.hit.is_some_and(|(a, b)| off >= a && off < b) {
                    flush(c, &mut run, run_x, run_color);
                    c.fill(Rect::new(x, y, w, look.ch), hit_bg);
                }
                let color = match cell.glyph {
                    Glyph::Bad(_) => t.c.syntax(Role::Removed),
                    Glyph::Control(_) => t.c.muted,
                    _ => t.c.syntax(role),
                };
                let font = if role == Role::Heading {
                    look.bold
                } else {
                    look.font
                };
                match cell.glyph {
                    Glyph::Char(ch) if ch.is_ascii() && std::ptr::eq(font, look.font) => {
                        if run.is_empty() || color != run_color {
                            flush(c, &mut run, run_x, run_color);
                            run_x = x;
                            run_color = color;
                        }
                        run.push(ch);
                    }
                    other => {
                        flush(c, &mut run, run_x, run_color);
                        let s = match other {
                            Glyph::Char(ch) => ch.to_string(),
                            Glyph::Tab => String::new(),
                            Glyph::Bad(_) => "?".into(),
                            Glyph::Control(_) => "\u{b7}".into(),
                        };
                        if !s.is_empty() {
                            font.draw(c, x, y + look.base, &s, color);
                        }
                    }
                }
            }
            flush(c, &mut run, run_x, run_color);
            if on_line {
                let at = (self.cursor - start).min(d.line.len() as u64) as usize;
                let col = view::col_of(&d.line, at);
                if col >= self.pan && col < self.pan + cols {
                    let x = x0 + (col - self.pan) as i32 * look.cw;
                    let ink = if self.mode == Mode::View {
                        t.c.muted
                    } else {
                        t.c.accent
                    };
                    c.fill(Rect::new(x, y, theme::TERM_CURSOR_W, look.ch), ink);
                }
            }
            if end >= len {
                break;
            }
            start = end + 1;
        }
    }
}

/// A binary file: offset, bytes in hex, and the bytes that are text.
/// Bytes on a row of the hex view `cols` cells wide: an offset, the bytes
/// in hex, then as text.
fn hex_per(cols: usize) -> usize {
    [16usize, 8, 4]
        .into_iter()
        .find(|&n| 9 + 3 * n + 1 + n <= cols)
        .unwrap_or(4)
}

fn draw_hex(ui: &mut Ui, t: &Theme, area: Rect, look: Look, b: &mut Buffer, top: u64, cursor: u64) {
    let per = hex_per((area.w / look.cw) as usize);
    let rows = (area.h / look.ch) as usize;
    let top = top / per as u64 * per as u64;
    let c = ui.canvas();
    for r in 0..rows {
        let off = top + (r * per) as u64;
        if off >= b.len() && r > 0 {
            break;
        }
        let bytes = b.bytes(off..off + per as u64).unwrap_or_default();
        let y = area.y + r as i32 * look.ch + look.base;
        let x = area.x + look.cw / 2;
        look.font.draw(c, x, y, &format!("{off:08x}"), t.c.muted);
        for (i, &byte) in bytes.iter().enumerate() {
            let hx = x + (9 + 3 * i) as i32 * look.cw;
            let role = if byte == 0 {
                Role::Comment
            } else {
                Role::Number
            };
            if off + i as u64 == cursor {
                c.fill(
                    Rect::new(hx - 1, y - look.base, 2 * look.cw + 2, look.ch),
                    mix(t.c.bg, t.c.accent, theme::TEXT_SELECT),
                );
            }
            look.font
                .draw(c, hx, y, &format!("{byte:02x}"), t.c.syntax(role));
            let ax = x + (9 + 3 * per + 1 + i) as i32 * look.cw;
            let ch = if byte.is_ascii_graphic() {
                byte as char
            } else {
                '.'
            };
            let ink = if byte.is_ascii_graphic() {
                t.c.syntax(Role::String)
            } else {
                t.c.muted
            };
            look.font.draw(c, ax, y, &ch.to_string(), ink);
        }
    }
}
