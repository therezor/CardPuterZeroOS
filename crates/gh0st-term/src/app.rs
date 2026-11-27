//! The terminal page: tabs of PTYs, keys in, cells out, and scroll mode for
//! everything that isn't typing: scroll, pan, zoom, search, copy, tabs.
//! Every key outside typing is a command, so the app menu, Help
//! and the hint bar come from one list.

use crate::keys;
use crate::panel::Panel;
use crate::prefs::{self, Prefs, key};
use crate::pty::{self, Pty};
use crate::render::{self, Colors, Cursor, Look, Span};
use crate::view::{Point, Pos, Src, View};
use crate::vt::cell::{Cell, Clusters};
use crate::vt::history::flag::PROMPT;
use crate::vt::{CursorShape, Terminal, Vt, event};
use gh0st_app::gfx::widgets::{self, Hint};
use gh0st_app::gfx::{Rect, Theme, theme};
use gh0st_app::prelude::*;
use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::code;
use std::cell::RefCell;
use std::io;
use std::path::PathBuf;

/// Most bytes read from one PTY per loop turn, so keys stay answered under
/// a flood.
const READ_MAX: usize = 64 * 1024;
/// About one panel refresh: under a flood the latest state is drawn at most
/// this often, and output waits at most this long to be drawn.
const FRAME_MS: u64 = 20;
/// A quiet gap this long ends a repaint. htop and others repaint in several
/// writes; drawing between them flashes, so each write waits again.
const SETTLE_MS: u64 = 4;
/// Notifications from programs, at most one this often.
const NOTICE_GAP_MS: u128 = 3000;
/// Synchronized output may hold the screen this long at most.
const SYNC_MS: u64 = 150;
/// After the panel opens or closes, look again this much later.
const PANEL_AGAIN_MS: u64 = 250;
/// The most one paste sends to the program, so a big clipboard can't
/// flood it.
const PASTE_MAX: usize = 64 * 1024;

/// What `gh0st-term` was started with.
pub struct Options {
    /// `-e cmd args...`: run this instead of a shell, in one tab.
    pub command: Option<Vec<String>>,
    /// `GH0ST_SYSPLAUSE=1`: after the command ends, wait for a key.
    pub sysplause: bool,
}

impl Options {
    pub fn from_args(args: impl Iterator<Item = String>) -> Options {
        let args: Vec<String> = args.collect();
        let mut command = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--sim" => i += 1,
                "-e" => {
                    command = Some(args[i + 1..].to_vec()).filter(|c| !c.is_empty());
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        Options {
            command,
            sysplause: std::env::var("GH0ST_SYSPLAUSE").is_ok_and(|v| v == "1"),
        }
    }
}

/// How the grid relates to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// The grid is a fixed size, 80x24 by default. Zoom only makes the
    /// text bigger; the screen follows the cursor and pans over the grid.
    Magnify,
    /// The grid is what fits at the current size; zoom reflows it.
    Fit,
}

impl Layout {
    fn from_key(s: &str) -> Layout {
        if s == "fit" {
            Layout::Fit
        } else {
            Layout::Magnify
        }
    }

    fn key(self) -> &'static str {
        match self {
            Layout::Magnify => "magnify",
            Layout::Fit => "fit",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Layout::Magnify => "Magnify",
            Layout::Fit => "Fit",
        }
    }
}

/// Command ids.
mod cmd {
    pub const SCROLL_MODE: u32 = 1;
    pub const SIZE: u32 = 2;
    pub const LAYOUT: u32 = 3;
    pub const FIND: u32 = 4;
    pub const COPY: u32 = 5;
    pub const PASTE: u32 = 6;
    pub const NEW_TAB: u32 = 7;
    pub const CLOSE_TAB: u32 = 8;
    pub const TAB: u32 = 9;
    pub const BAR: u32 = 10;
    pub const SETTINGS: u32 = 12;
    pub const SEND_ESC: u32 = 13;
    pub const SELECT_MODE: u32 = 14;
    pub const SCROLL: u32 = 20;
    pub const PAN: u32 = 21;
    pub const PAGE: u32 = 22;
    pub const ENDS: u32 = 23;
    pub const PROMPT: u32 = 24;
    pub const HOME: u32 = 25;
    pub const KEYS: u32 = 26;
    pub const DONE: u32 = 27;
    pub const HISTORY: u32 = 30;
    pub const TYPED_SIZE: u32 = 31;
    pub const OLDER: u32 = 40;
    pub const NEWER: u32 = 41;
    pub const FIND_DONE: u32 = 42;
    pub const EXTEND: u32 = 50;
    pub const YANK: u32 = 51;
    pub const CANCEL: u32 = 52;
    pub const SIDE: u32 = 53;
    pub const WORD: u32 = 54;
    pub const SEL_PAGE: u32 = 55;
    pub const SEL_ENDS: u32 = 56;
    pub const MARK: u32 = 57;
    pub const LINES: u32 = 58;
}

/// Names of the modes in Help.
const TYPING: &str = "Typing";
const SCROLLING: &str = "Scroll mode";
const FINDING: &str = "Find";
const SELECTING: &str = "Select";

struct Tab {
    vt: Box<Vt>,
    pty: Option<Pty>,
    watch: Option<Watch>,
    pid: i32,
    view: View,
    /// Synchronized output began: the hold ends at this timer.
    sync: Option<Timer>,
}

/// How the selection's head moves.
#[derive(Clone, Copy)]
enum Move {
    Up,
    Down,
    Left,
    Right,
    WordLeft,
    WordRight,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Typing,
    /// Scroll mode: keys scroll, pan, zoom and manage tabs.
    View,
    /// Typing a search.
    Find(String),
    /// A cursor of its own moves over the text. From the anchor, once set,
    /// to the head is selected; whole rows with `lines`.
    Select {
        anchor: Option<Point>,
        head: Point,
        lines: bool,
    },
}

/// A search match: the row and its columns.
#[derive(Clone, Copy)]
struct Hit {
    pos: Pos,
    cols: (usize, usize),
}

pub struct Term {
    opts: Options,
    tabs: Vec<Tab>,
    cur: usize,
    buf: Vec<u8>,
    out: Vec<u8>,
    title: String,
    zoom: usize,
    /// Window mode's grid; None is fit mode.
    layout: Layout,
    window_size: (usize, usize),
    bar: bool,
    alt_esc: bool,
    history: usize,
    cursor: CursorShape,
    area: Rect,
    mode: Mode,
    query: String,
    hit: Option<Hit>,
    /// The command of `-e` ended: the next key quits.
    waiting: bool,
    /// When the last notification went out.
    noticed: Option<std::time::Instant>,
    /// The clock for each tab's reply budget.
    epoch: std::time::Instant,
    /// Output came in: draw when it stops for `SETTLE_MS`, so a program's
    /// repaint lands whole rather than half done.
    settle: Option<Timer>,
    /// Or when this fires, `FRAME_MS` after the first byte, under a flood.
    cap: Option<Timer>,
    /// The screen is held: a program in the tab in view is mid-repaint in
    /// synchronized output, or a program draws on the panel itself.
    held: bool,
    /// Who opens the panel, and whether a program in a tab has it.
    panel: Option<(Panel, Watch)>,
    yielded: bool,
    /// A second look, in case the opener's fd wasn't in place at the first.
    panel_again: Option<Timer>,
    /// The hint bar's height, from the theme.
    hint_h: i32,
    /// Scroll mode has said once that its letters are shortcuts.
    told_keys: bool,
    /// The tab whose output waits while its text is selected, so the text
    /// holds still.
    frozen: Option<Watch>,
    scratch: RefCell<(Vec<Cell>, Clusters)>,
}

