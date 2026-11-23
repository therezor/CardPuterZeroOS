//! The shell link: one socket pair per app, one message per packet, UTF-8
//! fields split by tabs (`docs/apps.md`, Shell link). Built so far: `hello`
//! both ways, `pause` and `paused`, `resume`, `redraw`, `toast`.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// Where an app finds its end: the fd number, in its env.
pub const ENV: &str = "GH0ST_SHELL_FD";
/// The longest message either side sends.
pub const MAX: usize = 512;

pub struct Link {
    fd: OwnedFd,
    /// The other end hung up. Its fd then polls readable for good, so the
    /// owner drops the link rather than spin.
    closed: std::cell::Cell<bool>,
}

fn nonblock_cloexec(fd: RawFd) -> io::Result<()> {
    crate::rt::set_nonblock_cloexec(fd)
}

impl Link {
    /// A new pair: the shell's end, and the app's end for the child to
    /// inherit. Both close on exec until the spawn clears the app's.
    pub fn pair() -> io::Result<(Link, OwnedFd)> {
        let mut fds = [0; 2];
        // SEQPACKET where the system has it (Linux), datagrams elsewhere:
        // both keep message bounds.
        for ty in [libc::SOCK_SEQPACKET, libc::SOCK_DGRAM] {
            // SAFETY: socketpair fills a two-int array.
            if unsafe { libc::socketpair(libc::AF_UNIX, ty, 0, fds.as_mut_ptr()) } == 0 {
                // SAFETY: both fds are fresh and owned from here.
                let (a, b) =
                    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
                nonblock_cloexec(a.as_raw_fd())?;
                // SAFETY: fcntl on an fd we own.
                unsafe { libc::fcntl(b.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
                return Ok((Link::new(a), b));
            }
        }
        Err(io::Error::last_os_error())
    }

    /// The app's end, from the env the shell set. Takes the variable out,
    /// so the app's own children don't see it.
    pub fn from_env() -> Option<Link> {
        let n: RawFd = std::env::var(ENV).ok()?.parse().ok()?;
        // SAFETY: the app has one thread, and nothing has read the env yet.
        unsafe { std::env::remove_var(ENV) };
        // SAFETY: fcntl tells whether the fd is open before we own it.
        if n < 3 || unsafe { libc::fcntl(n, libc::F_GETFD) } < 0 {
            return None;
        }
        nonblock_cloexec(n).ok()?;
        // SAFETY: the shell gave us this fd and nothing else owns it.
        Some(Link::new(unsafe { OwnedFd::from_raw_fd(n) }))
    }

    /// A link over an end from [`pair`](Link::pair) that an app keeps for a
    /// helper of its own, or a helper inherited.
    pub fn from_fd(fd: OwnedFd) -> io::Result<Link> {
        nonblock_cloexec(fd.as_raw_fd())?;
        Ok(Link::new(fd))
    }

    fn new(fd: OwnedFd) -> Link {
        Link {
            fd,
            closed: std::cell::Cell::new(false),
        }
    }

    /// True once `recv` has seen the other end hang up.
    pub fn closed(&self) -> bool {
        self.closed.get()
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Sends one message, never blocking and never raising SIGPIPE. False
    /// if it didn't go: the other side is gone or not reading.
    pub fn send(&self, fields: &[&str]) -> bool {
        let msg = fields.join("\t");
        let bytes = &msg.as_bytes()[..msg.len().min(MAX)];
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::MSG_DONTWAIT;
        // SAFETY: sends from a live buffer.
        unsafe { libc::send(self.fd(), bytes.as_ptr().cast(), bytes.len(), flags) >= 0 }
    }

    /// The next message, split into fields. None when nothing waits, or
    /// when the other end is gone: [`closed`](Link::closed) tells which.
    pub fn recv(&self) -> Option<Vec<String>> {
        let mut buf = [0u8; MAX];
        // SAFETY: receives into a stack buffer of its own length.
        let n = unsafe {
            libc::recv(
                self.fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error().raw_os_error();
            if !matches!(e, Some(libc::EAGAIN | libc::EINTR)) {
                self.closed.set(true);
            }
            return None;
        }
        // We never send an empty message, so none is the end of the stream.
        if n == 0 {
            self.closed.set(true);
            return None;
        }
        let s = String::from_utf8_lossy(&buf[..n as usize]);
        Some(s.split('\t').map(String::from).collect())
    }

    /// Waits up to `ms` for the message `verb`, dropping others on the way.
    pub fn wait_for(&self, verb: &str, ms: u64) -> bool {
        let end = crate::rt::now_ms() + ms;
        loop {
            while let Some(m) = self.recv() {
                if m.first().is_some_and(|v| v == verb) {
                    return true;
                }
            }
            let now = crate::rt::now_ms();
            if now >= end {
                return false;
            }
            let mut fds = [crate::rt::pollfd(self.fd())];
            let _ = crate::rt::wait(&mut fds, Some(end));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_keep_their_bounds_and_fields() {
        let (shell, app) = Link::pair().unwrap();
        nonblock_cloexec(app.as_raw_fd()).unwrap();
        let app = Link::new(app);
        assert!(shell.send(&["pause"]));
        assert!(shell.send(&["toast", "Copied 3 lines"]));
        assert_eq!(app.recv().unwrap(), ["pause"]);
        assert_eq!(app.recv().unwrap(), ["toast", "Copied 3 lines"]);
        assert!(app.recv().is_none());
        assert!(app.send(&["paused"]));
        assert!(shell.wait_for("paused", 100));
        assert!(!shell.wait_for("paused", 10));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_hang_up_is_seen() {
        let (shell, app) = Link::pair().unwrap();
        assert!(shell.recv().is_none() && !shell.closed(), "nothing yet");
        drop(app);
        assert!(shell.recv().is_none());
        assert!(shell.closed());
    }
}
