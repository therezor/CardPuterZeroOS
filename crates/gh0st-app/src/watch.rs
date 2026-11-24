//! Changes in a folder, as an fd for [`Cx::watch`](crate::Cx::watch):
//! inotify, so nothing polls. A file's own changes show up as its name in
//! its folder, which also sees an editor's save by rename.

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;

pub struct DirWatch {
    fd: OwnedFd,
}

/// What [`DirWatch::changes`] saw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Names in the folder that were written, made, moved or removed.
    pub names: Vec<OsString>,
    /// The folder itself went away or moved.
    pub gone: bool,
    /// Too much happened to list: read the folder again.
    pub all: bool,
}

impl DirWatch {
    /// Watches `dir`. Off Linux there is no inotify, and this fails.
    #[cfg(target_os = "linux")]
    pub fn new(dir: &Path) -> io::Result<DirWatch> {
        use std::os::fd::FromRawFd;
        use std::os::unix::ffi::OsStrExt;
        let mut c = dir.as_os_str().as_bytes().to_vec();
        c.push(0);
        // SAFETY: inotify calls with a NUL-terminated path built here.
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(fd);
            let mask = libc::IN_CREATE
                | libc::IN_DELETE
                | libc::IN_CLOSE_WRITE
                | libc::IN_MOVED_FROM
                | libc::IN_MOVED_TO
                | libc::IN_ATTRIB
                | libc::IN_DELETE_SELF
                | libc::IN_MOVE_SELF
                | libc::IN_ONLYDIR;
            if libc::inotify_add_watch(fd.as_raw_fd(), c.as_ptr().cast(), mask) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(DirWatch { fd })
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(_dir: &Path) -> io::Result<DirWatch> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Reads every event waiting. Call it from [`Page::ready`](crate::Page::ready).
    pub fn changes(&self) -> Changes {
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut out = Changes::default();
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::OsStrExt;
            let mut buf = [0u8; 4096];
            loop {
                // SAFETY: reads into a stack buffer of its own length.
                let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                let mut at = 0usize;
                let head = std::mem::size_of::<libc::inotify_event>();
                while at + head <= n as usize {
                    // SAFETY: the kernel wrote whole events; read one unaligned.
                    let ev: libc::inotify_event =
                        unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast()) };
                    let name = &buf[at + head..(at + head + ev.len as usize).min(n as usize)];
                    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
                    if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                        out.all = true;
                    }
                    if ev.mask & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED) != 0
                    {
                        out.gone = true;
                    }
                    if !name.is_empty() {
                        let name = std::ffi::OsStr::from_bytes(name).to_os_string();
                        if !out.names.contains(&name) {
                            out.names.push(name);
                        }
                    }
                    at += head + ev.len as usize;
                }
            }
        }
        out
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn writes_renames_and_removals_are_seen_by_name() {
        let d = std::env::temp_dir().join(format!("gh0st-dirwatch-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let w = DirWatch::new(&d).unwrap();
        assert_eq!(w.changes(), Changes::default());
        std::fs::write(d.join("a.txt"), b"1").unwrap();
        std::fs::write(d.join(".a.tmp"), b"2").unwrap();
        std::fs::rename(d.join(".a.tmp"), d.join("a.txt")).unwrap();
        let c = w.changes();
        assert!(c.names.contains(&"a.txt".into()), "{c:?}");
        std::fs::remove_dir_all(&d).unwrap();
        assert!(w.changes().gone);
    }
}
