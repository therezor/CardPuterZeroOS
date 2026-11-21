//! The page stack and everything that drives it: keys, timers, jobs, the
//! frame. The device loop, the sim and the test driver all feed this.

use crate::bars::{Bars, Bottom};
use crate::key::{self, Key};
use crate::page::{Cx, Job, JobEvent, Page, Request, Shared, Timer, Watch};
use crate::ui::Ui;
use gh0st_core::keys::Keyboard;
use gh0st_core::{battery, child, job, rt, status};
use gh0st_ui::widgets::{self, Battery, Hint, Net, SoftKey as Slot, Status};
use gh0st_ui::{Canvas, Rect, Theme, theme};
use std::os::fd::RawFd;
use std::path::PathBuf;

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
    job: job::Job,
    /// Bytes of output already handed out as lines.
    sent: usize,
}

struct WatchEntry {
    id: u64,
    owner: usize,
    fd: RawFd,
    /// Bytes waiting for room in the fd.
    out: Vec<u8>,
}

/// What the self-drawn top bar shows. Read at start and on the minute.
struct Bar {
    clock: String,
    clock_at: u64,
    battery_dir: Option<PathBuf>,
    battery: Option<Battery>,
    net: Net,
}

impl Bar {
    fn new() -> Bar {
        Bar {
            clock: "--:--".into(),
            clock_at: 0,
            battery_dir: battery::find(std::path::Path::new("/sys/class/power_supply")),
            battery: None,
            net: Net::None,
        }
    }