fn parse_size(s: &str) -> Option<(usize, usize)> {
    let (c, r) = s.split_once('x')?;
    let (c, r) = (c.trim().parse().ok()?, r.trim().parse().ok()?);
    ((20..=300).contains(&c) && (5..=200).contains(&r)).then_some((c, r))
}

impl Term {
    pub fn new(opts: Options) -> Term {
        Term {
            opts,
            tabs: Vec::new(),
            cur: 0,
            buf: vec![0; 16 * 1024],
            out: Vec::new(),
            title: "Terminal".into(),
            zoom: 0,
            layout: Layout::Magnify,
            window_size: (80, 24),
            bar: true,
            alt_esc: true,
            history: 1 << 20,
            cursor: CursorShape::Block,
            area: Rect::new(0, 0, 320, 152),
            mode: Mode::Typing,
            query: String::new(),
            hit: None,
            waiting: false,
            noticed: None,
            epoch: std::time::Instant::now(),
            settle: None,
            cap: None,
            held: false,
            panel: None,
            yielded: false,
            panel_again: None,
            hint_h: 18,
            told_keys: false,
            frozen: None,
            scratch: RefCell::new((Vec::new(), Clusters::default())),
        }
    }

    fn load(&mut self, cx: &mut Cx) {
        let s = prefs::Settings::from(cx.settings());
        self.zoom = s.zoom.min(theme::TERM_TERMINUS.len() - 1);
        self.window_size = parse_size(&s.window_size).unwrap_or((80, 24));
        self.layout = Layout::from_key(&s.mode);
        self.bar = s.bar;
        self.alt_esc = s.alt_esc;
        self.history = (s.history_kb as usize) << 10;
        self.cursor = match s.cursor.as_str() {
            "underline" => CursorShape::Underline,
            "bar" => CursorShape::Bar,
            _ => CursorShape::Block,
        };
    }

    fn look(&self) -> Look {
        Look::of(&theme::TERM_TERMINUS[self.zoom])
    }

    /// The grid size, by layout.
    fn grid_size(&self) -> (usize, usize) {
        let fits = |look: Look| {
            (
                (self.area.w / look.cw).max(2) as usize,
                (self.area.h / look.ch).max(1) as usize,
            )
        };
        match self.layout {
            Layout::Fit => fits(self.look()),
            Layout::Magnify => self.window_size,
        }
    }

    /// The grid, when the screen is a window over it rather than all of it.
    fn window(&self) -> Option<(usize, usize)> {
        (self.layout != Layout::Fit).then(|| self.grid_size())
    }

    /// Cells on screen.
    fn vis(&self) -> (usize, usize) {
        let look = self.look();
        let (cols, rows) = self.grid_size();
        (
            ((self.area.w / look.cw).max(1) as usize).min(cols),
            (((self.area.h - self.reserve()) / look.ch).max(1) as usize).min(rows),
        )
    }

    /// Rows at the bottom that a mode's bars take: the text ends above
    /// them rather than under them.
    fn reserve(&self) -> i32 {
        match self.mode {
            Mode::Typing => 0,
            Mode::Find(_) => 2 * self.hint_h,
            _ => self.hint_h,
        }
    }

    fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.cur)
    }

    /// Size, face or bars changed: every tab takes the new grid.
    fn relayout(&mut self, cx: &mut Cx) {
        // Rows move on a resize: a found row or a selection would point
        // at the wrong place, or past the grid.
        self.hit = None;
        if matches!(self.mode, Mode::Select { .. } | Mode::Find(_)) {
            self.mode = Mode::View;
        }
        let t = *cx.theme();
        self.area = self.bars().content(&t);
        self.hint_h = t.m.hint_h;
        let (cols, rows) = self.grid_size();
        let window = self.window();
        for tab in &mut self.tabs {
            tab.vt.t.resize(cols, rows);
            if let Some(p) = &tab.pty {
                let _ = p.resize(cols as u16, rows as u16);
            }
            tab.view.window = window;
            tab.view.snap();
        }
        self.track();
        self.thaw(cx);
        cx.redraw();
    }

    /// Keeps the current tab's window on its cursor.
    fn track(&mut self) {
        let grid = self.grid_size();
        let vis = self.vis();
        if let Some(tab) = self.tabs.get_mut(self.cur) {
            let c = tab.vt.t.cursor();
            tab.view.follow_cursor(c, grid, vis);
        }
    }

    fn open_tab(&mut self, cx: &mut Cx) -> io::Result<()> {
        let (cols, rows) = self.grid_size();
        let (argv, arg0, cwd) = match &self.opts.command {
            Some(c) if self.tabs.is_empty() => (c.clone(), None, None),
            _ => {
                let sh = pty::shell();
                let name = std::path::Path::new(&sh)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sh".into());
                // A new tab starts where the shell in front last was.
                let cwd = self
                    .tab()
                    .and_then(|t| t.vt.t.cwd.clone())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
                (vec![sh], Some(name), cwd)
            }
        };
        let pty = Pty::spawn(
            &argv,
            arg0.as_deref(),
            cwd.as_deref(),
            cols as u16,
            rows as u16,
        )?;
        let watch = cx.watch(pty.fd());
        cx.adopt(pty.pid);
        let budget = self.history / (self.tabs.len() + 1);
        for t in &mut self.tabs {
            t.vt.t.history.set_max(budget);
        }
        let mut vt = Box::new(Vt::new(cols, rows, budget));
        vt.t.default_shape = self.cursor;
        vt.t.shape = self.cursor;
        self.tabs.push(Tab {
            vt,
            pid: pty.pid,
            pty: Some(pty),
            watch: Some(watch),
            view: View::new(self.window()),
            sync: None,
        });
        self.cur = self.tabs.len() - 1;
        self.retitle();
        cx.redraw();
        Ok(())
    }

    /// Drops tab `i`: its PTY closes and its session hangs up. A `-e`
    /// command with sysplause stays on screen until a key instead.
    fn close_tab(&mut self, i: usize, cx: &mut Cx) {
        if self.waiting {
            return;
        }
        self.hit = None;
        if matches!(self.mode, Mode::Select { .. }) {
            self.mode = Mode::View;
        }
        let last = self.tabs.len() == 1;
        let Some(tab) = self.tabs.get_mut(i) else {
            return;
        };
        if let Some(w) = tab.watch.take() {
            cx.unwatch(w);
        }
        tab.pty = None;
        if self.opts.command.is_some() && self.opts.sysplause && last {
            tab.vt.feed(b"\r\n\x1b[0m-- press any key --");
            self.waiting = true;
            self.mode = Mode::Typing;
            cx.redraw();
            return;
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            cx.quit();
            return;
        }
        let budget = self.history / self.tabs.len();
        for t in &mut self.tabs {
            t.vt.t.history.set_max(budget);
        }
        if self.cur > i || self.cur >= self.tabs.len() {
            self.cur = self.cur.saturating_sub(1);
        }
        self.hold(cx);
        self.retitle();
        cx.redraw();
    }

    fn switch(&mut self, i: usize, cx: &mut Cx) {
        if i < self.tabs.len() && i != self.cur {
            // A selection is the old tab's.
            if matches!(self.mode, Mode::Select { .. }) {
                self.mode = Mode::View;
            }
            self.front(false, cx);
            self.cur = i;
            self.front(true, cx);
            self.hit = None;
            self.retitle();
            self.track();
            self.hold(cx);
            cx.redraw();
        }
    }

    /// Reads what tab `i` printed, up to the per-turn cap. True if the
    /// child hung up.
    fn drain(&mut self, i: usize, cx: &mut Cx) -> bool {
        let tab = &mut self.tabs[i];
        let Some(pty) = &tab.pty else {
            return true;
        };
        let mut total = 0;
        let mut hup = false;
        tab.vt
            .t
            .budget
            .refill(self.epoch.elapsed().as_millis() as u64);
        while total < READ_MAX {
            match pty.read(&mut self.buf) {
                Ok(0) => {
                    hup = true;
                    break;
                }
                Ok(n) => {
                    tab.vt.feed(&self.buf[..n]);
                    total += n;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    hup = true;
                    break;
                }
            }
        }
        let t = &mut tab.vt.t;
        if !t.replies.is_empty() {
            if let Some(w) = tab.watch {
                cx.write(w, &t.replies);
            }
            t.replies.clear();
        }
        // OSC 52: a program in the tab in view copies, to the clipboard
        // every app shares. A tab out of sight doesn't.
        if let Some(text) = t.clipboard.take() {
            if i == self.cur && cx.copy(text.as_bytes()) {
                cx.toast("Copied");
            }
        }
        let notice = t.notice.take();
        let events = std::mem::take(&mut t.events);
        // A bell matters when its tab can't be seen.
        let bell = events & event::BELL != 0 && (i != self.cur || !cx.in_front());
        // The text comes from whatever runs in the tab, perhaps a remote
        // host: say where it is from, and not more than one a while.
        let text = notice.or_else(|| bell.then(|| format!("bell in tab {}", i + 1)));
        if let Some(text) = text {
            let now = std::time::Instant::now();
            if self
                .noticed
                .is_none_or(|t| now.duration_since(t).as_millis() >= NOTICE_GAP_MS)
            {
                self.noticed = Some(now);
                cx.notify(&format!("Terminal: {text}"));
            }
        }
        // Synchronized output: draw when it ends, or after SYNC_MS.
        match (t.modes.sync, tab.sync) {
            (true, None) => tab.sync = Some(cx.after(SYNC_MS)),
            (false, Some(timer)) => {
                cx.cancel(timer);
                tab.sync = None;
            }
            _ => {}
        }
        let synced = tab.sync.is_some();
        if events & event::TITLE != 0 && i == self.cur {
            self.retitle();
        }
        if i == self.cur {
            if (total > 0 || hup) && !synced {
                self.settle_later(cx);
            }
            self.hold(cx);
        }
        hup
    }

    /// Whether a program in a tab has the panel: if so, it gets the screen.
    fn check_panel(&mut self, cx: &mut Cx) {
        let Some((p, _)) = &self.panel else {
            return;
        };
        let sessions: Vec<i32> = self.tabs.iter().map(|t| t.pid).collect();
        let held = p.held(&sessions);
        if held != self.yielded {
            self.yielded = held;
            // What the program can't see must not take its keys.
            if held && self.mode != Mode::Typing {
                self.mode = Mode::Typing;
                self.hit = None;
                self.thaw(cx);
            }
            self.hold(cx);
        }
    }

    /// Output came: draw once it pauses, or at the cap.
    fn settle_later(&mut self, cx: &mut Cx) {
        if let Some(t) = self.settle.take() {
            cx.cancel(t);
        }
        self.settle = Some(cx.after(SETTLE_MS));
        if self.cap.is_none() {
            self.cap = Some(cx.after(FRAME_MS));
        }
    }

    /// Holds the screen while the tab in view is in synchronized output, so
    /// no redraw, a key's included, shows half a repaint, and while a
    /// program in a tab draws on the panel itself.
    fn hold(&mut self, cx: &mut Cx) {
        let held = self.yielded || self.tab().is_some_and(|t| t.sync.is_some());
        if held != self.held {
            self.held = held;
            cx.hold_screen(held);
        }
    }

    fn retitle(&mut self) {
        let name = self
            .tab()
            .map(|t| t.vt.t.title.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Terminal");
        self.title = if self.tabs.len() > 1 {
            format!("{}/{} {name}", self.cur + 1, self.tabs.len())
        } else {
            name.to_string()
        };
    }

    fn zoom_to(&mut self, zoom: usize, cx: &mut Cx) {
        let zoom = zoom.min(theme::TERM_TERMINUS.len() - 1);
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        self.relayout(cx);
        cx.settings().set_int(key::ZOOM, zoom as i64);
        let look = self.look();
        let (c, r) = self.grid_size();
        cx.toast(&format!("Text {}x{}, grid {c}x{r}", look.cw, look.ch));
    }

    /// Magnify or fit.
    fn next_layout(&mut self, cx: &mut Cx) {
        self.layout = match self.layout {
            Layout::Magnify => Layout::Fit,
            Layout::Fit => Layout::Magnify,
        };
        self.relayout(cx);
        cx.settings().set_str(key::MODE, self.layout.key());
        let (c, r) = self.grid_size();
        cx.toast(&format!("{}, grid {c}x{r}", self.layout.name()));
    }

    /// Writes bytes to the program in front, back at the cursor.
    fn send(&mut self, bytes: &[u8], cx: &mut Cx) {
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        if !tab.view.live() || !tab.view.follow {
            tab.view.snap();
            self.track();
            cx.redraw();
        }
        if let Some(w) = self.tab().and_then(|t| t.watch) {
            cx.write(w, bytes);
        }
    }

    fn scroll(&mut self, up: bool, n: usize, cx: &mut Cx) {
        let vis = self.vis();
        if let Some(tab) = self.tabs.get_mut(self.cur) {
            tab.view.scroll(up, n, &tab.vt.t, vis.1);
            cx.redraw();
        }
    }

    fn pan(&mut self, left: bool, cx: &mut Cx) {
        let vis = self.vis();
        let step = (vis.0 / 2).max(1) as isize;
        let cols = self.grid_size().0;
        if let Some(tab) = self.tabs.get_mut(self.cur) {
            tab.view.pan(if left { -step } else { step }, cols, vis.0);
            cx.redraw();
        }
    }

    /// One command, from its key or the app menu.
    fn run(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let back = step == Step::Prev;
        let page = self.vis().1.saturating_sub(1).max(1);
        match id {
            cmd::SCROLL_MODE if self.mode == Mode::Typing => {
                self.mode = Mode::View;
                if !self.told_keys {
                    self.told_keys = true;
                    cx.toast("Scroll mode: letters are shortcuts, h lists them");
                }
            }
            cmd::SCROLL_MODE | cmd::DONE => self.leave_view(cx),
            cmd::SIZE | cmd::TYPED_SIZE if back => self.zoom_to(self.zoom.saturating_sub(1), cx),
            cmd::SIZE | cmd::TYPED_SIZE => self.zoom_to(self.zoom + 1, cx),
            cmd::LAYOUT => self.next_layout(cx),
            cmd::FIND => self.mode = Mode::Find(self.query.clone()),
            cmd::COPY => self.start_select(cx),
            cmd::SELECT_MODE if matches!(self.mode, Mode::Select { .. }) => self.leave_view(cx),
            cmd::SELECT_MODE => self.start_select(cx),
            cmd::PASTE => {
                self.mode = Mode::Typing;
                self.paste(cx);
            }
            cmd::NEW_TAB => match self.open_tab(cx) {
                Ok(()) => self.mode = Mode::Typing,
                Err(e) => cx.toast(&format!("No new tab: {e}")),
            },
            cmd::CLOSE_TAB => {
                let i = self.cur;
                self.close_tab(i, cx);
            }
            cmd::TAB => {
                let n = self.tabs.len().max(1);
                let to = match step {
                    Step::Nth(i) => i,
                    Step::Prev => (self.cur + n - 1) % n,
                    _ => (self.cur + 1) % n,
                };
                self.switch(to, cx);
            }
            cmd::BAR => {
                self.bar = !self.bar;
                self.relayout(cx);
                cx.settings().set_bool(key::BAR, self.bar);
            }
            cmd::SETTINGS => cx.push(Prefs::new()),
            cmd::SEND_ESC => {
                self.mode = Mode::Typing;
                self.hit = None;
                self.send(b"\x1b", cx);
            }
            cmd::SCROLL => self.scroll(back, 1, cx),
            cmd::PAN => self.pan(back, cx),
            cmd::HISTORY => {
                self.mode = Mode::View;
                self.scroll(back, page, cx);
            }
            cmd::PAGE => self.scroll(back, page, cx),
            cmd::ENDS => self.scroll(back, usize::MAX / 2, cx),
            cmd::PROMPT => self.jump_prompt(back, cx),
            cmd::HOME => {
                let app = self.tab().is_some_and(|t| t.vt.t.modes.app_cursor);
                self.mode = Mode::Typing;
                self.send(if app { b"\x1bOH" } else { b"\x1b[H" }, cx);
            }
            cmd::KEYS => cx.show_keys(),
            cmd::OLDER | cmd::NEWER => self.find_step(id == cmd::OLDER, cx),
            cmd::FIND_DONE => self.mode = Mode::View,
            cmd::CANCEL => self.leave_view(cx),
            cmd::EXTEND => self.move_head(if back { Move::Up } else { Move::Down }, 1, cx),
            cmd::SIDE => self.move_head(if back { Move::Left } else { Move::Right }, 1, cx),
            cmd::WORD => self.move_head(
                if back {
                    Move::WordLeft
                } else {
                    Move::WordRight
                },
                1,
                cx,
            ),
            cmd::SEL_PAGE => {
                let m = if back { Move::Up } else { Move::Down };
                self.move_head(m, page, cx);
            }
            cmd::SEL_ENDS => self.head_to_end(back, cx),
            cmd::MARK => {
                if let Mode::Select { anchor, head, .. } = &mut self.mode {
                    *anchor = Some(*head);
                }
            }
            cmd::LINES => {
                // Whole lines start on the head's line if nothing is marked.
                if let Mode::Select {
                    lines,
                    anchor,
                    head,
                } = &mut self.mode
                {
                    *lines = !*lines;
                    anchor.get_or_insert(*head);
                }
            }
            cmd::YANK => self.yank(cx),
            _ => {}
        }
        cx.redraw();
    }

    /// Moves the view to the previous or next shell prompt (OSC 133).
    fn jump_prompt(&mut self, up: bool, cx: &mut Cx) {
        let vis = self.vis();
        let fit = self.layout == Layout::Fit;
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        let t = &tab.vt.t;
        let is_prompt = |p: Pos| match p.src(t) {
            Src::Grid(y) => !t.alt_active && t.grid().rows.get(y).is_some_and(|r| r.prompt),
            Src::Hist(i, sub) => sub == 0 && t.history.line(i).flags & PROMPT != 0,
        };
        let mut p = tab.view.top;
        let found = loop {
            let next = if up { p.up(t) } else { p.down(t) };
            if next == p {
                break None;
            }
            p = next;
            if is_prompt(p) {
                break Some(p);
            }
        };
        match found {
            Some(Pos::Grid(_)) if fit => tab.view.snap(),
            Some(Pos::Grid(y)) => tab.view.reveal(Pos::Grid(y), t, vis.1),
            Some(p) => {
                tab.view.top = p;
                tab.view.follow = false;
            }
            None => cx.toast("No more prompts"),
        }
        cx.redraw();
    }

    fn leave_view(&mut self, cx: &mut Cx) {
        self.mode = Mode::Typing;
        self.hit = None;
        if let Some(tab) = self.tabs.get_mut(self.cur) {
            tab.view.snap();
        }
        self.track();
        cx.redraw();
    }

    /// A key in find mode: it edits the query. Enter, Fn+F, Fn+X and Esc
    /// are commands.
    fn find_key(&mut self, k: &RawKey, cx: &mut Cx) {
        let Mode::Find(q) = &mut self.mode else {
            return;
        };
        if k.code == code::BACKSPACE {
            q.pop();
        } else if let Some(c) = k.text.filter(|c| !c.is_control()) {
            q.push(c);
        }
        cx.redraw();
    }

    /// The next older or newer match of what is typed.
    fn find_step(&mut self, back: bool, cx: &mut Cx) {
        let typed = match &self.mode {
            Mode::Find(q) if *q != self.query => Some(q.clone()),
            _ => None,
        };
        if let Some(q) = typed {
            self.query = q;
            self.hit = None;
        }
        self.find_next(back, cx);
    }

    /// Finds the query in an older (`back`) or newer row than the last
    /// hit, starting from the bottom.
    fn find_next(&mut self, back: bool, cx: &mut Cx) {
        if self.query.is_empty() {
            return;
        }
        let vis = self.vis();
        let needle = fold(&self.query);
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        let t = &tab.vt.t;
        let mut scratch = self.scratch.borrow_mut();
        let start = self.hit.map(|h| h.pos);
        let mut pos = start.unwrap_or(Pos::Grid(t.rows - 1));
        let mut first = start.is_none();
        let mut found = None;
        loop {
            if !first {
                let next = if back { pos.up(t) } else { pos.down(t) };
                if next == pos {
                    break;
                }
                pos = next;
            }
            first = false;
            let (text, cols) = row_text(t, pos.src(t), &mut scratch);
            let low = fold(&text);
            if let Some(b) = low.find(&needle) {
                let at = low[..b].chars().count();
                let n = needle.chars().count();
                let c0 = cols.get(at).copied().unwrap_or(0);
                let c1 = cols.get(at + n).copied().unwrap_or(t.cols);
                found = Some(Hit {
                    pos,
                    cols: (c0, c1),
                });
                break;
            }
        }
        drop(scratch);
        match found {
            Some(h) => {
                self.hit = Some(h);
                let cols = self.grid_size().0;
                let tab = &mut self.tabs[self.cur];
                tab.view.reveal(h.pos, &tab.vt.t, vis.1);
                // Pan to the match too, if it is off to the side.
                let v = &mut tab.view;
                if h.cols.0 < v.pan_x || h.cols.1 > v.pan_x + vis.0 {
                    let want = h.cols.0.saturating_sub(vis.0 / 4) as isize;
                    v.pan(want - v.pan_x as isize, cols, vis.0);
                }
            }
            None => cx.toast(&format!("No more \"{}\"", self.query)),
        }
        cx.redraw();
    }

    /// Select mode, its head where the eye is: the match found, the
    /// cursor, or the top of the view.
    fn start_select(&mut self, cx: &mut Cx) {
        let Some(tab) = self.tab() else {
            return;
        };
        let t = &tab.vt.t;
        let head = match self.hit {
            Some(h) => Point {
                pos: h.pos,
                col: h.cols.0,
            },
            None if tab.view.live() => {
                let (x, y) = t.cursor();
                Point {
                    pos: Pos::Grid(y),
                    col: x.min(t.cols - 1),
                }
            }
            None => Point {
                pos: tab.view.top,
                col: tab.view.pan_x,
            },
        };
        let watch = tab.watch;
        self.hit = None;
        self.mode = Mode::Select {
            anchor: None,
            head,
            lines: false,
        };
        // The program waits while its text is selected.
        if let Some(w) = watch {
            cx.hold_reads(w, true);
            self.frozen = Some(w);
        }
        cx.redraw();
    }

    /// Out of select mode: the program's output comes in again.
    fn thaw(&mut self, cx: &mut Cx) {
        if matches!(self.mode, Mode::Select { .. }) {
            return;
        }
        if let Some(w) = self.frozen.take() {
            cx.hold_reads(w, false);
        }
    }

    /// Moves the selection's head `n` times.
    fn move_head(&mut self, how: Move, n: usize, cx: &mut Cx) {
        let Mode::Select { head, .. } = self.mode else {
            return;
        };
        let Some(tab) = self.tabs.get(self.cur) else {
            return;
        };
        let t = &tab.vt.t;
        let mut scratch = self.scratch.borrow_mut();
        let mut p = head;
        for _ in 0..n {
            p = step(t, p, how, &mut scratch);
        }
        drop(scratch);
        self.place_head(p, cx);
    }

    /// The head to the oldest line, or back to the cursor.
    fn head_to_end(&mut self, oldest: bool, cx: &mut Cx) {
        let Some(tab) = self.tab() else {
            return;
        };
        let t = &tab.vt.t;
        let p = if oldest {
            let pos = if t.history.is_empty() {
                Pos::Grid(0)
            } else {
                Pos::Hist(t.history.id(0), 0)
            };
            Point { pos, col: 0 }
        } else {
            let (x, y) = t.cursor();
            Point {
                pos: Pos::Grid(y),
                col: x.min(t.cols - 1),
            }
        };
        self.place_head(p, cx);
    }

    /// Sets the head and brings it into view, panning to its column too.
    fn place_head(&mut self, p: Point, cx: &mut Cx) {
        let vis = self.vis();
        let cols = self.grid_size().0;
        if let Mode::Select { head, .. } = &mut self.mode {
            *head = p;
        }
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        tab.view.reveal(p.pos, &tab.vt.t, vis.1);
        let v = &mut tab.view;
        if p.col < v.pan_x || p.col >= v.pan_x + vis.0 {
            let want = p.col.saturating_sub(vis.0 / 2) as isize;
            v.pan(want - v.pan_x as isize, cols, vis.0);
        }
        cx.redraw();
    }

    /// Copies the selection and goes back to typing.
    fn yank(&mut self, cx: &mut Cx) {
        let Mode::Select {
            anchor,
            head,
            lines,
        } = self.mode
        else {
            return;
        };
        let Some(anchor) = anchor.or(lines.then_some(head)) else {
            cx.toast("Space marks where it starts");
            return;
        };
        let Some(tab) = self.tab() else {
            return;
        };
        let text = selected_text(
            &tab.vt.t,
            anchor,
            head,
            lines,
            &mut self.scratch.borrow_mut(),
        );
        let copied = cx.copy(text.as_bytes());
        self.leave_view(cx);
        if !copied {
            cx.toast("Couldn't copy that");
        } else {
            let (n, what) = match lines {
                true => (text.lines().count().max(1), "line"),
                false => (text.chars().count(), "character"),
            };
            let s = if n == 1 { "" } else { "s" };
            cx.toast(&format!("Copied {n} {what}{s}"));
        }
        cx.redraw();
    }

    /// What the clipboard holds, never a copy of the terminal's own: the
    /// lock clears the clipboard, and nothing here may bring it back.
    fn paste(&mut self, cx: &mut Cx) {
        let Some(bytes) = cx.paste().filter(|b| !b.is_empty()) else {
            cx.toast("Nothing copied");
            return;
        };
        let mut end = bytes.len().min(PASTE_MAX);
        // Cut where a character starts.
        while end < bytes.len() && end > 0 && bytes[end] & 0xc0 == 0x80 {
            end -= 1;
        }
        let text = String::from_utf8_lossy(&bytes[..end]);
        let Some(tab) = self.tab() else {
            return;
        };
        let mut out = Vec::new();
        keys::paste(&text, &tab.vt.t.modes, &mut out);
        self.send(&out, cx);
        if end < bytes.len() {
            cx.toast(&format!("Pasted the first {} KB", PASTE_MAX >> 10));
        }
    }

    /// A key no command took, by mode. Releases do nothing.
    fn raw_key(&mut self, k: &RawKey, cx: &mut Cx) {
        if k.state == KeyState::Release {
            return;
        }
        if self.waiting {
            if k.state == KeyState::Press {
                cx.quit();
            }
            return;
        }
        match self.mode {
            Mode::View | Mode::Select { .. } => {}
            Mode::Find(_) => self.find_key(k, cx),
            Mode::Typing => {
                let Some(modes) = self.tab().map(|t| t.vt.t.modes) else {
                    return;
                };
                self.out.clear();
                keys::encode(k, &modes, self.alt_esc, &mut self.out);
                if !self.out.is_empty() {
                    let out = std::mem::take(&mut self.out);
                    self.send(&out, cx);
                    self.out = out;
                }
            }
        }
    }

    /// A mode with a bar at the bottom shows fewer rows: keep the cursor
    /// in sight when that changes.
    fn after_mode(&mut self, before: i32, cx: &mut Cx) {
        if self.reserve() != before {
            self.track();
            cx.redraw();
        }
    }
}

