//! The freedesktop Trash in the home folder. A file moves to `files/`, and
//! an `.trashinfo` in `info/` says where it came from, so any file manager
//! can put it back.

use std::io;
use std::path::{Path, PathBuf};

/// `~/.local/share/Trash` under `home`.
pub fn dir(home: &Path) -> PathBuf {
    home.join(".local/share/Trash")
}

/// Moves `path` into the Trash under `home`. Returns where it went. A file
/// on another drive fails with `EXDEV`: that drive has no Trash here.
pub fn put(home: &Path, path: &Path) -> io::Result<PathBuf> {
    let base = dir(home);
    let (files, info) = (base.join("files"), base.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut pick = name.clone();
    let mut n = 2;
    while files.join(&pick).symlink_metadata().is_ok()
        || info.join(format!("{pick}.trashinfo")).exists()
    {
        pick = numbered(&name, n);
        n += 1;
    }
    let info_file = info.join(format!("{pick}.trashinfo"));
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        pct(&path.to_string_lossy()),
        now_iso()
    );
    std::fs::write(&info_file, body)?;
    let to = files.join(&pick);
    if let Err(e) = std::fs::rename(path, &to) {
        let _ = std::fs::remove_file(&info_file);
        return Err(e);
    }
    Ok(to)
}

/// "photo 2.png" for "photo.png", n = 2.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} {n}.{ext}"),
        _ => format!("{name} {n}"),
    }
}

fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Local time, as the spec asks.
fn now_iso() -> String {
    // SAFETY: time and localtime_r into a local struct tm.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_file_of_the_same_name_is_numbered_and_both_say_where_from() {
        let home = std::env::temp_dir().join(format!("gh0st-trash-{}", std::process::id()));
        let docs = home.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        for _ in 0..2 {
            std::fs::write(docs.join("x y.txt"), b"1").unwrap();
            put(&home, &docs.join("x y.txt")).unwrap();
        }
        let t = dir(&home);
        assert!(t.join("files/x y.txt").exists() && t.join("files/x y 2.txt").exists());
        let info = std::fs::read_to_string(t.join("info/x y 2.txt.trashinfo")).unwrap();
        assert!(
            info.contains("Path=") && info.contains("x%20y.txt"),
            "{info}"
        );
        assert!(info.contains("DeletionDate="));
        assert!(put(&home, Path::new("/")).is_err());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn numbers_go_before_the_extension() {
        assert_eq!(numbered("photo.png", 2), "photo 2.png");
        assert_eq!(numbered(".bashrc", 3), ".bashrc 3");
        assert_eq!(numbered("notes", 2), "notes 2");
    }
}
