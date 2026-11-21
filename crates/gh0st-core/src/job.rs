//! A helper whose answer we want: `nmcli`, `bluetoothctl`, `wpctl`, `pkexec`.
//! Its stdout and stderr share one non-blocking pipe that joins the event
//! loop, so a slow helper never blocks the shell. It is done once the pipe
//! has closed and the reaper has seen it exit.

use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Command, Stdio};

pub struct Job {
    pub pid: i32,
    pipe: Option<OwnedFd>,
    out: Vec<u8>,
    status: Option<i32>,
}

/// Starts `argv` with `input` written to its stdin, then closed. The C
/// locale keeps the output in the shape the parsers expect.
pub fn spawn(argv: &[&str], input: Option<&[u8]>) -> io::Result<Job> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    let mut p = [0; 2];
    // SAFETY: p is a two-element array as pipe(2) wants.
    if unsafe { libc::pipe(p.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both ends were just created and are owned only here.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(p[0]), OwnedFd::from_raw_fd(p[1])) };
    crate::rt::set_nonblock_cloexec(read.as_raw_fd())?;
    let err = write.try_clone()?;
    let mut child = Command::new(prog)
        .args(args)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("LANGUAGE")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::from(write))
        .stderr(Stdio::from(err))
        .spawn()?;
    if let (Some(mut stdin), Some(data)) = (child.stdin.take(), input) {
        // Small writes (a password, a line or two) fit the pipe at once.
        let _ = stdin.write_all(data);
    }
    Ok(Job {
        pid: child.id() as i32,
        pipe: Some(read),
        out: Vec::new(),
        status: None,
    })
}

impl Job {
    /// The pipe to poll, or -1 once it has closed (poll skips it).
    pub fn fd(&self) -> RawFd {
        self.pipe.as_ref().map_or(-1, |p| p.as_raw_fd())
    }

    /// Reads what is ready. Closes the pipe at end of file.
    pub fn read(&mut self) {
        let Some(fd) = self.pipe.as_ref().map(|p| p.as_raw_fd()) else {
            return;
        };
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: reading into a local buffer of its stated size.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                self.out.extend_from_slice(&buf[..n as usize]);
                continue;
            }
            if n == 0 {
                self.pipe = None;
            }
            return;
        }
    }

    /// The reaper saw it exit with `status`.
    pub fn exited(&mut self, status: i32) {
        self.status = Some(status);
    }

    pub fn done(&self) -> bool {
        self.pipe.is_none() && self.status.is_some()
    }

    /// Exited with status 0.
    pub fn ok(&self) -> bool {
        self.status
            .is_some_and(|s| libc::WIFEXITED(s) && libc::WEXITSTATUS(s) == 0)
    }

    /// The exit code, once it exited normally. `None` before that, or if a
    /// signal ended it.
    pub fn code(&self) -> Option<i32> {
        self.status
            .filter(|&s| libc::WIFEXITED(s))
            .map(|s| libc::WEXITSTATUS(s))
    }

    /// What it printed so far, stdout and stderr together.
    pub fn bytes(&self) -> &[u8] {
        &self.out
    }

    pub fn output(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_and_status_arrive_through_the_loop_parts() {
        let _one = crate::child::CHILD_TESTS.lock().unwrap();
        let mut j = spawn(
            &[
                "/bin/sh",
                "-c",
                "read x; echo got $x; echo oops >&2; exit 3",
            ],
            Some(b"hi\n"),
        )
        .unwrap();
        let start = crate::rt::now_ms();
        while !j.done() {
            let mut fds = [crate::rt::pollfd(j.fd())];
            crate::rt::wait(&mut fds, Some(crate::rt::now_ms() + 50)).unwrap();
            j.read();
            for (pid, status) in crate::child::reap() {
                if pid == j.pid {
                    j.exited(status);
                }
            }
            assert!(crate::rt::now_ms() - start < 3000, "job never finished");
        }
        assert_eq!(j.output(), "got hi\noops\n");
        assert!(!j.ok());
    }
}
