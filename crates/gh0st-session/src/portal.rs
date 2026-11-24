//! The XDG desktop portal's FileChooser and OpenURI (`docs/portal.md`). No
//! I/O here: a call gives its reply, and the rest goes to `out` as bus
//! signals and lines for the shell. The shell decides who may ask, runs
//! Files as the chooser, and says how it went.

use gh0st_dbus::{Message, Ty, Value, bus};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const NAME: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";
pub const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
pub const OPEN_URI: &str = "org.freedesktop.portal.OpenURI";
pub const REQUEST: &str = "org.freedesktop.portal.Request";
/// Ours: the chooser takes its question and gives its answer here.
pub const OURS: &str = "org.gh0st.Portal";
pub const OURS_PATH: &str = "/org/gh0st/Portal";
const REQUEST_ROOT: &str = "/org/freedesktop/portal/desktop/request";

const CHOOSER_VERSION: u32 = 4;
const OPEN_URI_VERSION: u32 = 5;
const NOTIFICATION: &str = "org.freedesktop.portal.Notification";
const NOTIFICATION_VERSION: u32 = 2;
pub const INHIBIT: &str = "org.freedesktop.portal.Inhibit";
const INHIBIT_VERSION: u32 = 3;
pub const ACCOUNT: &str = "org.freedesktop.portal.Account";
const ACCOUNT_VERSION: u32 = 1;
const SESSION: &str = "org.freedesktop.portal.Session";
pub const TRASH: &str = "org.freedesktop.portal.Trash";
const TRASH_VERSION: u32 = 1;
pub const WALLPAPER: &str = "org.freedesktop.portal.Wallpaper";
const WALLPAPER_VERSION: u32 = 1;
pub const SESSION_ROOT: &str = "/org/freedesktop/portal/desktop/session";
/// Inhibit's flags: suspending and the screen going dark.
const KEEP_AWAKE: u32 = 4 | 8;
/// Requests a caller may have open at once.
const PER_CALLER: usize = 4;
/// Bounds on what a caller's options may hold.
const MAX_FILTERS: usize = 32;
const MAX_PATTERNS: usize = 64;
const MAX_TEXT: usize = 1024;

pub const RESPONSE_OK: u32 = 0;
pub const RESPONSE_CANCELLED: u32 = 1;
pub const RESPONSE_OTHER: u32 = 2;

/// What goes out besides a call's reply.
#[derive(Debug)]
pub enum Out {
    Bus(Message),
    /// A line for the shell, as fields.
    Shell(Vec<String>),
}

/// A filter as the portal names it: kind 0 a glob, 1 a MIME type.
pub type Filter = (String, Vec<(u32, String)>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
    Folder,
}

impl Mode {
    fn word(self) -> &'static str {
        match self {
            Mode::Open => "open",
            Mode::Save => "save",
            Mode::Folder => "folder",
        }
    }
}

/// What a chooser is asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub mode: Mode,
    pub multiple: bool,
    /// The asking app, as the user knows it.
    pub app: String,
    pub title: String,
    pub accept: String,
    pub filters: Vec<Filter>,
    pub filter: Option<usize>,
    pub name: String,
    pub folder: Option<PathBuf>,
    /// SaveFiles: the names to save into the folder picked.
    pub files: Vec<Vec<u8>>,
    /// `choices`, answered with their defaults.
    pub choices: Vec<(String, String)>,
}

#[derive(Debug)]
enum Kind {
    Pick(Box<Question>),
    Open,
    /// Keeps the caller's app awake until Close or the caller leaves; pid 0
    /// holds nothing.
    Inhibit {
        pid: i32,
    },
    /// Asks the user before giving the caller the user's name.
    Account {
        app: String,
    },
    /// A session state monitor; its session object is at `session`.
    Monitor {
        session: String,
    },
    /// A wallpaper, fitted to the screen at `tmp` before the user says yes.
    Wallpaper {
        app: String,
        pid: i32,
        tmp: PathBuf,
    },
}

/// A wallpaper being fitted: `gh0st-settings --import-wallpaper`. Its
/// stdout closes when it exits, which wakes the loop.
struct Job {
    id: u64,
    child: std::process::Child,
}

#[derive(Debug)]
struct Req {
    id: u64,
    sender: String,
    path: String,
    kind: Kind,
    /// The chooser the shell started for it.
    picker: Option<i32>,
}

#[derive(Default)]
pub struct Portal {
    reqs: Vec<Req>,
    next: u64,
    pub out: Vec<Out>,
    /// Where the apps the user answered Always are kept; None: nowhere.
    pub remember: Option<PathBuf>,
    /// The shared config.json: the wallpaper's setting.
    pub config: PathBuf,
    /// Where a wallpaper waits for the user's yes.
    pub scratch: PathBuf,
    /// `gh0st-settings`, which fits a picture to the screen.
    pub settings_bin: PathBuf,
    job: Option<Job>,
}

/// The user, as the Account portal gives them: login name and real name.
fn user() -> (String, String) {
    // SAFETY: getpwuid's record is read at once, before any other call.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return (String::new(), String::new());
        }
        let s = |p: *const libc::c_char| {
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        let login = s((*pw).pw_name);
        let gecos = s((*pw).pw_gecos);
        let name = gecos.split(',').next().unwrap_or("").to_string();
        let name = if name.is_empty() { login.clone() } else { name };
        (login, name)
    }
}

fn text(v: Option<&Value>) -> String {
    let s = v.map(Value::inner).and_then(Value::as_str).unwrap_or("");
    s.chars().take(MAX_TEXT).collect()
}

