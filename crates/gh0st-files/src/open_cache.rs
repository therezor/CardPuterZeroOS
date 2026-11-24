//! A file on a server or in an archive opens from a copy here, read only:
//! `~/.cache/gh0st-files/open/<hash>/<name>`. The app that opens it can't
//! write back, so a change can't look saved where it isn't. Kept to 256 MB,
//! oldest first, trimmed when Files starts.

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

/// A copy already there, the size the source says: open it as it is.
pub fn fresh(file: &Path, size: u64) -> bool {
    std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.len() == size)
}

/// Read only, for the app that opens it.
pub fn seal(file: &Path) {
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o444));
}

/// Removes the oldest copies until all of them fit in `cap`.
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
    for (_, size, p) in slots {
        if total <= cap {
            break;
        }
        if std::fs::remove_dir_all(&p).is_ok() {
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

    #[test]
    fn the_oldest_copies_go_first() {
        let home = std::env::temp_dir().join(format!("gh0st-open-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for (i, key) in ["old", "mid", "new"].iter().enumerate() {
            let d = slot(&home, key);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("f"), vec![0u8; 100]).unwrap();
            // Distinct times, oldest first.
            let t =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            std::fs::File::open(&d).unwrap().set_modified(t).unwrap();
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
        assert!(fresh(&f, 100) && !fresh(&f, 99));
        seal(&f);
        let mode = std::fs::metadata(&f).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o444);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
