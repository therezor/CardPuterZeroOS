//! Root grants (docs/isolation.md, Root apps): apps the user let use sudo
//! without the password, as on stock firmware. The list is root's; only
//! `gh0st-helper` changes it, behind the password, and the Store's engine
//! drops an app's line when it removes the app.
//!
//! A grant counts for a sudo whose process sits in `app-<stem>` right under
//! the shell's unit, when that cgroup is root's: root's helper made it and
//! put the app in it, through the trampoline (`gh0st root-app`), which
//! runs only the app's own `Exec`, with a clean environment. No process of
//! the user can join a root-owned cgroup or make one under it.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// One app's desktop file stem a line; `#` starts a comment.
pub const LIST: &str = "/etc/gh0st/root-apps";
/// What sudo's PAM stack runs before the password (`pam_exec`).
pub const CHECK: &str = "/usr/libexec/gh0st-root-check";
/// The gh0st program the shell starts a granted app through.
pub const TRAMPOLINE: &str = "root-app";
/// The trampoline's go: the shell writes a byte to this fd once root's
/// helper placed it, or failed to.
pub const GO_ENV: &str = "GH0ST_ROOT_GO";
/// What a granted app's PATH holds: root's folders, after sudo's shim.
pub const SAFE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// The folders desktop files are read from, never overridden here.
pub const DESKTOP_DIRS: [(&str, bool); 2] = [
    ("/usr/share/gh0st/applications", true),
    ("/usr/share/APPLaunch/applications", false),
];

/// A stem as desktop files are named: no path, no space, no newline.
pub fn is_stem(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

pub fn parse(text: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if is_stem(l) && !v.iter().any(|x| x == l) {
            v.push(l.to_string());
        }
    }
    v
}

pub fn text(stems: &[String]) -> String {
    let mut s = String::from("# Apps allowed root without a password: Settings > Apps > Root.\n");
    for st in stems.iter().filter(|s| is_stem(s)) {
        s.push_str(st);
        s.push('\n');
    }
    s
}

/// Root's, and writable by root alone: what a grant may rest on. Group
/// write counts only for group root, which M5's debs give their files.
pub fn root_only(m: &std::fs::Metadata) -> bool {
    root_only_ids(m.uid(), m.gid(), m.mode())
}

fn root_only_ids(uid: u32, gid: u32, mode: u32) -> bool {
    uid == 0 && mode & 0o002 == 0 && (mode & 0o020 == 0 || gid == 0)
}

fn root_only_path(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink() && root_only(&m))
}

