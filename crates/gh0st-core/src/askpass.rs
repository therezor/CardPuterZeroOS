//! sudo inside a root app has no terminal to read a password from, and
//! the shell's launch rules keep it that way. The wire here carries
//! sudo's prompt to the shell over a socket only this user can reach, and
//! the answer back. `gh0st-askpass` is the client; the shell listens.

use std::cmp::Ordering;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Set in a root app's env: the socket to call.
pub const ENV: &str = "GH0ST_ASKPASS";

/// A prompt longer than this is refused before its body is read.
pub const MAX_PROMPT: usize = 4096;

/// Unfinished connections the shell keeps; a further one is closed unread.
pub const MAX_PENDING: usize = 4;

const HELPER: &str = "gh0st-askpass";

#[derive(Debug)]
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// The socket under `$XDG_RUNTIME_DIR`, in a folder this user alone
    /// can open. `Ok(None)` without the variable: there is no socket, and
    /// the shell runs without askpass.
    pub fn open() -> io::Result<Option<Listener>> {
        let Some(base) = std::env::var_os("XDG_RUNTIME_DIR") else {
            return Ok(None);
        };
        // SAFETY: getuid has no failure mode.
        let uid = unsafe { libc::getuid() };
        Self::open_in(Path::new(&base).join(format!("gh0st-askpass-{uid}"))).map(Some)
    }

    /// The socket in `dir`, made 0700 if missing, for whoever asks for one.
    pub fn open_in(dir: PathBuf) -> io::Result<Listener> {
        // SAFETY: getuid has no failure mode.
        Self::in_dir(dir, unsafe { libc::getuid() })
    }

    fn in_dir(dir: PathBuf, uid: libc::uid_t) -> io::Result<Listener> {
        // Made 0700 by us, and checked: a link or a stranger's folder
        // planted where we write is refused.
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
        let m = std::fs::symlink_metadata(&dir)?;
        if !m.is_dir() || m.uid() != uid || m.mode() & 0o077 != 0 {
            return Err(io::Error::other("the askpass folder isn't ours alone"));
        }
        let path = dir.join("askpass");
        if std::fs::symlink_metadata(&path).is_ok() {
            // The socket of a shell that is gone is replaced. A live one
            // keeps its owner: two shells would answer with two prompts.
            match UnixStream::connect(&path) {
                Ok(_) => return Err(io::Error::other("another shell owns the askpass socket")),
                Err(_) => std::fs::remove_file(&path)?,
            }
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Listener { listener, path })
    }

    pub fn fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The next connection, nonblocking. `WouldBlock` when there is none.
    pub fn accept(&self) -> io::Result<UnixStream> {
        let (s, _) = self.listener.accept()?;
        s.set_nonblocking(true)?;
        Ok(s)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Who connected. `pid` is 0 where the kernel does not tell, as on a host
/// without `SO_PEERCRED`, and the ancestry check is then skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: libc::pid_t,
    pub uid: libc::uid_t,
    pub gid: libc::gid_t,
}

pub fn peer(fd: RawFd) -> io::Result<Peer> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: a zeroed ucred is valid input, and as long as len says.
        let mut c: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: c is writable for len bytes.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut c as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Peer {
            pid: c.pid,
            uid: c.uid,
            gid: c.gid,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (0 as libc::uid_t, 0 as libc::gid_t);
        // SAFETY: getpeereid fills two ids.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Peer { pid: 0, uid, gid })
    }
}

/// Whether a connection may ask: ours, and descended from the app it is
/// asked for (`owner`, its group leader's pid). Ancestry decides, not the
/// process group: a program sudo starts may make a session of its own.
/// Without a pid to check, uid and owner decide.
pub fn admit(cred: Peer, owner: Option<libc::pid_t>, my_uid: libc::uid_t) -> bool {
    cred.uid == my_uid
        && match owner {
            None => false,
            Some(leader) => cred.pid <= 0 || crate::procfs::belongs_to(cred.pid, leader),
        }
}

/// Writes the request: the length as two bytes big-endian, then the
/// prompt. The client keeps its write side open. A half-close would read
/// as EOF on the shell's side while the client still waits for its answer.
pub fn send_request(s: &mut UnixStream, prompt: &str) -> io::Result<()> {
    let len =
        u16::try_from(prompt.len()).map_err(|_| io::Error::other("the prompt is too long"))?;
    s.write_all(&len.to_be_bytes())?;
    s.write_all(prompt.as_bytes())
}

fn announced(buf: &[u8]) -> io::Result<usize> {
    let n = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if n > MAX_PROMPT {
        return Err(io::Error::other("the prompt is too long"));
    }
    Ok(n)
}

