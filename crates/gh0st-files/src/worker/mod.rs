//! The worker: `gh0st-files --worker`. One thread, one `poll` loop. It runs
//! one request at a time; while one waits on the network or a child, the
//! pump keeps reading the page's link, so answers and cancels arrive.

mod archive;
pub mod askpass;
mod copy;
mod curl;
mod local;
pub mod parse;
pub mod root;
mod sftp;

use crate::child::FD_ENV;
use crate::engine::{Ask, Event, Id, Request};
use crate::model::{Entry, Loc};
use crate::proto::{self, Conn};
use crate::servers::{Proto, Server};
use askpass::Askpass;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub type Res<T> = Result<T, String>;

/// What a request ended with when the user stopped it.
pub const STOPPED: &str = "Stopped";

/// A place files live: the card, or a server. Paths are absolute.
pub trait Fs {
    fn list(&mut self, p: &mut Pump, path: &str) -> Res<Vec<Entry>>;
    /// None when nothing is there.
    fn stat(&mut self, p: &mut Pump, path: &str) -> Res<Option<Entry>>;
    /// Copies the file `path` to the local file `to`.
    fn get(&mut self, p: &mut Pump, path: &str, to: &Path, prog: &mut Prog) -> Res<()>;
    /// Copies the local file `from` to the file `path`.
    fn put(&mut self, p: &mut Pump, from: &Path, path: &str, prog: &mut Prog) -> Res<()>;
    fn mkdir(&mut self, p: &mut Pump, path: &str) -> Res<()>;
    fn remove(&mut self, p: &mut Pump, path: &str, dir: bool) -> Res<()>;
    fn rename(&mut self, p: &mut Pump, from: &str, to: &str) -> Res<()>;
    /// Removes a whole folder in one step, where the place can.
    fn remove_tree(&mut self, p: &mut Pump, path: &str) -> Option<Res<()>> {
        let _ = (p, path);
        None
    }
    /// The full path of "~", the home folder.
    fn home(&mut self, p: &mut Pump) -> Res<String> {
        let _ = p;
        Ok("/".into())
    }
}

/// The page's link, the askpass socket, and what arrived while a request
/// ran.
pub struct Pump {
    link: Conn,
    askpass: Option<Askpass>,
    /// Askpass prompts sent to the page, waiting for its answer.
    asks: Vec<(Id, std::os::unix::net::UnixStream)>,
    queue: VecDeque<Request>,
    replies: HashMap<Id, Option<String>>,
    cancelled: HashSet<Id>,
    /// The request running and the server it is on, for askpass prompts:
    /// and how many passwords it has asked for.
    pub current: Option<(Id, String, u32)>,
    /// Answers every askpass prompt with this, as making a key does.
    pub auto: Option<String>,
    /// The app's own SSH key: only a prompt naming it gets the passphrase.
    pub key: Option<PathBuf>,
    /// Set while a stopped request cleans up after itself, so the cleanup
    /// isn't stopped too.
    pub hold_cancel: bool,
    closed: bool,
}

impl Pump {
    fn new(link: Conn) -> Pump {
        Pump {
            link,
            askpass: Askpass::open().ok(),
            asks: Vec::new(),
            queue: VecDeque::new(),
            replies: HashMap::new(),
            cancelled: HashSet::new(),
            current: None,
            auto: None,
            key: None,
            hold_cancel: false,
            closed: false,
        }
    }

    pub fn send(&mut self, e: &Event) {
        for line in proto::event(e) {
            if self.link.send(&line).is_err() {
                self.closed = true;
                return;
            }
        }
    }

    pub fn cancelled(&self, id: Id) -> bool {
        !self.hold_cancel && (self.closed || self.cancelled.contains(&id))
    }

    /// The askpass socket's path, for ssh's env.
    pub fn askpass_path(&self) -> Option<&Path> {
        self.askpass.as_ref().map(Askpass::path)
    }

