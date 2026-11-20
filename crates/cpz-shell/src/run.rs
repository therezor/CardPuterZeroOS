//! The shell process. It owns the panel and the keyboard while no app runs,
//! and steps aside while one does: no drawing, no timers, only ESC watched.
//! One thread, one `poll`. While idle the only timer is the clock's minute.

use crate::home::{Entry, Home, Icon, Layout, Mode};
use cpz_core::apps::{self, App, Dirs, IconRef};
use cpz_core::backlight::Backlight;
use cpz_core::battery::{self, LowBattery, Reading, Warning};
use cpz_core::child::{self, Devices};
use cpz_core::config::{self, Config};
use cpz_core::esc::EscHold;
use cpz_core::evdev;
use cpz_core::keys::{self, Key, Keyboard, State, code};
use cpz_core::rt;
use cpz_core::status::{self, Net, Watch};
use cpz_ui::fb::{self, Framebuffer};
use cpz_ui::theme::{self, Theme};
use cpz_ui::widgets::{self, Battery, Status};
use cpz_ui::{Canvas, Rect, snapshot};
use std::path::{Path, PathBuf};

const DEFAULT_DARK_SECS: i64 = 30;
const TOAST_MS: u64 = 1500;
const LOCK_HOLD_MS: u64 = 3000;
const RESCAN_DEBOUNCE_MS: u64 = 300;
const STATE_DIR: &str = ".local/state/cpz";

