//! Root grants (docs/isolation.md, Root apps), the parts that run as or
//! for root:
//!
//! - `gh0st-root-check`: sudo's PAM stack runs it before the password.
//! - `gh0st root-app STEM`: the trampoline the shell starts a granted app
//!   through. It waits for root's helper to place it, then runs only the
//!   app's own `Exec`, clean.
//! - [`place`] and [`grant`]: the helper's `root-place` and `root-grant`.

use gh0st_core::{askpass, desktop, ident, rootapps};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

// ------------------------------------------------------------ the PAM check

/// `pam_exec seteuid`: exit 0 lets sudo through without the password. The
/// caller is sudo itself, so its cgroup is the app's. Nothing here comes
/// from the user: not `PAM_USER`, not the environment past which service
/// asks.
pub fn check_main() -> i32 {
    let var = |k: &str| std::env::var(k).ok();
    if var("PAM_SERVICE").as_deref() != Some("sudo") || var("PAM_TYPE").as_deref() != Some("auth") {
        return 1;
    }
    // SAFETY: geteuid and getppid have no failure mode.
    let (euid, sudo) = unsafe { (libc::geteuid(), libc::getppid()) };
    if euid != 0 {
        return 1;
    }
    let read = |f: &str| std::fs::read_to_string(format!("/proc/{sudo}/{f}")).unwrap_or_default();
    if !is_root_sudo(&read("status")) {
        return 1;
    }
    let grants = rootapps::load(Path::new(rootapps::LIST));
    match rootapps::grant_of(&read("cgroup"), Path::new(CGROUP_ROOT), &grants) {
        Some(_) => 0,
        None => 1,
    }
}

/// A sudo running with root's effective uid, as the real one is.
fn is_root_sudo(status: &str) -> bool {
    let name = status
        .lines()
        .find_map(|l| l.strip_prefix("Name:"))
        .map(str::trim);
    let euid = status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|v| v.split_whitespace().nth(1));
    name == Some("sudo") && euid == Some("0")
}

// ------------------------------------------------------------ the trampoline

/// Nothing traces this process: read from `/proc/self/status`.
fn untraced() -> bool {
    !cfg!(target_os = "linux")
        || std::fs::read_to_string("/proc/self/status").is_ok_and(|s| askpass::untraced(&s))
}

pub fn trampoline_main(args: &[String]) -> i32 {
    let [stem] = args else {
        eprintln!("usage: gh0st root-app STEM");
        return 2;
    };
    // No other process of the user may change this one while root's
    // helper looks at it, and none may have attached before.
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with these arguments only clears a flag of this process.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    if !untraced() {
        eprintln!("gh0st root-app: traced");
        return 1;
    }
    if let Some(fd) = std::env::var(rootapps::GO_ENV)
        .ok()
        .and_then(|v| v.parse::<RawFd>().ok())
        .filter(|&fd| fd > 2)
    {
        // SAFETY: the shell passed this fd for this read alone.
        let go = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut b = [0u8; 1];
        // Any byte or the end: the shell is done with root's helper.
        // SAFETY: read into a live one-byte buffer.
        let _ = unsafe { libc::read(fd, b.as_mut_ptr().cast(), 1) };
        drop(go);
    }
    if !untraced() {
        eprintln!("gh0st root-app: traced");
        return 1;
    }
    match command(stem) {
        Ok(mut cmd) => {
            let e = cmd.exec();
            eprintln!("gh0st root-app: {stem}: {e}");
            1
        }
        Err(e) => {
            eprintln!("gh0st root-app: {stem}: {e}");
            1
        }
    }
}

/// App `stem`'s `Exec`, read from its root-owned desktop file, with the
/// environment, folder and files the trampoline sets.
fn command(stem: &str) -> Result<Command, String> {
    let (path, ours) = rootapps::desktop_of(stem).ok_or("no desktop file only root can change")?;
    let entry = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| desktop::parse(&t))
        .ok_or("its desktop file doesn't read")?;
    if entry.terminal {
        return Err("it runs in a terminal".into());
    }
    let argv: Vec<std::ffi::OsString> = if ours {
        desktop::expand_exec(&entry.exec, &[])
    } else {
        vec![
            "/bin/sh".into(),
            "-c".into(),
            desktop::shell_line(&entry.exec).into(),
        ]
    };
    let (prog, rest) = argv.split_first().ok_or("an empty Exec")?;
    let mut cmd = Command::new(prog);
    cmd.args(rest).env_clear();
    for (k, v) in std::env::vars_os() {
        if k.to_str().is_some_and(rootapps::keep_env) {
            cmd.env(k, v);
        }
    }
    cmd.env(
        "PATH",
        format!("{}:{}", askpass::SHIM_DIR, rootapps::SAFE_PATH),
    )
    .env("SUDO_ASKPASS", askpass::HELPER)
    .current_dir(gh0st_core::child::APP_CWD)
    .stdin(Stdio::null());
    let link: RawFd = std::env::var("GH0ST_SHELL_FD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-1);
    // SAFETY: close is async-signal-safe; it runs between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            for fd in 3..1024 {
                if fd != link {
                    libc::close(fd);
                }
            }
            Ok(())
        });
    }
    Ok(cmd)
}

// ------------------------------------------------------------ the helper

