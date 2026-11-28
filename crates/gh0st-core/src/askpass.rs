//! sudo for an app that asks for root (docs/isolation.md, Root apps). Apps
//! have no terminal, so sudo asks `gh0st-askpass`, which asks the shell
//! over a socket only the device user can reach. The shell asks the user
//! on its own screen and answers with the password, or with no.
//!
//! Messages, one per packet:
//!
//! 1. helper: `ask<TAB>prompt`
//! 2. shell: `go`, once it has checked the helper is its own binary, run by
//!    a real sudo, in a root app's cgroup. Or `no`.
//! 3. helper: `ready`, once no other process can read its memory or trace it
//! 4. shell: `ok<TAB>password`, or `no`

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// The helper sudo runs: a hard link to the gh0st binary.
pub const HELPER: &str = "/usr/libexec/gh0st-askpass";
/// Holds the `sudo` that adds `-A`, first on a root app's PATH.
pub const SHIM_DIR: &str = "/usr/lib/gh0st/sudo";
/// The shell's socket, in `XDG_RUNTIME_DIR`.
pub const SOCKET: &str = "gh0st-askpass";
/// A packet never needs more: a prompt, or a password of the lock
/// screen's 512 bytes.
pub const MAX_MSG: usize = 1024;

pub fn socket_path(runtime: &Path) -> PathBuf {
    runtime.join(SOCKET)
}

/// `ask<TAB>prompt`, the prompt cleaned of tabs and newlines.
pub fn ask(prompt: &str) -> Vec<u8> {
    let p: String = prompt
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect();
    format!("ask\t{p}").into_bytes()
}

pub const GO: &[u8] = b"go";
pub const READY: &[u8] = b"ready";
pub const NO: &[u8] = b"no";

/// No tracer in a `/proc/self/status` body: nothing attached before the
/// helper shut the door.
pub fn untraced(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("TracerPid:"))
        .is_some_and(|v| v.trim() == "0")
}

/// The shell's answer: the password, or None for no.
pub fn password(msg: &[u8]) -> Option<&[u8]> {
    msg.strip_prefix(b"ok\t")
}

/// `ok<TAB>password` in a buffer sized up front, so it never moves and
/// leaves a copy behind. The caller wipes it.
pub fn ok(pw: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(3 + pw.len());
    v.extend_from_slice(b"ok\t");
    v.extend_from_slice(pw);
    v
}

/// The parent pid from a `/proc/<pid>/stat` body. The name in brackets
/// may hold spaces and brackets, so fields count from the last `)`.
pub fn ppid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Whether a `/proc/<pid>/status` body is a real sudo the device user ran:
/// named `sudo`, real uid `uid`, effective uid root. Only a setuid sudo
/// is that; one in a sandbox never gains root.
pub fn is_sudo_of(status: &str, uid: u32) -> bool {
    let field = |k: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(k))
            .map(str::trim)
    };
    let name = field("Name:");
    let ids: Vec<u32> = field("Uid:")
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    name == Some("sudo") && ids.len() >= 2 && ids[0] == uid && ids[1] == 0
}

#[cfg(target_os = "linux")]
fn addr(path: &Path) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: a zeroed sockaddr_un is valid.
    let mut a: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    a.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= a.sun_path.len() {
        return Err(io::Error::other("socket path too long"));
    }
    for (d, s) in a.sun_path.iter_mut().zip(bytes) {
        *d = *s as libc::c_char;
    }
    let len = std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1;
    Ok((a, len as libc::socklen_t))
}

