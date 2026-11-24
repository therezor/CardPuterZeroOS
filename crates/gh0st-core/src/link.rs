//! The shell link: one socket pair per app, one message per packet, UTF-8
//! fields split by tabs (`docs/apps.md`, Shell link). A path in a field is
//! escaped with [`path_field`], so tabs, newlines and any byte survive.

use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Where an app finds its end: the fd number, in its env.
pub const ENV: &str = "GH0ST_SHELL_FD";
/// The link's version, sent in `hello`. Version 1 peers read at most
/// [`LEGACY_MAX`] bytes a message.
pub const VERSION: u32 = 2;
/// The longest message a version 1 peer reads.
pub const LEGACY_MAX: usize = 512;
/// The longest message either side sends: a path and its verb fit. macOS
/// caps a datagram at 2048 bytes, so host runs carry less.
#[cfg(target_os = "linux")]
pub const MAX: usize = 8192;
#[cfg(not(target_os = "linux"))]
pub const MAX: usize = 2048;

/// A path as one field: `%XX` for `%`, control bytes and bytes that are not
/// UTF-8. [`path_of`] reverses it.
pub fn path_field(p: &Path) -> String {
    let bytes = p.as_os_str().as_bytes();
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if c == '%' || (c as u32) < 0x20 || c == '\u{7f}' {
                out.push_str(&format!("%{:02X}", c as u32));
            } else {
                out.push(c);
            }
        }
        for b in chunk.invalid() {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path a [`path_field`] names. A broken escape is kept as it is.
pub fn path_of(field: &str) -> PathBuf {
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    PathBuf::from(OsStr::from_bytes(&out))
}

/// Messages `wait_for` keeps at most; an app says a few as it pauses.
const KEPT_MAX: usize = 32;

pub struct Link {
    fd: OwnedFd,
    /// The other end hung up. Its fd then polls readable for good, so the
    /// owner drops the link rather than spin.
    closed: std::cell::Cell<bool>,
    /// Messages `wait_for` passed over, for `recv` to give out first.
    kept: std::cell::RefCell<std::collections::VecDeque<Vec<String>>>,
}

/// A socket pair that keeps message bounds: SEQPACKET where the system has
/// it (Linux), datagrams elsewhere. Both ends close on exec.
pub fn raw_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    for ty in [libc::SOCK_SEQPACKET, libc::SOCK_DGRAM] {
        // SAFETY: socketpair fills a two-int array.
        if unsafe { libc::socketpair(libc::AF_UNIX, ty, 0, fds.as_mut_ptr()) } == 0 {
            // SAFETY: both fds are fresh and owned from here.
            let (a, b) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
            for fd in [&a, &b] {
                // SAFETY: fcntl on an fd we own.
                unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
            }
            return Ok((a, b));
        }
    }
    Err(io::Error::last_os_error())
}

fn nonblock_cloexec(fd: RawFd) -> io::Result<()> {
    crate::rt::set_nonblock_cloexec(fd)
}

impl Link {
    /// A new pair: the shell's end, and the app's end for the child to
    /// inherit. Both close on exec until the spawn clears the app's.
    pub fn pair() -> io::Result<(Link, OwnedFd)> {
        let (a, b) = raw_pair()?;
        nonblock_cloexec(a.as_raw_fd())?;
        Ok((Link::new(a), b))
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
            kept: Default::default(),
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
    /// if it didn't go: the other side is gone or not reading, or the
    /// message is longer than [`MAX`]. A long message is never cut.
    pub fn send(&self, fields: &[&str]) -> bool {
        let msg = fields.join("\t");
        if msg.len() > MAX {
            return false;
        }
        let bytes = msg.as_bytes();
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
        if let Some(m) = self.kept.borrow_mut().pop_front() {
            return Some(m);
        }
        let mut buf = [0u8; MAX];
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        // SAFETY: an all-zero msghdr is valid; it points at one iovec.
        let mut hdr: libc::msghdr = unsafe { std::mem::zeroed() };
        hdr.msg_iov = &mut iov;
        hdr.msg_iovlen = 1;
        // SAFETY: receives into a stack buffer of its own length.
        let n = unsafe { libc::recvmsg(self.fd(), &mut hdr, libc::MSG_DONTWAIT) };
        if n > 0 && hdr.msg_flags & libc::MSG_TRUNC != 0 {
            // Longer than any peer may send: drop it whole, never half.
            return self.recv();
        }
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

    /// Waits up to `ms` for the message `verb`. Others on the way are kept
    /// for `recv`: an app says `awake 0` as it pauses, only once.
    pub fn wait_for(&self, verb: &str, ms: u64) -> bool {
        self.wait_msg(verb, ms).is_some()
    }

    /// As [`wait_for`](Link::wait_for), giving the message.
    pub fn wait_msg(&self, verb: &str, ms: u64) -> Option<Vec<String>> {
        let end = crate::rt::now_ms() + ms;
        let mut passed = Vec::new();
        let found = loop {
            match self.recv() {
                Some(m) if m.first().is_some_and(|v| v == verb) => break Some(m),
                Some(m) => {
                    if passed.len() < KEPT_MAX {
                        passed.push(m);
                    }
                    continue;
                }
                None => {}
            }
            let now = crate::rt::now_ms();
            if now >= end || self.closed() {
                break None;
            }
            let mut fds = [crate::rt::pollfd(self.fd())];
            let _ = crate::rt::wait(&mut fds, Some(end));
        };
        self.kept.borrow_mut().extend(passed);
        found
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

    #[test]
    fn what_comes_before_the_awaited_message_is_kept() {
        let (shell, app) = Link::pair().unwrap();
        nonblock_cloexec(app.as_raw_fd()).unwrap();
        let app = Link::new(app);
        assert!(app.send(&["awake", "0"]));
        assert!(app.send(&["toast", "Saved"]));
        assert!(app.send(&["paused"]));
        assert!(shell.wait_for("paused", 100));
        assert_eq!(shell.recv().unwrap(), ["awake", "0"]);
        assert_eq!(shell.recv().unwrap(), ["toast", "Saved"]);
        assert!(shell.recv().is_none());
    }

    #[test]
    fn any_path_survives_a_field() {
        let odd = PathBuf::from(OsStr::from_bytes(b"/home/u/a\tb\nc%d \xff\xc3\xa9.txt"));
        let f = path_field(&odd);
        assert!(!f.contains(['\t', '\n']));
        assert_eq!(f, "/home/u/a%09b%0Ac%25d %FF\u{e9}.txt");
        assert_eq!(path_of(&f), odd);
        assert_eq!(path_of("/plain/path"), PathBuf::from("/plain/path"));
        assert_eq!(path_of("/bad%2"), PathBuf::from("/bad%2"));
        assert_eq!(path_of("/end%"), PathBuf::from("/end%"));
    }

    #[test]
    fn a_long_message_goes_whole_or_not_at_all() {
        let (shell, app) = Link::pair().unwrap();
        nonblock_cloexec(app.as_raw_fd()).unwrap();
        let app = Link::new(app);
        let long = "x".repeat(MAX - 5);
        assert!(shell.send(&["open", &long]));
        assert_eq!(app.recv().unwrap(), ["open", long.as_str()]);
        assert!(!shell.send(&["open", &"y".repeat(MAX)]), "never cut");
        assert!(app.recv().is_none());
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
