//! Saving. The text goes to a new file next to the old one, which is synced
//! and then renamed over it, so a crash or a full card mid-save leaves the
//! old file whole. The pieces still read the old file, which stays open, so
//! a save never reads what it is writing.
//!
//! A file with other hard links, or in a folder we can't write, is written
//! in place instead: the page asks first, as a crash then can lose it.

use super::{Buffer, Piece, Src, Store};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The largest file written in place: it is read into memory first.
pub const IN_PLACE_MAX: u64 = 64 << 20;

/// How a save will go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// A new file renamed over the old one.
    Replace,
    /// Over the old file's bytes: it has other hard links, or its folder is
    /// read-only. A crash part way loses the file.
    InPlace,
    /// Neither: offer Save as.
    ReadOnly,
}

/// Which file a path names, so a change by another program shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disk {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mtime_ns: i64,
}

impl Disk {
    pub fn of(path: &Path) -> Option<Disk> {
        let m = std::fs::metadata(path).ok()?;
        Some(Disk {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
        })
    }
}

/// The file a save writes: symlinks followed, so the link stays a link.
pub fn target(path: &Path) -> io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or_else(|| io::Error::other("no file name"))?;
            let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
            let dir = std::fs::canonicalize(dir.unwrap_or(Path::new(".")))?;
            Ok(dir.join(name))
        }
        Err(e) => Err(e),
    }
}

fn writable(p: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let mut c = p.as_os_str().as_bytes().to_vec();
    c.push(0);
    // SAFETY: a NUL-terminated path built here.
    unsafe { libc::access(c.as_ptr() as *const libc::c_char, libc::W_OK) == 0 }
}

/// How saving to `path` would go.
pub fn way(path: &Path) -> io::Result<Way> {
    let t = target(path)?;
    let dir = t.parent().unwrap_or(Path::new("/"));
    let meta = std::fs::metadata(&t).ok();
    let dir_ok = writable(dir);
    match meta {
        None if dir_ok => Ok(Way::Replace),
        None => Ok(Way::ReadOnly),
        Some(m) if dir_ok && m.nlink() <= 1 => Ok(Way::Replace),
        Some(_) if writable(&t) => Ok(Way::InPlace),
        Some(_) => Ok(Way::ReadOnly),
    }
}

/// A save under way, a slice at a time so the screen keeps moving.
pub struct Saver {
    target: PathBuf,
    way: Way,
    /// The new file, for a replace; removed if the save stops part way.
    tmp: Option<PathBuf>,
    file: File,
    /// The whole text, for an in-place save.
    mem: Option<Vec<u8>>,
    off: u64,
    len: u64,
    mode: Option<u32>,
}

impl Saver {
    /// Starts saving `buf` to `path` the way [`way`] says. In place, the
    /// text is read into memory here, and must fit [`IN_PLACE_MAX`].
    pub fn start(buf: &mut Buffer, path: &Path, how: Way) -> io::Result<Saver> {
        let target = target(path)?;
        let len = buf.len();
        let mode = std::fs::metadata(&target)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777);
        match how {
            Way::ReadOnly => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            Way::Replace => {
                let dir = target.parent().unwrap_or(Path::new("/"));
                let name = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let tmp = dir.join(format!(".{name}.{}.gh0st-text", std::process::id()));
                let _ = std::fs::remove_file(&tmp);
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    // A new file gets the usual mode through the umask; an
                    // old file's mode is set once the text is in.
                    .mode(if mode.is_some() { 0o600 } else { 0o666 })
                    .open(&tmp)?;
                Ok(Saver {
                    target,
                    way: how,
                    tmp: Some(tmp),
                    file,
                    mem: None,
                    off: 0,
                    len,
                    mode,
                })
            }
            Way::InPlace => {
                if len > IN_PLACE_MAX {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        "too big to save in place",
                    ));
                }
                let mem = buf.bytes(0..len)?;
                let file = OpenOptions::new().write(true).open(&target)?;
                Ok(Saver {
                    target,
                    way: how,
                    tmp: None,
                    file,
                    mem: Some(mem),
                    off: 0,
                    len,
                    mode: None,
                })
            }
        }
    }

    /// How far it has got, of how much.
    pub fn progress(&self) -> (u64, u64) {
        (self.off, self.len)
    }

    /// Writes about `budget` bytes. True once the save is done: the file
    /// is synced and in place, and `buf` reads from it.
    pub fn step(&mut self, buf: &mut Buffer, budget: u64) -> io::Result<bool> {
        if self.way == Way::Replace && self.off < self.len {
            let end = self.len.min(self.off + budget.max(1));
            let mut err = None;
            let file = &mut self.file;
            let mut off = self.off;
            let from = self.off;
            // Only what is in memory: the card reads ahead meanwhile.
            let r = buf.nowait(|b| {
                b.for_each(from..end, |c| match file.write_all(c) {
                    Ok(()) => {
                        off += c.len() as u64;
                        true
                    }
                    Err(e) => {
                        err = Some(e);
                        false
                    }
                })
            });
            self.off = off;
            if let Some(e) = err {
                return Err(e);
            }
            r?;
            if self.off < self.len {
                return Ok(false);
            }
        }
        self.finish(buf)?;
        Ok(true)
    }

    fn finish(&mut self, buf: &mut Buffer) -> io::Result<()> {
        match self.way {
            Way::Replace => {
                if let Some(m) = self.mode {
                    self.file
                        .set_permissions(std::fs::Permissions::from_mode(m))?;
                }
                self.file.sync_all()?;
                let tmp = self
                    .tmp
                    .take()
                    .ok_or_else(|| io::Error::other("saved twice"))?;
                if let Err(e) = std::fs::rename(&tmp, &self.target) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                // The rename itself, on the card.
                if let Some(dir) = self.target.parent() {
                    File::open(dir)?.sync_all()?;
                }
            }
            Way::InPlace | Way::ReadOnly => {
                let mem = self.mem.take().unwrap_or_default();
                self.file.write_all(&mem)?;
                self.file.set_len(mem.len() as u64)?;
                self.file.sync_all()?;
                self.off = self.len;
            }
        }
        buf.saved_as(Store::open(&self.target)?);
        buf.seal();
        Ok(())
    }
}

