//! Starting the camera helper and talking to it. The helper owns libcamera
//! and the encoders; it is a C++ program (`camd/`) linked against glibc,
//! while this app is static.
//!
//! It runs in a process group of its own. The shell stops the app's group
//! right after the app answers `paused`, and the helper must still act on
//! the `stop` the app sent: close a recording, let go of the sensor. It
//! exits on `quit`, on a hang-up, and when the app dies (PDEATHSIG).

use crate::proto::{Cmd, Event};
use crate::ring::Ring;
use gh0st_core::link::Link;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where the helper finds its end of the link and the ring: fd numbers.
pub const FD_ENV: &str = "GH0ST_CAMD_FD";
pub const RING_ENV: &str = "GH0ST_CAMD_RING";
/// Overrides the helper's path. `fake` runs this binary's test helper.
pub const PATH_ENV: &str = "GH0ST_CAMD";
pub const DEFAULT_PATH: &str = "/usr/libexec/gh0st-camd";

/// Which helper runs.
#[derive(Clone, Debug)]
pub enum Source {
    /// A program: the real helper, or this binary with `--fake-helper`.
    Exec(Vec<String>),
    /// The fake helper on a thread, for tests and mockups that have no
    /// binary to run. Without a camera it answers as a Lite does.
    Fake { camera: bool },
}

impl Source {
    /// The real helper, or what `GH0ST_CAMD` names.
    pub fn from_env() -> Source {
        match std::env::var(PATH_ENV).ok().as_deref() {
            Some("fake") => {
                let me = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Source::Exec(vec![me, "--fake-helper".into()])
            }
            Some(p) if !p.is_empty() => Source::Exec(vec![p.into()]),
            _ => Source::Exec(vec![DEFAULT_PATH.into()]),
        }
    }
}

pub struct Helper {
    link: Link,
    pub ring: Ring,
    /// The child to adopt, when it is a process.
    pub pid: Option<i32>,
}

impl Helper {
    pub fn start(source: &Source) -> io::Result<Helper> {
        let (link, theirs) = Link::pair()?;
        let ring = Ring::create()?;
        let ring_fd = ring.share()?;
        let pid = match source {
            Source::Exec(argv) => Some(spawn(argv, &theirs, &ring_fd)?),
            &Source::Fake { camera } => {
                let link = Link::from_fd(theirs)?;
                let ring = Ring::open(ring_fd)?;
                std::thread::spawn(move || crate::fake::serve(link, ring, camera));
                None
            }
        };
        Ok(Helper { link, ring, pid })
    }

    pub fn fd(&self) -> RawFd {
        self.link.fd()
    }

    /// False when it didn't go: the helper is gone or not reading.
    pub fn send(&self, cmd: &Cmd) -> bool {
        let f = cmd.fields();
        let refs: Vec<&str> = f.iter().map(String::as_str).collect();
        self.link.send(&refs)
    }

    /// The next event, or None when nothing waits. Unknown words are
    /// skipped.
    pub fn recv(&self) -> Option<Event> {
        loop {
            let f = self.link.recv()?;
            if let Some(e) = Event::parse(&f) {
                return Some(e);
            }
        }
    }

    /// True once the helper hung up.
    pub fn closed(&self) -> bool {
        self.link.closed()
    }
}

/// A helper handle dropped without `quit` (a test, a panic) still tells the
/// helper to go: the hang-up alone is lost on datagram links.
impl Drop for Helper {
    fn drop(&mut self) {
        self.send(&Cmd::Quit);
    }
}

fn spawn(argv: &[String], link: &OwnedFd, ring: &OwnedFd) -> io::Result<i32> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("no helper"))?;
    let (l, r) = (link.as_raw_fd(), ring.as_raw_fd());
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .env(FD_ENV, l.to_string())
        .env(RING_ENV, r.to_string());
    // SAFETY: fcntl is async-signal-safe. Both fds stay open across exec
    // in the child alone.
    unsafe {
        cmd.pre_exec(move || {
            for fd in [l, r] {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(cmd.spawn()?.id() as i32)
}

/// The helper's side: its link and ring from the env this app set.
pub fn inherited() -> io::Result<(Link, Ring)> {
    let fd = |name: &str| -> io::Result<OwnedFd> {
        let n: RawFd = std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| io::Error::other(format!("{name} not set")))?;
        // SAFETY: fcntl says whether the fd is open before we own it.
        if n < 3 || unsafe { libc::fcntl(n, libc::F_GETFD) } < 0 {
            return Err(io::Error::other(format!("{name} is not open")));
        }
        // SAFETY: the app gave us this fd and nothing else owns it.
        Ok(unsafe { OwnedFd::from_raw_fd(n) })
    };
    let link = Link::from_fd(fd(FD_ENV)?)?;
    let ring = Ring::open(fd(RING_ENV)?)?;
    Ok((link, ring))
}

/// Where photos and videos go: stock Camera's folder.
pub fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join("Pictures/DCIM/Camera")
}
