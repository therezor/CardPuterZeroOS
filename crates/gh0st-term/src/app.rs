//! The terminal page: tabs of PTYs, keys in, cells out, and scroll mode for
//! everything that isn't typing: scroll, pan, zoom, search, copy, tabs.
//! Every key outside typing is a command, so the app menu, the Keys screen
//! and the hint bar come from one list.

use crate::keys;
use crate::prefs::{self, Prefs, key};
use crate::pty::{self, Pty};
use crate::render::{self, Colors, Cursor, Look, Span};
use crate::view::{Pos, Src, View};
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
/// this often.
const FRAME_MS: u64 = 20;
/// How long output may keep coming before it is drawn. htop and others
/// repaint in several writes; drawing between them flashes.
const SETTLE_MS: u64 = 5;
/// Notifications from programs, at most one this often.
const NOTICE_GAP_MS: u128 = 3000;
/// Synchronized output may hold the screen this long at most.
const SYNC_MS: u64 = 150;

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
}

/// Names of the modes on the Keys screen.
const TYPING: &str = "Typing";
const SCROLLING: &str = "Scroll mode";
const FINDING: &str = "Find";
const COPYING: &str = "Copy lines";

struct Tab {
    vt: Box<Vt>,
    pty: Option<Pty>,
    watch: Option<Watch>,
    pid: i32,
    view: View,
    /// Synchronized output began: the hold ends at this timer.
    sync: Option<Timer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Typing,
    /// Scroll mode: keys scroll, pan, zoom and manage tabs.
    View,
    /// Typing a search.
    Find(String),
    /// Lines from the anchor to the head are selected.
    Select {
        anchor: Pos,
        head: Pos,
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
    clipboard: String,
    /// The command of `-e` ended: the next key quits.
    waiting: bool,
    /// When the last notification went out.
    noticed: Option<std::time::Instant>,
    /// Output came in: draw when this fires, so a program's repaint lands
    /// whole rather than half done.
    settle: Option<Timer>,
    /// The hint bar's height, from the theme.
    hint_h: i32,
    /// Scroll mode has said once that its letters are shortcuts.
    told_keys: bool,
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
            clipboard: String::new(),
            waiting: false,
            noticed: None,
            settle: None,
            hint_h: 18,
            told_keys: false,
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
        self.retitle();
        cx.redraw();
    }

