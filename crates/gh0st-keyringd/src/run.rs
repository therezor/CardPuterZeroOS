//! `gh0st keyring`: one poll loop over the shell link, the session bus and
//! Settings' private socket. No threads, no timers.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Instant;

use gh0st_core::keyring::{self as link, Answer, Recv, ToDaemon, ToShell};
use gh0st_dbus::{Conn, Kind, Message, Value, bus};

use crate::ident;
use crate::secret::{self, Ask, Caller, Out, Service, Unlock};
use crate::vault::Cost;

const USAGE: &str = "usage: gh0st keyring [--vault PATH] [--test]";
/// Settings clients at once; more wait in the backlog.
const MAX_CLIENTS: usize = 4;
/// A wrong old password costs this long before the next try.
const PREPARE_BACKOFF_MS: u128 = 2000;

struct Opts {
    vault: PathBuf,
    /// No shell: unlock and answers come from env, for CI.
    test: bool,
}

fn default_vault() -> PathBuf {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/var/lib/gh0st/keyring/{uid}/login.vault"))
}

fn parse(args: &[String]) -> Option<Opts> {
    let mut o = Opts {
        vault: default_vault(),
        test: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vault" => o.vault = PathBuf::from(it.next()?),
            "--test" => o.test = true,
            _ => return None,
        }
    }
    Some(o)
}

/// The private socket Settings uses around passwd.
pub fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(dir).join("gh0st").join("keyring.sock"))
}

struct Daemon {
    svc: Service,
    bus: Conn,
    link: Option<OwnedFd>,
    callers: HashMap<String, Caller>,
    test_pw: Option<Vec<u8>>,
    test_answer: Answer,
    clients: Vec<UnixStream>,
    last_bad_prepare: Option<Instant>,
}

pub fn main(args: &[String]) -> i32 {
    let Some(opts) = parse(args) else {
        eprintln!("{USAGE}");
        return 2;
    };
    crate::secure::harden();
    // SAFETY: ignoring SIGPIPE has no preconditions.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let link = if opts.test { None } else { take_link() };
    if !opts.test && link.is_none() {
        eprintln!(
            "gh0st keyring: no shell link ({}); run by the shell",
            link::ENV
        );
        return 2;
    }
    if let Some(dir) = opts.vault.parent() {
        if !dir.is_dir() {
            eprintln!("gh0st keyring: {} is missing", dir.display());
            return 1;
        }
    }
    let mut bus = match Conn::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gh0st keyring: no session bus: {e}");
            return 1;
        }
    };
    match bus::own(&mut bus, secret::NAME) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("gh0st keyring: {} is taken", secret::NAME);
            return 1;
        }
        Err(e) => {
            eprintln!("gh0st keyring: can't own {}: {e}", secret::NAME);
            return 1;
        }
    }
    let rule = "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged'";
    if let Err(e) = bus::add_match(&mut bus, rule) {
        eprintln!("gh0st keyring: {e}");
        return 1;
    }
    let listener = listen();
    let cost = if opts.test { Cost::TEST } else { Cost::DEFAULT };
    let mut d = Daemon {
        svc: Service::new(opts.vault, cost),
        bus,
        link,
        callers: HashMap::new(),
        test_pw: std::env::var("GH0ST_KEYRING_TEST_PASSWORD")
            .ok()
            .map(String::into_bytes),
        test_answer: match std::env::var("GH0ST_KEYRING_TEST_ANSWER").as_deref() {
            Ok("once") => Answer::Once,
            Ok("deny") => Answer::Deny,
            _ => Answer::Always,
        },
        clients: Vec::new(),
        last_bad_prepare: None,
    };
    if let Some(pw) = d.test_pw.clone() {
        d.svc.unlock(&pw);
    }
    d.to_shell(&ToShell::Ready);
    d.pump();
    d.run(listener.as_ref())
}

fn take_link() -> Option<OwnedFd> {
    let n: RawFd = std::env::var(link::ENV).ok()?.parse().ok()?;
    // SAFETY: we have one thread and read nothing else from the env.
    unsafe { std::env::remove_var(link::ENV) };
    // SAFETY: fcntl only checks that the fd is open.
    if n < 3 || unsafe { libc::fcntl(n, libc::F_GETFD) } < 0 {
        return None;
    }
    gh0st_core::rt::set_nonblock_cloexec(n).ok()?;
    // SAFETY: the shell handed us this fd and nothing else owns it.
    Some(unsafe { OwnedFd::from_raw_fd(n) })
}