    fn refresh(&mut self, h24: bool) {
        let (clock, at) = rt::clock_text(h24);
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
    watches: Vec<WatchEntry>,
    /// Children pages started themselves: pid and owner.
    adopted: Vec<(i32, usize)>,
    /// When the last frame was drawn, for [`Page::frame_ms`].
    pub last_frame: u64,
    /// The shell has another app or itself in front: draw nothing, and
    /// keep no deadline that only the screen needs.
    pub paused: bool,
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
            watches: Vec::new(),
            adopted: Vec::new(),
            last_frame: 0,
            paused: false,
            toast: None,
            bar: Bar::new(),
            palette,
            dirty: true,
            quit: false,
        }
    }

    pub fn start(&mut self, now: u64) {
        self.bar.refresh(self.h24());
        self.call(0, now, |p, cx| p.start(cx));
    }

    /// The clock style the shell uses: `gh0st.clock` is `12h` or 24-hour.
    fn h24(&self) -> bool {
        self.s.settings.get_str("gh0st.clock") != Some("12h")
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Runs `f` on page `i` with a fresh `Cx`, then carries out what it asked.
    fn call(&mut self, i: usize, now: u64, f: impl FnOnce(&mut dyn Page, &mut Cx)) {
        if self.stack.get(i).is_some() {
            self.dirty = true;
        }
        self.call_quiet(i, now, f);
    }

    /// As `call`, but draws only if the page asks.
    fn call_quiet(&mut self, i: usize, now: u64, f: impl FnOnce(&mut dyn Page, &mut Cx)) {
        let Some(page) = self.stack.get_mut(i) else {
            return;
        };
        f(page.as_mut(), &mut Cx { s: &mut self.s });
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
                Request::Spawn {
                    id,
                    argv,
                    mut input,
                } => {
                    self.spawn(id, owner, &argv, input.as_deref());
                    // A password may be in it.
                    if let Some(i) = input.as_mut() {
                        i.fill(0);
                    }
                }
                Request::Watch { id, fd } => self.watches.push(WatchEntry {
                    id,
                    owner,
                    fd,
                    out: Vec::new(),
                }),
                Request::Unwatch(id) => self.watches.retain(|w| w.id != id),
                Request::Write { id, bytes } => {
                    if let Some(w) = self.watches.iter_mut().find(|w| w.id == id) {
                        w.out.extend_from_slice(&bytes);
                        drain(w);
                    }
                }
                Request::Adopt(pid) => self.adopted.push((pid, owner)),
                Request::Redraw => self.dirty = true,
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
        for j in self.jobs.iter().filter(|j| j.owner >= i) {
            // SAFETY: kill(2) on a pid this runtime started.
            unsafe { libc::kill(j.job.pid, libc::SIGTERM) };
        }
        self.jobs.retain(|j| j.owner < i);
        self.watches.retain(|w| w.owner < i);
        self.adopted.retain(|a| a.1 < i);
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
        let raw = self.stack.last().is_some_and(|p| p.raw_keys());
        self.kb.set_repeat_all(raw);
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
        if self.stack[top].raw_keys() {
            self.call_quiet(top, now, |p, cx| p.raw(raw, cx));
            return;
        }
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

    fn spawn(&mut self, id: u64, owner: usize, argv: &[String], input: Option<&[u8]>) {
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        match job::spawn(&args, input) {
            Ok(job) => self.jobs.push(JobProc {
                id,
                owner,
                job,
                sent: 0,
            }),
            Err(e) => {
                eprintln!("gh0st app: {}: {e}", args.first().unwrap_or(&""));
                self.pending.push((id, owner));
            }
        }
    }

    /// The fds of running jobs' output, for poll, with their job ids.
    pub fn job_fds(&self) -> Vec<(u64, RawFd)> {
        self.jobs
            .iter()
            .filter(|j| j.job.fd() >= 0)
            .map(|j| (j.id, j.job.fd()))
            .collect()
    }

    /// Watched fds for poll: id, fd, and whether output waits for room.
    pub fn watch_fds(&self) -> Vec<(u64, RawFd, bool)> {
        self.watches
            .iter()
            .map(|w| (w.id, w.fd, !w.out.is_empty()))
            .collect()
    }

    /// A watched fd is readable or closed.
    pub fn ready(&mut self, id: u64, now: u64) {
        if let Some(owner) = self.watches.iter().find(|w| w.id == id).map(|w| w.owner) {
            self.call_quiet(owner, now, |p, cx| p.ready(Watch(id), cx));
        }
    }

    /// A watched fd has room: send what waits.
    pub fn writable(&mut self, id: u64) {
        if let Some(w) = self.watches.iter_mut().find(|w| w.id == id) {
            drain(w);
        }
    }

    /// True when a frame is wanted and the page's least gap since the last
    /// one has passed.
    pub fn frame_due(&self, now: u64) -> bool {
        self.dirty && !self.quit && !self.paused && now >= self.frame_at()
    }

    /// Another app or the shell is in front: no keys, no frames.
    pub fn pause_input(&mut self) {
        self.kb.reset();
        self.paused = true;
    }

    /// Back in front after a pause: keys held before mean nothing now, and
    /// the whole screen is drawn again.
    pub fn resume_input(&mut self) {
        self.kb.reset();
        self.paused = false;
        self.dirty = true;
    }

    fn frame_at(&self) -> u64 {
        let gap = self.stack.last().map_or(0, |p| p.frame_ms());
        self.last_frame + gap
    }

    /// Reads what job `id` printed and hands out whole lines.
    pub fn read_job(&mut self, id: u64, now: u64) {
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        j.job.read();
        let closed = j.job.fd() < 0;
        let out = &j.job.bytes()[j.sent..];
        let end = if closed {
            out.len()
        } else {
            out.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1)
        };
        let lines: Vec<String> = String::from_utf8_lossy(&out[..end])
            .lines()
            .map(String::from)
            .collect();
        j.sent += end;
        let owner = j.owner;
        for line in lines {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Line(line), cx));
        }
        self.reap(now);
    }

    /// Collects exited children, and hands out the exit code of each job
    /// whose output has ended too. Runs on SIGCHLD and when a job's output
    /// ends.
    pub fn reap(&mut self, now: u64) {
        for (pid, status) in child::reap() {
            if let Some(j) = self.jobs.iter_mut().find(|j| j.job.pid == pid) {
                j.job.exited(status);
            } else if let Some(i) = self.adopted.iter().position(|a| a.0 == pid) {
                let (_, owner) = self.adopted.remove(i);
                let code = libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status));
                self.call_quiet(owner, now, |p, cx| p.exited(pid, code, cx));
            }
        }
        let mut done = Vec::new();
        self.jobs.retain(|j| {
            if j.job.done() {
                done.push((j.id, j.owner, j.job.code()));
                false
            } else {
                true
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
        if self.paused {
            let pending = (!self.pending.is_empty()).then_some(0);
            return rt::earliest(self.timers.iter().map(|t| Some(t.at)).chain([pending]));
        }
        let clock = self.top_bars().top.then_some(self.bar.clock_at);
        rt::earliest(self.timers.iter().map(|t| Some(t.at)).chain([
            self.kb.repeat_at(),
            self.toast.as_ref().map(|t| t.1),
            clock,
            (!self.pending.is_empty()).then_some(0),
            self.dirty.then(|| self.frame_at()),
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
        if !self.paused && now >= self.bar.clock_at && self.top_bars().top {
            let h24 = self.h24();
            self.bar.refresh(h24);
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
            // The dock of open apps is the shell's; an app's own bar has none.
            let s = Status {
                dock: &[],
                front: None,
                title: page.title(),
                clock: &self.bar.clock,
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

/// Writes queued bytes until the fd would block. A broken fd drops them:
/// its reader is gone.
fn drain(w: &mut WatchEntry) {
    while !w.out.is_empty() {
        // SAFETY: writes from a live buffer to an fd the page gave us.
        let n = unsafe { libc::write(w.fd, w.out.as_ptr().cast(), w.out.len()) };
        if n > 0 {
            w.out.drain(..n as usize);
            continue;
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) => return,
            _ => {
                w.out.clear();
                return;
            }
        }
    }
}