    /// Waits up to `ms` (-1 for ever) for one of `fds`, reading the page and
    /// askpass meanwhile. Returns each of `fds`' revents.
    pub fn wait(&mut self, fds: &[(RawFd, i16)], ms: i32) -> Vec<i16> {
        let mut pf = vec![libc::pollfd {
            fd: self.link.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        if let Some(a) = &self.askpass {
            pf.push(libc::pollfd {
                fd: a.fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let base = pf.len();
        pf.extend(fds.iter().map(|&(fd, events)| libc::pollfd {
            fd,
            events,
            revents: 0,
        }));
        // SAFETY: polls a live array of its own length.
        let n = unsafe { libc::poll(pf.as_mut_ptr(), pf.len() as libc::nfds_t, ms) };
        if n > 0 {
            if pf[0].revents != 0 {
                self.read_link();
            }
            if base == 2 && pf[1].revents != 0 {
                self.accept_askpass();
            }
        }
        pf[base..].iter().map(|p| p.revents).collect()
    }

    fn read_link(&mut self) {
        for f in self.link.recv() {
            match proto::parse_request(&f) {
                Some(Request::Reply { id, text }) => {
                    if let Some(i) = self.asks.iter().position(|a| a.0 == id) {
                        let (_, mut s) = self.asks.remove(i);
                        askpass::answer(&mut s, text.as_deref());
                    } else {
                        self.replies.insert(id, text);
                    }
                }
                Some(Request::Cancel { id }) => {
                    self.cancelled.insert(id);
                    if let Some(i) = self.queue.iter().position(|r| r.id() == Some(id)) {
                        self.queue.remove(i);
                        self.send(&Event::Error {
                            id,
                            text: STOPPED.into(),
                        });
                    }
                    if let Some(i) = self.asks.iter().position(|a| a.0 == id) {
                        let (_, mut s) = self.asks.remove(i);
                        askpass::answer(&mut s, None);
                    }
                }
                Some(r) => self.queue.push_back(r),
                None => {}
            }
        }
        if self.link.eof() {
            self.closed = true;
        }
    }

    fn accept_askpass(&mut self) {
        let Some(a) = &self.askpass else {
            return;
        };
        let Some((mut stream, prompt)) = a.accept() else {
            return;
        };
        if let Some(auto) = self.auto.clone() {
            askpass::answer(&mut stream, Some(&auto));
            return;
        }
        let Some((id, server, asked)) = self.current.clone() else {
            askpass::answer(&mut stream, None);
            return;
        };
        let ask = askpass::classify(&prompt, &server, asked > 0, self.key.as_deref());
        if matches!(ask, Ask::Password { .. }) {
            if let Some(c) = self.current.as_mut() {
                c.2 += 1;
            }
        }
        self.send(&Event::Ask { id, ask });
        self.asks.push((id, stream));
    }

    /// Asks the page and waits for its answer. None is a no.
    pub fn ask(&mut self, id: Id, ask: Ask) -> Option<String> {
        self.send(&Event::Ask { id, ask });
        loop {
            if let Some(a) = self.replies.remove(&id) {
                return a;
            }
            if self.cancelled(id) {
                return None;
            }
            self.wait(&[], -1);
        }
    }
}

/// How far a copy has come, told to the page a few times a second.
pub struct Prog {
    pub id: Id,
    pub done: u64,
    pub total: u64,
    name: String,
    last: Instant,
    /// Checks for a cancel, tells the page nothing.
    silent: bool,
}

impl Prog {
    pub fn new(id: Id, total: u64) -> Prog {
        Prog {
            id,
            done: 0,
            total,
            name: String::new(),
            last: Instant::now() - Duration::from_secs(1),
            silent: false,
        }
    }

    /// For a step whose bytes another bar counts.
    pub fn silent(id: Id) -> Prog {
        Prog {
            silent: true,
            ..Prog::new(id, 0)
        }
    }

    pub fn file(&mut self, p: &mut Pump, name: &str) {
        self.name = name.to_string();
        self.tell(p, true);
    }

    /// `n` more bytes. Err once the user stopped it.
    pub fn add(&mut self, p: &mut Pump, n: u64) -> Res<()> {
        self.done += n;
        self.tell(p, false);
        if p.cancelled(self.id) {
            return Err(STOPPED.into());
        }
        Ok(())
    }

    fn tell(&mut self, p: &mut Pump, now: bool) {
        if !now && self.last.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last = Instant::now();
        if self.silent {
            p.wait(&[], 0);
            return;
        }
        p.send(&Event::Progress {
            id: self.id,
            done: self.done.min(self.total),
            total: self.total,
            name: self.name.clone(),
        });
        // Reads a cancel that came meanwhile.
        p.wait(&[], 0);
    }
}

/// Runs a child to its end, its output read through the pump. `tick` sees
/// each piece of stderr, and a call every 250 ms. A cancel of `id` kills it.
pub fn run(
    p: &mut Pump,
    cmd: &mut Command,
    input: Option<&[u8]>,
    id: Option<Id>,
    tick: &mut dyn FnMut(&mut Pump, &str) -> Res<()>,
) -> Res<(i32, Vec<u8>, String)> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Can't run {:?}: {e}", cmd.get_program()))?;
    if let (Some(bytes), Some(mut sin)) = (input, child.stdin.take()) {
        use std::io::Write;
        let _ = sin.write_all(bytes);
    }
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut err = child.stderr.take().ok_or("no stderr")?;
    for fd in [out.as_raw_fd(), err.as_raw_fd()] {
        nonblock(fd);
    }
    let (mut stdout, mut stderr) = (Vec::new(), String::new());
    let (mut out_open, mut err_open) = (true, true);
    let mut buf = [0u8; 16384];
    while out_open || err_open {
        if id.is_some_and(|id| p.cancelled(id)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(STOPPED.into());
        }
        let mut fds = Vec::new();
        if out_open {
            fds.push((out.as_raw_fd(), libc::POLLIN));
        }
        if err_open {
            fds.push((err.as_raw_fd(), libc::POLLIN));
        }
        p.wait(&fds, 250);
        loop {
            match out.read(&mut buf) {
                Ok(0) => {
                    out_open = false;
                    break;
                }
                Ok(n) => stdout.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        let mut piece = String::new();
        loop {
            match err.read(&mut buf) {
                Ok(0) => {
                    err_open = false;
                    break;
                }
                Ok(n) => piece.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => break,
            }
        }
        stderr.push_str(&piece);
        if let Err(e) = tick(p, &piece) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    Ok((status.code().unwrap_or(-1), stdout, stderr))
}

pub fn nonblock(fd: RawFd) {
    // SAFETY: fcntl on an fd the caller owns.
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
    }
}

/// The servers, their open sessions, and where the worker keeps files.
pub struct Backends {
    servers: HashMap<String, Server>,
    sftp: HashMap<String, sftp::Sftp>,
    curl: HashMap<String, curl::Curl>,
    local: local::Local,
    /// Root mode: the card as root, the user's folders as the user.
    pub root: Option<root::RootLocal>,
    pub home: PathBuf,
    pub data: PathBuf,
}

impl Backends {
    fn new(home: PathBuf, data: PathBuf) -> Backends {
        Backends {
            servers: HashMap::new(),
            sftp: HashMap::new(),
            curl: HashMap::new(),
            local: local::Local,
            root: None,
            home,
            data,
        }
    }

    /// Runs `f` as whoever `path`'s folder belongs to, in root mode; as
    /// the worker is, otherwise.
    pub fn act<T>(&self, path: &Path, f: impl FnOnce() -> Res<T>) -> Res<T> {
        match &self.root {
            Some(r) => r.ident.run(root::whose(path, r.ident.uid), f),
            None => f(),
        }
    }

    /// Where files wait between two servers, or before an archive opens.
    pub fn stage(&self) -> PathBuf {
        let d = self.data.join("stage");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn server(&self, name: &str) -> Res<Server> {
        self.servers
            .get(name)
            .cloned()
            .ok_or_else(|| format!("No server named {name}"))
    }

    /// The place `l` is on, and its path there.
    pub fn fs(&mut self, p: &mut Pump, l: &Loc) -> Res<(&mut dyn Fs, String)> {
        match l {
            Loc::Local(path) => {
                let path = path.to_string_lossy().into_owned();
                match &mut self.root {
                    Some(r) => Ok((r, path)),
                    None => Ok((&mut self.local, path)),
                }
            }
            Loc::Remote { server, path } => {
                let s = self.server(server)?;
                let fs: &mut dyn Fs = match s.proto {
                    Proto::Sftp => {
                        let alive = self.sftp.get_mut(server).is_some_and(|x| x.alive());
                        if !alive {
                            self.sftp.remove(server);
                            let ssh = self.data.join("ssh");
                            // ssh's questions name the server it connects to,
                            // whichever the request is about.
                            let was = p.current.clone();
                            if let Some(c) = p.current.as_mut() {
                                *c = (c.0, server.clone(), 0);
                            }
                            let x = sftp::Sftp::connect(p, &s, &ssh);
                            p.current = was;
                            let x = x?;
                            self.sftp.insert(server.clone(), x);
                        }
                        self.sftp.get_mut(server).ok_or("no session")?
                    }
                    _ => self
                        .curl
                        .entry(server.clone())
                        .or_insert_with(|| curl::Curl::new(s)),
                };
                Ok((fs, path.clone()))
            }
            _ => Err("Not a place files can be".into()),
        }
    }

    pub fn stat(&mut self, p: &mut Pump, l: &Loc) -> Res<Option<Entry>> {
        let (fs, path) = self.fs(p, l)?;
        fs.stat(p, &path)
    }

    /// A folder's entries. A name that could reach outside the folder
    /// (empty, ".", "..", a slash, a NUL) is a server lying, and left out.
    pub fn list(&mut self, p: &mut Pump, l: &Loc) -> Res<Vec<Entry>> {
        let (fs, path) = self.fs(p, l)?;
        let mut v = fs.list(p, &path)?;
        v.retain(|e| crate::model::safe_name(&e.name));
        Ok(v)
    }
}

struct Worker {
    p: Pump,
    b: Backends,
}

impl Worker {
    fn run(&mut self, r: Request) {
        let Some(id) = r.id() else {
            if let Request::Server(s) = r {
                // A changed server starts over.
                self.b.sftp.remove(&s.name);
                self.b.curl.remove(&s.name);
                self.b.servers.insert(s.name.clone(), s);
            }
            return;
        };
        let server = match &r {
            Request::List { at, .. } => server_of(at),
            Request::Contents { archive, .. } => server_of(archive),
            Request::Copy { items, to, .. } => {
                server_of(to).or_else(|| items.first().and_then(server_of))
            }
            Request::Delete { items, .. } => items.first().and_then(server_of),
            Request::Rename { item, .. } => server_of(item),
            Request::Mkdir { at, .. } => server_of(at),
            Request::Extract { archive, to, .. } => server_of(archive).or_else(|| server_of(to)),
            _ => None,
        };
        self.p.current = Some((id, server.unwrap_or_default(), 0));
        let r = self.handle(id, r);
        self.p.current = None;
        if self.p.closed {
            return;
        }
        let e = match r {
            Ok(Some(e)) => e,
            Ok(None) => Event::Done { id },
            Err(text) => Event::Error { id, text },
        };
        self.p.cancelled.remove(&id);
        self.p.send(&e);
    }

    fn handle(&mut self, id: Id, r: Request) -> Res<Option<Event>> {
        let (p, b) = (&mut self.p, &mut self.b);
        if b.root.is_some() {
            root_allows(&r)?;
        }
        match r {
            Request::List { at, .. } => {
                let at = match &at {
                    Loc::Remote { server, path } if path == "~" => {
                        let (fs, _) = b.fs(p, &at)?;
                        let home = fs.home(p)?;
                        p.send(&Event::Resolved {
                            id,
                            path: home.clone(),
                        });
                        Loc::Remote {
                            server: server.clone(),
                            path: home,
                        }
                    }
                    _ => at,
                };
                let mut entries = b.list(p, &at)?;
                // The screen shows so many; a copy reads the rest itself.
                entries.truncate(crate::model::MAX_ENTRIES);
                for chunk in entries.chunks(256) {
                    p.send(&Event::Entries {
                        id,
                        entries: chunk.to_vec(),
                    });
                }
                Ok(Some(Event::Listed { id }))
            }
            Request::Contents { archive, .. } => {
                let entries = archive::contents(b, p, id, &archive)?;
                for chunk in entries.chunks(256) {
                    p.send(&Event::Entries {
                        id,
                        entries: chunk.to_vec(),
                    });
                }
                Ok(Some(Event::Listed { id }))
            }
            Request::Copy {
                items, to, remove, ..
            } => copy::copy(b, p, id, &items, &to, remove, true).map(|_| None),
            Request::Delete { items, trash, .. } => {
                for item in &items {
                    if p.cancelled(id) {
                        return Err(STOPPED.into());
                    }
                    match item {
                        Loc::Local(path) if trash => local::trash(&b.home, path)?,
                        _ => copy::delete_tree(b, p, item, 0)?,
                    }
                }
                Ok(None)
            }
            Request::Rename { item, name, .. } => {
                let to = item.parent_folder().ok_or("Can't rename this")?.join(&name);
                if b.stat(p, &to)?.is_some() {
                    return Err(format!("\u{201c}{name}\u{201d} is taken"));
                }
                let (fs, from) = b.fs(p, &item)?;
                let to = loc_path(&to);
                fs.rename(p, &from, &to).map(|_| None)
            }
            Request::Mkdir { at, name, .. } => {
                let target = at.join(&name);
                if b.stat(p, &target)?.is_some() {
                    return Err(format!("\u{201c}{name}\u{201d} is taken"));
                }
                let (fs, path) = b.fs(p, &target)?;
                fs.mkdir(p, &path).map(|_| None)
            }
            Request::Extract {
                archive,
                entries,
                to,
                ..
            } => archive::extract(b, p, id, &archive, &entries, &to).map(|_| None),
            Request::Compress { items, to, .. } => {
                archive::compress(p, id, &items, &to).map(|_| None)
            }
            Request::MakeKey { passphrase, .. } => {
                local::make_key(p, &b.data.join("ssh"), &passphrase).map(|_| None)
            }
            Request::Read { file, .. } => {
                let Loc::Local(path) = &file else {
                    return Err("Copy it to the card to read it".into());
                };
                let bytes = match &b.root {
                    Some(r) => r.read(path, crate::model::VIEW_MAX)?,
                    None => return Err("Only root mode reads for the page".into()),
                };
                let text = crate::model::view_text(bytes);
                Ok(Some(Event::Text { id, text }))
            }
            Request::Server(_) | Request::Cancel { .. } | Request::Reply { .. } => Ok(None),
        }
    }
}

/// What a root worker does: files on the card, and nothing that runs
/// another program (ssh, curl, bsdtar) or reaches a server as root. Its
/// deletes are for good: a root file in the user's Trash would be the
/// user's to read.
fn root_allows(r: &Request) -> Res<()> {
    let local = |l: &Loc| matches!(l, Loc::Local(_));
    let ok = match r {
        Request::List { at, .. } => local(at),
        Request::Copy { items, to, .. } => local(to) && items.iter().all(local),
        Request::Delete { items, trash, .. } => !trash && items.iter().all(local),
        Request::Rename { item, .. } => local(item),
        Request::Mkdir { at, .. } => local(at),
        Request::Read { file, .. } => local(file),
        Request::Cancel { .. } | Request::Reply { .. } => true,
        Request::Server(_)
        | Request::Contents { .. }
        | Request::Extract { .. }
        | Request::Compress { .. }
        | Request::MakeKey { .. } => false,
    };
    if ok {
        Ok(())
    } else {
        Err("Not as root: copy it to the card first".into())
    }
}

fn server_of(l: &Loc) -> Option<String> {
    match l {
        Loc::Remote { server, .. } => Some(server.clone()),
        _ => None,
    }
}

/// A place's path as the place takes it.
pub fn loc_path(l: &Loc) -> String {
    match l {
        Loc::Local(p) => p.to_string_lossy().into_owned(),
        Loc::Remote { path, .. } => path.clone(),
        _ => String::new(),
    }
}

/// The worker's main: reads requests from the page until it goes.
pub fn main() -> i32 {
    #[cfg(target_os = "linux")]
    // SAFETY: prctl on this process.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    let Some(fd) = std::env::var(FD_ENV)
        .ok()
        .and_then(|v| v.parse::<RawFd>().ok())
        // SAFETY: fcntl says whether the fd is open before we own it.
        .filter(|&n| n > 2 && unsafe { libc::fcntl(n, libc::F_GETFD) } >= 0)
    else {
        eprintln!("gh0st-files --worker: run by the app, with {FD_ENV}");
        return 2;
    };
    // SAFETY: the page gave us this fd and nothing else owns it.
    let link = Conn::new(unsafe { OwnedFd::from_raw_fd(fd) });
    // SAFETY: fcntl on an fd we own.
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into());
    let data = std::env::var_os("GH0ST_FILES_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share/gh0st-files"));
    // Root mode only when gh0st-helper started it, as root.
    let root = match std::env::var(root::ENV) {
        Ok(v) => {
            // SAFETY: geteuid has no arguments.
            let ident = root::Ident::parse(&v).filter(|_| unsafe { libc::geteuid() } == 0);
            let Some(ident) = ident else {
                eprintln!("gh0st-files --worker: {} wants root and a user", root::ENV);
                return 2;
            };
            Some(root::RootLocal::new(ident))
        }
        Err(_) => None,
    };
    // What a killed worker left half done.
    let _ = std::fs::remove_dir_all(data.join("stage"));
    let mut w = Worker {
        p: Pump::new(link),
        b: Backends::new(home, data.clone()),
    };
    w.b.root = root;
    w.p.key = Some(data.join("ssh/id_ed25519"));
    // The page queues its requests until this: for a root worker it says
    // the password was right.
    w.p.send(&Event::Ready);
    loop {
        while let Some(r) = w.p.queue.pop_front() {
            w.run(r);
        }
        if w.p.closed {
            return 0;
        }
        w.p.wait(&[], -1);
    }
}