impl Drop for Saver {
    /// Stopped part way: the old file is whole, and the new one goes.
    fn drop(&mut self) {
        if let Some(t) = self.tmp.take() {
            let _ = std::fs::remove_file(t);
        }
    }
}

/// Saves `buf` to `path` in one go.
pub fn save(buf: &mut Buffer, path: &Path) -> io::Result<()> {
    let how = way(path)?;
    let mut s = Saver::start(buf, path, how)?;
    while !s.step(buf, 4 << 20)? {}
    Ok(())
}

impl Buffer {
    /// The file just saved becomes what the pieces read: one piece again.
    /// Older files stay open while undo needs them.
    pub(crate) fn saved_as(&mut self, store: Store) {
        let g = self.stores.len() as u32;
        let len = store.len();
        self.stores.push(store);
        // The index stays: the text is the same.
        self.pieces.clear();
        self.starts.clear();
        if len > 0 {
            self.pieces.push(Piece {
                src: Src::Orig(g),
                off: 0,
                len,
            });
            self.starts.push(0);
        }
        self.len = len;
        self.version += 1;
        if !self.undo.can_undo() && !self.undo.can_redo() {
            for s in &mut self.stores[..g as usize] {
                *s = Store::empty();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gh0st-text-save-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn all(b: &mut Buffer) -> Vec<u8> {
        let n = b.len();
        b.bytes(0..n).unwrap()
    }

    #[test]
    fn a_save_replaces_the_file_and_keeps_its_mode() {
        let d = dir("replace");
        let p = d.join("a.txt");
        std::fs::write(&p, "hello\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o640)).unwrap();
        let mut b = Buffer::open(&p).unwrap();
        b.insert(5, b" world").unwrap();
        assert_eq!(way(&p).unwrap(), Way::Replace);
        save(&mut b, &p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello world\n");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(b.pieces(), 1);
        // Undo still reaches back past the save.
        b.undo();
        assert_eq!(all(&mut b), b"hello\n");
        // No temp file is left.
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn a_symlink_stays_a_link_and_its_target_changes() {
        let d = dir("link");
        let real = d.join("real.txt");
        let link = d.join("link.txt");
        std::fs::write(&real, "one\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut b = Buffer::open(&link).unwrap();
        b.insert(0, b"zero\n").unwrap();
        save(&mut b, &link).unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&real).unwrap(), b"zero\none\n");
    }

    #[test]
    fn a_hard_linked_file_is_written_in_place() {
        let d = dir("hard");
        let a = d.join("a.txt");
        let other = d.join("b.txt");
        std::fs::write(&a, "shared\n").unwrap();
        std::fs::hard_link(&a, &other).unwrap();
        assert_eq!(way(&a).unwrap(), Way::InPlace);
        let mut b = Buffer::open(&a).unwrap();
        b.replace(0..6, b"both").unwrap();
        save(&mut b, &a).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"both\n");
        assert_eq!(all(&mut b), b"both\n");
    }

    #[test]
    fn a_new_file_is_created() {
        let d = dir("new");
        let p = d.join("new.txt");
        let mut b = Buffer::from_bytes(b"fresh");
        save(&mut b, &p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"fresh");
    }

    #[test]
    fn a_save_stopped_part_way_leaves_the_old_file_whole() {
        let d = dir("stopped");
        let p = d.join("big.txt");
        let text = vec![b'a'; 3 << 20];
        std::fs::write(&p, &text).unwrap();
        let mut b = Buffer::open(&p).unwrap();
        b.insert(0, b"b").unwrap();
        let mut s = Saver::start(&mut b, &p, Way::Replace).unwrap();
        assert!(!s.step(&mut b, 1 << 20).unwrap());
        drop(s);
        assert_eq!(std::fs::read(&p).unwrap(), text);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn a_read_only_folder_writes_in_place_or_not_at_all() {
        let d = dir("ro");
        let p = d.join("f.txt");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let w = way(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        let w2 = way(&p).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root can write anywhere; as a user the folder decides.
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(w, Way::InPlace);
            assert_eq!(w2, Way::ReadOnly);
        }
    }

    #[test]
    fn identity_changes_when_another_program_writes() {
        let d = dir("disk");
        let p = d.join("f.txt");
        std::fs::write(&p, "one").unwrap();
        let before = Disk::of(&p).unwrap();
        assert_eq!(Disk::of(&p), Some(before));
        std::fs::write(&p, "two two").unwrap();
        assert_ne!(Disk::of(&p), Some(before));
    }
}