fn listen() -> Option<UnixListener> {
    let path = socket_path()?;
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).ok()?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok()?;
    l.set_nonblocking(true).ok()?;
    Some(l)
}

impl Daemon {
    fn run(&mut self, listener: Option<&UnixListener>) -> i32 {
        loop {
            let mut fds = vec![gh0st_core::rt::pollfd_rw(
                self.bus.fd(),
                true,
                self.bus.wants_write(),
            )];
            if let Some(l) = &self.link {
                fds.push(gh0st_core::rt::pollfd(l.as_raw_fd()));
            }
            let lfd = listener.map(|l| l.as_raw_fd());
            if let Some(l) = lfd {
                fds.push(gh0st_core::rt::pollfd(l));
            }
            for c in &self.clients {
                fds.push(gh0st_core::rt::pollfd(c.as_raw_fd()));
            }
            if gh0st_core::rt::wait(&mut fds, None).is_err() {
                continue;
            }
            let ready = |fd: RawFd| fds.iter().any(|p| p.fd == fd && p.revents != 0);
            if ready(self.bus.fd()) {
                if let Err(e) = self.bus.flush().and_then(|()| self.bus.fill()) {
                    eprintln!("gh0st keyring: bus: {e}");
                    return 1;
                }
                if self.bus.closed() {
                    eprintln!("gh0st keyring: the bus went away");
                    return 1;
                }
            }
            if let Err(e) = self.bus_messages() {
                eprintln!("gh0st keyring: bus: {e}");
                return 1;
            }
            if let Some(fd) = self.link.as_ref().map(AsRawFd::as_raw_fd) {
                if ready(fd) && !self.link_messages(fd) {
                    // The shell is gone: so is the vault's key.
                    return 0;
                }
            }
            if let (Some(l), Some(fd)) = (listener, lfd) {
                if ready(fd) {
                    self.accept(l);
                }
            }
            let ready_clients: Vec<usize> = (0..self.clients.len())
                .filter(|&i| ready(self.clients[i].as_raw_fd()))
                .collect();
            for i in ready_clients.into_iter().rev() {
                self.client(i);
            }
            self.pump();
        }
    }

    fn to_shell(&self, m: &ToShell) {
        if let Some(l) = &self.link {
            link::send(l.as_raw_fd(), &m.encode());
        }
    }

    /// Sends what the service queued; in test mode, answers its asks.
    fn pump(&mut self) {
        for _ in 0..16 {
            let outs = self.svc.drain();
            if outs.is_empty() {
                break;
            }
            for o in outs {
                match o {
                    Out::Signal(s) => {
                        if let Err(e) = self.bus.send(s) {
                            eprintln!("gh0st keyring: signal: {e}");
                        }
                    }
                    Out::Ask(a) => self.ask(a),
                }
            }
        }
    }

    fn ask(&mut self, a: Ask) {
        if self.link.is_none() {
            match a {
                Ask::Password {
                    id, retry: false, ..
                } => match self.test_pw.clone() {
                    Some(pw) => self.svc.password(id, &pw),
                    None => self.svc.answer(id, secret::Answer::Deny),
                },
                Ask::Password { id, .. } => self.svc.answer(id, secret::Answer::Deny),
                Ask::Approve { id, .. } => self.svc.answer(id, to_secret(self.test_answer)),
                Ask::Cancel { .. } => {}
            }
            return;
        }
        let m = match a {
            Ask::Password { id, reason, retry } => ToShell::AskPassword { id, reason, retry },
            Ask::Approve { id, app, what } => ToShell::AskApprove { id, app, what },
            Ask::Cancel { id } => ToShell::Cancel { id },
        };
        self.to_shell(&m);
    }

    fn bus_messages(&mut self) -> gh0st_dbus::Result<()> {
        while let Some(m) = self.bus.recv()? {
            match m.kind {
                Kind::Call => self.bus_call(m)?,
                Kind::Signal if m.is(bus::NAME, "NameOwnerChanged") => {
                    let name = m.body.first().and_then(Value::as_str).unwrap_or("");
                    let new = m.body.get(2).and_then(Value::as_str).unwrap_or("x");
                    if new.is_empty() && name.starts_with(':') {
                        self.callers.remove(name);
                        self.svc.gone(name);
                    }
                }
                _ => {}
            }
            self.pump();
        }
        Ok(())
    }

