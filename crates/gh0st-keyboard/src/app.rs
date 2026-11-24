//! The app: one SDK page holding a stack of screens (hosts, pairing, a
//! host's settings, the session), so link events reach whichever screen
//! is in front. State the screens share lives in [`Shared`].

use std::os::fd::RawFd;
use std::path::PathBuf;

use gh0st_app::prelude::*;
use gh0st_app::{Answer, Job, JobEvent, RawKey, Step, Timer, Watch};

use crate::engine::{Engine, LatchKey, Options};
use crate::hid::report::Report;
use crate::host::System;
use crate::link::usb::{self, Usb};
use crate::link::{Bluetooth, Cable, Device, Event, UsbState};
use crate::pages::hosts::Hosts;
use crate::store::{self, Host, Snippet};
use crate::words;

const HELPER: &str = "/usr/libexec/gh0st-helper";
/// The config key for the host used last: an address, or "usb".
pub const LAST: &str = "keyboard.last";
/// Shows typed characters on the typing screen, or dots.
pub const SHOW_TYPED: &str = "keyboard.show_typed";

/// Where keys go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A paired host, by Bluetooth address.
    Bt(String),
    Usb,
}

/// What every screen reads and changes.
pub struct Shared {
    pub bt: Option<Box<dyn Bluetooth>>,
    /// Why Bluetooth can't be used, when it can't.
    pub bt_error: Option<String>,
    pub usb: Box<dyn Cable>,
    pub engine: Engine,
    pub hosts: Vec<Host>,
    pub snippets: Vec<Snippet>,
    pub dir: PathBuf,
    pub target: Option<Target>,
    /// The host's keyboard lights: bit 1 is Caps Lock.
    pub leds: u8,
    /// The app turned Bluetooth on, so it turns it off on exit.
    pub radio_ours: bool,
    pub show_typed: bool,
    /// The job turning the radio on through the helper.
    radio_job: Option<Job>,
    /// Bluetooth was asked on before BlueZ said whether it is.
    want_radio: bool,
    /// Hosts whose address changed after pairing: old, new.
    moves: Vec<(String, String)>,
    /// Unblocks the radio; tests and mockups pass one that can't fail.
    pub unblock: fn() -> std::io::Result<()>,
}

