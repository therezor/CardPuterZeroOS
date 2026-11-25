//! The editor page. It takes the keyboard raw, as the terminal does, and
//! draws only the rows on screen.
//!
//! Two key models, chosen in the menu:
//! - Modeless: always typing. Ctrl and a letter run commands, Shift and an
//!   arrow select. No action bar: every row is text.
//! - View and edit: a file opens in View, where letters move and chips
//!   find, go to a line and save. Enter or `i` edits; Esc goes back.

use crate::buf::save::{self, Disk, Saver, Way};
use crate::buf::{Buffer, Lines};
use crate::find::{Finder, Found};
use crate::journal::{self, Journal, Recovered};
use crate::lex::{self, Cache, Lang, Role, Span};
use crate::recent::Recent;
use crate::view::{self, Glyph};
use crate::warm;
use crate::watch::{Change, FileWatch};
use gh0st_app::gfx::color::mix;
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::theme::{self, TermFace};
use gh0st_app::gfx::{Rect, Theme};
use gh0st_app::prelude::*;
use gh0st_app::{Pick, PickSpec, RawKey};
use gh0st_core::keys::{Nav, State, code};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Settings, in the shared config.
mod key {
    pub const KEYS: &str = "gh0st.text.keys";
    pub const SIZE: &str = "gh0st.text.size";
    pub const NUMBERS: &str = "gh0st.text.numbers";
    pub const WRAP: &str = "gh0st.text.wrap";
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
    pub const COMMENT: u32 = 24;
    pub const WRAP: u32 = 25;
    pub const NEXT_FILE: u32 = 26;
    pub const CLOSE_FILE: u32 = 27;
    pub const OPEN: u32 = 28;
    pub const SAVE_AS: u32 = 29;
    /// FILE + n switches to open file n.
    pub const FILE: u32 = 40;
}

/// Dialog ids.
mod ask {
    pub const QUIT: u32 = 1;
    pub const RESTORE: u32 = 2;
    pub const REPLACE: u32 = 3;
    pub const CHANGED: u32 = 4;
    pub const IN_PLACE: u32 = 5;
    pub const RECOVER: u32 = 6;
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
/// How long after an edit the journal reaches the card.
const JOURNAL_MS: u64 = 2000;
/// How far a bracket's pair is looked for.
const PAIR_REACH: u64 = 64 << 10;
/// Files open at once.
pub const FILES: usize = 4;
/// Lines longer than this never wrap: they scroll sideways. A key on a
/// wrapped 64 KB line took 20 ms on the device.
const WRAP_MAX: u64 = 16 << 10;
/// One step of indent when the file's own can't be told.
const INDENT: &[u8] = b"    ";
/// The most a copy reads.
const COPY_MAX: u64 = 16 << 20;

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
    /// Where the recent files and the journal go, instead of the app's
    /// data folder.
    pub data: Option<PathBuf>,
    /// Wrap long lines, instead of the setting.
    pub wrap: Option<bool>,
}

/// What a measure was of: the text on screen, its version, an offset.
type Memo = (u64, u64, u64);

struct Doc {
    buf: Buffer,
    cache: Cache,
    spans: Vec<Span>,
    line: Vec<u8>,
}

impl Doc {
    /// Lets go of what is read again on demand: blocks, lexer states and
    /// scratch lines. Edits stay.
    fn shrink(&mut self) {
        self.buf.shrink();
        let lang = self.cache.lang();
        self.cache.clear(lang);
        self.spans = Vec::new();
        self.line = Vec::new();
    }

    fn empty() -> Doc {
        Doc {
            buf: Buffer::from_bytes(b""),
            cache: Cache::new(Lang::Plain),
            spans: Vec::new(),
            line: Vec::new(),
        }
    }
}

/// An open file while another is on screen: everything that belongs to it
/// and not to the editor.
struct Parked {
    path: Option<PathBuf>,
    title: String,
    doc: Doc,
    error: Option<String>,
    binary: bool,
    read_only: bool,
    cursor: u64,
    anchor: Option<u64>,
    want: Option<u32>,
    top: u64,
    top_seg: u32,
    pan: u32,
    saved: u64,
    hit: Option<(u64, u64)>,
    journal: Option<Journal>,
    /// The file on disk as last read or saved: a change while parked shows
    /// when it is back on screen.
    known: Option<Disk>,
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
    /// Long lines wrap onto the rows under them.
    wrap: bool,
    /// With wrap, the first row of the top line on screen.
    top_seg: u32,
    size: usize,
    numbers: bool,
    /// The undo state the file on disk has.
    saved: u64,
    area: Rect,
    index: Option<Timer>,
    budget: u64,
    /// A find under way, and the text's version it searches.
    finder: Option<(Finder, Timer, u64)>,
    /// A save under way, and the file it writes: the page takes that name
    /// once it is done.
    saving: Option<(Saver, Timer, PathBuf)>,
    /// Close the file once the save under way is done.
    close_after: bool,
    /// Where a save goes while the user is asked to write it in place.
    in_place: Option<PathBuf>,
    search: Search,
    /// The query of the last find, for find next.
    needle: Vec<u8>,
    /// The match on screen.
    hit: Option<(u64, u64)>,
    /// The text's version at the last settle: a change since makes the
    /// match stale, and may send the index back.
    seen: u64,
    clip: Vec<u8>,
    watch: Option<(FileWatch, Watch)>,
    context: String,
    /// The reader bringing a big file into memory (`warm`).
    warm: Option<i32>,
    /// The bracket at the cursor and its pair.
    pair: Option<(u64, u64)>,
    /// Bumped when another text comes on screen: what was measured was of
    /// the one before.
    shown: u64,
    /// A line's width in columns and the cursor's column, kept while the
    /// text stays as it was: a long wrapped line is measured once a key,
    /// not once a row. Keyed by `shown`, the buffer's version and offset.
    line_cols: Cell<Option<(Memo, u32)>>,
    cursor_col: Cell<Option<(Memo, u32)>>,
    recent: Recent,
    /// The open files, by slot: the one on screen is None here.
    files: Vec<Option<Parked>>,
    /// The slot on screen.
    at: usize,
    /// A question to the file chooser, and what its answer is for.
    picking: Option<(Pick, Purpose)>,
    /// With no file: the start page's list, recent files under Open.
    start: Option<List>,
    /// The journal of the file on screen.
    journal: Option<Journal>,
    /// When the journal next reaches the card.
    flush_at: Option<Timer>,
    /// Edits found from before, waiting for the user's answer.
    recovered: Option<Recovered>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Open,
    SaveAs,
}

