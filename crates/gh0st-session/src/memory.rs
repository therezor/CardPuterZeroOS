//! The portal's `org.freedesktop.portal.MemoryMonitor`, on PSI triggers.
//! The kernel wakes the loop when tasks stall on memory for long enough in
//! a window; nothing polls. Each trigger sends `LowMemoryWarning` and tells
//! the shell, which tells the apps.
//!
//! PSI needs `psi=1` on the kernel command line. Without it there is no
//! `/proc/pressure/memory`, and the monitor is quiet.

use gh0st_dbus::{Message, Value};
use std::os::fd::{AsRawFd, OwnedFd};

pub const IFACE: &str = "org.freedesktop.portal.MemoryMonitor";
pub const VERSION: u32 = 1;
const PATH: &str = "/org/freedesktop/portal/desktop";
const PRESSURE: &str = "/proc/pressure/memory";

/// Stall time in a 2 s window, the shortest window an unprivileged
/// trigger may have, and the level each one reports.
const TRIGGERS: [(&str, u8); 3] = [
    ("some 140000 2000000", 50),
    ("some 300000 2000000", 100),
    ("full 300000 2000000", 255),
];

#[derive(Default)]
pub struct Memory {
    triggers: Vec<(OwnedFd, u8)>,
}

impl Memory {
    /// Opens the triggers. A kernel without PSI gives none.
    pub fn open() -> Memory {
        let mut triggers = Vec::new();
        for (spec, level) in TRIGGERS {
            match trigger(spec) {
                Ok(fd) => triggers.push((fd, level)),
                Err(e) => {
                    if triggers.is_empty() {
                        eprintln!("gh0st session: no memory pressure ({PRESSURE}: {e})");
                    }
                    break;
                }
            }
        }
        Memory { triggers }
    }

    /// Fds to poll, for `POLLPRI`.
    pub fn pollfds(&self) -> impl Iterator<Item = libc::pollfd> + '_ {
        self.triggers.iter().map(|(fd, _)| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLPRI,
            revents: 0,
        })
    }

    /// The level of the triggers that fired, in `pollfds` order: the worst
    /// one wins.
    pub fn level(&self, fired: &[bool]) -> Option<u8> {
        self.triggers
            .iter()
            .zip(fired)
            .filter(|(_, f)| **f)
            .map(|((_, l), _)| *l)
            .max()
    }

    pub fn len(&self) -> usize {
        self.triggers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }
}

/// The signal every listener gets.
pub fn warning(level: u8) -> Message {
    Message::signal(PATH, IFACE, "LowMemoryWarning").arg(Value::Byte(level))
}

fn trigger(spec: &str) -> std::io::Result<OwnedFd> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(PRESSURE)?;
    // The kernel takes the trigger with its NUL.
    f.write_all(format!("{spec}\0").as_bytes())?;
    Ok(f.into())
}

pub const XML: &str = r#"<interface name="org.freedesktop.portal.MemoryMonitor"><signal name="LowMemoryWarning"><arg type="y"/></signal><property name="version" type="u" access="read"/></interface>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worst_trigger_that_fired_is_the_level() {
        let m = Memory::open();
        // Whatever this host has, no trigger fired means no level.
        assert_eq!(m.level(&vec![false; m.len()]), None);
        assert_eq!(warning(100).body[0], Value::Byte(100));
    }
}
