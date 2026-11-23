//! Screenshots: fetched one at a time when the user asks, checked against
//! the catalog's SHA-256, and kept in a small cache in ~/.cache.

use gh0st_core::store;
use std::path::{Path, PathBuf};

/// The cache holds this much; the oldest go first.
const KEEP_BYTES: u64 = 4 << 20;
/// A 320x170 PNG is far smaller; this stops a wrong file early.
const MAX_BYTES: &str = "2000000";

fn cache_home() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(x);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join(".cache")
}

pub fn dir() -> PathBuf {
    cache_home().join("gh0st-store/shots")
}

/// M5's Store kept every deb it downloaded here.
pub fn old_store_cache() -> PathBuf {
    cache_home().join("cardputerzero-appstore")
}

pub fn path(sha256: &str) -> PathBuf {
    dir().join(format!("{sha256}.png"))
}

pub fn cached(sha256: &str) -> Option<PathBuf> {
    let p = path(sha256);
    p.is_file().then_some(p)
}

/// The curl command that downloads `url` to `to`: HTTPS only, redirects
/// too, and no bigger than a screenshot can be.
pub fn fetch(url: &str, to: &Path) -> Vec<String> {
    [
        "curl",
        "-fsS",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-time",
        "30",
        "--max-filesize",
        MAX_BYTES,
        "-o",
    ]
    .into_iter()
    .map(String::from)
    .chain([to.to_string_lossy().into_owned(), url.to_string()])
    .collect()
}

/// Keeps the downloaded file if it is the one the catalog names.
pub fn accept(tmp: &Path, sha256: &str) -> bool {
    let ok = store::file_sha256(tmp).as_deref() == Some(sha256);
    if ok {
        ok_or_remove(std::fs::rename(tmp, path(sha256)), tmp);
        prune(&dir(), KEEP_BYTES);
    } else {
        let _ = std::fs::remove_file(tmp);
    }
    ok
}

fn ok_or_remove(r: std::io::Result<()>, tmp: &Path) {
    if r.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
}

/// Removes the oldest files until the folder holds `keep` bytes or less.
pub fn prune(dir: &Path, keep: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total <= keep {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

/// Bytes under `p`, not following links.
pub fn size_of(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else {
        return 0;
    };
    if !m.is_dir() {
        return m.len();
    }
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_the_oldest_first() {
        let d = std::env::temp_dir().join(format!("gh0st-store-prune-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let p = d.join(name);
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        assert_eq!(size_of(&d), 300);
        prune(&d, 150);
        assert!(!d.join("a").exists() && !d.join("b").exists());
        assert!(d.join("c").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fetch_allows_only_https() {
        let argv = fetch("https://x/1.png", Path::new("/tmp/t"));
        let s = argv.join(" ");
        assert!(s.contains("--proto =https --proto-redir =https"));
        assert!(s.ends_with("-o /tmp/t https://x/1.png"));
    }
}
