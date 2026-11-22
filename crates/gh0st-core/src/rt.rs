//! The event loop's parts. One thread blocks in `poll(2)` until a file
//! descriptor is ready or the earliest deadline passes. Nothing ticks: every
//! timer is a deadline someone asked for, so an idle shell does not wake.

use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

/// Milliseconds on the monotonic clock.
pub fn now_ms() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

/// The earliest of some optional deadlines.
pub fn earliest(deadlines: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    deadlines.into_iter().flatten().min()
}

pub fn pollfd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

/// A pollfd that also waits for room to write, when `write` is set.
pub fn pollfd_rw(fd: RawFd, read: bool, write: bool) -> libc::pollfd {
    let mut events = 0;
    if read {
        events |= libc::POLLIN;
    }
    if write {
        events |= libc::POLLOUT;
    }
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

/// Blocks until a descriptor is readable or `deadline` passes. Returns false
/// on timeout. EINTR counts as a wake: a signal has just been queued.
pub fn wait(fds: &mut [libc::pollfd], deadline: Option<u64>) -> io::Result<bool> {
    let timeout = match deadline {
        None => -1,
        Some(d) => d.saturating_sub(now_ms()).min(i32::MAX as u64) as i32,
    };
    for f in fds.iter_mut() {
        f.revents = 0;
    }
    // SAFETY: the slice is a valid array of pollfd for its length.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::Interrupted {
            Ok(true)
        } else {
            Err(e)
        };
    }
    Ok(n > 0)
}

pub fn readable(f: &libc::pollfd) -> bool {
    f.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
}

pub fn writable(f: &libc::pollfd) -> bool {
    f.revents & (libc::POLLOUT | libc::POLLERR) != 0
}

static SIG_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(sig: libc::c_int) {
    let fd = SIG_PIPE.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = sig as u8;
        // SAFETY: write(2) is async-signal-safe; a full pipe just drops it.
        unsafe { libc::write(fd, &b as *const u8 as *const libc::c_void, 1) };
    }
}

/// Signals delivered as bytes on a pipe the loop polls (the self-pipe trick),
/// so a handler never runs shell code.
pub struct Signals {
    read: RawFd,
}

