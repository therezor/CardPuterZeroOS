//! The terminal page: tabs of PTYs, keys in, cells out, and view mode for
//! everything that isn't typing: scroll, pan, zoom, search, copy, tabs.

use crate::keys;
use crate::prefs::{self, Prefs, key};
use crate::pty::{self, Pty};
use crate::render::{self, Colors, Cursor, Look, Span};
use crate::view::{Family, Pos, Src, View};
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
/// Notifications from programs, at most one this often.
const NOTICE_GAP_MS: u128 = 3000;
/// Synchronized output may hold the screen this long at most.
const SYNC_MS: u64 = 150;
const PAGEUP: u16 = 104;
const PAGEDOWN: u16 = 109;

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

/// What the app menu and view mode's letters do. The menu id is the
/// variant's number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    View,
    ZoomIn,
    ZoomOut,
    Layout,
    Find,
    Copy,
    Paste,
    NewTab,
    CloseTab,
    NextTab,
    Bar,
    Settings,
    Keys,
}

const ACTS: &[Act] = &[
    Act::View,
    Act::ZoomIn,
    Act::ZoomOut,
    Act::Layout,
    Act::Find,
    Act::Copy,
    Act::Paste,
    Act::NewTab,
    Act::CloseTab,
    Act::NextTab,
    Act::Bar,
    Act::Settings,
    Act::Keys,
];

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
    /// Keys scroll, pan, zoom and manage tabs.
    View,
    /// Typing a search.
    Find(String),
    /// Lines from the anchor to the head are selected.
    Select {
        anchor: Pos,
        head: Pos,
    },
    /// Every view-mode key, on one screen.
    Help,
}

