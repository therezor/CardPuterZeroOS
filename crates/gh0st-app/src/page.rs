//! Pages and what they can ask of the system.

use crate::bars::Bars;
use crate::command::{Commands, Step};
use crate::dialog::{Answer, Dialog};
use crate::key::Key;
use crate::ui::Ui;
use gh0st_core::config::{self, Config};
use gh0st_core::keys::Key as RawKey;
use gh0st_ui::Theme;
use gh0st_ui::theme::Tone;
use std::os::fd::RawFd;
use std::path::PathBuf;

/// One screen of an app. An app is a stack of pages: the first one is the
/// app, [`Cx::push`] opens another on top, and Esc closes the top one. When
/// the last page closes, the app exits.
///
/// Only [`title`](Page::title) and [`draw`](Page::draw) are required. After
/// every event the SDK calls `draw`, and only the rows that changed go to
/// the panel, so a page never schedules its own redraws.
pub trait Page {
    /// The page's name in the top bar.
    fn title(&self) -> &str;

    /// The page's icon in the top bar, a Lucide name. Empty takes the first
    /// page's, then the gh0st mark.
    fn icon(&self) -> &str {
        ""
    }

    /// What follows the title, muted: a step, a file. `None` shows the
    /// app's name on a page above the first.
    fn context(&self) -> Option<&str> {
        None
    }

    /// One word of the app's own in the top bar, in a tone: `("REC",
    /// Tone::Critical)` while recording.
    fn status(&self) -> Option<(&str, Tone)> {
        None
    }

    /// Sessions, numbered in the top bar: how many, and which is in front.
    /// None by default.
    fn tabs(&self) -> (usize, usize) {
        (0, 0)
    }

    /// Draws the page into the area its bars leave.
    fn draw(&self, ui: &mut Ui);

    /// The OS bars this page shows: the top bar and the action bar, with the
    /// menu on 8.
    fn bars(&self) -> Bars {
        Bars::top()
    }

    /// A key press. Esc and 4 don't come here: see [`back`](Page::back).
    fn key(&mut self, key: Key, cx: &mut Cx) {
        let _ = (key, cx);
    }

    /// Esc, or 4, was pressed: go back one step. Return false to stay, for
    /// example after leaving a mode. The default closes the page; on the
    /// first page the SDK asks for a 0.7 s hold before the app closes.
    fn back(&mut self, cx: &mut Cx) -> bool {
        let _ = cx;
        true
    }

    /// True while the page takes text. Letters and digits then type instead
    /// of moving the selection or running chips, and the action bar shows
    /// the page's [`hints`](Bars::hints).
    fn typing(&self) -> bool {
        false
    }

    /// A dialog from [`Cx::ask`] closed: Yes with its field's text, or No.
    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        let _ = (id, answer, cx);
    }

    /// The page has opened. Start jobs and timers here.
    fn start(&mut self, cx: &mut Cx) {
        let _ = cx;
    }

    /// The page is on top again after the page above it closed.
    fn resume(&mut self, cx: &mut Cx) {
        let _ = cx;
    }

    /// A timer from [`Cx::after`] or [`Cx::every`] fired.
    fn timer(&mut self, timer: Timer, cx: &mut Cx) {
        let _ = (timer, cx);
    }

    /// A job from [`Cx::spawn`] printed a line or ended.
    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        let _ = (job, event, cx);
    }

    /// The page is closing, or the app is quitting (Esc held, SIGTERM). Save
    /// state here. Its timers and jobs stop after this.
    fn stop(&mut self, cx: &mut Cx) {
        let _ = cx;
    }

    /// An fd from [`Cx::watch`] is readable, or its other end closed. Read
    /// it until it would block. Nothing is drawn unless the page calls
    /// [`Cx::redraw`].
    fn ready(&mut self, watch: Watch, cx: &mut Cx) {
        let _ = (watch, cx);
    }

    /// A child from [`Cx::adopt`] ended. `code` is None when a signal ended
    /// it. Nothing is drawn unless the page calls [`Cx::redraw`].
    fn exited(&mut self, pid: i32, code: Option<i32>, cx: &mut Cx) {
        let _ = (pid, code, cx);
    }

    /// True to take the keyboard raw, as a terminal does: every press,
    /// repeat and release comes to [`raw`](Page::raw) with its modifiers,
    /// Esc included, and every key but Esc repeats. [`key`](Page::key) and
    /// [`back`](Page::back) are not called.
    fn raw_keys(&self) -> bool {
        false
    }

    /// A raw key event, when [`raw_keys`](Page::raw_keys) is true. Nothing
    /// is drawn unless the page calls [`Cx::redraw`].
    fn raw(&mut self, key: &RawKey, cx: &mut Cx) {
        let _ = (key, cx);
    }

    /// What the user can do on this page. The app menu (Fn+Esc), the Keys
    /// screen, command hints and the keys themselves all come from this
    /// list, so they never disagree. Asked again on every key and every
    /// time the menu changes. See "Commands" in the guide.
    fn commands(&self, c: &mut Commands) {
        let _ = c;
    }

    /// A command ran, by its key or its menu row. An action's menu has
    /// closed by now; a toggle's or a choice's stays open and shows the new
    /// value.
    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let _ = (id, step, cx);
    }

    /// The app came to the front (true) or went behind another (false).
    /// Every open page hears it. An app not allowed to run in the
    /// background is stopped right after, so let go of a device or finish a
    /// file here.
    fn front(&mut self, front: bool, cx: &mut Cx) {
        let _ = (front, cx);
    }

    /// The least time between two frames, in ms. A page that changes faster
    /// than the panel can show (a terminal under a flood of output) sets it
    /// near the panel's refresh, and the SDK draws the latest state at most
    /// that often.
    fn frame_ms(&self) -> u64 {
        0
    }
}

