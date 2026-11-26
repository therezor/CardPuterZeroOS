//! The lock screen end to end: the real shell under `--sim`, keys from a
//! script, and fake `passwd -S` and `unix_chkpwd` that log each call and
//! answer as told. Nothing on the host changes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Answers the nth call with the nth code in `$CODES`, then 0, after the
/// nth wait in `$SLEEPS` seconds.
const CHKPWD: &str = r#"#!/bin/sh
/bin/cat >/dev/null
n=$(/bin/cat "$SIM_DIR/n" 2>/dev/null || echo 0)
echo $((n + 1)) > "$SIM_DIR/n"
echo "chkpwd $*" >> "$SIM_DIR/log"
s=$(echo "$SLEEPS" | /usr/bin/cut -d' ' -f$((n + 1)))
[ -n "$s" ] && /bin/sleep "$s"
code=$(echo "$CODES" | /usr/bin/cut -d' ' -f$((n + 1)))
exit ${code:-0}
"#;

/// `passwd -S` for an account with a password, or nothing at all.
const PASSWD: &str = r#"#!/bin/sh
echo "passwd $*" >> "$SIM_DIR/log"
[ -n "$PASSWD_FAILS" ] && exit 1
echo "me P 2026-11-23 0 99999 7 -1"
"#;

/// Tab then Enter opens the field; a password and Enter check it.
const UNLOCK: &str = "wait 300\ntab\nenter\nwait 100\ntype hunter22\nenter\nwait 600\n";
/// Tab held locks again.
const RELOCK: &str = "down tab\nwait 3200\nup tab\nwait 400\n";

struct Run {
    log: String,
    stderr: String,
}

impl Run {
    fn checks(&self) -> usize {
        self.log.lines().filter(|l| l.starts_with("chkpwd")).count()
    }

