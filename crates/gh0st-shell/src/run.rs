//! The shell process. It owns the panel and the keyboard while no app runs,
//! and steps aside while one does: no drawing, no timers, only ESC watched.
//! One thread, one `poll`. While idle the only timer is the clock's minute.

use crate::home::{Entry, Home, Icon, Layout, Mode, Picture};
use crate::sim::Script;
use gh0st_core::apps::{self, App, Dirs, IconRef};
use gh0st_core::backlight::Backlight;
use gh0st_core::battery::{self, LowBattery, Reading, Warning};
use gh0st_core::child::{self, Devices};
use gh0st_core::config::{self, Config};
use gh0st_core::esc::EscHold;
use gh0st_core::evdev;
use gh0st_core::keys::{self, Key, Keyboard, State, code};
use gh0st_core::rt;
use gh0st_core::status::{self, Net, Watch};
use gh0st_ui::display::{Display, Frames};
use gh0st_ui::drm;
use gh0st_ui::fb::{self, Framebuffer};
use gh0st_ui::theme::{self, Theme};
use gh0st_ui::widgets::{self, Battery, Status};
use gh0st_ui::{Canvas, Rect, snapshot};
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

const DEFAULT_DARK_SECS: i64 = 30;
const TOAST_MS: u64 = 1500;
const LOCK_HOLD_MS: u64 = 3000;
const RESCAN_DEBOUNCE_MS: u64 = 300;
const STATE_DIR: &str = ".local/state/gh0st";
/// How often to look for a leftover group member once the app's leader has
/// exited: such a process may not be our child, so its exit sends no SIGCHLD.
const GROUP_RECHECK_MS: u64 = 200;