fn unblock_bluetooth() -> std::io::Result<()> {
    gh0st_core::rfkill::set(gh0st_core::rfkill::Radio::Bluetooth, false)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Shared {
    pub fn new(bt: Option<Box<dyn Bluetooth>>, usb: Box<dyn Cable>, dir: PathBuf) -> Shared {
        let mut hosts = store::hosts(&dir);
        hosts.sort_by_key(|h| std::cmp::Reverse(h.used));
        Shared {
            bt,
            bt_error: None,
            usb,
            engine: Engine::new(Options {
                system: System::Windows,
                latch: LatchKey::FnAlt,
                // Nothing on the device acts on Fn+Q, W, E (hardware-facts.md).
                media_keys: true,
            }),
            snippets: store::snippets(&dir),
            hosts,
            dir,
            target: None,
            leds: 0,
            radio_ours: false,
            show_typed: true,
            radio_job: None,
            want_radio: false,
            moves: Vec::new(),
            unblock: unblock_bluetooth,
        }
    }

    pub fn host(&self, address: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.address == address)
    }

    pub fn host_mut(&mut self, address: &str) -> Option<&mut Host> {
        self.hosts.iter_mut().find(|h| h.address == address)
    }

    /// BlueZ's view of a host, if BlueZ knows it.
    pub fn device(&self, address: &str) -> Option<&Device> {
        self.bt
            .as_ref()?
            .devices()
            .iter()
            .find(|d| d.address == address)
    }

    pub fn device_at(&self, path: &str) -> Option<&Device> {
        self.bt.as_ref()?.devices().iter().find(|d| d.path == path)
    }

    pub fn save_hosts(&self) {
        let _ = store::save_hosts(&self.dir, &self.hosts);
    }

    pub fn save_snippets(&self) {
        let _ = store::save_snippets(&self.dir, &self.snippets);
    }

    /// The target's system: what Alt sends and which shortcuts apply.
    pub fn system(&self) -> System {
        match &self.target {
            Some(Target::Bt(a)) => self.host(a).map(|h| h.system).unwrap_or_default(),
            _ => System::Windows,
        }
    }

    pub fn target_name(&self) -> String {
        match &self.target {
            Some(Target::Bt(a)) => self.host(a).map(|h| h.name.clone()).unwrap_or_default(),
            Some(Target::Usb) => words::USB.into(),
            None => String::new(),
        }
    }

    /// The target is connected and takes keys.
    pub fn live(&self) -> bool {
        match &self.target {
            // Connected is enough: a bonded host may keep listening from
            // before and never say so again.
            Some(Target::Bt(a)) => self.device(a).is_some_and(|d| d.connected),
            Some(Target::Usb) => self.usb.state() == UsbState::Connected,
            None => false,
        }
    }

    /// Keys now go to `t`. Everything held is let go first.
    pub fn set_target(&mut self, t: Target, cx: &mut Cx) {
        self.release();
        if let Target::Bt(a) = &t
            && let Some(h) = self.host_mut(a)
        {
            h.used = now_secs();
        }
        self.target = Some(t.clone());
        let o = self.engine.options();
        self.engine.set_options(Options {
            system: self.system(),
            ..o
        });
        let last = match &t {
            Target::Bt(a) => a.clone(),
            Target::Usb => "usb".into(),
        };
        cx.settings().set_str(LAST, &last);
        self.save_hosts();
    }

    /// Sends reports to the target. False when it doesn't take them.
    pub fn send(&mut self, reports: &[Report]) -> bool {
        let mut ok = true;
        for r in reports {
            ok &= match &self.target {
                Some(Target::Bt(_)) => self.bt.as_mut().is_some_and(|b| b.send(r)),
                Some(Target::Usb) => self.usb.send(r),
                None => false,
            };
        }
        ok
    }

    /// Lets go of every key and button on the host.
    pub fn release(&mut self) {
        let mut out = Vec::new();
        self.engine.release_all(&mut out);
        self.send(&out);
    }

    /// BlueZ's path for the host at `address`: the device BlueZ has with
    /// that address, else the path it would have.
    pub fn path_for(&self, address: &str) -> String {
        self.device(address)
            .map(|d| d.path.clone())
            .unwrap_or_else(|| store::path_of(address))
    }

    /// The address a host has now, if it moved: `address` was the private
    /// one it paired with, and BlueZ has since learned its own.
    pub fn moved_to(&self, address: &str) -> Option<String> {
        self.moves
            .iter()
            .find(|(old, _)| old == address)
            .map(|(_, new)| new.clone())
    }

    /// A phone pairs from a private address and then gives its own: BlueZ
    /// keeps the device and changes its address. The host follows.
    fn moved(&mut self, from: &str, to: &str) {
        let Some(h) = self.hosts.iter_mut().find(|h| h.address == from) else {
            return;
        };
        h.address = to.to_string();
        if self.target == Some(Target::Bt(from.to_string())) {
            self.target = Some(Target::Bt(to.to_string()));
        }
        self.moves.push((from.to_string(), to.to_string()));
        self.save_hosts();
        self.sync_hosts();
    }

    /// Tells the link which devices are the app's hosts.
    pub fn sync_hosts(&mut self) {
        let paths = self
            .hosts
            .iter()
            .map(|h| self.path_for(&h.address))
            .collect();
        if let Some(bt) = &mut self.bt {
            bt.set_hosts(paths);
        }
    }

    /// Bluetooth on: the radio unblocked, then the adapter powered. Asked
    /// before BlueZ has answered, it waits for the answer.
    pub fn radio_on(&mut self, cx: &mut Cx) {
        let Some(bt) = &mut self.bt else { return };
        if !bt.known() {
            self.want_radio = true;
            return;
        }
        self.want_radio = false;
        if bt.powered() || self.radio_job.is_some() {
            return;
        }
        self.radio_ours = true;
        if (self.unblock)().is_ok() {
            bt.power(true);
            return;
        }
        let argv = ["pkexec", HELPER, "rfkill", "bluetooth", "on"];
        self.radio_job = Some(cx.spawn(&argv));
        cx.busy(words::RADIO_ON);
    }

    fn job(&mut self, job: Job, e: JobEvent, cx: &mut Cx) {
        if self.radio_job != Some(job) {
            return;
        }
        if let JobEvent::Done(code) = e {
            self.radio_job = None;
            cx.idle();
            match (code, &mut self.bt) {
                (Some(0), Some(bt)) => bt.power(true),
                _ => cx.toast_tone(Tone::Critical, words::RADIO_FAILED),
            }
        }
    }

    /// Tells Bluetooth hosts the battery level, which they show beside the
    /// keyboard. Sent when one connects and when the app comes back, not on
    /// a timer.
    pub fn battery(&mut self) {
        let Some(b) = gh0st_app::system::battery() else {
            return;
        };
        if let Some(bt) = &mut self.bt {
            bt.battery(b.percent);
        }
    }

    /// Bookkeeping every screen relies on, before the screen in front hears
    /// the event.
    fn event(&mut self, e: &Event, cx: &mut Cx) {
        match e {
            Event::Ready => self.sync_hosts(),
            Event::Moved(from, to) => self.moved(from, to),
            Event::Power(false) if self.want_radio => self.radio_on(cx),
            Event::Power(true) => self.want_radio = false,
            Event::Leds(v) => self.leds = *v,
            Event::Connected(path) => {
                self.battery();
                let Some(addr) = self.device_at(path).map(|d| d.address.clone()) else {
                    return;
                };
                if self.host(&addr).is_none() {
                    return;
                }
                // One host at a time: another of ours that comes back while
                // the user picked this one is let go.
                match &self.target {
                    Some(Target::Bt(t)) if *t != addr => {
                        if let Some(bt) = &mut self.bt {
                            bt.disconnect(path);
                        }
                    }
                    _ => {}
                }
            }
            Event::Disconnected(path) => {
                let addr = self
                    .device_at(path)
                    .map(|d| d.address.clone())
                    .or_else(|| address_of(path));
                let ours = matches!(&self.target, Some(Target::Bt(a)) if addr.as_ref() == Some(a));
                if ours {
                    let mut out = Vec::new();
                    self.engine.release_all(&mut out);
                    self.leds = 0;
                }
            }
            Event::Failed(why) => {
                self.bt_error = Some(why.clone());
            }
            _ => {}
        }
    }
}

