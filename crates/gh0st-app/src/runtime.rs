//! The page stack and everything that drives it: keys, timers, jobs, the
//! frame. The device loop, the sim and the test driver all feed this.

use crate::bars::{Bars, Bottom};
use crate::key::{self, Key};
use crate::page::{Cx, Job, JobEvent, Page, Request, Shared, Timer};
use crate::ui::Ui;
use gh0st_core::keys::Keyboard;
use gh0st_core::{battery, rt, status};
use gh0st_ui::widgets::{self, Battery, Hint, Net, SoftKey as Slot, Status};
use gh0st_ui::{Canvas, Rect, Theme, theme};
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const TOAST_MS: u64 = 2000;

struct TimerEntry {
    id: u64,
    owner: usize,
    at: u64,
    every: Option<u64>,
}

struct JobProc {
    id: u64,
    owner: usize,
    child: Child,
    buf: Vec<u8>,
    eof: bool,
}

/// What the self-drawn top bar shows. Read at start and on the minute.
struct Bar {
    clock: [u8; 5],
    clock_at: u64,
    battery_dir: Option<PathBuf>,
    battery: Option<Battery>,
    net: Net,
}

impl Bar {
    fn new() -> Bar {
        Bar {
            clock: *b"--:--",
            clock_at: 0,
            battery_dir: battery::find(std::path::Path::new("/sys/class/power_supply")),
            battery: None,
            net: Net::None,
        }
    }

    fn refresh(&mut self) {
        let (clock, at) = rt::clock_text();
        self.clock = clock;
        self.clock_at = at;
        self.battery = self
            .battery_dir
            .as_deref()
            .and_then(battery::sample)
            .map(|r| Battery {
                percent: r.percent,
                charging: r.charging,
            });
        self.net = match status::network(
            std::path::Path::new("/sys/class/net"),
            std::path::Path::new("/proc/net/wireless"),
        ) {
            status::Net::None => Net::None,
            status::Net::Wifi(b) => Net::Wifi(b),
            status::Net::Ethernet => Net::Ethernet,
        };
    }
}

pub(crate) struct Runtime {
    stack: Vec<Box<dyn Page>>,
    pub s: Shared,
    kb: Keyboard,
    pub canvas: Canvas,
    timers: Vec<TimerEntry>,
    jobs: Vec<JobProc>,
    /// Jobs that couldn't start: id and owner, told on the next tick.
    pending: Vec<(u64, usize)>,
    toast: Option<(String, u64)>,
    bar: Bar,
    palette: String,
    pub dirty: bool,
    pub quit: bool,
}

impl Runtime {
    pub fn new(page: Box<dyn Page>, config: Option<PathBuf>) -> Runtime {
        let mut s = Shared::new(Theme::default(), config);
        let palette = s
            .settings
            .get_str("gh0st.palette")
            .unwrap_or("")
            .to_string();
        s.theme = Theme::new(theme::palette(&palette).unwrap_or(theme::MINT));
        let t = s.theme;
        Runtime {
            stack: vec![page],
            s,
            kb: Keyboard::load(),
            canvas: Canvas::new(t.m.width, t.m.height),
            timers: Vec::new(),
            jobs: Vec::new(),
            pending: Vec::new(),
            toast: None,
            bar: Bar::new(),
            palette,
            dirty: true,
            quit: false,
        }
    }

