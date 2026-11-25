//! The page stack and everything that drives it: keys, timers, jobs, the
//! frame. The device loop, the sim and the test driver all feed this.

use crate::bars::{Bars, Chip};
use crate::command::{Commands, Step};
use crate::dialog::{self, Answer, Dialog};
use crate::key::{self, Key};
use crate::menu::{Menu, Out, Sheet, SheetOut};
use crate::page::{Cx, Job, JobEvent, Page, Request, Shared, Timer, Watch};
use crate::ui::Ui;
use gh0st_core::keys::code as keys_code;
use gh0st_core::keys::{Keyboard, State};
use gh0st_core::sound::Sound;
use gh0st_core::{battery, child, job, rt, status};
use gh0st_ui::theme::Tone;
use gh0st_ui::widgets::{self, Battery, Hint, Look, Net, Status};
use gh0st_ui::{Canvas, Rect, Theme, theme};
use std::os::fd::RawFd;
use std::path::PathBuf;

const TOAST_MS: u64 = 2000;
/// A busy card shows only for work slower than this, so quick changes
/// don't flash.
const BUSY_GRACE_MS: u64 = 300;
/// The busy bar's step while it shows.
const BUSY_FRAME_MS: u64 = 60;
/// How long Esc is held on the first page to close the app, and a red chip
/// to run.
const HOLD_MS: u64 = 700;
/// A frame of the hold's fill.
const HOLD_FRAME_MS: u64 = 40;
/// Tab numbers for the top bar.
const TABS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];

struct Toast {
    text: String,
    tone: Tone,
    until: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Esc on the first page: the app closes.
    Exit,
    /// A red chip on key 5 to 7: it runs.
    Chip(u8),
    /// A dialog's held yes on 7.
    Dialog,
}

/// A key held to confirm. It exists only while the key is down.
struct Hold {
    code: u16,
    what: Held,
    start: u64,
    next: u64,
    pc: u8,
}

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

/// What the self-drawn top bar shows. Read at start and on the minute, and
/// the battery every `battery::POLL_MS`.
struct Bar {
    clock: String,
    clock_at: u64,
    /// The test driver's time starts at 0 and owes the wall clock nothing:
    /// its minutes fall on whole minutes of its own.
    own_minutes: bool,
    battery_at: u64,
    battery_dir: Option<PathBuf>,
    battery: Option<Battery>,
    net: Net,
    bluetooth: bool,
}

impl Bar {
    fn new() -> Bar {
        Bar {
            clock: "--:--".into(),
            clock_at: 0,
            own_minutes: false,
            battery_at: 0,
            battery_dir: battery::find(std::path::Path::new("/sys/class/power_supply")),
            battery: None,
            net: Net::None,
            bluetooth: false,
        }
    }

    fn refresh(&mut self, h24: bool, now: u64) {
        let (clock, at) = rt::clock_text(h24);
        self.clock = clock;
        self.clock_at = if self.own_minutes {
            (now / 60_000 + 1) * 60_000
        } else {
            at
        };
        self.battery = self.read_battery();
        self.links();
    }

    fn read_battery(&self) -> Option<Battery> {
        self.battery_dir
            .as_deref()
            .and_then(battery::sample)
            .map(|r| Battery {
                percent: r.percent,
                charging: r.charging,
            })
    }

    /// The battery alone. True if it changed.
    fn poll_battery(&mut self, now: u64) -> bool {
        self.battery_at = now + battery::POLL_MS;
        let b = self.read_battery();
        let changed = b != self.battery;
        self.battery = b;
        changed
    }

    /// Network and Bluetooth, which a finished job (nmcli, bluetoothctl)
    /// may have changed.
    fn links(&mut self) {
        self.bluetooth = status::bluetooth();
        self.net = match status::network(
            std::path::Path::new("/sys/class/net"),
            std::path::Path::new("/proc/net/wireless"),
        ) {
            status::Net::None => Net::None,
            status::Net::Wifi(b) => Net::Wifi(b),
            status::Net::Ethernet => Net::Ethernet,
            status::Net::Hotspot => Net::Hotspot,
        };
    }
}

enum Overlay {
    None,
    Menu(Menu),
    Help(Sheet),
    Dialog {
        id: u32,
        owner: usize,
        d: Box<Dialog>,
    },
}

/// Work keys wait for, from [`Cx::busy`].
struct Busy {
    text: String,
    since: u64,
    shown: bool,
    phase: u8,
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
    /// The shell link, when the shell started the app.
    pub link: Option<gh0st_core::link::Link>,
    /// The app menu or Help, while one is open.
    overlay: Overlay,
    /// Keys whose press the menu or a command took: their repeats and
    /// releases don't reach the page.
    taken: Vec<u16>,
    /// The menu row last picked, and the depth of the page it was on.
    menu_last: Option<(usize, u32)>,
    toast: Option<Toast>,
    busy: Option<Busy>,
    /// The app asked to keep the screen on.
    pub awake: bool,
    hold: Option<Hold>,
    /// A key whose hold has run: its repeats and release go nowhere, so
    /// they can't type into what the hold opened.
    spent: Option<u16>,
    /// The chip whose key is down, and that key: drawn pressed.
    pressed: Option<(u16, u8)>,
    /// The top page heard `covered` for what covers it now.
    covered: bool,
    bar: Bar,
    palette: String,
    pub dirty: bool,
    pub quit: bool,
}

