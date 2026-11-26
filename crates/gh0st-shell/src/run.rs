//! The shell process. It owns the panel and the keyboard while no app is in
//! front, and steps aside while one is: it draws nothing then but the ESC
//! hint, toasts and the battery countdown, laid over the app. Apps sent to
//! the background are paused. One thread, one `poll`. While idle with the
//! screen on, the clock ticks each minute and the battery is read every 2 s.
//! Dark, neither runs.

use crate::hold::{HeldKey, LOCK_HOLD_MS, TabHold};
use crate::home::{self, Entry, Home, Icon, Layout, Mode, Picture, Size};
use crate::lock::{self, Lock};
use crate::notices::{self, Panel};
use crate::overlay::{self, Overlay};
use crate::power::{self, Power};
use crate::prompt::{self, Ask, Kind, Origin, Prompts};
use crate::services::{Said, Services, Which};
use crate::sound::{self as sounds, Player};
use crate::switcher::{Card, HoldTick, Switcher};
use crate::tasks::{self, Task, Tasks};
use gh0st_core::apps::{self, App, Dirs, IconRef};
use gh0st_core::backlight::Backlight;
use gh0st_core::battery::{self, LowBattery, Reading, Warning};
use gh0st_core::child::{self, Devices, Stopped};
use gh0st_core::config::{self, Config};
use gh0st_core::esc::EscHold;
use gh0st_core::evdev;
use gh0st_core::handlers;
use gh0st_core::job::{self, Job};
use gh0st_core::keyring::{Answer, ToDaemon, ToShell};
use gh0st_core::keys::{self, Key, Keyboard, State, code};
use gh0st_core::link::{self, Link};
use gh0st_core::notify::{self, Dismissed, Notice};
use gh0st_core::password::{self, Account, Tries};
use gh0st_core::rfkill;
use gh0st_core::rt;
use gh0st_core::sandbox::{self, Cgroups};
use gh0st_core::script::Script;
use gh0st_core::sound::Sound;
use gh0st_core::status::{self, Net, Watch};
use gh0st_core::usage::{self, Sampler};
use gh0st_ui::display::{Display, Frames};
use gh0st_ui::drm;
use gh0st_ui::fb::{self, Framebuffer};
use gh0st_ui::theme::{self, Theme, Tone};
use gh0st_ui::wallpaper;
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
/// The backlight dims this long before it goes off.
const DIM_MS: u64 = 5000;
const DIM_PERCENT: u8 = 10;
/// The lock screen at rest goes dark sooner, so a pocket keeps it lit
/// only briefly.
const LOCK_DARK_MS: u64 = 10_000;
const TOAST_MS: u64 = 1500;
/// A notice stays a little longer: it has two parts to read.
const NOTICE_TOAST_MS: u64 = 3000;
const RESCAN_DEBOUNCE_MS: u64 = 300;
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
    /// The notifications, from the switcher's 5 or home's menu.
    Notices,
    Locked,
    /// Backlight off after `dark_time`. The next key wakes the lock screen
    /// once Lock after has passed, else goes straight back.
    Off,
    /// A keyring question, asked while the shell holds the keyboard.
    Prompt,
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
    /// The app in front asked for another over the link: pause it, then
    /// bring this one up, opening these files.
    Open(String, Vec<PathBuf>),
    /// The app in front steps back: the app that opened it comes back, or
    /// home, and it runs on behind if it may.
    Leave,
    /// Tab was held over the app: park it and lock, once Tab is up so the
    /// app sees the whole press.
    Lock,
}

/// The lock screen sliding up off the top at the unlock, over what it
/// unlocks to.
struct LockSlide {
    anim: crate::motion::Anim<()>,
    /// The lock screen's last frame.
    lock: Canvas,
    /// The app the unlock brings back: its last frame is under the slide,
    /// and it resumes once the slide is over.
    app: Option<String>,
}

/// Whose words the banner over an app shows, so each takes down only its
/// own. The battery's countdown wins over the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BannerOwner {
    Esc,
    Tab,
    Battery,
}

/// A portal request the shell acts on, once every key is up.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PortalAsk {
    /// Run Files as the chooser for request `id`, asked by `pid`.
    Pick { id: String, pid: i32 },
    /// Open `path` in its app for request `id`.
    Open { id: String, pid: i32, path: PathBuf },
}

/// Task ids of the choosers: never an app's own file name.
const PICKER: &str = "gh0st-picker-";
/// Portal questions over a frozen screen or a picture at once. Each keeps
/// a frame and its backdrop, about 300 KB; past this, the ask is refused.
const FROZEN_MAX: usize = 4;

/// What a watched fd says changed. Only Linux has the watches.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Wake {
    /// The app folders: rescan.
    Apps,
    /// Battery, charger and links: re-read the status.
    Status,
    /// A notice was posted or removed.
    Notices,
}

/// A helper whose answer the shell is waiting for.
#[derive(Clone, PartialEq, Eq)]
enum JobKind {
    Volume,
    /// `passwd -S`: does the account have a password?
    Passwd,
    /// `unix_chkpwd`: was that the password?
    Chkpwd,
    /// `unix_chkpwd` on a keyring prompt's password.
    PromptCheck,
    /// The helper restarting (true) or powering off.
    Power(bool),
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

