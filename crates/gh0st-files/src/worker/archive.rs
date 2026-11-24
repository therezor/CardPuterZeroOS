//! Archives through bsdtar: zip, tar and its compressions, 7z, rar, iso.
//! bsdtar refuses `..` and absolute paths unless given `-P`, and we never
//! give it. An extract goes into a fresh folder first, then moves in, so a
//! name already there asks like a copy does.

use super::copy;
use super::local::words;
use super::parse;
use super::{Backends, Prog, Pump, Res, STOPPED, run};
use crate::engine::Id;
use crate::model::{Entry, Loc};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The archive as a local file: itself on the card, a copy from a server.
fn local_copy(b: &mut Backends, p: &mut Pump, id: Id, archive: &Loc) -> Res<(PathBuf, bool)> {
    match archive {
        Loc::Local(path) => Ok((path.clone(), false)),
        _ => {
            let name = archive.leaf().unwrap_or_default();
            let to = b.stage().join(format!("{id}-{name}"));
            let (fs, path) = b.fs(p, archive)?;
            let mut prog = Prog::silent(id);
            fs.get(p, &path, &to, &mut prog)?;
            Ok((to, true))
        }
    }
}

fn bsdtar() -> Command {
    let mut c = Command::new("bsdtar");
    // Dates and messages as the parser reads them.
    c.env("LC_ALL", "C");
    c
}

fn tar_error(code: i32, err: &str) -> String {
    if code == -1 {
        return "bsdtar stopped".into();
    }
    let line = err
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("bsdtar failed")
        .trim_start_matches("bsdtar: ");
    if line.contains("Unrecognized archive format") {
        return "Not an archive Files can open".into();
    }
    if line.contains("passphrase") || line.contains("Encrypted") {
        return "The archive has a password: Files can't open it yet".into();
    }
    line.to_string()
}

fn list(p: &mut Pump, id: Id, file: &Path) -> Res<Vec<Entry>> {
    let mut cmd = bsdtar();
    cmd.arg("-tvf").arg(file);
    let (code, out, err) = run(p, &mut cmd, None, Some(id), &mut |_, _| Ok(())).map_err(|e| {
        if e.starts_with("Can't run") {
            "bsdtar isn't installed".into()
        } else {
            e
        }
    })?;
    if code != 0 {
        return Err(tar_error(code, &err));
    }
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(parse::ls_line)
        .map(|mut e| {
            e.name = e.name.trim_start_matches("./").to_string();
            e
        })
        .filter(|e| !e.name.is_empty())
        .collect())
}

pub fn contents(b: &mut Backends, p: &mut Pump, id: Id, archive: &Loc) -> Res<Vec<Entry>> {
    let (file, staged) = local_copy(b, p, id, archive)?;
    let r = list(p, id, &file);
    if staged {
        let _ = std::fs::remove_file(&file);
    }
    r
}

/// How many leading folders `entries` share: they come from one folder of
/// the archive, which the extract drops.
fn strip(entries: &[String]) -> usize {
    let dirs: Vec<Vec<&str>> = entries
        .iter()
        .map(|e| {
            let parts: Vec<&str> = e.trim_end_matches('/').split('/').collect();
            parts[..parts.len().saturating_sub(1)].to_vec()
        })
        .collect();
    let Some(first) = dirs.first() else {
        return 0;
    };
    (0..first.len())
        .take_while(|&i| dirs.iter().all(|d| d.get(i) == first.get(i)))
        .count()
}

/// Bytes free on the drive that holds `path`.
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs into a zeroed struct from a NUL-terminated path.
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) != 0 {
            return None;
        }
        Some(s.f_bavail as u64 * s.f_frsize as u64)
    }
}

pub fn extract(
    b: &mut Backends,
    p: &mut Pump,
    id: Id,
    archive: &Loc,
    entries: &[String],
    to: &Loc,
) -> Res<()> {
    let (file, staged) = local_copy(b, p, id, archive)?;
    let r = extract_local(b, p, id, &file, entries, to);
    if staged {
        let _ = std::fs::remove_file(&file);
    }
    r
}