impl Runtime {
    pub fn new(page: Box<dyn Page>, config: Option<PathBuf>) -> Runtime {
        let mut s = Shared::new(Theme::default(), config);
        let palette = s.settings.config().palette_name().unwrap_or("").to_string();
        s.theme = Theme::new(theme::named(Some(&palette)));
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
            link: None,
            overlay: Overlay::None,
            taken: Vec::new(),
            menu_last: None,
            toast: None,
            busy: None,
            awake: false,
            hold: None,
            spent: None,
            pressed: None,
            covered: false,
            bar: Bar::new(),
            palette,
            dirty: true,
            quit: false,
        }
    }

    /// The test driver's virtual time: the clock ticks on its minutes.
    pub(crate) fn own_time(&mut self) {
        self.bar.own_minutes = true;
    }

    /// Under `--sim` and in tests: no battery, as in the shell.
    pub(crate) fn off_device(&mut self) {
        self.bar.battery_dir = None;
        self.bar.battery = None;
    }

    pub fn start(&mut self, now: u64) {
        self.bar.refresh(self.h24(), now);
        self.call(0, now, |p, cx| p.start(cx));
    }

    /// The clock style the shell uses: `gh0st.clock` is `12h` or 24-hour.
    fn h24(&self) -> bool {
        self.s.settings.config().clock_24h()
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
        // Once its page has closed, what it asked for after belongs to no
        // one: Steps may have put the next step at the same index.
        let mut closed = false;
        for r in requests {
            if closed {
                match r {
                    Request::Spawn { mut input, .. } => {
                        if let Some(i) = input.as_mut() {
                            i.fill(0);
                        }
                        continue;
                    }
                    Request::Push(_)
                    | Request::Timer { .. }
                    | Request::Watch { .. }
                    | Request::Write { .. }
                    | Request::Adopt(_)
                    | Request::Ask(..)
                    | Request::ShowKeys
                    | Request::Back
                    | Request::Done => continue,
                    _ => {}
                }
            }
            match r {
                Request::Push(page) => {
                    if self.quit {
                        continue;
                    }
                    self.stack.push(page);
                    let top = self.stack.len() - 1;
                    self.call(top, now, |p, cx| p.start(cx));
                }
                Request::Back => {
                    let mine = self.stack.len() == owner + 1;
                    closed = self.pop(now) && mine;
                }
                Request::Done => {
                    let mine = self.stack.len() == owner + 1;
                    self.s.done = true;
                    closed = self.pop(now) && mine;
                }
                Request::Quit => self.quit_all(),
                Request::Toast(text, tone) => self.show_toast(text, tone, now),
                Request::Busy(Some(text)) => match &mut self.busy {
                    Some(b) => {
                        b.text = text;
                        self.dirty |= b.shown;
                    }
                    None => {
                        self.busy = Some(Busy {
                            text,
                            since: now,
                            shown: false,
                            phase: 0,
                        })
                    }
                },
                Request::Busy(None) => {
                    if self.busy.take().is_some_and(|b| b.shown) {
                        self.dirty = true;
                    }
                }
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
                        queue(w, &bytes);
                    }
                }
                Request::Adopt(pid) => self.adopted.push((pid, owner)),
                Request::Redraw => self.dirty = true,
                Request::KeepAwake(on) if on != self.awake => {
                    self.awake = on;
                    if let Some(l) = &self.link {
                        l.send(&["awake", if on { "1" } else { "0" }]);
                    }
                }
                Request::KeepAwake(_) => {}
                Request::Open(name) => {
                    if let Some(l) = &self.link {
                        l.send(&["open", &name]);
                    }
                }
                Request::Sound(s) => self.sound(s),
                Request::Notify(text) => match &self.link {
                    Some(l) if self.paused => {
                        l.send(&["toast", &text]);
                    }
                    _ => self.show_toast(text, Tone::Info, now),
                },
                Request::Ask(id, d) => {
                    // A question the work needs answered takes over.
                    self.busy = None;
                    self.s.asking = true;
                    self.overlay = Overlay::Dialog {
                        id,
                        owner,
                        d: Box::new(d),
                    };
                    self.dirty = true;
                }
                Request::ShowKeys => {
                    self.overlay = Overlay::Help(self.help(owner, None));
                    self.dirty = true;
                }
            }
        }
        self.retheme();
        self.check_cover(now);
    }

    /// Follows `gh0st.palette`, so a change to it shows at once.
    pub(crate) fn retheme(&mut self) {
        let name = self.s.settings.config().palette_name().unwrap_or("");
        if name != self.palette {
            self.palette = name.to_string();
            self.s.theme = Theme::new(theme::named(Some(name)));
            self.dirty = true;
        }
    }

    /// The busy card's text, while it shows.
    pub(crate) fn busy_text(&self) -> Option<&str> {
        self.busy
            .as_ref()
            .filter(|b| b.shown)
            .map(|b| b.text.as_str())
    }

    /// The open dialog's title and the error under its field.
    pub(crate) fn dialog_text(&self) -> Option<(&str, Option<&str>)> {
        match &self.overlay {
            Overlay::Dialog { d, .. } => Some((d.title.as_str(), d.error.as_deref())),
            _ => None,
        }
    }

    pub(crate) fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|t| t.text.as_str())
    }

    fn show_toast(&mut self, text: String, tone: Tone, now: u64) {
        self.toast = Some(Toast {
            text,
            tone,
            until: now + TOAST_MS,
        });
        self.dirty = true;
    }

    /// Closes the top page. False when it stays: the last page of an app
    /// that can't close.
    fn pop(&mut self, now: u64) -> bool {
        if self.stack.len() == 1 && !self.s.closable {
            self.s.done = false;
            return false;
        }
        self.overlay = Overlay::None;
        self.s.asking = false;
        self.menu_last = None;
        // The work a closed page waited on is no longer anyone's to show.
        self.busy = None;
        let Some(top) = self.stack.len().checked_sub(1) else {
            return false;
        };
        self.stop_page(top);
        self.stack.pop();
        match self.stack.len() {
            0 => self.quit = true,
            n => self.call(n - 1, now, |p, cx| p.resume(cx)),
        }
        // Heard by the parent's resume, and by no page after it.
        self.s.done = false;
        self.dirty = true;
        true
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
        self.check_cover(now);
    }

    /// Held keys that are due to repeat.
    pub fn repeat(&mut self, now: u64) {
        while let Some(k) = self.kb.repeat_due(now) {
            self.key(&k, now);
        }
    }

    /// Tells the top page once when Help, the menu, a dialog or the busy
    /// card comes over it: keys, releases too, go there until it closes.
    fn check_cover(&mut self, now: u64) {
        let over =
            !matches!(self.overlay, Overlay::None) || self.busy.as_ref().is_some_and(|b| b.shown);
        if !over {
            self.covered = false;
            return;
        }
        if self.covered {
            return;
        }
        self.covered = true;
        if let Some(top) = self.stack.len().checked_sub(1) {
            self.call_quiet(top, now, |p, cx| p.covered(cx));
        }
    }

    fn key(&mut self, raw: &gh0st_core::keys::Key, now: u64) {
        let Some(top) = self.stack.len().checked_sub(1) else {
            return;
        };
        if raw.state == State::Release {
            if self.hold.as_ref().is_some_and(|h| h.code == raw.code) {
                self.let_go(now);
            }
            if self.pressed.is_some_and(|p| p.0 == raw.code) {
                self.pressed = None;
                self.dirty = true;
            }
        }
        if self.spent == Some(raw.code) {
            if raw.state != State::Press {
                if raw.state == State::Release {
                    self.spent = None;
                }
                return;
            }
            self.spent = None;
        }
        let taken = self.taken.iter().position(|&c| c == raw.code);
        if let (State::Release, Some(i)) = (raw.state, taken) {
            self.taken.swap_remove(i);
            return;
        }
        // While the busy card shows, keys wait, so a second press can't
        // start the change again. Esc hides the card; the work goes on.
        // Before it shows, keys work as ever.
        if self.busy.as_ref().is_some_and(|b| b.shown) {
            if raw.state == State::Press {
                self.take(raw.code);
                if raw.code == keys_code::ESC {
                    self.busy = None;
                    self.dirty = true;
                }
            }
            if raw.state != State::Release {
                return;
            }
        }
        // Help (Fn+H) in every app, raw-key pages too.
        let in_dialog = matches!(self.overlay, Overlay::Dialog { .. });
        if raw.code == keys_code::HELP && !in_dialog {
            if raw.state == State::Press {
                self.take(raw.code);
                if let Overlay::Help(_) = self.overlay {
                    self.close_overlay();
                } else {
                    self.close_overlay();
                    self.overlay = Overlay::Help(self.help(top, None));
                }
                self.dirty = true;
            }
            return;
        }
        if !matches!(self.overlay, Overlay::None) {
            if raw.state == State::Press {
                self.take(raw.code);
            }
            self.overlay_key(raw, now);
            return;
        }
        if raw.state != State::Release {
            let typing = self.stack[top].typing();
            let held = self.taken.contains(&raw.code);
            match self.commands_of(top).lookup(raw, typing) {
                Some(hit) => {
                    if raw.state == State::Press {
                        self.take(raw.code);
                        self.sound(Sound::Select);
                    }
                    if raw.state == State::Press || hit.repeats {
                        self.call(top, now, |p, cx| p.command(hit.id, hit.step, cx));
                    }
                    return;
                }
                // Held from a command that has gone, as one that changed
                // the mode: its repeats mean nothing now.
                None if held => return,
                None => {}
            }
        }
        let raw_page = self.stack[top].raw_keys();
        // Esc goes back a step on every page, raw-key ones too; Fn+Esc is
        // the Esc key itself, for a program that wants it.
        if raw.code == keys_code::ESC {
            if raw.fn_ && raw_page {
                let plain = gh0st_core::keys::Key { fn_: false, ..*raw };
                self.call_quiet(top, now, |p, cx| p.raw(&plain, cx));
            } else if raw.state == State::Press {
                self.back(top, raw.code, now);
            }
            return;
        }
        if raw_page {
            self.call_quiet(top, now, |p, cx| p.raw(raw, cx));
            return;
        }
        let typing = self.stack[top].typing();
        let bars = self.top_bars();
        let plain = !raw.ctrl && !raw.alt && !raw.fn_;
        if raw.ch == Some('8') && plain && !typing && bars.menu && (bars.bottom || bars.corner) {
            if raw.state == State::Press {
                self.take(raw.code);
                self.sound(Sound::Select);
                self.open_menu(top);
            }
            return;
        }
        let chips = self.chips_of(top, &bars);
        let live = chips.each_ref().map(|c| c.is_some());
        let Some(k) = key::map(raw, typing, live) else {
            return;
        };
        match k {
            Key::Back => self.back(top, raw.code, now),
            Key::Soft(n) => {
                if let Some(chip) = chips[(n - 4) as usize].clone() {
                    self.press_chip(top, n, raw.code, chip, now);
                }
            }
            _ => {
                if let Some(s) = key_sound(k) {
                    self.sound(s);
                }
                self.call(top, now, |p, cx| p.key(k, cx));
            }
        }
    }

    /// A system sound through the shell: only in front, only while the
    /// user has sounds on. Without a shell, nothing plays.
    fn sound(&mut self, s: Sound) {
        let quiet = self.stack.last().is_some_and(|p| p.quiet() || p.raw_keys());
        if quiet || self.s.paused || !self.s.settings.config().sounds() {
            return;
        }
        if let Some(l) = &self.link {
            l.send(&["sound", s.name()]);
        }
    }

    /// Esc or 4: the page goes back a step if it can. On the first page,
    /// closing needs a hold, started only by this press.
    fn back(&mut self, top: usize, code: u16, now: u64) {
        let page = &mut self.stack[top];
        let close = page.back(&mut Cx { s: &mut self.s });
        self.apply(top, now);
        self.dirty = true;
        if !close || self.stack.len() != top + 1 {
            self.sound(Sound::Back);
            return;
        }
        if top > 0 {
            self.sound(Sound::Back);
            self.pop(now);
            return;
        }
        if !self.s.closable {
            return;
        }
        self.hold = Some(Hold {
            code,
            what: Held::Exit,
            start: now,
            next: now + HOLD_FRAME_MS,
            pc: 0,
        });
        self.show_toast("Hold to exit".into(), Tone::Info, now);
    }

    fn press_chip(&mut self, top: usize, n: u8, code: u16, chip: Chip, now: u64) {
        if !chip.runs_key() {
            return;
        }
        self.pressed = Some((code, n));
        self.dirty = true;
        self.sound(Sound::Select);
        if chip.hold {
            self.hold = Some(Hold {
                code,
                what: Held::Chip(n),
                start: now,
                next: now + HOLD_FRAME_MS,
                pc: 0,
            });
            return;
        }
        self.run_chip(top, n, chip.command, now);
    }

    fn run_chip(&mut self, top: usize, n: u8, command: Option<u32>, now: u64) {
        match command {
            Some(id) => self.call(top, now, |p, cx| p.command(id, Step::Run, cx)),
            None => self.call(top, now, |p, cx| p.key(Key::Soft(n), cx)),
        }
    }

    /// Closes the dialog and tells its page how.
    fn answer(&mut self, yes: bool, now: u64) {
        let Overlay::Dialog { id, owner, mut d } =
            std::mem::replace(&mut self.overlay, Overlay::None)
        else {
            return;
        };
        self.s.asking = false;
        let answer = if yes {
            Answer::Yes(d.take_text())
        } else {
            // A half-typed password goes too, zeroed in place.
            d.take_text().into_bytes().fill(0);
            Answer::No
        };
        self.dirty = true;
        self.call(owner, now, |p, cx| p.answer(id, answer, cx));
    }

    /// The held key came up before the hold was done: nothing happens. A
    /// red chip says it wants holding.
    fn let_go(&mut self, now: u64) {
        let Some(h) = self.hold.take() else {
            return;
        };
        if let (Held::Dialog, Overlay::Dialog { d, .. }) = (h.what, &self.overlay) {
            let say = format!("Hold to {}", d.yes);
            self.show_toast(say, Tone::Info, now);
        }
        if let Held::Chip(n) = h.what {
            let top = self.stack.len().saturating_sub(1);
            let bars = self.top_bars();
            let word = self.chips_of(top, &bars)[(n - 4) as usize]
                .as_ref()
                .map(|c| c.word.to_lowercase())
                .unwrap_or_default();
            self.show_toast(format!("Hold to {word}"), Tone::Info, now);
        }
        self.dirty = true;
    }

    /// The chips over keys 4 to 7: the page's own, else its live
    /// commands' (5 to 7 only).
    fn chips_of(&self, i: usize, bars: &Bars) -> [Option<Chip>; 4] {
        let from_cmds = self.commands_of(i).chips();
        std::array::from_fn(|k| {
            bars.chips[k].clone().or_else(|| {
                k.checked_sub(1)
                    .and_then(|c| from_cmds[c])
                    .map(|(id, icon, word, on)| Chip::new(icon, word).runs(id).disabled(!on))
            })
        })
    }

    fn open_menu(&mut self, top: usize) {
        let select = self.menu_last.filter(|l| l.0 == top).map(|l| l.1);
        let cmds = self.commands_of(top);
        self.overlay = Overlay::Menu(Menu::new(top, cmds, select, self.s.closable));
        self.dirty = true;
    }

    /// Help for page `i`: its chips and the menu, Esc and Enter, its
    /// commands, then the keys that work everywhere.
    fn help(&self, i: usize, menu: Option<Menu>) -> Sheet {
        let bars = self.stack.get(i).map(|p| p.bars()).unwrap_or_default();
        let cmds = self.commands_of(i);
        let mut under = Vec::new();
        if bars.bottom {
            for (k, chip) in self.chips_of(i, &bars).iter().enumerate() {
                let Some(chip) = chip else {
                    continue;
                };
                let mut keys = (k + 4).to_string();
                // An icon-only chip is named by the command it runs.
                let mut word = chip.word.clone();
                if let Some(c) = chip.command.and_then(|id| cmds.iter().find(|c| c.id == id)) {
                    if !c.keys.is_empty() {
                        keys = format!("{keys} {}", c.keys_text());
                    }
                    if word.is_empty() {
                        word = c.label.to_string();
                    }
                }
                under.push((keys, word));
            }
            if bars.menu {
                under.push(("8".into(), "Menu".into()));
            }
        } else if bars.corner && bars.menu {
            under.push(("8".into(), "Menu".into()));
        }
        let back = if i == 0 && self.s.closable {
            "Back; hold to exit"
        } else {
            "Back"
        };
        let keys = vec![
            ("Esc".into(), back.into()),
            ("Enter".into(), "Open, or the main thing".into()),
        ];
        let listed: Vec<&crate::command::Command> = cmds.iter().collect();
        let t = &self.s.theme;
        let about = self.stack.get(i).map(|p| p.about()).unwrap_or("");
        let w = t.m.width - 2 * t.m.margin - t.m.scrollbar_w - 18;
        let about = crate::ui::wrap(t.f.small, about, w);
        Sheet::new(about, under, keys, listed.into_iter(), menu)
    }

    fn take(&mut self, code: u16) {
        // Releases can go missing (a pause drops them): keep this small.
        if self.taken.len() >= 8 {
            self.taken.remove(0);
        }
        self.taken.push(code);
    }

    fn commands_of(&self, i: usize) -> Commands {
        let mut c = Commands::new();
        if let Some(p) = self.stack.get(i) {
            p.commands(&mut c);
        }
        c
    }

    fn close_overlay(&mut self) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        let menu = match overlay {
            Overlay::Menu(m) => Some(m),
            Overlay::Help(s) => s.menu,
            Overlay::None | Overlay::Dialog { .. } => None,
        };
        // Back on a setting just changed, to change it again; anywhere else
        // the menu opens at the top, on the most used row.
        self.menu_last = menu.and_then(|m| m.changed().map(|id| (m.owner, id)));
        self.dirty = true;
    }

    fn overlay_key(&mut self, raw: &gh0st_core::keys::Key, now: u64) {
        if raw.state != State::Release {
            self.dirty = true;
        }
        if let Overlay::Dialog { d, .. } = &mut self.overlay {
            if d.hold && raw.ch == Some('7') && raw.state == State::Press {
                self.pressed = Some((raw.code, 7));
                self.hold = Some(Hold {
                    code: raw.code,
                    what: Held::Dialog,
                    start: now,
                    next: now + HOLD_FRAME_MS,
                    pc: 0,
                });
                return;
            }
            match d.key(raw) {
                dialog::Out::Nothing => {}
                dialog::Out::Yes => {
                    self.sound(Sound::Select);
                    self.answer(true, now);
                }
                dialog::Out::No => {
                    self.sound(Sound::Back);
                    self.answer(false, now);
                }
            }
            return;
        }
        match &mut self.overlay {
            Overlay::None | Overlay::Dialog { .. } => {}
            Overlay::Help(s) => match s.key(raw) {
                SheetOut::Nothing => {}
                // One level back: to the menu it came from, else the page.
                SheetOut::Back => {
                    let menu = match std::mem::replace(&mut self.overlay, Overlay::None) {
                        Overlay::Help(s) => s.menu,
                        _ => None,
                    };
                    if let Some(m) = menu {
                        self.overlay = Overlay::Menu(m);
                    }
                }
                SheetOut::Menu => {
                    let top = self.stack.len().saturating_sub(1);
                    self.overlay = Overlay::None;
                    self.open_menu(top);
                }
            },
            Overlay::Menu(m) => {
                let owner = m.owner;
                // 8 opened it and 8 closes it.
                let eight = raw.ch == Some('8') && !raw.ctrl && !raw.alt && !raw.fn_;
                if eight {
                    if raw.state == State::Press {
                        self.close_overlay();
                    }
                    return;
                }
                let moved = raw.state != State::Release && raw.nav(true).is_some();
                match m.key(raw) {
                    Out::Nothing if moved => self.sound(Sound::Move),
                    Out::Nothing => {}
                    Out::Close => {
                        self.sound(Sound::Back);
                        self.close_overlay();
                    }
                    Out::Quit => {
                        self.close_overlay();
                        self.quit_all();
                    }
                    Out::Help => {
                        if let Overlay::Menu(m) =
                            std::mem::replace(&mut self.overlay, Overlay::None)
                        {
                            let owner = m.owner;
                            self.overlay = Overlay::Help(self.help(owner, Some(m)));
                        }
                    }
                    Out::Run { id, step, close } => {
                        self.sound(Sound::Select);
                        if close {
                            self.close_overlay();
                        }
                        let depth = self.stack.len();
                        self.call(owner, now, |p, cx| p.command(id, step, cx));
                        if !close {
                            let fresh = self.commands_of(owner);
                            match &mut self.overlay {
                                // The command opened or closed a page: the
                                // menu belonged to the old one.
                                _ if self.stack.len() != depth => self.close_overlay(),
                                Overlay::Menu(m) => m.refresh(fresh),
                                _ => {}
                            }
                        }
                    }
                }
            }
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

    /// A job is still running, or has yet to report that it couldn't start.
    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty() || !self.pending.is_empty()
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
    pub fn pause_input(&mut self, now: u64) {
        // A dialog waits for its answer; the menu and Help close.
        if !matches!(self.overlay, Overlay::Dialog { .. }) {
            self.overlay = Overlay::None;
        }
        self.hold = None;
        self.pressed = None;
        self.taken.clear();
        self.kb.reset();
        self.paused = true;
        self.s.paused = true;
        self.tell_front(false, now);
    }

    fn tell_front(&mut self, front: bool, now: u64) {
        for i in 0..self.stack.len() {
            self.call_quiet(i, now, |p, cx| p.front(front, cx));
        }
    }

    /// Back in front after a pause: keys held before mean nothing now, and
    /// the whole screen is drawn again.
    pub fn resume_input(&mut self, now: u64) {
        // Settings may have changed sounds or the palette meanwhile.
        self.s.settings.reload();
        self.retheme();
        self.kb.reset();
        self.paused = false;
        self.s.paused = false;
        self.dirty = true;
        self.tell_front(true, now);
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
        if !done.is_empty() {
            self.bar.links();
        }
        for (id, owner, code) in done {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Done(code), cx));
        }
        for (id, owner) in std::mem::take(&mut self.pending) {
            self.call(owner, now, |p, cx| p.job(Job(id), JobEvent::Done(None), cx));
        }
    }

    /// The next time something is due: a timer, a key repeat, the toast's
    /// end, or while the top bar shows, the clock's minute and the battery
    /// read.
    pub fn next_at(&self) -> Option<u64> {
        if self.quit {
            return None;
        }
        if self.paused {
            let pending = (!self.pending.is_empty()).then_some(0);
            return rt::earliest(self.timers.iter().map(|t| Some(t.at)).chain([pending]));
        }
        let top = self.top_bars().top;
        let clock = top.then_some(self.bar.clock_at);
        let battery = (top && self.bar.battery_dir.is_some()).then_some(self.bar.battery_at);
        rt::earliest(self.timers.iter().map(|t| Some(t.at)).chain([
            self.kb.repeat_at(),
            self.toast.as_ref().map(|t| t.until),
            self.busy.as_ref().map(|b| match b.shown {
                true => self.last_frame + BUSY_FRAME_MS,
                false => b.since + BUSY_GRACE_MS,
            }),
            self.hold.as_ref().map(|h| h.next.min(h.start + HOLD_MS)),
            clock,
            battery,
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
        self.hold_tick(now);
        if let Some(b) = &mut self.busy {
            let first = !b.shown && now >= b.since + BUSY_GRACE_MS;
            if first || b.shown && now >= self.last_frame + BUSY_FRAME_MS {
                b.shown = true;
                // Back and forth, once every 1.5 s.
                let t = ((now - b.since) % 1500) * 510 / 1500;
                b.phase = if t < 256 { t } else { 510 - t } as u8;
                self.dirty = true;
            }
        }
        self.check_cover(now);
        // A toast stays while its hold lasts.
        if self.toast.as_ref().is_some_and(|t| t.until <= now) && self.hold.is_none() {
            self.toast = None;
            self.dirty = true;
        }
        if !self.paused && self.top_bars().top {
            if now >= self.bar.clock_at {
                let h24 = self.h24();
                self.bar.refresh(h24, now);
                self.dirty = true;
            }
            if self.bar.battery_dir.is_some()
                && now >= self.bar.battery_at
                && self.bar.poll_battery(now)
            {
                self.dirty = true;
            }
        }
    }

    fn hold_tick(&mut self, now: u64) {
        let Some(h) = self.hold.as_mut() else {
            return;
        };
        if now < h.next && now < h.start + HOLD_MS {
            return;
        }
        if now < h.start + HOLD_MS {
            h.pc = ((now - h.start) * 100 / HOLD_MS) as u8;
            h.next = now + HOLD_FRAME_MS;
            self.dirty = true;
            return;
        }
        let what = h.what;
        self.spent = Some(h.code);
        self.hold = None;
        self.pressed = None;
        self.dirty = true;
        match what {
            Held::Exit => self.quit_all(),
            Held::Dialog => self.answer(true, now),
            Held::Chip(n) => {
                let top = self.stack.len().saturating_sub(1);
                let bars = self.top_bars();
                let command = self.chips_of(top, &bars)[(n - 4) as usize]
                    .as_ref()
                    .and_then(|c| c.command);
                self.run_chip(top, n, command, now);
            }
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
            gh0st_ui::lint::area(Some(area));
            page.draw(&mut ui);
            gh0st_ui::lint::area(None);
        }
        // Above the action bar the menu and Help draw.
        let panel_h = (t.m.height - t.m.hint_h - area.y).min(area.h);
        let panel = Rect::new(area.x, area.y, area.w, panel_h);
        match &self.overlay {
            Overlay::None => {}
            Overlay::Menu(m) => m.draw(&mut self.canvas, &t, panel),
            Overlay::Help(s) => s.draw(&mut self.canvas, &t, panel),
            Overlay::Dialog { d, .. } => d.draw(&mut self.canvas, &t, area),
        }
        if let Some(b) = self.busy.as_ref().filter(|b| b.shown) {
            widgets::busy(&mut self.canvas, &t, area, &b.text, b.phase);
        }
        let full = self.canvas.bounds();
        self.canvas.set_clip(full);
        let top = self.stack.len() - 1;
        let root = &self.stack[0];
        if bars.top {
            let icon = match page.icon() {
                "" => root.icon(),
                i => i,
            };
            let context = match page.context() {
                Some(c) => c,
                None if top > 0 => root.title(),
                None => "",
            };
            let (n, front) = page.tabs();
            let s = Status {
                dock: &[],
                front: None,
                icon,
                title: page.title(),
                context,
                tabs: &TABS[..n.min(TABS.len())],
                front_tab: front,
                item: page.status(),
                clock: &self.bar.clock,
                battery: self.bar.battery,
                net: self.bar.net,
                bluetooth: self.bar.bluetooth,
                notices: 0,
            };
            widgets::status_bar(&mut self.canvas, &t, &s);
        }
        let none = widgets::Chip::default();
        let menu = |look| widgets::Chip {
            icon: "menu",
            word: "",
            look,
        };
        let bottom = bars.bottom || !matches!(self.overlay, Overlay::None);
        match &self.overlay {
            Overlay::Menu(_) => {
                let chips = [none, none, none, none, menu(Look::On)];
                widgets::action_bar(&mut self.canvas, &t, &chips);
            }
            Overlay::Help(_) => {
                let chips = [none, none, none, none, menu(Look::Normal)];
                widgets::action_bar(&mut self.canvas, &t, &chips);
            }
            Overlay::Dialog { d, .. } if d.typing() => {
                let h: Vec<Hint> = d
                    .hints()
                    .into_iter()
                    .map(|(key, label)| Hint { key, label })
                    .collect();
                widgets::hint_bar(&mut self.canvas, &t, &h, None);
            }
            Overlay::Dialog { d, .. } => {
                let mut chips = [none; 5];
                // A chip's word starts with a capital; a hint's doesn't.
                let mut word = d.yes.clone();
                if let Some(f) = word.get_mut(..1) {
                    f.make_ascii_uppercase();
                }
                if d.hold {
                    let look = match &self.hold {
                        Some(h) if h.what == Held::Dialog => Look::Hold(h.pc),
                        _ => Look::Hold(0),
                    };
                    chips[3] = widgets::Chip {
                        icon: "trash-2",
                        word: &word,
                        look,
                    };
                }
                widgets::action_bar(&mut self.canvas, &t, &chips);
            }
            Overlay::None if bars.corner && bars.menu => {
                widgets::corner_chip(&mut self.canvas, &t, &menu(Look::Normal));
            }
            Overlay::None if !bars.bottom => {}
            Overlay::None if page.typing() && bars.search.is_some() => {
                let q = bars.search.as_deref().unwrap_or_default();
                widgets::search_bar(&mut self.canvas, &t, q);
            }
            // A mode with no chips, such as moving a row, explains its keys.
            Overlay::None
                if page.typing()
                    || (!bars.hints.is_empty()
                        && self.chips_of(top, &bars).iter().all(Option::is_none)) =>
            {
                let mut h: Vec<Hint> = bars
                    .hints
                    .iter()
                    .map(|(key, label)| Hint { key, label })
                    .collect();
                if h.is_empty() {
                    h = vec![
                        Hint {
                            key: "Esc",
                            label: "back",
                        },
                        Hint {
                            key: "Enter",
                            label: "ok",
                        },
                    ];
                }
                let help = Hint {
                    key: "Fn h",
                    label: "help",
                };
                widgets::hint_bar(&mut self.canvas, &t, &h, Some(help));
            }
            Overlay::None => {
                let found = self.chips_of(top, &bars);
                let mut chips = [none; 5];
                for (k, c) in found.iter().enumerate() {
                    let Some(c) = c else {
                        continue;
                    };
                    let n = k as u8 + 4;
                    let look = match &self.hold {
                        Some(h) if h.what == Held::Chip(n) => Look::Hold(h.pc),
                        // A red chip stays red while its key is down.
                        _ if self.pressed.is_some_and(|p| p.1 == n)
                            && !matches!(c.look, Look::Danger | Look::DangerMain) =>
                        {
                            Look::Pressed
                        }
                        _ => c.look,
                    };
                    chips[k] = widgets::Chip {
                        icon: &c.icon,
                        word: &c.word,
                        look,
                    };
                }
                if bars.menu {
                    chips[4] = menu(Look::Normal);
                }
                // Keys wait while the busy card shows, so no chip offers one.
                if self.busy.as_ref().is_some_and(|b| b.shown) {
                    chips = [none; 5];
                }
                if bars.over {
                    widgets::action_bar_over(&mut self.canvas, &t, &chips);
                } else {
                    widgets::action_bar(&mut self.canvas, &t, &chips);
                }
            }
        }
        if let Some(toast) = &self.toast {
            let foot = if bottom {
                t.m.height - t.m.hint_h
            } else {
                t.m.height
            };
            let held = self
                .hold
                .as_ref()
                .filter(|h| h.what == Held::Exit)
                .map(|h| h.pc);
            widgets::toast(&mut self.canvas, &t, foot, toast.tone, &toast.text, held);
        }
    }
}

/// Bytes a watch holds for a reader that isn't reading. A terminal answers
/// each query a program prints; a program that never reads its input would
/// otherwise grow this until the OOM killer comes.
const OUT_MAX: usize = 1024 * 1024;

/// Queues `bytes` for the watch's fd and writes what fits. Past `OUT_MAX`
/// the new bytes are dropped, as a full tty drops input.
fn queue(w: &mut WatchEntry, bytes: &[u8]) {
    let room = OUT_MAX.saturating_sub(w.out.len());
    w.out.extend_from_slice(&bytes[..bytes.len().min(room)]);
    drain(w);
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

/// The sound a page key makes on its own: a move, or a choice.
fn key_sound(k: Key) -> Option<Sound> {
    match k {
        Key::Up | Key::Down | Key::Left | Key::Right => Some(Sound::Move),
        Key::Enter => Some(Sound::Select),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bars::Bars;

    struct Plain(bool);

    impl Page for Plain {
        fn title(&self) -> &str {
            "Plain"
        }
        fn draw(&self, _: &mut Ui) {}
        fn bars(&self) -> Bars {
            if self.0 { Bars::top() } else { Bars::none() }
        }
    }

    /// A fake supply. `status` flips to Discharging halfway through 10 s of
    /// virtual time. Returns the wakes and the frames.
    fn run(top: bool, paused: bool) -> (u32, u32) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gh0st-poll-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("capacity"), "50\n").unwrap();
        std::fs::write(dir.join("status"), "Charging\n").unwrap();
        let mut rt = Runtime::new(Box::new(Plain(top)), Some(dir.join("config.json")));
        rt.off_device();
        rt.bar.battery_dir = Some(dir.clone());
        rt.start(0);
        rt.bar.clock_at = u64::MAX;
        rt.paused = paused;
        rt.tick(0);
        rt.render();
        let (mut wakes, mut frames) = (0, 0);
        while let Some(at) = rt.next_at().filter(|&at| at <= 10_000) {
            if at > 5_000 {
                std::fs::write(dir.join("status"), "Discharging\n").unwrap();
            }
            rt.tick(at);
            wakes += 1;
            if rt.dirty {
                rt.render();
                frames += 1;
            }
            assert!(wakes < 100, "the read's deadline never moves on");
        }
        std::fs::remove_dir_all(&dir).unwrap();
        (wakes, frames)
    }

    #[test]
    fn a_lit_bar_reads_the_battery_every_poll_and_draws_only_on_a_change() {
        let reads = (10_000 / battery::POLL_MS) as u32;
        assert_eq!(run(true, false), (reads, 1));
        assert_eq!(run(false, false), (0, 0));
        assert_eq!(run(true, true), (0, 0));
    }

    mod keys {
        use crate::prelude::*;
        use crate::testing::Driver;
        use gh0st_core::keys::Key as RawKey;
        use std::cell::RefCell;
        use std::rc::Rc;

        type Log = Rc<RefCell<Vec<String>>>;

        /// Logs what reaches it. `chips` sets a chip on 5 and a red one on 6.
        struct Probe {
            log: Log,
            typing: bool,
            raw: bool,
        }

        const ERASE: u32 = 1;

        impl Page for Probe {
            fn title(&self) -> &str {
                "Probe"
            }
            fn draw(&self, _: &mut Ui) {}
            fn bars(&self) -> Bars {
                Bars::top()
                    .chip(5, Chip::new("search", "Find"))
                    .chip(6, Chip::new("trash-2", "Erase").hold().runs(ERASE))
            }
            fn typing(&self) -> bool {
                self.typing
            }
            fn raw_keys(&self) -> bool {
                self.raw
            }
            fn key(&mut self, key: Key, cx: &mut Cx) {
                self.log.borrow_mut().push(format!("{key:?}"));
                if key == Key::Enter {
                    let log = self.log.clone();
                    cx.push(Probe {
                        log,
                        typing: false,
                        raw: false,
                    });
                }
            }
            fn raw(&mut self, key: &RawKey, _: &mut Cx) {
                if key.state == gh0st_core::keys::State::Press {
                    self.log
                        .borrow_mut()
                        .push(format!("raw {} fn {}", key.code, key.fn_));
                }
            }
            fn command(&mut self, id: u32, _: Step, _: &mut Cx) {
                self.log.borrow_mut().push(format!("command {id}"));
            }
            fn covered(&mut self, _: &mut Cx) {
                self.log.borrow_mut().push("covered".into());
            }
        }

        fn probe(typing: bool, raw: bool) -> (Driver, Log) {
            let log = Log::default();
            let d = Driver::new(Probe {
                log: log.clone(),
                typing,
                raw,
            });
            (d, log)
        }

        /// A hold chip that starts typing, as one that opens a password
        /// dialog does.
        struct Typer {
            log: Log,
            typing: bool,
        }

        impl Page for Typer {
            fn title(&self) -> &str {
                "Typer"
            }
            fn draw(&self, _: &mut Ui) {}
            fn bars(&self) -> Bars {
                Bars::top().chip(7, Chip::new("trash-2", "Delete").hold().runs(ERASE))
            }
            fn typing(&self) -> bool {
                self.typing
            }
            fn key(&mut self, key: Key, _: &mut Cx) {
                self.log.borrow_mut().push(format!("{key:?}"));
            }
            fn command(&mut self, _: u32, _: Step, _: &mut Cx) {
                self.typing = true;
            }
        }

        #[test]
        fn a_held_key_doesnt_type_into_what_its_hold_opened() {
            let log = Log::default();
            let mut d = Driver::new(Typer {
                log: log.clone(),
                typing: false,
            });
            d.keys("down 7").wait(2500).keys("up 7").wait(100);
            assert!(log.borrow().is_empty(), "{:?}", log.borrow());
            d.keys("7").wait(50);
            assert_eq!(log.borrow().as_slice(), ["Char('7')"]);
        }

        #[test]
        fn esc_on_the_first_page_closes_only_when_held() {
            let (mut d, _) = probe(false, false);
            d.keys("esc").wait(1000);
            assert!(!d.quit(), "a tap only asks for a hold");
            assert_eq!(d.toast(), Some("Hold to exit"));
            d.keys("down esc").wait(500);
            assert!(!d.quit());
            d.keys("up esc").wait(1000);
            assert!(!d.quit(), "let go early: nothing happens");
            d.keys("down esc").wait(800);
            assert!(d.quit());
        }

        #[test]
        fn a_hold_begun_on_a_deeper_page_goes_back_and_no_further() {
            let (mut d, _) = probe(false, false);
            d.keys("enter");
            d.keys("down esc").wait(1500);
            assert!(!d.quit());
            assert_eq!(d.toast(), None);
        }

        #[test]
        fn four_is_esc_and_five_runs_its_chip() {
            let (mut d, log) = probe(false, false);
            d.keys("enter\n5\n4\n4").wait(100);
            assert_eq!(*log.borrow(), ["Enter", "Soft(5)"]);
            assert!(!d.quit());
            assert_eq!(
                d.toast(),
                Some("Hold to exit"),
                "the second 4 was on the first page"
            );
        }

        #[test]
        fn a_red_chip_runs_only_when_held() {
            let (mut d, log) = probe(false, false);
            d.keys("6").wait(100);
            assert!(log.borrow().is_empty());
            assert_eq!(d.toast(), Some("Hold to erase"));
            d.keys("down 6").wait(800);
            assert_eq!(*log.borrow(), [format!("command {ERASE}")]);
        }

        #[test]
        fn while_typing_digits_type() {
            let (mut d, log) = probe(true, false);
            d.keys("5\n4\n8");
            assert_eq!(*log.borrow(), ["Char('5')", "Char('4')", "Char('8')"]);
        }

        #[test]
        fn eight_opens_the_menu_and_fn_h_help_and_esc_steps_back() {
            let (mut d, log) = probe(false, false);
            let keys = |log: &Log| -> Vec<String> {
                log.borrow()
                    .iter()
                    .filter(|l| *l != "covered")
                    .cloned()
                    .collect()
            };
            d.keys("8\n5");
            assert!(keys(&log).is_empty(), "5 is the menu's now");
            d.keys("8\n5");
            assert_eq!(keys(&log), ["Soft(5)"], "8 closed the menu");
            d.keys("8\nesc\n5");
            assert_eq!(keys(&log).len(), 2, "Esc closed the menu");
            d.keys("help\n8\nesc\n5");
            assert_eq!(keys(&log).len(), 3, "help, then the menu, then back");
            assert!(!d.quit());
        }

        #[test]
        fn a_full_page_opens_the_menu_on_eight_and_a_bare_one_does_not() {
            struct Bare(Log, bool);
            impl Page for Bare {
                fn title(&self) -> &str {
                    "Bare"
                }
                fn draw(&self, _: &mut Ui) {}
                fn bars(&self) -> Bars {
                    if self.1 { Bars::full() } else { Bars::none() }
                }
                fn key(&mut self, key: Key, _: &mut Cx) {
                    self.0.borrow_mut().push(format!("{key:?}"));
                }
            }
            for full in [true, false] {
                let log = Log::default();
                let mut d = Driver::new(Bare(log.clone(), full));
                d.keys("8\nx");
                assert_eq!(log.borrow().is_empty(), full, "full {full}: {log:?}");
                assert!(d.lint().is_empty());
            }
        }

        #[test]
        fn a_raw_page_gets_esc_from_fn_esc_and_goes_back_on_esc() {
            let (mut d, log) = probe(false, true);
            d.keys("fn+esc\n5");
            let esc = gh0st_core::keys::code::ESC;
            // The page sees Fn itself too: it takes every key.
            let got: Vec<String> = log
                .borrow()
                .iter()
                .filter(|l| l.starts_with(&format!("raw {esc} ")) || l.starts_with("raw 6 "))
                .cloned()
                .collect();
            assert_eq!(
                got,
                [format!("raw {esc} fn false"), "raw 6 fn false".into()]
            );
            d.keys("esc");
            assert_eq!(d.toast(), Some("Hold to exit"));
        }

        #[test]
        fn a_page_hears_once_when_help_or_the_menu_covers_it() {
            let covered = |log: &Log| log.borrow().iter().filter(|l| *l == "covered").count();
            let (mut d, log) = probe(false, true);
            d.keys("help");
            assert_eq!(covered(&log), 1);
            d.keys("x\nhelp");
            assert_eq!(covered(&log), 1, "keys over Help, then Help closes");
            d.keys("help");
            assert_eq!(covered(&log), 2, "a second time is told again");
            let (mut d, log) = probe(false, false);
            d.keys("8");
            assert_eq!(covered(&log), 1);
        }

        #[test]
        fn no_deadline_outlives_the_hold() {
            let (mut d, _) = probe(false, false);
            d.keys("down esc").wait(200).keys("up esc").wait(3000);
            assert!(!d.quit());
            assert_eq!(d.toast(), None, "the toast has gone too");
        }

        /// Asks on Enter: text on 5, a held erase on 6.
        struct Asker {
            log: Log,
        }

        impl Page for Asker {
            fn title(&self) -> &str {
                "Asker"
            }
            fn draw(&self, _: &mut Ui) {}
            fn bars(&self) -> Bars {
                Bars::top()
                    .chip(5, Chip::new("pencil", "Name"))
                    .chip(6, Chip::new("trash-2", "Erase").danger())
            }
            fn key(&mut self, key: Key, cx: &mut Cx) {
                match key {
                    Key::Soft(5) => cx.ask(1, Dialog::password("Join").yes("join")),
                    Key::Soft(6) => {
                        cx.ask(2, Dialog::confirm("Erase?", "Gone.").yes("erase").hold())
                    }
                    _ => {}
                }
            }
            fn answer(&mut self, id: u32, a: Answer, _: &mut Cx) {
                self.log.borrow_mut().push(format!("{id} {a:?}"));
            }
        }

        fn asker() -> (Driver, Log) {
            let log = Log::default();
            (Driver::new(Asker { log: log.clone() }), log)
        }

        #[test]
        fn a_dialog_takes_the_keys_until_enter_or_esc() {
            let (mut d, log) = asker();
            d.keys("5\ntype hunter2\nbackspace\n8\nenter");
            assert_eq!(*log.borrow(), ["1 Yes(\"hunter8\")"], "digits type, 8 too");
            d.keys("5\nx\nesc");
            assert_eq!(log.borrow()[1], "1 No");
            assert!(!d.quit());
        }

        #[test]
        fn a_held_dialog_ignores_enter_and_wants_seven_held() {
            let (mut d, log) = asker();
            d.keys("6\nenter\n7").wait(100);
            assert!(log.borrow().is_empty());
            assert_eq!(d.toast(), Some("Hold to erase"));
            d.keys("down 7").wait(800);
            assert_eq!(*log.borrow(), ["2 Yes(\"\")"]);
        }

        #[test]
        fn a_dialog_waits_out_a_pause() {
            let (mut d, log) = asker();
            d.keys("5\ntype abc").pause().resume().keys("enter");
            assert_eq!(*log.borrow(), ["1 Yes(\"abc\")"]);
        }
    }

    #[test]
    fn a_reader_that_never_reads_holds_at_most_out_max() {
        let mut fds = [0; 2];
        // SAFETY: pipe fills a two-int array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: fcntl on a fresh fd we own.
        unsafe { libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK) };
        let mut w = WatchEntry {
            id: 1,
            owner: 0,
            fd: fds[1],
            out: Vec::new(),
        };
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..64 {
            queue(&mut w, &chunk);
        }
        assert!(w.out.len() <= OUT_MAX, "{} queued", w.out.len());
        // SAFETY: both ends are ours.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }
}
