//! sudo's password prompt end to end (docs/isolation.md, Root apps): the
//! real shell under `--sim`, a root app that runs the real `gh0st-askpass`
//! as sudo would, and a fake `unix_chkpwd`. A sim has no sudo and no
//! cgroups, so `GH0ST_ASKPASS_SIM_APP` names the asking app; on a device
//! the shell reads it from the helper's cgroup. Linux only: the shell
//! learns the helper's pid from `SO_PEERCRED`.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Answers each call with the next code in `$CODES`, then 0.
const CHKPWD: &str = r#"#!/bin/sh
/bin/cat >/dev/null
n=$(/bin/cat "$SIM_DIR/n" 2>/dev/null || echo 0)
echo $((n + 1)) > "$SIM_DIR/n"
code=$(echo "$CODES" | /usr/bin/cut -d' ' -f$((n + 1)))
exit ${code:-0}
"#;

/// What sudo does with the helper: runs it and reads its line.
const APP: &str = r#"#!/bin/sh
"$SIM_DIR/gh0st-askpass" "[sudo] password for me: " > "$SIM_DIR/got"
echo "askpass exit $?" >> "$SIM_DIR/log"
sleep 3
"#;

struct Sim {
    dir: PathBuf,
}

impl Sim {
    /// One app on home. `exec_tail` with `sudo` in it makes it a root app.
    fn new(name: &str, exec_tail: &str) -> Sim {
        let dir = std::env::temp_dir().join(format!("gh0st-sudo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("home/.config/cardputerzero");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.json"),
            "{\"gh0st\": {\"lock\": {\"password\": false}}}",
        )
        .unwrap();
        for d in ["frames", "ours", "stock", "icons", "run"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let exe = dir.join("rooty");
        std::fs::write(&exe, APP).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_gh0st"), dir.join("gh0st-askpass")).unwrap();
        let chk = dir.join("chkpwd");
        std::fs::write(&chk, CHKPWD).unwrap();
        std::fs::set_permissions(&chk, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            dir.join("ours/rooty.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Rooty\nExec={} {exec_tail}\n",
                exe.display()
            ),
        )
        .unwrap();
        Sim { dir }
    }

    /// Runs the shell over `keys`; returns what the helper printed, the
    /// app's log and the shell's.
    fn run(&self, keys: &str, codes: &str) -> (String, String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["shell", "--sim"])
            .arg(self.dir.join("frames"))
            .env("HOME", self.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.dir.join("home/.config"))
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env("GH0ST_APPS_OURS", self.dir.join("ours"))
            .env("GH0ST_APPS_STOCK", self.dir.join("stock"))
            .env("GH0ST_ICON_ROOT", self.dir.join("icons"))
            .env("GH0ST_CHKPWD", self.dir.join("chkpwd"))
            .env("GH0ST_ASKPASS_SIM_APP", "rooty")
            .env("GH0ST_LOG", "debug")
            .env("SIM_DIR", &self.dir)
            .env("CODES", codes)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        {
            use std::io::Write;
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(keys.as_bytes()).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        let read = |f: &str| std::fs::read_to_string(self.dir.join(f)).unwrap_or_default();
        (
            read("got"),
            read("log"),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Open the app; the prompt comes up over it.
const OPEN: &str = "wait 600\nenter\nwait 1500\n";

#[test]
fn the_right_password_reaches_sudo() {
    let s = Sim::new("right", "sudo");
    let (got, log, shell) = s.run(&format!("{OPEN}type 1234\nenter\nwait 2000\n"), "0");
    assert_eq!(got, "1234\n", "{shell}");
    assert!(log.contains("askpass exit 0"), "{log}");
    assert!(shell.contains("askpass: answered"), "{shell}");
}

#[test]
fn esc_answers_no_and_sudo_hears_it() {
    let s = Sim::new("esc", "sudo");
    let (got, log, shell) = s.run(&format!("{OPEN}esc\nwait 1500\n"), "0");
    assert_eq!(got, "", "{shell}");
    assert!(log.contains("askpass exit 1"), "{log}");
    assert!(shell.contains("askpass: refused"), "{shell}");
}

#[test]
fn a_wrong_password_asks_again_and_never_reaches_sudo() {
    let s = Sim::new("wrong", "sudo");
    let keys = format!("{OPEN}type nope\nenter\nwait 800\ntype 1234\nenter\nwait 2000\n");
    let (got, log, shell) = s.run(&keys, "7 0");
    assert_eq!(got, "1234\n", "{shell}");
    assert!(log.contains("askpass exit 0"), "{log}");
}

#[test]
fn an_app_that_doesnt_run_as_root_is_refused_without_a_prompt() {
    let s = Sim::new("plain", "");
    let (got, log, shell) = s.run(&format!("{OPEN}wait 500\n"), "0");
    assert_eq!(got, "");
    assert!(log.contains("askpass exit 1"), "{log}");
    assert!(shell.contains("doesn't run as root"), "{shell}");
}
