//! The reader that brings a big file into memory ahead of the line index,
//! find and save, so they read without waiting for the card. A process of
//! its own, at idle IO priority, so the page never blocks on the disk and
//! the screen's own reads go first. It reads once, from where it starts to
//! the end, and dies with the editor.

use std::io;
use std::path::Path;

/// Smaller files read fast enough in slices.
pub const MIN: u64 = 8 << 20;

/// Starts a reader of `path` from `from`. Returns its pid, for
/// `Cx::adopt`.
pub fn spawn(path: &Path, from: u64) -> io::Result<i32> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("--warm")
        .arg(path)
        .arg(from.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    Ok(cmd.spawn()?.id() as i32)
}

/// Stops a reader.
pub fn stop(pid: i32) {
    // SAFETY: a signal to a child we started and have not reaped.
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

/// `gh0st-text --warm PATH FROM`: reads the file to the end and exits.
pub fn main(path: &Path, from: u64) -> i32 {
    use std::os::unix::fs::FileExt;
    #[cfg(target_os = "linux")]
    {
        // IOPRIO_WHO_PROCESS, this process, the idle class.
        const IDLE: libc::c_long = 3 << 13;
        // SAFETY: a plain syscall about ourselves.
        unsafe { libc::syscall(libc::SYS_ioprio_set, 1, 0, IDLE) };
    }
    let Ok(f) = std::fs::File::open(path) else {
        return 1;
    };
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: advice on a file we own.
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL) };
    }
    let mut buf = vec![0u8; 256 << 10];
    let mut off = from;
    loop {
        match f.read_at(&mut buf, off) {
            Ok(0) | Err(_) => return 0,
            Ok(n) => off += n as u64,
        }
    }
}