pub fn main() -> i32 {
    let t0 = rt::now_ms();
    match Shell::start(t0) {
        Ok(mut s) => s.run(),
        Err(e) => {
            eprintln!("cpz shell: {e}");
            1
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Locked,
    /// Backlight off after `dark_time`. The next key only wakes it.
    Off,
}

struct Shell {
    t: Theme,
    canvas: Canvas,
    fb: Framebuffer,
    dev: evdev::Device,
    kb: Keyboard,
    signals: rt::Signals,
    watches: Vec<Watch>,
    config: Config,
    dirs: Dirs,
    apps: Vec<App>,
    home: Home,
    pins: Vec<String>,
    terminal: String,
    backlight: Option<Backlight>,
    saved_raw: Option<u32>,
    battery_dir: Option<PathBuf>,
    battery: Option<Reading>,
    low: LowBattery,
    net: Net,
    clock: [u8; 5],
    clock_at: u64,
    screen: Screen,
    dirty: bool,
    toast: Option<(String, u64)>,
    dark_ms: Option<u64>,
    last_input: u64,
    tab_down: Option<u64>,
    enter_armed: bool,
    swallow_release: Option<u16>,
    rescan_at: Option<u64>,
    debug: bool,
    t0: u64,
    first_frame: bool,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

fn pins_path() -> PathBuf {
    home_dir().join(STATE_DIR).join("pinned")
}

fn load_pins() -> Vec<String> {
    std::fs::read_to_string(pins_path())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

impl Shell {
    fn start(t0: u64) -> Result<Shell, String> {
        child::become_subreaper();
        let signals =
            rt::Signals::install(&[libc::SIGCHLD, libc::SIGTERM, libc::SIGINT, libc::SIGHUP])
                .map_err(|e| format!("signals: {e}"))?;
        let config = Config::load(&config::default_path());
        let palette = config
            .get_str("cpz.palette")
            .and_then(|n| {
                theme::PALETTES
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(n))
            })
            .copied()
            .unwrap_or(theme::MINT);
        let t = Theme::new(palette);
        let fb_path = fb::find();
        let fb = Framebuffer::open(&fb_path).map_err(|e| format!("{}: {e}", fb_path.display()))?;
        let kb_path = evdev::find().ok_or("no keyboard found")?;
        let dev =
            evdev::Device::open(&kb_path).map_err(|e| format!("{}: {e}", kb_path.display()))?;
        let layout = config
            .get_str("cpz.home_layout")
            .and_then(Layout::from_config)
            .unwrap_or_default();
        let terminal = config
            .get_str("cpz.terminal")
            .unwrap_or(child::DEFAULT_TERMINAL)
            .to_string();
        let dark_secs = config.get_int("dark_time").unwrap_or(DEFAULT_DARK_SECS);

        let watches = watches(&Dirs::default());

        let mut s = Shell {
            canvas: Canvas::new(t.m.width, t.m.height),
            home: Home::new(Vec::new(), layout, &t),
            t,
            fb,
            dev,
            kb: Keyboard::load(),
            signals,
            watches,
            dirs: Dirs::default(),
            apps: Vec::new(),
            pins: load_pins(),
            terminal,
            backlight: Backlight::find(Path::new("/sys/class/backlight")),
            saved_raw: None,
            battery_dir: battery::find(Path::new("/sys/class/power_supply")),
            battery: None,
            low: LowBattery::default(),
            net: Net::None,
            clock: *b"--:--",
            clock_at: 0,
            screen: Screen::Home,
            dirty: true,
            toast: None,
            dark_ms: (dark_secs > 0).then_some(dark_secs as u64 * 1000),
            last_input: rt::now_ms(),
            tab_down: None,
            enter_armed: false,
            swallow_release: None,
            rescan_at: None,
            debug: std::env::var("CPZ_LOG").is_ok_and(|v| v == "debug"),
            t0,
            first_frame: true,
            config,
        };
        s.boot_restore();
        s.rescan();
        s.refresh_status();
        Ok(s)
    }

    /// What stock APPLaunch did at boot, and every stock app does at start:
    /// saved brightness and volume back on.
    fn boot_restore(&mut self) {
        if let (Some(bl), Some(raw)) = (&self.backlight, self.config.get_int("brightness")) {
            if raw > 0 {
                let _ = bl.set(raw as u32);
            }
        }
        if let Some(v) = self.config.get_int("volume") {
            let v = format!("{}%", v.clamp(0, 100));
            child::fire(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &v]);
        }
    }

    fn rescan(&mut self) {
        let terminal_ok = desktop_first_token_runs(&self.terminal);
        self.apps = apps::scan(&self.dirs, &self.config, &self.pins);
        self.apps.retain(|a| !a.entry.terminal || terminal_ok);
        let entries = self.apps.iter().map(|a| self.entry_for(a)).collect();
        self.home.replace_entries(entries);
        self.dirty = true;
    }

    fn entry_for(&self, a: &App) -> Entry {
        let tint = a
            .file
            .bytes()
            .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
        let icon = match &a.icon {
            IconRef::Symbol(name) => match cpz_ui::icon::ICON_NAMES
                .iter()
                .find(|n| **n == name.as_str())
            {
                Some(n) => Icon::Symbol { name: n, tint },
                None => Icon::Letter { tint },
            },
            IconRef::Path(p) => Icon::picture(p, &self.t).unwrap_or(Icon::Letter { tint }),
            IconRef::None => Icon::Letter { tint },
        };
        Entry {
            id: a.file.clone(),
            name: a.entry.name.clone(),
            note: a.entry.comment.clone(),
            icon,
            pinned: self.pins.contains(&a.file),
        }
    }

    fn refresh_status(&mut self) {
        let (clock, at) = rt::clock_text();
        self.clock = clock;
        self.clock_at = at;
        self.battery = self.battery_dir.as_deref().and_then(battery::sample);
        self.net = status::network(Path::new("/sys/class/net"), Path::new("/proc/net/wireless"));
        let before = self.low.warning();
        let w = self.low.update(self.battery, rt::now_ms());
        if w != before {
            self.dirty = true;
        }
        self.dirty = true;
    }

    fn run(&mut self) -> i32 {
        let mut keys_buf = Vec::new();
        loop {
            if self.dirty && self.screen != Screen::Off {
                self.draw();
            }
            let now = rt::now_ms();
            let deadline = rt::earliest([
                (self.screen != Screen::Off).then_some(self.clock_at),
                self.kb.repeat_at(),
                self.toast.as_ref().map(|t| t.1),
                self.tab_down.map(|t| t + LOCK_HOLD_MS),
                self.rescan_at,
                self.dark_ms
                    .filter(|_| self.screen == Screen::Home)
                    .map(|d| self.last_input + d),
                (self.low.warning() == Warning::Countdown).then_some(now + 1000),
            ]);
            let mut fds = vec![rt::pollfd(self.dev.fd()), rt::pollfd(self.signals.fd())];
            fds.extend(self.watches.iter().map(|w| rt::pollfd(w.fd())));
            if let Err(e) = rt::wait(&mut fds, deadline) {
                eprintln!("cpz shell: poll: {e}");
                return 1;
            }
            let now = rt::now_ms();
            if rt::readable(&fds[1]) {
                for sig in self.signals.drain() {
                    match sig {
                        libc::SIGCHLD => child::reap_helpers(),
                        libc::SIGHUP => self.rescan(),
                        _ => return 0,
                    }
                }
            }
            let mut changed = false;
            for (i, w) in self.watches.iter().enumerate() {
                if rt::readable(&fds[2 + i]) && w.drain() {
                    changed = true;
                }
            }
            if changed {
                // Apps change in bursts while the Store installs; settle first.
                self.rescan_at = Some(now + RESCAN_DEBOUNCE_MS);
                self.refresh_status();
            }
            if rt::readable(&fds[0]) {
                keys_buf.clear();
                self.dev.read_keys(&mut keys_buf);
                for &(c, v) in &keys_buf {
                    if let Some(k) = self.kb.event(c, v, now) {
                        self.on_key(k, now);
                    }
                }
            }
            self.on_timers(now);
        }
    }

    fn on_timers(&mut self, now: u64) {
        if now >= self.clock_at {
            self.refresh_status();
        }
        if let Some(k) = self.kb.repeat_due(now) {
            self.on_key(k, now);
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.toast = None;
            self.dirty = true;
        }
        if self.tab_down.is_some_and(|t| now >= t + LOCK_HOLD_MS) {
            self.tab_down = None;
            self.screen = Screen::Locked;
            self.dirty = true;
        }
        if self.rescan_at.is_some_and(|t| now >= t) {
            self.rescan_at = None;
            self.rescan();
        }
        if self.screen == Screen::Home && self.dark_ms.is_some_and(|d| now >= self.last_input + d) {
            self.screen_off();
        }
        if self.low.warning() == Warning::Countdown {
            self.dirty = true;
            if self.low.take_shutdown(now) {
                child::fire(&["systemctl", "poweroff", "-i"]);
            }
        }
    }

    fn screen_off(&mut self) {
        if let Some(bl) = &self.backlight {
            self.saved_raw = bl.get().filter(|&r| r > 0);
            let _ = bl.set(0);
        }
        self.screen = Screen::Off;
    }

    fn screen_on(&mut self) {
        if let (Some(bl), Some(raw)) = (&self.backlight, self.saved_raw.take()) {
            let _ = bl.set(raw);
        }
        self.screen = Screen::Home;
        self.refresh_status();
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), rt::now_ms() + TOAST_MS));
        self.dirty = true;
    }

    fn on_key(&mut self, k: Key, now: u64) {
        if k.state == State::Release && self.swallow_release == Some(k.code) {
            self.swallow_release = None;
            return;
        }
        if k.state == State::Press {
            self.last_input = now;
        }
        if self.screen == Screen::Off {
            if k.state == State::Press {
                self.swallow_release = Some(k.code);
                self.screen_on();
            }
            return;
        }
        if self.global_key(&k) {
            return;
        }
        if self.low.warning() != Warning::None
            && k.state == State::Press
            && self.low.warning() != Warning::Countdown
        {
            self.low.dismiss();
            self.dirty = true;
            self.swallow_release = Some(k.code);
            return;
        }
        if self.screen == Screen::Locked {
            if k.is(code::ENTER) && k.state == State::Release {
                self.screen = Screen::Home;
                self.dirty = true;
            }
            return;
        }
        if k.is(code::TAB) {
            match k.state {
                State::Press => self.tab_down = Some(now),
                State::Release => self.tab_down = None,
                State::Repeat => {}
            }
            return;
        }
        match self.home.mode() {
            Mode::Nav => self.nav_key(k),
            Mode::Search => self.search_key(k),
        }
    }

    /// Keys that work everywhere, apps included: brightness, volume, mute,
    /// screenshot.
    fn global_key(&mut self, k: &Key) -> bool {
        if k.state == State::Release {
            return matches!(
                k.code,
                code::BRIGHTNESSUP
                    | code::BRIGHTNESSDOWN
                    | code::VOLUMEUP
                    | code::VOLUMEDOWN
                    | code::MUTE
                    | code::SYSRQ
                    | code::PRINT
            );
        }
        let screenshot =
            k.is(code::SYSRQ) || k.is(code::PRINT) || (k.ctrl && k.alt && k.code == 31);
        if screenshot && k.state == State::Press {
            let msg = match save_screenshot(&self.canvas) {
                Ok(p) => format!(
                    "Saved {}",
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("screenshot")
                ),
                Err(e) => format!("Screenshot failed: {e}"),
            };
            self.toast(msg);
            return true;
        }
        match k.code {
            code::BRIGHTNESSUP | code::BRIGHTNESSDOWN => {
                if let Some(bl) = &self.backlight {
                    let raw = bl.step(bl.get().unwrap_or(bl.max), k.code == code::BRIGHTNESSUP);
                    let _ = bl.set(raw);
                    let pct = bl.percent(raw);
                    self.config.set_int("brightness", raw as i64);
                    let _ = self.config.save();
                    self.toast(format!("Brightness {pct}%"));
                }
                true
            }
            code::VOLUMEUP | code::VOLUMEDOWN => {
                let step = if k.code == code::VOLUMEUP {
                    "10%+"
                } else {
                    "10%-"
                };
                child::fire(&[
                    "wpctl",
                    "set-volume",
                    "-l",
                    "1.0",
                    "@DEFAULT_AUDIO_SINK@",
                    step,
                ]);
                self.toast(if k.code == code::VOLUMEUP {
                    "Volume up"
                } else {
                    "Volume down"
                });
                true
            }
            code::MUTE => {
                child::fire(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
                self.toast("Mute");
                true
            }
            _ => false,
        }
    }

    fn nav_key(&mut self, k: Key) {
        if k.is(code::ENTER) {
            match k.state {
                State::Press => self.enter_armed = true,
                // Stock opens on release, so the app never sees the Enter.
                State::Release if self.enter_armed => {
                    self.enter_armed = false;
                    self.open_selected();
                }
                _ => {}
            }
            return;
        }
        if k.state == State::Release {
            return;
        }
        if let Some(n) = k.nav(true) {
            self.move_sel(n);
            return;
        }
        if k.state != State::Press {
            return;
        }
        match k.ch {
            Some('s') => {
                self.home.start_search();
                self.dirty = true;
            }
            Some('p') => self.toggle_pin(),
            Some(d @ '1'..='8') if self.home.select_on_screen(d as usize - '1' as usize) => {
                self.open_selected();
            }
            _ => {}
        }
    }

    fn search_key(&mut self, k: Key) {
        if k.is(code::ENTER) {
            match k.state {
                State::Press => self.enter_armed = true,
                State::Release if self.enter_armed => {
                    self.enter_armed = false;
                    self.open_selected();
                }
                _ => {}
            }
            return;
        }
        if k.state == State::Release {
            return;
        }
        if let Some(n) = k.nav(false) {
            self.move_sel(n);
            return;
        }
        match k.code {
            code::ESC if k.state == State::Press => self.home.cancel_search(),
            code::BACKSPACE => self.home.backspace(),
            _ => match k.ch {
                Some(c) => self.home.type_char(c),
                None => return,
            },
        }
        self.dirty = true;
    }

    fn move_sel(&mut self, n: keys::Nav) {
        let (dx, dy) = match n {
            keys::Nav::Up => (0, -1),
            keys::Nav::Down => (0, 1),
            keys::Nav::Left => (-1, 0),
            keys::Nav::Right => (1, 0),
        };
        self.home.move_by(dx, dy);
        self.dirty = true;
    }

    fn toggle_pin(&mut self) {
        let Some(id) = self.home.selected().map(|e| e.id.clone()) else {
            return;
        };
        let name = self
            .home
            .selected()
            .map(|e| e.name.clone())
            .unwrap_or_default();
        if let Some(i) = self.pins.iter().position(|p| *p == id) {
            self.pins.remove(i);
            self.toast(format!("Unpinned {name}"));
        } else {
            self.pins.push(id);
            self.toast(format!("Pinned {name}"));
        }
        let path = pins_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = config::write_atomic(&path, (self.pins.join("\n") + "\n").as_bytes());
        self.rescan();
    }

    fn open_selected(&mut self) {
        let Some(id) = self.home.selected().map(|e| e.id.clone()) else {
            return;
        };
        let Some(app) = self.apps.iter().find(|a| a.file == id).cloned() else {
            return;
        };
        if self.home.mode() == Mode::Search {
            self.home.cancel_search();
        }
        self.run_app(&app);
    }

    /// Hands the panel and keyboard to `app` until its whole process group
    /// has exited. Nothing is drawn meanwhile except the ESC hold hint and a
    /// low-battery countdown, and only ESC and the global keys are read.
    fn run_app(&mut self, app: &App) {
        let argv = child::argv(app, &self.terminal);
        let fb_path = self.fb.path.to_string_lossy().into_owned();
        let kb_path = self.dev.path.to_string_lossy().into_owned();
        let dev = Devices {
            fb: &fb_path,
            keyboard: &kb_path,
            keymap: keys::SYM_MAP_PATH,
        };
        let mut running = match child::spawn(&argv, &dev) {
            Ok(r) => r,
            Err(e) => {
                self.toast(format!("Could not open {}: {e}", app.entry.name));
                return;
            }
        };
        if self.debug {
            eprintln!(
                "cpz shell: started {} (pgid {})",
                app.entry.name, running.pgid
            );
        }
        let mut esc = EscHold::default();
        let mut esc_down = false;
        let mut keys_buf = Vec::new();
        let mut strip = Canvas::new(self.t.m.width, self.t.m.bar_h + 4);
        loop {
            let countdown = self.low.warning() == Warning::Countdown;
            let deadline = rt::earliest([esc.next_at(), countdown.then(|| rt::now_ms() + 1000)]);
            let mut fds = [rt::pollfd(self.dev.fd()), rt::pollfd(self.signals.fd())];
            if rt::wait(&mut fds, deadline).is_err() {
                break;
            }
            let now = rt::now_ms();
            let mut done = false;
            if rt::readable(&fds[1]) {
                for sig in self.signals.drain() {
                    match sig {
                        libc::SIGCHLD => done |= running.reap(),
                        libc::SIGTERM | libc::SIGINT => {
                            running.terminate();
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                }
            }
            if done {
                break;
            }
            if rt::readable(&fds[0]) {
                keys_buf.clear();
                self.dev.read_keys(&mut keys_buf);
                for &(c, v) in &keys_buf {
                    if c == code::ESC && v != 2 {
                        esc_down = v == 1;
                    }
                    if let Some(k) = self.kb.event(c, v, now) {
                        if matches!(
                            k.code,
                            code::BRIGHTNESSUP
                                | code::BRIGHTNESSDOWN
                                | code::VOLUMEUP
                                | code::VOLUMEDOWN
                                | code::MUTE
                        ) {
                            self.global_key(&k);
                            self.toast = None;
                        }
                    }
                }
            }
            let d = esc.update(now, esc_down);
            if d.show_hint {
                let r = widgets::banner(
                    &mut strip,
                    &self.t,
                    "Hold ESC to close the app",
                    self.t.c.warning,
                );
                self.fb.flush(&strip, r);
            }
            if d.terminate {
                running.terminate();
            }
            if d.kill {
                running.kill();
            }
            if countdown {
                self.low
                    .update(self.battery_dir.as_deref().and_then(battery::sample), now);
                let text = format!(
                    "Battery low: powering off in {} s",
                    self.low.seconds_left(now)
                );
                let r = widgets::banner(&mut strip, &self.t, &text, self.t.c.critical);
                self.fb.flush(&strip, r);
                if self.low.take_shutdown(now) {
                    child::fire(&["systemctl", "poweroff", "-i"]);
                }
            }
        }
        if self.debug {
            eprintln!("cpz shell: {} closed", app.entry.name);
        }
        self.kb.reset();
        self.enter_armed = false;
        self.last_input = rt::now_ms();
        self.refresh_status();
        // The app may have installed or removed apps (the Store does).
        self.rescan();
    }

    fn draw(&mut self) {
        let t = self.t;
        let clock = self.clock;
        let status = status_of(&clock, self.battery, self.net);
        match self.screen {
            Screen::Locked => {
                let text = std::str::from_utf8(&clock).unwrap_or("--:--");
                widgets::lock_screen(&mut self.canvas, &t, text, &status);
            }
            _ => {
                self.home.draw(&mut self.canvas, &t, &status);
                self.draw_low_battery();
                if let Some((text, _)) = &self.toast {
                    widgets::toast(&mut self.canvas, &t, text);
                }
            }
        }
        let rows = self.fb.flush(&self.canvas, self.canvas.bounds());
        self.dirty = false;
        if self.debug {
            eprintln!("cpz shell: flushed {rows} rows");
            if self.first_frame {
                eprintln!(
                    "cpz shell: first frame {} ms after start",
                    rt::now_ms() - self.t0
                );
            }
        }
        self.first_frame = false;
    }

    fn draw_low_battery(&mut self) {
        let t = self.t;
        let now = rt::now_ms();
        let (title, big, line, color) = match self.low.warning() {
            Warning::None => return,
            Warning::Reminder(p) => (
                "Battery low",
                format!("{p}%"),
                "Charge soon".to_string(),
                t.c.warning,
            ),
            Warning::Low(p) => (
                "Battery very low",
                format!("{p}%"),
                "Plug in the charger".to_string(),
                t.c.critical,
            ),
            Warning::Countdown => (
                "Battery critically low",
                format!("{}%", self.battery.map(|b| b.percent).unwrap_or(0)),
                format!("Powering off in {} s", self.low.seconds_left(now)),
                t.c.critical,
            ),
        };
        let c = &mut self.canvas;
        let r = Rect::new(40, 30, 240, 110);
        let inner = widgets::dialog(c, &t, r, title, color);
        cpz_ui::icon::draw(
            c,
            "plug-zap",
            cpz_ui::icon::IconSize::Large,
            inner.x + 2,
            inner.y + 6,
            color,
        );
        let x = inner.x + 50;
        t.f.big.draw(c, x, inner.y + 24, &big, color);
        t.f.body.draw(c, x, inner.y + 43, &line, t.c.fg);
        let foot = if self.low.warning() == Warning::Countdown {
            "Plug in the charger to cancel."
        } else {
            "Any key closes this."
        };
        t.f.small
            .draw(c, inner.x, inner.bottom() - 2, foot, t.c.muted);
    }
}

/// Wakes for app installs, battery and charger changes, and link changes.
#[cfg(target_os = "linux")]
fn watches(d: &Dirs) -> Vec<Watch> {
    [
        Watch::dirs(&[d.ours.as_path(), d.stock.as_path()]),
        Watch::uevents(),
        Watch::links(),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .collect()
}

#[cfg(not(target_os = "linux"))]
fn watches(_: &Dirs) -> Vec<Watch> {
    Vec::new()
}

fn status_of(clock: &[u8; 5], battery: Option<Reading>, net: Net) -> Status<'_> {
    Status {
        title: "Zero",
        clock: std::str::from_utf8(clock).unwrap_or("--:--"),
        battery: battery.map(|r| Battery {
            percent: r.percent,
            charging: r.charging,
        }),
        net: match net {
            Net::None => widgets::Net::None,
            Net::Wifi(l) => widgets::Net::Wifi(l),
            Net::Ethernet => widgets::Net::Ethernet,
        },
    }
}

fn desktop_first_token_runs(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .is_some_and(|p| cpz_core::desktop::is_executable(Path::new(p)))
}

fn save_screenshot(c: &Canvas) -> std::io::Result<PathBuf> {
    let dir = home_dir().join("Pictures/Screenshots");
    std::fs::create_dir_all(&dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("cpz-{secs}.png"));
    snapshot::write_png(c, &path, 1)?;
    Ok(path)
}
