//! The shell link end to end: the real binary under --sim, a socket pair
//! for the shell's end. Behind another app the terminal draws nothing.

use gh0st_core::link::Link;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn frames(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn behind_another_app_it_draws_nothing() {
    let tmp = std::env::temp_dir().join(format!("gh0st-term-link-{}", std::process::id()));
    let dir = tmp.join("frames");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&dir).unwrap();
    let (shell, app) = Link::pair().unwrap();
    let fd = app.as_raw_fd();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gh0st-term"));
    cmd.args(["--sim", dir.to_str().unwrap(), "-e", "/bin/sh", "-c"])
        .arg("while true; do echo x; sleep 0.02; done")
        .env("GH0ST_SHELL_FD", fd.to_string())
        .env("HOME", &tmp)
        // Kept open, so the sim never runs out of script.
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: fcntl only, between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            libc::fcntl(fd, libc::F_SETFD, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    drop(app);

    assert!(shell.wait_for("hello", 3000), "no hello");
    std::thread::sleep(Duration::from_millis(400));
    assert!(frames(&dir) > 0, "nothing drawn in front");

    assert!(shell.send(&["pause"]));
    assert!(shell.wait_for("paused", 1000), "no paused");
    std::thread::sleep(Duration::from_millis(100));
    let before = frames(&dir);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(frames(&dir), before, "drew while behind");

    assert!(shell.send(&["resume"]));
    std::thread::sleep(Duration::from_millis(400));
    assert!(frames(&dir) > before, "no frame after resume");

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&tmp);
}