impl Signals {
    pub fn install(sigs: &[libc::c_int]) -> io::Result<Signals> {
        let mut p = [0; 2];
        // SAFETY: p is a two-element array as pipe(2) wants.
        if unsafe { libc::pipe(p.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for fd in p {
            set_nonblock_cloexec(fd)?;
        }
        SIG_PIPE.store(p[1], Ordering::Relaxed);
        for &s in sigs {
            // SAFETY: a zeroed sigaction with a plain handler and SA_RESTART.
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = on_signal as *const () as usize;
                sa.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut sa.sa_mask);
                if libc::sigaction(s, &sa, std::ptr::null_mut()) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(Signals { read: p[0] })
    }

    pub fn fd(&self) -> RawFd {
        self.read
    }

    /// Signals received since the last call, in order, deduplicated.
    pub fn drain(&self) -> Vec<libc::c_int> {
        let mut buf = [0u8; 64];
        let mut out: Vec<libc::c_int> = Vec::new();
        loop {
            // SAFETY: reading into a local buffer of its stated size.
            let n =
                unsafe { libc::read(self.read, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                let s = b as libc::c_int;
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        out
    }
}

pub fn set_nonblock_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor we own.
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        if fl < 0 || libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd_fl = libc::fcntl(fd, libc::F_GETFD);
        if fd_fl < 0 || libc::fcntl(fd, libc::F_SETFD, fd_fl | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Local wall time as (hour, minute, second).
pub fn local_hms() -> (u32, u32, u32) {
    // SAFETY: time(NULL) and localtime_r with a local struct tm.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
    }
}

/// Local date and time now: year, month 1-12, day, hour, minute.
pub fn local_ymdhm() -> (i32, u32, u32, u32, u32) {
    // SAFETY: as in local_hms.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (
            tm.tm_year + 1900,
            tm.tm_mon as u32 + 1,
            tm.tm_mday as u32,
            tm.tm_hour as u32,
            tm.tm_min as u32,
        )
    }
}

/// A moment in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// Day of the year, 0 to 365: with `year`, which day it is.
    pub yday: i32,
    /// 0 is January.
    pub month: usize,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Seconds since 1970 as local time.
pub fn local_at(epoch: i64) -> Local {
    // SAFETY: localtime_r into a local struct tm.
    unsafe {
        let t = epoch as _;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        Local {
            year: tm.tm_year + 1900,
            yday: tm.tm_yday,
            month: tm.tm_mon as usize,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
        }
    }
}

/// "Nov": a month's short name, `month` 0 for January.
pub fn month_short(month: usize) -> &'static str {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS.get(month).copied().unwrap_or("")
}

/// Seconds since 1970 for a local wall time, in the system's time zone.
/// `None` when the C library can't place it.
pub fn local_epoch(y: i32, mon: u32, d: u32, h: u32, min: u32) -> Option<i64> {
    // SAFETY: a zeroed tm is valid; mktime reads and normalises it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = y - 1900;
        tm.tm_mon = mon as i32 - 1;
        tm.tm_mday = d as i32;
        tm.tm_hour = h as i32;
        tm.tm_min = min as i32;
        tm.tm_isdst = -1;
        let t = libc::mktime(&mut tm);
        (t != -1).then_some(t as i64)
    }
}

/// "14:32", or "2:32" on a 12-hour clock, and the monotonic deadline of the
/// next minute change. With the battery read, the only timers a lit, idle home
/// screen keeps.
pub fn clock_text(h24: bool) -> (String, u64) {
    let (h, m, s) = local_hms();
    (
        clock_format(h, m, h24),
        now_ms() + (60 - s.min(59)) as u64 * 1000,
    )
}

/// Today as the lock screen says it: "Sunday 22 November".
pub fn date_text() -> String {
    // SAFETY: as in local_hms.
    let (wday, day, mon) = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_wday as usize, tm.tm_mday as u32, tm.tm_mon as usize)
    };
    date_format(wday, day, mon)
}

/// `wday` 0 is Sunday, `mon` 0 is January, as in `struct tm`.
pub fn date_format(wday: usize, day: u32, mon: usize) -> String {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!(
        "{} {day} {}",
        DAYS.get(wday).unwrap_or(&""),
        MONTHS.get(mon).unwrap_or(&"")
    )
}

pub fn clock_format(h: u32, m: u32, h24: bool) -> String {
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{}:{m:02}", (h + 11) % 12 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_times_out_at_the_deadline() {
        let start = now_ms();
        let woke = wait(&mut [], Some(start + 30)).unwrap();
        assert!(!woke);
        assert!(now_ms() >= start + 29);
    }

    #[test]
    fn dates_read_in_words() {
        assert_eq!(date_format(0, 22, 10), "Sunday 22 November");
        assert_eq!(date_format(6, 1, 0), "Saturday 1 January");
    }

    #[test]
    fn earliest_ignores_missing_deadlines() {
        assert_eq!(earliest([None, Some(9), Some(4), None]), Some(4));
        assert_eq!(earliest([None, None]), None);
    }

    #[test]
    fn a_signal_wakes_the_loop_through_the_pipe() {
        let s = Signals::install(&[libc::SIGUSR1]).unwrap();
        // SAFETY: raising a signal we installed a handler for.
        unsafe { libc::raise(libc::SIGUSR1) };
        let mut fds = [pollfd(s.fd())];
        assert!(wait(&mut fds, Some(now_ms() + 1000)).unwrap());
        assert_eq!(s.drain(), [libc::SIGUSR1]);
    }

    #[test]
    fn the_clock_reads_hh_mm() {
        let (s, next) = clock_text(true);
        assert_eq!(s.len(), 5);
        assert_eq!(&s[2..3], ":");
        assert!(next > now_ms() && next <= now_ms() + 60_000);
        assert_eq!(clock_format(0, 5, false), "12:05");
        assert_eq!(clock_format(14, 32, false), "2:32");
        assert_eq!(clock_format(9, 0, true), "09:00");
    }
}
