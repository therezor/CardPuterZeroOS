//! Pages and what they can ask of the system.

use crate::bars::Bars;
use crate::key::Key;
use crate::ui::Ui;
use gh0st_core::config::{self, Config};
use gh0st_ui::Theme;
use std::path::PathBuf;

/// One screen of an app. An app is a stack of pages: the first one is the
/// app, [`Cx::push`] opens another on top, and Esc closes the top one. When
/// the last page closes, the app exits.
///
/// Only [`title`](Page::title) and [`draw`](Page::draw) are required. After
/// every event the SDK calls `draw`, and only the rows that changed go to
/// the panel, so a page never schedules its own redraws.
pub trait Page {
    /// The title in the OS top bar.
    fn title(&self) -> &str;

    /// Draws the page into the area its bars leave.
    fn draw(&self, ui: &mut Ui);

    /// The OS bars this page shows. The default is the top bar alone.
    fn bars(&self) -> Bars {
        Bars::top()
    }

    /// A key press. Esc doesn't come here: see [`back`](Page::back).
    fn key(&mut self, key: Key, cx: &mut Cx) {
        let _ = (key, cx);
    }

    /// Esc was pressed. Return false to keep the page open, for example to
    /// close a dialog first. The default closes the page.
    fn back(&mut self, cx: &mut Cx) -> bool {
        let _ = cx;
        true
    }

    /// True while the page takes text. Letters and digits then type instead
    /// of moving the selection or pressing soft keys.
    fn typing(&self) -> bool {
        false
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
}

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
    Push(Box<dyn Page>),
    Back,
    Quit,
    Toast(String),
    Timer { id: u64, ms: u64, every: bool },
    Cancel(u64),
    Spawn { id: u64, argv: Vec<String> },
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

    /// Re-reads the file before each write, so a change the shell or a
    /// stock app made meanwhile survives.
    fn update(&mut self, f: impl FnOnce(&mut Config)) {
        let mut fresh = Config::load(self.config.path());
        f(&mut fresh);
        if let Err(e) = fresh.save() {
            eprintln!("gh0st app: saving {}: {e}", fresh.path().display());
        }
        self.config = fresh;
    }
}

pub(crate) struct Shared {
    pub theme: Theme,
    pub settings: Settings,
    pub requests: Vec<Request>,
    pub next_id: u64,
    pub app_id: String,
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

    /// Closes every page and exits.
    pub fn quit(&mut self) {
        self.s.requests.push(Request::Quit);
    }

    /// A short message over the bottom bar for two seconds.
    pub fn toast(&mut self, text: &str) {
        self.s.requests.push(Request::Toast(text.to_string()));
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

    /// Runs a program without a shell. Its output comes to [`Page::job`] a
    /// line at a time, then its exit code. It runs inside the app's own
    /// event loop, so the app never blocks on it.
    pub fn spawn(&mut self, argv: &[&str]) -> Job {
        let id = self.s.id();
        self.s.requests.push(Request::Spawn {
            id,
            argv: argv.iter().map(|a| a.to_string()).collect(),
        });
        Job(id)
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
