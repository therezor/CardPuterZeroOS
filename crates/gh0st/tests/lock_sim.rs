//! The lock screen end to end: the real shell under `--sim`, keys from a
//! script, and fake `passwd -S` and `unix_chkpwd` that log each call and
//! answer as told. Nothing on the host changes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Answers the nth call with the nth code in `$CODES`, then 0.
const CHKPWD: &str = r#"#!/bin/sh
/bin/cat >/dev/null
n=$(/bin/cat "$SIM_DIR/n" 2>/dev/null || echo 0)
echo $((n + 1)) > "$SIM_DIR/n"
echo "chkpwd $*" >> "$SIM_DIR/log"
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