/// `gh0st-helper root-place STEM PID`: puts the shell's trampoline for
/// granted app `stem` into a root-owned `app-<stem>`. No password: what it
/// places can only run the app's own `Exec`, so asking gains nobody more
/// than opening the app. It checks:
///
/// - `stem` is granted;
/// - `pid` runs this very binary as `root-app STEM`, for `caller`, and
///   nothing traces it;
/// - after the move, the same process (by pidfd) is the one in there.
pub fn place(stem: &str, pid: u32, caller: u32) -> Result<(), String> {
    if !rootapps::is_stem(stem) || !rootapps::granted(stem) {
        return Err(format!("{stem} isn't allowed root"));
    }
    let pidfd = ident::pidfd_open(pid).ok_or("no such process")?;
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let ours = || -> Result<String, String> {
        let me = std::fs::metadata("/proc/self/exe").map_err(|e| e.to_string())?;
        let it = std::fs::metadata(proc.join("exe")).map_err(|e| e.to_string())?;
        if (me.dev(), me.ino()) != (it.dev(), it.ino()) {
            return Err("not the gh0st binary".into());
        }
        let cmdline = std::fs::read(proc.join("cmdline")).map_err(|e| e.to_string())?;
        let words: Vec<&[u8]> = cmdline
            .split(|&b| b == 0)
            .filter(|w| !w.is_empty())
            .collect();
        if words.len() != 3
            || words[1] != rootapps::TRAMPOLINE.as_bytes()
            || words[2] != stem.as_bytes()
        {
            return Err("not the trampoline for this app".into());
        }
        let status = std::fs::read_to_string(proc.join("status")).map_err(|e| e.to_string())?;
        let ruid = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u32>().ok());
        if ruid != Some(caller) || !askpass::untraced(&status) {
            return Err("not the caller's, or traced".into());
        }
        std::fs::read_to_string(proc.join("cgroup")).map_err(|e| e.to_string())
    };
    let body = ours()?;
    let from = rootapps::cgroup_path(&body).ok_or("no cgroup")?;
    let to = format!("{}/app-{stem}", from.trim_end_matches('/'));
    if ident::app_of_cgroup(&format!("0::{to}")).as_deref() != Some(stem) {
        return Err("not started by the shell".into());
    }
    let dir = Path::new(CGROUP_ROOT).join(to.trim_start_matches('/'));
    root_cgroup(&dir)?;
    std::fs::write(dir.join("cgroup.procs"), pid.to_string()).map_err(|e| format!("move: {e}"))?;
    // A pidfd that is still alive means the number wasn't reused: what was
    // checked is what moved.
    let still = ident::alive(&pidfd)
        && ours().is_ok_and(|b| rootapps::cgroup_path(&b) == Some(to.as_str()));
    if !still {
        let _ = std::fs::write(dir.join("cgroup.kill"), "1");
        return Err("the process changed while it moved".into());
    }
    Ok(())
}

/// `dir` as a cgroup root alone owns, made if missing. One the user made
/// goes first, when empty; one in use is refused.
fn root_cgroup(dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if rootapps::root_only(&m) => return Ok(()),
        Ok(_) => {
            let procs = std::fs::read_to_string(dir.join("cgroup.procs")).unwrap_or_default();
            if !procs.trim().is_empty() {
                return Err("its cgroup is someone else's, and in use".into());
            }
            std::fs::remove_dir(dir).map_err(|e| format!("rmdir: {e}"))?;
        }
        Err(_) => {}
    }
    std::fs::create_dir(dir).map_err(|e| format!("mkdir: {e}"))?;
    let m = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if rootapps::root_only(&m) {
        Ok(())
    } else {
        Err("the new cgroup isn't root's".into())
    }
}

/// `gh0st-helper root-grant STEM on|off`, behind the password. On needs an
/// app that asks for root, run by files only root can change.
pub fn grant(stem: &str, on: bool) -> Result<(), String> {
    if on {
        if let Some(why) = rootapps::refuse(stem) {
            return Err(why);
        }
        let exec = rootapps::desktop_of(stem)
            .and_then(|(p, _)| std::fs::read_to_string(p).ok())
            .and_then(|t| desktop::parse(&t))
            .map(|e| e.exec)
            .unwrap_or_default();
        if !gh0st_core::store::needs_root(stem) && !gh0st_core::apps::runs_as_root(&exec) {
            return Err(format!("{stem} doesn't ask for root"));
        }
    }
    set(Path::new(rootapps::LIST), stem, on)
}

/// Adds or drops `stem` in the list at `path`, as root.
pub fn set(path: &Path, stem: &str, on: bool) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = path.parent().ok_or("no folder")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut list = rootapps::load(path);
    list.retain(|s| s != stem);
    if on {
        list.push(stem.to_string());
    }
    let tmp = path.with_extension("new");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::io::Write::write_all(&mut f, rootapps::text(&list).as_bytes())
        .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_root_sudo_counts() {
        assert!(is_root_sudo("Name:\tsudo\nUid:\t1000\t0\t0\t0\n"));
        assert!(!is_root_sudo("Name:\tsudo\nUid:\t1000\t1000\t1000\t1000\n"));
        assert!(!is_root_sudo("Name:\tbash\nUid:\t0\t0\t0\t0\n"));
    }

    #[test]
    fn the_list_changes_one_line_at_a_time() {
        let dir = std::env::temp_dir().join(format!("gh0st-root-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("root-apps");
        set(&p, "raspyjack", true).unwrap();
        let read = || rootapps::parse(&std::fs::read_to_string(&p).unwrap());
        assert_eq!(read(), ["raspyjack"]);
        // The test runs as a user: load() trusts no file of theirs, so the
        // list starts over each time. As root it accumulates.
        set(&p, "raspyjack", false).unwrap();
        assert!(read().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
