//! The shell process. It owns the panel and the keyboard while no app is in
//! front, and steps aside while one is: it draws nothing then but the ESC
//! hint, toasts and the battery countdown, laid over the app. Apps sent to
//! the background are paused. One thread, one `poll`. While idle with the
//! screen on, the clock ticks each minute and the battery is read every 2 s.
//! Dark, neither runs.

use crate::home::{self, Entry, Home, Icon, Layout, Mode, Picture, Size};
use crate::lock::{self, Lock};
use crate::overlay::Overlay;
use crate::switcher::{Card, Switcher};
use crate::tasks::{self, Task, Tasks};
use gh0st_core::apps::{self, App, Dirs, IconRef};
use gh0st_core::backlight::Backlight;
use gh0st_core::battery::{self, LowBattery, Reading, Warning};
use gh0st_core::child::{self, Devices, Stopped};
use gh0st_core::config::{self, Config};
use gh0st_core::esc::EscHold;
use gh0st_core::evdev;
use gh0st_core::job::{self, Job};
use gh0st_core::keys::{self, Key, Keyboard, State, code};
use gh0st_core::link::Link;
use gh0st_core::password::{self, Account, Tries};
use gh0st_core::rt;
use gh0st_core::script::Script;
use gh0st_core::status::{self, Net, Watch};
use gh0st_core::usage::{self, Sampler};
use gh0st_ui::display::{Display, Frames};
use gh0st_ui::drm;
use gh0st_ui::fb::{self, Framebuffer};
use gh0st_ui::theme::{self, Theme, Tone};
use gh0st_ui::widgets::{self, Battery, Status, Visual};
use gh0st_ui::{Canvas, Rect, snapshot};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Instant, SystemTime};

/// How long a paused app may take to stop before its frame is read anyway.
const STOP_WAIT_MS: u64 = 100;
/// How often the shell looks for rows a background app drew over it.
const GUARD_MS: u64 = 500;
const DEFAULT_DARK_SECS: i64 = 30;
/// `gh0st.lock.after`: dark this long, the lock asks for the password.
const DEFAULT_LOCK_AFTER_SECS: i64 = 300;
/// The backlight dims this long before it goes off.
const DIM_MS: u64 = 5000;
const DIM_PERCENT: u8 = 10;
/// The lock screen at rest goes dark sooner, so a pocket keeps it lit
/// only briefly.
const LOCK_DARK_MS: u64 = 10_000;
const TOAST_MS: u64 = 1500;
const LOCK_HOLD_MS: u64 = 3000;
const RESCAN_DEBOUNCE_MS: u64 = 300;
const VOLUME_STEP: i32 = 10;
/// The switcher's figures: the first CPU reading after this, then one every
/// `STATS_MS`. Read only while the switcher shows. 250 ms is 100 ticks over
/// four cores at `CLK_TCK` 100: whole percents, as shown.
const STATS_FIRST_MS: u64 = 250;
const STATS_MS: u64 = 1000;

/// `gh0st shell`, or `gh0st shell --sim DIR` on a host: keys from a script on
/// stdin (see `gh0st_core::script`), frames saved as PNGs in DIR.
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
    Switcher,
    Locked,
    /// Backlight off after `dark_time`. The next key wakes the lock screen.
    Off,
}

/// Where Esc on the switcher goes back to.
#[derive(Clone, PartialEq, Eq)]
enum Back {
    Home,
    App(String),
}

/// A handover that waits until every key is up, so an app never sees half
/// a key press: pausing the app in front, or bringing one back.
#[derive(Clone, PartialEq, Eq)]
enum Pending {
    Pause,
    Resume(String),
}

/// A helper whose answer the shell is waiting for.
#[derive(Clone, PartialEq, Eq)]
enum JobKind {
    Volume,
    /// `passwd -S`: does the account have a password?
    Passwd,
    /// `unix_chkpwd`: was that the password?
    Chkpwd,
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

    /// False when the grab failed: an app holds the keyboard.
    fn grab(&mut self, on: bool) -> bool {
        match self {
            Input::Device(d) => d.grab(on),
            Input::Script(_) => true,
        }
    }