    pub fn start(&mut self, now: u64) {
        self.bar.refresh();
        self.call(0, now, |p, cx| p.start(cx));
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Runs `f` on page `i` with a fresh `Cx`, then carries out what it asked.
    fn call(&mut self, i: usize, now: u64, f: impl FnOnce(&mut dyn Page, &mut Cx)) {
        let Some(page) = self.stack.get_mut(i) else {
            return;
        };
        f(page.as_mut(), &mut Cx { s: &mut self.s });
        self.dirty = true;
        self.apply(i, now);
    }

    fn apply(&mut self, owner: usize, now: u64) {
        let requests = std::mem::take(&mut self.s.requests);
        for r in requests {
            match r {
                Request::Push(page) => {
                    if self.quit {
                        continue;
                    }
                    self.stack.push(page);
                    let top = self.stack.len() - 1;
                    self.call(top, now, |p, cx| p.start(cx));
                }
                Request::Back => self.pop(now),
                Request::Quit => self.quit_all(),
                Request::Toast(text) => self.toast = Some((text, now + TOAST_MS)),
                Request::Timer { id, ms, every } => self.timers.push(TimerEntry {
                    id,
                    owner,
                    at: now + ms,
                    every: every.then_some(ms),
                }),
                Request::Cancel(id) => self.timers.retain(|t| t.id != id),
                Request::Spawn { id, argv } => self.spawn(id, owner, &argv),
            }
        }
        self.retheme();
    }

    /// Follows `gh0st.palette`, so a change to it shows at once.
    fn retheme(&mut self) {
        let name = self.s.settings.get_str("gh0st.palette").unwrap_or("");
        if name != self.palette {
            self.palette = name.to_string();
            self.s.theme = Theme::new(theme::palette(name).unwrap_or(theme::MINT));
            self.dirty = true;
        }
    }

    fn pop(&mut self, now: u64) {
        let Some(top) = self.stack.len().checked_sub(1) else {
            return;
        };
        self.stop_page(top);
        self.stack.pop();
        match self.stack.len() {
            0 => self.quit = true,
            n => self.call(n - 1, now, |p, cx| p.resume(cx)),
        }
        self.dirty = true;
    }

    fn stop_page(&mut self, i: usize) {
        if let Some(page) = self.stack.get_mut(i) {
            page.stop(&mut Cx { s: &mut self.s });
            self.s.requests.clear();
        }
        self.timers.retain(|t| t.owner < i);
        for j in self.jobs.iter_mut().filter(|j| j.owner >= i) {
            let _ = j.child.kill();
            let _ = j.child.wait();
        }
        self.jobs.retain(|j| j.owner < i);
    }

    /// Closes every page, top first. SIGTERM and SIGINT land here.
    pub fn quit_all(&mut self) {
        for i in (0..self.stack.len()).rev() {
            self.stop_page(i);
        }
        self.stack.clear();
        self.quit = true;
    }

    fn top_bars(&self) -> Bars {
        self.stack.last().map(|p| p.bars()).unwrap_or_default()
    }

    /// One evdev key event.
    pub fn raw_key(&mut self, code: u16, value: i32, now: u64) {
        if let Some(k) = self.kb.event(code, value, now) {
            self.key(&k, now);
        }
    }

    /// Held keys that are due to repeat.
    pub fn repeat(&mut self, now: u64) {
        while let Some(k) = self.kb.repeat_due(now) {
            self.key(&k, now);
        }
    }

    fn key(&mut self, raw: &gh0st_core::keys::Key, now: u64) {
        let Some(top) = self.stack.len().checked_sub(1) else {
            return;
        };
        let typing = self.stack[top].typing();
        let Some(k) = key::map(raw, typing, self.top_bars().soft_slots()) else {
            return;
        };
        if k == Key::Back {
            let page = &mut self.stack[top];
            let close = page.back(&mut Cx { s: &mut self.s });
            self.apply(top, now);
            if close && self.stack.len() == top + 1 {
                self.pop(now);
            }
            self.dirty = true;
        } else {
            self.call(top, now, |p, cx| p.key(k, cx));
        }
    }

    fn spawn(&mut self, id: u64, owner: usize, argv: &[String]) {
        let Some((prog, args)) = argv.split_first() else {
            self.pending.push((id, owner));
            return;
        };
        let child = Command::new(prog)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn();
        match child {
            Ok(child) => {
                if let Some(out) = &child.stdout {
                    let _ = rt::set_nonblock_cloexec(out.as_raw_fd());
                }
                self.jobs.push(JobProc {
                    id,
                    owner,
                    child,
                    buf: Vec::new(),
                    eof: false,
                });
            }
            Err(e) => {
                eprintln!("gh0st app: {prog}: {e}");
                self.pending.push((id, owner));
            }
        }
    }

    /// The fds of running jobs' output, for poll, with their job ids.
    pub fn job_fds(&self) -> Vec<(u64, RawFd)> {
        self.jobs
            .iter()
            .filter(|j| !j.eof)
            .filter_map(|j| Some((j.id, j.child.stdout.as_ref()?.as_raw_fd())))
            .collect()
    }

    /// Reads what job `id` printed and hands out whole lines.
    pub fn read_job(&mut self, id: u64, now: u64) {
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        let mut chunk = [0u8; 4096];
        loop {
            let n = match j.child.stdout.as_mut().map(|o| o.read(&mut chunk)) {
                Some(Ok(0)) | None => {
                    j.eof = true;
                    break;
                }
                Some(Ok(n)) => n,
                Some(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Some(Err(_)) => {
                    j.eof = true;
                    break;
                }
            };
            j.buf.extend_from_slice(&chunk[..n]);
        }
        let mut lines = Vec::new();
        while let Some(i) = j.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = j.buf.drain(..=i).collect();
            lines.push(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
        }
        if j.eof && !j.buf.is_empty() {
            lines.push(String::from_utf8_lossy(&std::mem::take(&mut j.buf)).into_owned());
        }
        let owner = j.owner;
        for line in lines {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Line(line), cx));
        }
        self.reap(now);
    }