    fn unlocks(&self) -> usize {
        self.stderr.matches("gh0st shell: unlocked").count()
    }
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn run(name: &str, keys: &str, codes: &str, passwd_fails: bool) -> Run {
    run_slow(name, keys, codes, "", passwd_fails)
}

fn run_slow(name: &str, keys: &str, codes: &str, sleeps: &str, passwd_fails: bool) -> Run {
    let dir = std::env::temp_dir().join(format!("gh0st-lock-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    let config = home.join(".config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        "{\"gh0st\": {\"lock\": {\"password\": true}}}",
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let chkpwd = script(&dir, "chkpwd", CHKPWD);
    let passwd = script(&dir, "passwd", PASSWD);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gh0st"));
    cmd.args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", &home)
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_CHKPWD", &chkpwd)
        .env("GH0ST_PASSWD", &passwd)
        .env("GH0ST_LOG", "debug")
        .env("SIM_DIR", &dir)
        .env("CODES", codes)
        .env("SLEEPS", sleeps)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if passwd_fails {
        cmd.env("PASSWD_FAILS", "1");
    }
    let mut child = cmd.spawn().unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(keys.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let log = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    Run { log, stderr }
}

#[test]
fn the_right_password_unlocks() {
    let r = run("right", UNLOCK, "0", false);
    assert_eq!(r.checks(), 1, "{}", r.log);
    assert_eq!(r.unlocks(), 1, "{}", r.stderr);
}

#[test]
fn a_wrong_password_stays_locked() {
    let r = run("wrong", UNLOCK, "7", false);
    assert_eq!(r.checks(), 1, "{}", r.log);
    assert_eq!(r.unlocks(), 0, "{}", r.stderr);
}

#[test]
fn a_broken_check_runs_again() {
    let r = run("retry", UNLOCK, "9 7", false);
    assert_eq!(r.checks(), 2, "{}", r.log);
    assert_eq!(r.unlocks(), 0, "the second check said wrong: {}", r.stderr);
}

#[test]
fn a_checker_broken_twice_opens_this_lock_only() {
    let keys = format!("{UNLOCK}{RELOCK}{UNLOCK}");
    let r = run("broken", &keys, "9 9 7", false);
    assert_eq!(r.checks(), 3, "the next lock asks again: {}", r.log);
    assert_eq!(r.unlocks(), 1, "{}", r.stderr);
}

#[test]
fn a_failed_account_probe_is_asked_again() {
    let r = run("probe", "wait 600\n", "", true);
    let probes = r.log.lines().filter(|l| l.starts_with("passwd")).count();
    assert_eq!(probes, 2, "{}", r.log);
}

/// Esc leaves a slow check; a second one starts. The first answers right
/// while the second still runs: that answer is old and opens nothing.
#[test]
fn a_late_answer_from_a_left_check_opens_nothing() {
    let keys = "wait 300\ntab\nenter\nwait 100\ntype hunter22\nenter\nwait 100\nesc\n\
                tab\nenter\nwait 100\ntype wrong\nenter\nwait 4000\n";
    let r = run_slow("late", keys, "0 7", "2 3", false);
    assert_eq!(r.checks(), 2, "{}", r.log);
    assert_eq!(r.unlocks(), 0, "{}", r.stderr);
}

/// A linked app that answers `pause` and logs `resume`.
const APP: &str = r#"#!/usr/bin/env python3
import os, select, socket
d = os.environ["SIM_DIR"]
s = socket.socket(fileno=int(os.environ["GH0ST_SHELL_FD"]))
s.send(b"hello\t2")
while True:
    select.select([s], [], [])
    m = s.recv(8192)
    if not m:
        break
    f = m.decode().split("\t")
    if f[0] == "pause":
        s.send(b"paused")
    with open(d + "/log", "a") as out:
        out.write("got " + f[0] + "\n")
"#;

/// Enter in the switcher waits for a held key to come up. 6 locks first:
/// the app must not come back over the lock when the key comes up, only
/// after the unlock.
#[test]
fn a_resume_waiting_for_a_key_does_not_pass_the_lock() {
    let dir = std::env::temp_dir().join(format!("gh0st-lock-pending-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        "{\"gh0st\": {\"lock\": {\"password\": false}}}",
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let exe = script(&dir, "a", APP);
    std::fs::write(
        dir.join("ours/a.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Alpha\nExec={}\n",
            exe.display()
        ),
    )
    .unwrap();
    // Alpha up; Fn+K parks it and opens the switcher on it. Q held, Enter
    // tapped, 6 locks, Z up. Then Tab and Enter unlock.
    let keys = "wait 600\nenter\nwait 800\nhome\nwait 400\ndown q\nenter\nwait 100\n6\n\
                wait 100\nup q\nwait 600\ntab\nenter\nwait 800\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
        .args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", dir.join("home"))
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_LOG", "debug")
        .env("SIM_DIR", &dir)
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
    let shell = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    let paused = shell.find("paused Alpha").expect(&shell);
    let unlocked = shell.find("gh0st shell: unlocked").expect(&shell);
    assert_eq!(shell.matches("resumed Alpha").count(), 1, "{shell}");
    let resumed = shell.find("resumed Alpha").unwrap_or(0);
    assert!(paused < unlocked && unlocked < resumed, "{shell}");
}

/// Esc held over an app: the hint goes to the panel driver once and stays
/// there; the shell neither repaints it nor sends it again.
#[test]
fn the_esc_hint_over_an_app_is_held_by_the_driver() {
    let dir = std::env::temp_dir().join(format!("gh0st-lock-esc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        "{\"gh0st\": {\"lock\": {\"password\": false}}}",
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let exe = script(&dir, "a", APP);
    std::fs::write(
        dir.join("ours/a.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Alpha\nExec={}\n",
            exe.display()
        ),
    )
    .unwrap();
    // Alpha up; Esc held 2.5 s shows the hint at 1.5 s and lets go before
    // the app is told to close.
    let keys = "wait 600\nenter\nwait 800\ndown esc\nwait 2500\nup esc\nwait 400\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
        .args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", dir.join("home"))
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_LOG", "debug")
        .env("SIM_DIR", &dir)
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
    let shell = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        shell.matches("banner held by the driver").count(),
        1,
        "{shell}"
    );
    assert!(!shell.contains("banner drawn"), "{shell}");
    assert!(!shell.contains("Alpha closed"), "{shell}");
}

/// The shell with Alpha, a linked app, and no lock password; keys from
/// `keys`. Returns what the shell logged.
fn with_app(name: &str, keys: &str) -> String {
    let dir = std::env::temp_dir().join(format!("gh0st-lock-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        "{\"gh0st\": {\"lock\": {\"password\": false}}}",
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let exe = script(&dir, "a", APP);
    std::fs::write(
        dir.join("ours/a.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Alpha\nExec={}\n",
            exe.display()
        ),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
        .args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", dir.join("home"))
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_LOG", "debug")
        .env("SIM_DIR", &dir)
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
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Tab held 3 s over an app locks once Tab is up; the unlock brings the
/// app back.
#[test]
fn tab_held_over_an_app_locks_and_the_unlock_brings_it_back() {
    let keys = "wait 600\nenter\nwait 800\ndown tab\nwait 3200\nup tab\nwait 600\n\
                tab\nenter\nwait 800\n";
    let shell = with_app("tab-app", keys);
    let hint = shell.find("banner held by the driver").expect(&shell);
    let paused = shell.find("paused Alpha").expect(&shell);
    let locked = shell.find("gh0st shell: locked").expect(&shell);
    let unlocked = shell.find("gh0st shell: unlocked").expect(&shell);
    let resumed = shell.find("resumed Alpha").expect(&shell);
    assert!(hint < paused && paused < locked, "{shell}");
    assert!(locked < unlocked && unlocked < resumed, "{shell}");
}

/// Another key while Tab is held: the app is typing, not locking.
#[test]
fn tab_with_another_key_over_an_app_never_locks() {
    let keys = "wait 600\nenter\nwait 800\ndown tab\nwait 1000\ndown q\nwait 2500\n\
                up q\nup tab\nwait 600\n";
    let shell = with_app("tab-spoiled", keys);
    assert!(!shell.contains("gh0st shell: locked"), "{shell}");
    assert!(!shell.contains("paused Alpha"), "{shell}");
}

/// Tab down on home, Enter opens an app, Tab comes up over it: home's hold
/// never saw the release, and must not lock once home is back.
#[test]
fn a_tab_let_go_over_an_app_leaves_no_lock_behind() {
    let keys = "wait 600\ndown tab\nenter\nwait 500\nup tab\nwait 3200\nhome\nwait 800\n";
    let shell = with_app("tab-stale", keys);
    assert!(shell.contains("paused Alpha"), "{shell}");
    assert!(!shell.contains("gh0st shell: locked"), "{shell}");
}
