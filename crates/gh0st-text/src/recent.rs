//! Files opened lately, newest first. Text's own list for now: one path a
//! line in its data folder. A list shared with other apps can stand in
//! behind the same calls.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Paths kept.
pub const MAX: usize = 20;

#[derive(Default)]
pub struct Recent {
    file: Option<PathBuf>,
    list: Vec<PathBuf>,
}

impl Recent {
    /// The list kept in `dir`.
    pub fn load(dir: &Path) -> Recent {
        let file = dir.join("recent");
        let list = std::fs::read(&file)
            .unwrap_or_default()
            .split(|&b| b == b'\n')
            .filter(|l| l.first() == Some(&b'/'))
            .map(|l| PathBuf::from(OsStr::from_bytes(l)))
            .take(MAX)
            .collect();
        Recent {
            file: Some(file),
            list,
        }
    }

    pub fn list(&self) -> &[PathBuf] {
        &self.list
    }

    /// `p` goes first. Only absolute paths with no newline are kept.
    pub fn add(&mut self, p: &Path) {
        let b = p.as_os_str().as_bytes();
        if !p.is_absolute() || b.contains(&b'\n') {
            return;
        }
        self.list.retain(|q| q != p);
        self.list.insert(0, p.to_path_buf());
        self.list.truncate(MAX);
        self.save();
    }

    pub fn remove(&mut self, p: &Path) {
        self.list.retain(|q| q != p);
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut out = Vec::new();
        for p in &self.list {
            out.extend_from_slice(p.as_os_str().as_bytes());
            out.push(b'\n');
        }
        let tmp = file.with_extension("new");
        if std::fs::write(&tmp, &out).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_once_each_and_kept() {
        let d = std::env::temp_dir().join(format!("gh0st-text-recent-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut r = Recent::load(&d);
        r.add(Path::new("/a"));
        r.add(Path::new("/b"));
        r.add(Path::new("/a"));
        r.add(Path::new("relative"));
        r.add(Path::new("/odd\nname"));
        assert_eq!(r.list(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        let again = Recent::load(&d);
        assert_eq!(again.list(), r.list());
        for i in 0..30 {
            r.add(Path::new(&format!("/f{i}")));
        }
        assert_eq!(r.list().len(), MAX);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
