//! Starting an app and knowing when it is gone, under the stock contract:
//! its own process group, cwd `/usr/share/APPLaunch`, the `LV_LINUX_*` and
//! `APPLAUNCH_LINUX_*` env, and done only when the whole group has exited.

use crate::apps::App;
use crate::desktop;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

pub const APP_CWD: &str = "/usr/share/APPLaunch";
pub const DEFAULT_TERMINAL: &str = "/usr/bin/gh0st-term -e";

/// Devices a child should use, passed in the env stock apps read.
pub struct Devices<'a> {
    pub fb: &'a str,
    pub keyboard: &'a str,
    pub keymap: &'a str,
}

/// The argv for `app`. Ours run directly. Third-party ones go through
/// `sh -c` as in stock, since the stock Exec rule allows `~` and quoting.
/// `Terminal=true` wraps either in the terminal command.
pub fn argv(app: &App, terminal: &str) -> Vec<String> {
    let mut v = if app.ours {
        desktop::split_exec(&app.entry.exec)
    } else {
        vec!["/bin/sh".into(), "-c".into(), app.entry.exec.clone()]
    };
    if app.entry.terminal {
        let mut t = desktop::split_exec(terminal);
        if !app.ours {
            // The terminal runs the shell line itself.
            v = vec!["/bin/sh".into(), "-c".into(), app.entry.exec.clone()];
        }
        t.append(&mut v);
        v = t;
    }
    v
}

pub struct Running {
    pub pgid: i32,
    leader_done: bool,
}

/// `pause` asks a terminal app to wait for a key after its command exits
/// (stock `Sysplause`). It reaches the terminal as `GH0ST_SYSPLAUSE=1`.
pub fn spawn(argv: &[String], dev: &Devices, pause: bool) -> io::Result<Running> {
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
        .env("GH0ST_SYSPLAUSE", if pause { "1" } else { "0" });
    if Path::new(APP_CWD).is_dir() {
        cmd.current_dir(APP_CWD);
    }
    let child = cmd.spawn()?;
    let pid = child.id() as i32;
    // The Child handle is dropped without waiting: `reap` collects the
    // leader and anything orphaned to us, so nothing stays a zombie.
    drop(child);
    Ok(Running {
        pgid: pid,
        leader_done: false,
    })
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

impl Running {
    /// Collects exited children after SIGCHLD. True when the app is done:
    /// its leader exited and no process is left in its group.
    pub fn reap(&mut self) -> bool {
        loop {
            let mut status = 0;
            // SAFETY: non-blocking wait for any child.
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            if pid == self.pgid {
                self.leader_done = true;
            }
        }
        self.leader_done && !self.group_alive()
    }

    /// The leader is gone but the group may not be: the caller rechecks on a
    /// short timer, since a leftover member that is not our child sends no
    /// SIGCHLD when it exits.
    pub fn leader_exited(&self) -> bool {
        self.leader_done
    }

    fn group_alive(&self) -> bool {
        // SAFETY: signal 0 only checks that the group exists.
        unsafe { libc::kill(-self.pgid, 0) == 0 }
    }

    pub fn terminate(&self) {
        // SAFETY: signalling our own child's group.
        unsafe { libc::kill(-self.pgid, libc::SIGTERM) };
    }

    pub fn kill(&self) {
        // SAFETY: as above.
        unsafe { libc::kill(-self.pgid, libc::SIGKILL) };
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

/// Collects finished helpers while no app runs.
pub fn reap_helpers() {
    loop {
        let mut status = 0;
        // SAFETY: non-blocking wait for any child.
        if unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) } <= 0 {
            break;
        }
    }
}

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
                DEFAULT_TERMINAL
            ),
            ["/usr/bin/gh0st", "settings"]
        );
        assert_eq!(
            argv(&app("python3 ~/g.py", false, false), DEFAULT_TERMINAL),
            ["/bin/sh", "-c", "python3 ~/g.py"]
        );
    }

    #[test]
    fn terminal_entries_run_inside_the_terminal() {
        assert_eq!(
            argv(&app("vim", false, true), DEFAULT_TERMINAL),
            ["/usr/bin/gh0st-term", "-e", "/bin/sh", "-c", "vim"]
        );
    }

    #[test]
    fn done_means_the_whole_group_is_gone() {
        let dev = Devices {
            fb: "/dev/null",
            keyboard: "/dev/null",
            keymap: "",
        };
        // The leader exits at once; its background child keeps the group
        // alive for a moment.
        let argv: Vec<String> = ["/bin/sh", "-c", "sleep 0.3 & exit 0"]
            .map(String::from)
            .to_vec();
        let mut r = spawn(&argv, &dev, false).unwrap();
        let start = crate::rt::now_ms();
        while !r.reap() {
            assert!(crate::rt::now_ms() - start < 5000, "group never finished");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            crate::rt::now_ms() - start >= 250,
            "returned before the background child exited"
        );
    }
}