impl Editor {
    /// The file at `path`, or a new one there if it doesn't exist.
    pub fn open(path: Option<&Path>, opts: Options) -> Editor {
        // One name per file: the journal, recent files and the four slots
        // go by it.
        let abs = path.map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
        let path = abs.as_deref();
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
            model: KeyModel::Modal,
            mode: Mode::Edit,
            cursor: 0,
            anchor: None,
            want: None,
            top: 0,
            pan: 0,
            wrap: false,
            top_seg: 0,
            size: 0,
            numbers: true,
            saved: 0,
            area: Rect::new(0, 18, 320, 152),
            index: None,
            budget: 1 << 20,
            finder: None,
            saving: None,
            close_after: false,
            in_place: None,
            search: Search::new(),
            needle: Vec::new(),
            hit: None,
            seen: 0,
            clip: Vec::new(),
            watch: None,
            context: String::new(),
            warm: None,
            pair: None,
            shown: 0,
            line_cols: Cell::new(None),
            cursor_col: Cell::new(None),
            recent: Recent::default(),
            files: vec![None],
            at: 0,
            picking: None,
            start: None,
            journal: None,
            flush_at: None,
            recovered: None,
        }
    }

    fn data_dir(&self, cx: &Cx) -> PathBuf {
        self.opts.data.clone().unwrap_or_else(|| cx.data_dir())
    }

    /// Starts the journal of the file on screen, and asks about edits a
    /// journal kept from before.
    fn attach_journal(&mut self, cx: &mut Cx) {
        let Some(p) = self.path.clone() else {
            return;
        };
        if self.binary || self.read_only || self.journal.is_some() || !p.exists() {
            return;
        }
        let data = self.data_dir(cx);
        self.doc.borrow_mut().buf.keep_changes(true);
        self.journal = Some(Journal::new(&data, &p));
        let Some(r) = journal::read(&data, &p) else {
            return;
        };
        let n = r.changes.len();
        if r.fits(&p) {
            cx.ask(
                ask::RESTORE,
                Dialog::confirm(
                    "Restore unsaved edits?",
                    &format!("{} closed with {n} edits not saved.", self.title),
                )
                .yes("restore")
                .no("drop them"),
            );
        } else {
            cx.ask(
                ask::RECOVER,
                Dialog::confirm(
                    "Keep the unsaved edits?",
                    &format!(
                        "{} changed since. What you typed opens as a new file.",
                        self.title
                    ),
                )
                .yes("keep")
                .no("drop them"),
            );
        }
        self.recovered = Some(r);
    }

    /// The journal's answer: replays the edits, opens what was typed as a
    /// new file, or drops them.
    fn recover(&mut self, id: u32, yes: bool, cx: &mut Cx) {
        let Some(r) = self.recovered.take() else {
            return;
        };
        match (id, yes) {
            (ask::RESTORE, true) => {
                let mut d = self.doc.borrow_mut();
                d.buf.seal();
                for c in &r.changes {
                    if let crate::buf::Change::Replace { at, len, bytes } = c {
                        let _ = d.buf.replace(*at..at + len, bytes);
                    }
                }
                d.buf.seal();
                // Already in the journal: not again.
                d.buf.take_changes();
                let lang = d.cache.lang();
                d.cache.clear(lang);
                drop(d);
                if let Some(j) = &mut self.journal {
                    j.resume(r.good);
                }
                cx.toast(&format!("Restored {} edits", r.changes.len()));
            }
            (ask::RECOVER, true) => {
                if self.files.len() >= FILES {
                    // Its journal stays untouched, and the question comes
                    // back as this file shows again.
                    self.journal = None;
                    self.doc.borrow_mut().buf.keep_changes(false);
                    cx.toast("Four files are open: close one to keep the edits");
                    return;
                }
                let typed = r.typed();
                if let Some(j) = &mut self.journal {
                    j.discard();
                }
                let name = format!("Edits to {}", self.title);
                let mut p = Editor::open(None, self.opts.clone()).take_file();
                p.doc.buf = Buffer::from_bytes(&typed);
                p.title = name;
                let cur = self.park(cx);
                self.files[self.at] = Some(cur);
                self.files.push(None);
                self.at = self.files.len() - 1;
                self.unpark(p, cx);
            }
            _ => {
                if let Some(j) = &mut self.journal {
                    j.discard();
                }
            }
        }
    }

    /// Takes the changes since the last look into the journal, and has it
    /// reach the card soon.
    fn note_changes(&mut self, cx: &mut Cx) {
        let changes = self.doc.borrow_mut().buf.take_changes();
        if changes.is_empty() {
            return;
        }
        let Some(j) = &mut self.journal else {
            return;
        };
        for c in &changes {
            j.add(c);
        }
        if self.flush_at.is_none() {
            self.flush_at = Some(cx.after(JOURNAL_MS));
        }
    }

    /// Writes the journal now. A file with nothing unsaved needs none.
    /// `durable` waits for the card: on pause, park and stop.
    fn flush_journal(&mut self, cx: &mut Cx) {
        self.write_journal(cx, true);
    }

    fn write_journal(&mut self, cx: &mut Cx, durable: bool) {
        self.note_changes(cx);
        if let Some(t) = self.flush_at.take() {
            cx.cancel(t);
        }
        let dirty = self.dirty();
        if let Some(j) = &mut self.journal {
            if dirty {
                if let Err(e) = j.flush(durable) {
                    cx.toast_tone(Tone::Warning, &format!("Unsaved edits not kept: {e}"));
                }
            } else {
                j.discard();
            }
        }
    }

    /// Another program changed the file on screen: with no changes of
    /// ours it is read again, else the user hears of it.
    fn disk_changed(&mut self, change: Change, cx: &mut Cx) {
        match change {
            Change::Gone => cx.toast_tone(Tone::Warning, "The file was deleted or moved"),
            Change::Modified if !self.dirty() => self.reload(cx),
            Change::Modified => cx.ask(
                ask::CHANGED,
                Dialog::confirm(
                    "Changed on disk",
                    "Another program changed this file. Your edits stay until you save over it.",
                )
                .yes("ok"),
            ),
        }
    }

    /// Reads the file on screen again. The watch stays; what ran for the
    /// old text stops.
    fn reload(&mut self, cx: &mut Cx) {
        let Some(p) = self.path.clone() else {
            return;
        };
        self.stop_warm();
        if let Some(t) = self.index.take() {
            cx.cancel(t);
        }
        if let Some((_, t, _)) = self.finder.take() {
            cx.cancel(t);
            cx.idle();
        }
        if let Some((_, t, _)) = self.saving.take() {
            cx.cancel(t);
            cx.idle();
        }
        self.close_after = false;
        // Nothing unsaved: what its journal holds is stale.
        if let Some(mut j) = self.journal.take() {
            j.discard();
        }
        let mut fresh = Editor::open(Some(&p), self.opts.clone()).take_file();
        let len = fresh.doc.buf.len();
        fresh.cursor = self.cursor.min(len);
        fresh.top = fresh.doc.buf.line_start(self.top.min(len)).unwrap_or(0);
        self.put_file(fresh);
        self.attach_journal(cx);
        if !self.doc.borrow().buf.lines().exact {
            self.warm_from(0, cx);
            self.index = Some(cx.after(0));
        }
        cx.toast("Reloaded: changed on disk");
    }

    /// Asks the file chooser for files to open.
    fn ask_open(&mut self, cx: &mut Cx) {
        let mut spec = text_types(PickSpec::open().multiple());
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            spec = spec.starting_in(dir);
        }
        self.picking = Some((cx.pick(spec), Purpose::Open));
    }

    /// Asks the file chooser where to save the file on screen.
    fn ask_save_as(&mut self, cx: &mut Cx) {
        let name = if self.path.is_some() {
            self.title.clone()
        } else {
            "untitled.txt".to_string()
        };
        let mut spec = text_types(PickSpec::save(&name));
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            spec = spec.starting_in(dir);
        }
        self.picking = Some((cx.pick(spec), Purpose::SaveAs));
    }

    /// Saves the file on screen to `path`. It is called that once the save
    /// is done; a save that fails or is declined leaves it as it was.
    fn save_as(&mut self, path: PathBuf, cx: &mut Cx) {
        let path = std::path::absolute(&path).unwrap_or(path);
        self.save_to(path, cx);
    }

    /// The text is saved to `path`: the file on screen is that one now.
    fn bind(&mut self, path: PathBuf, cx: &mut Cx) {
        if let Some((_, w)) = self.watch.take() {
            cx.unwatch(w);
        }
        self.title = path
            .file_name()
            .map_or("Text".into(), |n| n.to_string_lossy().into_owned());
        let first = {
            let mut d = self.doc.borrow_mut();
            let end = d.buf.line_end(0).unwrap_or(0).min(256);
            d.buf.bytes(0..end).unwrap_or_default()
        };
        let lang = lex::detect(&path, &first);
        self.doc.borrow_mut().cache.clear(lang);
        self.read_only = false;
        self.path = Some(path.clone());
        // The journal follows the new name: edits kept from here on are
        // that file's.
        if let Some(mut j) = self.journal.take() {
            j.discard();
        }
        self.new_journal(cx);
        let w = FileWatch::new(&path);
        if let Some(fd) = w.fd() {
            let id = cx.watch(fd);
            self.watch = Some((w, id));
        }
        self.recent.add(&path);
        cx.recent(&path);
    }

    /// A journal for the file on screen, from now on.
    fn new_journal(&mut self, cx: &mut Cx) {
        let Some(p) = self.path.clone() else {
            return;
        };
        if self.binary || self.read_only || self.journal.is_some() {
            return;
        }
        let data = self.data_dir(cx);
        self.doc.borrow_mut().buf.keep_changes(true);
        self.journal = Some(Journal::new(&data, &p));
    }

    fn start_pick(&mut self, i: usize, cx: &mut Cx) {
        match i {
            0 => self.ask_open(cx),
            1 => {
                self.start = None;
                self.set_mode(Mode::Edit);
            }
            _ => {
                let Some(p) = self.recent.list().get(i - 2).cloned() else {
                    return;
                };
                if !p.exists() {
                    self.recent.remove(&p);
                    cx.toast("That file is gone");
                    return;
                }
                self.start = None;
                self.open_file(&p, cx);
            }
        }
    }

    /// Takes out the file on screen. Its timers, watch and reader must have
    /// stopped: see `park`.
    fn take_file(&mut self) -> Parked {
        Parked {
            path: self.path.take(),
            title: std::mem::take(&mut self.title),
            doc: std::mem::replace(self.doc.get_mut(), Doc::empty()),
            error: self.error.take(),
            binary: self.binary,
            read_only: self.read_only,
            cursor: self.cursor,
            anchor: self.anchor.take(),
            want: self.want.take(),
            top: self.top,
            top_seg: self.top_seg,
            pan: self.pan,
            saved: self.saved,
            hit: self.hit.take(),
            journal: self.journal.take(),
            known: self.path.as_deref().and_then(Disk::of),
        }
    }

    fn put_file(&mut self, p: Parked) {
        self.shown += 1;
        self.path = p.path;
        self.title = p.title;
        *self.doc.get_mut() = p.doc;
        self.error = p.error;
        self.binary = p.binary;
        self.read_only = p.read_only;
        self.cursor = p.cursor;
        self.anchor = p.anchor;
        self.want = p.want;
        self.top = p.top;
        self.top_seg = p.top_seg;
        self.pan = p.pan;
        self.saved = p.saved;
        self.hit = p.hit;
        self.journal = p.journal;
        self.pair = None;
        // The parked match is of this text.
        self.seen = self.doc.get_mut().buf.version();
    }

    /// Stops what runs for the file on screen and takes it out.
    fn park(&mut self, cx: &mut Cx) -> Parked {
        self.flush_journal(cx);
        self.stop_warm();
        if let Some(t) = self.index.take() {
            cx.cancel(t);
        }
        if let Some((_, t, _)) = self.finder.take() {
            cx.cancel(t);
            cx.idle();
        }
        let known = match self.watch.take() {
            Some((w, id)) => {
                cx.unwatch(id);
                w.known()
            }
            None => self.path.as_deref().and_then(Disk::of),
        };
        let mut p = self.take_file();
        p.known = known;
        p
    }

    /// Puts `p` on screen and starts what runs for it: its watch, and the
    /// index while it has more to read.
    fn unpark(&mut self, p: Parked, cx: &mut Cx) {
        let known = p.known;
        self.put_file(p);
        if !self.doc.borrow().buf.lines().exact {
            let from = self.doc.borrow().buf.indexed();
            self.warm_from(from, cx);
            self.index = Some(cx.after(1));
        }
        if let Some(p) = self.path.clone() {
            let mut w = FileWatch::since(&p, known);
            let change = w.check();
            if let Some(fd) = w.fd() {
                let id = cx.watch(fd);
                self.watch = Some((w, id));
            }
            if let Some(c) = change {
                self.disk_changed(c, cx);
            }
        }
        if let Some(e) = self.error.clone() {
            cx.toast_tone(Tone::Critical, &e);
        }
        self.attach_journal(cx);
    }

    /// Shows open file `to`. Not while one saves.
    fn switch(&mut self, to: usize, cx: &mut Cx) {
        if to == self.at || to >= self.files.len() || self.saving.is_some() {
            return;
        }
        let Some(next) = self.files[to].take() else {
            return;
        };
        let cur = self.park(cx);
        self.files[self.at] = Some(cur);
        self.at = to;
        self.unpark(next, cx);
        self.leave_query();
    }

    /// The slot of the open file at `path`, if it is open.
    fn slot_of(&self, path: &Path) -> Option<usize> {
        if self.path.as_deref() == Some(path) {
            return Some(self.at);
        }
        self.files
            .iter()
            .position(|f| f.as_ref().is_some_and(|f| f.path.as_deref() == Some(path)))
    }

    /// Nothing to lose here: a new file never typed in.
    fn blank(&self) -> bool {
        self.path.is_none() && self.doc.borrow().buf.is_empty() && !self.dirty()
    }

    /// Opens `path`: shows it if it is open already, else in a new slot, or
    /// in place of a blank new file. False when four files are open.
    fn open_file(&mut self, path: &Path, cx: &mut Cx) -> bool {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let path = abs.as_path();
        if self.saving.is_some() {
            cx.toast("Saving: open it when that is done");
            return false;
        }
        if let Some(i) = self.slot_of(path) {
            self.switch(i, cx);
            return true;
        }
        let fresh = Editor::open(Some(path), self.opts.clone()).take_file();
        if self.blank() {
            let _ = self.park(cx);
        } else if self.files.len() < FILES {
            let cur = self.park(cx);
            self.files[self.at] = Some(cur);
            self.files.push(None);
            self.at = self.files.len() - 1;
        } else {
            cx.toast("Four files are open: close one first");
            return false;
        }
        self.unpark(fresh, cx);
        if path.exists() {
            self.recent.add(path);
            cx.recent(path);
        }
        self.leave_query();
        true
    }

    /// Closes the file on screen, changes and all, and shows the one
    /// before it. The last one closes the app. Not while it saves.
    fn close_file(&mut self, cx: &mut Cx) {
        if self.saving.is_some() {
            cx.toast("Saving: close it when that is done");
            return;
        }
        // Closed on purpose: its unsaved edits go with it.
        if let Some(j) = &mut self.journal {
            j.discard();
        }
        self.doc.borrow_mut().buf.take_changes();
        if self.files.len() <= 1 {
            cx.quit();
            return;
        }
        let _ = self.park(cx);
        self.files.remove(self.at);
        self.at = self.at.saturating_sub(1).min(self.files.len() - 1);
        if let Some(next) = self.files[self.at].take() {
            self.unpark(next, cx);
        }
        self.leave_query();
    }

    /// The name of open file `i`.
    fn file_name(&self, i: usize) -> &str {
        if i == self.at {
            return &self.title;
        }
        self.files
            .get(i)
            .and_then(|f| f.as_ref())
            .map_or("", |f| f.title.as_str())
    }

    /// The bracket at or just before the cursor and the one it pairs with,
    /// looked for at most `PAIR_REACH` away. Quotes and comments are not
    /// told apart.
    fn find_pair(&self) -> Option<(u64, u64)> {
        if self.binary {
            return None;
        }
        let mut d = self.doc.borrow_mut();
        let len = d.buf.len();
        let mut at_cursor = [self.cursor, self.cursor.wrapping_sub(1)];
        if self.cursor == 0 {
            at_cursor[1] = u64::MAX;
        }
        for at in at_cursor.into_iter().filter(|&a| a < len) {
            let b = d.buf.bytes(at..at + 1).ok()?.first().copied()?;
            let (open, close, ahead) = match b {
                b'(' => (b'(', b')', true),
                b'[' => (b'[', b']', true),
                b'{' => (b'{', b'}', true),
                b')' => (b'(', b')', false),
                b']' => (b'[', b']', false),
                b'}' => (b'{', b'}', false),
                _ => continue,
            };
            let mut depth = 0i64;
            let mut found = None;
            if ahead {
                let mut off = at;
                let _ = d.buf.for_each(at..(at + PAIR_REACH).min(len), |c| {
                    for &x in c {
                        depth += (x == open) as i64 - (x == close) as i64;
                        if depth == 0 {
                            found = Some(off);
                            return false;
                        }
                        off += 1;
                    }
                    true
                });
            } else {
                let mut end = at + 1;
                let _ = d
                    .buf
                    .for_each_back(at.saturating_sub(PAIR_REACH)..at + 1, |c| {
                        for &x in c.iter().rev() {
                            end -= 1;
                            depth += (x == close) as i64 - (x == open) as i64;
                            if depth == 0 {
                                found = Some(end);
                                return false;
                            }
                        }
                        true
                    });
            }
            return found.map(|o| (at.min(o), at.max(o)));
        }
        None
    }

    /// The step of indent this line uses: a tab if it starts with one.
    fn unit(line: &[u8]) -> &'static [u8] {
        if line.first() == Some(&b'\t') {
            b"\t"
        } else {
            INDENT
        }
    }

    /// Comments the lines the selection touches, or the cursor's line, or
    /// takes the comments off when every one of them has one. One undo step.
    fn toggle_comment(&mut self) {
        if self.read_only || self.binary {
            return;
        }
        let Some(token) = self.doc.borrow().cache.lang().comment() else {
            return;
        };
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        let mut d = self.doc.borrow_mut();
        let len = d.buf.len();
        // The lines: their starts and texts, at most a few thousand.
        let mut lines: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut at = d.buf.line_start(a).unwrap_or(0);
        loop {
            let end = d.buf.line_end(at).unwrap_or(len);
            let mut text = d.buf.bytes(at..end.min(at + 4096)).unwrap_or_default();
            // The CR of a CR LF ending is not text: a blank line stays blank.
            if end <= at + 4096 && text.last() == Some(&b'\r') {
                text.pop();
            }
            lines.push((at, text));
            // A selection ending at a line's start leaves that line out.
            if end >= len || end + 1 >= b.max(a + 1) || lines.len() >= 4096 {
                break;
            }
            at = end + 1;
        }
        let tok = token.as_bytes();
        let body = |t: &[u8]| t[view::indent(t).len()..].to_vec();
        let filled: Vec<&(u64, Vec<u8>)> =
            lines.iter().filter(|(_, t)| !body(t).is_empty()).collect();
        if filled.is_empty() {
            return;
        }
        let all = filled.iter().all(|(_, t)| body(t).starts_with(tok));
        let col = filled
            .iter()
            .map(|(_, t)| view::indent(t).len())
            .min()
            .unwrap_or(0);
        d.buf.seal();
        // Bottom up, so the offsets above stay true.
        let mut shift = 0i64;
        let mut last_shift = 0i64;
        for (i, (start, t)) in filled.iter().enumerate().rev() {
            let ind = view::indent(t).len();
            let by = if all {
                let mut n = tok.len();
                if t.get(ind + n) == Some(&b' ') {
                    n += 1;
                }
                let at = start + ind as u64;
                let _ = d.buf.delete(at..at + n as u64);
                d.cache.edited(at);
                -(n as i64)
            } else {
                let mut ins = tok.to_vec();
                ins.push(b' ');
                let at = start + col as u64;
                let _ = d.buf.insert(at, &ins);
                d.cache.edited(at);
                ins.len() as i64
            };
            shift += by;
            if i == filled.len() - 1 {
                last_shift = by;
            }
        }
        d.buf.seal();
        // The same lines stay selected, so the toggle undoes itself.
        let first = lines[0].0;
        let last = lines[lines.len() - 1]
            .0
            .saturating_add_signed(shift - last_shift);
        let mut end = d.buf.line_end(last).unwrap_or(d.buf.len());
        if end > last && d.buf.bytes(end - 1..end).ok().as_deref() == Some(b"\r") {
            end -= 1;
        }
        drop(d);
        if self.anchor.is_some() {
            self.anchor = Some(first);
            self.cursor = end;
        } else {
            // One line: the cursor moves with its text.
            let at = lines[0].0 + col as u64;
            if self.cursor >= at {
                self.cursor = self
                    .cursor
                    .saturating_add_signed(last_shift)
                    .max(at)
                    .min(end);
            }
        }
    }

    /// A closing bracket typed on a line of blanks: one step of indent
    /// less first.
    fn close_bracket(&mut self, c: u8) {
        let (start, _, line) = self.line_at(self.cursor);
        let at = (self.cursor - start) as usize;
        let before = &line[..at.min(line.len())];
        if self.selection().is_none()
            && !before.is_empty()
            && before.iter().all(|&x| x == b' ' || x == b'\t')
        {
            let unit = Self::unit(before);
            if before.ends_with(unit) {
                self.anchor = Some(self.cursor - unit.len() as u64);
            }
        }
        self.type_bytes(&[c]);
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

    /// True while a save runs, and says so: the text stays as the save
    /// found it.
    fn held(&self, cx: &mut Cx) -> bool {
        if self.saving.is_some() {
            cx.toast("Saving: edit when that is done");
        }
        self.saving.is_some()
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

    /// The line holding `off`: its start, end and bytes up to the lex
    /// limit. The CR of a CR LF ending is part of the ending, not the text:
    /// the end is where the CR is.
    fn line_at(&self, off: u64) -> (u64, u64, Vec<u8>) {
        let mut d = self.doc.borrow_mut();
        let start = d.buf.line_start(off).unwrap_or(0);
        let mut end = d.buf.line_end(off).unwrap_or(start);
        let take = (end - start).min(lex::LINE_MAX as u64 * 4);
        let mut bytes = d.buf.bytes(start..start + take).unwrap_or_default();
        if take == end - start && bytes.last() == Some(&b'\r') {
            bytes.pop();
            end -= 1;
        }
        (start, end, bytes)
    }

    /// Where the next line starts, after the line ending at `end`.
    fn next_line(&self, end: u64) -> u64 {
        end + self.eol_len(end)
    }

    /// The length of the line ending at `end`: 2 for CR LF, else 1.
    fn eol_len(&self, end: u64) -> u64 {
        let mut d = self.doc.borrow_mut();
        let two = d.buf.bytes(end..end + 2).unwrap_or_default();
        if two == b"\r\n" { 2 } else { 1 }
    }

    /// Where the line before the one starting at `start` ends.
    fn end_before(&self, start: u64) -> u64 {
        if start >= 2 && self.eol_len(start - 2) == 2 {
            start - 2
        } else {
            start.saturating_sub(1)
        }
    }

    fn col_of(&self, off: u64) -> u32 {
        remember(&self.cursor_col, self.memo(off), || {
            let (start, _, line) = self.line_at(off);
            view::col_of(&line, (off - start) as usize)
        })
    }

    fn memo(&self, at: u64) -> Memo {
        (self.shown, self.doc.borrow().buf.version(), at)
    }

    /// The rows the line starting at `start` takes, and its length without
    /// its ending, reading it only when it was not measured.
    fn rows_of(&self, start: u64) -> (u32, u64) {
        let mut end = {
            let mut d = self.doc.borrow_mut();
            let len = d.buf.len();
            d.buf.line_end(start).unwrap_or(len)
        };
        if end > start
            && self
                .doc
                .borrow_mut()
                .buf
                .bytes(end - 1..end)
                .ok()
                .as_deref()
                == Some(b"\r")
        {
            end -= 1;
        }
        if !self.wrap || end - start > WRAP_MAX {
            return (1, end - start);
        }
        let w = remember(&self.line_cols, self.memo(start), || {
            let (_, _, l) = self.line_at(start);
            view::col_of(&l, l.len())
        });
        (w.div_ceil(self.cols()).max(1), end - start)
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
            self.end_before(self.cursor)
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
            end + self.eol_len(end)
        } else if word {
            start + view::word_right(&line, at) as u64
        } else {
            start + view::next_char(&line, at) as u64
        };
        self.want = None;
        self.go(to, select);
    }

    fn vertical(&mut self, lines: i64, select: bool) {
        if self.wrap {
            return self.vertical_wrapped(lines, select);
        }
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
                at = end + self.eol_len(end);
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

    /// Rows the line starting at `start` takes, its bytes and length: one
    /// unless it wraps.
    /// The row of its line the byte at `off` sits on.
    fn seg_of(&self, off: u64) -> (u64, u32) {
        let start = self.doc.borrow_mut().buf.line_start(off).unwrap_or(0);
        let (n, _) = self.rows_of(start);
        let col = self.col_of(off);
        (start, (col / self.cols()).min(n - 1))
    }

    /// With wrap: keeps the cursor's row on screen, by line and row.
    fn reveal_wrapped(&mut self) {
        let rows = self.rows() as u32;
        let (cur, cs) = self.seg_of(self.cursor);
        self.pan = 0;
        if (cur, cs) < (self.top, self.top_seg) {
            self.top = cur;
            self.top_seg = cs;
            return;
        }
        // Rows from the top one to the cursor's, a line at a time: a long
        // line is measured once, not once a row.
        let (mut at, mut seg, mut n) = (self.top, self.top_seg, 0);
        let len = self.doc.borrow().buf.len();
        loop {
            if at >= cur {
                n += cs.saturating_sub(seg);
                break;
            }
            let (segs, l) = self.rows_of(at);
            n += segs.saturating_sub(seg);
            if n >= rows || at + l >= len {
                break;
            }
            at = self.next_line(at + l);
            seg = 0;
        }
        if n < rows {
            return;
        }
        // Put it on the last row.
        let (mut a, mut sg) = (cur, cs);
        for _ in 0..rows - 1 {
            if sg > 0 {
                sg -= 1;
            } else if a == 0 {
                break;
            } else {
                a = self.doc.borrow_mut().buf.line_start(a - 1).unwrap_or(0);
                sg = self.rows_of(a).0 - 1;
            }
        }
        self.top = a;
        self.top_seg = sg;
    }

    /// With wrap: up and down by rows on screen, keeping the column in the
    /// row.
    fn vertical_wrapped(&mut self, rows: i64, select: bool) {
        let cols = self.cols();
        let (start, seg) = self.seg_of(self.cursor);
        let vcol = match self.want {
            Some(c) => c,
            None => self.col_of(self.cursor) - seg * cols,
        };
        let len = self.doc.borrow().buf.len();
        let (mut line, mut seg) = (start, seg);
        for _ in 0..rows.unsigned_abs() {
            if rows < 0 {
                if seg > 0 {
                    seg -= 1;
                } else if line == 0 {
                    break;
                } else {
                    line = self.doc.borrow_mut().buf.line_start(line - 1).unwrap_or(0);
                    seg = self.rows_of(line).0 - 1;
                }
            } else {
                let (segs, l) = self.rows_of(line);
                if seg + 1 < segs {
                    seg += 1;
                } else if line + l >= len {
                    break;
                } else {
                    line = self.next_line(line + l);
                    seg = 0;
                }
            }
        }
        let (_, _, bytes) = self.line_at(line);
        // The row's last cell, not the next row's first.
        let col = seg * cols + vcol.min(cols - 1);
        let mut to = line + view::byte_at(&bytes, col) as u64;
        if self.seg_of(to).1 > seg {
            to = line + view::prev_char(&bytes, (to - line) as usize) as u64;
        }
        self.go(to, select);
        self.want = Some(vcol);
    }

    /// Keeps the cursor on screen.
    fn reveal(&mut self) {
        if self.wrap && !self.binary {
            return self.reveal_wrapped();
        }
        self.top_seg = 0;
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

    /// A new line with the indent of this one, a step more after an open
    /// bracket (or a colon in Python and YAML). Between a bracket and its
    /// closer, the closer goes on a line of its own.
    fn newline(&mut self) {
        let (start, _, line) = self.line_at(self.cursor);
        let at = (self.cursor - start) as usize;
        let before = &line[..at.min(line.len())];
        let indent = view::indent(before).to_vec();
        let last = before
            .iter()
            .rev()
            .find(|&&b| b != b' ' && b != b'\t')
            .copied();
        let lang = self.doc.borrow().cache.lang();
        let opens = matches!(last, Some(b'{' | b'(' | b'['))
            || (last == Some(b':') && matches!(lang, Lang::Python | Lang::Yaml));
        let next = line.get(at).copied();
        let closes = matches!(
            (last, next),
            (Some(b'{'), Some(b'}')) | (Some(b'('), Some(b')')) | (Some(b'['), Some(b']'))
        );
        // A CR LF file gets CR LF: as this line ends, or the one before
        // when this one is the last and has no ending.
        let (_, end, _) = self.line_at(self.cursor);
        let len = self.doc.borrow().buf.len();
        let crlf = if end < len {
            self.eol_len(end) == 2
        } else {
            start > 0 && self.eol_len(self.end_before(start)) == 2
        };
        let eol: &[u8] = if crlf { b"\r\n" } else { b"\n" };
        let mut ins = eol.to_vec();
        ins.extend_from_slice(&indent);
        if opens {
            ins.extend_from_slice(Self::unit(&line));
        }
        let land = ins.len() as u64;
        if closes {
            ins.extend_from_slice(eol);
            ins.extend_from_slice(&indent);
        }
        self.doc.borrow_mut().buf.seal();
        let from = self.selection().map_or(self.cursor, |s| s.0);
        self.type_bytes(&ins);
        self.cursor = from + land;
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
                        end + self.eol_len(end)
                    } else {
                        start + view::next_char(&line, at) as u64
                    };
                    (self.cursor, to.min(self.doc.borrow().buf.len()))
                } else if at == 0 {
                    (self.end_before(self.cursor), self.cursor)
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

    /// The selection to the clipboard every app shares, bytes as they are.
    /// This app keeps its own copy too, for when the shared one won't give
    /// it back. False, and said, unless all of it was copied: a cut then
    /// keeps the text.
    fn copy(&mut self, cx: &mut Cx) -> bool {
        let Some((a, b)) = self.selection() else {
            return false;
        };
        if b - a > COPY_MAX {
            cx.toast_tone(Tone::Warning, "Too much to copy");
            return false;
        }
        let bytes = match self.doc.borrow_mut().buf.bytes(a..b) {
            Ok(bytes) => bytes,
            Err(e) => {
                cx.toast_tone(Tone::Critical, &e.to_string());
                return false;
            }
        };
        if !cx.copy(&bytes) {
            cx.toast_tone(Tone::Warning, "Too much to copy");
            return false;
        }
        self.clip = bytes;
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
        if let Some((_, t, _)) = self.finder.take() {
            cx.cancel(t);
        }
        let version = self.doc.borrow().buf.version();
        self.finder = Some((f, cx.after(0), version));
    }

    fn find_step(&mut self, cx: &mut Cx) {
        let Some((mut f, _, version)) = self.finder.take() else {
            return;
        };
        let t0 = Instant::now();
        loop {
            let r = f.step(&mut self.doc.borrow_mut().buf, crate::find::STEP);
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
                    self.finder = Some((f, cx.after(0), version));
                    return;
                }
                Ok(Found::Wait) => {
                    cx.busy("Finding");
                    self.finder = Some((f, cx.after(DISK_WAIT_MS), version));
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
            self.ask_save_as(cx);
            return;
        };
        if self.read_only {
            cx.toast_tone(Tone::Warning, "Read only");
            return;
        }
        self.save_to(path, cx);
    }

    /// Saves the text to `path`, asking first when it must go in place.
    fn save_to(&mut self, path: PathBuf, cx: &mut Cx) {
        if self.saving.is_some() {
            cx.toast("Saving already");
            return;
        }
        match save::way(&path) {
            Ok(Way::Replace) => self.start_save(path, Way::Replace, cx),
            Ok(Way::InPlace) => {
                self.in_place = Some(path);
                cx.ask(
                    ask::IN_PLACE,
                    Dialog::confirm(
                        "Save over the file?",
                        "It has other links or its folder is read only. A crash while saving loses it.",
                    )
                    .yes("save"),
                );
            }
            Ok(Way::ReadOnly) => cx.toast_tone(Tone::Warning, "Read only"),
            Err(e) => cx.toast_tone(Tone::Critical, &e.to_string()),
        }
    }

    fn start_save(&mut self, path: PathBuf, how: Way, cx: &mut Cx) {
        let started = Saver::start(&mut self.doc.borrow_mut().buf, &path, how);
        match started {
            Ok(s) => {
                let big = self.doc.borrow().buf.len() > SAVE_AT_ONCE;
                if big {
                    cx.busy("Saving");
                    self.warm_from(0, cx);
                }
                self.saving = Some((s, cx.after(0), path));
            }
            Err(e) => {
                self.close_after = false;
                cx.toast_tone(Tone::Critical, &e.to_string());
            }
        }
    }

    fn save_step(&mut self, cx: &mut Cx) {
        let Some((mut s, _, path)) = self.saving.take() else {
            return;
        };
        let t0 = Instant::now();
        loop {
            let r = s.step(&mut self.doc.borrow_mut().buf, 1 << 20);
            match r {
                Ok(true) => {
                    if self.path.as_ref() != Some(&path) {
                        self.bind(path, cx);
                    }
                    self.saved = self.doc.borrow().buf.state();
                    if let Some((w, _)) = &mut self.watch {
                        w.saved();
                    }
                    self.doc.borrow_mut().buf.take_changes();
                    // A file new when opened has one from its first save.
                    self.new_journal(cx);
                    if let Some(j) = &mut self.journal {
                        j.discard();
                    }
                    cx.idle();
                    cx.toast_tone(Tone::Ok, "Saved");
                    if std::mem::take(&mut self.close_after) && !self.dirty() {
                        self.close_file(cx);
                    }
                    return;
                }
                Ok(false) if (t0.elapsed().as_millis() as u64) < SLICE_MS => {}
                Ok(false) => {
                    self.saving = Some((s, cx.after(0), path));
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.saving = Some((s, cx.after(DISK_WAIT_MS), path));
                    return;
                }
                Err(e) => {
                    self.close_after = false;
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
        if self.files.len() > 1 {
            s = format!("{}/{} {s}", self.at + 1, self.files.len());
        }
        self.context = s;
    }

    /// After every event: the bars may have changed, the cursor moved.
    fn settle(&mut self, cx: &mut Cx) {
        let version = self.doc.borrow().buf.version();
        if version != self.seen {
            self.seen = version;
            // What was found is of the text before.
            self.hit = None;
            if self.finder.as_ref().is_some_and(|f| f.2 != version) {
                if let Some((_, t, _)) = self.finder.take() {
                    cx.cancel(t);
                    cx.idle();
                }
            }
            // An edit the index could not follow sends it back a way.
            if self.index.is_none() && !self.lines().exact {
                let from = self.doc.borrow().buf.indexed();
                self.warm_from(from, cx);
                self.index = Some(cx.after(1));
            }
        }
        let t = *cx.theme();
        self.area = self.bars().content(&t);
        self.reveal();
        self.refresh();
        self.pair = self.find_pair();
        self.note_changes(cx);
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
            Mode::Edit => self.edit_key(k, cx),
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

    fn edit_key(&mut self, k: &RawKey, cx: &mut Cx) {
        let sel = k.shift;
        match k.nav(false) {
            Some(Nav::Up) => return self.vertical(-1, sel),
            Some(Nav::Down) => return self.vertical(1, sel),
            Some(Nav::Left) => return self.left(sel, k.ctrl),
            Some(Nav::Right) => return self.right(sel, k.ctrl),
            None => {}
        }
        let types = !k.fn_ && k.ch.is_some_and(|c| !c.is_control());
        match layer(k) {
            code::PAGEUP => self.vertical(-(self.rows() as i64 - 1), sel),
            code::PAGEDOWN => self.vertical(self.rows() as i64 - 1, sel),
            code::END => self.line_edge(true, sel),
            _ if self.read_only => {}
            code::ENTER | code::BACKSPACE | code::DELETE | code::TAB if self.held(cx) => {}
            _ if types && self.held(cx) => {}
            code::ENTER => self.newline(),
            code::BACKSPACE => self.backspace(false),
            code::DELETE => self.backspace(true),
            code::TAB => self.type_bytes(INDENT),
            // Fn and a key that means nothing here types nothing.
            _ if k.fn_ => {}
            _ => match k.ch.filter(|c| !c.is_control()) {
                Some(c @ (')' | ']' | '}')) => self.close_bracket(c as u8),
                Some(c) => {
                    let mut b = [0u8; 4];
                    self.type_bytes(c.encode_utf8(&mut b).as_bytes());
                }
                None => {}
            },
        }
    }

    fn run(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let edit_ok = !self.read_only;
        let edits = matches!(
            id,
            cmd::UNDO | cmd::REDO | cmd::CUT | cmd::PASTE | cmd::COMMENT
        );
        if edit_ok && edits && self.held(cx) {
            return;
        }
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
                if self.copy(cx) {
                    cx.toast("Copied");
                }
            }
            cmd::CUT if edit_ok => {
                if self.copy(cx) {
                    self.backspace(false);
                }
            }
            cmd::PASTE if edit_ok => {
                let clip = cx.paste().unwrap_or_else(|| self.clip.clone());
                if clip.is_empty() {
                    cx.toast("Nothing copied");
                } else {
                    self.doc.borrow_mut().buf.seal();
                    self.type_bytes(&clip);
                    self.doc.borrow_mut().buf.seal();
                }
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
            cmd::WRAP => {
                self.wrap = !self.wrap;
                self.top_seg = 0;
                self.pan = 0;
                self.want = None;
                self.top = self.doc.borrow_mut().buf.line_start(self.top).unwrap_or(0);
                cx.settings().set_bool(key::WRAP, self.wrap);
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
            cmd::DISCARD => self.close_file(cx),
            cmd::OPEN => self.ask_open(cx),
            cmd::SAVE_AS => self.ask_save_as(cx),
            cmd::NEXT_FILE => {
                let n = self.files.len();
                self.switch(
                    (self.at + if step == Step::Prev { n - 1 } else { 1 }) % n,
                    cx,
                );
            }
            cmd::CLOSE_FILE if self.dirty() => cx.ask(
                ask::QUIT,
                Dialog::confirm(
                    &format!("Save {}?", self.title),
                    "Your changes are not saved.",
                )
                .yes("save")
                .no("keep editing")
                .also("discard"),
            ),
            cmd::CLOSE_FILE => self.close_file(cx),
            id if (cmd::FILE..cmd::FILE + FILES as u32).contains(&id) => {
                self.switch((id - cmd::FILE) as usize, cx)
            }
            cmd::EDIT if edit_ok => self.set_mode(Mode::Edit),
            cmd::COMMENT if edit_ok => self.toggle_comment(),
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
        if self.start.is_some() {
            return Bars::top();
        }
        match (self.model, self.mode) {
            (_, Mode::Find | Mode::Goto) => Bars::top().search(&self.search),
            (KeyModel::Modeless, _) => Bars::top_only(),
            (KeyModel::Modal, Mode::View) => Bars::top(),
            (KeyModel::Modal, _) => {
                Bars::top().hints([("Esc", "view"), ("Ctrl S", "save"), ("Fn H", "keys")])
            }
        }
    }

    /// The file chooser's answer.
    fn picked(&mut self, pick: Pick, paths: Option<Vec<PathBuf>>, cx: &mut Cx) {
        let Some((asked, why)) = self.picking.take() else {
            return;
        };
        if asked != pick {
            self.picking = Some((asked, why));
            return;
        }
        let paths = paths.unwrap_or_default();
        match why {
            Purpose::Open => {
                if !paths.is_empty() {
                    self.start = None;
                }
                for p in &paths {
                    if !self.open_file(p, cx) {
                        break;
                    }
                }
            }
            Purpose::SaveAs => {
                if let Some(p) = paths.into_iter().next() {
                    self.start = None;
                    self.save_as(p, cx);
                }
            }
        }
        self.settle(cx);
        cx.redraw();
    }

    /// Files another app opened here, or the one the app started with:
    /// each in a slot of its own, shown if it is open already.
    fn opened(&mut self, paths: &[PathBuf], cx: &mut Cx) {
        if !paths.is_empty() {
            self.start = None;
        }
        for p in paths {
            if !self.open_file(p, cx) {
                break;
            }
        }
        self.settle(cx);
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

    /// Leaving the front: the journal reaches the card now, as the app may
    /// be stopped or killed next.
    fn front(&mut self, front: bool, cx: &mut Cx) {
        if !front {
            self.flush_journal(cx);
        }
    }

    fn low_memory(&mut self, _level: u8, cx: &mut Cx) {
        self.doc.borrow_mut().shrink();
        for p in self.files.iter_mut().flatten() {
            p.doc.shrink();
        }
        cx.redraw();
    }

    fn stop(&mut self, cx: &mut Cx) {
        self.flush_journal(cx);
        self.stop_warm();
    }

    fn start(&mut self, cx: &mut Cx) {
        let s = cx.settings();
        self.model = match self.opts.keys {
            Some(m) => m,
            // Opens to read: a key can't change a file by accident.
            None if s.get_str(key::KEYS) == Some("modeless") => KeyModel::Modeless,
            None => KeyModel::Modal,
        };
        self.size = self
            .opts
            .size
            .or_else(|| s.get_int(key::SIZE).map(|n| n.max(0) as usize))
            .unwrap_or(0)
            .min(theme::TERM_TERMINUS.len() - 1);
        self.numbers = s.get_bool(key::NUMBERS).unwrap_or(true);
        self.wrap = self
            .opts
            .wrap
            .unwrap_or_else(|| s.get_bool(key::WRAP).unwrap_or(false));
        self.recent = Recent::from(cx.recent_files(crate::recent::MAX * 3));
        if let Some(p) = self.path.clone().filter(|p| p.exists()) {
            self.recent.add(&p);
            cx.recent(&p);
        }
        if self.path.is_none() {
            self.start = Some(List::new());
        }
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
        self.attach_journal(cx);
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
        if self.flush_at == Some(t) {
            self.flush_at = None;
            self.write_journal(cx, false);
        } else if self.index == Some(t) {
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
        let Some(change) = fw.check() else {
            return;
        };
        self.disk_changed(change, cx);
        self.settle(cx);
        cx.redraw();
    }

    /// Raw while typing. View takes keys as a list does, so its chips and
    /// the menu on 8 work.
    fn raw_keys(&self) -> bool {
        self.mode != Mode::View && self.start.is_none()
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if let Some(list) = &mut self.start {
            // View's bare keys move here too.
            let key = match key {
                Key::Char('f') => Key::Up,
                Key::Char('x') => Key::Down,
                k => k,
            };
            if let Some(Action::Open(i)) = list.key(key) {
                self.start_pick(i, cx);
            }
            self.settle(cx);
            return;
        }
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
            if let Some((_, t, _)) = self.finder.take() {
                cx.cancel(t);
                cx.idle();
            }
            self.leave_query();
        } else if self.anchor.is_some() {
            self.anchor = None;
        } else if self.model == KeyModel::Modal && self.mode == Mode::Edit {
            self.set_mode(Mode::View);
        } else if !self.dirty() && self.files.len() > 1 {
            self.close_file(cx);
        } else if self.dirty() {
            cx.ask(
                ask::QUIT,
                Dialog::confirm(
                    &format!("Save {}?", self.title),
                    "Your changes are not saved.",
                )
                .yes("save")
                .no("keep editing")
                .also("discard"),
            );
        } else {
            return true;
        }
        self.settle(cx);
        false
    }

    fn answer(&mut self, id: u32, a: Answer, cx: &mut Cx) {
        if id == ask::RESTORE || id == ask::RECOVER {
            self.recover(id, matches!(a, Answer::Yes(_)), cx);
            self.settle(cx);
            return;
        }
        match (id, a) {
            // Held 7: close, the changes gone.
            (ask::QUIT, Answer::Also) => self.close_file(cx),
            (ask::QUIT, Answer::Yes(_)) => {
                self.save(cx);
                // Closes once saved, on the save's own timer. A save that
                // fails, or goes to the chooser, leaves the file open.
                self.close_after = self.saving.is_some() || self.in_place.is_some();
            }
            (ask::IN_PLACE, a) => match (self.in_place.take(), a) {
                (Some(p), Answer::Yes(_)) => self.start_save(p, Way::InPlace, cx),
                _ => self.close_after = false,
            },
            (ask::REPLACE, Answer::Yes(_)) if self.held(cx) => {}
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
        c.add(Command::action(cmd::OPEN, "Open").keys("Ctrl+o").opens());
        c.add(
            Command::action(cmd::SAVE_AS, "Save as")
                .opens()
                .enabled(!self.binary),
        );
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
                .enabled(can_edit),
        ));
        c.add(here(Command::action(cmd::ALL, "Select all")));
        c.add(here(
            Command::action(cmd::COMMENT, "Comment lines")
                .keys("Ctrl+k")
                .enabled(can_edit && self.doc.borrow().cache.lang().comment().is_some()),
        ));
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
        c.add(Command::toggle(cmd::WRAP, "Wrap long lines", self.wrap));
        c.add(Command::choice(cmd::KEYS, "Keys", self.model.word()));
        if self.files.len() > 1 {
            c.group("Open files");
            for i in 0..self.files.len() {
                let mut row = Command::action(cmd::FILE + i as u32, self.file_name(i).to_string())
                    .enabled(i != self.at);
                if i < 4 {
                    row = row.keys(["Alt+1", "Alt+2", "Alt+3", "Alt+4"][i]);
                }
                c.add(row);
            }
            c.add(here(
                Command::choice(cmd::NEXT_FILE, "Next file", "")
                    .keys("Ctrl+tab")
                    .keys_only(),
            ));
        }
        c.group("Close");
        c.add(here(
            Command::action(cmd::CLOSE_FILE, "Close file").keys("Ctrl+w"),
        ));
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
        if let Some(list) = &self.start {
            let names: Vec<(String, String)> = self
                .recent
                .list()
                .iter()
                .map(|p| {
                    let name = p
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    let dir = p
                        .parent()
                        .map_or(String::new(), |d| d.to_string_lossy().into_owned());
                    (name, dir)
                })
                .collect();
            let mut rows = vec![
                Row::link("Open a file").icon("folder-open"),
                Row::text("New file").icon("file"),
            ];
            rows.extend(
                names
                    .iter()
                    .map(|(n, dir)| Row::link(n).value(dir).icon("file-text")),
            );
            ui.list(list, &rows);
            return;
        }
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
        let pair_bg = mix(t.c.bg, t.c.accent, theme::TEXT_PAIR);
        let mut num = String::new();
        let c = ui.canvas();
        let mut start = self.top;
        let mut first_seg = self.top_seg;
        let mut run = String::new();
        let mut r = 0usize;
        let mut line_no = first_line;
        while r < rows {
            if start > len {
                break;
            }
            let end = d.buf.line_end(start).unwrap_or(len);
            let wraps = self.wrap && end - start <= WRAP_MAX;
            let take = (end - start).min(if wraps {
                WRAP_MAX
            } else {
                lex::LINE_MAX as u64
            });
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
            // A wrapped line is read whole, as the measures read it, so
            // they hold for it.
            let version = d.buf.version();
            let segs = if wraps {
                remember(&self.line_cols, (self.shown, version, start), || {
                    view::col_of(&d.line, d.line.len())
                })
                .div_ceil(cols)
                .max(1)
            } else {
                1
            };
            let cur_col = on_line.then(|| {
                let at = (self.cursor - start).min(d.line.len() as u64) as usize;
                if wraps {
                    remember(&self.cursor_col, (self.shown, version, self.cursor), || {
                        view::col_of(&d.line, at)
                    })
                } else {
                    view::col_of(&d.line, at)
                }
            });
            // One walk over the line's cells for all its rows.
            let mut cells = view::cells(&d.line).peekable();
            for seg in first_seg..segs {
                if r >= rows {
                    break;
                }
                let y = area.y + r as i32 * look.ch;
                // The columns this row shows.
                let c0 = if wraps { seg * cols } else { self.pan };
                if on_line && self.mode != Mode::View {
                    c.fill(Rect::new(area.x, y, area.w, look.ch), line_bg);
                }
                if gutter > 0 && seg == 0 {
                    let ink = if on_line { t.c.soft() } else { t.c.muted };
                    num.clear();
                    match line_no {
                        Some(n) => {
                            use std::fmt::Write;
                            let _ = write!(num, "{}", n + 1);
                        }
                        None => num.push('?'),
                    }
                    let w = look.font.measure(&num);
                    look.font.draw(
                        c,
                        x0 - theme::TEXT_GUTTER_GAP - w - look.cw / 2,
                        y + look.base,
                        &num,
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
                while let Some(&cell) = cells.peek() {
                    if cell.col >= c0 + cols || (wraps && cell.col + cell.width > c0 + cols) {
                        break;
                    }
                    cells.next();
                    if cell.col + cell.width <= c0 || (wraps && cell.col < c0) {
                        continue;
                    }
                    while si < d.spans.len() && d.spans[si].end as usize <= cell.byte {
                        si += 1;
                    }
                    let role = d.spans.get(si).map_or(Role::Plain, |s| s.role);
                    let x = x0 + (cell.col - c0.min(cell.col)) as i32 * look.cw;
                    let off = start + cell.byte as u64;
                    let w = cell.width as i32 * look.cw;
                    if sel.is_some_and(|(a, b)| off >= a && off < b) {
                        flush(c, &mut run, run_x, run_color);
                        c.fill(Rect::new(x, y, w, look.ch), sel_bg);
                    } else if self.hit.is_some_and(|(a, b)| off >= a && off < b) {
                        flush(c, &mut run, run_x, run_color);
                        c.fill(Rect::new(x, y, w, look.ch), hit_bg);
                    } else if self.pair.is_some_and(|(a, b)| off == a || off == b) {
                        flush(c, &mut run, run_x, run_color);
                        c.fill(Rect::new(x, y, w, look.ch), pair_bg);
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
                            let mut b = [0u8; 4];
                            let s: &str = match other {
                                Glyph::Char(ch) => ch.encode_utf8(&mut b),
                                Glyph::Tab => "",
                                Glyph::Bad(_) => "?",
                                Glyph::Control(_) => "\u{b7}",
                            };
                            if !s.is_empty() {
                                font.draw(c, x, y + look.base, s, color);
                            }
                        }
                    }
                }
                flush(c, &mut run, run_x, run_color);
                if let Some(col) = cur_col {
                    // With wrap, the cursor sits on the row holding its column;
                    // at a full row's end, on that row's last cell.
                    let on_row = if wraps {
                        (col / cols).min(segs - 1) == seg
                    } else {
                        col >= self.pan && col < self.pan + cols
                    };
                    if on_row {
                        let x = x0 + (col - c0).min(cols - 1) as i32 * look.cw;
                        let ink = if self.mode == Mode::View {
                            t.c.muted
                        } else {
                            t.c.accent
                        };
                        c.fill(Rect::new(x, y, theme::TERM_CURSOR_W, look.ch), ink);
                    }
                }
                r += 1;
            }
            first_seg = 0;
            line_no = line_no.map(|n| n + 1);
            if end >= len {
                break;
            }
            start = end + 1;
        }
    }
}

/// `f()`, unless `cell` holds it for `key` already.
fn remember(cell: &Cell<Option<(Memo, u32)>>, key: Memo, f: impl FnOnce() -> u32) -> u32 {
    if let Some((k, c)) = cell.get()
        && k == key
    {
        return c;
    }
    let c = f();
    cell.set(Some((key, c)));
    c
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
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let per = hex_per((area.w / look.cw) as usize);
    let rows = (area.h / look.ch) as usize;
    let top = top / per as u64 * per as u64;
    let len = b.len();
    let sel = mix(t.c.bg, t.c.accent, theme::TEXT_SELECT);
    let c = ui.canvas();
    // On the stack: a frame asks nothing of the heap.
    let mut row = [0u8; 16];
    let mut off_text = [0u8; 16];
    for r in 0..rows {
        let off = top + (r * per) as u64;
        if off >= len && r > 0 {
            break;
        }
        let mut got = 0;
        let _ = b.for_each(off..(off + per as u64).min(len), |chunk| {
            row[got..got + chunk.len()].copy_from_slice(chunk);
            got += chunk.len();
            true
        });
        let y = area.y + r as i32 * look.ch + look.base;
        let x = area.x + look.cw / 2;
        let digits = 8.max(16 - off.leading_zeros() as usize / 4);
        for (i, d) in off_text[..digits].iter_mut().enumerate() {
            *d = HEX[(off >> (4 * (digits - 1 - i)) & 15) as usize];
        }
        let text = std::str::from_utf8(&off_text[..digits]).unwrap_or("");
        look.font.draw(c, x, y, text, t.c.muted);
        for (i, &byte) in row[..got].iter().enumerate() {
            let hx = x + (9 + 3 * i) as i32 * look.cw;
            let role = if byte == 0 {
                Role::Comment
            } else {
                Role::Number
            };
            if off + i as u64 == cursor {
                c.fill(
                    Rect::new(hx - 1, y - look.base, 2 * look.cw + 2, look.ch),
                    sel,
                );
            }
            let pair = [HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]];
            let pair = std::str::from_utf8(&pair).unwrap_or("");
            look.font.draw(c, hx, y, pair, t.c.syntax(role));
            let ax = x + (9 + 3 * per + 1 + i) as i32 * look.cw;
            let shown = [if byte.is_ascii_graphic() { byte } else { b'.' }];
            let ink = if byte.is_ascii_graphic() {
                t.c.syntax(Role::String)
            } else {
                t.c.muted
            };
            look.font
                .draw(c, ax, y, std::str::from_utf8(&shown).unwrap_or("."), ink);
        }
    }
}

/// The chooser's filters for text: text first, then everything.
fn text_types(spec: PickSpec) -> PickSpec {
    spec.filter(
        "Text",
        &[
            "text/plain",
            "application/json",
            "application/xml",
            "application/x-shellscript",
        ],
    )
    .filter("All files", &["*"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &[u8]) -> Editor {
        let d = std::env::temp_dir().join(format!("gh0st-text-pair-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("pair.rs");
        std::fs::write(&p, text).unwrap();
        Editor::open(Some(&p), Options::default())
    }

    #[test]
    fn a_bracket_finds_its_pair_both_ways() {
        let mut e = editor(b"f(a[1], {b}) x");
        e.cursor = 1;
        assert_eq!(e.find_pair(), Some((1, 11)));
        // Just after a closer counts too.
        e.cursor = 12;
        assert_eq!(e.find_pair(), Some((1, 11)));
        e.cursor = 8;
        assert_eq!(e.find_pair(), Some((8, 10)));
        e.cursor = 13;
        assert_eq!(e.find_pair(), None);
        let mut open = editor(b"(((");
        open.cursor = 0;
        assert_eq!(open.find_pair(), None, "no pair, no mark");
    }
}