/// The text of one row on screen, and the column each char starts at.
fn row_text(t: &Terminal, src: Src, scratch: &mut (Vec<Cell>, Clusters)) -> (String, Vec<usize>) {
    let cells: &[Cell] = match src {
        Src::Grid(y) => match t.grid().rows.get(y) {
            Some(r) => &r.cells,
            None => return (String::new(), Vec::new()),
        },
        Src::Hist(i, sub) => {
            scratch.1.clear();
            t.history
                .line(i)
                .cells(t.cols, sub, &mut scratch.0, &mut scratch.1);
            &scratch.0
        }
    };
    let clusters = match src {
        Src::Grid(_) => &t.clusters,
        Src::Hist(..) => &scratch.1,
    };
    let mut text = String::new();
    let mut cols = Vec::new();
    for (x, c) in cells.iter().enumerate().filter(|(_, c)| !c.is_spacer()) {
        let before = text.chars().count();
        clusters.push_text(c, &mut text);
        for _ in before..text.chars().count() {
            cols.push(x);
        }
    }
    let trimmed = text.trim_end().len();
    text.truncate(trimmed);
    (text, cols)
}

/// From `a` to `b`, either way round, as text: whole rows with `lines`.
/// Rows that autowrap joined stay one line.
fn selected_text(
    t: &Terminal,
    a: Point,
    b: Point,
    lines: bool,
    scratch: &mut (Vec<Cell>, Clusters),
) -> String {
    let (s, e) = if a.key() <= b.key() { (a, b) } else { (b, a) };
    let mut p = s.pos;
    let mut out = String::new();
    loop {
        let src = p.src(t);
        let (text, cols) = row_text(t, src, scratch);
        if lines {
            out.push_str(&text);
        } else {
            let c0 = if p == s.pos { s.col } else { 0 };
            let c1 = if p == e.pos { e.col } else { usize::MAX };
            out.extend(
                text.chars()
                    .zip(&cols)
                    .filter(|&(_, &c)| c >= c0 && c <= c1)
                    .map(|(ch, _)| ch),
            );
        }
        let joined = match src {
            Src::Grid(y) => t.grid().rows.get(y).is_some_and(|r| r.wrapped),
            Src::Hist(i, sub) => sub + 1 < t.history.line(i).rows(t.cols),
        };
        if p == e.pos {
            break;
        }
        if !joined {
            out.push('\n');
        }
        let next = p.down(t);
        if next == p {
            break;
        }
        p = next;
    }
    out
}

