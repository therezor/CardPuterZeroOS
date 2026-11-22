//! What the lock screen and the root helper share about the user's
//! password: the wait after wrong tries, the try count, whether the account
//! has a password at all, and what `unix_chkpwd` reads. The hashes are
//! yescrypt, which static musl can't compute, so the check itself is always
//! `unix_chkpwd`'s.

use std::io;
use std::path::Path;

pub const CHKPWD: &str = "/usr/sbin/unix_chkpwd";
pub const FREE_TRIES: u32 = 3;
const FIRST_WAIT: u64 = 5;
const MAX_WAIT: u64 = 300;
/// Counts suspend too, and `/run` starts empty each boot.
#[cfg(target_os = "linux")]
const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
#[cfg(not(target_os = "linux"))]
const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

/// Seconds left before another try is allowed: three are free, then each
/// waits 5 s, doubling to 5 minutes.
pub fn lock_wait(fails: u32, last: u64, now: u64) -> u64 {
    if fails < FREE_TRIES {
        return 0;
    }
    let wait = FIRST_WAIT
        .checked_shl(fails - FREE_TRIES)
        .unwrap_or(MAX_WAIT)
        .min(MAX_WAIT);
    (last + wait).saturating_sub(now)
}

/// Seconds since boot, the clock the try count keeps.
pub fn boottime() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(CLOCK, &mut ts) };
    ts.tv_sec as u64
}

/// Wrong tries so far and when the last one was, in `boottime` seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tries {
    pub fails: u32,
    pub last: u64,
}

impl Tries {
    pub fn parse(s: &str) -> Option<Tries> {
        let mut t = Tries::default();
        for word in s.split_whitespace() {
            match word.split_once('=')? {
                ("fails", v) => t.fails = v.parse().ok()?,
                ("last", v) => t.last = v.parse().ok()?,
                _ => {}
            }
        }
        Some(t)
    }

    pub fn format(&self) -> String {
        format!("fails={} last={}\n", self.fails, self.last)
    }

    /// The count in `path`, or none yet. A count from a past boot is
    /// dropped: its clock started again.
    pub fn load(path: &Path, now: u64) -> Tries {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| Tries::parse(&s))
            .filter(|t| t.last <= now)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(self.format().as_bytes())
    }

    pub fn wait(&self, now: u64) -> u64 {
        lock_wait(self.fails, self.last, now)
    }
}

/// What `passwd -S` says about the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Account {
    Password,
    NoPassword,
    /// `!` or `*` in shadow: no password can match.
    Locked,
}

/// `passwd -S` prints `name P 11/21/2026 0 99999 7 -1`: the second word
/// says whether the account has a usable password.
pub fn parse_status(out: &str) -> Option<Account> {
    match out.split_whitespace().nth(1)? {
        "P" | "PS" => Some(Account::Password),
        "NP" => Some(Account::NoPassword),
        "L" | "LK" => Some(Account::Locked),
        _ => None,
    }
}

/// What `unix_chkpwd` reads on stdin: the password and a NUL.
pub fn chkpwd_input(password: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(password.len() + 1);
    msg.extend_from_slice(password);
    msg.push(0);
    msg
}

pub fn user_name(uid: u32) -> Option<String> {
    // SAFETY: getpwuid returns NULL or a pointer to static storage, read at
    // once; callers are single-threaded.
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_str()
            .ok()
            .map(String::from)
    }
}

/// Zeroes `buf` in a way the compiler keeps.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: b is a valid &mut u8; volatile so the zeroing stays.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_tries_then_a_doubling_wait() {
        assert_eq!(lock_wait(2, 100, 100), 0);
        assert_eq!(lock_wait(3, 100, 100), 5);
        assert_eq!(lock_wait(3, 100, 104), 1);
        assert_eq!(lock_wait(3, 100, 105), 0);
        assert_eq!(lock_wait(4, 100, 100), 10);
        assert_eq!(lock_wait(9, 100, 100), 300, "capped");
        assert_eq!(lock_wait(80, 100, 100), 300, "no overflow");
    }

    #[test]
    fn tries_round_trip_and_drop_a_past_boot() {
        let dir = std::env::temp_dir().join(format!("gh0st-tries-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("lock");
        assert_eq!(Tries::load(&p, 50), Tries::default(), "none yet");
        let t = Tries { fails: 4, last: 40 };
        t.save(&p).unwrap();
        assert_eq!(Tries::load(&p, 50), t);
        assert_eq!(Tries::load(&p, 10), Tries::default(), "a clock from before");
        assert_eq!(Tries::parse("fails=x"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn passwd_status_words() {
        assert_eq!(
            parse_status("rezor P 11/21/2026 0 99999 7 -1\n"),
            Some(Account::Password)
        );
        assert_eq!(
            parse_status("zero NP 11/21/2026 0 99999 7 -1"),
            Some(Account::NoPassword)
        );
        assert_eq!(
            parse_status("zero L 11/21/2026 0 99999 7 -1"),
            Some(Account::Locked)
        );
        assert_eq!(parse_status("passwd: Permission denied."), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn chkpwd_reads_a_nul_after_the_password() {
        assert_eq!(chkpwd_input(b"hunter2"), b"hunter2\0");
    }
}