fn read_into(buf: &mut Vec<u8>, s: &mut UnixStream, room: usize) -> io::Result<()> {
    let mut chunk = [0u8; MAX_PROMPT + 3];
    let n = s.read(&mut chunk[..room])?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the asker left before a whole prompt",
        ));
    }
    buf.extend_from_slice(&chunk[..n]);
    Ok(())
}

/// Reads one request into `buf`, the connection's own buffer. `Ok(None)`
/// until the whole prompt is in. An error for a length over `MAX_PROMPT`
/// (before any body is read), bytes after the prompt, an asker that left
/// early, or a read failure.
pub fn read_request(buf: &mut Vec<u8>, s: &mut UnixStream) -> io::Result<Option<String>> {
    if buf.len() < 2 {
        read_into(buf, s, 2 - buf.len())?;
        if buf.len() < 2 {
            return Ok(None);
        }
    }
    let want = 2 + announced(buf)?;
    if buf.len() < want {
        // One byte past the prompt, so bytes after it show in this read.
        read_into(buf, s, want + 1 - buf.len())?;
    }
    match buf.len().cmp(&want) {
        Ordering::Greater => Err(io::Error::other("bytes after the prompt")),
        Ordering::Equal => {
            let prompt = String::from_utf8_lossy(&buf[2..]).into_owned();
            buf.clear();
            Ok(Some(prompt))
        }
        Ordering::Less => Ok(None),
    }
}