/// The columns of row `pos` the selection covers, if any.
fn sel_span(
    anchor: Option<Point>,
    head: Point,
    lines: bool,
    pos: Pos,
    cols: usize,
) -> Option<(usize, usize)> {
    let a = anchor.or(lines.then_some(head))?;
    let (s, e) = if a.key() <= head.key() {
        (a, head)
    } else {
        (head, a)
    };
    let k = pos.key();
    if k < s.pos.key() || k > e.pos.key() {
        return None;
    }
    if lines {
        return Some((0, cols));
    }
    let c0 = if k == s.pos.key() { s.col } else { 0 };
    let c1 = if k == e.pos.key() { e.col + 1 } else { cols };
    Some((c0, c1))
}

/// The row's cells, a history row's unpacked into `scratch`.
fn cells_of(t: &Terminal, pos: Pos, scratch: &mut (Vec<Cell>, Clusters)) -> Vec<Cell> {
    match pos.src(t) {
        Src::Grid(y) => t
            .grid()
            .rows
            .get(y)
            .map(|r| r.cells.clone())
            .unwrap_or_default(),
        Src::Hist(i, sub) => {
            scratch.1.clear();
            t.history
                .line(i)
                .cells(t.cols, sub, &mut scratch.0, &mut scratch.1);
            scratch.0.clone()
        }
    }
}

