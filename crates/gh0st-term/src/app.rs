//! The terminal page: tabs of PTYs, keys in, cells out.

use crate::keys;
use crate::pty::{self, Pty};
use crate::render::{self, Colors, Cursor, Look};
use crate::vt::{Vt, event};
use gh0st_app::prelude::*;
use gh0st_app::{KeyState, RawKey};
use std::io;
use std::path::PathBuf;

/// History budget for all tabs together, in bytes.
const HISTORY: usize = 1 << 20;
/// Most bytes read from one PTY per loop turn, so keys stay answered under
/// a flood.
const READ_MAX: usize = 64 * 1024;
/// About one panel refresh: under a flood the latest state is drawn at most
/// this often.
const FRAME_MS: u64 = 20;

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

struct Tab {
    vt: Box<Vt>,
    pty: Option<Pty>,
    watch: Option<Watch>,
    pid: i32,
}

pub struct Term {
    opts: Options,
    tabs: Vec<Tab>,
    cur: usize,
    buf: Vec<u8>,
    out: Vec<u8>,
    title: String,
    look: Option<Look>,
    size: (usize, usize),
    /// The command of `-e` ended: the next key quits.
    waiting: bool,
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
            look: None,
            size: (80, 24),
            waiting: false,
        }
    }

    /// The grid that fits the content area at the current face.
    fn fit(&self, t: &gh0st_app::gfx::Theme) -> (usize, usize) {
        let look = self.look.unwrap_or_else(|| Look::mono(t));
        let area = self.bars().content(t);
        (
            (area.w / look.cw).max(2) as usize,
            (area.h / look.ch).max(1) as usize,
        )
    }

    fn open_tab(&mut self, cx: &mut Cx) -> io::Result<()> {
        let (cols, rows) = self.size;
        let (argv, arg0, cwd) = match &self.opts.command {
            Some(c) => (c.clone(), None, None),
            None => {
                let sh = pty::shell();
                let name = std::path::Path::new(&sh)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sh".into());
                let cwd = self
                    .tabs
                    .get(self.cur)
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
        let budget = HISTORY / (self.tabs.len() + 1);
        for t in &mut self.tabs {
            t.vt.t.history.set_max(budget);
        }
        self.tabs.push(Tab {
            vt: Box::new(Vt::new(cols, rows, budget)),
            pid: pty.pid,
            pty: Some(pty),
            watch: Some(watch),
        });
        self.cur = self.tabs.len() - 1;
        self.retitle();
        Ok(())
    }

    /// The child hung up or ended. A `-e` command with sysplause stays on
    /// screen until a key; otherwise the tab goes.
    fn close_tab(&mut self, i: usize, cx: &mut Cx) {
        let Some(tab) = self.tabs.get_mut(i) else {
            return;
        };
        if let Some(w) = tab.watch.take() {
            cx.unwatch(w);
        }
        tab.pty = None;
        if self.opts.command.is_some() && self.opts.sysplause {
            tab.vt.feed(b"\r\n\x1b[0m-- press any key --");
            self.waiting = true;
            cx.redraw();
            return;
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            cx.quit();
            return;
        }
        let budget = HISTORY / self.tabs.len();
        for t in &mut self.tabs {
            t.vt.t.history.set_max(budget);
        }
        self.cur = self.cur.min(self.tabs.len() - 1);
        self.retitle();
        cx.redraw();
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
        if t.events & event::TITLE != 0 && i == self.cur {
            self.retitle();
        }
        let t = &mut self.tabs[i].vt.t;
        t.events = 0;
        if total > 0 && i == self.cur && !t.modes.sync {
            cx.redraw();
        }
        hup
    }

    fn retitle(&mut self) {
        let name = self
            .tabs
            .get(self.cur)
            .map(|t| t.vt.t.title.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Terminal");
        self.title = if self.tabs.len() > 1 {
            format!("[{}/{}] {name}", self.cur + 1, self.tabs.len())
        } else {
            name.to_string()
        };
    }
}

impl Page for Term {
    fn title(&self) -> &str {
        &self.title
    }

    fn start(&mut self, cx: &mut Cx) {
        let t = *cx.theme();
        self.look = Some(Look::mono(&t));
        self.size = self.fit(&t);
        if let Err(e) = self.open_tab(cx) {
            eprintln!("gh0st-term: {e}");
            cx.quit();
        }
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

    fn raw_keys(&self) -> bool {
        true
    }

    fn raw(&mut self, k: &RawKey, cx: &mut Cx) {
        if self.waiting {
            if k.state == KeyState::Press {
                cx.quit();
            }
            return;
        }
        let Some(tab) = self.tabs.get(self.cur) else {
            return;
        };
        self.out.clear();
        keys::encode(k, &tab.vt.t.modes, true, &mut self.out);
        if let (false, Some(w)) = (self.out.is_empty(), tab.watch) {
            cx.write(w, &self.out);
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

    fn frame_ms(&self) -> u64 {
        FRAME_MS
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        let Some(tab) = self.tabs.get(self.cur) else {
            return;
        };
        let look = self.look.unwrap_or_else(|| Look::mono(&t));
        let pal = Colors::of(&t);
        let term = &tab.vt.t;
        let g = term.grid();
        let (cx, cy) = term.cursor();
        let x0 = area.x + (area.w - g.cols as i32 * look.cw).max(0) / 2;
        let c = ui.canvas();
        for (y, r) in g.rows.iter().enumerate() {
            let cursor = (y == cy && term.modes.show_cursor && !self.waiting).then_some(Cursor {
                col: cx,
                shape: term.shape,
            });
            let top = area.y + y as i32 * look.ch;
            render::row(
                c,
                area,
                &look,
                &pal,
                x0,
                top,
                &r.cells,
                &term.clusters,
                0,
                g.cols,
                cursor,
            );
        }
    }
}