    /// The keys down now, or None where only events tell (a script).
    fn pressed(&self) -> Option<Vec<u16>> {
        match self {
            Input::Device(d) => d.pressed(),
            Input::Script(_) => None,
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
    /// Home's and the lock's picture, baked; none when it is off.
    wall: Option<wallpaper::Baked>,
    /// Wallpapers apps asked for, or frozen screens, by portal request:
    /// each shown behind its question.
    preview: Vec<(u64, wallpaper::Baked)>,
    /// The frozen screens portal questions are about, by request.
    shot: Vec<(u64, Canvas)>,
    /// Choosers of a `gh0st session` that died: the new one may reuse
    /// their ids, so their end is not told to it.
    orphans: Vec<String>,
    /// The process group `gh0st session` was last told is in front, for
    /// the clipboard; -1 before it knows.
    front_told: i32,
    /// What `wall` was baked from: the setting, the palette, the file's age.
    wall_from: Option<(String, &'static str, Option<std::time::SystemTime>)>,
    display: Box<dyn Display>,
    input: Input,
    kb: Keyboard,
    signals: rt::Signals,
    watches: Vec<(Wake, Watch)>,
    pictures: HashMap<PathBuf, (Option<SystemTime>, Rc<Picture>)>,
    lock: Lock,
    /// This lock holds however short the dark: locked by hand or at start.
    lock_secure: bool,
    /// When the screen first went dark since the last unlock.
    dark_since: Option<u64>,
    /// Where unlocking goes.
    unlock_to: Back,
    /// Home or the switcher, when the screen went dark from there: where a
    /// wake inside Lock after goes back to.
    wake_to: Option<Screen>,
    /// `gh0st.lock.password`.
    lock_password: bool,
    /// `gh0st.lock.after`, or None for only by hand.
    lock_after_ms: Option<u64>,
    /// What `passwd -S` said; None before it answered or when it failed.
    account: Option<Account>,
    probing: bool,
    /// This lock already asked `passwd -S` a second time.
    probe_retried: bool,
    /// What the running `unix_chkpwd` was given, kept for one retry and
    /// wiped when it answers, and its pid: an answer from an older check,
    /// left by Esc, is not this one's.
    check_input: Option<Vec<u8>>,
    check_pid: i32,
    /// This check already ran a second time.
    check_retried: bool,
    tries: Tries,
    /// Where the try count is kept, so a restarted shell still waits.
    tries_path: Option<PathBuf>,
    user: String,
    /// The backlight before it dimmed.
    dimmed: Option<u32>,
    /// `gh0st.dark.wifi_off`: Wi-Fi goes off this long into the dark.
    dark_wifi_ms: Option<u64>,
    /// When the screen last went dark.
    off_at: u64,
    /// This dark has had its Wi-Fi turn.
    wifi_done: bool,
    /// The shell turned Wi-Fi off, so the wake turns it back on.
    wifi_slept: bool,
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
    /// What the banner says, so a wake that changes nothing sends nothing.
    banner_text: Option<(String, gh0st_ui::theme::Tone)>,
    banner_owner: Option<BannerOwner>,
    /// Tab held over an app, toward the lock.
    tab_hold: TabHold,
    lock_slide: Option<LockSlide>,
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
    /// Every notice shown, newest first: posted and not dismissed.
    notices: Vec<Notice>,
    /// Ids and times already seen, so only a new post toasts.
    notices_known: Vec<(String, u64)>,
    dismissed: Dismissed,
    panel: Panel,
    /// Where Esc on the notifications goes: Home or the switcher.
    panel_back: Screen,
    /// The toast is this notice, drawn with its app's picture.
    toast_notice: Option<notices::Row>,
    /// (opened, opener), innermost last: an app opened over another over
    /// the link. When it closes, the opener comes back, and so on down.
    return_to: Vec<(String, String)>,
    /// Every app that opens files, hidden ones too.
    openers: Vec<App>,
    /// Portal requests waiting for the keys to be up and the screen on.
    portal: std::collections::VecDeque<PortalAsk>,
    home_armed: bool,
    swallow_release: Option<u16>,
    /// Esc held on home: the fill toward the power dialog.
    esc_hold: HeldKey,
    /// The power dialog over home.
    power: Option<Power>,
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
    /// `gh0st keyring` and `gh0st session`.
    services: Services,
    /// Under `--sim`: tests may swap the programs that check passwords.
    sim: bool,
    /// Where each app's cgroup goes, when the unit delegates one.
    cgroups: Option<Cgroups>,
    /// The system is going down: restarting (true) or powering off. The
    /// last frame says so, and the panel keeps it until the power goes.
    bye: Option<bool>,
    /// The shell started a power off (a flat battery), which halts with
    /// the power on: the bye screen asks for the side switch.
    ask_switch: bool,
    /// The real device, not `--sim`: apps run sandboxed.
    on_device: bool,
    /// The binary `gh0st audio` runs from. In a sim, only a test's stub.
    audio_exe: Option<PathBuf>,
    /// System sounds.
    player: Player,
    /// A notice came in the dark: it chimes at the next wake.
    chime_at_wake: bool,
    /// The keyring's questions.
    prompts: Prompts,
    /// The keyring said its vault is open.
    keyring_open: bool,
    /// This unlock sent the keyring a password.
    unlock_sent: bool,
    /// Asked to open the keyring this boot already.
    open_asked: bool,
    /// Where the prompt goes back to: the screen, and the app it parked.
    prompt_from: Screen,
    prompt_app: Option<String>,
    /// The prompt password under `unix_chkpwd`, and that check's pid.
    prompt_input: Option<Vec<u8>>,
    prompt_pid: i32,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

/// Where the baked wallpaper is kept between starts.
fn wall_cache() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("gh0st").join("wallpaper.bin"))
}

impl Shell {
    fn start(t0: u64, sim: Option<PathBuf>) -> Result<Shell, String> {
        child::become_subreaper();
        // The lock screen and the keyring's prompts read passwords here.
        // With no Yama on the device, any program of ours could trace a
        // dumpable shell and read them; a non-dumpable one only root can.
        // Apps it starts are dumpable again: exec resets it.
        #[cfg(target_os = "linux")]
        // SAFETY: prctl with plain values.
        unsafe {
            libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0)
        };
        let signals = rt::Signals::install(&[
            libc::SIGCHLD,
            libc::SIGTERM,
            libc::SIGINT,
            libc::SIGHUP,
            libc::SIGUSR1,
        ])
        .map_err(|e| format!("signals: {e}"))?;
        // Before anything reads them: the keys an app must not set move out
        // of config.json, and the folders a sandbox locks are made.
        if let Err(e) = Config::migrate(&config::default_path()) {
            eprintln!("gh0st shell: settings: {e}");
        }
        if let (None, Some(home)) = (&sim, std::env::var_os("HOME")) {
            sandbox::prepare(Path::new(&home), &notify::user_dir());
        }
        let config = Config::load(&config::default_path());
        let t = Theme::new(theme::named(config.palette_name()));
        let (display, input): (Box<dyn Display>, Input) = match &sim {
            Some(dir) => {
                let frames = Frames::composited(dir.clone(), t.m.width, t.m.height)
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
        let layout = Layout::of(&config);
        let terminal = terminal_of(&config);
        let dark_ms = config.dark_ms();

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
            banner_text: None,
            banner_owner: None,
            tab_hold: TabHold::default(),
            lock_slide: None,
            popup: None,
            level: None,
            spare: None,
            jobs: Vec::new(),
            volume: config
                .get_int(config::key::VOLUME)
                .map(|v| v.clamp(0, 100) as u8),
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
            wake_to: None,
            lock_password: config.lock_password(),
            lock_after_ms: config.lock_after_ms(),
            account: None,
            probing: false,
            probe_retried: false,
            check_input: None,
            check_pid: 0,
            check_retried: false,
            tries: Tries::default(),
            tries_path: None,
            // SAFETY: getuid has no failure mode.
            user: password::user_name(unsafe { libc::getuid() }).unwrap_or_default(),
            dimmed: None,
            dark_wifi_ms: config.dark_wifi_ms(),
            off_at: 0,
            wifi_done: false,
            wifi_slept: false,
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
            notices: Vec::new(),
            notices_known: Vec::new(),
            dismissed: Dismissed::load(&notify::dismissed_path()),
            panel: Panel::default(),
            panel_back: Screen::Home,
            toast_notice: None,
            return_to: Vec::new(),
            openers: Vec::new(),
            portal: Default::default(),
            home_armed: false,
            swallow_release: None,
            esc_hold: HeldKey::default(),
            power: None,
            rescan_at: None,
            sampler: Sampler::default(),
            stats_at: None,
            debug: std::env::var("GH0ST_LOG").is_ok_and(|v| v == "debug"),
            t0,
            wake_at: Instant::now(),
            paced: false,
            first_frame: true,
            // No bus on a host sim, unless a test brings its own.
            services: match (&sim, std::env::var_os("GH0ST_SERVICES")) {
                (Some(_), None) => Services::none(),
                // Test overrides (another binary, a scratch vault) only in a sim.
                _ => Services::start(rt::now_ms(), sim.is_some()),
            },
            prompts: Prompts::default(),
            sim: sim.is_some(),
            cgroups: if on_device { Cgroups::own() } else { None },
            bye: None,
            ask_switch: false,
            on_device,
            audio_exe: if on_device {
                Some(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/gh0st")))
            } else {
                // A test watches the calls through a stub.
                std::env::var_os("GH0ST_AUDIO_EXE").map(PathBuf::from)
            },
            player: Player::new(if on_device {
                Some("pacat".into())
            } else {
                std::env::var("GH0ST_PACAT").ok()
            }),
            chime_at_wake: false,
            keyring_open: false,
            unlock_sent: false,
            open_asked: false,
            prompt_from: Screen::Home,
            prompt_app: None,
            prompt_input: None,
            prompt_pid: 0,
            config,
            wall: None,
            preview: Vec::new(),
            shot: Vec::new(),
            orphans: Vec::new(),
            front_told: -1,
            wall_from: None,
        };
        s.prompts.size = (s.t.m.width, s.t.m.height);
        s.set_animations();
        s.rebake();
        s.boot_restore();
        if on_device {
            s.start_job(
                JobKind::Volume,
                &["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"],
            );
        }
        s.rescan();
        s.reload_notices(false);
        s.refresh_status();
        if on_device {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = dir.join("gh0st-lock");
            s.tries = Tries::load(&path, password::boottime());
            s.tries_path = Some(path);
        }
        // A shell that stopped in the dark left Wi-Fi off.
        if on_device && std::fs::remove_file(wifi_marker()).is_ok() {
            child::fire(&["nmcli", "radio", "wifi", "on"]);
        }
        // Its paused apps went with it; their null sinks did not.
        audio(s.audio_exe.as_deref(), "clear", None);
        if on_device && sounds::first_since_boot() {
            s.sound(Sound::Boot);
        }
        // A restart or a power cycle must not skip the lock.
        if s.lock_password {
            s.lock_secure = true;
            s.enter_locked(rt::now_ms());
        } else {
            // Straight to home: no password opens the keyring, so ask
            // once `passwd -S` says the account has one.
            s.probe_account();
        }
        Ok(s)
    }

    fn set_animations(&mut self) {
        let on = self.config.animations();
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
                    self.volume = Some(v.min(100));
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
                // One more try before a failed probe means no password.
                if self.account.is_none() && !self.probe_retried {
                    self.probe_retried = true;
                    self.probe_account();
                }
                // The answer came after the unlock: ask then.
                if !matches!(self.screen, Screen::Locked | Screen::Off) {
                    self.maybe_ask_open();
                }
                self.dirty = true;
            }
            JobKind::Chkpwd => {
                let result = password::chkpwd_result(j.code());
                if self.debug {
                    eprintln!("gh0st shell: password {result:?}");
                }
                // A check Esc left: its try counted, its answer is old.
                if j.pid != self.check_pid {
                    return;
                }
                // Dark or Esc meanwhile rested the field: the answer is stale.
                let current = self.screen == Screen::Locked && self.lock.checking;
                if result == password::Check::Broken && current && !self.check_retried {
                    if let Some(input) = self.check_input.take() {
                        self.check_retried = true;
                        self.run_check(input);
                        return;
                    }
                }
                if let Some(mut input) = self.check_input.take() {
                    // Only a password the system agreed to opens the vault,
                    // and it goes before the app comes back.
                    if result == password::Check::Right && current {
                        let pw = input.strip_suffix(&[0]).unwrap_or(&input).to_vec();
                        let mut m = ToDaemon::Unlock { pw };
                        self.unlock_sent |= self.services.tell(&m);
                        m.wipe();
                    }
                    password::wipe(&mut input);
                }
                // A password the system refused stays out: if the vault took
                // it meanwhile, the device is still locked, so is the vault.
                if result == password::Check::Wrong && current {
                    self.services.tell(&ToDaemon::Lock);
                    self.keyring_open = false;
                }
                match result {
                    password::Check::Right => self.tries = Tries::default(),
                    // Not a try: it never checked.
                    password::Check::Broken => {
                        self.tries.fails = self.tries.fails.saturating_sub(1)
                    }
                    password::Check::Wrong => {}
                }
                self.save_tries();
                if current {
                    let ok = result != password::Check::Wrong;
                    self.sound(if ok { Sound::Unlock } else { Sound::Error });
                    self.lock.checked(ok, rt::now_ms());
                    if ok {
                        // Broken twice lets the user in this once: a
                        // lockout means a reflash. The next lock asks again.
                        self.unlock();
                    }
                }
                self.dirty = true;
            }
            JobKind::PromptCheck => self.prompt_checked(&j),
            JobKind::Power(restart) if !j.ok() => {
                self.bye = None;
                self.ask_switch = false;
                self.full_flush = true;
                self.toast(if restart {
                    "Can't restart"
                } else {
                    "Can't power off"
                });
            }
            JobKind::Power(_) => {}
        }
    }

    /// Restarts or powers off through the helper. logind asks first when
    /// the caller is not in an active seat's session, and the shell, a user
    /// service, is not. Run without a terminal, systemctl doesn't stop for
    /// inhibitors, which the battery's power off needs.
    fn power_now(&mut self, restart: bool) {
        let verb = if restart { "reboot" } else { "off" };
        if self.debug || self.sim {
            eprintln!("gh0st shell: power {verb}");
        }
        if self.sim {
            self.bye = Some(restart);
            // Only the battery powers off from here.
            self.ask_switch = !restart;
            self.dirty = true;
            return;
        }
        let argv = ["pkexec", "/usr/libexec/gh0st-helper", "power", verb];
        if self.start_job(JobKind::Power(restart), &argv) {
            self.bye = Some(restart);
            self.ask_switch = !restart;
            self.dirty = true;
        } else {
            self.toast(if restart {
                "Can't restart"
            } else {
                "Can't power off"
            });
        }
    }

    /// Esc held on home fills toward the power dialog; in the dialog, a
    /// held chip fills toward its action. Each runs only if its key is
    /// still down at the end.
    fn power_tick(&mut self, now: u64) {
        match self.esc_hold.tick(now) {
            HoldTick::Idle => {}
            HoldTick::Fill => self.dirty = true,
            HoldTick::Done(key) => {
                self.dirty = true;
                if self.held.contains(&key) {
                    self.power = Some(Power::default());
                    self.swallow_release = Some(key);
                }
            }
        }
        let Some((act, key)) = self.power.as_mut().map(|p| p.tick(now)) else {
            return;
        };
        match act {
            power::Action::Redraw => self.dirty = true,
            power::Action::Run => {
                self.dirty = true;
                if key.is_some_and(|k| self.held.contains(&k)) {
                    self.power = None;
                    self.power_now(true);
                }
            }
            _ => {}
        }
    }

    /// No password came with the unlock (the lock password is off), so
    /// nothing opened the keyring: ask once a boot. At the unlock, and when
    /// `passwd -S` answers after it.
    fn maybe_ask_open(&mut self) {
        if self.unlock_sent
            || self.keyring_open
            || self.open_asked
            || !self.services.running(Which::Keyring)
            || self.account != Some(Account::Password)
        {
            return;
        }
        self.open_asked = true;
        self.prompts.push(Ask {
            origin: Origin::Open,
            kind: Kind::Password {
                reason: "Your password opens it for your apps.".into(),
                retry: false,
            },
        });
    }

    /// What the daemons said.
    fn services_said(&mut self) {
        for said in self.services.read() {
            match said {
                Said::Keyring(m) => self.keyring_said(m),
                Said::Session(m) => match m.first().map(String::as_str) {
                    Some("awake") => {
                        if let [_, stem, on] = m.as_slice() {
                            self.inhibit(stem, on == "1");
                        }
                    }
                    Some("pick") => {
                        if let [_, id, pid] = m.as_slice() {
                            let pid = pid.parse().unwrap_or(0);
                            self.portal.push_back(PortalAsk::Pick {
                                id: id.clone(),
                                pid,
                            });
                        }
                    }
                    Some("open") => {
                        if let [_, id, pid, path] = m.as_slice() {
                            self.portal.push_back(PortalAsk::Open {
                                id: id.clone(),
                                pid: pid.parse().unwrap_or(0),
                                path: link::path_of(path),
                            });
                        }
                    }
                    Some("account") => {
                        if let [_, id, pid, app] = m.as_slice() {
                            self.ask_account(id, pid.parse().unwrap_or(0), app);
                        }
                    }
                    Some("lowmem") => {
                        if let Some(level) = m.get(1) {
                            self.low_memory(level);
                        }
                    }
                    Some(v @ ("shot" | "color")) => {
                        if let [_, id, pid, app, rest @ ..] = m.as_slice() {
                            let verb = if v == "shot" { "shot" } else { "color" };
                            let quiet = rest.first().is_some_and(|q| q == "quiet");
                            self.ask_screen(verb, id, pid.parse().unwrap_or(0), app, quiet);
                        }
                    }
                    Some("wallpaper") => {
                        if let [_, id, pid, app, path] = m.as_slice() {
                            let path = link::path_of(path);
                            self.ask_wallpaper(id, pid.parse().unwrap_or(0), app, &path);
                        }
                    }
                    Some("cancel") => {
                        if let Some(id) = m.get(1) {
                            if let Ok(n) = id.parse() {
                                self.prompts.cancel_portal(n);
                                self.drop_preview(n);
                                self.take_shot(n);
                            }
                            self.portal.retain(|a| match a {
                                PortalAsk::Pick { id: i, .. } | PortalAsk::Open { id: i, .. } => {
                                    i != id
                                }
                            });
                            let task = format!("{PICKER}{id}");
                            self.tasks.close(&task, rt::now_ms());
                        }
                    }
                    _ => {}
                },
            }
        }
        self.dirty = true;
    }

    fn keyring_said(&mut self, m: ToShell) {
        if self.debug {
            eprintln!("gh0st shell: keyring {m:?}");
        }
        match m {
            ToShell::Ready => {}
            ToShell::Unlocked => {
                self.keyring_open = true;
                self.prompts.remove(&Origin::Stale);
                self.prompts.remove(&Origin::Open);
            }
            ToShell::Wrong => {
                // The old password was wrong too: ask again.
                self.prompts.push(stale_ask(true));
            }
            ToShell::Stale => self.prompts.push(stale_ask(false)),
            ToShell::Broken => {
                self.prompts.remove(&Origin::Stale);
                self.toast("The keyring can't read its file");
            }
            ToShell::AskPassword { id, reason, retry } => {
                self.prompts.push(Ask {
                    origin: Origin::Daemon(id),
                    kind: Kind::Password { reason, retry },
                });
                self.light_for_question();
            }
            ToShell::AskApprove { id, app, what } => {
                self.prompts.push(Ask {
                    origin: Origin::Daemon(id),
                    kind: Kind::Approve { app, what },
                });
                self.light_for_question();
            }
            ToShell::Cancel { id } => self.prompts.cancel(id),
        }
    }

    /// A question nobody can see waits for ever: light the screen. Past
    /// Lock after that is the lock screen; the question shows after it.
    fn light_for_question(&mut self) {
        let now = rt::now_ms();
        if self.screen == Screen::Off && !self.app_front() {
            self.wake(now);
        }
        self.last_input = now;
        self.undim();
    }

    /// An app asks for the user's name: the app in front, or a program
    /// it started, gets a yes or no. Anything else is refused.
    fn ask_account(&mut self, id: &str, pid: i32, app: &str) {
        match (id.parse(), self.front_asker(pid, app)) {
            (Ok(n), Some(app)) => {
                self.prompts.push(Ask {
                    origin: Origin::Portal(n, "account"),
                    kind: Kind::Approve {
                        app,
                        what: "see your name".into(),
                    },
                });
                self.light_for_question();
            }
            _ => {
                self.services.say_session(&["account", id, "deny"]);
            }
        }
    }

    /// Who asks, as the question names them, when `pid` belongs to the
    /// app in front; None for anything else. A program run inside the app
    /// (`/usr/bin/foo` in the terminal) reads "foo in Terminal".
    fn front_asker(&self, pid: i32, app: &str) -> Option<String> {
        let t = self.tasks.front()?;
        if pid <= 0 || !gh0st_core::procfs::belongs_to(pid, t.pgid) {
            return None;
        }
        Some(match app.rsplit_once('/') {
            Some((_, exe)) => format!("{exe} in {}", t.name),
            None => app.to_string(),
        })
    }

    /// An app wants `path`, already fitted to the screen, as the
    /// wallpaper. The question shows over the picture itself.
    fn ask_wallpaper(&mut self, id: &str, pid: i32, app: &str, path: &Path) {
        let m = self.t.m;
        if self.preview.len() >= FROZEN_MAX {
            self.services.say_session(&["wallpaper", id, "deny"]);
            return;
        }
        let pic = wallpaper::load(path, m.width as usize, m.height as usize);
        match (id.parse::<u64>(), pic, self.front_asker(pid, app)) {
            (Ok(n), Some(pic), Some(app)) => {
                let baked = wallpaper::bake(&pic, &self.t.c, m.bar_h as usize, m.hint_h as usize);
                self.preview.push((n, baked));
                self.prompts.push(Ask {
                    origin: Origin::Portal(n, "wallpaper"),
                    kind: Kind::Approve {
                        app,
                        what: "make this your wallpaper".into(),
                    },
                });
                self.light_for_question();
            }
            _ => {
                self.services.say_session(&["wallpaper", id, "deny"]);
            }
        }
    }

    /// Memory is short: every app that speaks the link hears it, in front
    /// or behind.
    fn low_memory(&mut self, level: &str) {
        if self.debug {
            eprintln!("gh0st shell: low memory {level}");
        }
        for t in self.tasks.iter() {
            if let Some(l) = t.link.as_ref().filter(|_| t.link_v >= 2) {
                l.send(&["lowmem", level]);
            }
        }
    }

    fn drop_preview(&mut self, id: u64) {
        let had = self.preview.len();
        self.preview.retain(|(i, _)| *i != id);
        if self.preview.len() != had {
            self.full_flush = true;
        }
    }

    /// The frozen screen of request `id`, which no longer keeps it.
    fn take_shot(&mut self, id: u64) -> Option<Canvas> {
        let at = self.shot.iter().position(|(i, _)| *i == id)?;
        Some(self.shot.swap_remove(at).1)
    }

    /// The answer to a portal's question. A screenshot is saved now, and
    /// its path goes with the answer.
    fn answer_portal(&mut self, id: u64, verb: &str, answer: Answer) {
        let mut word = match answer {
            Answer::Once => "once",
            Answer::Always => "always",
            Answer::Deny => "deny",
        };
        self.drop_preview(id);
        let shot = self.take_shot(id);
        let mut fields = vec![verb.to_string(), id.to_string()];
        if verb == "shot" && answer != Answer::Deny {
            match shot.map(|c| save_screenshot(&c)) {
                Some(Ok(p)) => fields.push(link::path_field(&p)),
                Some(Err(e)) => {
                    self.toast(format!("Screenshot failed: {e}"));
                    word = "fail";
                }
                None => word = "fail",
            }
        }
        fields.insert(2, word.to_string());
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        self.services.say_session(&fields);
    }

    /// `gh0st session` relays an inhibit as the app's `awake`. `pid:N` is a
    /// program outside apps: its task is the one that started it.
    fn inhibit(&mut self, stem: &str, on: bool) {
        let file = format!("{stem}.desktop");
        let pid = stem
            .strip_prefix("pid:")
            .and_then(|p| p.parse::<i32>().ok());
        let task = self.tasks.iter_mut().find(|t| match pid {
            Some(p) => p > 0 && gh0st_core::procfs::belongs_to(p, t.pgid),
            None => t.id == file,
        });
        if let Some(t) = task {
            t.inhibited = on;
        }
        if on && self.tasks.front().is_some_and(|t| t.inhibited) {
            self.undim();
        }
    }

    /// What waited for every key to come up.
    fn run_pending(&mut self, now: u64) {
        if self.lock_holds() {
            // Queued before the lock, it dies with it. Kept, it would stop
            // the lock screen drawing and bring the app up past it.
            self.pending = None;
            return;
        }
        // The app comes back once the lock screen has slid off it.
        if !self.held.is_empty() || self.lock_slide.is_some() {
            return;
        }
        match self.pending.take() {
            Some(Pending::Pause) => self.pause_front(now),
            Some(Pending::Resume(id)) => self.resume(&id, now),
            Some(Pending::Open(id, files)) => self.open_over_front(&id, &files, now),
            Some(Pending::Leave) => self.leave_front(now),
            Some(Pending::Lock) => self.lock_over_app(now),
            None => {}
        }
    }

    /// Tells `gh0st session` which app is in front when that changes: only
    /// it may use the clipboard.
    fn tell_front(&mut self) {
        let front = self.tasks.front().map_or(0, |t| t.pgid);
        if front != self.front_told && self.services.say_session(&["front", &front.to_string()]) {
            self.front_told = front;
        }
    }

    /// A daemon died and was started again: what it held is gone, and so
    /// are its questions. The other's stay.
    fn daemon_died(&mut self, w: Which) {
        match w {
            Which::Session => {
                self.front_told = -1;
                self.drop_portal();
            }
            Which::Keyring => {
                self.keyring_open = false;
                self.prompts.retain(|o| matches!(o, Origin::Portal(..)));
            }
        }
        if self.screen == Screen::Prompt && self.prompts.is_empty() {
            self.prompt_leave();
        }
    }

    /// The old `gh0st session` is gone and the new one numbers requests
    /// afresh: nothing kept for the old ones may answer the new.
    fn drop_portal(&mut self) {
        self.prompts.retain(|o| !matches!(o, Origin::Portal(..)));
        self.portal.clear();
        self.shot.clear();
        if !self.preview.is_empty() {
            self.preview.clear();
            self.full_flush = true;
        }
        let now = rt::now_ms();
        let choosers: Vec<String> = self
            .tasks
            .iter()
            .filter(|t| t.id.starts_with(PICKER))
            .map(|t| t.id.clone())
            .collect();
        for id in choosers {
            self.tasks.close(&id, now);
            self.orphans.push(id);
        }
        for t in self.tasks.iter_mut() {
            t.inhibited = false;
        }
    }

    /// A daemon kept dying: say so, as a notice that stays.
    fn daemon_gave_up(&mut self, w: Which) {
        let (tag, title) = match w {
            Which::Keyring => ("keyring", "The keyring stopped"),
            Which::Session => ("session", "Notifications stopped"),
        };
        let n = Notice {
            app: "gh0st".into(),
            tag: tag.into(),
            title: title.into(),
            body: "It kept failing to start. Restart the device.".into(),
            tone: notify::Tone::Critical,
            time: notify::now_secs(),
            ..Notice::default()
        };
        let _ = notify::post(&notify::user_dir(), &n);
    }

    /// Acts on the next portal request once nothing else is under way.
    /// Only the app in front, or a program it started, may ask.
    fn maybe_portal(&mut self, now: u64) {
        if self.portal.is_empty()
            || self.pending.is_some()
            || !self.held.is_empty()
            || self.screen == Screen::Prompt
            || self.locked()
        {
            return;
        }
        let Some(ask) = self.portal.pop_front() else {
            return;
        };
        let (id, pid) = match &ask {
            PortalAsk::Pick { id, pid } | PortalAsk::Open { id, pid, .. } => (id.clone(), *pid),
        };
        let front = self.tasks.front().map(|t| t.pgid);
        let allowed = front.is_some_and(|g| pid > 0 && gh0st_core::procfs::belongs_to(pid, g));
        if !allowed {
            if self.debug {
                eprintln!("gh0st shell: portal {id} refused: pid {pid} is not in front");
            }
            self.services.say_session(&["refuse", &id]);
            return;
        }
        match ask {
            PortalAsk::Open { path, .. } => {
                let ok = self.open_file(None, path);
                self.services
                    .say_session(&[if ok { "done" } else { "refuse" }, &id]);
            }
            // A chooser of a session that died may still be closing under
            // this id: the new request must not take its place.
            PortalAsk::Pick { .. } if self.tasks.get(&format!("{PICKER}{id}")).is_some() => {
                self.services.say_session(&["refuse", &id]);
            }
            PortalAsk::Pick { .. } => {
                let Some(files) = self
                    .openers
                    .iter()
                    .chain(&self.apps)
                    .find(|a| a.ours && a.file == "files.desktop")
                    .cloned()
                else {
                    self.services.say_session(&["refuse", &id]);
                    return;
                };
                let mut picker = files;
                let program = picker.entry.exec.split_whitespace().next().unwrap_or("");
                picker.entry.exec = format!("{program} --pick {id}");
                picker.file = format!("{PICKER}{id}");
                picker.entry.background = false;
                self.openers.push(picker);
                self.open_over_front(&format!("{PICKER}{id}"), &[], now);
                self.openers.retain(|a| !a.file.starts_with(PICKER));
                match self.tasks.get(&format!("{PICKER}{id}")) {
                    Some(t) => {
                        let pgid = t.pgid.to_string();
                        self.services.say_session(&["picker", &id, &pgid]);
                    }
                    None => {
                        self.services.say_session(&["refuse", &id]);
                    }
                }
            }
        }
    }

    /// Shows the next question when nothing else is in the way. Over an
    /// app, it parks the app first; never over the lock screen or the dark.
    fn maybe_prompt(&mut self, now: u64) {
        if self.screen == Screen::Prompt {
            if self.prompts.is_empty() && self.pending.is_none() {
                self.prompt_leave();
            }
            return;
        }
        if self.prompts.is_empty()
            || self.pending.is_some()
            || !self.held.is_empty()
            || self.locked()
        {
            return;
        }
        // Every key is up (checked above): park the app now. A pending
        // handover would wait for a key that may never come.
        if self.app_front() {
            self.prompt_over_front(now);
            return;
        }
        self.prompt_from = self.screen;
        self.prompt_app = None;
        self.open_prompt();
    }

    fn open_prompt(&mut self) {
        self.home_menu = None;
        self.switcher.cancel_hold();
        self.screen = Screen::Prompt;
        self.full_flush = true;
        self.dirty = true;
        self.sync_grab();
    }

    /// Parks the app in front, then asks. An app that can't be parked (run
    /// through sudo, or holding the keyboard) could read the answer: the
    /// question is refused instead.
    fn prompt_over_front(&mut self, now: u64) {
        if self.prompts.is_empty() {
            return;
        }
        match self.park_front(now) {
            Ok(id) => {
                self.take_screen();
                self.prompt_from = Screen::Home;
                self.prompt_app = Some(id);
                self.open_prompt();
            }
            Err(name) => {
                let why = match name {
                    Some(n) => format!("{n} holds the screen: can't ask"),
                    None => "Can't ask now".into(),
                };
                self.refuse_prompts(&why);
            }
        }
    }

    fn prompt_leave(&mut self) {
        self.prompts.checking = false;
        // Home under a resuming app is never drawn: the loop skips frames
        // while it comes back.
        self.screen = match self.prompt_from {
            Screen::Switcher | Screen::Notices => self.prompt_from,
            _ => Screen::Home,
        };
        if let Some(id) = self.prompt_app.take() {
            if self.tasks.get(&id).is_some_and(|t| t.resumable()) {
                self.pending = Some(Pending::Resume(id));
            }
        }
        self.full_flush = true;
        self.dirty = true;
        self.sync_grab();
    }

    /// Every waiting question is answered no.
    fn refuse_prompts(&mut self, why: &str) {
        while let Some(a) = self.prompts.current().cloned() {
            match a.origin {
                Origin::Daemon(id) => {
                    self.services.tell(&ToDaemon::Answer {
                        id,
                        answer: Answer::Deny,
                    });
                }
                Origin::Portal(id, verb) => self.answer_portal(id, verb, Answer::Deny),
                _ => {}
            }
            self.prompts.done();
        }
        self.toast(why.to_string());
        if self.screen == Screen::Prompt {
            self.prompt_leave();
        }
    }

    fn prompt_key(&mut self, k: &Key, now: u64) {
        match self.prompts.key(k, now) {
            prompt::Action::None => {}
            prompt::Action::Redraw => self.dirty = true,
            prompt::Action::Check(mut pw) => {
                if self.tries.wait(password::boottime()) > 0 {
                    password::wipe(&mut pw);
                    self.prompts.checking = false;
                    self.toast("Too many tries: wait a little");
                } else {
                    self.tries.fails += 1;
                    self.tries.last = password::boottime();
                    self.save_tries();
                    let input = password::chkpwd_input(&pw);
                    self.run_prompt_check(input);
                    self.try_vault(pw);
                }
                self.dirty = true;
            }
            prompt::Action::Old(pw) => {
                let mut m = ToDaemon::Old { pw };
                self.services.tell(&m);
                m.wipe();
                self.prompts.done();
                self.dirty = true;
            }
            prompt::Action::Answer(origin, answer) => {
                match origin {
                    Origin::Daemon(id) => {
                        self.services.tell(&ToDaemon::Answer { id, answer });
                    }
                    Origin::Portal(id, verb) => self.answer_portal(id, verb, answer),
                    _ => {}
                }
                self.prompts.done();
                self.dirty = true;
            }
            prompt::Action::Dismiss(origin) => {
                match origin {
                    Origin::Daemon(id) => {
                        self.services.tell(&ToDaemon::Answer {
                            id,
                            answer: Answer::Deny,
                        });
                    }
                    Origin::Portal(id, verb) => self.answer_portal(id, verb, Answer::Deny),
                    _ => {}
                }
                if let Some(mut old) = self.prompt_input.take() {
                    password::wipe(&mut old);
                }
                self.prompts.done();
                self.dirty = true;
            }
            prompt::Action::Color(origin, (x, y)) => {
                if let Origin::Portal(id, _) = origin {
                    let picked = self
                        .take_shot(id)
                        .map(|c| gh0st_ui::color::to_rgb8(c.get(x, y)));
                    let id = id.to_string();
                    match picked {
                        Some([r, g, b]) => self.services.say_session(&[
                            "color",
                            &id,
                            "ok",
                            &r.to_string(),
                            &g.to_string(),
                            &b.to_string(),
                        ]),
                        None => self.services.say_session(&["color", &id, "deny"]),
                    };
                    self.drop_preview(origin_id(&origin));
                }
                self.prompts.done();
                self.dirty = true;
            }
        }
    }

    fn run_prompt_check(&mut self, mut input: Vec<u8>) {
        let prog = self.test_prog("GH0ST_CHKPWD", password::CHKPWD);
        let user = self.user.clone();
        if self.start_job_with(
            JobKind::PromptCheck,
            &[&prog, &user, "nonull"],
            Some(&input),
        ) {
            self.prompt_pid = self.jobs.last().map_or(0, |(_, j)| j.pid);
            if let Some(mut old) = self.prompt_input.replace(input) {
                password::wipe(&mut old);
            }
        } else {
            password::wipe(&mut input);
            self.prompts.checked_wrong();
            self.toast("Can't check the password");
        }
    }

    /// The prompt's password was checked: a right one goes to the keyring.
    fn prompt_checked(&mut self, j: &Job) {
        let Some(mut input) = self.prompt_input.take() else {
            return;
        };
        let stale = j.pid != self.prompt_pid || !self.prompts.checking;
        let result = password::chkpwd_result(j.code());
        if stale {
            // The vault opened on the same password meanwhile and the
            // prompt went: the check still counts for the backoff.
            if result == password::Check::Right {
                self.tries = Tries::default();
                self.save_tries();
            }
            password::wipe(&mut input);
            return;
        }
        match result {
            password::Check::Right => {
                self.tries = Tries::default();
                self.save_tries();
                let pw = input.strip_suffix(&[0]).unwrap_or(&input).to_vec();
                let mut m = match self.prompts.current().map(|a| a.origin.clone()) {
                    Some(Origin::Daemon(id)) => ToDaemon::Password { id, pw },
                    _ => ToDaemon::Unlock { pw },
                };
                self.services.tell(&m);
                m.wipe();
                self.prompts.done();
            }
            password::Check::Wrong => self.prompts.checked_wrong(),
            password::Check::Broken => {
                // Not a try: nothing checked it. Unchecked, it can't open
                // the vault either.
                self.tries.fails = self.tries.fails.saturating_sub(1);
                self.save_tries();
                self.prompts.checked_wrong();
                self.toast("Can't check the password");
            }
        }
        password::wipe(&mut input);
        self.dirty = true;
    }

    /// The program for a password check: a test's stand-in under `--sim`
    /// only. On a device, nothing in the environment picks what reads the
    /// login password.
    fn test_prog(&self, var: &str, real: &str) -> String {
        match self.sim.then(|| std::env::var(var).ok()).flatten() {
            Some(p) => p,
            None => real.into(),
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
        let prog = self.test_prog("GH0ST_PASSWD", "passwd");
        self.probing = self.start_job(JobKind::Passwd, &[&prog, "-S"]);
    }

    /// What stock APPLaunch did at boot, and every stock app does at start:
    /// saved brightness and volume back on.
    fn boot_restore(&mut self) {
        // As stock: the saved level, or full when none is saved. The panel
        // may be dark from the previous launcher's idle timeout.
        if let Some(bl) = &self.backlight {
            let saved = self
                .config
                .get_int(config::key::BRIGHTNESS)
                .filter(|&r| r > 0);
            let _ = bl.set(saved.map(|r| r as u32).unwrap_or(bl.max));
        }
        if let Some(v) = self.config.get_int(config::key::VOLUME) {
            let v = format!("{}%", v.clamp(0, 100));
            child::fire(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &v]);
        }
    }

    /// Writes one change to config.json, read again first so keys others
    /// wrote meanwhile stay.
    fn save_config(&mut self, f: impl FnOnce(&mut Config)) {
        let path = config::default_path();
        match Config::update(&path, f) {
            Ok(c) => self.config = c,
            Err(e) => eprintln!("gh0st shell: saving {}: {e}", path.display()),
        }
    }

    /// Re-reads config.json. Settings and stock apps write it while the shell
    /// waits, so it is read again after every app and before every write.
    fn reload_config(&mut self) {
        self.config = Config::load(&config::default_path());
        self.t = Theme::new(theme::named(self.config.palette_name()));
        self.home.set_layout(Layout::of(&self.config));
        self.terminal = terminal_of(&self.config);
        self.dark_ms = self.config.dark_ms();
        self.lock_password = self.config.lock_password();
        self.lock_after_ms = self.config.lock_after_ms();
        self.dark_wifi_ms = self.config.dark_wifi_ms();
        self.set_animations();
        self.rebake();
        self.dirty = true;
    }

    /// Bakes the wallpaper again when its setting, the palette or its file
    /// changed. `GH0ST_WALLPAPER` overrides the setting, for sims.
    fn rebake(&mut self) {
        let setting = std::env::var("GH0ST_WALLPAPER")
            .unwrap_or_else(|_| self.config.wallpaper().to_string());
        let dir = config::default_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let file = wallpaper::picture_path(&dir);
        let age = (setting == wallpaper::OWN)
            .then(|| std::fs::metadata(&file).and_then(|m| m.modified()).ok())
            .flatten();
        let from = (setting.clone(), self.t.c.name, age);
        if self.wall_from.as_ref() == Some(&from) {
            return;
        }
        let m = self.t.m;
        let (w, h) = (m.width as usize, m.height as usize);
        // A bake costs about 150 ms on the device; a cached one is a read.
        let key = format!("{setting} {} {age:?}", self.t.c.name);
        let cache = wall_cache();
        let cached = cache
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| wallpaper::from_cache(&b, &key, w * h));
        self.wall = cached.or_else(|| {
            let pic = if setting == wallpaper::OWN {
                wallpaper::load(&file, w, h)
            } else {
                wallpaper::builtin(&setting, &self.t.c, w, h)
            }?;
            let b = wallpaper::bake(&pic, &self.t.c, m.bar_h as usize, m.hint_h as usize);
            if let Some(p) = &cache {
                let _ = p.parent().map(std::fs::create_dir_all);
                let _ = config::write_atomic(p, &wallpaper::cache_bytes(&b, &key));
            }
            Some(b)
        });
        self.wall_from = Some(from);
        self.full_flush = true;
    }