fn extract_local(
    b: &mut Backends,
    p: &mut Pump,
    id: Id,
    file: &Path,
    entries: &[String],
    to: &Loc,
) -> Res<()> {
    let all = list(p, id, file)?;
    let picked: Vec<&Entry> = all
        .iter()
        .filter(|e| {
            entries.is_empty()
                || entries.iter().any(|w| {
                    let w = w.trim_end_matches('/');
                    e.name == w || e.name.starts_with(&format!("{w}/"))
                })
        })
        .collect();
    let total: u64 = picked.iter().map(|e| e.size).sum();
    // The new folder sits in the target when that is on the card, so the
    // move in is a rename.
    let stage = match to {
        Loc::Local(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| words(&e))?;
            dir.join(format!(".gh0st-extract-{}-{id}", std::process::id()))
        }
        _ => b.stage().join(format!("extract-{id}")),
    };
    if let Some(free) = stage.parent().and_then(free_bytes) {
        if free < total {
            return Err(format!(
                "Not enough space: it needs {}, {} is free",
                crate::model::size_text(total),
                crate::model::size_text(free)
            ));
        }
    }
    std::fs::create_dir_all(&stage).map_err(|e| words(&e))?;
    let r = (|| {
        let mut prog = Prog::new(id, total);
        prog.file(
            p,
            &file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let sizes: std::collections::HashMap<&str, u64> =
            picked.iter().map(|e| (e.name.as_str(), e.size)).collect();
        let mut cmd = bsdtar();
        cmd.arg("-xvf")
            .arg(file)
            .arg("-C")
            .arg(&stage)
            .arg("--no-same-owner")
            .arg("--strip-components")
            .arg(strip(entries).to_string());
        cmd.args(entries.iter().map(|e| e.trim_end_matches('/')));
        let mut rest = String::new();
        let (code, _, err) = run(p, &mut cmd, None, Some(id), &mut |p, piece| {
            // "x path" a file, on stderr, as each is written.
            rest.push_str(piece);
            while let Some(i) = rest.find('\n') {
                let line: String = rest.drain(..=i).collect();
                if let Some(path) = line.trim_end().strip_prefix("x ") {
                    let path = path.trim_start_matches("./").trim_end_matches('/');
                    if let Some(&n) = sizes.get(path) {
                        prog.add(p, n)?;
                    }
                }
            }
            Ok(())
        })?;
        if code != 0 {
            return Err(tar_error(code, &err));
        }
        copy::bring(b, p, id, &stage, to)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    r
}

pub fn compress(p: &mut Pump, id: Id, items: &[Loc], to: &Loc) -> Res<()> {
    let Loc::Local(out) = to else {
        return Err("Compress works on the card".into());
    };
    let mut names = Vec::new();
    let mut parent = None;
    for item in items {
        let Loc::Local(path) = item else {
            return Err("Compress works on the card".into());
        };
        parent = path.parent().map(Path::to_path_buf);
        names.push(format!(
            "./{}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
    }
    let parent = parent.ok_or("Nothing to compress")?;
    if out.symlink_metadata().is_ok() {
        return Err("That name is taken".into());
    }
    let mut cmd = bsdtar();
    cmd.arg("-a")
        .arg("-cf")
        .arg(out)
        .arg("-C")
        .arg(&parent)
        .args(&names);
    let r = run(p, &mut cmd, None, Some(id), &mut |_, _| Ok(()));
    match r {
        Ok((0, _, _)) => Ok(()),
        Ok((code, _, err)) => {
            let _ = std::fs::remove_file(out);
            Err(tar_error(code, &err))
        }
        Err(e) => {
            let _ = std::fs::remove_file(out);
            Err(if e == STOPPED {
                e
            } else {
                "bsdtar isn't installed".into()
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_drops_the_shared_folders() {
        assert_eq!(strip(&["backup/notes.txt".into()]), 1);
        assert_eq!(strip(&["backup/a".into(), "backup/sub/".into()]), 1);
        assert_eq!(strip(&["a.txt".into(), "docs".into()]), 0);
        assert_eq!(strip(&[]), 0);
        assert_eq!(strip(&["x/y/z.txt".into(), "x/y/w".into()]), 2);
    }
}