/// An fd watched by [`Cx::watch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Watch(pub(crate) u64);

/// A timer started by [`Cx::after`] or [`Cx::every`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timer(pub(crate) u64);

/// A child process started by [`Cx::spawn`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Job(pub(crate) u64);

/// What a job did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobEvent {
    /// One line of its output, without the newline.
    Line(String),
    /// It ended with this exit code. `None`: it couldn't start, or a signal
    /// ended it.
    Done(Option<i32>),
}

pub(crate) enum Request {
    /// Work keys wait for, and what to call it; `None` when it ends.
    Busy(Option<String>),
    Push(Box<dyn Page>),
    Back,
    Done,
    Quit,
    Toast(String, Tone),
    Timer {
        id: u64,
        ms: u64,
        every: bool,
    },
    Cancel(u64),
    Spawn {
        id: u64,
        argv: Vec<String>,
        input: Option<Vec<u8>>,
    },
    Watch {
        id: u64,
        fd: RawFd,
    },
    Unwatch(u64),
    Write {
        id: u64,
        bytes: Vec<u8>,
    },
    Adopt(i32),
    Redraw,
    Notify(String),
    KeepAwake(bool),
    Open(String),
    ShowKeys,
    Ask(u32, Dialog),
}

/// The app's settings in the shared device config,
/// `~/.config/cardputerzero/config.json`. Stock apps read that file too, so
/// keys the SDK doesn't know stay as they are. An app's own keys should
/// start with its name and a dot.
pub struct Settings {
    config: Config,
}

impl Settings {
    pub(crate) fn load(path: &std::path::Path) -> Settings {
        Settings {
            config: Config::load(path),
        }
    }

    /// The whole file, for the typed readers of shared keys such as
    /// [`Config::clock_24h`].
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.config.get_str(key)
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.config.get_int(key)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.config.get_bool(key)
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.update(|c| c.set_str(key, v));
    }

    pub fn set_int(&mut self, key: &str, v: i64) {
        self.update(|c| c.set_int(key, v));
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.update(|c| c.set_bool(key, v));
    }

    pub fn remove(&mut self, key: &str) {
        self.update(|c| c.remove(key));
    }

    /// Re-reads the file before each write, so a change the shell or a
    /// stock app made meanwhile survives.
    fn update(&mut self, f: impl FnOnce(&mut Config)) {
        let path = self.config.path().to_path_buf();
        match Config::update(&path, f) {
            Ok(fresh) => self.config = fresh,
            Err(e) => eprintln!("gh0st app: saving {}: {e}", path.display()),
        }
    }
}

pub(crate) struct Shared {
    pub theme: Theme,
    pub settings: Settings,
    pub requests: Vec<Request>,
    pub next_id: u64,
    pub app_id: String,
    /// The shell has paused the app: something else is in front.
    pub paused: bool,
    /// The page that just closed called [`Cx::done`]: its parent hears it
    /// in [`Page::resume`], as [`Steps`](crate::Steps) does.
    pub done: bool,
    /// False for an app the user can't close (first-boot setup): no hold
    /// to exit, no Close app, and its last page stays. SIGTERM and
    /// [`Cx::quit`] still end it.
    pub closable: bool,
}