/// A column on the start of its cell, not on a wide char's right half.
fn snap(t: &Terminal, p: Point, scratch: &mut (Vec<Cell>, Clusters)) -> Point {
    let cells = cells_of(t, p.pos, scratch);
    let mut col = p.col.min(t.cols - 1);
    while col > 0 && cells.get(col).is_some_and(|c| c.is_spacer()) {
        col -= 1;
    }
    Point { pos: p.pos, col }
}

/// Word, space or other: a word runs while its class holds.
fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// Where each word of row `pos` starts.
fn word_starts(t: &Terminal, pos: Pos, scratch: &mut (Vec<Cell>, Clusters)) -> Vec<usize> {
    let (text, cols) = row_text(t, pos.src(t), scratch);
    let mut out = Vec::new();
    let mut prev = 0;
    for (ch, &col) in text.chars().zip(&cols) {
        let k = class(ch);
        if k != 0 && k != prev && out.last() != Some(&col) {
            out.push(col);
        }
        prev = k;
    }
    out
}

/// The column just past row `pos`'s text.
fn text_end(t: &Terminal, pos: Pos, scratch: &mut (Vec<Cell>, Clusters)) -> usize {
    let (text, cols) = row_text(t, pos.src(t), scratch);
    let n = text.chars().count();
    cols.get(n.wrapping_sub(1)).map_or(0, |&c| c + 1)
}

/// One step of the selection's head. Left and right run on to the row
/// before or after; word moves stop on the start of a word.
fn step(t: &Terminal, p: Point, how: Move, scratch: &mut (Vec<Cell>, Clusters)) -> Point {
    let up = p.pos.up(t);
    let down = p.pos.down(t);
    match how {
        Move::Up => snap(
            t,
            Point {
                pos: up,
                col: p.col,
            },
            scratch,
        ),
        Move::Down => snap(
            t,
            Point {
                pos: down,
                col: p.col,
            },
            scratch,
        ),
        Move::Left if p.col > 0 => snap(
            t,
            Point {
                pos: p.pos,
                col: p.col - 1,
            },
            scratch,
        ),
        Move::Left if up != p.pos => {
            let end = text_end(t, up, scratch).min(t.cols - 1);
            snap(t, Point { pos: up, col: end }, scratch)
        }
        Move::Right => {
            let cells = cells_of(t, p.pos, scratch);
            let wide = cells.get(p.col).is_some_and(|c| c.is_wide());
            let next = p.col + if wide { 2 } else { 1 };
            let end = text_end(t, p.pos, scratch);
            if next < t.cols && next <= end {
                Point {
                    pos: p.pos,
                    col: next,
                }
            } else if down != p.pos {
                Point { pos: down, col: 0 }
            } else {
                p
            }
        }
        Move::WordRight => {
            if let Some(&c) = word_starts(t, p.pos, scratch).iter().find(|&&c| c > p.col) {
                return Point { pos: p.pos, col: c };
            }
            let mut pos = p.pos;
            loop {
                let next = pos.down(t);
                if next == pos {
                    return p;
                }
                pos = next;
                if let Some(&c) = word_starts(t, pos, scratch).first() {
                    return Point { pos, col: c };
                }
            }
        }
        Move::WordLeft => {
            if let Some(&c) = word_starts(t, p.pos, scratch)
                .iter()
                .rev()
                .find(|&&c| c < p.col)
            {
                return Point { pos: p.pos, col: c };
            }
            let mut pos = p.pos;
            loop {
                let next = pos.up(t);
                if next == pos {
                    return p;
                }
                pos = next;
                if let Some(&c) = word_starts(t, pos, scratch).last() {
                    return Point { pos, col: c };
                }
            }
        }
        Move::Left => p,
    }
}

/// The first run said how Esc works here.
const TOLD: &str = "gh0st.term.esc_told";

impl Page for Term {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> &str {
        "square-terminal"
    }

    /// The program takes every key, so no action bar: its view modes draw
    /// their own hints.
    fn bars(&self) -> Bars {
        if self.bar {
            Bars::top_only()
        } else {
            Bars::none()
        }
    }

    fn tabs(&self) -> (usize, usize) {
        if self.tabs.len() > 1 {
            (self.tabs.len(), self.cur)
        } else {
            (0, 0)
        }
    }

    fn start(&mut self, cx: &mut Cx) {
        self.load(cx);
        // Esc opens the menu here: say once how a program gets it.
        if cx.settings().get_bool(TOLD) != Some(true) {
            cx.toast("Esc: menu. Fn Esc sends Esc");
            cx.settings().set_bool(TOLD, true);
        }
        let t = *cx.theme();
        self.area = self.bars().content(&t);
        if let Ok(p) = Panel::open(&gh0st_app::gfx::fb::find()) {
            let w = cx.watch(p.fd());
            self.panel = Some((p, w));
        }
        if let Err(e) = self.open_tab(cx) {
            eprintln!("gh0st-term: {e}");
            cx.quit();
        }
    }

