//! The panel, for a program that draws on it itself. A stock app run with
//! `Terminal=true` may map `/dev/fb_lcd` and paint over the terminal; on
//! stock it pauses the launcher by name to keep the screen. Here the
//! terminal stops drawing while a program in one of its tabs has the panel
//! open. inotify says when anyone opens or closes it, then a look in
//! `/proc` says whose it is. Nothing polls.

use std::fs::Metadata;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub struct Panel {
    fd: OwnedFd,
    id: Id,
}

/// What makes an fd the panel's: the device, whichever node opened it, or
/// for a plain file (a test's stand-in) the file itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Id {
    Device(u64),
    File(u64, u64),
}

impl Id {
    fn of(m: &Metadata) -> Id {
        match m.rdev() {
            0 => Id::File(m.dev(), m.ino()),
            r => Id::Device(r),
        }
    }
}

impl Panel {
    /// Watches who opens the panel. Off Linux, or with no panel, this fails.
    #[cfg(target_os = "linux")]
    pub fn open(path: &Path) -> io::Result<Panel> {
        use std::os::fd::FromRawFd;
        use std::os::unix::ffi::OsStrExt;
        let id = Id::of(&std::fs::metadata(path)?);
        let mut c = path.as_os_str().as_bytes().to_vec();
        c.push(0);
        // SAFETY: inotify calls with a NUL-terminated path built here.
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(fd);
            let mask = libc::IN_OPEN | libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE;
            if libc::inotify_add_watch(fd.as_raw_fd(), c.as_ptr().cast(), mask) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Panel { fd, id })
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn open(_path: &Path) -> io::Result<Panel> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Reads the events waiting. Which they were doesn't matter: each means
    /// a look in `/proc`.
    pub fn drain(&self) {
        let mut buf = [0u8; 1024];
        // SAFETY: reads into a stack buffer of its own length.
        while unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    }

    /// True when a process in one of `sessions` has the panel open.
    pub fn held(&self, sessions: &[i32]) -> bool {
        let id = self.id;
        held(Path::new("/proc"), sessions, |m| Id::of(m) == id)
    }
}

/// True when a process under `proc` whose session is one of `sessions` has
/// an fd that `is_panel` takes.
pub fn held(proc: &Path, sessions: &[i32], is_panel: impl Fn(&Metadata) -> bool) -> bool {
    let Ok(dir) = std::fs::read_dir(proc) else {
        return false;
    };
    dir.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .filter(|e| session(&e.path()).is_some_and(|s| sessions.contains(&s)))
        .any(|e| {
            std::fs::read_dir(e.path().join("fd")).is_ok_and(|fds| {
                fds.flatten()
                    .any(|f| std::fs::metadata(f.path()).is_ok_and(|m| is_panel(&m)))
            })
        })
}

/// The session in `/proc/PID/stat`: the fourth field after the name, which
/// is in brackets and may hold anything.
fn session(pid_dir: &Path) -> Option<i32> {
    let stat = std::fs::read_to_string(pid_dir.join("stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(3)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake `/proc`: pid 10 in session 10 holds the "panel" file, pid 20
    /// in session 20 holds another.
    fn fake(root: &Path, panel: &Path, other: &Path) {
        for (pid, file) in [(10, panel), (20, other)] {
            let d = root.join(pid.to_string());
            std::fs::create_dir_all(d.join("fd")).unwrap();
            let stat = format!("{pid} (a) b) S 1 {pid} {pid} 0 -1\n");
            std::fs::write(d.join("stat"), stat).unwrap();
            std::os::unix::fs::symlink(file, d.join("fd/3")).unwrap();
        }
        std::fs::create_dir_all(root.join("self")).unwrap();
    }

    #[test]
    fn only_a_tabs_session_holding_the_panel_counts() {
        let root = std::env::temp_dir().join(format!("gh0st-term-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (panel, other) = (root.join("panel"), root.join("other"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&panel, b"").unwrap();
        std::fs::write(&other, b"").unwrap();
        let proc = root.join("proc");
        fake(&proc, &panel, &other);
        let ino = std::fs::metadata(&panel).unwrap().ino();
        let is_panel = |m: &Metadata| m.ino() == ino;
        assert!(held(&proc, &[10], is_panel));
        assert!(!held(&proc, &[20], is_panel), "another session's file");
        assert!(!held(&proc, &[30], is_panel), "no process in it");
        std::fs::remove_file(proc.join("10/fd/3")).unwrap();
        assert!(!held(&proc, &[10], is_panel), "closed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_open_is_heard() {
        let path = std::env::temp_dir().join(format!("gh0st-term-open-{}", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let p = Panel::open(&path).unwrap();
        let mut fds = [libc::pollfd {
            fd: p.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: poll on one pollfd built here.
        assert_eq!(unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) }, 0);
        drop(std::fs::File::open(&path).unwrap());
        // SAFETY: as above.
        assert_eq!(unsafe { libc::poll(fds.as_mut_ptr(), 1, 100) }, 1);
        p.drain();
        // SAFETY: as above.
        assert_eq!(unsafe { libc::poll(fds.as_mut_ptr(), 1, 0) }, 0);
        let _ = std::fs::remove_file(&path);
    }
}