    fn flush_readers(&self) {
        if let Input::Device(d) = self {
            d.flush_readers();
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
    lock: Lock,
    /// This lock asks for the password however short the dark: locked by
    /// hand or at start.
    lock_secure: bool,
    /// When the screen first went dark since the last unlock.
    dark_since: Option<u64>,
    /// Where unlocking goes.
    unlock_to: Back,
    /// `gh0st.lock.password`.
    lock_password: bool,
    /// `gh0st.lock.after`, or None for only by hand.
    lock_after_ms: Option<u64>,
    /// What `passwd -S` said; None before it answered or when it failed.
    account: Option<Account>,
    probing: bool,
    /// `unix_chkpwd` could not run: the lock can't check, so it doesn't ask.
    checker_missing: bool,
    tries: Tries,
    /// Where the try count is kept, so a restarted shell still waits.
    tries_path: Option<PathBuf>,
    user: String,
    /// The backlight before it dimmed.
    dimmed: Option<u32>,
    /// Write every row next frame: the mapping may not match the glass.
    full_flush: bool,
    config: Config,
    dirs: Dirs,
    apps: Vec<App>,
    home: Home,
    switcher: Switcher,
    tasks: Tasks,
    back: Back,
    pending: Option<Pending>,
    /// Raw key codes down right now, apps' keys included.
    held: Vec<u16>,
    esc: EscHold,
    esc_down: bool,
    /// Over an app: the ESC hint or the battery countdown.
    banner: Option<Overlay>,
    /// Over an app: a toast.
    popup: Option<Overlay>,
    /// The volume or brightness level, shown in the top bar's place while
    /// `toast` lasts: icon, name, percent.
    level: Option<(&'static str, &'static str, u8)>,
    /// A panel-sized canvas kept for reuse.
    spare: Option<Canvas>,
    jobs: Vec<(JobKind, Job)>,
    volume: Option<u8>,
    muted: Option<bool>,
    pins: Vec<String>,
    terminal: String,
    backlight: Option<Backlight>,
    saved_raw: Option<u32>,
    battery_dir: Option<PathBuf>,
    battery: Option<Reading>,
    low: LowBattery,
    net: Net,
    bluetooth: bool,
    clock: String,
    clock_at: u64,
    /// The next battery read while the screen is on.
    battery_at: u64,
    screen: Screen,
    dirty: bool,
    toast: Option<(String, u64)>,
    dark_ms: Option<u64>,
    last_input: u64,
    tab_down: Option<u64>,
    enter_armed: bool,
    esc_armed: bool,
    /// Home's menu, open on 8: the selected row.
    home_menu: Option<usize>,
    /// The switcher asks before closing every app.
    confirm_all: bool,
    home_armed: bool,
    swallow_release: Option<u16>,
    rescan_at: Option<u64>,
    sampler: Sampler,
    /// Next read of the switcher's figures.
    stats_at: Option<u64>,
    debug: bool,
    t0: u64,
    /// When the loop last woke, for the debug log.
    wake_at: Instant,
    /// The last frame waited for the panel, so the next one draws at once
    /// instead of on the FRAME_MS timer.
    paced: bool,
    first_frame: bool,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

/// `gh0st.clock`: `12h`, or a 24-hour clock.
fn clock_24h(c: &Config) -> bool {
    c.get_str("gh0st.clock") != Some("12h")
}

impl Shell {
    fn start(t0: u64, sim: Option<PathBuf>) -> Result<Shell, String> {
        child::become_subreaper();
        let signals = rt::Signals::install(&[
            libc::SIGCHLD,
            libc::SIGTERM,
            libc::SIGINT,
            libc::SIGHUP,
            libc::SIGUSR1,
        ])
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
            switcher: Switcher::new(&t),
            tasks: Tasks::default(),
            back: Back::Home,
            pending: None,
            held: Vec::new(),
            esc: EscHold::default(),
            esc_down: false,
            banner: None,
            popup: None,
            level: None,
            spare: None,
            jobs: Vec::new(),
            volume: config.get_int("volume").map(|v| v.clamp(0, 100) as u8),
            muted: None,
            t,
            display,
            input,
            kb: Keyboard::load(),
            signals,
            watches,
            pictures: HashMap::new(),
            lock: Lock::default(),
            lock_secure: false,
            dark_since: None,
            unlock_to: Back::Home,
            lock_password: lock_password_of(&config),
            lock_after_ms: lock_after_of(&config),
            account: None,
            probing: false,
            checker_missing: false,
            tries: Tries::default(),
            tries_path: None,
            // SAFETY: getuid has no failure mode.
            user: password::user_name(unsafe { libc::getuid() }).unwrap_or_default(),
            dimmed: None,
            full_flush: true,
            dirs,
            apps: Vec::new(),
            pins: apps::load_pins(),
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
            bluetooth: false,
            clock: "--:--".into(),
            clock_at: 0,
            battery_at: 0,
            screen: Screen::Home,
            dirty: true,
            toast: None,
            dark_ms,
            last_input: rt::now_ms(),
            tab_down: None,
            enter_armed: false,
            esc_armed: false,
            home_menu: None,
            confirm_all: false,
            home_armed: false,
            swallow_release: None,
            rescan_at: None,
            sampler: Sampler::default(),
            stats_at: None,
            debug: std::env::var("GH0ST_LOG").is_ok_and(|v| v == "debug"),
            t0,
            wake_at: Instant::now(),
            paced: false,
            first_frame: true,
            config,
        };
        s.set_animations();
        s.boot_restore();
        if on_device {
            s.start_job(
                JobKind::Volume,
                &["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"],
            );
        }
        s.rescan();
        s.refresh_status();
        if on_device {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = dir.join("gh0st-lock");
            s.tries = Tries::load(&path, password::boottime());
            s.tries_path = Some(path);
        }
        // A restart or a power cycle must not skip the lock.
        if s.lock_password {
            s.lock_secure = true;
            s.enter_locked(rt::now_ms());
        }
        Ok(s)
    }

    fn set_animations(&mut self) {
        let on = self.config.get_bool("gh0st.animations").unwrap_or(true);
        self.home.set_animations(on);
        self.switcher.set_animations(on);
    }

    fn start_job(&mut self, kind: JobKind, argv: &[&str]) -> bool {
        self.start_job_with(kind, argv, None)
    }

    /// As `start_job`, with `input` written to the helper's stdin. False
    /// when it could not start.
    fn start_job_with(&mut self, kind: JobKind, argv: &[&str], input: Option<&[u8]>) -> bool {
        match job::spawn(argv, input) {
            Ok(j) => {
                self.jobs.push((kind, j));
                true
            }
            Err(e) => {
                if self.debug {
                    eprintln!("gh0st shell: {}: {e}", argv[0]);
                }
                false
            }
        }
    }

    fn job_done(&mut self, kind: JobKind, j: Job) {
        match kind {
            JobKind::Volume => {
                if let Some((v, muted)) = parse_volume(&j.output()) {
                    self.volume.get_or_insert(v.min(100));
                    self.muted = Some(muted);
                }
            }
            JobKind::Passwd => {
                self.probing = false;
                self.account = j
                    .ok()
                    .then(|| password::parse_status(&j.output()))
                    .flatten();
                if self.debug {
                    eprintln!("gh0st shell: account {:?}", self.account);
                }
                self.dirty = true;
            }
            JobKind::Chkpwd => {
                let ok = j.ok();
                if self.debug {
                    eprintln!(
                        "gh0st shell: password {}",
                        if ok { "right" } else { "wrong" }
                    );
                }
                if ok {
                    self.tries = Tries::default();
                    self.save_tries();
                }
                // Dark meanwhile rested the field: the answer is stale.
                if self.screen == Screen::Locked && self.lock.checking {
                    self.lock.checked(ok, rt::now_ms());
                    if ok {
                        self.unlock();
                    }
                }
                self.dirty = true;
            }
        }
    }

    fn save_tries(&self) {
        if let Some(p) = &self.tries_path {
            let _ = self.tries.save(p);
        }
    }

    /// Asks `passwd -S` again: Settings may have changed the password.
    fn probe_account(&mut self) {
        if self.probing {
            return;
        }
        let prog = std::env::var("GH0ST_PASSWD").unwrap_or_else(|_| "passwd".into());
        self.probing = self.start_job(JobKind::Passwd, &[&prog, "-S"]);
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
        self.lock_password = lock_password_of(&self.config);
        self.lock_after_ms = lock_after_of(&self.config);
        self.set_animations();
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
        let custom = self
            .config
            .get_str(&apps::app_key(&a.file, "icon"))
            .and_then(symbol);
        let icon = icon_for(&a.icon, a.ours, custom, tint, |p| self.picture(p));
        Entry {
            id: a.file.clone(),
            name: a.entry.name.clone(),
            note: a.entry.comment.clone(),
            icon,
            pinned: self.pins.contains(&a.file),
        }
    }

    fn refresh_status(&mut self) {
        let (clock, at) = rt::clock_text(clock_24h(&self.config));
        self.clock = clock;
        self.clock_at = at;
        self.battery = self.battery_dir.as_deref().and_then(battery::sample);
        self.net = status::network(Path::new("/sys/class/net"), Path::new("/proc/net/wireless"));
        self.bluetooth = status::bluetooth();
        let before = self.low.warning();
        let w = self.low.update(self.battery, rt::now_ms());
        if w != before {
            self.dirty = true;
        }
        self.dirty = true;
    }

    /// The battery alone, on `POLL_MS`. A frame only if it changed.
    fn poll_battery(&mut self, now: u64) {
        self.battery_at = now + battery::POLL_MS;
        let r = self.battery_dir.as_deref().and_then(battery::sample);
        if r != self.battery {
            self.battery = r;
            self.low.update(r, now);
            self.dirty = true;
        }
    }

    fn app_front(&self) -> bool {
        self.tasks.front().is_some()
    }

    fn run(&mut self) -> i32 {
        let mut keys_buf = Vec::new();
        loop {
            let app = self.app_front();
            if !app && self.dirty && self.screen != Screen::Off {
                self.draw();
            }
            let now = rt::now_ms();
            if self.input.finished(now) {
                self.tasks.close_all(now);
                return 0;
            }
            let shell = !app && self.screen != Screen::Off;
            let deadline = rt::earliest([
                shell.then_some(self.clock_at),
                (shell && self.battery_dir.is_some()).then_some(self.battery_at),
                // Over an app no shell key repeats, so no repeat wakeups.
                (!app).then(|| self.kb.repeat_at()).flatten(),
                self.toast.as_ref().map(|t| t.1),
                (!app)
                    .then_some(self.tab_down.map(|t| t + LOCK_HOLD_MS))
                    .flatten(),
                self.rescan_at,
                (shell && self.screen == Screen::Switcher && self.next_frame(now).is_none())
                    .then_some(self.stats_at)
                    .flatten(),
                self.dark_at(),
                self.dim_at(),
                (shell && self.screen == Screen::Locked)
                    .then(|| self.lock.next_at())
                    .flatten(),
                (shell && self.lock_waiting()).then_some(now + 1000),
                (self.low.warning() == Warning::Countdown).then_some(now + 1000),
                self.input.next_at(),
                shell
                    .then(|| match self.paced {
                        true => Some(now),
                        false => self.next_frame(now),
                    })
                    .flatten(),
                app.then(|| self.esc.next_at()).flatten(),
                (shell
                    && self
                        .tasks
                        .iter()
                        .any(|t| t.state == tasks::State::Background && !t.quiet))
                .then_some(now + GUARD_MS),
                (app && (self.banner.is_some() || self.popup.is_some()))
                    .then_some(now + crate::overlay::REFRESH_MS),
                self.tasks.next_at(now),
            ]);
            let mut fds = vec![rt::pollfd(self.input.fd()), rt::pollfd(self.signals.fd())];
            fds.extend(self.watches.iter().map(|(_, w)| rt::pollfd(w.fd())));
            let jobs_at = fds.len();
            fds.extend(self.jobs.iter().map(|(_, j)| rt::pollfd(j.fd())));
            let links: Vec<(i32, i32)> = self
                .tasks
                .iter()
                .filter_map(|t| t.link.as_ref().map(|l| (t.pgid, l.fd())))
                .collect();
            let links_at = fds.len();
            fds.extend(links.iter().map(|&(_, fd)| rt::pollfd(fd)));
            if let Err(e) = rt::wait(&mut fds, deadline) {
                eprintln!("gh0st shell: poll: {e}");
                return 1;
            }
            self.wake_at = Instant::now();
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
                        libc::SIGCHLD => self.reap(),
                        libc::SIGHUP => self.rescan(),
                        // `pkill -USR1 gh0st`: a screenshot without a key,
                        // for checking the panel over SSH.
                        libc::SIGUSR1 => {
                            let _ = self.screenshot();
                        }
                        _ => {
                            self.tasks.close_all(now);
                            return 0;
                        }
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
            for (i, (_, j)) in self.jobs.iter_mut().enumerate() {
                if rt::readable(&fds[jobs_at + i]) {
                    j.read();
                }
            }
            self.finish_jobs();
            for (i, &(pgid, _)) in links.iter().enumerate() {
                if rt::readable(&fds[links_at + i]) {
                    self.read_link(pgid);
                }
            }
            keys_buf.clear();
            self.input.keys(now, rt::readable(&fds[0]), &mut keys_buf);
            for &(c, v) in &keys_buf {
                match v {
                    1 if !self.held.contains(&c) => self.held.push(c),
                    0 => self.held.retain(|&h| h != c),
                    _ => {}
                }
                if c == code::ESC && v != 2 {
                    self.esc_down = v == 1;
                }
                if let Some(k) = self.kb.event(c, v, now) {
                    self.on_key(k, now);
                }
            }
            self.on_timers(now);
            self.sweep(now);
            if self.held.is_empty() {
                match self.pending.take() {
                    Some(Pending::Pause) => self.pause_front(now),
                    Some(Pending::Resume(id)) => self.resume(&id, now),
                    None => {}
                }
            }
        }
    }

    /// Routes every exited child: an app's leader, or a job.
    fn reap(&mut self) {
        for (pid, status) in child::reap() {
            self.tasks.exited(pid);
            if let Some((_, j)) = self.jobs.iter_mut().find(|(_, j)| j.pid == pid) {
                j.exited(status);
            }
        }
        self.finish_jobs();
    }

    fn finish_jobs(&mut self) {
        let (done, running): (Vec<_>, Vec<_>) = std::mem::take(&mut self.jobs)
            .into_iter()
            .partition(|(_, j)| j.done());
        self.jobs = running;
        for (kind, j) in done {
            self.job_done(kind, j);
        }
    }

    /// Drops apps whose group is gone. If it was the one in front, the
    /// shell takes the panel back.
    fn sweep(&mut self, now: u64) {
        let gone = self.tasks.sweep(now);
        if gone.is_empty() {
            return;
        }
        for t in &gone {
            if self.debug {
                eprintln!("gh0st shell: {} closed at {}", t.name, now - self.t0);
            }
            if self.pending == Some(Pending::Resume(t.id.clone())) {
                self.pending = None;
            }
            if self.back == Back::App(t.id.clone()) {
                self.back = Back::Home;
            }
        }
        if gone.iter().any(|t| t.state == tasks::State::Front) {
            if self.pending == Some(Pending::Pause) {
                self.pending = None;
            }
            self.banner = None;
            self.popup = None;
            self.screen = Screen::Home;
            self.take_screen();
        } else {
            let cards = self.cards();
            self.switcher.set_apps(cards);
            self.sync_grab();
            self.dirty = true;
        }
    }

    fn next_frame(&self, now: u64) -> Option<u64> {
        match self.screen {
            Screen::Home => self.home.next_frame(now),
            Screen::Switcher => self.switcher.next_frame(now),
            _ => None,
        }
    }

    fn on_timers(&mut self, now: u64) {
        if let Some(k) = self.kb.repeat_due(now) {
            self.on_key(k, now);
        }
        if self.dim_at().is_some_and(|t| now >= t) {
            self.dim();
        }
        if self.dark_at().is_some_and(|t| now >= t) {
            if self.app_front() {
                self.dark_over_app(now);
            } else {
                self.screen_off(now);
            }
        }
        let app = self.app_front();
        if self.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.toast = None;
            self.level = None;
            self.dirty = true;
            if let Some(p) = self.popup.take() {
                self.spare = Some(p.hide(self.display.as_mut()));
            }
        }
        if self.rescan_at.is_some_and(|t| now >= t) {
            self.rescan_at = None;
            self.rescan();
        }
        if app {
            self.app_timers(now);
            return;
        }
        // Any wake while something moves draws the next frame.
        if self.next_frame(now).is_some() {
            self.dirty = true;
        } else if self.screen == Screen::Switcher && self.stats_at.is_some_and(|t| now >= t) {
            self.sample_usage(now);
        }
        if now >= self.clock_at {
            self.refresh_status();
        }
        if self.screen != Screen::Off {
            self.guard_background();
        }
        // After the guard, which skips a frame already dirty.
        if self.screen != Screen::Off && self.battery_dir.is_some() && now >= self.battery_at {
            self.poll_battery(now);
        }
        if self.tab_down.is_some_and(|t| now >= t + LOCK_HOLD_MS) {
            self.tab_down = None;
            self.lock_now(now);
        }
        if self.screen == Screen::Locked && (self.lock.tick(now) || self.lock_waiting()) {
            self.dirty = true;
        }
        if self.low.warning() == Warning::Countdown {
            if self.screen == Screen::Off {
                self.screen_on(now);
            }
            self.dirty = true;
            if self.low.take_shutdown(now) {
                child::fire(&["systemctl", "poweroff", "-i"]);
            }
        }
    }

    /// ESC held over an app, and the battery countdown drawn over it.
    fn app_timers(&mut self, now: u64) {
        for o in [self.banner.as_mut(), self.popup.as_mut()]
            .into_iter()
            .flatten()
        {
            o.refresh(self.display.as_mut());
        }
        let d = self.esc.update(now, self.esc_down);
        if d.hide_hint {
            if let Some(b) = self.banner.take() {
                self.spare = Some(b.hide(self.display.as_mut()));
            }
        }
        if d.show_hint {
            self.show_banner("Hold Esc to close the app", gh0st_ui::theme::Tone::Warning);
        }
        let pgid = self.tasks.front().map_or(0, |t| t.pgid);
        if d.terminate {
            child::terminate(pgid);
        }
        if d.kill {
            child::kill(pgid);
        }
        if self.low.warning() == Warning::Countdown {
            self.low
                .update(self.battery_dir.as_deref().and_then(battery::sample), now);
            let text = format!(
                "Battery low: powering off in {} s",
                self.low.seconds_left(now)
            );
            self.show_banner(&text, gh0st_ui::theme::Tone::Critical);
            if self.low.take_shutdown(now) {
                child::fire(&["systemctl", "poweroff", "-i"]);
            }
        }
    }

    /// A background app draws to the same framebuffer. Rows it changed are
    /// written again from the shell's frame, and its pixels are kept, so it
    /// shows them when it comes back.
    fn guard_background(&mut self) {
        if self.dirty
            || !self
                .tasks
                .iter()
                .any(|t| t.state == tasks::State::Background && !t.quiet)
        {
            return;
        }
        let mut now = self.canvas_for_overlay();
        self.display.read_into(&mut now);
        let mut changed = false;
        if let Some(t) = self.tasks.background_mut() {
            let px = t
                .pixels
                .get_or_insert_with(|| Canvas::new(now.width(), now.height()));
            for y in 0..now.height() {
                if now.row(y) == self.canvas.row(y) {
                    continue;
                }
                changed = true;
                for x in 0..now.width() {
                    let p = now.get(x, y);
                    if p != self.canvas.get(x, y) {
                        px.set(x, y, p);
                    }
                }
            }
        }
        self.spare = Some(now);
        if changed {
            self.display.flush(&self.canvas, self.canvas.bounds());
        }
    }

    fn canvas_for_overlay(&mut self) -> Canvas {
        self.spare
            .take()
            .unwrap_or_else(|| Canvas::new(self.t.m.width, self.t.m.height))
    }

    fn show_banner(&mut self, text: &str, tone: gh0st_ui::theme::Tone) {
        if let Some(b) = self.banner.take() {
            self.spare = Some(b.hide(self.display.as_mut()));
        }
        let t = self.t;
        let under = self.canvas_for_overlay();
        self.banner = Some(Overlay::show(self.display.as_mut(), under, |c| {
            widgets::banner(c, &t, text, tone)
        }));
    }

    fn show_popup(&mut self, text: &str) {
        if let Some(p) = self.popup.take() {
            self.spare = Some(p.hide(self.display.as_mut()));
        }
        let t = self.t;
        let under = Canvas::new(t.m.width, t.m.height);
        self.popup = Some(Overlay::show(self.display.as_mut(), under, |c| {
            let bottom = t.m.height - t.m.hint_h;
            widgets::toast(c, &t, bottom, gh0st_ui::theme::Tone::Info, text, None)
        }));
    }

    /// When the screen goes dark if no key comes first. Never while an
    /// app in front asked to stay awake; sooner on the lock screen.
    fn dark_at(&self) -> Option<u64> {
        if self.screen == Screen::Off && !self.app_front() {
            return None;
        }
        let d = self.dark_ms?;
        let d = match self.tasks.front() {
            Some(t) if t.awake => return None,
            Some(_) => d,
            None if self.screen == Screen::Locked => d.min(LOCK_DARK_MS),
            None => d,
        };
        Some(self.last_input + d)
    }

    /// When the backlight dims, a short while before dark.
    fn dim_at(&self) -> Option<u64> {
        if self.backlight.is_none() || self.dimmed.is_some() {
            return None;
        }
        let at = self.dark_at()?;
        let lead = DIM_MS.min(at.saturating_sub(self.last_input) / 2);
        Some(at - lead)
    }

    fn dim(&mut self) {
        let Some(bl) = &self.backlight else {
            return;
        };
        let raw = bl.get().unwrap_or(bl.max);
        let low = bl.raw(DIM_PERCENT);
        // Kept even when it is already low, so dark knows what to restore.
        self.dimmed = Some(raw);
        if raw > low {
            let _ = bl.set(low);
        }
    }

    fn undim(&mut self) {
        if let (Some(bl), Some(raw)) = (&self.backlight, self.dimmed.take()) {
            let _ = bl.set(raw);
        }
    }

    /// Dark from home, the switcher or the lock screen.
    fn screen_off(&mut self, now: u64) {
        match self.screen {
            Screen::Home => self.unlock_to = Back::Home,
            Screen::Switcher => self.unlock_to = self.back.clone(),
            _ => {}
        }
        if self.screen != Screen::Locked {
            self.dark_since = Some(now);
        }
        self.go_dark();
    }

    /// Dark over an app: the shell takes the keyboard, then parks the app
    /// as Fn+K does. An app that holds the keyboard itself or can't be
    /// paused keeps the screen on: the shell can't see its keys or stop
    /// its drawing.
    fn dark_over_app(&mut self, now: u64) {
        if !self.input.grab(true) {
            self.last_input = now;
            self.undim();
            return;
        }
        match self.park_front(now) {
            Ok(id) => {
                // Locked first, so take_screen keeps the keyboard.
                self.screen = Screen::Locked;
                self.take_screen();
                self.unlock_to = Back::App(id);
                self.dark_since = Some(now);
                self.go_dark();
            }
            Err(_) => {
                self.input.grab(false);
                self.last_input = now;
                self.undim();
            }
        }
    }

    fn go_dark(&mut self) {
        if let Some(bl) = &self.backlight {
            let raw = self.dimmed.take().or_else(|| bl.get());
            self.saved_raw = raw.filter(|&r| r > 0);
            let _ = bl.set(0);
        }
        self.lock.rest();
        self.home_menu = None;
        self.confirm_all = false;
        self.tab_down = None;
        self.screen = Screen::Off;
        self.sync_grab();
    }

    /// A key in the dark: the lock screen, at rest.
    fn screen_on(&mut self, now: u64) {
        if let Some(bl) = &self.backlight {
            // The panel may have been dark already (stock's timeout), so
            // fall back to the saved level, then full.
            let raw = self
                .saved_raw
                .take()
                .or_else(|| {
                    self.config
                        .get_int("brightness")
                        .filter(|&r| r > 0)
                        .map(|r| r as u32)
                })
                .unwrap_or(bl.max);
            let _ = bl.set(raw);
        }
        self.enter_locked(now);
        self.refresh_status();
    }

    fn enter_locked(&mut self, now: u64) {
        self.screen = Screen::Locked;
        self.lock.rest();
        self.home_menu = None;
        self.confirm_all = false;
        self.tab_down = None;
        self.last_input = now;
        self.full_flush = true;
        self.dirty = true;
        self.probe_account();
        self.sync_grab();
    }

    /// Locked by hand, from home or the switcher: the password is asked.
    fn lock_now(&mut self, now: u64) {
        self.unlock_to = match self.screen {
            Screen::Switcher => self.back.clone(),
            _ => Back::Home,
        };
        self.lock_secure = true;
        self.enter_locked(now);
    }

    fn unlock(&mut self) {
        self.lock.rest();
        self.lock_secure = false;
        self.dark_since = None;
        self.dirty = true;
        if self.debug {
            eprintln!("gh0st shell: unlocked");
        }
        match std::mem::replace(&mut self.unlock_to, Back::Home) {
            // The lock screen stays until the app is back, so home never
            // flashes in between.
            Back::App(id) if self.tasks.get(&id).is_some_and(|t| t.resumable()) => {
                self.pending = Some(Pending::Resume(id));
            }
            _ => {
                self.screen = Screen::Home;
                self.sync_grab();
            }
        }
    }

    fn wants_password(&self, now: u64) -> bool {
        !self.checker_missing
            && asks_password(
                self.lock_secure,
                self.dark_since,
                self.lock_after_ms,
                now,
                self.lock_password,
                self.account,
                self.probing,
            )
    }

    /// The field is open and wrong tries ran out for now.
    fn lock_waiting(&self) -> bool {
        self.screen == Screen::Locked
            && self.lock.field_open()
            && self.tries.wait(password::boottime()) > 0
    }

    /// The lock screen or the dark, with no app in front.
    fn locked(&self) -> bool {
        !self.app_front() && matches!(self.screen, Screen::Locked | Screen::Off)
    }

    /// Runs `unix_chkpwd` on what was typed. The try counts before the
    /// check, so a restart in between can't skip the wait.
    fn check_password(&mut self, mut typed: Vec<u8>) {
        self.tries.fails += 1;
        self.tries.last = password::boottime();
        self.save_tries();
        let mut input = password::chkpwd_input(&typed);
        password::wipe(&mut typed);
        let prog = std::env::var("GH0ST_CHKPWD").unwrap_or_else(|_| password::CHKPWD.into());
        let user = self.user.clone();
        let started = self.start_job_with(JobKind::Chkpwd, &[&prog, &user, "nonull"], Some(&input));
        password::wipe(&mut input);
        if !started {
            // No checker means no way in: let the user in, and stop asking.
            self.checker_missing = true;
            self.tries = Tries::default();
            self.save_tries();
            self.unlock();
        }
    }

    /// Messages from the app in group `pgid`.
    fn read_link(&mut self, pgid: i32) {
        let mut toasts = Vec::new();
        let mut slept = false;
        if let Some(t) = self.tasks.iter_mut().find(|t| t.pgid == pgid) {
            while let Some(m) = t.link.as_ref().and_then(|l| l.recv()) {
                match m.first().map(String::as_str) {
                    Some("hello") => t.linked = true,
                    Some("toast") => toasts.extend(m.get(1).cloned()),
                    Some("awake") => {
                        let on = m.get(1).is_some_and(|v| v == "1");
                        slept |= t.awake && !on;
                        t.awake = on;
                    }
                    _ => {}
                }
            }
        }
        // The full timeout again from here, not at once.
        if slept {
            self.last_input = rt::now_ms();
        }
        // What an app says stays private while the device is locked.
        if self.locked() {
            return;
        }
        for text in toasts {
            self.toast(text);
        }
    }

    fn draw_menu(&mut self, sel: usize) {
        let t = self.t;
        let rows = self.menu_rows();
        let has_app = self.home.selected().is_some();
        let rows: Vec<home::MenuRow> = rows
            .iter()
            .map(|(act, label, value, keys)| home::MenuRow {
                label,
                choice: (*act == MenuAct::Layout).then_some(value.as_str()),
                keys,
                // Nothing selected, nothing to pin.
                dim: *act == MenuAct::Pin && !has_app,
            })
            .collect();
        home::draw_menu(&mut self.canvas, &t, &rows, sel);
    }

    fn draw_lock(&mut self, clock: &str) {
        let t = self.t;
        let mut bar = status_of("", self.battery, self.net, &[], None);
        bar.bluetooth = self.bluetooth;
        let date = rt::date_text();
        let wait = self.tries.wait(password::boottime());
        let pw_on = self.lock_password && !self.checker_missing;
        let wait_note = format!("Try again in {wait} s");
        let note = if !self.lock.field_open() {
            let no_password =
                pw_on && !self.probing && self.account.is_some_and(|a| a != Account::Password);
            no_password.then_some((Tone::Info, "No password set"))
        } else if self.lock.checking {
            Some((Tone::Info, "Checking\u{2026}"))
        } else if wait > 0 {
            Some((Tone::Warning, wait_note.as_str()))
        } else if self.lock.wrong {
            Some((Tone::Critical, "Wrong password"))
        } else {
            None
        };
        let l = widgets::Lock {
            clock,
            date: &date,
            field: self.lock.field_open().then(|| self.lock.typed()),
            ready: !self.lock.checking && wait == 0,
            note,
        };
        widgets::lock_screen(&mut self.canvas, &t, &bar, &l);
    }

    /// The switcher's Close all window.
    fn draw_confirm_all(&mut self) {
        let t = self.t;
        let c = &mut self.canvas;
        let body = widgets::body(&t);
        widgets::scrim(c, &t, body);
        let n = self.switcher.apps().len();
        let line = if n == 1 {
            "The open app closes. Work it hasn't saved is lost.".to_string()
        } else {
            format!("{n} apps close. Work they haven't saved is lost.")
        };
        let w = t.m.width - 44;
        let lines = wrap_words(t.f.body, &line, w - 24);
        let body_h = lines.len() as i32 * 18 + t.m.card_gap + t.m.pill_h;
        let h = widgets::card_h(&t, body_h);
        let r = Rect::new(22, body.y + (body.h - h) / 2, w, h);
        let inner = widgets::card(c, &t, r, "Close all apps?", t.c.critical);
        let mut y = inner.y;
        for l in lines {
            widgets::text_in(
                c,
                t.f.body,
                Rect::new(inner.x, y, inner.w, 18),
                &l,
                t.c.soft(),
            );
            y += 18;
        }
        let hints = [
            widgets::Hint {
                key: "Esc",
                label: "cancel",
            },
            widgets::Hint {
                key: "Enter",
                label: "close all",
            },
        ];
        widgets::key_row(c, &t, inner.x, inner.bottom() - t.m.pill_h, &hints);
        let none = widgets::Chip::default();
        widgets::action_bar(c, &t, &[none; 5]);
    }

    /// The volume or brightness level in the top bar's place, over an app
    /// too, for as long as a toast.
    fn show_level(&mut self, icon: &'static str, label: &'static str, pct: u8) {
        if self.app_front() {
            if let Some(p) = self.popup.take() {
                self.spare = Some(p.hide(self.display.as_mut()));
            }
            let t = self.t;
            let under = Canvas::new(t.m.width, t.m.height);
            self.popup = Some(Overlay::show(self.display.as_mut(), under, |c| {
                widgets::level_bar(c, &t, icon, label, pct)
            }));
        }
        self.level = Some((icon, label, pct));
        self.toast = Some((String::new(), rt::now_ms() + TOAST_MS));
        self.dirty = true;
    }

    fn toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.level = None;
        if self.app_front() {
            self.show_popup(&text);
        }
        self.toast = Some((text, rt::now_ms() + TOAST_MS));
        self.dirty = true;
    }