    /// Back from the settings page: take what changed.
    fn resume(&mut self, cx: &mut Cx) {
        self.load(cx);
        let budget = self.history / self.tabs.len().max(1);
        for tab in &mut self.tabs {
            tab.vt.t.history.set_max(budget);
            tab.vt.t.default_shape = self.cursor;
            tab.vt.t.shape = self.cursor;
        }
        self.mode = Mode::Typing;
        self.relayout(cx);
    }

    /// Esc while typing: a tap opens the menu, with Send Esc and Close app
    /// in it, and a hold exits as in every app (`esc_menu`). The modes take
    /// Esc as a command and never get here.
    fn back(&mut self, cx: &mut Cx) -> bool {
        if self.yielded {
            // The program has the screen: Esc is its.
            self.send(b"\x1b", cx);
        } else if self.waiting {
            cx.quit();
        } else if self.mode == Mode::Typing {
            return true;
        } else {
            self.leave_view(cx);
            self.thaw(cx);
        }
        false
    }

    fn esc_menu(&self) -> bool {
        self.mode == Mode::Typing && !self.yielded && !self.waiting
    }

    fn ready(&mut self, w: Watch, cx: &mut Cx) {
        if let Some((p, pw)) = &self.panel
            && *pw == w
        {
            p.drain();
            self.check_panel(cx);
            if self.panel_again.is_none() {
                self.panel_again = Some(cx.after(PANEL_AGAIN_MS));
            }
            return;
        }
        let Some(i) = self.tabs.iter().position(|t| t.watch == Some(w)) else {
            return;
        };
        if self.drain(i, cx) {
            self.close_tab(i, cx);
        }
    }

    fn exited(&mut self, pid: i32, _: Option<i32>, cx: &mut Cx) {
        let Some(i) = self.tabs.iter().position(|t| t.pid == pid) else {
            return;
        };
        // Its last words may still be in the PTY.
        self.drain(i, cx);
        self.close_tab(i, cx);
    }

