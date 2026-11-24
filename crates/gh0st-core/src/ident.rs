//! Who a process is. The shell runs each app in a cgroup named
//! `app-<desktop stem>` right under its own; an isolated app can't leave
//! it, so that name is the identity. Anything else is named by its
//! executable.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;

/// The shell's unit: it makes `app-<stem>` right under its own cgroup.
const SHELL_UNIT: &str = "gh0st.service";

/// The app from a `/proc/<pid>/cgroup` body: the `app-<stem>` right under
/// the shell's cgroup. A process can make cgroups under its own, and
/// `systemd-run` can name a scope `app-*`, so nothing else counts.
pub fn app_of_cgroup(body: &str) -> Option<String> {
    let path = body.lines().find_map(|l| l.strip_prefix("0::"))?;
    let segs: Vec<&str> = path.split('/').collect();
    let at = segs.iter().position(|s| *s == SHELL_UNIT)?;
    let stem = segs.get(at + 1)?.strip_prefix("app-")?;
    (!stem.is_empty()).then(|| stem.to_string())
}

/// The app's `Name=` from its desktop file, for prompts.
pub fn app_label(stem: &str) -> String {
    let dirs = crate::apps::Dirs::from_env();
    for dir in [&dirs.ours, &dirs.stock] {
        let p = dir.join(format!("{stem}.desktop"));
        if let Some(e) = std::fs::read_to_string(p)
            .ok()
            .and_then(|s| crate::desktop::parse(&s))
        {
            return e.name;
        }
    }
    stem.to_string()
}

fn alive(pidfd: &OwnedFd) -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: pidfd_send_signal with signal 0 only checks the process.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            ) == 0
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pidfd.as_raw_fd();
        true
    }
}

/// A process as services see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    /// The app's desktop stem, or `exe:<path>` for a process outside apps.
    pub app: String,
    /// What to call it in a prompt: its `Name=`, or the program's name.
    pub label: String,
}

impl Who {
    /// A name fit for a notice file: lowercase letters, digits, - and _.
    pub fn short(&self) -> String {
        let base = self.app.rsplit('/').next().unwrap_or(&self.app);
        let s: String = base
            .chars()
            .map(|c| match c.to_ascii_lowercase() {
                c @ ('a'..='z' | '0'..='9' | '-' | '_') => c,
                _ => '-',
            })
            .take(40)
            .collect();
        if s.is_empty() { "app".into() } else { s }
    }
}

/// Who process `pid` is. None when it can't be read, or when the pidfd
/// says the pid now belongs to someone else.
pub fn of_pid(pid: u32, pidfd: Option<&OwnedFd>) -> Option<Who> {
    let proc_dir = Path::new("/proc").join(pid.to_string());
    let cgroup = std::fs::read_to_string(proc_dir.join("cgroup")).unwrap_or_default();
    let exe = std::fs::read_link(proc_dir.join("exe")).ok();
    // The pid could have been reused while we read: the pidfd can't.
    if pidfd.is_some_and(|fd| !alive(fd)) {
        return None;
    }
    if let Some(stem) = app_of_cgroup(&cgroup) {
        return Some(Who {
            label: app_label(&stem),
            app: stem,
        });
    }
    let exe = exe?;
    Some(Who {
        label: exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        app: format!("exe:{}", exe.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_cgroup_names_the_app() {
        let body = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/gh0st.service/app-chromium\n";
        assert_eq!(app_of_cgroup(body).as_deref(), Some("chromium"));
        assert_eq!(
            app_of_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
            None
        );
        assert_eq!(
            app_of_cgroup("0::/x/gh0st.service/app-term/app-sneaky\n").as_deref(),
            Some("term"),
            "a cgroup made inside an app is still that app"
        );
        assert_eq!(app_of_cgroup("12:cpu:/x\n"), None);
        assert_eq!(
            app_of_cgroup("0::/user.slice/user@1000.service/app.slice/app-chromium.scope\n"),
            None,
            "a systemd-run scope is not the shell's"
        );
    }

    #[test]
    fn short_names_fit_a_notice_file() {
        let w = |app: &str| Who {
            app: app.into(),
            label: String::new(),
        };
        assert_eq!(w("exe:/usr/bin/Notify.Send").short(), "notify-send");
        assert_eq!(w("chromium").short(), "chromium");
        assert_eq!(w("exe:/").short(), "app");
    }
}
