//! The freedesktop Trash in the home folder. A file moves to `files/`, and
//! an `.trashinfo` in `info/` says where it came from, so any file manager
//! can put it back.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The longest file name most file systems take.
const NAME_MAX: usize = 255;
const INFO: &str = ".trashinfo";

/// `~/.local/share/Trash` under `home`.
pub fn dir(home: &Path) -> PathBuf {
    home.join(".local/share/Trash")
}

/// Moves `path` into the Trash under `home`. Returns where it went. A file
/// on another drive fails with `EXDEV`: that drive has no Trash here.
///
/// The `.trashinfo` is made first and only if new, as the spec asks, so
/// two putting the same name at once (Files and the portal) each get their
/// own. A name keeps every byte.
pub fn put(home: &Path, path: &Path) -> io::Result<PathBuf> {
    let base = dir(home);
    let (files, info) = (base.join("files"), base.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .as_bytes();
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        pct(path.as_os_str().as_bytes()),
        now_iso()
    );
    for n in 1.. {
        let pick = pick(name, n);
        let to = files.join(OsStr::from_bytes(&pick));
        let info_file = info.join(OsStr::from_bytes(&[&pick[..], INFO.as_bytes()].concat()));
        // Exclusive, so it never follows a link left in info/ either.
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_file)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        // A file with no info of its own still holds its name.
        if to.symlink_metadata().is_ok() {
            drop(f);
            let _ = std::fs::remove_file(&info_file);
            continue;
        }
        let moved = f
            .write_all(body.as_bytes())
            .and_then(|_| std::fs::rename(path, &to));
        if let Err(e) = moved {
            let _ = std::fs::remove_file(&info_file);
            return Err(e);
        }
        return Ok(to);
    }
    Err(io::Error::other("no free name in the Trash"))
}

/// The name in the Trash: `name`, numbered from 2 as [`numbered`] does,
/// cut so its `.trashinfo` still fits in a file name.
fn pick(name: &[u8], n: u32) -> Vec<u8> {
    let tag = if n < 2 {
        String::new()
    } else {
        format!(" {n}")
    };
    let room = NAME_MAX - INFO.len() - tag.len();
    let (stem, ext) = match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 && name.len() - i < room / 2 => name.split_at(i),
        _ => (name, &[][..]),
    };
    let mut keep = stem.len().min(room - ext.len());
    // Never inside a UTF-8 character.
    while keep > 0 && keep < stem.len() && stem[keep] & 0xC0 == 0x80 {
        keep -= 1;
    }
    [&stem[..keep], tag.as_bytes(), ext].concat()
}

/// "photo 2.png" for "photo.png", n = 2.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} {n}.{ext}"),
        _ => format!("{name} {n}"),
    }
}

/// The spec's URL escaping, of the raw bytes: any name survives.
fn pct(s: &[u8]) -> String {
    let mut out = String::new();
    for &b in s {
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
        for (name, n) in [("photo.png", 2), (".bashrc", 3), ("notes", 2), ("a.b.c", 4)] {
            assert_eq!(pick(name.as_bytes(), n), numbered(name, n).as_bytes());
        }
        assert_eq!(pick(b"photo.png", 1), b"photo.png");
    }

    #[test]
    fn a_long_name_is_cut_to_leave_room_for_its_info() {
        let long = format!("{}.txt", "\u{e9}".repeat(200));
        for n in [1, 2, 12345] {
            let p = pick(long.as_bytes(), n);
            assert!(p.len() + INFO.len() <= NAME_MAX, "{}", p.len());
            assert!(std::str::from_utf8(&p).is_ok(), "no character is split");
            assert!(p.ends_with(format!(" {n}.txt").as_bytes()) || n == 1);
        }
        let home = std::env::temp_dir().join(format!("gh0st-trash-long-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let f = home.join("x".repeat(250));
        std::fs::write(&f, b"1").unwrap();
        let to = put(&home, &f).unwrap();
        assert!(to.exists() && !f.exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn every_byte_of_the_path_is_escaped() {
        assert_eq!(pct(b"/home/u/a b\xff%.txt"), "/home/u/a%20b%FF%25.txt");
    }

    #[test]
    fn a_link_in_info_is_never_written_through() {
        let home = std::env::temp_dir().join(format!("gh0st-trash-link-{}", std::process::id()));
        let info = dir(&home).join("info");
        std::fs::create_dir_all(&info).unwrap();
        let aim = home.join("aim");
        std::os::unix::fs::symlink(&aim, info.join("x.txt.trashinfo")).unwrap();
        std::fs::write(home.join("x.txt"), b"1").unwrap();
        let to = put(&home, &home.join("x.txt")).unwrap();
        assert_eq!(to.file_name().unwrap(), "x 2.txt");
        assert!(!aim.exists(), "the link was not followed");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
