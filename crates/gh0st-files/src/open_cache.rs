//! A file on a server or in an archive opens from a copy here, read only:
//! `~/.cache/gh0st-files/open/<hash>/<name>`. The app that opens it can't
//! write back, not even by a new file renamed over it, so a change can't
//! look saved where it isn't. Kept to 256 MB, the least lately opened
//! first, trimmed when Files starts.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const CAP: u64 = 256 << 20;

pub fn root(home: &Path) -> PathBuf {
    home.join(".cache/gh0st-files/open")
}

/// The folder for the file at `key` (where it is, as Files names it).
pub fn slot(home: &Path, key: &str) -> PathBuf {
    root(home).join(format!("{:016x}", fnv64(key)))
}

/// A copy already there, the size and time the source says: open it as
/// it is. A source with no time (some WebDAV servers) is fetched again.
pub fn fresh(file: &Path, size: u64, mtime: i64) -> bool {
    use std::os::unix::fs::MetadataExt;
    mtime > 0
        && std::fs::metadata(file)
            .is_ok_and(|m| m.is_file() && m.len() == size && m.mtime() == mtime)
}

/// Read only, for the app that opens it, with the source's time so a
/// later look can tell it is the same. Its folder is read only too.
pub fn seal(file: &Path, mtime: i64) {
    if let (Ok(f), Ok(secs)) = (
        std::fs::File::options().write(true).open(file),
        u64::try_from(mtime),
    ) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let _ = f.set_modified(t);
    }
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o444));
    if let Some(dir) = file.parent() {
        touch(dir);
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555));
    }
}

/// The slot was opened now: trim keeps it longer.
pub fn touch(dir: &Path) {
    if let Ok(f) = std::fs::File::open(dir) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Removes a slot, sealed or not.
pub fn remove(dir: &Path) {
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    let _ = std::fs::remove_dir_all(dir);
}

/// Removes the copies opened longest ago until all of them fit in `cap`.
/// The newest stays, however big: it may be open in its app now.
pub fn trim(home: &Path, cap: u64) {
    let Ok(rd) = std::fs::read_dir(root(home)) else {
        return;
    };
    let mut slots: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let m = e.metadata().ok()?;
            let size: u64 = std::fs::read_dir(&p)
                .ok()?
                .flatten()
                .filter_map(|f| f.metadata().ok())
                .map(|m| m.len())
                .sum();
            Some((m.modified().ok()?, size, p))
        })
        .collect();
    slots.sort();
    let mut total: u64 = slots.iter().map(|s| s.1).sum();
    slots.pop();
    for (_, size, p) in slots {
        if total <= cap {
            break;
        }
        remove(&p);
        if !p.exists() {
            total -= size;
        }
    }
}

fn fnv64(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(dir: &Path, secs: u64) {
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::open(dir).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn the_oldest_copies_go_first() {
        let home = std::env::temp_dir().join(format!("gh0st-open-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for (i, key) in ["old", "mid", "new"].iter().enumerate() {
            let d = slot(&home, key);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("f"), vec![0u8; 100]).unwrap();
            // Distinct times, oldest first.
            at(&d, 1000 + i as u64);
        }
        trim(&home, 250);
        assert!(!slot(&home, "old").exists());
        assert!(slot(&home, "mid").exists() && slot(&home, "new").exists());
        assert_eq!(
            slot(&home, "new"),
            slot(&home, "new"),
            "a key keeps its folder"
        );
        let f = slot(&home, "new").join("f");
        seal(&f, 1_700_000_000);
        assert!(fresh(&f, 100, 1_700_000_000));
        assert!(!fresh(&f, 99, 1_700_000_000) && !fresh(&f, 100, 1_700_000_001));
        let mode = std::fs::metadata(&f).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o444);
        // Nothing new can be put in its folder, so nothing renamed over it.
        // SAFETY: geteuid can't fail. Root writes anywhere.
        if unsafe { libc::geteuid() } != 0 {
            assert!(std::fs::write(slot(&home, "new").join("g"), b"x").is_err());
        }
        // Sealed last, so opened last: mid goes first now.
        trim(&home, 150);
        assert!(!slot(&home, "mid").exists() && f.exists());
        // The newest stays, however big.
        trim(&home, 10);
        assert!(f.exists());
        remove(&slot(&home, "new"));
        assert!(!slot(&home, "new").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_source_with_no_time_is_never_fresh() {
        let dir = std::env::temp_dir().join(format!("gh0st-open-notime-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("f");
        std::fs::write(&f, b"abc").unwrap();
        seal(&f, 0);
        assert!(!fresh(&f, 3, 0));
        remove(&dir);
        assert!(!dir.exists());
    }
}