impl Shared {
    pub fn new(theme: Theme, config_path: Option<PathBuf>) -> Shared {
        let path = config_path.unwrap_or_else(config::default_path);
        Shared {
            theme,
            settings: Settings::load(&path),
            requests: Vec::new(),
            next_id: 1,
            app_id: app_id(),
            paused: false,
            done: false,
            closable: true,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// The app's id: `$GH0ST_APP_ID` if set (the shell will set it once the
/// shell link lands), else the program's file name.
fn app_id() -> String {
    std::env::var("GH0ST_APP_ID").ok().unwrap_or_else(|| {
        std::env::args()
            .next()
            .and_then(|a| {
                std::path::Path::new(&a)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "app".into())
    })
}

/// What a page can ask of the system. Every call takes effect after the
/// callback returns.
pub struct Cx<'a> {
    pub(crate) s: &'a mut Shared,
}

impl Cx<'_> {
    /// Opens `page` on top of this one.
    pub fn push(&mut self, page: impl Page + 'static) {
        self.s.requests.push(Request::Push(Box::new(page)));
    }

    /// Closes this page, as Esc does.
    pub fn back(&mut self) {
        self.s.requests.push(Request::Back);
    }

    /// Closes this page because its job is done. In [`Steps`](crate::Steps)
    /// that goes to the next step, where Esc and [`back`](Cx::back) go to
    /// the one before; anywhere else it is the same as `back`.
    pub fn done(&mut self) {
        self.s.requests.push(Request::Done);
    }

    /// Closes every page and exits.
    pub fn quit(&mut self) {
        self.s.requests.push(Request::Quit);
    }

    /// A short line that floats above the action bar for two seconds.
    pub fn toast(&mut self, text: &str) {
        self.toast_tone(Tone::Info, text);
    }

    /// A toast in a tone: Ok for done, Warning for take care, Critical for
    /// an error.
    pub fn toast_tone(&mut self, tone: Tone, text: &str) {
        self.s.requests.push(Request::Toast(text.to_string(), tone));
    }

    /// Work the user waits for: a root helper, a radio. After a moment the
    /// page dims under a card with `text` and a moving bar, and keys wait.
    /// Esc hides the card; the work goes on. Call [`Cx::idle`] when it ends,
    /// then say how it went with a toast. A change that is done sooner never
    /// shows the card.
    pub fn busy(&mut self, text: &str) {
        self.s.requests.push(Request::Busy(Some(text.to_string())));
    }

    /// The work [`Cx::busy`] named has ended.
    pub fn idle(&mut self) {
        self.s.requests.push(Request::Busy(None));
    }

    /// Opens a dialog over the page. It takes every key until Enter, Esc or
    /// its held chip closes it, then [`Page::answer`] hears `id` and how.
    pub fn ask(&mut self, id: u32, dialog: Dialog) {
        self.s.requests.push(Request::Ask(id, dialog));
    }

    /// Fires [`Page::timer`] once, `ms` from now.
    pub fn after(&mut self, ms: u64) -> Timer {
        let id = self.s.id();
        self.s.requests.push(Request::Timer {
            id,
            ms,
            every: false,
        });
        Timer(id)
    }

    /// Fires [`Page::timer`] every `ms` until cancelled. Every timer wakes
    /// the CPU: keep them for things on screen, and stop them when idle.
    pub fn every(&mut self, ms: u64) -> Timer {
        let id = self.s.id();
        self.s.requests.push(Request::Timer {
            id,
            ms: ms.max(1),
            every: true,
        });
        Timer(id)
    }

    pub fn cancel(&mut self, timer: Timer) {
        self.s.requests.push(Request::Cancel(timer.0));
    }

    /// Runs a program without a shell. Its output, stdout and stderr
    /// together, comes to [`Page::job`] a line at a time, then its exit code.
    /// It runs inside the app's own event loop, so the app never blocks on
    /// it. The C locale keeps tool output in a shape parsers expect.
    pub fn spawn(&mut self, argv: &[&str]) -> Job {
        self.spawn_inner(argv, None)
    }

    /// As [`spawn`](Cx::spawn), with `input` written to its stdin. Pass a
    /// password this way, never in `argv`, where any user can read it.
    /// The SDK zeroes its copy once written.
    pub fn spawn_with_input(&mut self, argv: &[&str], input: &[u8]) -> Job {
        self.spawn_inner(argv, Some(input.to_vec()))
    }

    fn spawn_inner(&mut self, argv: &[&str], input: Option<Vec<u8>>) -> Job {
        let id = self.s.id();
        self.s.requests.push(Request::Spawn {
            id,
            argv: argv.iter().map(|a| a.to_string()).collect(),
            input,
        });
        Job(id)
    }

    /// Joins `fd` to the app's event loop: [`Page::ready`] runs when it is
    /// readable. The page keeps owning the fd and closes it after
    /// [`unwatch`](Cx::unwatch). Make it non-blocking.
    pub fn watch(&mut self, fd: RawFd) -> Watch {
        let id = self.s.id();
        self.s.requests.push(Request::Watch { id, fd });
        Watch(id)
    }

    pub fn unwatch(&mut self, watch: Watch) {
        self.s.requests.push(Request::Unwatch(watch.0));
    }

    /// Writes to a watched fd without blocking. What doesn't fit now waits
    /// in a queue and goes out when the fd has room, in order.
    pub fn write(&mut self, watch: Watch, bytes: &[u8]) {
        self.s.requests.push(Request::Write {
            id: watch.0,
            bytes: bytes.to_vec(),
        });
    }

    /// Takes a child the page started itself (a terminal's shell): its exit
    /// comes to [`Page::exited`]. The SDK reaps every child, so a page must
    /// never call `waitpid`.
    pub fn adopt(&mut self, pid: i32) {
        self.s.requests.push(Request::Adopt(pid));
    }

    /// Tells the user something even while the app runs behind another:
    /// the shell shows it over whatever is in front then. In front it is
    /// a [`toast`](Cx::toast).
    pub fn notify(&mut self, text: &str) {
        self.s.requests.push(Request::Notify(text.to_string()));
    }

    /// Asks the shell to bring up app `name`: its desktop file stem
    /// (`update`) or its program (`gh0st-update`). This app pauses, and
    /// comes back when that one closes. Without a shell, nothing happens.
    pub fn open(&mut self, name: &str) {
        self.s.requests.push(Request::Open(name.to_string()));
    }

    /// Posts a notice that stays until the user clears it or this app
    /// replaces it: a toast now, a row in the shell's notifications, a
    /// count on this app's tile. Posting `tag` again replaces it. See
    /// [`notify`](crate::notify).
    pub fn post(&mut self, tag: &str, title: &str, body: &str) {
        let n = crate::notify::Notice {
            app: self.s.app_id.clone(),
            tag: tag.into(),
            title: title.into(),
            body: body.into(),
            time: crate::notify::now_secs(),
            ..Default::default()
        };
        if let Err(e) = crate::notify::post(&crate::notify::user_dir(), &n) {
            eprintln!("gh0st app: notice {tag}: {e}");
        }
    }

    /// Takes back notice `tag`, posted by [`post`](Cx::post).
    pub fn unpost(&mut self, tag: &str) {
        let _ = crate::notify::remove(&crate::notify::user_dir(), &self.s.app_id, tag);
    }

    /// Keeps the screen on while the app is in front, for a player or a
    /// clock the user watches without typing. False lets it go dark again.
    pub fn keep_awake(&mut self, on: bool) {
        self.s.requests.push(Request::KeepAwake(on));
    }

    /// False while the shell has another app, or itself, in front.
    pub fn in_front(&self) -> bool {
        !self.s.paused
    }

    /// Draws again after this callback. Keys, timers and jobs always draw;
    /// [`Page::ready`], [`Page::exited`] and [`Page::raw`] draw only when
    /// asked.
    pub fn redraw(&mut self) {
        self.s.requests.push(Request::Redraw);
    }

    /// Opens Help: every key of this page, as Fn+H does.
    pub fn show_keys(&mut self) {
        self.s.requests.push(Request::ShowKeys);
    }

    /// The shared device settings.
    pub fn settings(&mut self) -> &mut Settings {
        &mut self.s.settings
    }

    /// The OS theme: palette, metrics and fonts.
    pub fn theme(&self) -> &Theme {
        &self.s.theme
    }

    /// A directory for the app's own files, `~/.local/share/<app id>`.
    /// Created on first use.
    pub fn data_dir(&self) -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = home.join(".local/share").join(&self.s.app_id);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}
