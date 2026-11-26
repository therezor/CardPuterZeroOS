//! Starting an app and knowing when it is gone, under the stock contract:
//! its own process group, cwd `/usr/share/APPLaunch`, the `LV_LINUX_*` and
//! `APPLAUNCH_LINUX_*` env, and done only when the whole group has exited.

use crate::apps::App;
use crate::desktop;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const APP_CWD: &str = "/usr/share/APPLaunch";
pub const DEFAULT_TERMINAL: &str = "/usr/bin/gh0st-term -e";

/// Devices a child should use, passed in the env stock apps read.
pub struct Devices<'a> {
    pub fb: &'a str,
    pub keyboard: &'a str,
    pub keymap: &'a str,
}

/// The argv for `app`, opening `files` if its `Exec` takes them. Ours run
/// directly, with the files in place of the field code. Third-party ones go
/// through `sh -c` as in stock, since the stock Exec rule allows `~` and
/// quoting; their files follow as `sh`'s positional arguments and never
/// enter the line. `Terminal=true` wraps either in the terminal command.
pub fn argv(app: &App, terminal: &str, files: &[PathBuf]) -> Vec<OsString> {
    let exec = &app.entry.exec;
    let mut v = if app.ours {
        desktop::expand_exec(exec, files)
    } else {
        let mut v: Vec<OsString> = vec![
            "/bin/sh".into(),
            "-c".into(),
            desktop::shell_line(exec).into(),
        ];
        if let Some(code) = desktop::field_code(exec) {
            v.push("sh".into());
            v.extend(code.args(files));
        }
        v
    };
    if app.entry.terminal {
        let mut t: Vec<OsString> = desktop::split_exec(terminal)
            .into_iter()
            .map(Into::into)
            .collect();
        t.append(&mut v);
        v = t;
    }
    v
}

/// `pause` asks a terminal app to wait for a key after its command exits
/// (stock `Sysplause`). It reaches the terminal as `GH0ST_SYSPLAUSE=1`.
/// `link` is the app's end of the shell link; the app finds its number in
/// `GH0ST_SHELL_FD`. Returns the new process group, whose id is the
/// leader's pid.
pub fn spawn<S: AsRef<OsStr>>(
    argv: &[S],
    dev: &Devices,
    pause: bool,
    link: Option<RawFd>,
) -> io::Result<i32> {
    spawn_with(argv, dev, pause, link, &Extra::default())
}

/// What else a launch sets up.
#[derive(Default)]
pub struct Extra<'a> {
    /// The app's cgroup (`sandbox::Cgroups`): the child joins it before
    /// exec, so all it starts is named for the app.
    pub cgroup: Option<&'a Path>,
    pub env: Vec<(&'static str, String)>,
}

