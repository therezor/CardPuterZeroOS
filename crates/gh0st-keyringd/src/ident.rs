//! The keyring's callers: a bus name plus who the process behind it is.

use std::os::fd::OwnedFd;

use crate::secret::Caller;

pub fn caller(bus: &str, pid: Option<u32>, pidfd: Option<&OwnedFd>) -> Caller {
    match pid.and_then(|p| gh0st_core::ident::of_pid(p, pidfd)) {
        Some(w) => Caller {
            bus: bus.into(),
            app: w.app,
            label: w.label,
        },
        None => Caller {
            bus: bus.into(),
            app: format!("bus:{bus}"),
            label: "An unknown program".into(),
        },
    }
}