/// What a screen asks of the stack after a call.
pub enum Nav {
    Stay,
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
}

/// One screen of the app. The defaults are the SDK's.
pub trait Screen {
    fn title(&self) -> &str;
    fn icon(&self) -> &str {
        "keyboard"
    }
    fn context(&self) -> Option<&str> {
        Some(words::APP)
    }
    fn about(&self) -> &str {
        ""
    }
    fn bars(&self, _: &Shared) -> Bars {
        Bars::top()
    }
    fn draw(&self, s: &Shared, ui: &mut Ui);
    fn key(&mut self, _: &mut Shared, _: Key, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn raw_keys(&self) -> bool {
        false
    }
    fn raw(&mut self, _: &mut Shared, _: &RawKey, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn typing(&self) -> bool {
        false
    }
    fn commands(&self, _: &Shared, _: &mut Commands) {}
    fn command(&mut self, _: &mut Shared, _: u32, _: Step, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn answer(&mut self, _: &mut Shared, _: u32, _: Answer, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn timer(&mut self, _: &mut Shared, _: Timer, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn event(&mut self, _: &mut Shared, _: &Event, _: &mut Cx) -> Nav {
        Nav::Stay
    }
    fn start(&mut self, _: &mut Shared, _: &mut Cx) {}
    fn stop(&mut self, _: &mut Shared, _: &mut Cx) {}
    /// Help, the menu or a dialog came over the screen.
    fn covered(&mut self, s: &mut Shared, _: &mut Cx) {
        s.release();
    }
    /// Esc: true leaves the screen.
    fn back(&mut self, _: &mut Shared, _: &mut Cx) -> bool {
        true
    }
    fn frame_ms(&self) -> u64 {
        0
    }
}

/// A screen to open as the app starts: for mockups and tests.
pub enum Open {
    Hosts,
    Pair,
    Profile(String),
    Session(usize),
}

pub struct App {
    pub s: Shared,
    stack: Vec<Box<dyn Screen>>,
    watches: Vec<(Watch, RawFd)>,
    /// Bluetooth from the system bus, made again after a stop.
    real: bool,
    opening: Option<(Option<Target>, Open)>,
    /// Runs only while USB reports wait for the host.
    usb_tick: Option<Timer>,
}

impl App {
    /// The app on the device: BlueZ on the system bus, and the USB cable.
    pub fn new() -> App {
        App {
            s: Shared::new(None, Box::new(Usb::new()), PathBuf::new()),
            stack: vec![Box::new(Hosts::new())],
            watches: Vec::new(),
            real: true,
            opening: None,
            usb_tick: None,
        }
    }

    /// Over the links and folder given: for tests and mockups.
    pub fn with(bt: Box<dyn Bluetooth>, usb: Box<dyn Cable>, dir: PathBuf) -> App {
        let mut s = Shared::new(Some(bt), usb, dir);
        s.unblock = || Ok(());
        App {
            s,
            stack: vec![Box::new(Hosts::new())],
            watches: Vec::new(),
            real: false,
            opening: None,
            usb_tick: None,
        }
    }

    /// Starts on `target`, with `open` over the hosts.
    pub fn opening(mut self, target: Option<Target>, open: Open) -> App {
        self.opening = Some((target, open));
        self
    }

    fn top(&self) -> &dyn Screen {
        self.stack.last().map(|b| b.as_ref()).unwrap_or(&NOTHING)
    }

    fn go(&mut self, nav: Nav, cx: &mut Cx) {
        if !matches!(nav, Nav::Stay) {
            cx.redraw();
        }
        match nav {
            Nav::Stay => {}
            Nav::Push(mut next) => {
                next.start(&mut self.s, cx);
                self.stack.push(next);
            }
            Nav::Pop => self.pop(cx),
            Nav::Replace(mut next) => {
                self.pop(cx);
                next.start(&mut self.s, cx);
                self.stack.push(next);
            }
        }
    }

    fn pop(&mut self, cx: &mut Cx) {
        if self.stack.len() < 2 {
            return;
        }
        if let Some(mut top) = self.stack.pop() {
            top.stop(&mut self.s, cx);
        }
    }

    fn with_top(
        &mut self,
        cx: &mut Cx,
        f: impl FnOnce(&mut dyn Screen, &mut Shared, &mut Cx) -> Nav,
    ) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        let nav = f(top.as_mut(), &mut self.s, cx);
        self.go(nav, cx);
        // A handler may have turned the USB keyboard on or off.
        self.sync_watches(cx);
    }

    /// Joins every link fd to the loop, and drops the ones gone. Also
    /// hands a new helper to the SDK and runs the USB queue's timer only
    /// while reports wait.
    fn sync_watches(&mut self, cx: &mut Cx) {
        if let Some(pid) = self.s.usb.spawned() {
            cx.adopt(pid);
        }
        match (self.s.usb.waiting(), self.usb_tick) {
            (true, None) => self.usb_tick = Some(cx.every(usb::TICK_MS)),
            (false, Some(t)) => {
                cx.cancel(t);
                self.usb_tick = None;
            }
            _ => {}
        }
        let mut want: Vec<RawFd> = self
            .s
            .bt
            .as_ref()
            .and_then(|b| b.fd())
            .into_iter()
            .collect();
        want.extend(self.s.usb.fds());
        self.watches.retain(|&(w, fd)| {
            let keep = want.contains(&fd);
            if !keep {
                cx.unwatch(w);
            }
            keep
        });
        for fd in want {
            if !self.watches.iter().any(|&(_, f)| f == fd) {
                let w = cx.watch(fd);
                self.watches.push((w, fd));
            }
        }
    }

    fn connect_bt(&mut self) {
        if !self.real || self.s.bt.is_some() {
            return;
        }
        match crate::link::ble::Ble::system() {
            Ok(b) => {
                self.s.bt = Some(Box::new(b));
                self.s.bt_error = None;
            }
            Err(e) => self.s.bt_error = Some(e),
        }
    }

    fn handle(&mut self, events: Vec<Event>, cx: &mut Cx) {
        for e in events {
            self.s.event(&e, cx);
            if let Event::Failed(_) = e {
                if let Some(mut bt) = self.s.bt.take() {
                    bt.stop();
                }
            }
            self.with_top(cx, |t, s, cx| t.event(s, &e, cx));
        }
        self.sync_watches(cx);
        cx.redraw();
    }
}

impl Default for App {
    fn default() -> Self {
        App::new()
    }
}

struct Nothing;
const NOTHING: Nothing = Nothing;

impl Screen for Nothing {
    fn title(&self) -> &str {
        words::APP
    }
    fn draw(&self, _: &Shared, _: &mut Ui) {}
}

impl Page for App {
    fn title(&self) -> &str {
        self.top().title()
    }

    fn icon(&self) -> &str {
        self.top().icon()
    }

    fn context(&self) -> Option<&str> {
        if self.stack.len() < 2 {
            return None;
        }
        self.top().context()
    }

    fn about(&self) -> &str {
        self.top().about()
    }

    fn status(&self) -> Option<(&str, Tone)> {
        (self.s.leds & 2 != 0 && self.s.live()).then_some((words::CAPS, Tone::Info))
    }

    fn bars(&self) -> Bars {
        self.top().bars(&self.s)
    }

    fn draw(&self, ui: &mut Ui) {
        self.top().draw(&self.s, ui);
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.real {
            self.s.dir = cx.data_dir();
            let dir = self.s.dir.clone();
            let mut hosts = store::hosts(&dir);
            hosts.sort_by_key(|h| std::cmp::Reverse(h.used));
            self.s.hosts = hosts;
            self.s.snippets = store::snippets(&dir);
            self.connect_bt();
        }
        self.s.sync_hosts();
        self.s.show_typed = cx.settings().config().get_bool(SHOW_TYPED).unwrap_or(true);
        self.sync_watches(cx);
        let last = cx.settings().config().get_str(LAST).map(String::from);
        if let Some(root) = self.stack.first_mut() {
            root.start(&mut self.s, cx);
        }
        let mut events = Vec::new();
        if let Some(bt) = &mut self.s.bt {
            bt.ready(&mut events);
        }
        if let Some((target, open)) = self.opening.take() {
            if let Some(t) = target {
                self.s.set_target(t, cx);
            }
            let next: Box<dyn Screen> = match open {
                Open::Hosts => {
                    self.handle(events, cx);
                    return;
                }
                Open::Pair => Box::new(crate::pages::pair::Pair::new()),
                Open::Profile(a) => {
                    Box::new(crate::pages::profile::Profile::new(&self.s, &a, true))
                }
                Open::Session(mode) => {
                    Box::new(crate::pages::session::Session::in_mode(&self.s, mode))
                }
            };
            self.go(Nav::Push(next), cx);
            self.handle(events, cx);
            return;
        }
        self.handle(events, cx);
        // Back to the host used last: it reconnects on its own once the
        // keyboard advertises.
        match last.as_deref() {
            Some("usb") => {}
            Some(a) if self.s.host(a).is_some() => {
                let t = Target::Bt(a.to_string());
                self.s.set_target(t, cx);
                self.s.radio_on(cx);
                if let Some(bt) = &mut self.s.bt {
                    bt.reconnect(true);
                }
            }
            _ => {}
        }
    }

    fn stop(&mut self, cx: &mut Cx) {
        self.s.release();
        while self.stack.len() > 1 {
            self.pop(cx);
        }
        if let Some(root) = self.stack.first_mut() {
            root.stop(&mut self.s, cx);
        }
        self.s.usb.stop();
        if let Some(bt) = &mut self.s.bt {
            bt.stop();
            if self.s.radio_ours {
                bt.power(false);
            }
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        self.with_top(cx, |t, s, cx| t.key(s, key, cx));
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        let Some(top) = self.stack.last_mut() else {
            return true;
        };
        if !top.back(&mut self.s, cx) {
            cx.redraw();
            return false;
        }
        if self.stack.len() > 1 {
            self.pop(cx);
            cx.redraw();
            return false;
        }
        true
    }

    fn typing(&self) -> bool {
        self.top().typing()
    }

    fn raw_keys(&self) -> bool {
        self.top().raw_keys()
    }

    fn raw(&mut self, key: &RawKey, cx: &mut Cx) {
        self.with_top(cx, |t, s, cx| t.raw(s, key, cx));
    }

    fn commands(&self, c: &mut Commands) {
        self.top().commands(&self.s, c);
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        self.with_top(cx, |t, s, cx| t.command(s, id, step, cx));
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        self.with_top(cx, |t, s, cx| t.answer(s, id, answer, cx));
    }

    fn timer(&mut self, timer: Timer, cx: &mut Cx) {
        if self.usb_tick == Some(timer) {
            self.s.usb.tick();
            self.sync_watches(cx);
            return;
        }
        self.with_top(cx, |t, s, cx| t.timer(s, timer, cx));
    }

    fn exited(&mut self, pid: i32, code: Option<i32>, cx: &mut Cx) {
        let mut events = Vec::new();
        self.s.usb.exited(pid, code, &mut events);
        self.handle(events, cx);
    }

    fn job(&mut self, job: Job, e: JobEvent, cx: &mut Cx) {
        self.s.job(job, e, cx);
        cx.redraw();
    }

    fn ready(&mut self, watch: Watch, cx: &mut Cx) {
        let Some(&(_, fd)) = self.watches.iter().find(|(w, _)| *w == watch) else {
            return;
        };
        let mut events = Vec::new();
        if self.s.bt.as_ref().and_then(|b| b.fd()) == Some(fd) {
            if let Some(bt) = &mut self.s.bt {
                bt.ready(&mut events);
            }
        } else {
            self.s.usb.ready(fd, &mut events);
        }
        self.handle(events, cx);
    }

    fn covered(&mut self, cx: &mut Cx) {
        let Some(top) = self.stack.last_mut() else {
            return;
        };
        top.covered(&mut self.s, cx);
    }

    fn front(&mut self, front: bool, cx: &mut Cx) {
        if !front {
            self.s.release();
            let behind =
                gh0st_core::apps::runs_behind(cx.settings().config(), "keyboard.desktop", true);
            let handheld = matches!(&self.s.target, Some(Target::Bt(a))
                if self.s.host(a).is_some_and(|h| !h.stay));
            if !behind {
                // About to be stopped: BlueZ must not call a stopped app.
                if let Some(mut bt) = self.s.bt.take() {
                    bt.stop();
                }
                self.s.usb.stop();
            } else if handheld && let Some(Target::Bt(a)) = &self.s.target {
                // A phone gets its own keyboard back.
                let path = self.s.path_for(a);
                if let Some(bt) = &mut self.s.bt {
                    bt.reconnect(false);
                    bt.disconnect(&path);
                }
            }
            self.sync_watches(cx);
            return;
        }
        self.connect_bt();
        self.s.battery();
        self.sync_watches(cx);
        if let Some(Target::Bt(_)) = &self.s.target
            && let Some(bt) = &mut self.s.bt
            && bt.connected().is_none()
        {
            bt.reconnect(true);
        }
        cx.redraw();
    }

    fn frame_ms(&self) -> u64 {
        self.top().frame_ms()
    }
}

/// The host a BlueZ device path belongs to, by its address.
pub fn address_of(path: &str) -> Option<String> {
    let dev = path.rsplit('/').next()?.strip_prefix("dev_")?;
    Some(dev.replace('_', ":"))
}
