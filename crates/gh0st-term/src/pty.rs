//! A pseudo-terminal and the child on its other end.
//!
//! The child gets its own session with the PTY as its controlling terminal.
//! So the shell's SIGSTOP to the app's group never reaches it, and closing
//! the master hangs it up. The SDK reaps it: nothing here calls `waitpid`.

use std::ffi::CStr;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// The environment a program in the terminal sees.
const TERM: &str = "xterm-256color";

/// Variables meant for the app alone, which a child must not inherit.
const PRIVATE: &[&str] = &[
    "GH0ST_SHELL_FD",
    "GH0ST_SYSPLAUSE",
    "GH0ST_APP_ID",
    "LV_LINUX_",
    "APPLAUNCH_LINUX_",
];

pub struct Pty {
    master: OwnedFd,
    pub pid: i32,
}

fn last() -> io::Error {
    io::Error::last_os_error()
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(last()) } else { Ok(r) }
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Pty {
    /// Starts `argv` on a new PTY of `cols` x `rows`. `arg0`, when given,
    /// replaces the name the program sees (a login shell's `-bash`).
    pub fn spawn(
        argv: &[String],
        arg0: Option<&str>,
        cwd: Option<&Path>,
        cols: u16,
        rows: u16,
    ) -> io::Result<Pty> {
        let prog = argv.first().ok_or(io::ErrorKind::InvalidInput)?;
        // SAFETY: opens a new PTY master; the fd is owned right after.
        let m = check(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) })?;
        // SAFETY: m is a fresh fd that nothing else owns.
        let master = unsafe { OwnedFd::from_raw_fd(m) };
        // SAFETY: fcntl, grantpt and unlockpt on the fd we just opened.
        unsafe {
            check(libc::fcntl(m, libc::F_SETFD, libc::FD_CLOEXEC))?;
            let fl = check(libc::fcntl(m, libc::F_GETFL))?;
            check(libc::fcntl(m, libc::F_SETFL, fl | libc::O_NONBLOCK))?;
            check(libc::grantpt(m))?;
            check(libc::unlockpt(m))?;
        }
        // SAFETY: ptsname returns a static buffer, copied before any other
        // call; the app has one thread.
        let name = unsafe {
            let p = libc::ptsname(m);
            if p.is_null() {
                return Err(last());
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        };
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(&name)?;
        set_size(slave.as_raw_fd(), cols, rows)?;

        let mut cmd = Command::new(prog);
        cmd.args(&argv[1..])
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        if let Some(a) = arg0 {
            cmd.arg0(a);
        }
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy();
            if PRIVATE.iter().any(|p| k.starts_with(p)) {
                cmd.env_remove(&*k);
            }
        }
        cmd.env("TERM", TERM)
            .env("COLORTERM", "truecolor")
            .env("TERM_PROGRAM", "gh0st-term");
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(last());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(last());
                }
                Ok(())
            });
        }
        // The SDK reaps every child, so the handle is dropped unwaited.
        #[allow(clippy::zombie_processes)]
        let child = cmd.spawn()?;
        Ok(Pty {
            master,
            pid: child.id() as i32,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    /// Reads what the child wrote. `Ok(0)` and EIO both mean it hung up.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: reads into a live buffer of its own length.
        let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 { Err(last()) } else { Ok(n as usize) }
    }

    /// Tells the child its new size; the kernel sends it SIGWINCH.
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        set_size(self.fd(), cols, rows)
    }
}

fn set_size(fd: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let ws = winsize(cols, rows);
    // SAFETY: TIOCSWINSZ reads a winsize from a live struct.
    check(unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &ws) }).map(|_| ())
}

/// The user's shell: `$SHELL`, else the passwd entry, else `/bin/sh`.
pub fn shell() -> String {
    if let Some(s) = std::env::var_os("SHELL").filter(|s| Path::new(s).is_file()) {
        return s.to_string_lossy().into_owned();
    }
    // SAFETY: getpwuid's result is read at once; the app has one thread.
    let pw = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_shell.is_null() {
            None
        } else {
            Some(
                CStr::from_ptr((*pw).pw_shell)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    };
    pw.filter(|s| Path::new(s).is_file())
        .unwrap_or_else(|| "/bin/sh".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_until(p: &Pty, want: &str) -> String {
        let mut out = Vec::new();
        let mut buf = [0u8; 1024];
        for _ in 0..200 {
            match p.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(_) => break,
            }
            if String::from_utf8_lossy(&out).contains(want) {
                break;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn the_child_sees_a_terminal_of_our_size() {
        let argv: Vec<String> = [
            "/bin/sh",
            "-c",
            "stty size; echo $TERM; test -t 0 && echo tty",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let p = Pty::spawn(&argv, None, None, 53, 12).unwrap();
        let out = read_until(&p, "tty");
        assert!(out.contains("12 53"), "{out:?}");
        assert!(out.contains("xterm-256color"), "{out:?}");
        assert!(out.contains("tty"), "{out:?}");
    }
}