/// The grants at `path`: none unless the file and its folder are root's
/// alone.
pub fn load(path: &Path) -> Vec<String> {
    let dir_ok = path.parent().is_some_and(root_only_path);
    if !dir_ok || !root_only_path(path) {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn granted(stem: &str) -> bool {
    load(Path::new(LIST)).iter().any(|s| s == stem)
}

/// The cgroup path of a `/proc/<pid>/cgroup` body.
pub fn cgroup_path(body: &str) -> Option<&str> {
    body.lines().find_map(|l| l.strip_prefix("0::"))
}

/// The granted app a process with cgroup `body` runs for, with the cgroup
/// tree mounted at `root`: `app-<stem>` right under the shell's unit, that
/// cgroup and its `cgroup.procs` root's alone, and `stem` in `grants`.
pub fn grant_of(body: &str, root: &Path, grants: &[String]) -> Option<String> {
    let path = cgroup_path(body)?;
    let stem = crate::ident::app_of_cgroup(body)?;
    if !path.ends_with(&format!("/app-{stem}")) || !grants.contains(&stem) {
        return None;
    }
    let dir = root.join(path.trim_start_matches('/'));
    (root_only_path(&dir) && root_only_path(&dir.join("cgroup.procs"))).then_some(stem)
}

/// Whether the trampoline keeps an environment variable. A list of what
/// apps need, not of what is dangerous: anything that tells a loader or
/// an interpreter what to run (`LD_*`, `PYTHON*`, `BASH_ENV`...) is out.
pub fn keep_env(name: &str) -> bool {
    matches!(
        name,
        "HOME"
            | "USER"
            | "LOGNAME"
            | "LANG"
            | "LANGUAGE"
            | "TZ"
            | "TERM"
            | "XDG_RUNTIME_DIR"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "PULSE_PROP"
            | "GH0ST_SHELL_FD"
            | "GH0ST_SYSPLAUSE"
            | "CAMERA_APP_KEYBOARD_GRAB"
    ) || name.starts_with("LC_")
        || name.starts_with("LV_LINUX_")
        || name.starts_with("APPLAUNCH_LINUX_")
}

/// App `stem`'s desktop file from the fixed folders, and whether it is
/// ours. Only a file root alone can write, in a folder root alone can
/// write.
pub fn desktop_of(stem: &str) -> Option<(PathBuf, bool)> {
    if !is_stem(stem) {
        return None;
    }
    DESKTOP_DIRS.iter().find_map(|(dir, ours)| {
        let d = Path::new(dir);
        let f = d.join(format!("{stem}.desktop"));
        (root_only_path(d) && root_only_path(&f)).then_some((f, *ours))
    })
}

/// Why app `stem` can't be granted: none when it can. Its `Exec` must run
/// what root alone can change, and not in a terminal, which the user
/// picks.
pub fn refuse(stem: &str) -> Option<String> {
    let Some((path, _)) = desktop_of(stem) else {
        return Some(format!("{stem} has no desktop file only root can change"));
    };
    let Some(entry) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| crate::desktop::parse(&t))
    else {
        return Some(format!("{stem}'s desktop file doesn't read"));
    };
    if entry.terminal {
        return Some(format!("{} runs in a terminal", entry.name));
    }
    for word in crate::desktop::split_exec(&entry.exec) {
        let p = Path::new(&word);
        if p.is_absolute() && p.exists() && !path_root_only(p) {
            return Some(format!(
                "{} runs {word}, which others can change",
                entry.name
            ));
        }
    }
    None
}

/// `p` and every folder above it are root's alone.
pub fn path_root_only(p: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(p) else {
        return false;
    };
    real.ancestors()
        .all(|a| std::fs::metadata(a).is_ok_and(|m| root_only(&m)) || a.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_alone_writes_it() {
        assert!(root_only_ids(0, 0, 0o644));
        assert!(root_only_ids(0, 0, 0o664), "group root, as M5's debs");
        assert!(!root_only_ids(0, 27, 0o664), "another group writes");
        assert!(!root_only_ids(0, 0, 0o666), "anyone writes");
        assert!(!root_only_ids(1000, 1000, 0o644), "the user's");
    }

    #[test]
    fn the_list_reads_stems_only() {
        let t = "# c\nraspyjack\n../etc/passwd\nwith space\nclawputer\nraspyjack\n.hidden\n";
        assert_eq!(parse(t), ["raspyjack", "clawputer"]);
        assert_eq!(parse(&text(&["a".into(), "b c".into()])), ["a"]);
    }

    #[test]
    fn the_environment_keeps_what_apps_need_and_nothing_that_loads_code() {
        for k in [
            "HOME",
            "LANG",
            "LC_ALL",
            "LV_LINUX_FBDEV_DEVICE",
            "GH0ST_SHELL_FD",
            "XDG_RUNTIME_DIR",
        ] {
            assert!(keep_env(k), "{k}");
        }
        for k in [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "PYTHONPATH",
            "PYTHONSTARTUP",
            "PYTHONHOME",
            "BASH_ENV",
            "ENV",
            "PERL5LIB",
            "PERL5OPT",
            "NODE_OPTIONS",
            "RUBYOPT",
            "GCONV_PATH",
            "PATH",
            "SUDO_ASKPASS",
            "XDG_DATA_DIRS",
            "GIO_MODULE_DIR",
            "GTK_MODULES",
            "IFS",
        ] {
            assert!(!keep_env(k), "{k}");
        }
    }

    /// A cgroup tree under `root` with app `stem`, owned as given.
    fn tree(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("gh0st-rootapps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    const BODY: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/gh0st.service/app-raspyjack\n";

    #[test]
    fn a_grant_needs_a_root_cgroup_right_under_the_shell() {
        let root = tree("cg");
        let dir = root.join(
            "user.slice/user-1000.slice/user@1000.service/app.slice/gh0st.service/app-raspyjack",
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cgroup.procs"), "").unwrap();
        let grants = vec!["raspyjack".to_string()];
        // Run as a user, these files are the user's: the user made the
        // cgroup, so it grants nothing.
        // SAFETY: getuid has no failure mode.
        let root_runs = unsafe { libc::getuid() } == 0;
        assert_eq!(grant_of(BODY, &root, &grants).is_some(), root_runs);
        assert_eq!(grant_of(BODY, &root, &[]), None, "not granted");
        let deeper = BODY.replace("app-raspyjack", "app-raspyjack/x");
        assert_eq!(
            grant_of(&deeper, &root, &grants),
            None,
            "a cgroup made inside"
        );
        let other = BODY.replace("gh0st.service", "run-x.service");
        assert_eq!(
            grant_of(&other, &root, &grants),
            None,
            "not under the shell"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_list_anyone_can_write_holds_nothing() {
        let root = tree("list");
        std::fs::create_dir_all(&root).unwrap();
        let p = root.join("root-apps");
        std::fs::write(&p, "raspyjack\n").unwrap();
        // SAFETY: getuid has no failure mode.
        if unsafe { libc::getuid() } != 0 {
            assert!(load(&p).is_empty(), "the user's own file");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