/// `gh0st shell`, or `gh0st shell --sim DIR` on a host: keys from a script on
/// stdin (see `sim.rs`), frames saved as PNGs in DIR.
pub fn main(args: &[String]) -> i32 {
    let t0 = rt::now_ms();
    let sim = match args {
        [flag, dir] if flag == "--sim" => Some(PathBuf::from(dir)),
        [] => None,
        _ => {
            eprintln!("usage: gh0st shell [--sim DIR]");
            return 2;
        }
    };
    // Two shells would both read the keyboard and both draw. The sim is
    // allowed beside a real one: it touches no device.
    let _lock = match sim {
        Some(_) => None,
        None => match single_instance() {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("gh0st shell: {e}");
                return 1;
            }
        },
    };
    match Shell::start(t0, sim) {
        Ok(mut s) => s.run(),
        Err(e) => {
            eprintln!("gh0st shell: {e}");
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

/// Where keys come from: the keyboard, or a script in `--sim`.
enum Input {
    Device(evdev::Device),
    Script(Script),
}

impl Input {
    fn fd(&self) -> RawFd {
        match self {
            Input::Device(d) => d.fd(),
            Input::Script(s) => s.fd(),
        }
    }

    fn next_at(&self) -> Option<u64> {
        match self {
            Input::Device(_) => None,
            Input::Script(s) => s.next_at(),
        }
    }

    fn keys(&mut self, now: u64, readable: bool, out: &mut Vec<(u16, i32)>) {
        match self {
            Input::Device(d) if readable => d.read_keys(out),
            Input::Device(_) => {}
            Input::Script(s) => {
                if readable {
                    s.read(now);
                }
                s.due(now, out);
            }
        }
    }

    fn finished(&self, now: u64) -> bool {
        matches!(self, Input::Script(s) if s.finished(now))
    }

    fn path(&self) -> String {
        match self {
            Input::Device(d) => d.path.to_string_lossy().into_owned(),
            Input::Script(_) => "/dev/null".into(),
        }
    }
}

struct Shell {
    t: Theme,
    canvas: Canvas,
    display: Box<dyn Display>,
    input: Input,
    kb: Keyboard,
    signals: rt::Signals,
    /// (true: app folders, rescan; false: status, re-read sysfs).
    watches: Vec<(bool, Watch)>,
    pictures: HashMap<PathBuf, (Option<SystemTime>, Rc<Picture>)>,
    /// Screen to go back to when the backlight comes on.
    woke_to: Screen,
    /// Write every row next frame: the mapping may not match the glass.
    full_flush: bool,
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
    fn start(t0: u64, sim: Option<PathBuf>) -> Result<Shell, String> {
        child::become_subreaper();
        let signals =
            rt::Signals::install(&[libc::SIGCHLD, libc::SIGTERM, libc::SIGINT, libc::SIGHUP])
                .map_err(|e| format!("signals: {e}"))?;
        let config = Config::load(&config::default_path());
        let t = Theme::new(palette_of(&config));
        let (display, input): (Box<dyn Display>, Input) = match &sim {
            Some(dir) => {
                let frames = Frames::new(dir.clone(), t.m.width, t.m.height)
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                (Box::new(frames), Input::Script(Script::stdin()))
            }
            None => {
                let kb_path = evdev::find().ok_or("no keyboard found")?;
                let dev = evdev::Device::open(&kb_path)
                    .map_err(|e| format!("{}: {e}", kb_path.display()))?;
                (open_panel()?, Input::Device(dev))
            }
        };
        let on_device = sim.is_none();
        let layout = layout_of(&config);
        let terminal = terminal_of(&config);
        let dark_ms = dark_ms_of(&config);

        let dirs = Dirs::from_env();
        let watches = if on_device {
            watches(&dirs)
        } else {
            Vec::new()
        };

        let mut s = Shell {
            canvas: Canvas::new(t.m.width, t.m.height),
            home: Home::new(Vec::new(), layout, &t),
            t,
            display,
            input,
            kb: Keyboard::load(),
            signals,
            watches,
            pictures: HashMap::new(),
            woke_to: Screen::Home,
            full_flush: true,
            dirs,
            apps: Vec::new(),
            pins: load_pins(),
            terminal,
            backlight: on_device
                .then(|| Backlight::find(Path::new("/sys/class/backlight")))
                .flatten(),
            saved_raw: None,
            battery_dir: on_device
                .then(|| battery::find(Path::new("/sys/class/power_supply")))
                .flatten(),
            battery: None,
            low: LowBattery::default(),
            net: Net::None,
            clock: *b"--:--",
            clock_at: 0,
            screen: Screen::Home,
            dirty: true,
            toast: None,
            dark_ms,
            last_input: rt::now_ms(),
            tab_down: None,
            enter_armed: false,
            swallow_release: None,
            rescan_at: None,
            debug: std::env::var("GH0ST_LOG").is_ok_and(|v| v == "debug"),
            t0,
            first_frame: true,
            config,
        };
        let animations = s.config.get_bool("gh0st.animations").unwrap_or(true);
        s.home.set_animations(animations);
        s.boot_restore();
        s.rescan();
        s.refresh_status();
        Ok(s)
    }

    /// What stock APPLaunch did at boot, and every stock app does at start:
    /// saved brightness and volume back on.
    fn boot_restore(&mut self) {
        // As stock: the saved level, or full when none is saved. The panel
        // may be dark from the previous launcher's idle timeout.
        if let Some(bl) = &self.backlight {
            let saved = self.config.get_int("brightness").filter(|&r| r > 0);
            let _ = bl.set(saved.map(|r| r as u32).unwrap_or(bl.max));
        }
        if let Some(v) = self.config.get_int("volume") {
            let v = format!("{}%", v.clamp(0, 100));
            child::fire(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &v]);
        }
    }

    /// Re-reads config.json. Settings and stock apps write it while the shell
    /// waits, so it is read again after every app and before every write.
    fn reload_config(&mut self) {
        self.config = Config::load(&config::default_path());
        self.t = Theme::new(palette_of(&self.config));
        self.home.set_layout(layout_of(&self.config));
        self.terminal = terminal_of(&self.config);
        self.dark_ms = dark_ms_of(&self.config);
        self.home
            .set_animations(self.config.get_bool("gh0st.animations").unwrap_or(true));
        self.dirty = true;
    }

    fn rescan(&mut self) {
        let terminal_ok = desktop_first_token_runs(&self.terminal);
        self.apps = apps::scan(&self.dirs, &self.config, &self.pins);
        self.apps.retain(|a| !a.entry.terminal || terminal_ok);
        let apps = std::mem::take(&mut self.apps);
        let entries = apps.iter().map(|a| self.entry_for(a)).collect();
        self.apps = apps;
        let live: Vec<&PathBuf> = self
            .apps
            .iter()
            .filter_map(|a| match &a.icon {
                IconRef::Path(p) => Some(p),
                _ => None,
            })
            .collect();
        self.pictures.retain(|p, _| live.contains(&p));
        self.home.replace_entries(entries);
        self.dirty = true;
    }

    /// A decoded icon, reused until its file changes.
    fn picture(&mut self, path: &Path) -> Option<Rc<Picture>> {
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((m, p)) = self.pictures.get(path) {
            if *m == mtime {
                return Some(p.clone());
            }
        }
        let Ok(Icon::Picture(p)) = Icon::picture(path, &self.t) else {
            return None;
        };
        self.pictures.insert(path.to_path_buf(), (mtime, p.clone()));
        Some(p)
    }

    fn entry_for(&mut self, a: &App) -> Entry {
        let tint = a
            .file
            .bytes()
            .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
        let icon = match &a.icon {
            IconRef::Symbol(name) => match gh0st_ui::icon::ICON_NAMES
                .iter()
                .find(|n| **n == name.as_str())
            {
                Some(n) => Icon::Symbol { name: n, tint },
                None => Icon::Letter { tint },
            },
            IconRef::Path(p) => match self.picture(p) {
                Some(pic) => Icon::Picture(pic),
                None => Icon::Letter { tint },
            },
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
            if self.input.finished(rt::now_ms()) {
                return 0;
            }
            let now = rt::now_ms();
            let deadline = rt::earliest([
                (self.screen != Screen::Off).then_some(self.clock_at),
                self.kb.repeat_at(),
                self.toast.as_ref().map(|t| t.1),
                self.tab_down.map(|t| t + LOCK_HOLD_MS),
                self.rescan_at,
                self.dark_ms
                    .filter(|_| self.screen != Screen::Off)
                    .map(|d| self.last_input + d),
                (self.low.warning() == Warning::Countdown).then_some(now + 1000),
                self.input.next_at(),
                (self.screen == Screen::Home)
                    .then(|| self.home.next_frame(now))
                    .flatten(),
            ]);
            let mut fds = vec![rt::pollfd(self.input.fd()), rt::pollfd(self.signals.fd())];
            fds.extend(self.watches.iter().map(|(_, w)| rt::pollfd(w.fd())));
            if let Err(e) = rt::wait(&mut fds, deadline) {
                eprintln!("gh0st shell: poll: {e}");
                return 1;
            }
            let now = rt::now_ms();
            if self.debug {
                let ready: Vec<usize> = (0..fds.len()).filter(|&i| rt::readable(&fds[i])).collect();
                eprintln!(
                    "wake at {} fds {ready:?} deadline {:?}",
                    now - self.t0,
                    deadline.map(|d| d as i64 - (now as i64))
                );
            }
            if rt::readable(&fds[1]) {
                for sig in self.signals.drain() {
                    match sig {
                        libc::SIGCHLD => child::reap_helpers(),
                        libc::SIGHUP => self.rescan(),
                        _ => return 0,
                    }
                }
            }
            let (mut apps_changed, mut status_changed) = (false, false);
            for (i, (apps, w)) in self.watches.iter().enumerate() {
                if rt::readable(&fds[2 + i]) && w.drain() {
                    if *apps {
                        apps_changed = true;
                    } else {
                        status_changed = true;
                    }
                }
            }
            if apps_changed {
                // Apps change in bursts while the Store installs; settle first.
                self.rescan_at = Some(now + RESCAN_DEBOUNCE_MS);
            }
            if status_changed {
                self.refresh_status();
            }
            keys_buf.clear();
            self.input.keys(now, rt::readable(&fds[0]), &mut keys_buf);
            for &(c, v) in &keys_buf {
                if let Some(k) = self.kb.event(c, v, now) {
                    self.on_key(k, now);
                }
            }
            self.on_timers(now);
        }
    }

    fn on_timers(&mut self, now: u64) {
        // Any wake while something moves draws the next frame.
        if self.home.next_frame(now).is_some() {
            self.dirty = true;
        }
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
        if self.screen != Screen::Off && self.dark_ms.is_some_and(|d| now >= self.last_input + d) {
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
        self.woke_to = self.screen;
        self.screen = Screen::Off;
    }

    fn screen_on(&mut self) {
        if let (Some(bl), Some(raw)) = (&self.backlight, self.saved_raw.take()) {
            let _ = bl.set(raw);
        }
        self.screen = self.woke_to;
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
        if is_screenshot(k) && k.state == State::Press {
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
                    self.config = Config::load(&config::default_path());
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
        self.home.move_by(dx, dy, rt::now_ms());
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
        let fb_path = self.display.device();
        let kb_path = self.input.path();
        let dev = Devices {
            fb: &fb_path,
            keyboard: &kb_path,
            keymap: keys::SYM_MAP_PATH,
        };
        let pause = app.entry.terminal && app.entry.sysplause;
        self.display.suspend();
        let mut running = match child::spawn(&argv, &dev, pause) {
            Ok(r) => r,
            Err(e) => {
                self.display.resume();
                self.full_flush = true;
                self.toast(format!("Could not open {}: {e}", app.entry.name));
                return;
            }
        };
        if self.debug {
            eprintln!(
                "gh0st shell: started {} (pgid {}) at {}",
                app.entry.name,
                running.pgid,
                rt::now_ms() - self.t0
            );
        }
        let mut esc = EscHold::default();
        let mut esc_down = false;
        let mut keys_buf = Vec::new();
        let mut strip = Canvas::new(self.t.m.width, self.t.m.bar_h + 4);
        // The app's own pixels under the ESC hint, put back if ESC is let go.
        let mut under_hint: Option<(Canvas, Rect)> = None;
        loop {
            let countdown = self.low.warning() == Warning::Countdown;
            let now = rt::now_ms();
            let deadline = rt::earliest([
                esc.next_at(),
                countdown.then_some(now + 1000),
                self.input.next_at(),
                running.leader_exited().then_some(now + GROUP_RECHECK_MS),
            ]);
            let mut fds = [rt::pollfd(self.input.fd()), rt::pollfd(self.signals.fd())];
            if let Err(e) = rt::wait(&mut fds, deadline) {
                eprintln!("gh0st shell: poll while {} runs: {e}", app.entry.name);
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
            if running.leader_exited() {
                done |= running.reap();
            }
            if done {
                break;
            }
            keys_buf.clear();
            self.input.keys(now, rt::readable(&fds[0]), &mut keys_buf);
            {
                for &(c, v) in &keys_buf {
                    if c == code::ESC && v != 2 {
                        esc_down = v == 1;
                    }
                    if let Some(k) = self.kb.event(c, v, now) {
                        if is_screenshot(&k) && k.state == State::Press {
                            let mut shot = Canvas::new(self.t.m.width, self.t.m.height);
                            self.display.read_into(&mut shot);
                            let _ = save_screenshot(&shot);
                        } else if matches!(
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
            if d.hide_hint {
                if let Some((saved, r)) = under_hint.take() {
                    self.display.flush(&saved, r);
                }
            }
            if d.show_hint {
                let mut saved = Canvas::new(self.t.m.width, self.t.m.height);
                self.display.read_into(&mut saved);
                under_hint = Some((saved, strip.bounds()));
                let r = widgets::banner(
                    &mut strip,
                    &self.t,
                    "Hold ESC to close the app",
                    self.t.c.warning,
                );
                self.display.flush(&strip, r);
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
                self.display.flush(&strip, r);
                if self.low.take_shutdown(now) {
                    child::fire(&["systemctl", "poweroff", "-i"]);
                }
            }
        }
        if self.debug {
            eprintln!(
                "gh0st shell: {} closed at {}",
                app.entry.name,
                rt::now_ms() - self.t0
            );
        }
        self.display.resume();
        self.kb.reset();
        self.enter_armed = false;
        self.last_input = rt::now_ms();
        self.full_flush = true;
        // Settings or a stock app may have changed config.json, and the app
        // may have installed or removed apps (the Store does).
        self.reload_config();
        self.refresh_status();
        self.rescan();
    }

    fn draw(&mut self) {
        let t = self.t;
        let now = rt::now_ms();
        let clock = self.clock;
        let status = status_of(&clock, self.battery, self.net);
        match self.screen {
            Screen::Locked => {
                let text = std::str::from_utf8(&clock).unwrap_or("--:--");
                widgets::lock_screen(&mut self.canvas, &t, text, &status);
            }
            _ => {
                self.home.draw(&mut self.canvas, &t, &status, now);
                self.draw_low_battery();
                if let Some((text, _)) = &self.toast {
                    widgets::toast(&mut self.canvas, &t, text);
                }
            }
        }
        let rows = if self.full_flush {
            self.full_flush = false;
            self.display.flush_all(&self.canvas);
            self.canvas.height() as usize
        } else {
            self.display.flush(&self.canvas, self.canvas.bounds())
        };
        self.dirty = false;
        self.home.settle(now);
        if self.debug {
            eprintln!("gh0st shell: flushed {rows} rows");
            if self.first_frame {
                eprintln!(
                    "gh0st shell: first frame {} ms after start",
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
        gh0st_ui::icon::draw(
            c,
            "plug-zap",
            gh0st_ui::icon::IconSize::Large,
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

/// Wakes for app installs (true: rescan), and for battery, charger and link
/// changes (false: re-read status only).
#[cfg(target_os = "linux")]
fn watches(d: &Dirs) -> Vec<(bool, Watch)> {
    let apps = Watch::dirs(&[d.ours.as_path(), d.stock.as_path()]).map(|w| (true, w));
    let power = Watch::uevents().map(|w| (false, w));
    let links = Watch::links().map(|w| (false, w));
    [apps, power, links]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn watches(_: &Dirs) -> Vec<(bool, Watch)> {
    Vec::new()
}

/// Holds an exclusive lock for the life of the process. Close-on-exec, so an
/// app started from the shell never inherits it.
fn single_instance() -> Result<std::fs::File, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // SAFETY: getuid has no failure mode.
    let path = dir.join(format!("gh0st-shell-{}.lock", unsafe { libc::getuid() }));
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!("another shell is running ({})", path.display()));
    }
    Ok(f)
}

/// DRM when it opens, so a frame goes out whole and only the rectangle that
/// changed is sent. fbdev otherwise (`GH0ST_FBDEV=1` forces it).
fn open_panel() -> Result<Box<dyn Display>, String> {
    let fb_path = fb::find();
    let fbdev = fb_path.to_string_lossy().into_owned();
    if std::env::var_os("GH0ST_FBDEV").is_none() {
        if let Some(card) = drm::find() {
            match drm::Drm::open(&card, fbdev.clone()) {
                Ok(d) => return Ok(Box::new(d)),
                Err(e) => eprintln!("gh0st: DRM {}: {e}; using fbdev", card.display()),
            }
        }
    }
    let fb = Framebuffer::open(&fb_path).map_err(|e| format!("{}: {e}", fb_path.display()))?;
    Ok(Box::new(fb))
}

fn is_screenshot(k: &Key) -> bool {
    k.is(code::SYSRQ) || k.is(code::PRINT) || (k.ctrl && k.alt && k.code == 31)
}

fn palette_of(c: &Config) -> theme::Palette {
    c.get_str("gh0st.palette")
        .and_then(|n| {
            theme::PALETTES
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(n))
        })
        .copied()
        .unwrap_or(theme::MINT)
}

fn layout_of(c: &Config) -> Layout {
    c.get_str("gh0st.home_layout")
        .and_then(Layout::from_config)
        .unwrap_or_default()
}

fn terminal_of(c: &Config) -> String {
    c.get_str("gh0st.terminal")
        .unwrap_or(child::DEFAULT_TERMINAL)
        .to_string()
}

/// `dark_time` in seconds, as stock stores it. 0 means never.
fn dark_ms_of(c: &Config) -> Option<u64> {
    let secs = c.get_int("dark_time").unwrap_or(DEFAULT_DARK_SECS);
    (secs > 0).then_some(secs as u64 * 1000)
}

fn status_of(clock: &[u8; 5], battery: Option<Reading>, net: Net) -> Status<'_> {
    Status {
        title: "Gh0stdeck",
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
        .is_some_and(|p| gh0st_core::desktop::is_executable(Path::new(p)))
}

fn save_screenshot(c: &Canvas) -> std::io::Result<PathBuf> {
    let dir = home_dir().join("Pictures/Screenshots");
    std::fs::create_dir_all(&dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("gh0st-{secs}.png"));
    snapshot::write_png(c, &path, 1)?;
    Ok(path)
}