    fn rescan(&mut self) {
        let terminal_ok = desktop_first_token_runs(&self.terminal);
        self.apps = apps::scan(&self.dirs, &self.config, &self.pins);
        self.apps.retain(|a| !a.entry.terminal || terminal_ok);
        self.openers = apps::openers(&self.dirs, &self.config);
        self.openers.retain(|a| !a.entry.terminal || terminal_ok);
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
            count: self.count_for(a),
        }
    }

    /// The number on app `a`'s badge: the largest `count` its notices
    /// carry, else how many it has.
    fn count_for(&self, a: &App) -> u32 {
        let mine = self.notices.iter().filter(|n| a.answers_to(&n.app));
        let (most, n) = mine.fold((0, 0), |(m, n), x| (m.max(x.count), n + 1));
        if most > 0 { most } else { n }
    }

    fn refresh_status(&mut self) {
        let (clock, at) = rt::clock_text(self.config.clock_24h());
        self.clock = clock;
        self.clock_at = at;
        self.battery = self.battery_dir.as_deref().and_then(battery::sample);
        self.net = status::network(Path::new("/sys/class/net"), Path::new("/proc/net/wireless"));
        self.bluetooth = status::bluetooth();
        let before = self.low.warning();
        let w = self.low.update(self.battery, rt::now_ms());
        if w != before {
            if w != Warning::None {
                self.sound(Sound::Warn);
            }
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
            let before = self.low.warning();
            let w = self.low.update(r, now);
            if w != before && w != Warning::None {
                self.sound(Sound::Warn);
            }
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
            // Not while an app is about to come back: it would flash. The
            // unlock's slide draws the app's own last frame, so it may.
            let resuming =
                matches!(self.pending, Some(Pending::Resume(_))) && self.lock_slide.is_none();
            if !app && self.dirty && self.screen != Screen::Off && !resuming {
                self.draw();
            }
            let now = rt::now_ms();
            if self.input.finished(now) {
                self.tasks.close_all(now);
                self.services.stop();
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
                self.player.deadline(),
                (shell && self.screen == Screen::Switcher && self.next_frame(now).is_none())
                    .then_some(self.stats_at)
                    .flatten(),
                (shell && self.screen == Screen::Switcher)
                    .then(|| self.switcher.hold_at())
                    .flatten(),
                (shell && self.screen == Screen::Notices)
                    .then(|| self.panel.hold_at())
                    .flatten(),
                (shell && self.screen == Screen::Home)
                    .then(|| self.esc_hold.at())
                    .flatten(),
                (shell && self.screen == Screen::Home)
                    .then(|| self.power.as_ref().and_then(Power::hold_at))
                    .flatten(),
                self.dark_at(),
                self.dim_at(),
                (shell && self.screen == Screen::Locked)
                    .then(|| self.lock.next_at())
                    .flatten(),
                (shell && self.lock_waiting()).then_some(now + 1000),
                self.wifi_off_at(),
                self.keyring_lock_at(),
                (self.low.warning() == Warning::Countdown).then_some(now + 1000),
                self.input.next_at(),
                shell
                    .then(|| match self.paced {
                        true => Some(now),
                        false => self.next_frame(now),
                    })
                    .flatten(),
                app.then(|| self.esc.next_at()).flatten(),
                app.then(|| self.tab_hold.next_at()).flatten(),
                (shell
                    && self
                        .tasks
                        .iter()
                        .any(|t| t.state == tasks::State::Background && !t.quiet))
                .then_some(now + GUARD_MS),
                (app && [&self.banner, &self.popup]
                    .into_iter()
                    .flatten()
                    .any(Overlay::needs_refresh))
                .then_some(now + overlay::REFRESH_MS),
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
            let services_at = fds.len();
            fds.extend(self.services.fds().into_iter().map(rt::pollfd));
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
                            self.services.stop();
                            if self.bye.is_none() && !self.sim && system_stopping() {
                                self.bye = Some(false);
                            }
                            if let Some(restart) = self.bye {
                                eprintln!("gh0st shell: the system is going down");
                                self.draw_bye(restart);
                            }
                            return 0;
                        }
                    }
                }
            }
            let (mut apps_changed, mut status_changed, mut notices_changed) = (false, false, false);
            for (i, (wake, w)) in self.watches.iter().enumerate() {
                if rt::readable(&fds[2 + i]) && w.drain() {
                    match wake {
                        Wake::Apps => apps_changed = true,
                        Wake::Status => status_changed = true,
                        Wake::Notices => notices_changed = true,
                    }
                }
            }
            if notices_changed {
                self.reload_notices(true);
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
            if fds[services_at..].iter().any(rt::readable) {
                self.services_said();
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
                // Over an app the shell only listens: Tab reaches the app too.
                if self.app_front() && self.tab_hold.key(c, v, now, self.held.len() == 1) {
                    self.hide_banner(BannerOwner::Tab);
                }
                if let Some(k) = self.kb.event(c, v, now) {
                    self.on_key(k, now);
                }
            }
            self.on_timers(now);
            self.sweep(now);
            self.run_pending(now);
            self.maybe_prompt(now);
            self.maybe_portal(now);
            // A portal open sets what is pending: now, not at the next key.
            self.run_pending(now);
            self.tell_front();
        }
    }

    /// Routes every exited child: an app's leader, or a job.
    fn reap(&mut self) {
        for (pid, status) in child::reap() {
            if self.player.exited(pid, rt::now_ms()) {
                continue;
            }
            if let Some((w, gave_up)) = self.services.exited(pid, rt::now_ms()) {
                self.daemon_died(w);
                if gave_up {
                    self.daemon_gave_up(w);
                }
                continue;
            }
            let now = rt::now_ms();
            let early = self
                .tasks
                .iter()
                .find(|t| t.pgid == pid && t.sandboxed && now < t.started + EARLY_EXIT_MS)
                .filter(|_| libc::WIFEXITED(status) && libc::WEXITSTATUS(status) != 0)
                .map(|t| (t.id.clone(), t.name.clone()));
            self.tasks.exited(pid);
            if let Some((id, name)) = early {
                sandbox_hint(&id, &name);
            }
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
            if let Some(c) = &self.cgroups {
                c.remove(t.id.strip_suffix(".desktop").unwrap_or(&t.id));
            }
            if t.sound_parked {
                audio(self.audio_exe.as_deref(), "unpark", Some(t.pgid));
            }
            if self.pending == Some(Pending::Resume(t.id.clone())) {
                self.pending = None;
            }
            if self.back == Back::App(t.id.clone()) {
                self.back = Back::Home;
            }
        }
        for t in gone.iter().filter(|t| t.id.starts_with(PICKER)) {
            if let Some(i) = self.orphans.iter().position(|o| *o == t.id) {
                self.orphans.swap_remove(i);
                continue;
            }
            let id = &t.id[PICKER.len()..];
            self.services.say_session(&["gone", id]);
        }
        // A link in the chain whose opener left can't lead back.
        self.return_to
            .retain(|(_, opener)| !gone.iter().any(|t| t.id == *opener));
        if gone.iter().any(|t| t.state == tasks::State::Front) {
            if matches!(self.pending, Some(Pending::Pause | Pending::Lock)) {
                self.pending = None;
            }
            self.tab_hold.reset();
            // The driver would keep a held overlay over home.
            for o in [self.banner.take(), self.popup.take()]
                .into_iter()
                .flatten()
            {
                self.spare = Some(o.hide(self.display.as_mut()));
            }
            self.screen = Screen::Home;
            self.undim();
            self.take_screen();
            self.sound(Sound::Close);
            // An app opened over another goes back to it.
            let front_gone: Vec<&str> = gone
                .iter()
                .filter(|t| t.state == tasks::State::Front)
                .map(|t| t.id.as_str())
                .collect();
            if let Some(at) = self
                .return_to
                .iter()
                .rposition(|(opened, _)| front_gone.contains(&opened.as_str()))
            {
                let (_, opener) = self.return_to.remove(at);
                if self.tasks.get(&opener).is_some_and(|t| t.resumable()) {
                    self.pending = Some(Pending::Resume(opener));
                }
            }
        } else {
            let cards = self.cards();
            self.switcher.set_apps(cards);
            self.sync_grab();
            self.dirty = true;
        }
        // Gone from behind, it leads back nowhere: opened again from home,
        // it must not bring up its old opener.
        self.return_to
            .retain(|(opened, _)| !gone.iter().any(|t| t.id == *opened));
    }

    fn next_frame(&self, now: u64) -> Option<u64> {
        if let Some(s) = &self.lock_slide {
            return Some(s.anim.next_frame(now));
        }
        match self.screen {
            Screen::Home => self.home.next_frame(now),
            Screen::Switcher => self.switcher.next_frame(now),
            _ => None,
        }
    }

    fn on_timers(&mut self, now: u64) {
        self.player.tick(now);
        if let Some(k) = self.kb.repeat_due(now) {
            self.on_key(k, now);
        }
        if self.dim_at().is_some_and(|t| now >= t) {
            self.dim();
        }
        if self.wifi_off_at().is_some_and(|t| now >= t) {
            self.wifi_sleep();
        }
        if self.keyring_lock_at().is_some_and(|t| now >= t) {
            self.services.tell(&ToDaemon::Lock);
            self.keyring_open = false;
            self.unlock_sent = false;
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
            self.toast_notice = None;
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
        if self.screen == Screen::Switcher {
            self.close_all_tick(now);
        }
        if self.screen == Screen::Notices {
            self.clear_all_tick();
        }
        if self.screen == Screen::Home {
            self.power_tick(now);
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
                // The warning is this wake's sound.
                self.chime_at_wake = false;
                self.sound(Sound::Warn);
            }
            self.dirty = true;
            if self.low.take_shutdown(now) {
                self.power_now(false);
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
        let tab = self.tab_hold.update(now);
        let battery = self.banner_owner == Some(BannerOwner::Battery) && self.banner.is_some();
        let info = gh0st_ui::theme::Tone::Info;
        if tab.hint && !battery {
            self.show_banner(BannerOwner::Tab, "Hold Tab to lock", info);
        }
        if tab.lock {
            if !battery {
                self.show_banner(BannerOwner::Tab, "Let go of Tab to lock", info);
            }
            self.pending = Some(Pending::Lock);
        }
        let d = self.esc.update(now, self.esc_down);
        if d.hide_hint {
            self.hide_banner(BannerOwner::Esc);
        }
        if d.show_hint && !battery {
            let warn = gh0st_ui::theme::Tone::Warning;
            self.show_banner(BannerOwner::Esc, "Hold Esc to close the app", warn);
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
                "Battery low: shutting down in {} s",
                self.low.seconds_left(now)
            );
            let critical = gh0st_ui::theme::Tone::Critical;
            self.show_banner(BannerOwner::Battery, &text, critical);
            if self.low.take_shutdown(now) {
                self.power_now(false);
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

    /// The banner over an app, changed in place. The same words again send
    /// nothing: the battery countdown asks on every wake.
    fn show_banner(&mut self, owner: BannerOwner, text: &str, tone: gh0st_ui::theme::Tone) {
        let same = self
            .banner_text
            .as_ref()
            .is_some_and(|(s, k)| s == text && *k == tone);
        self.banner_owner = Some(owner);
        if same && self.banner.is_some() {
            return;
        }
        self.banner_text = Some((text.to_string(), tone));
        let t = self.t;
        let paint = |c: &mut Canvas| widgets::banner(c, &t, text, tone);
        if let Some(b) = &mut self.banner {
            b.update(self.display.as_mut(), paint);
            return;
        }
        let under = self.canvas_for_overlay();
        let o = Overlay::show(self.display.as_mut(), overlay::BANNER, under, paint);
        self.log_overlay("banner", &o);
        self.banner = Some(o);
    }

    /// Takes the banner down if `owner` put it up.
    fn hide_banner(&mut self, owner: BannerOwner) {
        if self.banner_owner != Some(owner) {
            return;
        }
        self.banner_owner = None;
        if let Some(b) = self.banner.take() {
            self.spare = Some(b.hide(self.display.as_mut()));
        }
    }

    fn log_overlay(&self, what: &str, o: &Overlay) {
        if self.debug {
            let how = if o.needs_refresh() {
                "drawn"
            } else {
                "held by the driver"
            };
            eprintln!("gh0st shell: {what} {how}");
        }
    }

    /// A toast over an app, changed in place if one is up.
    fn set_popup(&mut self, paint: impl Fn(&mut Canvas) -> Rect) {
        if let Some(p) = &mut self.popup {
            p.update(self.display.as_mut(), paint);
            return;
        }
        let under = self.canvas_for_overlay();
        let o = Overlay::show(self.display.as_mut(), overlay::POPUP, under, paint);
        self.log_overlay("popup", &o);
        self.popup = Some(o);
    }

    fn show_popup(&mut self, text: &str) {
        let t = self.t;
        self.set_popup(|c| {
            let bottom = t.m.height - t.m.hint_h;
            widgets::toast(c, &t, bottom, gh0st_ui::theme::Tone::Info, text, None)
        });
    }

    /// When the screen goes dark if no key comes first. Never while an
    /// app in front asked to stay awake; sooner on the lock screen.
    fn dark_at(&self) -> Option<u64> {
        if self.screen == Screen::Off && !self.app_front() {
            return None;
        }
        if !self.held.is_empty() {
            return None;
        }
        let d = self.dark_ms?;
        let d = match self.tasks.front() {
            Some(t) if t.awake || t.inhibited || t.keeps_on => return None,
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
        let raw = bl.level();
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
            // The question waits; the app it parked comes back first.
            Screen::Prompt => {
                self.unlock_to = self.prompt_app.take().map_or(Back::Home, Back::App);
            }
            _ => {}
        }
        // From the lock screen, the lock and where it goes back to stay.
        if self.screen != Screen::Locked {
            self.dark_since = Some(now);
            self.wake_to = Some(self.screen);
        }
        self.go_dark();
    }

    /// Dark over an app: the shell takes the keyboard, then parks the app
    /// as Fn+K does. An app that holds the keyboard itself or can't be
    /// paused keeps the screen on: the shell can't see its keys or stop
    /// its drawing.
    fn dark_over_app(&mut self, now: u64) {
        if !self.input.grab(true) {
            self.keep_front_on(now);
            return;
        }
        match self.park_front(now) {
            Ok(id) => {
                // Locked first, so take_screen keeps the keyboard.
                self.screen = Screen::Locked;
                self.take_screen();
                self.unlock_to = Back::App(id);
                self.wake_to = None;
                self.dark_since = Some(now);
                self.go_dark();
            }
            Err(_) => {
                self.input.grab(false);
                self.keep_front_on(now);
            }
        }
    }

    /// Tab held over an app: the app is parked and the lock screen asks
    /// for the password, as a lock from home does. The unlock brings the
    /// app back. An app that grabbed the keyboard never lets Tab reach the
    /// shell, so it never gets here.
    fn lock_over_app(&mut self, now: u64) {
        if !self.input.grab(true) {
            self.hide_banner(BannerOwner::Tab);
            self.toast("Can't lock now");
            return;
        }
        match self.park_front(now) {
            Ok(id) => {
                // Locked first, so take_screen keeps the keyboard.
                self.screen = Screen::Locked;
                self.take_screen();
                self.unlock_to = Back::App(id);
                self.lock_secure = true;
                self.sound(Sound::Lock);
                self.enter_locked(now);
            }
            Err(name) => {
                self.input.grab(false);
                self.hide_banner(BannerOwner::Tab);
                if let Some(name) = name {
                    self.toast(format!("{name} can't be paused"));
                }
            }
        }
    }

    fn keep_front_on(&mut self, now: u64) {
        if let Some(t) = self.tasks.front_mut() {
            t.keeps_on = true;
        }
        self.last_input = now;
        self.undim();
    }

    /// The bye screen, lit and over any app or overlay.
    fn draw_bye(&mut self, restart: bool) {
        let t = self.t;
        self.display.resume();
        self.display.overlay_clear(0);
        self.display.overlay_clear(1);
        power::bye(&mut self.canvas, &t, restart, self.ask_switch);
        self.display.flush_all(&self.canvas);
        if let Some(bl) = &self.backlight {
            if bl.get().is_none_or(|r| r <= 1) || self.dimmed.is_some() {
                let raw = self.saved_raw.or(self.dimmed).unwrap_or(bl.max);
                let _ = bl.set(raw);
            }
        }
    }

    fn go_dark(&mut self) {
        if let Some(bl) = &self.backlight {
            let raw = self.dimmed.take().or_else(|| bl.get());
            self.saved_raw = raw.filter(|&r| r > 1);
            let _ = bl.off();
        }
        // The backlight never goes fully out (see Backlight::off), so the
        // panel shows black, not the last frame.
        self.canvas
            .fill(self.canvas.bounds(), gh0st_ui::theme::PANEL_OFF);
        self.display.flush_all(&self.canvas);
        self.full_flush = true;
        self.lock.rest();
        self.home_menu = None;
        self.power = None;
        self.lock_slide = None;
        self.esc_hold.cancel();
        self.switcher.cancel_hold();
        self.tab_down = None;
        self.pending = None;
        self.screen = Screen::Off;
        self.off_at = rt::now_ms();
        self.player.close(self.off_at);
        self.wifi_done = false;
        self.sync_grab();
    }

    /// When Wi-Fi goes off in this dark, if the user asked for that.
    /// The dark has lasted Lock after: the keyring closes now, not at the
    /// key that wakes the lock screen.
    fn keyring_lock_at(&self) -> Option<u64> {
        if !self.keyring_open || self.screen != Screen::Off {
            return None;
        }
        Some(self.dark_since? + self.lock_after_ms?)
    }

    fn wifi_off_at(&self) -> Option<u64> {
        let dark = self.screen == Screen::Off && !self.app_front();
        (dark && !self.wifi_done)
            .then(|| self.dark_wifi_ms.map(|ms| self.off_at + ms))
            .flatten()
    }

    /// Wi-Fi off through NetworkManager, which our polkit rule allows. Only
    /// a radio that is on, so the wake never turns on one the user left off.
    /// Never the hotspot: it is meant to work with the device in a pocket.
    fn wifi_sleep(&mut self) {
        self.wifi_done = true;
        if self.backlight.is_none()
            || !rfkill::wlan_on(Path::new("/sys/class/rfkill"))
            || status::hotspot()
        {
            return;
        }
        if self.debug {
            eprintln!("gh0st shell: Wi-Fi off for the dark");
        }
        let _ = std::fs::write(wifi_marker(), b"");
        child::fire(&["nmcli", "radio", "wifi", "off"]);
        self.wifi_slept = true;
    }

    fn wifi_wake(&mut self) {
        if !std::mem::take(&mut self.wifi_slept) {
            return;
        }
        let _ = std::fs::remove_file(wifi_marker());
        child::fire(&["nmcli", "radio", "wifi", "on"]);
    }

    /// A key in the dark: the lock screen at rest once Lock after has
    /// passed, else straight back to where it was. True if it locked.
    fn wake(&mut self, now: u64) -> bool {
        let locked = lock_due(self.lock_secure, self.dark_since, self.lock_after_ms, now);
        if locked {
            self.screen_on(now);
        } else {
            self.light();
            self.back_from_dark();
            self.refresh_status();
        }
        self.wake_sounds(now);
        locked
    }

    /// A notice that came in the dark chimes now. Else the stream opens
    /// ahead of the first key: after a dark the amplifier is off, and it
    /// takes longer to start than a key sound lasts.
    fn wake_sounds(&mut self, now: u64) {
        if std::mem::take(&mut self.chime_at_wake) {
            self.sound(Sound::Notice);
        } else if self.config.sounds() && !self.quiet() {
            self.player.warm(now);
        }
    }

    /// Lit on the lock screen at rest: a wake past Lock after, or the low
    /// battery countdown, which no key asked for.
    fn screen_on(&mut self, now: u64) {
        self.light();
        self.enter_locked(now);
        self.refresh_status();
    }

    /// Back from a dark shorter than Lock after, with no lock screen.
    fn back_from_dark(&mut self) {
        if self.wake_to.take() == Some(Screen::Switcher) {
            self.lock.rest();
            self.lock_secure = false;
            self.dark_since = None;
            self.screen = Screen::Switcher;
            self.full_flush = true;
            self.dirty = true;
            self.sync_grab();
            return;
        }
        // Until the app is back, stray keys land on the lock screen, which
        // is not drawn meanwhile.
        self.screen = Screen::Locked;
        self.unlock();
        if let Some(Pending::Resume(id)) = &self.pending {
            if let Some(px) = self.tasks.get(id).and_then(|t| t.pixels.as_ref()) {
                self.display.flush_all(px);
            }
        }
    }

    fn light(&mut self) {
        self.wifi_wake();
        if let Some(bl) = &self.backlight {
            // The panel may have been dark already (stock's timeout), so
            // fall back to the saved level, then full.
            let raw = self
                .saved_raw
                .take()
                .or_else(|| {
                    self.config
                        .get_int(config::key::BRIGHTNESS)
                        .filter(|&r| r > 0)
                        .map(|r| r as u32)
                })
                .unwrap_or(bl.max);
            let _ = bl.set(raw);
        }
    }

    fn enter_locked(&mut self, now: u64) {
        if self.debug {
            eprintln!("gh0st shell: locked");
        }
        // Nothing queued before the lock may run past it.
        self.pending = None;
        self.services.tell(&ToDaemon::Lock);
        // What was copied may be a password: it goes with the lock, unless
        // Settings keeps it.
        if self.config.get_bool(config::key::CLIPBOARD_KEEP) != Some(true) {
            self.services.say_session(&["clipboard-clear"]);
        }
        self.keyring_open = false;
        self.unlock_sent = false;
        self.screen = Screen::Locked;
        self.lock.rest();
        self.probe_retried = false;
        self.home_menu = None;
        self.power = None;
        self.lock_slide = None;
        self.esc_hold.cancel();
        self.switcher.cancel_hold();
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
        self.sound(Sound::Lock);
        self.enter_locked(now);
    }

    fn unlock(&mut self) {
        self.lock.rest();
        self.lock_secure = false;
        self.dark_since = None;
        self.wake_to = None;
        self.dirty = true;
        if self.debug {
            eprintln!("gh0st shell: unlocked");
        }
        self.maybe_ask_open();
        let mut slide = self.config.animations().then(|| LockSlide {
            anim: crate::motion::Anim {
                start: rt::now_ms(),
                ms: crate::motion::UNLOCK_MS,
                motion: (),
            },
            lock: self.canvas.clone(),
            app: None,
        });
        match std::mem::replace(&mut self.unlock_to, Back::Home) {
            // The lock screen stays until the app is back, so home never
            // flashes in between.
            Back::App(id) if self.tasks.get(&id).is_some_and(|t| t.resumable()) => {
                if let Some(s) = &mut slide {
                    s.app = Some(id.clone());
                }
                self.pending = Some(Pending::Resume(id));
            }
            _ => {
                self.screen = Screen::Home;
                self.sync_grab();
            }
        }
        self.lock_slide = slide;
    }

    /// Mid-slide: what the unlock goes to, with the lock screen's last
    /// frame moved up over it. Under an app that is the app's last frame,
    /// which `resume` puts back once the slide is over, so nothing jumps.
    fn draw_lock_slide(&mut self, now: u64) {
        let Some(s) = &self.lock_slide else {
            return;
        };
        let app = s.app.as_ref().and_then(|id| self.tasks.get(id));
        if let Some(px) = app.and_then(|t| t.pixels.as_ref()) {
            for y in 0..self.canvas.height().min(px.height()) {
                self.canvas.copy_row_from(px, y, px.width());
            }
        }
        let h = self.canvas.height().min(s.lock.height());
        let gone = (h as f32 * s.anim.progress(now)).round() as i32;
        for y in 0..h - gone {
            let w = self.canvas.width().min(s.lock.width()) as usize;
            self.canvas.row_mut(y)[..w].copy_from_slice(&s.lock.row(y + gone)[..w]);
        }
    }

    fn wants_password(&self, now: u64) -> bool {
        asks_password(
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

    /// A lock or a dark that was not passed yet. Unlike [`Self::locked`],
    /// false once `unlock` ran, while the lock screen waits for the app.
    fn lock_holds(&self) -> bool {
        !self.app_front() && (self.lock_secure || self.dark_since.is_some())
    }

    /// Runs `unix_chkpwd` on what was typed. The try counts before the
    /// check, so a restart in between can't skip the wait.
    fn check_password(&mut self, typed: Vec<u8>) {
        self.tries.fails += 1;
        self.tries.last = password::boottime();
        self.save_tries();
        let input = password::chkpwd_input(&typed);
        self.check_retried = false;
        self.run_check(input);
        self.try_vault(typed);
    }

    /// The vault's Argon2 runs while `unix_chkpwd` does, not after it. The
    /// daemon opens an existing vault only, and only on its own tag.
    fn try_vault(&mut self, pw: Vec<u8>) {
        let mut m = ToDaemon::Try { pw };
        if !self.keyring_open {
            self.services.tell(&m);
        }
        m.wipe();
    }

    /// Starts `unix_chkpwd` on `input`, once more if it won't start. If it
    /// still won't, the user is let in this once: no checker means no way
    /// in, and a lockout means a reflash.
    fn run_check(&mut self, mut input: Vec<u8>) {
        let prog = self.test_prog("GH0ST_CHKPWD", password::CHKPWD);
        let user = self.user.clone();
        if self.start_job_with(JobKind::Chkpwd, &[&prog, &user, "nonull"], Some(&input)) {
            self.check_pid = self.jobs.last().map_or(0, |(_, j)| j.pid);
            if let Some(mut old) = self.check_input.replace(input) {
                password::wipe(&mut old);
            }
            return;
        }
        if !self.check_retried {
            self.check_retried = true;
            return self.run_check(input);
        }
        password::wipe(&mut input);
        self.tries = Tries::default();
        self.save_tries();
        self.unlock();
    }

    /// Messages from the app in group `pgid`.
    fn read_link(&mut self, pgid: i32) {
        let mut toasts = Vec::new();
        let mut slept = false;
        let mut open = None;
        let mut sounds = Vec::new();
        let mut open_files: Vec<(Option<String>, PathBuf)> = Vec::new();
        let mut leave = false;
        if let Some(t) = self.tasks.iter_mut().find(|t| t.pgid == pgid) {
            let front = t.state == tasks::State::Front;
            while let Some(m) = t.link.as_ref().and_then(|l| l.recv()) {
                match m.first().map(String::as_str) {
                    Some("hello") => {
                        t.linked = true;
                        t.link_v = m.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                    }
                    Some("toast") => toasts.extend(m.get(1).cloned()),
                    // Only the app in front may bring another up.
                    Some("open") if front => open = m.get(1).cloned(),
                    Some("sound") if front => {
                        sounds.extend(m.get(1).and_then(|n| Sound::from_name(n)))
                    }
                    Some("open-file") if front => {
                        open_files.extend(m.get(1).map(|p| (None, link::path_of(p))));
                    }
                    Some("leave") if front => leave = true,
                    Some("open-with") if front => {
                        if let (Some(app), Some(p)) = (m.get(1), m.get(2)) {
                            open_files.push((Some(app.clone()), link::path_of(p)));
                        }
                    }
                    Some("awake") => {
                        let on = m.get(1).is_some_and(|v| v == "1");
                        slept |= t.awake && !on;
                        t.awake = on;
                    }
                    _ => {}
                }
            }
            // The app is gone but its group may live on: stop polling a
            // socket that will only ever say so.
            if t.link.as_ref().is_some_and(|l| l.closed()) {
                t.link = None;
                t.awake = false;
            }
        }
        // The full timeout again from here, not at once.
        if slept {
            self.last_input = rt::now_ms();
        }
        // Asked mid-dim: back to full.
        if self.tasks.front().is_some_and(|t| t.awake) {
            self.undim();
        }
        if let Some(name) = open {
            self.open_request(&name);
        }
        for s in sounds {
            self.app_sound(s);
        }
        for (app, path) in open_files {
            self.open_file(app.as_deref(), path);
        }
        if leave && self.pending.is_none() {
            self.pending = Some(Pending::Leave);
        }
        // What an app says stays private while the device is locked.
        if self.locked() {
            return;
        }
        for text in toasts {
            self.toast(text);
        }
    }

    /// An app in front asked, over the link, for app `name`.
    fn open_request(&mut self, name: &str) {
        let Some(app) = self.apps.iter().find(|a| a.answers_to(name)) else {
            self.toast(format!("Can't find {name}"));
            return;
        };
        let front = self.tasks.front().map(|t| t.id.clone());
        if front.is_some() && front.as_deref() != Some(app.file.as_str()) && self.pending.is_none()
        {
            self.pending = Some(Pending::Open(app.file.clone(), Vec::new()));
        }
    }

    /// The app in front asked to open `path`, in `app` or in the app its
    /// type opens with. A running target that speaks link version 2 gets
    /// the file; one that doesn't is left alone, so nothing it holds is
    /// lost.
    fn open_file(&mut self, app: Option<&str>, path: PathBuf) -> bool {
        // Relative, it would resolve against the shell's folder, and one
        // that starts with `-` would reach the target as an option.
        if !path.is_absolute() {
            self.toast("Can't open that file");
            return false;
        }
        let target = match app {
            Some(name) => match self.openers.iter().find(|a| a.answers_to(name)) {
                Some(_) if !path.exists() => {
                    self.toast("That file is gone");
                    return false;
                }
                Some(a) => a.clone(),
                None => {
                    self.toast(format!("Can't find {name}"));
                    return false;
                }
            },
            None => {
                let Ok(mime) = gh0st_core::mime::of_path(&path) else {
                    self.toast("That file is gone");
                    return false;
                };
                let list = handlers::MimeApps::load(&handlers::list_path());
                match handlers::default_for(&self.openers, &list, mime) {
                    Some(a) => a.clone(),
                    None => {
                        self.toast("No app opens this file");
                        return false;
                    }
                }
            }
        };
        // Started a moment ago, its hello may wait unread.
        if let Some(pgid) = self
            .tasks
            .get(&target.file)
            .filter(|t| !t.linked && t.link.is_some())
            .map(|t| t.pgid)
        {
            self.read_link(pgid);
        }
        let running = self.tasks.get(&target.file);
        if running.is_some_and(|t| t.link_v < 2) {
            self.toast(format!("Close {} to open this file", target.entry.name));
            return false;
        }
        let front = self.tasks.front().map(|t| t.id.clone());
        if front.as_deref() == Some(target.file.as_str()) {
            // The app in front opens it itself.
            return self.hand_files(&target.file, &[path]);
        }
        // Another handover waits for keys: one more file for the same app
        // joins it, anything else waits for nothing.
        match &mut self.pending {
            None => self.pending = Some(Pending::Open(target.file.clone(), vec![path])),
            Some(Pending::Open(id, files)) if *id == target.file => files.push(path),
            Some(_) => return false,
        }
        true
    }

    /// Sends `open` for each file to a running app on link version 2.
    /// False, with a toast, if it didn't take them.
    fn hand_files(&mut self, id: &str, files: &[PathBuf]) -> bool {
        if files.is_empty() {
            return true;
        }
        let Some(t) = self.tasks.get(id) else {
            return false;
        };
        let name = t.name.clone();
        let sent = t.link.as_ref().filter(|_| t.link_v >= 2).is_some_and(|l| {
            files
                .iter()
                .all(|f| l.send(&["open", &link::path_field(f)]))
        });
        if !sent {
            self.toast(format!("{name} didn't take the file"));
        }
        sent
    }

    /// Pauses the app in front and brings up app `id` over it, opening
    /// `files`; when `id` closes, the first comes back.
    fn open_over_front(&mut self, id: &str, files: &[PathBuf], now: u64) {
        let app = self
            .apps
            .iter()
            .chain(&self.openers)
            .find(|a| a.file == id)
            .cloned();
        let Some(app) = app else {
            return;
        };
        // From home or the notifications, nothing is parked and nothing
        // comes back after.
        let opener = if self.tasks.front().is_some() {
            match self.park_front(now) {
                Ok(o) => Some(o),
                Err(Some(name)) => return self.toast(format!("{name} can't be paused")),
                Err(None) => return,
            }
        } else {
            None
        };
        self.return_to.retain(|(opened, _)| opened != id);
        if let Some(o) = &opener {
            self.return_to.push((app.file.clone(), o.clone()));
        }
        match self.tasks.get(id).map(|t| t.state) {
            Some(tasks::State::Closing { .. }) => {
                if opener.is_some() {
                    self.return_to.pop();
                }
                self.take_screen();
                self.toast(format!("{} is closing", app.entry.name));
            }
            Some(_) => {
                self.resume(id, now);
                self.hand_files(id, files);
            }
            None => self.launch(&app, files),
        }
    }

    /// Watches an app's own notice folder too.
    fn watch_notices(&self, dir: &Path) {
        #[cfg(target_os = "linux")]
        for (wake, w) in &self.watches {
            if *wake == Wake::Notices {
                w.add(dir);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = dir;
    }

    /// Reads the notice folders again. With `announce`, a notice not seen
    /// before shows as a toast, unless the screen is dark or locked.
    fn reload_notices(&mut self, announce: bool) {
        let mut all = notify::read_dir(&notify::system_dir(), true);
        all.extend(notify::read_dir(&notify::user_dir(), false));
        all.extend(notify::read_app_dirs(&notify::user_dir()));
        self.dismissed.keep_only(&all);
        all.retain(|n| !(n.system && self.dismissed.has(n)));
        all.sort_by_key(|n| std::cmp::Reverse(n.time));
        let fresh = all
            .iter()
            .find(|n| {
                !self
                    .notices_known
                    .iter()
                    .any(|(id, t)| *id == n.id() && *t == n.time)
            })
            .cloned();
        self.notices_known = all.iter().map(|n| (n.id(), n.time)).collect();
        self.notices = all;
        self.sync_notices();
        if let Some(n) = fresh.filter(|_| announce) {
            // The dark makes no sound: the chime waits for the wake. The
            // lock screen chimes but shows nothing.
            if self.screen == Screen::Off {
                self.chime_at_wake = true;
            } else {
                self.sound(Sound::Notice);
                if !self.locked() {
                    self.announce(&n);
                }
            }
        }
    }

    /// The app a notice belongs to, as home shows it: name and picture.
    fn notice_row(&mut self, n: &Notice) -> notices::Row {
        let app = self.apps.iter().find(|a| a.answers_to(&n.app)).cloned();
        match app {
            Some(a) => {
                let e = self.entry_for(&a);
                notices::Row {
                    notice: n.clone(),
                    name: e.name,
                    icon: e.icon,
                }
            }
            None => notices::Row {
                notice: n.clone(),
                name: n.app.clone(),
                icon: Icon::Letter { tint: n.app.len() },
            },
        }
    }

    /// The panel's rows, the tiles' numbers and the switcher's chip follow
    /// the notices.
    fn sync_notices(&mut self) {
        let list = self.notices.clone();
        let rows = list.iter().map(|n| self.notice_row(n)).collect();
        self.panel.set(rows);
        let counts: Vec<(String, u32)> = self
            .apps
            .iter()
            .map(|a| (a.file.clone(), self.count_for(a)))
            .collect();
        self.home
            .set_counts(|id| counts.iter().find(|(f, _)| f == id).map_or(0, |(_, n)| *n));
        self.switcher.set_notices(list.len());
        self.dirty = true;
    }

    /// A new notice: a toast with its app's picture, over an app too.
    fn announce(&mut self, n: &Notice) {
        let row = self.notice_row(n);
        if self.app_front() {
            let t = self.t;
            let v = home::visual(&t, &row.icon, &row.name, Size::Tiny);
            let (name, title) = (row.name.clone(), row.notice.title.clone());
            self.set_popup(|c| {
                let bottom = t.m.height - t.m.hint_h;
                widgets::notice_toast(c, &t, bottom, &v, &name, &title)
            });
        }
        self.level = None;
        self.toast = Some((String::new(), rt::now_ms() + NOTICE_TOAST_MS));
        self.toast_notice = Some(row);
        self.dirty = true;
    }

    fn open_notices(&mut self, back: Screen) {
        self.panel_back = back;
        self.panel.reset();
        self.switcher.cancel_hold();
        self.screen = Screen::Notices;
        self.dirty = true;
    }

    fn notices_back(&mut self) {
        self.panel.cancel_hold();
        self.screen = self.panel_back;
        self.dirty = true;
    }

    /// Clears notice `n`: an app's file goes; a system notice is remembered
    /// as dismissed until it is posted anew.
    fn dismiss(&mut self, n: &Notice) {
        // A poster on the bus hears that the user cleared it.
        if n.via_bus {
            self.services.say_session(&["closed", &n.id()]);
        }
        self.drop_notice(n);
    }

    /// Takes notice `n` off the list and its file away.
    fn drop_notice(&mut self, n: &Notice) {
        if n.system {
            self.dismissed.add(n);
            if let Err(e) = self.dismissed.save(&notify::dismissed_path()) {
                eprintln!("gh0st shell: notices: {e}");
            }
        } else if let Some(p) = &n.from {
            match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    eprintln!("gh0st shell: notices: {e}")
                }
                _ => {}
            }
        } else if let Err(e) = notify::remove(&notify::user_dir(), &n.app, &n.tag) {
            eprintln!("gh0st shell: notices: {e}");
        }
        self.notices
            .retain(|m| m.id() != n.id() || m.system != n.system);
        self.sync_notices();
    }

    /// Action `id` of notice `n`: to its poster on the bus, or to its app
    /// over the link if it runs. Nothing comes to the front for it.
    fn notice_action(&mut self, n: &Notice, id: &str) -> bool {
        if n.via_bus {
            return self.services.say_session(&["action", &n.id(), id]);
        }
        let task = self
            .apps
            .iter()
            .chain(&self.openers)
            .find(|a| a.answers_to(&n.app))
            .map(|a| a.file.clone());
        let Some(t) = task.and_then(|f| self.tasks.get(&f)) else {
            return false;
        };
        t.link_v >= 2
            && t.link
                .as_ref()
                .is_some_and(|l| l.send(&["action", &n.tag, id]))
    }

    /// Enter on a notice: its default action runs, then its file opens in
    /// its app, or its app comes up.
    fn open_notice(&mut self) {
        let Some(n) = self.panel.selected().map(|r| r.notice.clone()) else {
            return;
        };
        if !n.default.is_empty() {
            self.notice_action(&n, &n.default.clone());
        }
        if let Some(f) = n.file.clone() {
            self.drop_notice(&n);
            self.open_file(None, f);
            return;
        }
        let Some(app) = self.apps.iter().find(|a| a.answers_to(n.target())).cloned() else {
            if n.default.is_empty() {
                self.toast("No app opens this");
            } else {
                self.drop_notice(&n);
            }
            return;
        };
        self.drop_notice(&n);
        match self.tasks.get(&app.file).map(|t| t.state) {
            Some(tasks::State::Closing { .. }) => {
                self.toast(format!("{} is closing", app.entry.name))
            }
            Some(_) => self.pending = Some(Pending::Resume(app.file.clone())),
            None => self.launch(&app, &[]),
        }
    }

    /// The hold on 4 fills, and at its end clears every notice, if its key
    /// is still down.
    fn clear_all_tick(&mut self) {
        match self.panel.hold_tick(rt::now_ms()) {
            HoldTick::Idle => {}
            HoldTick::Fill => self.dirty = true,
            HoldTick::Done(key) => {
                self.dirty = true;
                if !self.held.contains(&key) {
                    return;
                }
                for n in self.notices.clone() {
                    self.dismiss(&n);
                }
            }
        }
    }

    fn notices_key(&mut self, k: Key, now: u64) {
        if k.state == State::Release && self.panel.let_go(k.code) {
            self.toast("Hold to clear all");
            return;
        }
        if k.state == State::Press && self.panel.holding() {
            self.panel.cancel_hold();
            self.dirty = true;
        }
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
                self.open_notice();
            }
            return;
        }
        if k.is(code::ESC) {
            if armed(&mut self.esc_armed) {
                self.notices_back();
            }
            return;
        }
        if k.is(code::HOME) {
            if armed(&mut self.home_armed) {
                self.notices_back();
            }
            return;
        }
        if k.state == State::Release {
            return;
        }
        match k.nav(true) {
            Some(keys::Nav::Up) => self.panel.move_by(-1),
            Some(keys::Nav::Down) => self.panel.move_by(1),
            Some(_) => return,
            None if k.state != State::Press => return,
            None if k.is(code::BACKSPACE) || k.ch == Some('8') => {
                if let Some(n) = self.panel.selected().map(|r| r.notice.clone()) {
                    self.dismiss(&n);
                }
            }
            None if k.ch == Some('4') => self.panel.hold_all(k.code, now),
            None if matches!(k.ch, Some('5' | '6' | '7')) => {
                let Some(n) = self.panel.selected().map(|r| r.notice.clone()) else {
                    return;
                };
                let at = (k.ch.map_or(0, |c| c as u32) - '5' as u32) as usize;
                let id = notices::action_keys(&n)[at].map(String::from);
                if let Some(id) = id {
                    let label = n.action(&id).unwrap_or_default().to_string();
                    if self.notice_action(&n, &id) {
                        self.toast(label);
                    } else {
                        self.toast("That app isn't running");
                    }
                }
            }
            None => return,
        }
        self.dirty = true;
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
                count: (*act == MenuAct::Notices && !value.is_empty()).then_some(value.as_str()),
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
        let pw_on = self.lock_password;
        let wait_note = format!("Try again in {wait} s");
        // Only how many: what they say stays behind the lock.
        let waiting = match self.notices.len() {
            0 => String::new(),
            1 => "1 notification".into(),
            n => format!("{n} notifications"),
        };
        let no_password = !self.lock.field_open()
            && pw_on
            && !self.probing
            && self.account.is_some_and(|a| a != Account::Password);
        let note = if !self.lock.field_open() {
            if no_password {
                Some((Tone::Info, "No password set"))
            } else {
                (!waiting.is_empty()).then_some((Tone::Info, waiting.as_str()))
            }
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
            bell: !self.lock.field_open() && !no_password && !waiting.is_empty(),
        };
        widgets::lock_screen(&mut self.canvas, &t, &bar, &l);
    }

    /// The volume or brightness level in the top bar's place, over an app
    /// too, for as long as a toast.
    fn show_level(&mut self, icon: &'static str, label: &'static str, pct: u8) {
        if self.app_front() {
            let t = self.t;
            self.set_popup(|c| widgets::level_bar(c, &t, icon, label, pct));
        }
        self.level = Some((icon, label, pct));
        self.toast = Some((String::new(), rt::now_ms() + TOAST_MS));
        self.dirty = true;
    }

    /// A system sound of the shell's, while the user has them on and could
    /// hear one.
    fn sound(&mut self, s: Sound) {
        if self.config.sounds() {
            self.app_sound(s);
        }
    }

    /// A sound an app asked for: the app checked the setting, which the
    /// shell reads again only once the app leaves the front.
    fn app_sound(&mut self, s: Sound) {
        if !self.quiet() && self.screen != Screen::Off {
            self.player.play(s, rt::now_ms());
        }
    }

    /// Muted, or the volume at nothing.
    fn quiet(&self) -> bool {
        self.muted == Some(true) || self.volume == Some(0)
    }

    fn toast(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.debug {
            eprintln!("gh0st shell: toast {text}");
        }
        self.level = None;
        self.toast_notice = None;
        if self.app_front() {
            self.show_popup(&text);
        }
        self.toast = Some((text, rt::now_ms() + TOAST_MS));
        self.dirty = true;
    }

    /// Saves what the panel shows. Over an app, without the shell's overlays.
    fn screenshot(&mut self) -> std::io::Result<PathBuf> {
        save_screenshot(&self.grab())
    }

    /// What the panel shows. Over an app, without the shell's overlays.
    fn grab(&mut self) -> Canvas {
        if !self.app_front() {
            return self.canvas.clone();
        }
        let mut shot = Canvas::new(self.t.m.width, self.t.m.height);
        self.display.read_into(&mut shot);
        for o in [&self.banner, &self.popup].into_iter().flatten() {
            o.patch_under(&mut shot);
        }
        shot
    }

    /// An app asks for a screenshot (`quiet`: the user said Always) or a
    /// colour from the screen. The screen is frozen now, as the app left
    /// it, and the question shows over it.
    fn ask_screen(&mut self, verb: &'static str, id: &str, pid: i32, app: &str, quiet: bool) {
        let Ok(n) = id.parse::<u64>() else {
            return;
        };
        let Some(app) = self.front_asker(pid, app) else {
            self.services.say_session(&[verb, id, "deny"]);
            return;
        };
        if self.shot.len() >= FROZEN_MAX {
            self.services.say_session(&[verb, id, "deny"]);
            return;
        }
        let shot = self.grab();
        if quiet {
            self.shot.push((n, shot));
            return self.answer_portal(n, verb, Answer::Once);
        }
        let m = self.t.m;
        let ground = if verb == "color" {
            // Colours as they are: no dimming, no frost.
            let px = shot.pixels().to_vec();
            let b = Rc::new(gh0st_ui::canvas::Backdrop {
                frost: Rc::new(px.clone()),
                back: px,
            });
            wallpaper::Baked {
                home: b.clone(),
                lock: b,
            }
        } else {
            let b = wallpaper::frozen(&shot, &self.t.c, m.bar_h as usize, m.hint_h as usize);
            wallpaper::Baked {
                home: b.clone(),
                lock: b,
            }
        };
        self.preview.push((n, ground));
        self.shot.push((n, shot));
        let kind = match verb {
            "color" => Kind::PickColor,
            _ => Kind::Approve {
                app,
                what: String::new(),
            },
        };
        self.prompts.push(Ask {
            origin: Origin::Portal(n, verb),
            kind,
        });
        self.light_for_question();
    }

    fn on_key(&mut self, k: Key, now: u64) {
        if self.bye.is_some() {
            return;
        }
        // The key that woke the screen or closed a banner does nothing
        // more: not its repeats, not its release.
        if k.state != State::Press && self.swallow_release == Some(k.code) {
            if k.state == State::Release {
                self.swallow_release = None;
            }
            return;
        }
        // A key ends the slide at once. Under an app the app is not back
        // yet, so the key goes nowhere.
        if k.state == State::Press {
            if let Some(s) = self.lock_slide.take() {
                self.dirty = true;
                if s.app.is_some() {
                    self.swallow_release = Some(k.code);
                    return;
                }
            }
        }
        // A held key is input too: dark waits until it comes up.
        if k.state != State::Repeat {
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
                // Tab may wake it to the lock screen: Enter then unlocks.
                if self.wake(now) && k.is(code::TAB) {
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
        if self.screen == Screen::Prompt {
            self.prompt_key(&k, now);
            return;
        }
        if self.screen == Screen::Locked {
            let password = self.wants_password(now);
            let waiting = self.tries.wait(password::boottime()) > 0;
            match self.lock.key(&k, now, password, waiting) {
                lock::Action::None => {}
                lock::Action::Redraw => self.dirty = true,
                lock::Action::Unlock => {
                    self.sound(Sound::Unlock);
                    self.unlock();
                }
                lock::Action::Check(typed) => {
                    self.check_password(typed);
                    self.dirty = true;
                }
            }
            return;
        }
        if k.state == State::Press && !k.is(code::ESC) && self.esc_hold.holding() {
            self.esc_hold.cancel();
            self.dirty = true;
        }
        if self.screen == Screen::Home && self.power.is_some() {
            self.power_key(&k, now);
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
            self.switcher_key(k, now);
            return;
        }
        if self.screen == Screen::Notices {
            self.notices_key(k, now);
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
                    let raw = bl.step(bl.level(), k.code == code::BRIGHTNESSUP);
                    let _ = bl.set(raw);
                    let pct = bl.percent(raw);
                    self.save_config(|c| c.set_int(config::key::BRIGHTNESS, raw as i64));
                    self.show_level("sun", "Brightness", pct);
                }
                true
            }
            code::VOLUMEUP | code::VOLUMEDOWN => {
                let step = if k.code == code::VOLUMEUP {
                    config::VOLUME_STEP
                } else {
                    -config::VOLUME_STEP
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
        self.save_config(|c| c.set_int(config::key::VOLUME, v as i64));
        self.sound(Sound::Move);
    }

    fn apply_mute(&mut self, muted: bool) {
        let flag = if muted { "1" } else { "0" };
        child::fire(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", flag]);
        self.muted = Some(muted);
    }

    fn power_key(&mut self, k: &Key, now: u64) {
        let Some(p) = &mut self.power else {
            return;
        };
        match p.key(k, now) {
            power::Action::None | power::Action::Run => {}
            power::Action::Redraw => self.dirty = true,
            power::Action::Hint(h) => self.toast(h),
            power::Action::Close => {
                self.power = None;
                self.dirty = true;
            }
        }
    }

    fn nav_key(&mut self, k: Key) {
        // Held, Esc opens the power dialog. Esc out of search never starts
        // it: that press was search's.
        if k.is(code::ESC) {
            match k.state {
                State::Press => {
                    self.esc_hold.start(k.code, rt::now_ms());
                    self.dirty = true;
                }
                State::Release if self.esc_hold.let_go(k.code) => {
                    self.toast("Hold Esc for power");
                }
                _ => {}
            }
            return;
        }
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
            Some('l') => self.step_layout(1),
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
        let layout = self.home.layout().label();
        [
            (MenuAct::Pin, format!("{pin} {name}"), String::new(), "p"),
            (MenuAct::Layout, "Layout".into(), layout.into(), "l"),
            (MenuAct::Find, "Find".into(), String::new(), "s"),
            (
                MenuAct::Notices,
                "Notifications".into(),
                match self.notices.len() {
                    0 => String::new(),
                    n => n.to_string(),
                },
                "",
            ),
            (MenuAct::Lock, "Lock".into(), String::new(), "Tab"),
            (MenuAct::Power, "Power".into(), String::new(), "Esc"),
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
            Some(keys::Nav::Left) if sel == 1 => {
                self.step_layout(-1);
                None
            }
            Some(keys::Nav::Right) if sel == 1 => Some(MenuAct::Layout),
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
            MenuAct::Layout => self.step_layout(1),
            MenuAct::Find => self.home.start_search(),
            MenuAct::Notices => self.open_notices(Screen::Home),
            MenuAct::Lock => self.lock_now(rt::now_ms()),
            MenuAct::Power => self.power = Some(Power::default()),
        }
    }

    /// The next layout (`d` 1) or the one before: Grid, List, Row.
    fn step_layout(&mut self, d: i32) {
        let l = self.home.layout().step(d);
        self.save_config(|c| c.set_str(config::key::HOME_LAYOUT, l.config_name()));
        self.home.set_layout(l);
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
        let from = self.home.index();
        self.home.move_by(dx, dy, rt::now_ms());
        if self.home.index() != from {
            self.sound(Sound::Move);
        }
        self.dirty = true;
    }

    fn toggle_pin(&mut self) {
        let Some(id) = self.home.selected().map(|e| e.id.clone()) else {
            return;
        };
        self.toggle_pin_of(&id);
    }

    fn toggle_pin_of(&mut self, id: &str) {
        let name = self
            .apps
            .iter()
            .find(|a| a.file == id)
            .map(|a| a.entry.name.clone())
            .unwrap_or_default();
        match apps::toggle_pin(id) {
            Ok((pins, on)) => {
                self.pins = pins;
                self.toast(format!("{} {name}", if on { "Pinned" } else { "Unpinned" }));
            }
            Err(e) => self.toast(format!("Couldn't save the pins: {e}")),
        }
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
                self.sound(Sound::Error);
                self.toast(format!("{} is closing", app.entry.name))
            }
            Some(_) => {
                self.sound(Sound::Open);
                self.pending = Some(Pending::Resume(id));
            }
            None => {
                self.sound(Sound::Open);
                // Started from home, it has no opener to go back to.
                self.return_to.retain(|(opened, _)| *opened != id);
                self.launch(&app, &[]);
            }
        }
    }

    /// Starts `app` in front. The shell lets go of the panel and keyboard
    /// until the app is paused or gone.
    fn launch(&mut self, app: &App, files: &[PathBuf]) {
        self.power = None;
        self.esc_hold.cancel();
        self.tab_down = None;
        let argv = child::argv(app, &self.terminal, files);
        let stem = app.id().to_string();
        // Read now, not from the copy the shell keeps: a grant changed by
        // hand or by another program counts from this launch.
        let fresh = Config::load(&config::default_path());
        let iso = apps::isolation(&fresh, app, gh0st_core::store::needs_root(&stem));
        let sandboxed = self.on_device && iso.sandbox;
        let argv = if sandboxed {
            let user = notify::user_dir();
            let own = sandbox::notice_dir(&user, &stem);
            let _ = std::fs::create_dir_all(&own);
            self.watch_notices(&own);
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_default();
            let host = sandbox::Host::read(&home, &runtime);
            // Fail closed: an app the user expects isolated never starts
            // outside its sandbox.
            match sandbox::argv(&host, iso.grants, &stem, &user, &argv)
                .filter(|_| Path::new(sandbox::BWRAP).exists())
            {
                Some(a) => a,
                None => {
                    self.sound(Sound::Error);
                    self.toast(format!("Couldn't open {}: no sandbox", app.entry.name));
                    return;
                }
            }
        } else {
            argv
        };
        let cgroup = self.cgroups.as_ref().and_then(|c| c.make(&stem));
        // Pulse reports the pid inside a sandbox; `gh0st audio` finds the
        // app's streams by this instead.
        let mut env = vec![("PULSE_PROP", format!("gh0st.app={stem}"))];
        // Outside a sandbox an input method's bridge comes as inside one:
        // only to an app granted it.
        if !sandboxed && iso.grants.input {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let b = sandbox::bridges(&home);
            if !b.is_empty() {
                env.push(("LD_PRELOAD", sandbox::preload(&b)));
            }
        }
        let extra = child::Extra {
            cgroup: cgroup.as_deref(),
            env,
        };
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
        let spawned = child::spawn_with(
            &argv,
            &dev,
            pause,
            app_end.as_ref().map(|f| f.as_raw_fd()),
            &extra,
        );
        // The app holds its own copy now.
        drop(app_end);
        if let Some(l) = &link {
            l.send(&["hello", &link::VERSION.to_string()]);
        }
        let pgid = match spawned {
            Ok(p) => p,
            Err(e) => {
                self.display.resume();
                self.full_flush = true;
                self.sync_grab();
                self.toast(format!("Couldn't open {}: {e}", app.entry.name));
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
            link_v: 0,
            background: app.entry.background,
            quiet: false,
            told_pause: false,
            awake: false,
            inhibited: false,
            keeps_on: false,
            sound_parked: false,
            sandboxed,
            started: now,
        });
        self.esc = EscHold::default();
        self.toast = None;
    }

    /// The app in front asked to step back (`leave`): it is parked as Fn+K
    /// parks it, running on behind if it may, and the app that opened it
    /// comes back, or home.
    fn leave_front(&mut self, now: u64) {
        let Some(id) = self.tasks.front().map(|t| t.id.clone()) else {
            return;
        };
        let opener = self
            .return_to
            .iter()
            .rposition(|(opened, _)| *opened == id)
            .map(|i| self.return_to.remove(i).1);
        match self.park_front(now) {
            Ok(_) => {
                self.take_screen();
                match opener.filter(|o| self.tasks.get(o).is_some_and(|t| t.resumable())) {
                    Some(o) => self.resume(&o, now),
                    None => {
                        self.screen = Screen::Home;
                        self.dirty = true;
                    }
                }
            }
            Err(Some(name)) => self.toast(format!("{name} can't be paused")),
            Err(None) => {}
        }
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

    /// Takes the app in front off the panel. A linked app is told to pause;
    /// one that may not run behind is then stopped, its last frame kept. The
    /// caller takes the screen. Err with the app's name when it can't be
    /// stopped.
    fn park_front(&mut self, now: u64) -> Result<String, Option<String>> {
        let pgid = self.tasks.front().map(|t| t.pgid);
        let parked = self.stop_front(now);
        // What it said while pausing, such as `awake 0`, waited behind
        // `paused` and no longer polls readable.
        if let Some(pgid) = pgid {
            self.read_link(pgid);
        }
        parked
    }

    fn stop_front(&mut self, now: u64) -> Result<String, Option<String>> {
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
        // An app that declares it but never joined the link can't be told
        // to pause, so it runs behind only if the user said so.
        let background = apps::runs_behind(&self.config, &t.id, t.background && t.linked);
        if background {
            // An app on the link stops drawing and reading keys when told;
            // one that doesn't answer is guarded as before.
            t.ask_pause(STOP_WAIT_MS);
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
        // A linked app hears the pause first, so it can let go of what a
        // stop would freeze: a camera streaming, a recording half written.
        t.ask_pause(STOP_WAIT_MS);
        let sent = child::stop(t.pgid);
        if !t.leader_done && child::wait_stopped(t.pgid, STOP_WAIT_MS) == Stopped::Exited {
            t.leader_done = true;
        }
        // An app run through sudo is root's: the stop does not reach it,
        // and a "paused" app that keeps drawing would sit over the shell.
        if !sent || !child::group_stopped(t.pgid, STOP_WAIT_MS) {
            child::cont(t.pgid);
            t.tell_resume();
            let name = t.name.clone();
            self.spare = Some(frame);
            return Err(Some(name));
        }
        self.display.read_into(&mut frame);
        t.pixels = Some(frame);
        t.state = tasks::State::Paused;
        t.used = now;
        // A stopped app's open stream keeps the codec running.
        let stem = t.id.strip_suffix(".desktop").unwrap_or(&t.id);
        audio_of(self.audio_exe.as_deref(), "park", Some(t.pgid), Some(stem));
        t.sound_parked = true;
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
        self.tab_hold.reset();
        if let Some(px) = t.pixels.take() {
            self.display.flush_all(&px);
            self.spare = Some(px);
        }
        self.display.suspend();
        t.state = tasks::State::Front;
        t.used = now;
        // Back from the lock or the dark: the lock is over. Left Locked, the
        // screen would hold every question the app asks.
        if self.screen == Screen::Locked {
            self.screen = Screen::Home;
        }
        t.mem_kb = None;
        t.keeps_on = false;
        let pgid = t.pgid;
        if std::mem::take(&mut t.sound_parked) {
            audio(self.audio_exe.as_deref(), "unpark", Some(pgid));
        }
        t.tell_resume();
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
        self.switcher.cancel_hold();
        self.kb.reset();
        self.enter_armed = false;
        self.esc_armed = false;
        self.home_armed = false;
        self.esc = EscHold::default();
        // A Tab let go over an app never reached home's hold: left set, it
        // would lock as soon as the shell is in front.
        self.tab_down = None;
        self.tab_hold.reset();
        // An app that grabbed the keyboard hid the releases from us; a key
        // left in `held` would keep the screen on and stall every Pending.
        if let Some(down) = self.input.pressed() {
            self.held.retain(|c| down.contains(c));
            self.esc_down = down.contains(&code::ESC);
        }
        self.last_input = rt::now_ms();
        self.full_flush = true;
        // Settings and stock apps may have changed config.json and the pins,
        // and the app may have installed or removed apps (the Store does).
        self.reload_config();
        // And the volume and mute, which system sounds follow.
        if !self.sim {
            self.start_job(
                JobKind::Volume,
                &["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"],
            );
        }
        self.pins = apps::load_pins();
        self.refresh_status();
        self.rescan();
        self.reload_notices(false);
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
            && (self.tasks.any_paused()
                || matches!(self.screen, Screen::Locked | Screen::Off | Screen::Prompt));
        if !self.input.grab(want) && want && self.screen == Screen::Prompt {
            // Another program holds the keyboard: it would read the password.
            self.refuse_prompts("The keyboard is taken: can't ask");
        }
    }

    fn open_switcher(&mut self, back: Back, at: usize) {
        self.back = back;
        self.switcher.cancel_hold();
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
        self.switcher.cancel_hold();
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

    /// The hold on 4 fills, and at its end closes every app. Only if its
    /// key is still down: a release lost on the way must not close them.
    fn close_all_tick(&mut self, now: u64) {
        match self.switcher.hold_tick(now) {
            HoldTick::Idle => {}
            HoldTick::Fill => self.dirty = true,
            HoldTick::Done(key) => {
                self.dirty = true;
                if !self.held.contains(&key) || self.app_front() {
                    return;
                }
                self.tasks.close_all(now);
                let cards = self.cards();
                self.switcher.set_apps(cards);
            }
        }
    }

    fn switcher_key(&mut self, k: Key, now: u64) {
        // The held key up early closes nothing; so does any other key.
        if k.state == State::Release && self.switcher.let_go(k.code) {
            self.toast("Hold to close all");
            return;
        }
        if k.state == State::Press && self.switcher.holding() {
            self.switcher.cancel_hold();
            self.dirty = true;
        }
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
                            self.sound(Sound::Open);
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
        let from = self.switcher.selected();
        match k.nav(true) {
            Some(keys::Nav::Left | keys::Nav::Right) => {
                let dx = if k.nav(true) == Some(keys::Nav::Left) {
                    -1
                } else {
                    1
                };
                self.switcher.move_by(dx, now);
                if self.switcher.selected() != from {
                    self.sound(Sound::Move);
                }
            }
            _ if (k.is(code::BACKSPACE) || k.ch == Some('8')) && k.state == State::Press => {
                // Del or 8 on an app closes it; on Home, Del held closes
                // every app, as 4 does.
                match self.switcher.selected_app().map(|c| c.id.clone()) {
                    Some(id) => {
                        self.sound(Sound::Close);
                        self.tasks.close(&id, now);
                        let cards = self.cards();
                        self.switcher.set_apps(cards);
                    }
                    None if k.is(code::BACKSPACE) => self.switcher.hold_all(k.code, now),
                    None => {}
                }
            }
            _ if k.ch == Some('4') && k.state == State::Press => {
                self.switcher.hold_all(k.code, now);
            }
            _ if k.ch == Some('6') && k.state == State::Press => self.lock_now(now),
            _ if k.ch == Some('5') && k.state == State::Press && !self.notices.is_empty() => {
                self.open_notices(Screen::Switcher);
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn draw(&mut self) {
        if let Some(restart) = self.bye {
            self.draw_bye(restart);
            self.dirty = false;
            return;
        }
        // A menu or a question belongs to the screen it was opened on.
        if self.screen != Screen::Home {
            self.home_menu = None;
            self.power = None;
            self.esc_hold.cancel();
        }
        if self.screen != Screen::Switcher {
            self.switcher.cancel_hold();
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
        let previewing = match self.prompts.current().map(|a| &a.origin) {
            Some(Origin::Portal(id, _)) => self
                .preview
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, b)| b.home.clone()),
            _ => None,
        };
        let ground = self.wall.as_ref().and_then(|w| match self.screen {
            Screen::Prompt if previewing.is_some() => previewing,
            Screen::Locked | Screen::Prompt => Some(w.lock.clone()),
            Screen::Home | Screen::Switcher | Screen::Notices => Some(w.home.clone()),
            _ => None,
        });
        self.canvas.set_backdrop(ground);
        let mut status = status_of(&clock, self.battery, self.net, &dock, front);
        status.bluetooth = self.bluetooth;
        status.notices = self.notices.len().min(u8::MAX as usize) as u8;
        match self.screen {
            Screen::Locked => self.draw_lock(&clock),
            Screen::Prompt => match self.prompts.current().cloned() {
                Some(ask) => {
                    self.prompts.seen(now);
                    let ask = &ask;
                    let (icon, title) = prompt::bar(&ask.origin);
                    let status = Status {
                        dock: &[],
                        front: None,
                        icon,
                        title,
                        ..status
                    };
                    let v = prompt::View {
                        ask,
                        typed: self.prompts.typed(),
                        checking: self.prompts.checking,
                        wrong: self.prompts.wrong,
                        at: self.prompts.at(),
                    };
                    prompt::draw(&mut self.canvas, &t, &status, &v);
                }
                None => self.home.draw(&mut self.canvas, &t, &status, now),
            },
            Screen::Switcher => self.switcher.draw(&mut self.canvas, &t, &status, now),
            Screen::Notices => {
                let status = Status {
                    dock: &[],
                    front: None,
                    icon: "bell",
                    title: "Notifications",
                    ..status
                };
                self.panel
                    .draw(&mut self.canvas, &t, &status, notify::now_secs());
            }
            Screen::Home if self.power.is_some() => {
                let status = Status {
                    dock: &[],
                    front: None,
                    icon: "power",
                    title: "Power",
                    ..status
                };
                if let Some(p) = &self.power {
                    p.draw(&mut self.canvas, &t, &status);
                }
            }
            _ => self.home.draw(&mut self.canvas, &t, &status, now),
        }
        if self.screen == Screen::Home && self.power.is_none() {
            if let Some(sel) = self.home_menu {
                self.draw_menu(sel);
            }
        }
        drop(dock);
        drop(cards);
        self.draw_lock_slide(now);
        if matches!(
            self.screen,
            Screen::Home | Screen::Switcher | Screen::Notices | Screen::Locked | Screen::Prompt
        ) {
            self.draw_low_battery();
            if let Some(pc) = self.esc_hold.fill() {
                let bottom = t.m.height - t.m.hint_h;
                let info = gh0st_ui::theme::Tone::Info;
                widgets::toast(
                    &mut self.canvas,
                    &t,
                    bottom,
                    info,
                    "Hold for power",
                    Some(pc),
                );
            } else if let Some((icon, label, pct)) = self.level {
                widgets::level_bar(&mut self.canvas, &t, icon, label, pct);
            } else if let (Some(row), Some(_)) = (&self.toast_notice, &self.toast) {
                let bottom = t.m.height - t.m.hint_h;
                let v = home::visual(&t, &row.icon, &row.name, Size::Tiny);
                widgets::notice_toast(
                    &mut self.canvas,
                    &t,
                    bottom,
                    &v,
                    &row.name,
                    &row.notice.title,
                );
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
        if self.lock_slide.as_ref().is_some_and(|s| s.anim.done(now)) {
            self.lock_slide = None;
            // The app waited for the slide: it comes back now, not at the
            // next key.
            self.run_pending(now);
        }
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
                format!("Shutting down in {} s", self.low.seconds_left(now)),
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

/// `gh0st audio VERB [PGID]` as a child, since pactl can take a while.
/// Says, as a notice that stays, that app `id` may have failed for want of
/// what its sandbox keeps out. Enter opens Settings.
fn sandbox_hint(id: &str, name: &str) {
    let stem = id.strip_suffix(".desktop").unwrap_or(id);
    let tag: String = stem
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '-' | '_') => c,
            _ => '_',
        })
        .collect();
    let n = Notice {
        app: "settings".into(),
        tag: format!("sandbox-{tag}"),
        title: format!("{name} closed as it opened"),
        body: format!("Its sandbox may be why. Settings > Apps > {name} turns Isolation off."),
        tone: notify::Tone::Warning,
        time: notify::now_secs(),
        ..Notice::default()
    };
    let _ = notify::post(&notify::user_dir(), &n);
}

/// A sandboxed app that fails sooner than this after its start gets a
/// hint that its sandbox may be why.
const EARLY_EXIT_MS: u64 = 5000;

fn audio(exe: Option<&Path>, verb: &str, pgid: Option<i32>) {
    audio_of(exe, verb, pgid, None);
}

/// As `audio`, naming the app too: a sandboxed app's streams carry its
/// stem, not a pid the shell knows.
fn audio_of(exe: Option<&Path>, verb: &str, pgid: Option<i32>, app: Option<&str>) {
    let Some(exe) = exe.and_then(Path::to_str) else {
        return;
    };
    let pgid = pgid.map(|p| p.to_string());
    let mut argv = vec![exe, "audio", verb];
    argv.extend(pgid.as_deref());
    argv.extend(app);
    child::fire(&argv);
}

/// Wakes for app installs (true: rescan), and for battery, charger and link
/// changes (false: re-read status only).
fn stale_ask(retry: bool) -> Ask {
    Ask {
        origin: Origin::Stale,
        kind: Kind::Password {
            reason: String::new(),
            retry,
        },
    }
}

#[cfg(target_os = "linux")]
fn watches(d: &Dirs) -> Vec<(Wake, Watch)> {
    let apps = Watch::dirs(&[d.ours.as_path(), d.stock.as_path()]).map(|w| (Wake::Apps, w));
    let power = Watch::uevents().map(|w| (Wake::Status, w));
    let links = Watch::links().map(|w| (Wake::Status, w));
    // The apps' folder is made here, so it is there to watch.
    let user = notify::user_dir();
    let _ = std::fs::create_dir_all(&user);
    let system = notify::system_dir();
    let notices = Watch::dirs(&[system.as_path(), user.as_path()]).map(|w| {
        // Sandboxed apps' own folders, from earlier runs.
        for e in std::fs::read_dir(&user).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                w.add(&e.path());
            }
        }
        (Wake::Notices, w)
    });
    [apps, power, links, notices]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn watches(_: &Dirs) -> Vec<(Wake, Watch)> {
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
    let mut fb = Framebuffer::open(&fb_path).map_err(|e| format!("{}: {e}", fb_path.display()))?;
    // A shell that died with an overlay up left it in the driver.
    fb.overlay_clear(overlay::BANNER);
    fb.overlay_clear(overlay::POPUP);
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

fn terminal_of(c: &Config) -> String {
    c.get_str(config::key::TERMINAL)
        .unwrap_or(child::DEFAULT_TERMINAL)
        .to_string()
}

/// The system manager says "stopping" from the moment a shutdown or
/// restart starts; the user manager stopping the shell may not. Asked over
/// D-Bus from here: a child such as systemctl started now can die in the
/// SIGTERM systemd sends the whole unit.
fn system_stopping() -> bool {
    use gh0st_dbus::{Conn, Message, Value};
    let Ok(mut c) = Conn::system() else {
        return false;
    };
    let m = Message::call(
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.DBus.Properties",
        "Get",
    )
    .arg(Value::str("org.freedesktop.systemd1.Manager"))
    .arg(Value::str("SystemState"));
    c.call(m, 1000).is_ok_and(|r| {
        r.body
            .first()
            .and_then(|v| v.inner().as_str())
            .is_some_and(|s| s == "stopping")
    })
}

/// Marks Wi-Fi as turned off by the dark, so a restarted shell turns it on.
fn wifi_marker() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("gh0st-wifi-slept")
}

/// Whether a wake finds the device locked: by hand or at start, or dark at
/// least `after`. None for `after` locks only by hand.
fn lock_due(secure: bool, dark_since: Option<u64>, after: Option<u64>, now: u64) -> bool {
    secure || matches!((dark_since, after), (Some(d), Some(a)) if now >= d + a)
}

/// Whether the lock asks for the password: a lock that is due, with the
/// password lock on and a password to ask.
fn asks_password(
    secure: bool,
    dark_since: Option<u64>,
    after: Option<u64>,
    now: u64,
    on: bool,
    account: Option<Account>,
    probing: bool,
) -> bool {
    let real = lock_due(secure, dark_since, after, now);
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
            Net::Hotspot => widgets::Net::Hotspot,
        },
        bluetooth: false,
        notices: 0,
    }
}

fn desktop_first_token_runs(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .is_some_and(|p| gh0st_core::desktop::is_executable(Path::new(p)))
}

/// A portal question's id; 0 for any other.
fn origin_id(o: &Origin) -> u64 {
    match o {
        Origin::Portal(id, _) => *id,
        _ => 0,
    }
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
    fn a_wake_locks_past_lock_after_only() {
        let due = |secure, since, after, now| lock_due(secure, since, after, now);
        assert!(due(true, None, None, 0), "by hand or at start");
        assert!(
            !due(false, Some(1000), Some(300_000), 200_000),
            "a short dark"
        );
        assert!(due(false, Some(1000), Some(300_000), 301_000), "a long one");
        assert!(due(false, Some(1000), Some(0), 1000), "at once");
        assert!(!due(false, Some(1000), None, u64::MAX / 2), "only by hand");
    }

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
    /// Switch apps left the menu for this: it has 4 and Fn+K.
    Notices,
    Lock,
    Power,
}

const MENU_ROWS: usize = 6;
