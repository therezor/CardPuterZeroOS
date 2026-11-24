//! A helper process beside the app: a program it starts, which talks over a
//! socket pair and draws into a [`Ring`]. The camera's `gh0st-camd` and the
//! media app's `gh0st-mediad` are C++ programs linked against glibc, while
//! apps are static; this is how they meet.
//!
//! The helper runs in a process group of its own. The shell stops the app's
//! group right after the app answers `paused`, and the helper must still
//! act on what the app sent just before. It should exit when its link hangs
//! up and when the app dies (PDEATHSIG).
//!
//! Messages are tab-separated fields, one per packet, as on the shell link
//! ([`Link`]). Register [`Sidecar::fd`] with [`crate::Cx::watch`] and the
//! pid with [`crate::Cx::adopt`]; drain [`Sidecar::recv`] in
//! [`crate::Page::ready`].

use crate::frames::Ring;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// The link a helper talks over: one message per packet, fields split by
/// tabs.
pub use gh0st_core::link::{Link, path_field, path_of};

/// Where a helper finds its end of the link and the ring: the names of two
/// variables holding fd numbers.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub link: &'static str,
    pub ring: &'static str,
}

pub struct Sidecar {
    link: Link,
    pub ring: Ring,
    /// The child to adopt, when it is a process.
    pub pid: Option<i32>,
}

impl Sidecar {
    /// Starts `argv` with its ends of a fresh link and of `ring`.
    pub fn spawn(argv: &[String], env: Env, ring: Ring) -> io::Result<Sidecar> {
        let (link, theirs) = Link::pair()?;
        let ring_fd = ring.share()?;
        let pid = spawn(argv, env, &theirs, &ring_fd)?;
        Ok(Sidecar {
            link,
            ring,
            pid: Some(pid),
        })
    }

    /// Runs `serve` on a thread in place of a program: a fake helper for
    /// tests and mockups, with the same link and ring a process gets.
    pub fn thread(
        ring: Ring,
        serve: impl FnOnce(Link, Ring) + Send + 'static,
    ) -> io::Result<Sidecar> {
        let (link, theirs) = Link::pair()?;
        let theirs = Link::from_fd(theirs)?;
        let their_ring = Ring::open(ring.share()?)?;
        std::thread::spawn(move || serve(theirs, their_ring));
        Ok(Sidecar {
            link,
            ring,
            pid: None,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.link.fd()
    }

    /// False when it didn't go: the helper is gone or not reading.
    pub fn send(&self, fields: &[&str]) -> bool {
        self.link.send(fields)
    }

    /// The next message, or None when nothing waits.
    pub fn recv(&self) -> Option<Vec<String>> {
        self.link.recv()
    }

    /// True once the helper hung up.
    pub fn closed(&self) -> bool {
        self.link.closed()
    }
}

fn spawn(argv: &[String], env: Env, link: &OwnedFd, ring: &OwnedFd) -> io::Result<i32> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("no helper"))?;
    let (l, r) = (link.as_raw_fd(), ring.as_raw_fd());
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .env(env.link, l.to_string())
        .env(env.ring, r.to_string());
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

/// The helper's side, for one written in Rust: its link and ring from the
/// env the app set.
pub fn inherited(env: Env) -> io::Result<(Link, Ring)> {
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
    let link = Link::from_fd(fd(env.link)?)?;
    let ring = Ring::open(fd(env.ring)?)?;
    Ok((link, ring))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::SCREEN_PX;

    #[test]
    fn a_thread_helper_answers_and_draws() {
        let ring = Ring::create(c"gh0st-test", 2, SCREEN_PX).unwrap();
        let side = Sidecar::thread(ring, |link, ring| {
            while !link.closed() {
                if let Some(m) = link.recv() {
                    ring.write(1, 2, 1, 1, |px| px[0] = 0xf800);
                    link.send(&["drawn", &m[0]]);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
        .unwrap();
        assert!(side.send(&["draw"]));
        let mut got = None;
        for _ in 0..1000 {
            got = side.recv();
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(got.unwrap(), ["drawn", "draw"]);
        let mut px = Vec::new();
        assert_eq!(side.ring.read(1, 2, &mut px), Some((1, 1)));
        assert_eq!(px, [0xf800]);
    }
}