/// As `spawn`, with `extra`.
pub fn spawn_with<S: AsRef<OsStr>>(
    argv: &[S],
    dev: &Devices,
    pause: bool,
    link: Option<RawFd>,
    extra: &Extra,
) -> io::Result<i32> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::other("empty command"))?;
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .process_group(0)
        .stdin(Stdio::null())
        .env("LV_LINUX_FBDEV_DEVICE", dev.fb)
        .env("APPLAUNCH_LINUX_FBDEV_DEVICE", dev.fb)
        .env("LV_LINUX_KEYBOARD_DEVICE", dev.keyboard)
        .env("APPLAUNCH_LINUX_KEYBOARD_DEVICE", dev.keyboard)
        .env("APPLAUNCH_LINUX_KEYBOARD_MAP", dev.keymap)
        .env("GH0ST_SYSPLAUSE", if pause { "1" } else { "0" })
        // The stock camera grabs the keyboard unless told not to, and a grab
        // cuts the shell off from Fn+K, ESC held and the volume keys.
        .env("CAMERA_APP_KEYBOARD_GRAB", "false");
    // A preload reaches an app only through its grant, in `extra`.
    cmd.env_remove("LD_PRELOAD");
    if Path::new(APP_CWD).is_dir() {
        cmd.current_dir(APP_CWD);
    }
    for (k, v) in &extra.env {
        cmd.env(k, v);
    }
    if let Some(cg) = extra.cgroup {
        use std::os::unix::ffi::OsStrExt;
        let procs = std::ffi::CString::new(cg.join("cgroup.procs").as_os_str().as_bytes())
            .map_err(io::Error::other)?;
        // SAFETY: open, write and close are async-signal-safe, and the path
        // was built before the fork. Best effort: an app left out of its
        // cgroup still runs, named by its executable.
        unsafe {
            cmd.pre_exec(move || {
                let fd = libc::open(procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                if fd >= 0 {
                    libc::write(fd, b"0".as_ptr().cast(), 1);
                    libc::close(fd);
                }
                Ok(())
            });
        }
    }
    if let Some(fd) = link {
        cmd.env(crate::link::ENV, fd.to_string());
        // SAFETY: fcntl is async-signal-safe. It keeps the fd open across
        // exec in the child alone.
        unsafe {
            cmd.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = cmd.spawn()?;
    // The Child handle is dropped without waiting: `reap` collects the
    // leader and anything orphaned to us, so nothing stays a zombie.
    Ok(child.id() as i32)
}

/// Makes the shell the reaper for orphans of its apps, so an app that forks
/// and exits early leaves nothing behind (Linux only).
pub fn become_subreaper() {
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with an integer argument.
    unsafe {
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
    }
}

fn signal_group(pgid: i32, sig: libc::c_int) {
    if pgid > 0 {
        // SAFETY: signalling a process group we started.
        unsafe { libc::kill(-pgid, sig) };
    }
}

/// Pauses every process in the group. False if the signal reached none of
/// them: a group led by a root process, say.
pub fn stop(pgid: i32) -> bool {
    // SAFETY: signalling a process group we started.
    pgid > 0 && unsafe { libc::kill(-pgid, libc::SIGSTOP) } == 0
}

/// Waits up to `ms` until every live member of the group is stopped. A
/// member the shell may not signal (run through sudo) never stops.
pub fn group_stopped(pgid: i32, ms: u64) -> bool {
    let end = crate::rt::now_ms() + ms;
    loop {
        if !group_has_running(pgid) {
            return true;
        }
        if crate::rt::now_ms() >= end {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Any member of the group still running, from `/proc`. Without `/proc` (a
/// host run) the stop is trusted.
fn group_has_running(pgid: i32) -> bool {
    crate::procfs::processes().any(|p| p.pgrp == pgid && !matches!(p.state, 'T' | 't' | 'Z' | 'X'))
}

pub fn cont(pgid: i32) {
    signal_group(pgid, libc::SIGCONT);
}

/// Asks the group to exit. A paused group cannot act on SIGTERM until it
/// runs, so it gets SIGCONT too.
pub fn terminate(pgid: i32) {
    signal_group(pgid, libc::SIGTERM);
    signal_group(pgid, libc::SIGCONT);
}

pub fn kill(pgid: i32) {
    signal_group(pgid, libc::SIGKILL);
}

/// True while any process of the group exists.
pub fn group_alive(pgid: i32) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    pgid > 0 && unsafe { libc::kill(-pgid, 0) } == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    Yes,
    /// The leader exited instead; it has been reaped here.
    Exited,
    /// Not stopped in time: in uninterruptible sleep, say.
    Late,
}

/// Waits up to `ms` for our child `pid` to stop after SIGSTOP. SIGSTOP is
/// queued at once but takes effect when the process next runs.
pub fn wait_stopped(pid: i32, ms: u64) -> Stopped {
    let end = crate::rt::now_ms() + ms;
    loop {
        let mut status = 0;
        // SAFETY: a non-blocking wait on our own child.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED | libc::WNOHANG) };
        if r == pid {
            return if libc::WIFSTOPPED(status) {
                Stopped::Yes
            } else {
                Stopped::Exited
            };
        }
        if r < 0 {
            return Stopped::Exited;
        }
        if crate::rt::now_ms() >= end {
            return Stopped::Late;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Collects every exited child: apps, their orphans, helpers and jobs. One
/// reaper for all, since `waitpid(-1)` in two places would steal each
/// other's exits. Returns (pid, status).
pub fn reap() -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    loop {
        let mut status = 0;
        // SAFETY: non-blocking wait for any child.
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            return out;
        }
        out.push((pid, status));
    }
}

/// Runs a helper without waiting for it, for one-shot commands like
/// `wpctl set-volume`. Reaped by the next `reap` or `reap_helpers`.
pub fn fire(argv: &[&str]) {
    if let Some((p, a)) = argv.split_first() {
        let _ = Command::new(p)
            .args(a)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Tests that start children hold this: `reap` takes any child's exit, so
/// two such tests running at once would steal each other's.
#[cfg(test)]
pub(crate) static CHILD_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::DesktopEntry;

    fn app(exec: &str, ours: bool, terminal: bool) -> App {
        let entry = DesktopEntry {
            name: "x".into(),
            exec: exec.into(),
            terminal,
            ..Default::default()
        };
        App::for_test(entry, ours)
    }

    #[test]
    fn ours_run_direct_and_third_party_through_sh() {
        assert_eq!(
            argv(
                &app("/usr/bin/gh0st settings", true, false),
                DEFAULT_TERMINAL,
                &[]
            ),
            ["/usr/bin/gh0st", "settings"]
        );
        assert_eq!(
            argv(&app("python3 ~/g.py", false, false), DEFAULT_TERMINAL, &[]),
            ["/bin/sh", "-c", "python3 ~/g.py"]
        );
    }

    #[test]
    fn terminal_entries_run_inside_the_terminal() {
        assert_eq!(
            argv(&app("vim", false, true), DEFAULT_TERMINAL, &[]),
            ["/usr/bin/gh0st-term", "-e", "/bin/sh", "-c", "vim"]
        );
        let f = [PathBuf::from("/home/u/notes.txt")];
        assert_eq!(
            argv(&app("vim %f", false, true), DEFAULT_TERMINAL, &f),
            [
                "/usr/bin/gh0st-term",
                "-e",
                "/bin/sh",
                "-c",
                "vim ${1+\"$1\"}",
                "sh",
                "/home/u/notes.txt"
            ]
        );
    }

    #[test]
    fn one_file_code_with_no_file_passes_no_argument() {
        // Another test's waitpid(-1) would take this child's exit.
        let _one = CHILD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("gh0st-nofile-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join("out");
        let script = d.join("count");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho $# >> {}\n", out.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let a = app(&format!("{} %f", script.display()), false, false);
        for files in [&[][..], &[PathBuf::from("/tmp/a b")][..]] {
            let v = argv(&a, DEFAULT_TERMINAL, files);
            let st = std::process::Command::new(&v[0])
                .args(&v[1..])
                .status()
                .unwrap();
            assert!(st.success());
        }
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "0\n1\n");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn files_reach_a_real_shell_as_they_are() {
        // Another test's waitpid(-1) would take this child's exit.
        let _one = CHILD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        use std::os::unix::ffi::OsStrExt;
        let d = std::env::temp_dir().join(format!("gh0st-argv-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let odd = [
            "it's \"quoted\" $(touch pwned) `x`.txt".as_bytes(),
            b"tab\there\nnewline",
            b"\xff\xfe not utf8",
        ];
        let files: Vec<PathBuf> = odd
            .iter()
            .map(|n| d.join(std::ffi::OsStr::from_bytes(n)))
            .collect();
        let out = d.join("out");
        // A third-party program that writes each argument it gets.
        let script = d.join("put");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor a; do printf '%s\\0' \"$a\"; done > {}\n",
                out.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let v = argv(
            &app(&format!("{} %F", script.display()), false, false),
            DEFAULT_TERMINAL,
            &files,
        );
        let st = std::process::Command::new(&v[0])
            .args(&v[1..])
            .current_dir(&d)
            .status()
            .unwrap();
        assert!(st.success());
        let got = std::fs::read(&out).unwrap();
        let want: Vec<u8> = files
            .iter()
            .flat_map(|f| f.as_os_str().as_bytes().iter().copied().chain([0]))
            .collect();
        assert_eq!(got, want);
        assert!(!d.join("pwned").exists(), "nothing in a name ran");
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn dev() -> Devices<'static> {
        Devices {
            fb: "/dev/null",
            keyboard: "/dev/null",
            keymap: "",
        }
    }

    fn sh(cmd: &str) -> Vec<String> {
        ["/bin/sh", "-c", cmd].map(String::from).to_vec()
    }

    #[test]
    fn the_group_outlives_its_leader_until_the_last_member_exits() {
        let _one = CHILD_TESTS.lock().unwrap();
        // The leader exits at once; its background child keeps the group
        // alive for a moment.
        let pgid = spawn(&sh("sleep 0.3 & exit 0"), &dev(), false, None).unwrap();
        let start = crate::rt::now_ms();
        let mut leader_done = false;
        while !(leader_done && !group_alive(pgid)) {
            leader_done |= reap().iter().any(|&(p, _)| p == pgid);
            assert!(crate::rt::now_ms() - start < 5000, "group never finished");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            crate::rt::now_ms() - start >= 250,
            "returned before the background child exited"
        );
    }

    #[test]
    fn a_paused_group_stops_resumes_and_still_takes_terminate() {
        let _one = CHILD_TESTS.lock().unwrap();
        let pgid = spawn(&sh("exec sleep 30"), &dev(), false, None).unwrap();
        assert!(stop(pgid));
        assert_eq!(wait_stopped(pgid, 2000), Stopped::Yes);
        assert!(group_stopped(pgid, 1000));
        cont(pgid);
        stop(pgid);
        assert_eq!(wait_stopped(pgid, 2000), Stopped::Yes);
        terminate(pgid);
        let start = crate::rt::now_ms();
        while group_alive(pgid) {
            reap();
            assert!(
                crate::rt::now_ms() - start < 3000,
                "a stopped group ignored SIGTERM"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
