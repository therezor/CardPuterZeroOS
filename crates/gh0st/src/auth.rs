//! The helper's password check. pkexec runs the helper as root for any
//! process of the device user, so a gated verb wants that user's password on
//! stdin. `unix_chkpwd` checks it: the hashes are yescrypt, which static musl
//! can't compute.
//!
//! State lives in `/run`, root only, so a restart clears it. Three tries are
//! free, then each one waits 5 s, doubling to 5 minutes. A right password
//! holds for 5 minutes, but only for the same calling process (its pid and
//! start time), like sudo's per-parent timestamp.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::process::{Command, Stdio};

use gh0st_core::password::{CHKPWD, boottime, chkpwd_input, lock_wait};
pub(crate) use gh0st_core::password::{user_name, wipe};

const DIR: &str = "/run/gh0st-helper";
const GRACE_SECS: u64 = 300;
const MAX_PASSWORD: usize = 512;

#[derive(Debug, PartialEq, Eq)]
pub enum Auth {
    Ok,
    Required,
    Wrong,
    /// Seconds until the next try.
    Locked(u64),
    Failed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Grace {
    uid: u32,
    ppid: i32,
    start: u64,
    at: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct State {
    fails: u32,
    last: u64,
    grace: Option<Grace>,
}

impl State {
    fn parse(s: &str) -> Option<State> {
        let mut st = State::default();
        for word in s.split_whitespace() {
            let (k, v) = word.split_once('=')?;
            match k {
                "fails" => st.fails = v.parse().ok()?,
                "last" => st.last = v.parse().ok()?,
                "grace" => {
                    let f: Vec<&str> = v.split(':').collect();
                    let [uid, ppid, start, at] = f.as_slice() else {
                        return None;
                    };
                    st.grace = Some(Grace {
                        uid: uid.parse().ok()?,
                        ppid: ppid.parse().ok()?,
                        start: start.parse().ok()?,
                        at: at.parse().ok()?,
                    });
                }
                _ => return None,
            }
        }
        Some(st)
    }

    fn format(&self) -> String {
        let mut s = format!("fails={} last={}", self.fails, self.last);
        if let Some(g) = self.grace {
            s += &format!(" grace={}:{}:{}:{}", g.uid, g.ppid, g.start, g.at);
        }
        s + "\n"
    }
}

fn grace_valid(g: &Grace, uid: u32, ppid: i32, start: u64, now: u64) -> bool {
    g.uid == uid && g.ppid == ppid && g.start == start && now >= g.at && now - g.at < GRACE_SECS
}

/// Field 22 of `/proc/PID/stat`, counted after the name, which may hold
/// spaces and brackets.
fn stat_starttime(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// One line from stdin, without its newline. Stops at a newline so a
/// terminal works too.
pub(crate) fn read_password() -> Vec<u8> {
    read_line(0)
}

/// One line from `fd`, a byte at a time with no buffer, so nothing after
/// the newline is taken: `files-worker` hands the rest of its stdin to the
/// worker. A line longer than a password ends the read.
pub(crate) fn read_line(fd: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    while out.len() < MAX_PASSWORD {
        // SAFETY: reads one byte into a one-byte buffer.
        let n = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
        match n {
            1 if byte[0] == b'\n' => break,
            1 => out.push(byte[0]),
            _ => break,
        }
    }
    if out.last() == Some(&b'\r') {
        out.pop();
    }
    out
}

fn chkpwd(user: &str, password: &[u8]) -> bool {
    let child = Command::new(CHKPWD)
        .args([user, "nonull"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let mut msg = chkpwd_input(password);
        let _ = stdin.write_all(&msg);
        wipe(&mut msg);
    }
    child.wait().is_ok_and(|s| s.success())
}

fn save(f: &mut File, st: &State) -> std::io::Result<()> {
    f.set_len(0)?;
    f.rewind()?;
    f.write_all(st.format().as_bytes())?;
    f.sync_all()
}

/// Decides whether the caller may run a gated verb.
pub fn check() -> Auth {
    check_given(None)
}

/// As `check`, with the password line already read (`files-worker` always
/// reads one, so nothing it didn't want reaches the worker). Under grace it
/// is wiped unused.
pub fn check_given(given: Option<Vec<u8>>) -> Auth {
    // SAFETY: plain syscalls with no arguments.
    let (uid, euid, ppid) = unsafe { (libc::getuid(), libc::geteuid(), libc::getppid()) };
    if euid != 0 {
        return Auth::Failed("not root; run through pkexec".into());
    }
    let caller = match std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        Some(0) => return Auth::Ok,
        Some(u) => u,
        None if uid == 0 => return Auth::Ok,
        None => return Auth::Failed("not called through pkexec".into()),
    };
    let Some(user) = user_name(caller) else {
        return Auth::Failed(format!("no user {caller}"));
    };
    match check_as(caller, &user, ppid, given) {
        Ok(a) => a,
        Err(e) => Auth::Failed(format!("{DIR}: {e}")),
    }
}

fn check_as(uid: u32, user: &str, ppid: i32, given: Option<Vec<u8>>) -> std::io::Result<Auth> {
    let mut given = given;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(DIR)?;
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(format!("{DIR}/state"))?;
    // SAFETY: f is an open file; the lock goes when it closes.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut text = String::new();
    f.read_to_string(&mut text)?;
    let Some(mut st) = State::parse(&text) else {
        return Ok(Auth::Failed("unreadable state".into()));
    };
    let start = std::fs::read_to_string(format!("/proc/{ppid}/stat"))
        .ok()
        .and_then(|s| stat_starttime(&s))
        .unwrap_or(0);
    let now = boottime();
    if st
        .grace
        .is_some_and(|g| ppid > 1 && grace_valid(&g, uid, ppid, start, now))
    {
        if let Some(g) = &mut given {
            wipe(g);
        }
        return Ok(Auth::Ok);
    }
    let mut password = given.unwrap_or_else(read_password);
    if password.is_empty() {
        return Ok(Auth::Required);
    }
    let wait = lock_wait(st.fails, st.last, now);
    if wait > 0 {
        wipe(&mut password);
        return Ok(Auth::Locked(wait));
    }
    // Counted before the check, so parallel calls can't skip the limit.
    st.fails += 1;
    st.last = now;
    save(&mut f, &st)?;
    let ok = chkpwd(user, &password);
    wipe(&mut password);
    if !ok {
        return Ok(Auth::Wrong);
    }
    st.fails = 0;
    // SAFETY: plain syscall. A changed parent means the caller died.
    let same = unsafe { libc::getppid() } == ppid;
    st.grace = (ppid > 1 && same && start > 0).then_some(Grace {
        uid,
        ppid,
        start,
        at: now,
    });
    save(&mut f, &st)?;
    Ok(Auth::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_read_leaves_what_follows_it() {
        let mut fds = [0; 2];
        // SAFETY: a socket pair into a two-int array.
        assert_eq!(
            unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) },
            0
        );
        let msg = b"hunter22\r\nready\n";
        // SAFETY: writes the bytes of a live buffer to our own socket.
        unsafe { libc::write(fds[1], msg.as_ptr().cast(), msg.len()) };
        assert_eq!(read_line(fds[0]), b"hunter22");
        let mut rest = [0u8; 16];
        // SAFETY: reads into a live buffer from our own socket.
        let n = unsafe { libc::read(fds[0], rest.as_mut_ptr().cast(), rest.len()) };
        assert_eq!(
            &rest[..n as usize],
            b"ready\n",
            "nothing after the line was taken"
        );
        // SAFETY: closes the fds this test opened.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn a_line_past_a_password_stops_the_read() {
        let mut fds = [0; 2];
        // SAFETY: a socket pair into a two-int array.
        unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
        let long = vec![b'a'; MAX_PASSWORD + 10];
        // SAFETY: writes a live buffer to our own socket.
        unsafe { libc::write(fds[1], long.as_ptr().cast(), long.len()) };
        assert_eq!(read_line(fds[0]).len(), MAX_PASSWORD);
        // SAFETY: closes the fds this test opened.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn grace_holds_for_one_process_for_five_minutes() {
        let g = Grace {
            uid: 1000,
            ppid: 42,
            start: 777,
            at: 1000,
        };
        assert!(grace_valid(&g, 1000, 42, 777, 1299));
        assert!(!grace_valid(&g, 1000, 42, 777, 1300), "expired");
        assert!(!grace_valid(&g, 1000, 43, 777, 1001), "another process");
        assert!(!grace_valid(&g, 1000, 42, 778, 1001), "a reused pid");
        assert!(!grace_valid(&g, 1001, 42, 777, 1001), "another user");
        assert!(!grace_valid(&g, 1000, 42, 777, 999), "clock went back");
    }

    #[test]
    fn start_time_survives_odd_names() {
        let stat =
            "42 (gh0st (set) x) S 1 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1000 50";
        assert_eq!(stat_starttime(stat), Some(98765));
        assert_eq!(stat_starttime("garbage"), None);
    }

    #[test]
    fn state_round_trips() {
        let st = State {
            fails: 4,
            last: 123,
            grace: Some(Grace {
                uid: 1000,
                ppid: 9,
                start: 8,
                at: 7,
            }),
        };
        assert_eq!(State::parse(&st.format()), Some(st));
        assert_eq!(State::parse(""), Some(State::default()));
        assert_eq!(State::parse("fails=x"), None);
        assert_eq!(State::parse("grace=1:2"), None);
    }
}