/// View mode's keys, for the help screen.
const HELP_KEYS: &[(&str, &str)] = &[
    ("↑↓ F X", "scroll"),
    ("←→ Z C", "pan"),
    ("PgUp/Dn", "page"),
    ("g G", "top, end"),
    ("r t", "zoom out, in"),
    ("m", "magnify, fit"),
    ("n", "find"),
    ("v", "copy lines"),
    ("u i", "prev, next prompt"),
    ("p", "paste"),
    ("o w", "new, close tab"),
    ("1-9 Tab", "go to tab"),
    ("k", "Home key"),
    ("Fn+Esc", "menu"),
    ("s", "settings"),
    ("q Esc", "typing"),
];

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
    family: Family,
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
            family: Family::Terminus,
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
            scratch: RefCell::new((Vec::new(), Clusters::default())),
        }
    }

    fn load(&mut self, cx: &mut Cx) {
        let s = prefs::Settings::from(cx.settings());
        self.family = Family::from_key(&s.font);
        self.zoom = s.zoom.min(self.family.faces().len() - 1);
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
        Look::of(&self.family.faces()[self.zoom])
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
            ((self.area.h / look.ch).max(1) as usize).min(rows),
        )
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
            view: View::new(self.family, self.zoom, self.window()),
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
        if i == self.cur && (total > 0 || hup) && !held {
            self.track();
            cx.redraw();
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
        let zoom = zoom.min(self.family.faces().len() - 1);
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        self.relayout(cx);
        cx.settings().set_int(key::ZOOM, zoom as i64);
        let look = self.look();
        let (c, r) = self.grid_size();
        cx.toast(&format!("{}x{} cells, {c}x{r}", look.cw, look.ch));
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
        cx.toast(&format!("{}: {c}x{r}", self.layout.name()));
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

    fn view_key(&mut self, k: &RawKey, cx: &mut Cx) {
        let vis = self.vis();
        let page = vis.1.saturating_sub(1).max(1);
        let ch = k.text.map(|c| c.to_ascii_lowercase());
        // A held key repeats scrolling, panning and zoom, never an action
        // such as closing a tab.
        let moves = matches!(
            k.code,
            code::UP
                | code::DOWN
                | code::LEFT
                | code::RIGHT
                | code::F
                | code::X
                | code::Z
                | code::C
                | PAGEUP
                | PAGEDOWN
        ) || matches!(ch, Some('r' | 't' | '-' | '=' | '+' | 'u' | 'i'));
        if k.state == KeyState::Repeat && !moves {
            return;
        }
        match (k.code, ch) {
            (code::UP, _) | (code::F, _) => self.scroll(true, 1, cx),
            (code::DOWN, _) | (code::X, _) => self.scroll(false, 1, cx),
            (PAGEUP, _) => self.scroll(true, page, cx),
            (PAGEDOWN, _) => self.scroll(false, page, cx),
            (code::LEFT, _) | (code::Z, _) | (code::RIGHT, _) | (code::C, _) => {
                let left = matches!(k.code, code::LEFT | code::Z);
                let step = (vis.0 / 2).max(1) as isize;
                let cols = self.grid_size().0;
                if let Some(tab) = self.tabs.get_mut(self.cur) {
                    tab.view.pan(if left { -step } else { step }, cols, vis.0);
                    cx.redraw();
                }
            }
            // Letters, so nothing here needs the Sym key; - = / [ ] too.
            (_, Some('r' | '-')) => self.act(Act::ZoomOut, cx),
            (_, Some('t' | '=' | '+')) => self.act(Act::ZoomIn, cx),
            (_, Some('m')) => self.act(Act::Layout, cx),
            (_, Some('b')) => self.act(Act::Bar, cx),
            (_, Some('o')) => self.act(Act::NewTab, cx),
            (_, Some('w')) => self.act(Act::CloseTab, cx),
            (code::TAB, _) => self.act(Act::NextTab, cx),
            (_, Some(d @ '1'..='9')) => self.switch(d as usize - '1' as usize, cx),
            (_, Some('g')) if k.shift => self.scroll(false, usize::MAX / 2, cx),
            (_, Some('g')) => self.scroll(true, usize::MAX / 2, cx),
            (_, Some('n' | '/')) => self.act(Act::Find, cx),
            (_, Some('v')) => self.act(Act::Copy, cx),
            (_, Some('u' | '[')) => self.jump_prompt(true, cx),
            (_, Some('i' | ']')) => self.jump_prompt(false, cx),
            (_, Some('s')) => self.act(Act::Settings, cx),
            (_, Some('p')) => self.act(Act::Paste, cx),
            (_, Some('h')) => self.act(Act::Keys, cx),
            (_, Some('k')) => {
                let app = self.tab().is_some_and(|t| t.vt.t.modes.app_cursor);
                self.mode = Mode::Typing;
                self.send(if app { b"\x1bOH" } else { b"\x1b[H" }, cx);
            }
            (code::ESC | code::ENTER, _) | (_, Some('q')) => self.leave_view(cx),
            _ => {}
        }
    }

    /// One action, from the menu or a view-mode letter.
    fn act(&mut self, a: Act, cx: &mut Cx) {
        match a {
            Act::View => self.mode = Mode::View,
            Act::ZoomIn => self.zoom_to(self.zoom + 1, cx),
            Act::ZoomOut => self.zoom_to(self.zoom.saturating_sub(1), cx),
            Act::Layout => self.next_layout(cx),
            Act::Find => self.mode = Mode::Find(self.query.clone()),
            Act::Copy => self.start_select(cx),
            Act::Paste => {
                self.mode = Mode::Typing;
                self.paste(cx);
            }
            Act::NewTab => match self.open_tab(cx) {
                Ok(()) => self.mode = Mode::Typing,
                Err(e) => cx.toast(&format!("No new tab: {e}")),
            },
            Act::CloseTab => {
                let i = self.cur;
                self.close_tab(i, cx);
            }
            Act::NextTab => {
                let next = (self.cur + 1) % self.tabs.len().max(1);
                self.switch(next, cx);
            }
            Act::Bar => {
                self.bar = !self.bar;
                self.relayout(cx);
                cx.settings().set_bool(key::BAR, self.bar);
            }
            Act::Settings => cx.push(Prefs::new()),
            Act::Keys => self.mode = Mode::Help,
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

    fn find_key(&mut self, k: &RawKey, cx: &mut Cx) {
        let Mode::Find(q) = &mut self.mode else {
            return;
        };
        // The query stays up: Enter or ↑ finds the next older match, ↓ the
        // next newer one, until Esc.
        match k.code {
            code::ESC => self.mode = Mode::View,
            code::ENTER | code::UP | code::DOWN => {
                if *q != self.query {
                    self.query = q.clone();
                    self.hit = None;
                }
                self.find_next(k.code != code::DOWN, cx);
            }
            code::BACKSPACE => {
                q.pop();
            }
            _ => {
                if let Some(c) = k.text.filter(|c| !c.is_control()) {
                    q.push(c);
                }
            }
        }
        cx.redraw();
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
                let tab = &mut self.tabs[self.cur];
                tab.view.reveal(h.pos, &tab.vt.t, vis.1);
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

    fn select_key(&mut self, k: &RawKey, cx: &mut Cx) {
        let Mode::Select { anchor, head } = self.mode.clone() else {
            return;
        };
        let vis = self.vis();
        let Some(tab) = self.tabs.get_mut(self.cur) else {
            return;
        };
        let t = &tab.vt.t;
        let ch = k.text.map(|c| c.to_ascii_lowercase());
        let head = match (k.code, ch) {
            (code::UP, _) | (code::F, _) => head.up(t),
            (code::DOWN, _) | (code::X, _) => head.down(t),
            (code::ESC, _) => {
                self.mode = Mode::View;
                cx.redraw();
                return;
            }
            (code::ENTER, _) | (_, Some('y')) => {
                let text = selected_text(t, anchor, head, &mut self.scratch.borrow_mut());
                let lines = text.lines().count().max(1);
                self.clipboard = text;
                self.mode = Mode::View;
                cx.toast(&format!("Copied {lines} lines"));
                cx.redraw();
                return;
            }
            _ => return,
        };
        tab.view.reveal(head, t, vis.1);
        self.mode = Mode::Select { anchor, head };
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

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        match &self.mode {
            Mode::Typing => Vec::new(),
            Mode::View => vec![
                ("↑↓", "scroll"),
                ("r t", "zoom"),
                ("n", "find"),
                ("h", "keys"),
            ],
            Mode::Help => vec![("any key", "back")],
            Mode::Find(_) => vec![("Enter ↑", "older"), ("↓", "newer"), ("Esc", "done")],
            Mode::Select { .. } => vec![("↑↓", "extend"), ("y", "copy"), ("Esc", "cancel")],
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

    fn menu(&self) -> Vec<MenuItem> {
        let look = self.look();
        ACTS.iter()
            .enumerate()
            .filter(|&(_, &a)| a != Act::NextTab || self.tabs.len() > 1)
            .map(|(i, &a)| {
                let item = |label: &str, key: char| MenuItem::new(i as u32, label).key(key);
                match a {
                    Act::View => MenuItem::new(i as u32, "Scroll and pan"),
                    Act::ZoomIn => item("Zoom in", 't').value(format!("{}x{}", look.cw, look.ch)),
                    Act::ZoomOut => item("Zoom out", 'r'),
                    Act::Layout => item("Grid", 'm').value(self.layout.name()),
                    Act::Find => item("Find", 'n'),
                    Act::Copy => item("Copy lines", 'v'),
                    Act::Paste => item("Paste", 'p'),
                    Act::NewTab => item("New tab", 'o'),
                    Act::CloseTab => item("Close tab", 'w'),
                    Act::NextTab => MenuItem::new(i as u32, "Next tab").value(format!(
                        "{}/{}",
                        self.cur + 1,
                        self.tabs.len()
                    )),
                    Act::Bar => item("Top bar", 'b').value(if self.bar { "On" } else { "Off" }),
                    Act::Settings => item("Settings", 's'),
                    Act::Keys => item("View mode keys", 'h'),
                }
            })
            .collect()
    }

    fn menu_pick(&mut self, id: u32, cx: &mut Cx) {
        if let Some(&a) = ACTS.get(id as usize) {
            self.act(a, cx);
        }
    }

    fn raw(&mut self, k: &RawKey, cx: &mut Cx) {
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
            Mode::Help => {
                if k.state == KeyState::Press {
                    self.mode = Mode::View;
                    cx.redraw();
                }
            }
            Mode::View => self.view_key(k, cx),
            Mode::Find(_) => self.find_key(k, cx),
            Mode::Select { .. } => self.select_key(k, cx),
            Mode::Typing => {
                if k.shift && matches!(k.code, PAGEUP | PAGEDOWN) {
                    let page = self.vis().1.saturating_sub(1).max(1);
                    self.scroll(k.code == PAGEUP, page, cx);
                    return;
                }
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
            let hints = self.hints();
            let hints: Vec<Hint> = hints
                .iter()
                .map(|&(key, label)| Hint { key, label })
                .collect();
            let pages = (self.tabs.len() > 1).then_some((self.cur, self.tabs.len()));
            widgets::hint_bar(c, &th, &hints, pages);
            if self.mode == Mode::Help {
                help(c, &th, area);
            }
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

/// The help screen: view mode's keys in two columns over the content.
fn help(c: &mut gh0st_app::gfx::Canvas, th: &Theme, area: Rect) {
    let r = Rect::new(area.x, area.y, area.w, area.h - th.m.hint_h);
    c.fill(r, th.c.surface);
    let per = HELP_KEYS.len().div_ceil(2);
    let line = th.m.line_h;
    let col_w = r.w / 2;
    for (i, (keys, what)) in HELP_KEYS.iter().enumerate() {
        let x = r.x + th.m.pad + (i / per) as i32 * col_w;
        let y = r.y + theme::TERM_HELP_TOP + (i % per) as i32 * line;
        let kw = theme::TERM_HELP_KEY_W;
        let k = Rect::new(x, y, kw, line);
        widgets::text_in(c, th.f.small_bold, k, keys, th.c.accent);
        let w = Rect::new(x + kw + 2, y, col_w - th.m.pad - kw - 2, line);
        widgets::text_in(c, th.f.small, w, what, th.c.fg);
    }
}

/// Lowercase with one char out for each char in, so a match's char index in
/// the folded text is its index in the row.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}