    fn switch(&mut self, i: usize, cx: &mut Cx) {
        if i < self.tabs.len() && i != self.cur {
            self.front(false, cx);
            self.cur = i;
            self.front(true, cx);
            self.hit = None;
            self.retitle();
            self.track();
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
        if let Some(text) = t.clipboard.take() {
            self.clipboard = text;
            cx.toast("Copied");
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
        let held = tab.sync.is_some();
        if events & event::TITLE != 0 && i == self.cur {
            self.retitle();
        }
        if i == self.cur && (total > 0 || hup) && !held && self.settle.is_none() {
            self.settle = Some(cx.after(SETTLE_MS));
        }
        hup
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
            cmd::FIND_DONE | cmd::CANCEL => self.mode = Mode::View,
            cmd::EXTEND => self.extend(back, cx),
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

    fn start_select(&mut self, cx: &mut Cx) {
        let Some(tab) = self.tab() else {
            return;
        };
        let t = &tab.vt.t;
        let at = match self.hit {
            Some(h) => h.pos,
            None if tab.view.live() => Pos::Grid(t.cursor().1),
            None => tab.view.top,
        };
        self.mode = Mode::Select {
            anchor: at,
            head: at,
        };
        cx.redraw();
    }

    /// Moves the selection's head a row up or down.
    fn extend(&mut self, up: bool, cx: &mut Cx) {
        let Mode::Select { anchor, head } = self.mode.clone() else {
            return;
        };
        let vis = self.vis();
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        let t = &tab.vt.t;
        let head = if up { head.up(t) } else { head.down(t) };
        tab.view.reveal(head, t, vis.1);
        self.mode = Mode::Select { anchor, head };
        cx.redraw();
    }

    /// Copies the selected lines and goes back to scroll mode.
    fn yank(&mut self, cx: &mut Cx) {
        let Mode::Select { anchor, head } = self.mode.clone() else {
            return;
        };
        let Some(tab) = self.tab() else {
            return;
        };
        let text = selected_text(&tab.vt.t, anchor, head, &mut self.scratch.borrow_mut());
        let lines = text.lines().count().max(1);
        self.clipboard = text;
        self.mode = Mode::View;
        cx.toast(&format!("Copied {lines} lines"));
        cx.redraw();
    }

    fn paste(&mut self, cx: &mut Cx) {
        if self.clipboard.is_empty() {
            cx.toast("Nothing copied");
            return;
        }
        let Some(tab) = self.tab() else {
            return;
        };
        let mut out = Vec::new();
        keys::paste(&self.clipboard, &tab.vt.t.modes, &mut out);
        self.send(&out, cx);
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

/// Rows from `a` to `b`, either way round, as text. Rows that autowrap
/// joined stay one line.
fn selected_text(t: &Terminal, a: Pos, b: Pos, scratch: &mut (Vec<Cell>, Clusters)) -> String {
    let (mut p, end) = if a.key() <= b.key() { (a, b) } else { (b, a) };
    let mut out = String::new();
    loop {
        let src = p.src(t);
        let (text, _) = row_text(t, src, scratch);
        out.push_str(&text);
        let joined = match src {
            Src::Grid(y) => t.grid().rows.get(y).is_some_and(|r| r.wrapped),
            Src::Hist(i, sub) => sub + 1 < t.history.line(i).rows(t.cols),
        };
        if p == end {
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

impl Page for Term {
    fn title(&self) -> &str {
        &self.title
    }

    fn bars(&self) -> Bars {
        if self.bar { Bars::top() } else { Bars::none() }
    }

    fn start(&mut self, cx: &mut Cx) {
        self.load(cx);
        let t = *cx.theme();
        self.area = self.bars().content(&t);
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

    fn ready(&mut self, w: Watch, cx: &mut Cx) {
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
        if self.settle == Some(timer) {
            self.settle = None;
            self.track();
            cx.redraw();
            return;
        }
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if tab.sync == Some(timer) {
                tab.sync = None;
                if i == self.cur {
                    self.track();
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
            Command::toggle(cmd::SCROLL_MODE, "Scroll mode", !typing)
                .keys("Fn+Space")
                .closes()
                .mode(TYPING, (typing || scroll) && idle),
        );
        let size = format!("{}x{}", look.cw, look.ch);
        c.add(on(Command::choice(cmd::SIZE, "Text size", size).keys("r t").repeats()));
        let grid = format!("{} {gc}x{gr}", self.layout.name());
        c.add(on(Command::choice(cmd::LAYOUT, "Grid", grid).keys("m")));
        c.add(on(Command::action(cmd::FIND, "Find").keys("n")));
        c.add(on(Command::action(cmd::COPY, "Copy lines").keys("v")));
        let pasteable = !self.clipboard.is_empty();
        c.add(on(Command::action(cmd::PASTE, "Paste").keys("p").enabled(pasteable)));
        c.group("Tabs");
        c.add(on(Command::action(cmd::NEW_TAB, "New tab").keys("o")));
        if self.tabs.len() > 1 {
            let at = format!("{} of {}", self.cur + 1, self.tabs.len());
            c.add(on(Command::choice(cmd::TAB, "Tab", at).keys("Tab 1-9")));
        }
        c.add(on(Command::action(cmd::CLOSE_TAB, "Close tab").keys("w").confirm()));
        c.group("Settings");
        c.add(on(Command::toggle(cmd::BAR, "Top bar", self.bar).keys("b")));
        c.add(on(Command::action(cmd::SETTINGS, "More settings").keys("s").opens()));

        // Scroll mode's moves have keys but no menu rows.
        let up_down = |id, label| on(moves(Command::choice(id, label, "")));
        c.add(up_down(cmd::SCROLL, "Scroll up, down").keys("f x").hint("scroll"));
        c.add(up_down(cmd::PAN, "Pan left, right").keys("z c").hint("pan"));
        c.add(up_down(cmd::PAGE, "Page up, down").keys("PgUp PgDn"));
        c.add(up_down(cmd::PROMPT, "Previous, next prompt").keys("u i"));
        c.add(on(Command::choice(cmd::ENDS, "Oldest line, live", "").keys("g G").keys_only()));
        c.add(on(Command::action(cmd::HOME, "Send Home").keys("k").keys_only()));
        c.add(on(Command::action(cmd::KEYS, "Keys").keys("h").keys_only().hint("keys")));
        c.add(on(Command::action(cmd::DONE, "Back to typing").keys("Esc q").keys_only().hint("done")));

        let f = |c: Command| c.keys_only().mode(FINDING, find);
        c.add(f(Command::action(cmd::OLDER, "Older match").keys("Enter Up").repeats().hint("older")));
        c.add(f(Command::action(cmd::NEWER, "Newer match").keys("Down").repeats().hint("newer")));
        c.add(f(Command::action(cmd::FIND_DONE, "Done").keys("Esc").hint("done")));

        let s = |c: Command| c.keys_only().mode(COPYING, select);
        c.add(s(Command::choice(cmd::EXTEND, "Extend up, down", "").keys("f x").repeats().hint("extend")));
        c.add(s(Command::action(cmd::YANK, "Copy").keys("y Enter").hint("copy")));
        c.add(s(Command::action(cmd::CANCEL, "Cancel").keys("Esc").hint("cancel")));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let before = self.reserve();
        self.run(id, step, cx);
        self.after_mode(before, cx);
    }

    fn raw(&mut self, k: &RawKey, cx: &mut Cx) {
        let before = self.reserve();
        self.raw_key(k, cx);
        self.after_mode(before, cx);
    }
    fn stop(&mut self, cx: &mut Cx) {
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
        let show_cursor = t.modes.show_cursor && !self.waiting && view.live();
        let sel = match self.mode {
            Mode::Select { anchor, head } => {
                let (a, b) = (anchor.key().min(head.key()), anchor.key().max(head.key()));
                Some((a, b))
            }
            _ => None,
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
                (Some((a, b)), _) if pos.key() >= a && pos.key() <= b => Some((0, t.cols)),
                (None, Some(h)) if h.pos == pos => Some(h.cols),
                _ => None,
            };
            match src {
                Src::Grid(y) => {
                    let cursor = (show_cursor && y == cy).then_some(Cursor {
                        col: cx,
                        shape: t.shape,
                    });
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
                    render::row(c, area, &look, &pal, &span, cells, clusters, None, marked);
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
            let pages = (self.tabs.len() > 1).then_some((self.cur, self.tabs.len()));
            widgets::hint_bar(c, &th, &hints, pages);
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