    /// Saves what the panel shows. Over an app, without the shell's overlays.
    fn screenshot(&mut self) -> std::io::Result<PathBuf> {
        if !self.app_front() {
            return save_screenshot(&self.canvas);
        }
        let mut shot = Canvas::new(self.t.m.width, self.t.m.height);
        self.display.read_into(&mut shot);
        if let Some(o) = self.banner.as_ref().or(self.popup.as_ref()) {
            // An overlay is up: its saved pixels are the app's.
            shot = o.under().clone();
        }
        save_screenshot(&shot)
    }

    fn on_key(&mut self, k: Key, now: u64) {
        if k.state == State::Release && self.swallow_release == Some(k.code) {
            self.swallow_release = None;
            return;
        }
        if k.state == State::Press {
            self.last_input = now;
            self.undim();
        }
        if self.app_front() {
            self.app_key(&k);
            return;
        }
        if self.screen == Screen::Off {
            if k.state == State::Press {
                self.swallow_release = Some(k.code);
                self.screen_on(now);
                // Tab may wake it: Enter then unlocks.
                if k.is(code::TAB) {
                    self.lock.key(&k, now, false, false);
                }
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
            let password = self.wants_password(now);
            let waiting = self.tries.wait(password::boottime()) > 0;
            match self.lock.key(&k, now, password, waiting) {
                lock::Action::None => {}
                lock::Action::Redraw => self.dirty = true,
                lock::Action::Unlock => self.unlock(),
                lock::Action::Check(typed) => {
                    self.check_password(typed);
                    self.dirty = true;
                }
            }
            return;
        }
        if k.is(code::TAB) && self.home_menu.is_none() {
            match k.state {
                State::Press => self.tab_down = Some(now),
                // A tap opens the task manager from home and closes it
                // again. A hold locks the screen instead (see tick).
                State::Release if self.tab_down.take().is_some() => match self.screen {
                    Screen::Switcher => self.switcher_back(),
                    Screen::Home => {
                        // Opened from home, the switcher starts on Home: what is in front.
                        let at = 0;
                        self.open_switcher(Back::Home, at);
                    }
                    _ => {}
                },
                _ => {}
            }
            return;
        }
        if self.screen == Screen::Switcher {
            if self.confirm_all {
                self.confirm_key(k, now);
            } else {
                self.switcher_key(k, now);
            }
            return;
        }
        if self.home_menu.is_some() {
            self.menu_key(k);
            return;
        }
        if k.is(code::HOME) {
            match k.state {
                State::Press => self.home_armed = true,
                State::Release if self.home_armed => {
                    self.home_armed = false;
                    // Opened from home, the switcher starts on Home: what is in front.
                    let at = 0;
                    self.open_switcher(Back::Home, at);
                }
                _ => {}
            }
            return;
        }
        match self.home.mode() {
            Mode::Nav => self.nav_key(k),
            Mode::Search => self.search_key(k),
        }
    }

    /// Keys while an app is in front. The app reads them too: the shell
    /// takes only the Home key, ESC held, and the global keys.
    fn app_key(&mut self, k: &Key) {
        if is_screenshot(k) && k.state == State::Press {
            let msg = screenshot_message(self.screenshot());
            self.toast(msg);
            return;
        }
        if k.is(code::HOME) && k.state == State::Press {
            self.pending = Some(Pending::Pause);
            return;
        }
        self.global_key(k);
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
            let msg = screenshot_message(self.screenshot());
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
                    self.show_level("sun", "Brightness", pct);
                }
                true
            }
            code::VOLUMEUP | code::VOLUMEDOWN => {
                let step = if k.code == code::VOLUMEUP {
                    VOLUME_STEP
                } else {
                    -VOLUME_STEP
                };
                let v = (self.volume.unwrap_or(50) as i32 + step).clamp(0, 100) as u8;
                self.apply_volume(v);
                let label = if self.muted == Some(true) {
                    "Muted"
                } else {
                    "Volume"
                };
                self.show_level("volume-2", label, v);
                true
            }
            code::MUTE => {
                let muted = !self.muted.unwrap_or(false);
                self.apply_mute(muted);
                match (muted, self.volume) {
                    (true, v) => self.show_level("volume-x", "Muted", v.unwrap_or(0)),
                    (false, Some(v)) => self.show_level("volume-2", "Volume", v),
                    (false, None) => self.toast("Sound on"),
                }
                true
            }
            _ => false,
        }
    }

    /// Sets the output level and keeps it in the stock `volume` key, which
    /// every stock app restores at start.
    fn apply_volume(&mut self, v: u8) {
        let arg = format!("{v}%");
        child::fire(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &arg]);
        self.volume = Some(v);
        self.config = Config::load(&config::default_path());
        self.config.set_int("volume", v as i64);
        let _ = self.config.save();
    }

    fn apply_mute(&mut self, muted: bool) {
        let flag = if muted { "1" } else { "0" };
        child::fire(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", flag]);
        self.muted = Some(muted);
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
            Some('s' | '6') => {
                self.home.start_search();
                self.dirty = true;
            }
            Some('4') => {
                // Opened from home, the switcher starts on Home: what is in front.
                let at = 0;
                self.open_switcher(Back::Home, at);
            }
            Some('8') => {
                self.home_menu = Some(0);
                self.dirty = true;
            }
            Some('p') => self.toggle_pin(),
            Some('l') => self.toggle_layout(),
            // The keys over the chips are theirs; the rest open tiles.
            Some(d @ '1'..='3') if self.home.select_on_screen(d as usize - '1' as usize) => {
                self.open_selected();
            }
            _ => {}
        }
    }

    /// Home's menu rows: what each does, its label, its value, its keys.
    fn menu_rows(&self) -> [(MenuAct, String, String, &'static str); MENU_ROWS] {
        let name = self
            .home
            .selected()
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let pinned = self
            .home
            .selected()
            .is_some_and(|e| self.pins.contains(&e.id));
        let pin = if pinned { "Unpin" } else { "Pin" };
        let layout = match self.home.layout() {
            Layout::Grid => "Grid",
            Layout::List => "List",
        };
        [
            (MenuAct::Pin, format!("{pin} {name}"), String::new(), "p"),
            (MenuAct::Layout, "Layout".into(), layout.into(), "l"),
            (MenuAct::Find, "Find".into(), String::new(), "s"),
            (MenuAct::Switch, "Switch apps".into(), String::new(), "Fn k"),
            (MenuAct::Lock, "Lock".into(), String::new(), "Tab"),
        ]
    }

    fn menu_key(&mut self, k: Key) {
        if k.state != State::Press && k.state != State::Repeat {
            return;
        }
        let Some(sel) = self.home_menu else {
            return;
        };
        let n = MENU_ROWS;
        let act = match k.nav(true) {
            Some(keys::Nav::Up) => {
                self.home_menu = Some((sel + n - 1) % n);
                None
            }
            Some(keys::Nav::Down) => {
                self.home_menu = Some((sel + 1) % n);
                None
            }
            Some(keys::Nav::Left | keys::Nav::Right) if sel == 1 => Some(MenuAct::Layout),
            Some(_) => None,
            None if k.state == State::Repeat => None,
            None if k.is(code::ESC) || k.ch == Some('8') => {
                self.home_menu = None;
                None
            }
            None if k.is(code::ENTER) => Some(self.menu_rows()[sel].0),
            None if k.is(code::TAB) => Some(MenuAct::Lock),
            None => match k.ch {
                Some('p') => Some(MenuAct::Pin),
                Some('l') => Some(MenuAct::Layout),
                Some('s') => Some(MenuAct::Find),
                _ => None,
            },
        };
        self.dirty = true;
        let Some(act) = act else {
            return;
        };
        // A setting changes in place; an action closes the menu first.
        if act != MenuAct::Layout {
            self.home_menu = None;
        }
        match act {
            MenuAct::Pin if self.home.selected().is_none() => {}
            MenuAct::Pin => self.toggle_pin(),
            MenuAct::Layout => self.toggle_layout(),
            MenuAct::Find => self.home.start_search(),
            MenuAct::Switch => {
                // Opened from home, the switcher starts on Home: what is in front.
                let at = 0;
                self.open_switcher(Back::Home, at);
            }
            MenuAct::Lock => self.lock_now(rt::now_ms()),
        }
    }

    fn toggle_layout(&mut self) {
        let l = self.home.layout().toggled();
        self.config.set_str("gh0st.home_layout", l.config_name());
        let _ = self.config.save();
        self.home.set_layout(l);
        self.dirty = true;
    }

    /// The switcher's Close all window: Enter closes every app, Esc keeps
    /// them.
    fn confirm_key(&mut self, k: Key, now: u64) {
        if k.is(code::ENTER) {
            match k.state {
                State::Press => self.enter_armed = true,
                State::Release if self.enter_armed => {
                    self.enter_armed = false;
                    self.confirm_all = false;
                    self.tasks.close_all(now);
                    let cards = self.cards();
                    self.switcher.set_apps(cards);
                }
                _ => {}
            }
        } else if k.is(code::ESC) && k.state == State::Press {
            self.confirm_all = false;
        } else {
            return;
        }
        self.dirty = true;
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
        self.toggle_pin_of(&id);
    }

    fn toggle_pin_of(&mut self, id: &str) {
        let id = id.to_string();
        let name = self
            .apps
            .iter()
            .find(|a| a.file == id)
            .map(|a| a.entry.name.clone())
            .unwrap_or_default();
        if let Some(i) = self.pins.iter().position(|p| *p == id) {
            self.pins.remove(i);
            self.toast(format!("Unpinned {name}"));
        } else {
            self.pins.push(id);
            self.toast(format!("Pinned {name}"));
        }
        let _ = apps::save_pins(&self.pins);
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
        match self.tasks.get(&id).map(|t| t.state) {
            Some(tasks::State::Closing { .. }) => {
                self.toast(format!("{} is closing", app.entry.name))
            }
            Some(_) => self.pending = Some(Pending::Resume(id)),
            None => self.launch(&app),
        }
    }

    /// Starts `app` in front. The shell lets go of the panel and keyboard
    /// until the app is paused or gone.
    fn launch(&mut self, app: &App) {
        let argv = child::argv(app, &self.terminal);
        let fb_path = self.display.device();
        let kb_path = self.input.path();
        let dev = Devices {
            fb: &fb_path,
            keyboard: &kb_path,
            keymap: keys::SYM_MAP_PATH,
        };
        let pause = app.entry.terminal && app.entry.sysplause;
        self.input.grab(false);
        self.display.suspend();
        let (link, app_end) = match Link::pair() {
            Ok((l, a)) => (Some(l), Some(a)),
            Err(_) => (None, None),
        };
        let spawned = child::spawn(&argv, &dev, pause, app_end.as_ref().map(|f| f.as_raw_fd()));
        // The app holds its own copy now.
        drop(app_end);
        if let Some(l) = &link {
            l.send(&["hello", "1"]);
        }
        let pgid = match spawned {
            Ok(p) => p,
            Err(e) => {
                self.display.resume();
                self.full_flush = true;
                self.sync_grab();
                self.toast(format!("Could not open {}: {e}", app.entry.name));
                return;
            }
        };
        let now = rt::now_ms();
        if self.debug {
            eprintln!(
                "gh0st shell: started {} (pgid {pgid}) at {}",
                app.entry.name,
                now - self.t0
            );
        }
        let icon = self.entry_for(app).icon;
        self.tasks.add(Task {
            id: app.file.clone(),
            name: app.entry.name.clone(),
            icon,
            pgid,
            state: tasks::State::Front,
            leader_done: false,
            pixels: None,
            used: now,
            mem_kb: None,
            link,
            linked: false,
            background: app.entry.background,
            quiet: false,
            awake: false,
        });
        self.esc = EscHold::default();
        self.toast = None;
    }

    /// Fn+K over an app: pause it, keep its frame, open the switcher.
    fn pause_front(&mut self, now: u64) {
        match self.park_front(now) {
            Ok(id) => {
                self.take_screen();
                let at = self.card_of(&id);
                self.open_switcher(Back::App(id), at);
            }
            Err(Some(name)) => self.toast(format!("{name} can't be paused")),
            Err(None) => {}
        }
    }

    /// Takes the app in front off the panel: an app that may run behind is
    /// told to pause, any other is stopped, its last frame kept. The caller
    /// takes the screen. Err with the app's name when it can't be stopped.
    fn park_front(&mut self, now: u64) -> Result<String, Option<String>> {
        if let Some(b) = self.banner.take() {
            self.spare = Some(b.hide(self.display.as_mut()));
        }
        if let Some(p) = self.popup.take() {
            self.spare = Some(p.hide(self.display.as_mut()));
        }
        let mut frame = self.canvas_for_overlay();
        let Some(t) = self.tasks.front_mut() else {
            self.spare = Some(frame);
            return Err(None);
        };
        let background = self
            .config
            .get_bool(&apps::app_key(&t.id, "background"))
            .unwrap_or(t.background && t.linked);
        if background {
            // An app on the link stops drawing and reading keys when told;
            // one that doesn't answer is guarded as before.
            t.quiet = t.linked
                && t.link
                    .as_ref()
                    .is_some_and(|l| l.send(&["pause"]) && l.wait_for("paused", STOP_WAIT_MS));
            if t.quiet {
                t.pixels = None;
                self.spare = Some(frame);
            } else {
                self.display.read_into(&mut frame);
                t.pixels = Some(frame);
            }
            t.state = tasks::State::Background;
            t.used = now;
            return Ok(t.id.clone());
        }
        let sent = child::stop(t.pgid);
        if !t.leader_done && child::wait_stopped(t.pgid, STOP_WAIT_MS) == Stopped::Exited {
            t.leader_done = true;
        }
        // An app run through sudo is root's: the stop does not reach it,
        // and a "paused" app that keeps drawing would sit over the shell.
        if !sent || !child::group_stopped(t.pgid, STOP_WAIT_MS) {
            child::cont(t.pgid);
            let name = t.name.clone();
            self.spare = Some(frame);
            return Err(Some(name));
        }
        self.display.read_into(&mut frame);
        t.pixels = Some(frame);
        t.state = tasks::State::Paused;
        t.used = now;
        if self.debug {
            eprintln!("gh0st shell: paused {} at {}", t.name, now - self.t0);
        }
        Ok(t.id.clone())
    }

    /// Brings a paused app back: its last frame on the panel, the keyboard
    /// released with nothing stale queued, then SIGCONT.
    fn resume(&mut self, id: &str, now: u64) {
        let Some(t) = self.tasks.get_mut(id) else {
            return;
        };
        if !matches!(t.state, tasks::State::Paused | tasks::State::Background) {
            return;
        }
        if let Some(px) = t.pixels.take() {
            self.display.flush_all(&px);
            self.spare = Some(px);
        }
        self.display.suspend();
        t.state = tasks::State::Front;
        t.used = now;
        t.mem_kb = None;
        let pgid = t.pgid;
        // A quiet app redraws all of itself when told to resume.
        if std::mem::take(&mut t.quiet) {
            if let Some(l) = &t.link {
                l.send(&["resume"]);
            }
        }
        if self.debug {
            eprintln!("gh0st shell: resumed {} at {}", t.name, now - self.t0);
        }
        self.input.grab(false);
        self.input.flush_readers();
        child::cont(pgid);
        self.esc = EscHold::default();
        self.esc_down = false;
        self.toast = None;
    }

    /// Takes the panel back from an app that paused or exited.
    fn take_screen(&mut self) {
        self.display.resume();
        self.kb.reset();
        self.enter_armed = false;
        self.esc_armed = false;
        self.home_armed = false;
        self.esc = EscHold::default();
        self.last_input = rt::now_ms();
        self.full_flush = true;
        // Settings and stock apps may have changed config.json and the pins,
        // and the app may have installed or removed apps (the Store does).
        self.reload_config();
        self.pins = apps::load_pins();
        self.refresh_status();
        self.rescan();
        let cards = self.cards();
        self.switcher.set_apps(cards);
        self.sync_grab();
        self.dirty = true;
    }

    /// The keyboard is the shell's alone while it is in front and some app
    /// is paused, so a paused app queues none of the shell's keys, and while
    /// locked or dark, so no app behind reads the password.
    fn sync_grab(&mut self) {
        let want = !self.app_front()
            && (self.tasks.any_paused() || matches!(self.screen, Screen::Locked | Screen::Off));
        self.input.grab(want);
    }

    fn open_switcher(&mut self, back: Back, at: usize) {
        self.back = back;
        let cards = self.cards();
        self.switcher.set_apps(cards);
        self.switcher.select(at);
        self.screen = Screen::Switcher;
        self.dirty = true;
        self.sampler.reset();
        self.switcher.clear_usage();
        // The first frame shows what costs nothing to know: memory of the
        // system and of apps already read. The walk waits for the frame:
        // the loop draws before it polls.
        if let Some(system) = usage::memory() {
            let usage = self.tasks.usage(|_| None);
            self.switcher.set_usage(&usage, system);
        }
        self.stats_at = Some(rt::now_ms());
    }

    /// Reads the switcher's figures. Only the trees of apps that need a
    /// read are walked: those running behind, and paused ones not yet read.
    fn sample_usage(&mut self, now: u64) {
        let read: Vec<(String, i32)> = self
            .tasks
            .iter()
            .filter(|t| match t.state {
                tasks::State::Background => true,
                tasks::State::Paused => t.mem_kb.is_none(),
                _ => false,
            })
            .map(|t| (t.id.clone(), t.pgid))
            .collect();
        let groups: Vec<i32> = read.iter().map(|r| r.1).collect();
        let Some(sample) = self.sampler.sample(now, &groups) else {
            self.stats_at = None;
            return;
        };
        for ((id, _), u) in read.iter().zip(&sample.apps) {
            if let Some(t) = self.tasks.get_mut(id) {
                t.mem_kb = Some(u.mem_kb);
            }
        }
        let usage = self.tasks.usage(|id| {
            let i = read.iter().position(|r| r.0 == id)?;
            sample.apps[i].cpu
        });
        if self.switcher.set_usage(&usage, sample.system) {
            self.dirty = true;
        }
        let wait = match sample.system.cpu {
            Some(_) => STATS_MS,
            None => STATS_FIRST_MS,
        };
        self.stats_at = Some(now + wait);
    }

    /// The dock and the switcher: open apps, in the order they opened.
    fn cards(&self) -> Vec<Card> {
        self.tasks.cards()
    }

    /// The card of app `id`, so the switcher opens on the app it was opened
    /// over. 0 is Home.
    fn card_of(&self, id: &str) -> usize {
        self.cards()
            .iter()
            .position(|c| c.id == id)
            .map_or(0, |i| i + 1)
    }

    /// Esc on the switcher, or Fn+K again: back to where it was opened.
    fn switcher_back(&mut self) {
        match self.back.clone() {
            Back::App(id) if self.tasks.get(&id).is_some_and(|t| t.resumable()) => {
                self.pending = Some(Pending::Resume(id));
            }
            _ => {
                self.screen = Screen::Home;
                self.dirty = true;
            }
        }
    }

    fn switcher_key(&mut self, k: Key, now: u64) {
        let armed = |flag: &mut bool| match k.state {
            State::Press => {
                *flag = true;
                false
            }
            State::Release if *flag => {
                *flag = false;
                true
            }
            _ => false,
        };
        if k.is(code::ENTER) {
            if armed(&mut self.enter_armed) {
                match self.switcher.selected_app().map(|c| c.id.clone()) {
                    None => {
                        self.screen = Screen::Home;
                        self.dirty = true;
                    }
                    Some(id) => {
                        if self.tasks.get(&id).is_some_and(|t| t.resumable()) {
                            self.pending = Some(Pending::Resume(id));
                        }
                    }
                }
            }
            return;
        }
        if k.is(code::ESC) {
            if armed(&mut self.esc_armed) {
                self.switcher_back();
            }
            return;
        }
        if k.is(code::HOME) {
            if armed(&mut self.home_armed) {
                self.switcher_back();
            }
            return;
        }
        if k.state == State::Release {
            return;
        }
        match k.nav(true) {
            Some(keys::Nav::Left) => self.switcher.move_by(-1, now),
            Some(keys::Nav::Right) => self.switcher.move_by(1, now),
            _ if (k.is(code::BACKSPACE) || k.ch == Some('5')) && k.state == State::Press => {
                // Del or 5 on an app closes it; on Home, Del asks to close
                // every app, as 7 does.
                match self.switcher.selected_app().map(|c| c.id.clone()) {
                    Some(id) => {
                        self.tasks.close(&id, now);
                        let cards = self.cards();
                        self.switcher.set_apps(cards);
                    }
                    None if k.is(code::BACKSPACE) && !self.switcher.apps().is_empty() => {
                        self.confirm_all = true;
                    }
                    None => {}
                }
            }
            _ if k.ch == Some('7') && k.state == State::Press => {
                if !self.switcher.apps().is_empty() {
                    self.confirm_all = true;
                }
            }
            _ if k.ch == Some('6') && k.state == State::Press => self.lock_now(now),
            _ => return,
        }
        self.dirty = true;
    }

    fn draw(&mut self) {
        // A menu or a question belongs to the screen it was opened on.
        if self.screen != Screen::Home {
            self.home_menu = None;
        }
        if self.screen != Screen::Switcher {
            self.confirm_all = false;
        }
        let start = Instant::now();
        let t = self.t;
        let now = rt::now_ms();
        let clock = self.clock.clone();
        let cards = self.cards();
        let dock: Vec<Visual> = cards
            .iter()
            .map(|c| home::visual(&t, &c.icon, &c.name, Size::Tiny))
            .collect();
        let front = match self.screen {
            Screen::Switcher => Some(self.switcher.selected()),
            _ => Some(0),
        };
        let mut status = status_of(&clock, self.battery, self.net, &dock, front);
        status.bluetooth = self.bluetooth;
        match self.screen {
            Screen::Locked => self.draw_lock(&clock),
            Screen::Switcher => self.switcher.draw(&mut self.canvas, &t, &status, now),
            _ => self.home.draw(&mut self.canvas, &t, &status, now),
        }
        if self.screen == Screen::Home {
            if let Some(sel) = self.home_menu {
                self.draw_menu(sel);
            }
        }
        if self.screen == Screen::Switcher && self.confirm_all {
            self.draw_confirm_all();
        }
        drop(dock);
        drop(cards);
        if matches!(
            self.screen,
            Screen::Home | Screen::Switcher | Screen::Locked
        ) {
            self.draw_low_battery();
            if let Some((icon, label, pct)) = self.level {
                widgets::level_bar(&mut self.canvas, &t, icon, label, pct);
            } else if let Some((text, _)) = &self.toast {
                let bottom = t.m.height - t.m.hint_h;
                let info = gh0st_ui::theme::Tone::Info;
                widgets::toast(&mut self.canvas, &t, bottom, info, text, None);
            }
        }
        let drawn = Instant::now();
        let rows = if self.full_flush {
            self.full_flush = false;
            self.display.flush_all(&self.canvas);
            self.canvas.height() as usize
        } else {
            self.display.flush(&self.canvas, self.canvas.bounds())
        };
        let flushed = Instant::now();
        self.dirty = false;
        self.home.settle(now);
        self.switcher.settle(now);
        // While something moves, wait for this frame to reach the panel and
        // draw the next one right away: frames then lock to its refresh.
        self.paced = rows > 0 && self.next_frame(now).is_some() && self.display.present();
        if self.debug {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
            eprintln!(
                "gh0st shell: flushed {rows} rows: wake to draw {:.2} ms, draw {:.2} ms, flush {:.2} ms, present {:.2} ms",
                ms(self.wake_at, start),
                ms(start, drawn),
                ms(drawn, flushed),
                ms(flushed, Instant::now())
            );
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
        widgets::scrim(c, &t, c.bounds());
        let inner = widgets::card(c, &t, r, title, color);
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

/// fbdev, as stock apps use it. The panel driver syncs every send to the
/// panel's refresh, so fbdev no longer tears. `GH0ST_DRM=1` draws through DRM
/// instead: tighter rectangles, but the handover to and from an app shows.
fn open_panel() -> Result<Box<dyn Display>, String> {
    let fb_path = fb::find();
    let fbdev = fb_path.to_string_lossy().into_owned();
    if std::env::var_os("GH0ST_DRM").is_some() {
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

/// A symbol baked into gh0st-ui, by name.
fn symbol(name: &str) -> Option<&'static str> {
    gh0st_ui::icon::ICON_NAMES
        .iter()
        .find(|n| **n == name)
        .copied()
}

/// An app's icon. A symbol picked in Settings > Apps (`custom`) wins over
/// the app's own. A system app (ours, with `Icon=gh0st:`) keeps the theme's
/// colours either way. Any other symbol takes the hashed `tint`: SDK apps
/// use `gh0st:` too.
fn icon_for(
    own: &IconRef,
    ours: bool,
    custom: Option<&'static str>,
    tint: usize,
    picture: impl FnOnce(&Path) -> Option<Rc<Picture>>,
) -> Icon {
    let name = match own {
        IconRef::Symbol(name) => custom.or_else(|| symbol(name)),
        _ => custom,
    };
    match (own, name) {
        (IconRef::Symbol(_), Some(name)) if ours => Icon::System { name },
        (_, Some(name)) => Icon::Symbol { name, tint },
        (IconRef::Path(p), None) => match picture(p) {
            Some(pic) => Icon::Picture(pic),
            None => Icon::Letter { tint },
        },
        _ => Icon::Letter { tint },
    }
}

fn palette_of(c: &Config) -> theme::Palette {
    c.get_str("gh0st.palette")
        .and_then(theme::find)
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

fn lock_password_of(c: &Config) -> bool {
    c.get_bool("gh0st.lock.password").unwrap_or(true)
}

/// `gh0st.lock.after` in seconds; -1 means only by hand.
fn lock_after_of(c: &Config) -> Option<u64> {
    let secs = c
        .get_int("gh0st.lock.after")
        .unwrap_or(DEFAULT_LOCK_AFTER_SECS);
    (secs >= 0).then_some(secs as u64 * 1000)
}

/// Whether a lock asks for the password: one made by hand or at start, or
/// dark at least `after`, with the password lock on and a password to ask.
fn asks_password(
    secure: bool,
    dark_since: Option<u64>,
    after: Option<u64>,
    now: u64,
    on: bool,
    account: Option<Account>,
    probing: bool,
) -> bool {
    let real = secure || matches!((dark_since, after), (Some(d), Some(a)) if now >= d + a);
    let has = match account {
        Some(Account::Password) => true,
        // Until passwd -S answers, ask: a probe that fails says no.
        None => probing,
        Some(_) => false,
    };
    real && on && has
}

fn status_of<'a>(
    clock: &'a str,
    battery: Option<Reading>,
    net: Net,
    dock: &'a [Visual<'a>],
    front: Option<usize>,
) -> Status<'a> {
    Status {
        dock,
        front,
        icon: "",
        title: "",
        context: "",
        tabs: &[],
        front_tab: 0,
        item: None,
        clock,
        battery: battery.map(|r| Battery {
            percent: r.percent,
            charging: r.charging,
        }),
        net: match net {
            Net::None => widgets::Net::None,
            Net::Wifi(l) => widgets::Net::Wifi(l),
            Net::Ethernet => widgets::Net::Ethernet,
        },
        bluetooth: false,
    }
}

fn desktop_first_token_runs(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .is_some_and(|p| gh0st_core::desktop::is_executable(Path::new(p)))
}

fn screenshot_message(r: std::io::Result<PathBuf>) -> String {
    match r {
        Ok(p) => format!(
            "Saved {}",
            p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("screenshot")
        ),
        Err(e) => format!("Screenshot failed: {e}"),
    }
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

/// "Volume: 0.40 [MUTED]" from `wpctl get-volume`, as (percent, muted).
fn parse_volume(s: &str) -> Option<(u8, bool)> {
    let v: f32 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some((
        (v * 100.0).round().clamp(0.0, 150.0) as u8,
        s.contains("[MUTED]"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_comes_with_a_real_lock_only() {
        let pw = Some(Account::Password);
        let ask =
            |secure, since, after, now| asks_password(secure, since, after, now, true, pw, false);
        assert!(ask(true, None, Some(300_000), 0), "by hand or at start");
        assert!(
            !ask(false, Some(1000), Some(300_000), 200_000),
            "a short dark"
        );
        assert!(ask(false, Some(1000), Some(300_000), 301_000), "a long one");
        assert!(ask(false, Some(1000), Some(0), 1000), "at once");
        assert!(!ask(false, Some(1000), None, u64::MAX / 2), "only by hand");
        assert!(
            !asks_password(true, None, None, 0, false, pw, false),
            "the toggle off"
        );
        let none = |account, probing| asks_password(true, None, None, 0, true, account, probing);
        assert!(!none(Some(Account::NoPassword), false));
        assert!(!none(Some(Account::Locked), false), "no lockout");
        assert!(none(None, true), "asked until passwd answers");
        assert!(!none(None, false), "a failed probe asks nothing");
    }

    #[test]
    fn only_system_apps_take_theme_colours() {
        let sys = IconRef::Symbol("settings".into());
        let png = IconRef::Path("/x.png".into());
        let style = |own: &IconRef, custom| match icon_for(own, true, custom, 3, |_| None) {
            Icon::System { name } => format!("system {name}"),
            Icon::Symbol { name, tint } => format!("tint {tint} {name}"),
            Icon::Letter { tint } => format!("letter {tint}"),
            Icon::Picture(_) => "picture".into(),
        };
        assert_eq!(style(&sys, None), "system settings");
        assert_eq!(style(&sys, Some("pin")), "system pin");
        assert_eq!(style(&png, Some("pin")), "tint 3 pin");
        assert_eq!(style(&png, None), "letter 3", "PNG failed to load");
        assert_eq!(style(&IconRef::Symbol("nope".into()), None), "letter 3");
        let theirs = icon_for(&sys, false, None, 3, |_| None);
        assert!(
            matches!(
                theirs,
                Icon::Symbol {
                    name: "settings",
                    tint: 3
                }
            ),
            "an SDK app from the Store keeps a tint"
        );
    }

    #[test]
    fn wpctl_volume_reads_as_percent_and_mute() {
        assert_eq!(parse_volume("Volume: 0.40\n"), Some((40, false)));
        assert_eq!(parse_volume("Volume: 0.65 [MUTED]\n"), Some((65, true)));
        assert_eq!(parse_volume("nope"), None);
    }
}

/// What a row of home's menu does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAct {
    Pin,
    Layout,
    Find,
    Switch,
    Lock,
}

const MENU_ROWS: usize = 5;

/// Splits `s` into lines no wider than `w`, at spaces.
fn wrap_words(f: gh0st_ui::font::Font, s: &str, w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if f.measure(&next) > w && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