#[cfg(target_os = "linux")]
fn seqpacket(nonblock: bool) -> io::Result<OwnedFd> {
    use std::os::fd::FromRawFd;
    let flags =
        libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | if nonblock { libc::SOCK_NONBLOCK } else { 0 };
    // SAFETY: socket takes plain ints and returns a new fd or -1.
    let fd = unsafe { libc::socket(libc::AF_UNIX, flags, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the fd is new and ours alone.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The shell's listener at `path`, non-blocking, reachable by the device
/// user alone.
#[cfg(target_os = "linux")]
pub fn listen(path: &Path) -> io::Result<OwnedFd> {
    use std::os::unix::fs::PermissionsExt;
    let fd = seqpacket(true)?;
    let _ = std::fs::remove_file(path);
    let (a, len) = addr(path)?;
    // SAFETY: bind reads `len` bytes of the sockaddr we built.
    if unsafe { libc::bind(fd.as_raw_fd(), (&a as *const libc::sockaddr_un).cast(), len) } < 0 {
        return Err(io::Error::last_os_error());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    // SAFETY: listen on our own socket.
    if unsafe { libc::listen(fd.as_raw_fd(), 4) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(not(target_os = "linux"))]
pub fn listen(_: &Path) -> io::Result<OwnedFd> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// The helper's end: blocking, as it has nothing else to do.
#[cfg(target_os = "linux")]
pub fn connect(path: &Path) -> io::Result<OwnedFd> {
    let fd = seqpacket(false)?;
    let (a, len) = addr(path)?;
    // SAFETY: connect reads `len` bytes of the sockaddr we built.
    if unsafe { libc::connect(fd.as_raw_fd(), (&a as *const libc::sockaddr_un).cast(), len) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(not(target_os = "linux"))]
pub fn connect(_: &Path) -> io::Result<OwnedFd> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// One packet, without blocking or SIGPIPE.
pub fn send(fd: &OwnedFd, msg: &[u8]) -> bool {
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: send reads `msg.len()` bytes of a live buffer.
    let n = unsafe { libc::send(fd.as_raw_fd(), msg.as_ptr().cast(), msg.len(), flags) };
    n == msg.len() as isize
}

/// One packet into `buf`: its length, 0 when the other end closed, or an
/// error (`WouldBlock` on a non-blocking socket with nothing to read).
pub fn recv(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: recv writes at most `buf.len()` bytes into a live buffer.
    let n = unsafe { libc::recv(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_comes_after_the_last_bracket() {
        assert_eq!(ppid("4242 (sudo) S 4100 4242 4100 0 -1"), Some(4100));
        assert_eq!(ppid("77 (a) b (c)) R 12 77 1"), Some(12));
        assert_eq!(ppid("garbage"), None);
    }

    #[test]
    fn only_a_setuid_sudo_of_the_user_counts() {
        let sudo = "Name:\tsudo\nUmask:\t0022\nState:\tS (sleeping)\nUid:\t1000\t0\t0\t0\nGid:\t1000\t1000\t1000\t1000\n";
        assert!(is_sudo_of(sudo, 1000));
        assert!(!is_sudo_of(sudo, 1001), "another user's");
        // In a sandbox sudo never gains root; a renamed process neither.
        let no_root = sudo.replace("1000\t0\t0\t0", "1000\t1000\t1000\t1000");
        assert!(!is_sudo_of(&no_root, 1000));
        let renamed = "Name:\tsudo-ish\nUid:\t1000\t0\t0\t0\n";
        assert!(!is_sudo_of(renamed, 1000));
        let fake = "Name:\tpython3\nUid:\t1000\t0\t0\t0\n";
        assert!(!is_sudo_of(fake, 1000));
    }

    #[test]
    fn a_traced_helper_is_seen() {
        assert!(untraced("Name:\tgh0st-askpass\nTracerPid:\t0\n"));
        assert!(!untraced("TracerPid:\t4242\n"));
        assert!(!untraced("Name:\tx\n"), "unknown counts as traced");
    }

    #[test]
    fn messages_read_back() {
        assert_eq!(
            ask("[sudo] password\tfor lol:\n"),
            b"ask\t[sudo] password for lol: ".to_vec()
        );
        let m = ok(b"s3cret\tx");
        assert_eq!(password(&m), Some(&b"s3cret\tx"[..]));
        assert_eq!(password(b"no"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_question_and_its_answer_cross_the_socket() {
        use std::os::fd::FromRawFd;
        let dir = std::env::temp_dir().join(format!("gh0st-askpass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = socket_path(&dir);
        let l = listen(&path).unwrap();
        let c = connect(&path).unwrap();
        // SAFETY: accept4 on our listener; the new fd is ours.
        let s = unsafe {
            libc::accept4(
                l.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        assert!(s >= 0);
        // SAFETY: the fd is new and ours alone.
        let s = unsafe { OwnedFd::from_raw_fd(s) };
        assert!(send(&c, &ask("pw?")));
        let mut buf = [0u8; MAX_MSG];
        let n = recv(&s, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"ask\tpw?");
        assert!(send(&s, &ok(b"hunter2")));
        let n = recv(&c, &mut buf).unwrap();
        assert_eq!(password(&buf[..n]), Some(&b"hunter2"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
