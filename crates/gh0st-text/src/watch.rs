//! Another program changing the open file. inotify on its folder wakes the
//! loop; then the file's identity (device, inode, size, mtime) says whether
//! it is still the file we read. Our own saves update that identity, so
//! they don't count. No polling.

use crate::buf::save::Disk;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// It has other contents now.
    Modified,
    /// It was deleted or moved away.
    Gone,
}

pub struct FileWatch {
    inner: Option<gh0st_core::status::Watch>,
    path: PathBuf,
    known: Option<Disk>,
}

impl FileWatch {
    pub fn new(path: &Path) -> FileWatch {
        FileWatch {
            inner: open(path.parent().unwrap_or(Path::new("/"))),
            path: path.to_path_buf(),
            known: Disk::of(path),
        }
    }

    /// For the page to watch. None off Linux.
    pub fn fd(&self) -> Option<RawFd> {
        self.inner.as_ref().map(|w| w.fd())
    }

    /// We wrote the file: what is there now is ours.
    pub fn saved(&mut self) {
        self.known = Disk::of(&self.path);
    }

    /// Empties the watch and says what changed, if anything.
    pub fn check(&mut self) -> Option<Change> {
        if let Some(w) = &self.inner {
            w.drain();
        }
        let now = Disk::of(&self.path);
        if now == self.known {
            return None;
        }
        // Say it once.
        self.known = now;
        Some(if now.is_some() {
            Change::Modified
        } else {
            Change::Gone
        })
    }
}

#[cfg(target_os = "linux")]
fn open(dir: &Path) -> Option<gh0st_core::status::Watch> {
    gh0st_core::status::Watch::dirs(&[dir]).ok()
}

#[cfg(not(target_os = "linux"))]
fn open(_: &Path) -> Option<gh0st_core::status::Watch> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_save_is_quiet_and_theirs_is_not() {
        let d = std::env::temp_dir().join(format!("gh0st-text-watch-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("f.txt");
        std::fs::write(&p, "one").unwrap();
        let mut w = FileWatch::new(&p);
        assert_eq!(w.check(), None);
        let mut b = crate::buf::Buffer::open(&p).unwrap();
        b.insert(0, b"x").unwrap();
        crate::buf::save::save(&mut b, &p).unwrap();
        w.saved();
        assert_eq!(w.check(), None);
        std::fs::write(&p, "someone else").unwrap();
        assert_eq!(w.check(), Some(Change::Modified));
        assert_eq!(w.check(), None);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(w.check(), Some(Change::Gone));
    }
}