    /// Hands out exit codes of jobs whose output has ended and whose process
    /// is gone. Runs on SIGCHLD and after a job's output ends.
    pub fn reap(&mut self, now: u64) {
        let mut done = Vec::new();
        self.jobs.retain_mut(|j| {
            if !j.eof {
                return true;
            }
            match j.child.try_wait() {
                Ok(Some(status)) => {
                    done.push((j.id, j.owner, status.code()));
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    done.push((j.id, j.owner, None));
                    false
                }
            }
        });
        for (id, owner, code) in done {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Done(code), cx));
        }
        for (id, owner) in std::mem::take(&mut self.pending) {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Done(None), cx));
        }
    }

    /// The next time something is due: a timer, a key repeat, the toast's
    /// end, or the clock's minute when the top bar shows.
    pub fn next_at(&self) -> Option<u64> {
        let clock = self.top_bars().top.then_some(self.bar.clock_at);
        rt::earliest(self.timers.iter().map(|t| Some(t.at)).chain([
            self.kb.repeat_at(),
            self.toast.as_ref().map(|t| t.1),
            clock,
            (!self.pending.is_empty()).then_some(0),
        ]))
    }

    /// Fires what is due at `now`.
    pub fn tick(&mut self, now: u64) {
        self.repeat(now);
        if !self.pending.is_empty() {
            self.reap(now);
        }
        while let Some(i) = self.timers.iter().position(|t| t.at <= now) {
            let (id, owner) = (self.timers[i].id, self.timers[i].owner);
            match self.timers[i].every {
                Some(ms) => self.timers[i].at = now + ms,
                None => {
                    self.timers.remove(i);
                }
            }
            self.call(owner, now, |p, cx| p.timer(Timer(id), cx));
        }
        if self.toast.as_ref().is_some_and(|t| t.1 <= now) {
            self.toast = None;
            self.dirty = true;
        }
        if now >= self.bar.clock_at && self.top_bars().top {
            self.bar.refresh();
            self.dirty = true;
        }
    }

    /// Draws the top page and, until the shell draws them, its bars.
    pub fn render(&mut self) {
        self.dirty = false;
        let Some(page) = self.stack.last() else {
            return;
        };
        let t = self.s.theme;
        let bars = page.bars();
        let area = bars.content(&t);
        {
            let mut ui = Ui::new(&mut self.canvas, &t, area);
            page.draw(&mut ui);
        }
        let full = self.canvas.bounds();
        self.canvas.set_clip(full);
        let c = &mut self.canvas;
        if bars.top {
            let s = Status {
                title: page.title(),
                clock: std::str::from_utf8(&self.bar.clock).unwrap_or("--:--"),
                battery: self.bar.battery,
                net: self.bar.net,
            };
            widgets::status_bar(c, &t, &s);
        }
        match &bars.bottom {
            Bottom::None => {}
            Bottom::Hints(h) => {
                let hints: Vec<Hint> = h.iter().map(|(key, label)| Hint { key, label }).collect();
                widgets::hint_bar(c, &t, &hints, None);
            }
            Bottom::Soft(keys) => {
                let slots: [Slot; 5] = std::array::from_fn(|i| Slot {
                    icon: &keys[i].icon,
                    label: &keys[i].label,
                    lit: keys[i].lit,
                });
                widgets::softkey_bar(c, &t, &slots);
            }
        }
        if let Some((text, _)) = &self.toast {
            let r = if bars.bottom != Bottom::None {
                Rect::new(0, t.m.height - t.m.hint_h, t.m.width, t.m.hint_h)
            } else if bars.top {
                Rect::new(0, 0, t.m.width, t.m.bar_h)
            } else {
                Rect::new(0, t.m.height - t.m.hint_h, t.m.width, t.m.hint_h)
            };
            c.fill(r, t.c.surface);
            c.hline(0, r.w, r.y, t.c.accent);
            widgets::text_centre(c, t.f.small_bold, r, text, t.c.fg);
        }
    }
}