/// Answers and drops the connection, which the client reads to its end.
/// The write may fail once the client is gone; the caller does not care.
pub fn answer(mut s: UnixStream, pw: Option<&[u8]>) {
    let _ = s.write_all(&[if pw.is_some() { b'1' } else { b'0' }]);
    if let Some(pw) = pw {
        let _ = s.write_all(pw);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Password(String),
    Refused,
}

/// One request, start to finish. No timeout: the user may be away, and
/// sudo waits as long as the helper does.
pub fn ask(path: &Path, prompt: &str) -> io::Result<Reply> {
    let mut s = UnixStream::connect(path)?;
    send_request(&mut s, prompt)?;
    let mut tag = [0u8; 1];
    s.read_exact(&mut tag)?;
    match tag[0] {
        b'0' => Ok(Reply::Refused),
        b'1' => {
            let mut pw = Vec::new();
            s.read_to_end(&mut pw)?;
            Ok(Reply::Password(String::from_utf8_lossy(&pw).into_owned()))
        }
        other => Err(io::Error::other(format!("bad reply byte {other}"))),
    }
}

/// The helper's `main`. The prompt is its argument, the socket
/// `$GH0ST_ASKPASS`. One stderr line on failure.
pub fn client(prompt: Option<&str>) -> i32 {
    let fail = |what: String| {
        eprintln!("{HELPER}: {what}");
        1
    };
    let Some(prompt) = prompt else {
        return fail("usage: gh0st-askpass <prompt>".into());
    };
    let Some(path) = std::env::var_os(ENV) else {
        return fail(format!("${ENV} is not set"));
    };
    match ask(Path::new(&path), prompt) {
        Ok(Reply::Password(pw)) => {
            let mut out = io::stdout().lock();
            let _ = out.write_all(pw.as_bytes());
            let _ = out.flush();
            0
        }
        Ok(Reply::Refused) => fail("refused".into()),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => fail("no reply".into()),
        Err(e) => fail(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::time::Duration;

    static N: AtomicUsize = AtomicUsize::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gh0st-askpass-{}-{}",
            std::process::id(),
            N.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn my_uid() -> libc::uid_t {
        // SAFETY: getuid has no failure mode.
        unsafe { libc::getuid() }
    }

    fn listener_in(dir: &Path) -> Listener {
        Listener::in_dir(dir.join("gh0st-askpass-me"), my_uid()).unwrap()
    }

    fn accepted(l: &Listener) -> UnixStream {
        loop {
            match l.accept() {
                Ok(s) => {
                    s.set_nonblocking(false).unwrap();
                    return s;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("accept: {e}"),
            }
        }
    }

    fn whole(l: &Listener) -> UnixStream {
        let mut s = accepted(l);
        let mut buf = Vec::new();
        loop {
            match read_request(&mut buf, &mut s).unwrap() {
                Some(_) => return s,
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    #[test]
    fn a_prompt_crosses_and_the_answer_comes_back() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        let t = std::thread::spawn(move || ask(&path, "[sudo] password for pi: ").unwrap());
        let mut s = accepted(&l);
        #[cfg(target_os = "linux")]
        {
            let cred = peer(s.as_raw_fd()).unwrap();
            assert_eq!(cred.uid, my_uid());
            let me = std::process::id() as libc::pid_t;
            assert!(
                admit(cred, Some(me), my_uid()),
                "our own connection descends from us"
            );
            assert!(!admit(cred, Some(1), my_uid()), "init owns nobody's app");
            assert!(!admit(cred, None, my_uid()), "no app, no question");
            assert!(!admit(cred, Some(me), my_uid() + 1), "another user");
        }
        let mut buf = Vec::new();
        loop {
            match read_request(&mut buf, &mut s).unwrap() {
                Some(p) => {
                    assert_eq!(p, "[sudo] password for pi: ");
                    break;
                }
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        answer(s, Some(b"hunter22"));
        assert_eq!(t.join().unwrap(), Reply::Password("hunter22".into()));
    }

    #[test]
    fn a_refusal_is_a_zero() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        let t = std::thread::spawn(move || ask(&path, "pw: ").unwrap());
        let s = whole(&l);
        answer(s, None);
        assert_eq!(t.join().unwrap(), Reply::Refused);
    }

    #[test]
    fn a_split_write_reassembles() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        let t = std::thread::spawn(move || {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(&5u16.to_be_bytes()).unwrap();
            s.write_all(b"hel").unwrap();
            std::thread::sleep(Duration::from_millis(30));
            s.write_all(b"lo").unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).unwrap();
            r
        });
        let mut s = accepted(&l);
        let mut buf = Vec::new();
        let mut partial = 0;
        loop {
            match read_request(&mut buf, &mut s).unwrap() {
                Some(p) => {
                    assert_eq!(p, "hello");
                    break;
                }
                None => partial += 1,
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(partial >= 1, "the prompt came in parts, none whole");
        assert!(
            buf.is_empty(),
            "the buffer is ready for the next connection"
        );
        answer(s, Some(b"x"));
        assert_eq!(t.join().unwrap(), "1x");
    }

    #[test]
    fn a_long_prompt_is_refused_before_its_body() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        std::thread::spawn(move || {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(&((MAX_PROMPT + 1) as u16).to_be_bytes())
                .unwrap();
            s.write_all(&[b'x'; 16]).unwrap();
            let mut r = String::new();
            let _ = s.read_to_string(&mut r);
        });
        let mut s = accepted(&l);
        let mut buf = Vec::new();
        let e = read_request(&mut buf, &mut s).unwrap_err();
        assert_eq!(e.to_string(), "the prompt is too long");
        assert_eq!(buf.len(), 2, "the length alone was read");
    }

    #[test]
    fn bytes_after_the_prompt_are_an_error() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        std::thread::spawn(move || {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(&2u16.to_be_bytes()).unwrap();
            s.write_all(b"hi!").unwrap();
            let mut r = String::new();
            let _ = s.read_to_string(&mut r);
        });
        let mut s = accepted(&l);
        let mut buf = Vec::new();
        let e = read_request(&mut buf, &mut s).unwrap_err();
        assert_eq!(e.to_string(), "bytes after the prompt");
    }

    #[test]
    fn an_empty_prompt_is_whole() {
        let l = listener_in(&scratch());
        let path = l.path().to_path_buf();
        let t = std::thread::spawn(move || ask(&path, "").unwrap());
        let s = whole(&l);
        answer(s, Some(b""));
        assert_eq!(t.join().unwrap(), Reply::Password(String::new()));
    }

    #[test]
    fn a_folder_others_can_read_is_refused() {
        let dir = scratch();
        let _ = std::fs::DirBuilder::new()
            .mode(0o755)
            .create(dir.join("gh0st-askpass-me"));
        let e = Listener::in_dir(dir.join("gh0st-askpass-me"), my_uid()).unwrap_err();
        assert_eq!(e.to_string(), "the askpass folder isn't ours alone");
        assert!(!dir.join("gh0st-askpass-me/askpass").exists());
    }

    #[test]
    fn a_stale_socket_is_replaced_and_a_live_one_kept() {
        let dir = scratch();
        let socket = dir.join("gh0st-askpass-me/askpass");
        let _ = std::fs::create_dir_all(socket.parent().unwrap());
        let _ = std::fs::set_permissions(
            socket.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        );
        {
            let _stale = UnixListener::bind(&socket).unwrap();
        }
        assert!(socket.exists(), "a closed listener leaves its file");
        let l = Listener::in_dir(dir.join("gh0st-askpass-me"), my_uid()).unwrap();
        assert_eq!(l.path(), socket);
        let e = Listener::in_dir(dir.join("gh0st-askpass-me"), my_uid()).unwrap_err();
        assert_eq!(e.to_string(), "another shell owns the askpass socket");
        drop(l);
        assert!(!socket.exists());
    }

    #[test]
    fn without_a_pid_only_the_uid_and_owner_decide() {
        let host = Peer {
            pid: 0,
            uid: 7,
            gid: 7,
        };
        assert!(admit(host, Some(42), 7));
        assert!(!admit(host, Some(42), 8));
        assert!(!admit(host, None, 7));
    }
}
