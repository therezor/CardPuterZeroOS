//! The session daemons the shell runs: `gh0st keyring` and `gh0st session`.
//! Each gets a socket pair the shell made, and is restarted at once when it
//! dies, so no app gets the time to take its bus names.

use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use gh0st_core::keyring::{self as kr, Recv, ToDaemon, ToShell};
use gh0st_core::link::Link;

/// Starts in this window that mean a daemon can't stay up.
const RESTART_WINDOW_MS: u64 = 60_000;
const RESTARTS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    Keyring,
    Session,
}

impl Which {
    fn program(self) -> &'static str {
        match self {
            Which::Keyring => "keyring",
            Which::Session => "session",
        }
    }

    fn env(self) -> &'static str {
        match self {
            Which::Keyring => kr::ENV,
            Which::Session => "GH0ST_SESSION_FD",
        }
    }
}

enum End {
    Keyring(OwnedFd),
    Session(Link),
}

struct Daemon {
    which: Which,
    pid: i32,
    end: End,
    ready: bool,
}

pub struct Services {
    sim: bool,
    exe: PathBuf,
    running: Vec<Daemon>,
    starts: Vec<(Which, u64)>,
    given_up: Vec<Which>,
}

/// What a daemon said.
pub enum Said {
    Keyring(ToShell),
    Session(Vec<String>),
}

impl Services {
    /// Starts both, from the binary the shell runs from.
    /// Starts both. `sim`: a test may name another binary and a scratch
    /// vault; on a device nothing outside the shell picks what gets the
    /// login password.
    pub fn start(now: u64, sim: bool) -> Services {
        let exe = sim
            .then(|| std::env::var_os("GH0ST_SERVICES_EXE").map(PathBuf::from))
            .flatten()
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| PathBuf::from("/usr/bin/gh0st"));
        let mut s = Services {
            sim,
            exe,
            running: Vec::new(),
            starts: Vec::new(),
            given_up: Vec::new(),
        };
        for w in [Which::Keyring, Which::Session] {
            s.spawn(w, now);
        }
        s
    }

    /// No daemons: `--sim` on a host, where there is no bus.
    pub fn none() -> Services {
        Services {
            sim: true,
            exe: PathBuf::new(),
            running: Vec::new(),
            starts: Vec::new(),
            given_up: Vec::new(),
        }
    }

    fn spawn(&mut self, which: Which, now: u64) -> bool {
        self.starts
            .retain(|(_, t)| now.saturating_sub(*t) < RESTART_WINDOW_MS);
        if self.starts.iter().filter(|(w, _)| *w == which).count() >= RESTARTS {
            eprintln!(
                "gh0st shell: {} keeps dying; not starting it again",
                which.program()
            );
            self.given_up.push(which);
            return false;
        }
        self.starts.push((which, now));
        let (theirs, end) = match which {
            Which::Keyring => match kr::pair() {
                Ok((a, b)) => (b, End::Keyring(a)),
                Err(_) => return false,
            },
            Which::Session => match Link::pair() {
                Ok((l, b)) => (b, End::Session(l)),
                Err(_) => return false,
            },
        };
        let child_fd = theirs.as_raw_fd();
        let mut cmd = Command::new(&self.exe);
        cmd.arg(which.program())
            .stdin(Stdio::null())
            .env(which.env(), child_fd.to_string());
        if which == Which::Keyring && self.sim {
            if let Some(v) = std::env::var_os("GH0ST_KEYRING_VAULT") {
                cmd.arg("--vault").arg(v);
            }
        }
        // SAFETY: fcntl is async-signal-safe: the child alone keeps its end
        // open across exec.
        unsafe {
            cmd.pre_exec(move || {
                if libc::fcntl(child_fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        match cmd.spawn() {
            Ok(c) => {
                drop(theirs);
                self.running.push(Daemon {
                    which,
                    pid: c.id() as i32,
                    end,
                    ready: false,
                });
                true
            }
            Err(e) => {
                eprintln!("gh0st shell: can't start {}: {e}", which.program());
                false
            }
        }
    }

    /// A child exited. Some if it was one of ours: it starts again, and
    /// the flag says it has died too often and won't.
    pub fn exited(&mut self, pid: i32, now: u64) -> Option<(Which, bool)> {
        let i = self.running.iter().position(|d| d.pid == pid)?;
        let which = self.running.swap_remove(i).which;
        let started = !self.given_up.contains(&which) && self.spawn(which, now);
        Some((which, !started && self.given_up.contains(&which)))
    }

    pub fn fds(&self) -> Vec<RawFd> {
        self.running
            .iter()
            .map(|d| match &d.end {
                End::Keyring(fd) => fd.as_raw_fd(),
                End::Session(l) => l.fd(),
            })
            .collect()
    }

    pub fn ready(&self, which: Which) -> bool {
        self.running.iter().any(|d| d.which == which && d.ready)
    }

    pub fn running(&self, which: Which) -> bool {
        self.running.iter().any(|d| d.which == which)
    }

    /// Tells the keyring. False when it isn't running.
    /// A line for `gh0st session`: the portal's answers.
    pub fn say_session(&self, fields: &[&str]) -> bool {
        self.running.iter().any(|d| match &d.end {
            End::Session(l) => l.send(fields),
            End::Keyring(_) => false,
        })
    }

    pub fn tell(&self, m: &ToDaemon) -> bool {
        let Some(fd) = self.running.iter().find_map(|d| match &d.end {
            End::Keyring(fd) => Some(fd.as_raw_fd()),
            End::Session(_) => None,
        }) else {
            return false;
        };
        let mut bytes = m.encode();
        let ok = kr::send(fd, &bytes);
        gh0st_core::password::wipe(&mut bytes);
        ok
    }

    /// Everything the daemons said since the last read.
    pub fn read(&mut self) -> Vec<Said> {
        let mut out = Vec::new();
        for d in &mut self.running {
            match &d.end {
                End::Keyring(fd) => {
                    while let Recv::Packet(p) = kr::recv(fd.as_raw_fd()) {
                        if let Some(m) = ToShell::decode(&p) {
                            if m == ToShell::Ready {
                                d.ready = true;
                            }
                            out.push(Said::Keyring(m));
                        }
                    }
                }
                End::Session(l) => {
                    while let Some(m) = l.recv() {
                        if m.first().is_some_and(|v| v == "ready") {
                            d.ready = true;
                        }
                        out.push(Said::Session(m));
                    }
                }
            }
        }
        out
    }

    /// Stops both, as the shell ends.
    pub fn stop(&mut self) {
        for d in self.running.drain(..) {
            // SAFETY: kill with a pid we started.
            unsafe { libc::kill(d.pid, libc::SIGTERM) };
        }
    }
}