    fn bus_call(&mut self, mut m: Message) -> gh0st_dbus::Result<()> {
        let Some(sender) = m.sender.clone() else {
            return Ok(());
        };
        let c = match self.callers.get(&sender) {
            Some(c) => c.clone(),
            None => {
                let creds = bus::credentials(&mut self.bus, &sender).unwrap_or_default();
                let c = ident::caller(&sender, creds.pid, creds.pidfd.as_ref());
                self.callers.insert(sender.clone(), c.clone());
                c
            }
        };
        let reply = self.svc.call(&m, &c);
        m.wipe();
        if let Some(r) = reply {
            self.bus.send(r)?;
        }
        Ok(())
    }

    /// False once the shell hangs up.
    fn link_messages(&mut self, fd: RawFd) -> bool {
        loop {
            let mut p = match link::recv(fd) {
                Recv::Packet(p) => p,
                Recv::Empty => return true,
                Recv::Closed => return false,
            };
            let msg = ToDaemon::decode(&p);
            gh0st_core::password::wipe(&mut p);
            let Some(mut msg) = msg else {
                continue;
            };
            self.shell_said(&msg);
            msg.wipe();
            self.pump();
        }
    }

    fn shell_said(&mut self, m: &ToDaemon) {
        match m {
            ToDaemon::Unlock { pw, right } => {
                let r = self.svc.unlock(pw);
                let reply = match r {
                    Unlock::Opened | Unlock::Created => ToShell::Unlocked,
                    Unlock::Wrong if *right => {
                        self.svc.hold_stale(pw);
                        ToShell::Stale
                    }
                    Unlock::Wrong => ToShell::Wrong,
                    Unlock::Broken => ToShell::Broken,
                };
                self.to_shell(&reply);
            }
            ToDaemon::Lock => self.svc.lock(),
            ToDaemon::Old { pw } => {
                let reply = match self.svc.rewrap_from_old(pw) {
                    Unlock::Opened | Unlock::Created => ToShell::Unlocked,
                    Unlock::Wrong => ToShell::Wrong,
                    Unlock::Broken => ToShell::Broken,
                };
                self.to_shell(&reply);
            }
            ToDaemon::Password { id, pw } => self.svc.password(*id, pw),
            ToDaemon::Answer { id, answer } => self.svc.answer(*id, to_secret(*answer)),
        }
    }

    fn accept(&mut self, l: &UnixListener) {
        while let Ok((s, _)) = l.accept() {
            // SAFETY: getuid cannot fail.
            let me = unsafe { libc::getuid() };
            let ok = gh0st_dbus::conn::peer_uid(s.as_raw_fd()).ok() == Some(me);
            if ok && self.clients.len() < MAX_CLIENTS && s.set_nonblocking(true).is_ok() {
                self.clients.push(s);
            }
        }
    }

    /// One request per connection: `prepare\0old\0new` or `commit\0{1,0}`.
    fn client(&mut self, i: usize) {
        let mut buf = [0u8; 1024];
        let n = match self.clients[i].read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(_) => 0,
        };
        let mut s = self.clients.swap_remove(i);
        if n == 0 {
            return;
        }
        let f: Vec<&[u8]> = buf[..n].split(|&b| b == 0).collect();
        let ok = match f.as_slice() {
            [b"prepare", old, new] => {
                let waited = self
                    .last_bad_prepare
                    .is_none_or(|t| t.elapsed().as_millis() >= PREPARE_BACKOFF_MS);
                let ok = waited && self.svc.prepare(old, new);
                if !ok {
                    self.last_bad_prepare = Some(Instant::now());
                }
                ok
            }
            [b"commit", yes] => self.svc.commit(*yes == b"1"),
            _ => false,
        };
        gh0st_core::password::wipe(&mut buf);
        use std::io::Write;
        let _ = s.set_nonblocking(false);
        let _ = s.write_all(if ok { b"ok" } else { b"no" });
    }
}

fn to_secret(a: Answer) -> secret::Answer {
    match a {
        Answer::Once => secret::Answer::Once,
        Answer::Always => secret::Answer::Always,
        Answer::Deny => secret::Answer::Deny,
    }
}

/// Settings' side: asks the daemon to prepare or commit a password change.
/// False when the daemon isn't running or says no.
pub fn request(path: &Path, fields: &[&[u8]]) -> bool {
    use std::io::Write;
    let Ok(mut s) = UnixStream::connect(path) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut msg = fields.join(&0u8);
    let sent = s.write_all(&msg).is_ok();
    gh0st_core::password::wipe(&mut msg);
    let mut reply = [0u8; 2];
    sent && s.read_exact(&mut reply).is_ok() && &reply == b"ok"
}