fn flag(v: Option<&Value>) -> bool {
    v.map(Value::inner)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// `ay` with its NUL end taken off.
fn bytes_path(v: Option<&Value>) -> Option<PathBuf> {
    let b = v.map(Value::inner).and_then(Value::as_bytes)?;
    let b = b.strip_suffix(&[0]).unwrap_or(b);
    (!b.is_empty() && !b.contains(&0) && b.len() <= 4096)
        .then(|| PathBuf::from(std::ffi::OsStr::from_bytes(b)))
}

fn one_filter(v: &Value) -> Option<Filter> {
    let f = v.inner().as_struct()?;
    let name: String = f.first()?.as_str()?.chars().take(MAX_TEXT).collect();
    let pats = f
        .get(1)?
        .as_array()?
        .iter()
        .filter_map(|p| {
            let p = p.as_struct()?;
            let kind = p.first()?.as_u32()?;
            let pat = p.get(1)?.as_str()?;
            (kind <= 1 && pat.len() <= MAX_TEXT).then(|| (kind, pat.to_string()))
        })
        .take(MAX_PATTERNS)
        .collect();
    Some((name, pats))
}

fn filter_value(f: &Filter) -> Value {
    let pat_ty = Ty::Struct(vec![Ty::U32, Ty::Str]);
    Value::Struct(vec![
        Value::str(f.0.clone()),
        Value::Array(
            pat_ty,
            f.1.iter()
                .map(|(k, p)| Value::Struct(vec![Value::U32(*k), Value::str(p.clone())]))
                .collect(),
        ),
    ])
}

fn filter_ty() -> Ty {
    Ty::Struct(vec![
        Ty::Str,
        Ty::Array(Box::new(Ty::Struct(vec![Ty::U32, Ty::Str]))),
    ])
}

/// A unique name as a path element: `:1.42` is `1_42`.
fn sender_part(sender: &str) -> String {
    sender
        .trim_start_matches(':')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn token_ok(t: &str) -> bool {
    !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Question {
    fn from_options(mode: Mode, app: &str, title: &str, o: Option<&Value>) -> Question {
        // A program with no app entry is named by its path: its last part
        // says enough in a top bar.
        let app = app.rsplit('/').next().unwrap_or(app);
        let get = |k: &str| o.and_then(|o| o.get(k));
        let mut filters: Vec<Filter> = get("filters")
            .map(Value::inner)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(one_filter)
            .take(MAX_FILTERS)
            .collect();
        let current = get("current_filter").and_then(one_filter);
        let filter = current.map(|c| match filters.iter().position(|f| *f == c) {
            Some(i) => i,
            None => {
                filters.insert(0, c);
                0
            }
        });
        let filter = filter.or((!filters.is_empty()).then_some(0));
        let mode = if mode == Mode::Open && flag(get("directory")) {
            Mode::Folder
        } else {
            mode
        };
        let choices = get("choices")
            .map(Value::inner)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| {
                let c = c.as_struct()?;
                Some((
                    c.first()?.as_str()?.to_string(),
                    c.get(3)?.as_str()?.to_string(),
                ))
            })
            .take(MAX_FILTERS)
            .collect();
        let files = get("files")
            .map(Value::inner)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|f| {
                let b = f.as_bytes()?;
                let b = b.strip_suffix(&[0]).unwrap_or(b);
                (!b.is_empty() && !b.contains(&b'/') && !b.contains(&0)).then(|| b.to_vec())
            })
            .take(256)
            .collect();
        let current_file = bytes_path(get("current_file"));
        let name = match &current_file {
            Some(f) => f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None => text(get("current_name")),
        };
        let folder = bytes_path(get("current_folder")).or_else(|| {
            current_file
                .as_ref()
                .and_then(|f| f.parent())
                .map(Path::to_path_buf)
        });
        Question {
            mode,
            multiple: mode == Mode::Open && flag(get("multiple")),
            app: app.chars().take(16).collect(),
            title: title.chars().take(MAX_TEXT).collect(),
            accept: text(get("accept_label")).replace('_', ""),
            filters,
            filter,
            name,
            folder,
            files,
            choices,
        }
    }

    /// What `Take` gives the chooser.
    pub fn to_value(&self) -> Value {
        let mut v = vec![
            ("mode", Value::str(self.mode.word())),
            ("multiple", Value::Bool(self.multiple)),
            ("app", Value::str(self.app.clone())),
            ("title", Value::str(self.title.clone())),
            ("accept", Value::str(self.accept.clone())),
            (
                "filters",
                Value::Array(filter_ty(), self.filters.iter().map(filter_value).collect()),
            ),
            (
                "filter",
                Value::I32(self.filter.map(|f| f as i32).unwrap_or(-1)),
            ),
            ("name", Value::str(self.name.clone())),
        ];
        if let Some(f) = &self.folder {
            v.push(("folder", Value::Bytes(f.as_os_str().as_bytes().to_vec())));
        }
        Value::props(v)
    }
}

impl Portal {
    /// A new request's id and path, or None past the caller's limit.
    fn new_request(&mut self, m: &Message, sender: &str, kind: Kind) -> Option<(u64, String)> {
        if self.reqs.iter().filter(|r| r.sender == sender).count() >= PER_CALLER {
            return None;
        }
        self.next += 1;
        let id = self.next;
        let options = m.body.last();
        let token = text(options.and_then(|o| o.get("handle_token")));
        let token = if token_ok(&token) {
            token
        } else {
            format!("gh0st{id}")
        };
        let mut path = format!("{REQUEST_ROOT}/{}/{token}", sender_part(sender));
        if self.reqs.iter().any(|r| r.path == path) {
            path = format!("{path}_{id}");
        }
        self.reqs.push(Req {
            id,
            sender: sender.to_string(),
            path: path.clone(),
            kind,
            picker: None,
        });
        Some((id, path))
    }

    /// A call to the portal from `sender`, process `pid`, known to the user
    /// as `app`.
    pub fn call(&mut self, m: &Message, sender: &str, pid: Option<i32>, app: &str) -> Message {
        let path = m.path.as_deref().unwrap_or("");
        if path.starts_with(REQUEST_ROOT) {
            return self.request_call(m, sender, path);
        }
        if path.starts_with(SESSION_ROOT) {
            return self.session_call(m, sender);
        }
        if m.is(bus::PROPERTIES, "Get") || m.is(bus::PROPERTIES, "GetAll") {
            return properties(m);
        }
        if m.is(bus::INTROSPECTABLE, "Introspect") {
            return Message::reply(m).arg(Value::str(introspect()));
        }
        let pid = pid.unwrap_or(0);
        if m.interface.as_deref() == Some(INHIBIT) {
            return self.inhibit_call(m, sender, pid);
        }
        if m.is(TRASH, "TrashFile") {
            let ok = match m.body.first() {
                Some(Value::Fd(i)) => m.fds.get(*i as usize).is_some_and(trash_fd),
                _ => false,
            };
            return Message::reply(m).arg(Value::U32(ok as u32));
        }
        if m.interface.as_deref() == Some(WALLPAPER) {
            return self.wallpaper_call(m, sender, pid, app);
        }
        if m.is(ACCOUNT, "GetUserInformation") {
            return match self.new_request(m, sender, Kind::Account { app: app.into() }) {
                Some((id, rp)) => {
                    if self.remembered("account", app) {
                        self.account_answer(id, "once");
                    } else {
                        self.shell(&["account", &id.to_string(), &pid.to_string(), app]);
                    }
                    Message::reply(m).arg(Value::path(rp))
                }
                None => Message::error(m, bus::err::LIMITS, "too many requests"),
            };
        }
        let title = m.body.get(1).and_then(Value::as_str).unwrap_or("");
        let options = m.body.get(2);
        let mode = match (m.interface.as_deref(), m.member.as_deref()) {
            (Some(CHOOSER), Some("OpenFile")) => Some(Mode::Open),
            (Some(CHOOSER), Some("SaveFile")) => Some(Mode::Save),
            (Some(CHOOSER), Some("SaveFiles")) => Some(Mode::Folder),
            _ => None,
        };
        if let Some(mode) = mode {
            let mut q = Question::from_options(mode, app, title, options);
            if m.member.as_deref() == Some("SaveFiles") && q.accept.is_empty() {
                q.accept = "Save".into();
            }
            return match self.new_request(m, sender, Kind::Pick(Box::new(q))) {
                Some((id, rp)) => {
                    self.shell(&["pick", &id.to_string(), &pid.to_string()]);
                    Message::reply(m).arg(Value::path(rp))
                }
                None => Message::error(m, bus::err::LIMITS, "too many requests"),
            };
        }
        match (m.interface.as_deref(), m.member.as_deref()) {
            (Some(OPEN_URI), Some("SchemeSupported")) => {
                let scheme = m.body.first().and_then(Value::as_str).unwrap_or("");
                Message::reply(m).arg(Value::Bool(scheme == "file"))
            }
            (Some(OPEN_URI), Some(member @ ("OpenURI" | "OpenFile" | "OpenDirectory"))) => {
                let target = match member {
                    "OpenURI" => m
                        .body
                        .get(1)
                        .and_then(Value::as_str)
                        .and_then(gh0st_core::uri::to_path),
                    _ => match m.body.get(1) {
                        Some(Value::Fd(i)) => m.fds.get(*i as usize).and_then(fd_path),
                        _ => None,
                    },
                };
                let Some(mut target) = target else {
                    return Message::error(m, bus::err::INVALID_ARGS, "not a local file");
                };
                if member == "OpenDirectory" && !target.is_dir() {
                    target = target.parent().map(Path::to_path_buf).unwrap_or(target);
                }
                match self.new_request(m, sender, Kind::Open) {
                    Some((id, rp)) => {
                        self.shell(&[
                            "open",
                            &id.to_string(),
                            &pid.to_string(),
                            &gh0st_core::link::path_field(&target),
                        ]);
                        Message::reply(m).arg(Value::path(rp))
                    }
                    None => Message::error(m, bus::err::LIMITS, "too many requests"),
                }
            }
            _ => Message::error(m, bus::err::UNKNOWN_METHOD, "no such method"),
        }
    }

    fn inhibit_call(&mut self, m: &Message, sender: &str, pid: i32) -> Message {
        match m.member.as_deref().unwrap_or("") {
            "Inhibit" => {
                let flags = m.body.get(1).and_then(Value::as_u32).unwrap_or(0);
                let pid = if flags & KEEP_AWAKE != 0 { pid } else { 0 };
                match self.new_request(m, sender, Kind::Inhibit { pid }) {
                    Some((_, rp)) => {
                        self.awake(pid, true);
                        Message::reply(m).arg(Value::path(rp))
                    }
                    None => Message::error(m, bus::err::LIMITS, "too many requests"),
                }
            }
            "CreateMonitor" => {
                let token = text(m.body.get(1).and_then(|o| o.get("session_handle_token")));
                let token = if token_ok(&token) {
                    token
                } else {
                    format!("s{}", self.next + 1)
                };
                let session = format!("{SESSION_ROOT}/{}/{token}", sender_part(sender));
                let kind = Kind::Monitor {
                    session: session.clone(),
                };
                match self.new_request(m, sender, kind) {
                    Some((_, rp)) => {
                        let handle = vec![("session_handle", Value::path(session.clone()))];
                        self.respond(sender, &rp, RESPONSE_OK, handle);
                        // Running, and the screen saver is not on.
                        let mut s = Message::signal(PATH, INHIBIT, "StateChanged")
                            .arg(Value::path(session))
                            .arg(Value::props([
                                ("screensaver-active", Value::Bool(false)),
                                ("session-state", Value::U32(1)),
                            ]));
                        s.destination = Some(sender.to_string());
                        self.out.push(Out::Bus(s));
                        Message::reply(m).arg(Value::path(rp))
                    }
                    None => Message::error(m, bus::err::LIMITS, "too many requests"),
                }
            }
            "QueryEndResponse" => Message::reply(m),
            _ => Message::error(m, bus::err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// Keeps the app holding `pid` awake, or lets it sleep once no other
    /// inhibitor of that pid is left.
    fn awake(&mut self, pid: i32, on: bool) {
        if pid <= 0 {
            return;
        }
        let held = self
            .reqs
            .iter()
            .filter(|r| matches!(r.kind, Kind::Inhibit { pid: p } if p == pid))
            .count();
        if (on && held == 1) || (!on && held == 0) {
            self.shell(&["awake", &format!("pid:{pid}"), if on { "1" } else { "0" }]);
        }
    }

    /// Whether the user answered Always to `what` (account, wallpaper)
    /// from `app`. One `what<TAB>app` a line.
    fn remembered(&self, what: &str, app: &str) -> bool {
        let line = format!("{what}\t{app}");
        self.remember
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|t| t.lines().any(|l| l == line))
    }

    fn keep(&self, what: &str, app: &str) {
        if self.remembered(what, app) || app.contains(['\n', '\t']) {
            return;
        }
        if let Some(p) = &self.remember {
            let mut t = std::fs::read_to_string(p).unwrap_or_default();
            t.push_str(&format!("{what}\t{app}\n"));
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = gh0st_core::config::write_atomic(p, t.as_bytes());
        }
    }

    fn wallpaper_call(&mut self, m: &Message, sender: &str, pid: i32, app: &str) -> Message {
        let file = match m.member.as_deref() {
            Some("SetWallpaperURI") => m
                .body
                .get(1)
                .and_then(Value::as_str)
                .and_then(gh0st_core::uri::to_path)
                .and_then(|p| std::fs::File::open(p).ok()),
            Some("SetWallpaperFile") => match m.body.get(1) {
                Some(Value::Fd(i)) => m
                    .fds
                    .get(*i as usize)
                    .and_then(|f| f.try_clone().ok())
                    .map(std::fs::File::from),
                _ => None,
            },
            _ => return Message::error(m, bus::err::UNKNOWN_METHOD, "no such method"),
        };
        let Some(file) = file.filter(|f| f.metadata().is_ok_and(|md| md.is_file())) else {
            return Message::error(m, bus::err::INVALID_ARGS, "not a local file");
        };
        if self.job.is_some() {
            return Message::error(m, bus::err::LIMITS, "a wallpaper is being set");
        }
        let tmp = self
            .scratch
            .join(format!("wallpaper-{}.png", self.next + 1));
        let kind = Kind::Wallpaper {
            app: app.into(),
            pid,
            tmp: tmp.clone(),
        };
        let Some((id, rp)) = self.new_request(m, sender, kind) else {
            return Message::error(m, bus::err::LIMITS, "too many requests");
        };
        let _ = std::fs::create_dir_all(&self.scratch);
        let child = std::process::Command::new(&self.settings_bin)
            .arg("--import-wallpaper")
            .arg("-")
            .arg(&tmp)
            .stdin(file)
            .stdout(std::process::Stdio::piped())
            .spawn();
        match child {
            Ok(child) => self.job = Some(Job { id, child }),
            Err(e) => {
                eprintln!("gh0st session: {}: {e}", self.settings_bin.display());
                self.finish(id, RESPONSE_OTHER);
            }
        }
        Message::reply(m).arg(Value::path(rp))
    }

    /// The fitting child's pipe, to poll: it closes when the child is done.
    pub fn job_fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.job
            .as_ref()
            .and_then(|j| j.child.stdout.as_ref())
            .map(|o| o.as_raw_fd())
    }

    /// The fitting child is done: ask the user, or answer.
    pub fn job_done(&mut self) {
        let Some(mut j) = self.job.take() else {
            return;
        };
        let ok = j.child.wait().is_ok_and(|s| s.success());
        let Some(r) = self.reqs.iter().find(|r| r.id == j.id) else {
            return;
        };
        let Kind::Wallpaper { app, pid, tmp } = &r.kind else {
            return;
        };
        if !ok {
            return self.finish(j.id, RESPONSE_OTHER);
        }
        if self.remembered("wallpaper", app) {
            return self.wallpaper_answer(j.id, "once");
        }
        let fields = [
            "wallpaper".to_string(),
            j.id.to_string(),
            pid.to_string(),
            app.clone(),
            gh0st_core::link::path_field(tmp),
        ];
        self.out.push(Out::Shell(fields.to_vec()));
    }

    /// The user's answer about a wallpaper: once, always or deny.
    fn wallpaper_answer(&mut self, id: u64, answer: &str) {
        let Some(r) = self.reqs.iter().find(|r| r.id == id) else {
            return;
        };
        let Kind::Wallpaper { app, tmp, .. } = &r.kind else {
            return;
        };
        if answer == "deny" {
            return self.finish(id, RESPONSE_CANCELLED);
        }
        if answer == "always" {
            self.keep("wallpaper", app);
        }
        let dir = self.config.parent().unwrap_or(Path::new("."));
        let dst = gh0st_ui::wallpaper::picture_path(dir);
        let _ = std::fs::create_dir_all(dir);
        let kept = std::fs::rename(tmp, &dst).is_ok()
            && gh0st_core::config::Config::update(&self.config, |c| {
                c.set_str(gh0st_core::config::key::WALLPAPER, gh0st_ui::wallpaper::OWN)
            })
            .is_ok();
        self.finish(id, if kept { RESPONSE_OK } else { RESPONSE_OTHER });
    }

    /// Ends request `id` with `code`, and drops what it left behind.
    fn finish(&mut self, id: u64, code: u32) {
        let Some(at) = self.reqs.iter().position(|r| r.id == id) else {
            return;
        };
        let r = self.reqs.remove(at);
        self.forget(&r);
        self.respond(&r.sender, &r.path, code, Vec::new());
    }

    /// What a request leaves behind when it ends, answered or not.
    fn forget(&mut self, r: &Req) {
        if let Kind::Wallpaper { tmp, .. } = &r.kind {
            let _ = std::fs::remove_file(tmp);
            if let Some(mut j) = self.job.take_if(|j| j.id == r.id) {
                let _ = j.child.kill();
                let _ = j.child.wait();
            }
        }
    }

    /// A request ended without an answer: Close, or its caller left.
    fn drop_req(&mut self, r: Req) {
        self.forget(&r);
        match r.kind {
            Kind::Inhibit { pid } => self.awake(pid, false),
            Kind::Monitor { .. } => {}
            _ => self.shell(&["cancel", &r.id.to_string()]),
        }
    }

    /// The user's answer about their name: once, always or deny.
    fn account_answer(&mut self, id: u64, answer: &str) {
        let Some(at) = self.reqs.iter().position(|r| r.id == id) else {
            return;
        };
        let r = self.reqs.remove(at);
        let Kind::Account { app } = &r.kind else {
            return;
        };
        if answer == "deny" {
            self.respond(&r.sender, &r.path, RESPONSE_CANCELLED, Vec::new());
            return;
        }
        if answer == "always" {
            self.keep("account", app);
        }
        let (login, name) = user();
        let info = vec![
            ("id", Value::str(login)),
            ("name", Value::str(name)),
            ("image", Value::str("")),
        ];
        self.respond(&r.sender, &r.path, RESPONSE_OK, info);
    }

    fn request_call(&mut self, m: &Message, sender: &str, path: &str) -> Message {
        if m.is(bus::INTROSPECTABLE, "Introspect") {
            let xml = bus::introspect(
                &[
                    "<interface name=\"org.freedesktop.portal.Request\"><method name=\"Close\"/><signal name=\"Response\"><arg type=\"u\"/><arg type=\"a{sv}\"/></signal></interface>",
                ],
                &[],
            );
            return Message::reply(m).arg(Value::str(xml));
        }
        if !m.is(REQUEST, "Close") {
            return Message::error(m, bus::err::UNKNOWN_METHOD, "no such method");
        }
        match self.reqs.iter().position(|r| r.path == path) {
            Some(i) if self.reqs[i].sender == sender => {
                let r = self.reqs.remove(i);
                self.drop_req(r);
                Message::reply(m)
            }
            Some(_) => Message::error(m, bus::err::ACCESS_DENIED, "not yours"),
            None => Message::error(m, bus::err::UNKNOWN_OBJECT, "no such request"),
        }
    }

    /// `Take` and `Done` from the chooser, process `pid`.
    pub fn chooser_call(&mut self, m: &Message, pid: Option<i32>) -> Message {
        let id = m.body.first().and_then(|v| match v {
            Value::U64(n) => Some(*n),
            _ => None,
        });
        let at = id.and_then(|id| self.reqs.iter().position(|r| r.id == id));
        let Some(at) = at.filter(|&i| pid.is_some() && self.reqs[i].picker == pid) else {
            return Message::error(m, bus::err::ACCESS_DENIED, "not this request's chooser");
        };
        if m.is(OURS, "Take") {
            return match &self.reqs[at].kind {
                Kind::Pick(q) => Message::reply(m).arg(q.to_value()),
                _ => Message::error(m, bus::err::INVALID_ARGS, "not a chooser"),
            };
        }
        if !m.is(OURS, "Done") {
            return Message::error(m, bus::err::UNKNOWN_METHOD, "no such method");
        }
        let response = m
            .body
            .get(1)
            .and_then(Value::as_u32)
            .unwrap_or(RESPONSE_OTHER);
        let paths: Vec<PathBuf> = m
            .body
            .get(2)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| p.as_bytes())
            .filter(|b| b.first() == Some(&b'/') && !b.contains(&0))
            .map(|b| PathBuf::from(std::ffi::OsStr::from_bytes(b)))
            .collect();
        let filter = m.body.get(3).and_then(Value::as_i32);
        let r = self.reqs.remove(at);
        let Kind::Pick(q) = r.kind else {
            return Message::reply(m);
        };
        let mut results: Vec<(&str, Value)> = Vec::new();
        if response == RESPONSE_OK {
            let paths: Vec<PathBuf> = if q.files.is_empty() {
                paths
            } else {
                // SaveFiles: each name into the folder picked.
                let dir = paths.first().cloned().unwrap_or_default();
                q.files
                    .iter()
                    .map(|n| dir.join(std::ffi::OsStr::from_bytes(n)))
                    .collect()
            };
            results.push((
                "uris",
                Value::strs(paths.iter().map(|p| gh0st_core::uri::from_path(p))),
            ));
            let f = filter
                .and_then(|f| usize::try_from(f).ok())
                .and_then(|f| q.filters.get(f));
            if let Some(f) = f {
                results.push(("current_filter", filter_value(f)));
            }
            if !q.choices.is_empty() {
                let pairs = q.choices.iter().map(|(a, b)| {
                    Value::Struct(vec![Value::str(a.clone()), Value::str(b.clone())])
                });
                results.push((
                    "choices",
                    Value::Array(Ty::Struct(vec![Ty::Str, Ty::Str]), pairs.collect()),
                ));
            }
        }
        self.respond(&r.sender, &r.path, response, results);
        Message::reply(m)
    }

    /// A line from the shell about request `id`.
    pub fn shell_said(&mut self, fields: &[String]) {
        let verb = fields.first().map(String::as_str).unwrap_or("");
        let Some(id) = fields.get(1).and_then(|v| v.parse::<u64>().ok()) else {
            return;
        };
        if verb == "account" || verb == "wallpaper" {
            let answer = fields.get(2).map(String::as_str).unwrap_or("deny");
            return match verb {
                "account" => self.account_answer(id, answer),
                _ => self.wallpaper_answer(id, answer),
            };
        }
        let Some(at) = self.reqs.iter().position(|r| r.id == id) else {
            return;
        };
        match verb {
            "picker" => self.reqs[at].picker = fields.get(2).and_then(|p| p.parse().ok()),
            // The app wasn't in front, or the device is locked.
            "refuse" => {
                let r = self.reqs.remove(at);
                self.respond(&r.sender, &r.path, RESPONSE_OTHER, Vec::new());
            }
            // The chooser ended without an answer: the user gave up.
            "gone" => {
                let r = self.reqs.remove(at);
                self.respond(&r.sender, &r.path, RESPONSE_CANCELLED, Vec::new());
            }
            "done" => {
                let r = self.reqs.remove(at);
                self.respond(&r.sender, &r.path, RESPONSE_OK, Vec::new());
            }
            _ => {}
        }
    }

    /// A caller left the bus: its requests go, its inhibitors let go, and
    /// its choosers close.
    pub fn gone(&mut self, sender: &str) {
        let (theirs, rest): (Vec<Req>, Vec<Req>) = std::mem::take(&mut self.reqs)
            .into_iter()
            .partition(|r| r.sender == sender);
        self.reqs = rest;
        for r in theirs {
            self.drop_req(r);
        }
    }

    /// A call to a monitor's session object: Close ends it.
    pub fn session_call(&mut self, m: &Message, sender: &str) -> Message {
        let path = m.path.as_deref().unwrap_or("");
        if !m.is(SESSION, "Close") {
            return Message::error(m, bus::err::UNKNOWN_METHOD, "no such method");
        }
        self.reqs.retain(|r| {
            !(r.sender == sender && matches!(&r.kind, Kind::Monitor { session } if session == path))
        });
        Message::reply(m)
    }

    /// `gh0st session --test` with no shell: answers request `id` at once,
    /// as a chooser that picked `path` would, so real clients can be
    /// tested against the bus.
    pub fn answer_for_test(&mut self, id: u64, path: &Path) {
        let Some(at) = self.reqs.iter().position(|r| r.id == id) else {
            return;
        };
        let r = self.reqs.remove(at);
        let results = match r.kind {
            Kind::Pick(_) => vec![("uris", Value::strs([gh0st_core::uri::from_path(path)]))],
            _ => Vec::new(),
        };
        self.respond(&r.sender, &r.path, RESPONSE_OK, results);
    }

    pub fn open(&self) -> usize {
        self.reqs.len()
    }

    fn respond(&mut self, sender: &str, path: &str, code: u32, results: Vec<(&str, Value)>) {
        let mut s = Message::signal(path, REQUEST, "Response")
            .arg(Value::U32(code))
            .arg(Value::props(results));
        s.destination = Some(sender.to_string());
        self.out.push(Out::Bus(s));
    }

    fn shell(&mut self, fields: &[&str]) {
        self.out
            .push(Out::Shell(fields.iter().map(|f| f.to_string()).collect()));
    }
}

/// The file an fd we were handed names.
/// Moves the file `fd` names to the Trash, if the name still is that file.
fn trash_fd(fd: &std::os::fd::OwnedFd) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(path) = fd_path(fd) else {
        return false;
    };
    let Ok(held) = fd
        .try_clone()
        .map(std::fs::File::from)
        .and_then(|f| f.metadata())
    else {
        return false;
    };
    let same = std::fs::symlink_metadata(&path)
        .is_ok_and(|md| md.dev() == held.dev() && md.ino() == held.ino());
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    same && gh0st_core::trash::put(Path::new(&home), &path).is_ok()
}