    fn timer(&mut self, timer: Timer, cx: &mut Cx) {
        if self.panel_again == Some(timer) {
            self.panel_again = None;
            self.check_panel(cx);
            return;
        }
        if self.settle == Some(timer) || self.cap == Some(timer) {
            for t in [self.settle.take(), self.cap.take()].into_iter().flatten() {
                if t != timer {
                    cx.cancel(t);
                }
            }
            self.track();
            cx.redraw();
            return;
        }
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if tab.sync == Some(timer) {
                tab.sync = None;
                if i == self.cur {
                    self.track();
                    self.hold(cx);
                    cx.redraw();
                }
                return;
            }
        }
    }

    fn raw_keys(&self) -> bool {
        true
    }

    /// Typing goes to the program; find mode types its query.
    fn typing(&self) -> bool {
        matches!(self.mode, Mode::Typing | Mode::Find(_))
    }

    fn commands(&self, c: &mut Commands) {
        // A program has the screen: every key is its.
        if self.yielded {
            return;
        }
        let typing = self.mode == Mode::Typing;
        let scroll = self.mode == Mode::View;
        let find = matches!(self.mode, Mode::Find(_));
        let select = matches!(self.mode, Mode::Select { .. });
        let look = self.look();
        let (gc, gr) = self.grid_size();
        let on = |c: Command| c.mode(SCROLLING, scroll);
        let moves = |c: Command| c.keys_only().repeats();

        // Scrolling back from typing enters scroll mode, as tmux does.
        c.add(
            moves(Command::choice(cmd::HISTORY, "Scroll back", ""))
                .keys("Shift+PgUp Shift+PgDn")
                .mode(TYPING, (typing || scroll) && !self.waiting),
        );
        // Fn+Space: two taps, as Fn is sticky, and no program reads Fn.
        let idle = !self.waiting;
        c.add(
            moves(Command::choice(cmd::TYPED_SIZE, "Text size", ""))
                .keys("Fn+r Fn+t")
                .mode(TYPING, typing && idle),
        );
        c.group("View");
        c.add(
            Command::action(cmd::SEND_ESC, "Send Esc")
                .keys("Fn+Esc")
                .mode(TYPING, idle),
        );
        c.add(
            Command::toggle(cmd::SCROLL_MODE, "Scroll mode", scroll)
                .keys("Fn+Space")
                .closes()
                .mode(TYPING, (typing || scroll) && idle),
        );
        // Fn+V: a mode of its own, as scroll mode is.
        c.add(
            Command::toggle(cmd::SELECT_MODE, "Select mode", select)
                .keys("Fn+v")
                .closes()
                .mode(TYPING, (typing || select) && idle),
        );
        let size = format!("{}x{}", look.cw, look.ch);
        c.add(on(Command::choice(cmd::SIZE, "Text size", size)
            .keys("r t")
            .repeats()));
        let grid = format!("{} {gc}x{gr}", self.layout.name());
        c.add(on(Command::choice(cmd::LAYOUT, "Grid", grid).keys("m")));
        c.add(on(Command::action(cmd::FIND, "Find").keys("n")));
        c.add(on(Command::action(cmd::COPY, "Select mode")
            .keys("v")
            .keys_only()));
        // Another app may have copied: Paste is always there.
        c.add(on(Command::action(cmd::PASTE, "Paste").keys("p")));
        c.group("Tabs");
        c.add(on(Command::action(cmd::NEW_TAB, "New tab").keys("o")));
        if self.tabs.len() > 1 {
            let at = format!("{} of {}", self.cur + 1, self.tabs.len());
            c.add(on(Command::choice(cmd::TAB, "Tab", at).keys("Tab 1-9")));
        }
        c.add(on(Command::action(cmd::CLOSE_TAB, "Close tab")
            .keys("w")
            .confirm()));
        c.group("Settings");
        c.add(on(Command::toggle(cmd::BAR, "Top bar", self.bar).keys("b")));
        c.add(on(Command::action(cmd::SETTINGS, "More settings")
            .keys("s")
            .opens()));

        // Scroll mode's moves have keys but no menu rows.
        let up_down = |id, label| on(moves(Command::choice(id, label, "")));
        c.add(
            up_down(cmd::SCROLL, "Scroll up, down")
                .keys("f x")
                .hint("scroll"),
        );
        c.add(up_down(cmd::PAN, "Pan left, right").keys("z c").hint("pan"));
        c.add(up_down(cmd::PAGE, "Page up, down").keys("PgUp PgDn"));
        c.add(up_down(cmd::PROMPT, "Previous, next prompt").keys("u i"));
        c.add(on(Command::choice(cmd::ENDS, "Oldest line, live", "")
            .keys("g G")
            .keys_only()));
        c.add(on(Command::action(cmd::HOME, "Send Home")
            .keys("k")
            .keys_only()));
        c.add(on(Command::action(cmd::KEYS, "Help")
            .keys("h")
            .keys_only()
            .hint("help")));
        c.add(on(Command::action(cmd::DONE, "Back to typing")
            .keys("Esc q")
            .keys_only()
            .hint("done")));

        let f = |c: Command| c.keys_only().mode(FINDING, find);
        c.add(f(Command::action(cmd::OLDER, "Older match")
            .keys("Enter Up")
            .repeats()
            .hint("older")));
        c.add(f(Command::action(cmd::NEWER, "Newer match")
            .keys("Down")
            .repeats()
            .hint("newer")));
        c.add(f(Command::action(cmd::FIND_DONE, "Done")
            .keys("Esc")
            .hint("done")));

        let s = |c: Command| c.keys_only().mode(SELECTING, select);
        c.add(s(Command::choice(cmd::EXTEND, "Up, down", "")
            .keys("f x")
            .repeats()
            .hint("move")));
        c.add(s(Command::choice(cmd::SIDE, "Left, right", "")
            .keys("z c")
            .repeats()));
        c.add(s(Command::choice(cmd::WORD, "Word left, right", "")
            .keys("Z C")
            .repeats()));
        c.add(s(Command::choice(cmd::SEL_PAGE, "Page up, down", "")
            .keys("PgUp PgDn")
            .repeats()));
        c.add(s(
            Command::choice(cmd::SEL_ENDS, "Oldest line, cursor", "").keys("g G")
        ));
        c.add(s(Command::action(cmd::MARK, "Start here")
            .keys("Space")
            .hint("start")));
        c.add(s(Command::action(cmd::LINES, "Whole lines").keys("l")));
        c.add(s(Command::action(cmd::YANK, "Copy")
            .keys("Enter")
            .hint("copy")));
        // Esc backs out everywhere: its hint would push copy off the bar.
        c.add(s(Command::action(cmd::CANCEL, "Cancel").keys("Esc")));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let before = self.reserve();
        self.run(id, step, cx);
        self.after_mode(before, cx);
        self.thaw(cx);
    }

    fn raw(&mut self, k: &RawKey, cx: &mut Cx) {
        let before = self.reserve();
        self.raw_key(k, cx);
        self.after_mode(before, cx);
        self.thaw(cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
        if let Some((_, w)) = self.panel.take() {
            cx.unwatch(w);
        }
        // Closing each master hangs up its session.
        for t in &mut self.tabs {
            if let Some(w) = t.watch.take() {
                cx.unwatch(w);
            }
            t.pty = None;
        }
    }

    /// Focus reports (mode 1004) for the program in the tab in front.
    fn front(&mut self, front: bool, cx: &mut Cx) {
        let Some(tab) = self.tab() else {
            return;
        };
        if let (true, Some(w)) = (tab.vt.t.modes.focus, tab.watch) {
            cx.write(w, if front { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    fn frame_ms(&self) -> u64 {
        FRAME_MS
    }

    fn draw(&self, ui: &mut Ui) {
        let th: Theme = *ui.theme();
        let area = ui.area();
        let Some(tab) = self.tab() else {
            return;
        };
        let look = self.look();
        let pal = Colors::of(&th);
        let t = &tab.vt.t;
        let (vis_cols, vis_rows) = self.vis();
        let x0 = area.x + (area.w - vis_cols as i32 * look.cw).max(0) / 2;
        let (cx, cy) = t.cursor();
        let view = &tab.view;
        let typing = self.mode == Mode::Typing;
        let sel = match self.mode {
            Mode::Select {
                anchor,
                head,
                lines,
            } => Some((anchor, head, lines)),
            _ => None,
        };
        // Selecting, the head is the one cursor.
        let show_cursor = t.modes.show_cursor && !self.waiting && view.live() && sel.is_none();
        let head = sel.map(|(_, h, _)| h);
        let head_at = |pos: Pos| {
            head.filter(|h| h.pos == pos).map(|h| Cursor {
                col: h.col,
                shape: CursorShape::Block,
            })
        };
        let rows = view.rows(t, vis_rows);
        let mut scratch = self.scratch.borrow_mut();
        scratch.1.clear();
        let c = ui.canvas();
        for (v, &src) in rows.iter().enumerate() {
            let span = Span {
                x: x0,
                y: area.y + v as i32 * look.ch,
                first: view.pan_x,
                n: vis_cols,
            };
            let pos = match src {
                Src::Grid(y) => Pos::Grid(y),
                Src::Hist(i, sub) => Pos::Hist(t.history.id(i), sub),
            };
            let marked = match (sel, self.hit) {
                (Some((anchor, head, lines)), _) => sel_span(anchor, head, lines, pos, t.cols),
                (None, Some(h)) if h.pos == pos => Some(h.cols),
                _ => None,
            };
            match src {
                Src::Grid(y) => {
                    let cursor = (show_cursor && y == cy)
                        .then_some(Cursor {
                            col: cx,
                            shape: t.shape,
                        })
                        .or_else(|| head_at(pos));
                    let Some(r) = t.grid().rows.get(y) else {
                        continue;
                    };
                    let cells = &r.cells;
                    render::row(
                        c,
                        area,
                        &look,
                        &pal,
                        &span,
                        cells,
                        &t.clusters,
                        cursor,
                        marked,
                    );
                }
                Src::Hist(i, sub) => {
                    let (cells, clusters) = &mut *scratch;
                    t.history.line(i).cells(t.cols, sub, cells, clusters);
                    let cursor = head_at(pos);
                    render::row(c, area, &look, &pal, &span, cells, clusters, cursor, marked);
                }
            }
        }
        // Images, over the grid rows on screen.
        let first_grid = rows.iter().enumerate().find_map(|(v, s)| match s {
            Src::Grid(y) => Some((v, *y)),
            Src::Hist(..) => None,
        });
        if let Some((v, y)) = first_grid.filter(|_| !t.grid().images.is_empty()) {
            let band = render::Band {
                x: x0,
                first: view.pan_x,
                cols: vis_cols,
                top: y,
                bottom: (y + rows.len() - v).min(t.rows),
                y: area.y + v as i32 * look.ch,
            };
            render::images(c, area, &look, pal.bg, &band, &t.grid().images);
        }
        // How far back: a mark on the right edge.
        if let Some(d) = view.depth(t) {
            let h = (area.h / 8).max(4);
            let y = area.y + ((area.h - h) as f32 * (1.0 - d)) as i32;
            let w = theme::TERM_MARK_W;
            c.fill(Rect::new(area.right() - w, y, w, h), pal.accent);
        }
        // Window mode: a line on each edge with cells beyond it.
        if self.layout != Layout::Fit {
            let (cols, grows) = self.grid_size();
            let top_row = match view.top {
                Pos::Grid(y) => y,
                Pos::Hist(..) => 0,
            };
            let bottom = area.y + vis_rows as i32 * look.ch;
            if view.pan_x > 0 {
                c.fill(Rect::new(area.x, area.y, 1, bottom - area.y), pal.accent);
            }
            if view.pan_x + vis_cols < cols {
                c.fill(
                    Rect::new(area.right() - 1, area.y, 1, bottom - area.y),
                    pal.accent,
                );
            }
            if view.live() && top_row + vis_rows < grows {
                c.fill(Rect::new(area.x, bottom - 1, area.w, 1), pal.accent);
            }
        }
        if !typing {
            let mut cmds = Commands::new();
            self.commands(&mut cmds);
            let hints = cmds.hints();
            let hints: Vec<Hint> = hints
                .iter()
                .map(|(key, label)| Hint { key, label })
                .collect();
            widgets::hint_bar(c, &th, &hints, None);
            if let Mode::Find(q) = &self.mode {
                let r = Rect::new(0, th.m.height - 2 * th.m.hint_h, th.m.width, th.m.hint_h);
                c.fill(r, th.c.surface);
                let text = format!("Find: {q}_");
                let line = Rect::new(r.x + th.m.pad, r.y, r.w - th.m.pad, r.h);
                widgets::text_in(c, th.f.small, line, &text, th.c.fg);
            }
        }
    }
}

/// Lowercase with one char out for each char in, so a match's char index in
/// the folded text is its index in the row.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}
