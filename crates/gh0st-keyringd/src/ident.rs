//! The keyring's callers: a bus name plus who the process behind it is.

use std::os::fd::OwnedFd;

use crate::secret::Caller;

pub fn caller(bus: &str, pid: Option<u32>, pidfd: Option<&OwnedFd>) -> Caller {
    match pid.and_then(|p| gh0st_core::ident::of_pid(p, pidfd)) {
        Some(w) => Caller {
            bus: bus.into(),
            was: pid.and_then(|p| was(&w.app, p)),
            app: w.app,
            label: w.label,
        },
        None => Caller {
            bus: bus.into(),
            app: format!("bus:{bus}"),
            label: "An unknown program".into(),
            was: None,
        },
    }
}

/// What one of our own apps was called before apps had cgroups: its
/// program's path. Only ours: every Python app shared one path then.
fn was(app: &str, pid: u32) -> Option<String> {
    if app.starts_with("exe:") {
        return None;
    }
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let exe = exe.to_str()?;
    exe.starts_with("/usr/bin/gh0st-")
        .then(|| format!("exe:{exe}"))
}