fn fd_path(fd: &std::os::fd::OwnedFd) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;
    let p = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
    (p.is_absolute() && p.exists()).then_some(p)
}

fn properties(m: &Message) -> Message {
    let iface = m.body.first().and_then(Value::as_str).unwrap_or("");
    let version = match iface {
        CHOOSER => CHOOSER_VERSION,
        OPEN_URI => OPEN_URI_VERSION,
        NOTIFICATION => NOTIFICATION_VERSION,
        INHIBIT => INHIBIT_VERSION,
        TRASH => TRASH_VERSION,
        WALLPAPER => WALLPAPER_VERSION,
        ACCOUNT => ACCOUNT_VERSION,
        crate::settings::IFACE => crate::settings::VERSION,
        _ => return Message::error(m, bus::err::UNKNOWN_INTERFACE, "no such interface"),
    };
    if m.is(bus::PROPERTIES, "GetAll") {
        return Message::reply(m).arg(Value::props([("version", Value::U32(version))]));
    }
    match m.body.get(1).and_then(Value::as_str) {
        Some("version") => Message::reply(m).arg(Value::variant(Value::U32(version))),
        _ => Message::error(m, bus::err::UNKNOWN_PROPERTY, "no such property"),
    }
}

fn introspect() -> String {
    const CHOOSER_XML: &str = r#"<interface name="org.freedesktop.portal.FileChooser"><method name="OpenFile"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="SaveFile"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="SaveFiles"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><property name="version" type="u" access="read"/></interface>"#;
    const OPEN_URI_XML: &str = r#"<interface name="org.freedesktop.portal.OpenURI"><method name="OpenURI"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="OpenFile"><arg type="s" direction="in"/><arg type="h" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="OpenDirectory"><arg type="s" direction="in"/><arg type="h" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="SchemeSupported"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="b" direction="out"/></method><property name="version" type="u" access="read"/></interface>"#;
    const NOTIFICATION_XML: &str = r#"<interface name="org.freedesktop.portal.Notification"><method name="AddNotification"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/></method><method name="RemoveNotification"><arg type="s" direction="in"/></method><signal name="ActionInvoked"><arg type="s"/><arg type="s"/><arg type="av"/></signal><property name="version" type="u" access="read"/></interface>"#;
    const INHIBIT_XML: &str = r#"<interface name="org.freedesktop.portal.Inhibit"><method name="Inhibit"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="CreateMonitor"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="QueryEndResponse"><arg type="o" direction="in"/></method><signal name="StateChanged"><arg type="o"/><arg type="a{sv}"/></signal><property name="version" type="u" access="read"/></interface>"#;
    const ACCOUNT_XML: &str = r#"<interface name="org.freedesktop.portal.Account"><method name="GetUserInformation"><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><property name="version" type="u" access="read"/></interface>"#;
    const TRASH_XML: &str = r#"<interface name="org.freedesktop.portal.Trash"><method name="TrashFile"><arg type="h" direction="in"/><arg type="u" direction="out"/></method><property name="version" type="u" access="read"/></interface>"#;
    const WALLPAPER_XML: &str = r#"<interface name="org.freedesktop.portal.Wallpaper"><method name="SetWallpaperURI"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><method name="SetWallpaperFile"><arg type="s" direction="in"/><arg type="h" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method><property name="version" type="u" access="read"/></interface>"#;
    bus::introspect(
        &[
            CHOOSER_XML,
            OPEN_URI_XML,
            NOTIFICATION_XML,
            INHIBIT_XML,
            ACCOUNT_XML,
            TRASH_XML,
            WALLPAPER_XML,
            crate::settings::XML,
            bus::PROPERTIES_XML,
        ],
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = ":1.42";

    fn call(iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::call(NAME, PATH, iface, member);
        m.sender = Some(ME.into());
        m.serial = 7;
        m.body = body;
        m
    }

    fn open_file(options: Value) -> Message {
        call(
            CHOOSER,
            "OpenFile",
            vec![Value::str(""), Value::str("Open a file"), options],
        )
    }

    fn shell_lines(p: &mut Portal) -> Vec<Vec<String>> {
        let out = std::mem::take(&mut p.out);
        out.into_iter()
            .filter_map(|o| match o {
                Out::Shell(f) => Some(f),
                Out::Bus(_) => None,
            })
            .collect()
    }

    fn responses(p: &mut Portal) -> Vec<(String, u32, Value)> {
        let out = std::mem::take(&mut p.out);
        out.into_iter()
            .filter_map(|o| match o {
                Out::Bus(m) if m.is(REQUEST, "Response") => Some((
                    m.path.clone().unwrap_or_default(),
                    m.body[0].as_u32().unwrap(),
                    m.body[1].clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn chooser(member: &str, pid: i32, body: Vec<Value>) -> (Message, Option<i32>) {
        let mut m = Message::call(NAME, OURS_PATH, OURS, member);
        m.sender = Some(":1.99".into());
        m.body = body;
        (m, Some(pid))
    }

    #[test]
    fn a_pick_asks_the_shell_and_answers_through_its_request() {
        let mut p = Portal::default();
        let options = Value::props([
            ("handle_token", Value::str("t1")),
            ("multiple", Value::Bool(true)),
            (
                "filters",
                Value::Array(
                    filter_ty(),
                    vec![filter_value(&(
                        "Pictures".into(),
                        vec![(1, "image/*".into())],
                    ))],
                ),
            ),
        ]);
        let reply = p.call(&open_file(options), ME, Some(500), "Text");
        let path = "/org/freedesktop/portal/desktop/request/1_42/t1";
        assert_eq!(reply.body[0].as_str(), Some(path));
        assert_eq!(shell_lines(&mut p), [["pick", "1", "500"]]);
        // A stranger can't take the question; the chooser the shell
        // started can.
        let (take, _) = chooser("Take", 777, vec![Value::U64(1)]);
        assert!(p.chooser_call(&take, Some(777)).error_name.is_some());
        p.shell_said(&["picker".into(), "1".into(), "777".into()]);
        let q = p.chooser_call(&take, Some(777));
        assert!(q.error_name.is_none());
        assert_eq!(
            q.body[0]
                .get("mode")
                .map(Value::inner)
                .and_then(Value::as_str),
            Some("open")
        );
        assert_eq!(
            q.body[0]
                .get("app")
                .map(Value::inner)
                .and_then(Value::as_str),
            Some("Text")
        );
        let pics = Value::Array(
            Ty::Array(Box::new(Ty::Byte)),
            vec![Value::Bytes(b"/home/u/a b.png".to_vec())],
        );
        let (done, pid) = chooser(
            "Done",
            777,
            vec![Value::U64(1), Value::U32(0), pics, Value::I32(0)],
        );
        assert!(p.chooser_call(&done, pid).error_name.is_none());
        let r = responses(&mut p);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].0.as_str(), r[0].1), (path, RESPONSE_OK));
        let uris = r[0]
            .2
            .get("uris")
            .map(Value::inner)
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(uris[0].as_str(), Some("file:///home/u/a%20b.png"));
        assert!(r[0].2.get("current_filter").is_some());
        assert_eq!(p.open(), 0);
    }

    #[test]
    fn refusals_closes_and_departures_end_requests() {
        let mut p = Portal::default();
        p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A");
        p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A");
        p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A");
        p.out.clear();
        p.shell_said(&["refuse".into(), "1".into()]);
        assert_eq!(responses(&mut p)[0].1, RESPONSE_OTHER);
        p.shell_said(&["gone".into(), "2".into()]);
        assert_eq!(responses(&mut p)[0].1, RESPONSE_CANCELLED);
        // Close from someone else is refused; from the caller, it ends quietly.
        let path = "/org/freedesktop/portal/desktop/request/1_42/gh0st3";
        let mut close = Message::call(NAME, path, REQUEST, "Close");
        close.sender = Some(":1.5".into());
        assert!(p.call(&close, ":1.5", None, "x").error_name.is_some());
        close.sender = Some(ME.into());
        assert!(p.call(&close, ME, None, "x").error_name.is_none());
        assert_eq!(shell_lines(&mut p), [["cancel", "3"]]);
        p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A");
        p.out.clear();
        p.gone(ME);
        assert_eq!(shell_lines(&mut p), [["cancel", "4"]]);
        assert_eq!(p.open(), 0);
    }

    #[test]
    fn inhibit_keeps_the_caller_awake_until_its_last_request_goes() {
        let mut p = Portal::default();
        let idle = |flags| {
            call(
                INHIBIT,
                "Inhibit",
                vec![Value::str(""), Value::U32(flags), Value::props::<&str>([])],
            )
        };
        let first = p.call(&idle(8), ME, Some(300), "mpv").body[0]
            .as_str()
            .unwrap()
            .to_string();
        p.call(&idle(8), ME, Some(300), "mpv");
        p.call(&idle(1), ME, Some(300), "mpv");
        assert_eq!(shell_lines(&mut p), [["awake", "pid:300", "1"]]);
        let mut close = Message::call(NAME, &first, REQUEST, "Close");
        close.sender = Some(ME.into());
        p.call(&close, ME, None, "mpv");
        assert!(shell_lines(&mut p).is_empty(), "the second still holds");
        p.gone(ME);
        assert_eq!(shell_lines(&mut p), [["awake", "pid:300", "0"]]);
        let monitor = call(
            INHIBIT,
            "CreateMonitor",
            vec![
                Value::str(""),
                Value::props([("session_handle_token", Value::str("m"))]),
            ],
        );
        p.call(&monitor, ME, Some(300), "mpv");
        let session = "/org/freedesktop/portal/desktop/session/1_42/m";
        let r = responses(&mut p);
        assert_eq!(
            r[0].2
                .get("session_handle")
                .map(Value::inner)
                .and_then(Value::as_str),
            Some(session)
        );
        let mut end = Message::call(NAME, session, SESSION, "Close");
        end.sender = Some(ME.into());
        assert!(p.call(&end, ME, None, "mpv").error_name.is_none());
        assert_eq!(p.open(), 0);
    }

    #[test]
    fn the_users_name_needs_a_yes_and_always_is_kept() {
        let dir = std::env::temp_dir().join(format!("gh0st-account-{}", std::process::id()));
        let mut p = Portal {
            remember: Some(dir.join("portal-account")),
            ..Portal::default()
        };
        let ask = || {
            call(
                ACCOUNT,
                "GetUserInformation",
                vec![Value::str(""), Value::props::<&str>([])],
            )
        };
        p.call(&ask(), ME, Some(300), "Chat");
        assert_eq!(shell_lines(&mut p), [["account", "1", "300", "Chat"]]);
        p.shell_said(&["account".into(), "1".into(), "deny".into()]);
        assert_eq!(responses(&mut p)[0].1, RESPONSE_CANCELLED);
        p.call(&ask(), ME, Some(300), "Chat");
        p.out.clear();
        p.shell_said(&["account".into(), "2".into(), "always".into()]);
        let r = responses(&mut p);
        assert_eq!(r[0].1, RESPONSE_OK);
        assert!(r[0].2.get("name").is_some() && r[0].2.get("id").is_some());
        // Asked again, the answer comes without the shell.
        p.call(&ask(), ME, Some(300), "Chat");
        assert_eq!(responses(&mut p)[0].1, RESPONSE_OK);
        assert!(shell_lines(&mut p).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wallpaper_is_fitted_then_kept_only_after_a_yes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gh0st-wall-portal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Stands in for gh0st-settings: the fitted picture is the input.
        let fit = dir.join("fit");
        std::fs::write(&fit, "#!/bin/sh\ncat > \"$3\"\n").unwrap();
        std::fs::set_permissions(&fit, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("pic.png"), b"picture").unwrap();
        let mut p = Portal {
            remember: Some(dir.join("always")),
            config: dir.join("conf/config.json"),
            scratch: dir.join("cache"),
            settings_bin: fit,
            ..Portal::default()
        };
        let set = || {
            let mut m = call(
                WALLPAPER,
                "SetWallpaperFile",
                vec![Value::str(""), Value::Fd(0), Value::props::<&str>([])],
            );
            m.fds = vec![std::fs::File::open(dir.join("pic.png")).unwrap().into()];
            m
        };
        assert!(p.call(&set(), ME, Some(300), "Media").error_name.is_none());
        assert!(
            p.call(&set(), ME, Some(300), "Media").error_name.is_some(),
            "one at a time"
        );
        p.job_done();
        let asked = shell_lines(&mut p);
        assert_eq!(asked[0][..4], ["wallpaper", "1", "300", "Media"]);
        let tmp = gh0st_core::link::path_of(&asked[0][4]);
        assert_eq!(std::fs::read(&tmp).unwrap(), b"picture");
        p.shell_said(&["wallpaper".into(), "1".into(), "deny".into()]);
        assert_eq!(responses(&mut p)[0].1, RESPONSE_CANCELLED);
        assert!(!tmp.exists());

        p.call(&set(), ME, Some(300), "Media");
        p.job_done();
        p.out.clear();
        p.shell_said(&["wallpaper".into(), "2".into(), "always".into()]);
        assert_eq!(responses(&mut p)[0].1, RESPONSE_OK);
        assert_eq!(
            std::fs::read(dir.join("conf/wallpaper.png")).unwrap(),
            b"picture"
        );
        let conf = gh0st_core::config::Config::load(&dir.join("conf/config.json"));
        assert_eq!(conf.wallpaper(), gh0st_ui::wallpaper::OWN);
        // Always: the next one is kept without a question.
        p.call(&set(), ME, Some(300), "Media");
        p.job_done();
        assert_eq!(responses(&mut p)[0].1, RESPONSE_OK);
        assert!(shell_lines(&mut p).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_takes_the_file_its_fd_names() {
        let mut m = call(TRASH, "TrashFile", vec![Value::Fd(0)]);
        m.fds = vec![std::fs::File::open("/").unwrap().into()];
        // No name to move: refused, answered 0.
        assert_eq!(
            Portal::default().call(&m, ME, Some(1), "A").body[0].as_u32(),
            Some(0)
        );
    }

    #[test]
    fn a_caller_has_a_few_requests_at_most() {
        let mut p = Portal::default();
        for _ in 0..PER_CALLER {
            assert!(
                p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A")
                    .error_name
                    .is_none()
            );
        }
        assert!(
            p.call(&open_file(Value::props::<&str>([])), ME, Some(1), "A")
                .error_name
                .is_some()
        );
    }

    #[test]
    fn options_shape_the_question() {
        let folder = Value::Bytes(b"/home/u/Docs\0".to_vec());
        let q = Question::from_options(
            Mode::Save,
            "Text",
            "Save",
            Some(&Value::props([
                ("current_name", Value::str("notes.txt")),
                ("current_folder", folder),
                ("accept_label", Value::str("_Export")),
            ])),
        );
        assert_eq!(q.name, "notes.txt");
        assert_eq!(q.folder, Some("/home/u/Docs".into()));
        assert_eq!(q.accept, "Export");
        let q = Question::from_options(Mode::Open, "/home/lol/bin/pick", "", None);
        assert_eq!(q.app, "pick");
        let q = Question::from_options(
            Mode::Open,
            "A",
            "",
            Some(&Value::props([("directory", Value::Bool(true))])),
        );
        assert_eq!(q.mode, Mode::Folder);
        // Junk where a dict should be is no options at all.
        let q = Question::from_options(Mode::Open, "A", "", Some(&Value::U32(3)));
        assert!(q.filters.is_empty() && q.folder.is_none());
    }

    #[test]
    fn open_uri_takes_local_files_and_the_version_shows() {
        let mut p = Portal::default();
        let m = call(
            OPEN_URI,
            "OpenURI",
            vec![
                Value::str(""),
                Value::str("file:///tmp"),
                Value::props::<&str>([]),
            ],
        );
        assert!(p.call(&m, ME, Some(9), "A").error_name.is_none());
        assert_eq!(shell_lines(&mut p), [["open", "1", "9", "/tmp"]]);
        let web = call(
            OPEN_URI,
            "OpenURI",
            vec![
                Value::str(""),
                Value::str("https://x"),
                Value::props::<&str>([]),
            ],
        );
        assert!(p.call(&web, ME, Some(9), "A").error_name.is_some());
        let ask = call(
            OPEN_URI,
            "SchemeSupported",
            vec![Value::str("file"), Value::props::<&str>([])],
        );
        assert_eq!(p.call(&ask, ME, None, "A").body[0], Value::Bool(true));
        let mut get = Message::call(NAME, PATH, bus::PROPERTIES, "Get");
        get.body = vec![Value::str(CHOOSER), Value::str("version")];
        assert_eq!(p.call(&get, ME, None, "A").body[0].inner(), &Value::U32(4));
    }
}
