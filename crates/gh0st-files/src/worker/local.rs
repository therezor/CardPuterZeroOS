//! The card: plain file calls, the freedesktop Trash, and the app's SSH key.

use super::{Fs, Prog, Pump, Res, run};
use crate::model::{self, Entry, Kind};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;

const CHUNK: usize = 64 * 1024;

pub struct Local;

/// An io error in the user's words.
pub fn words(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        PermissionDenied => "No permission".into(),
        NotFound => "It isn't there any more".into(),
        AlreadyExists => "That name is taken".into(),
        _ => match e.raw_os_error() {
            Some(libc::ENOSPC) => "The card is full".into(),
            Some(libc::EXDEV) => "That's on another drive".into(),
            Some(libc::ENOTEMPTY) => "The folder isn't empty".into(),
            Some(libc::EROFS) => "That drive is read only".into(),
            _ => {
                let s = e.to_string();
                s.split(" (os error").next().unwrap_or(&s).to_string()
            }
        },
    }
}

pub fn entry_of(name: &str, path: &Path) -> std::io::Result<Entry> {
    let l = std::fs::symlink_metadata(path)?;
    let ft = l.file_type();
    let (kind, m, link) = if ft.is_symlink() {
        let target = std::fs::read_link(path)
            .ok()
            .map(|t| t.to_string_lossy().into_owned());
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => (Kind::LinkDir, m, target),
            Ok(m) => (Kind::Link, m, target),
            Err(_) => (Kind::Link, l, target),
        }
    } else if ft.is_dir() {
        (Kind::Dir, l, None)
    } else if ft.is_file() {
        (Kind::File, l, None)
    } else {
        (Kind::Other, l, None)
    };
    Ok(Entry {
        name: name.into(),
        kind,
        size: if kind.is_dir() { 0 } else { m.len() },
        mtime: m.mtime(),
        mode: m.mode() & 0o7777,
        link,
    })
}

/// Copies one file in chunks, telling `prog`. A stopped or failed copy
/// leaves no half file.
pub fn copy_file(p: &mut Pump, from: &Path, to: &Path, prog: &mut Prog) -> Res<()> {
    let r = (|| {
        let mut src = std::fs::File::open(from).map_err(|e| words(&e))?;
        let mode = src.metadata().map(|m| m.mode() & 0o777).unwrap_or(0o644);
        let mut dst = std::fs::File::create(to).map_err(|e| words(&e))?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = src.read(&mut buf).map_err(|e| words(&e))?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).map_err(|e| words(&e))?;
            prog.add(p, n as u64)?;
        }
        dst.sync_all().map_err(|e| words(&e))?;
        let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode));
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(to);
    }
    r
}

impl Fs for Local {
    fn list(&mut self, _: &mut Pump, path: &str) -> Res<Vec<Entry>> {
        model::list_local(Path::new(path))
            .map(|(v, _)| v)
            .map_err(|e| words(&e))
    }

    fn stat(&mut self, _: &mut Pump, path: &str) -> Res<Option<Entry>> {
        let p = Path::new(path);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match entry_of(&name, p) {
            Ok(e) => Ok(Some(e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(words(&e)),
        }
    }

    fn get(&mut self, p: &mut Pump, path: &str, to: &Path, prog: &mut Prog) -> Res<()> {
        copy_file(p, Path::new(path), to, prog)
    }

    fn put(&mut self, p: &mut Pump, from: &Path, path: &str, prog: &mut Prog) -> Res<()> {
        copy_file(p, from, Path::new(path), prog)
    }

    fn mkdir(&mut self, _: &mut Pump, path: &str) -> Res<()> {
        std::fs::create_dir(path).map_err(|e| words(&e))
    }

    fn remove(&mut self, _: &mut Pump, path: &str, dir: bool) -> Res<()> {
        let r = if dir {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        };
        r.map_err(|e| words(&e))
    }

    fn rename(&mut self, _: &mut Pump, from: &str, to: &str) -> Res<()> {
        if Path::new(to).symlink_metadata().is_ok() {
            return Err("That name is taken".into());
        }
        std::fs::rename(from, to).map_err(|e| words(&e))
    }

    fn remove_tree(&mut self, _: &mut Pump, path: &str) -> Option<Res<()>> {
        Some(std::fs::remove_dir_all(path).map_err(|e| words(&e)))
    }
}

/// Moves `path` to the freedesktop Trash under `home`, with the
/// `.trashinfo` file managers read to put it back.
pub fn trash(home: &Path, path: &Path) -> Res<()> {
    let base = home.join(".local/share/Trash");
    let (files, info) = (base.join("files"), base.join("info"));
    std::fs::create_dir_all(&files).map_err(|e| words(&e))?;
    std::fs::create_dir_all(&info).map_err(|e| words(&e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("Can't move this to the Trash")?;
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
    std::fs::write(&info_file, body).map_err(|e| words(&e))?;
    if let Err(e) = std::fs::rename(path, files.join(&pick)) {
        let _ = std::fs::remove_file(&info_file);
        if e.raw_os_error() == Some(libc::EXDEV) {
            return Err("This drive has no Trash: delete it for good instead".into());
        }
        return Err(words(&e));
    }
    Ok(())
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

/// Makes `ssh/id_ed25519`, sealed with `passphrase`: ssh-keygen asks for it
/// through askpass, and the pump answers.
pub fn make_key(p: &mut Pump, dir: &Path, passphrase: &str) -> Res<()> {
    std::fs::create_dir_all(dir).map_err(|e| words(&e))?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let key = dir.join("id_ed25519");
    if key.exists() {
        return Err("There is a key already".into());
    }
    let askpass = p.askpass_path().ok_or("No askpass socket")?.to_path_buf();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("ssh-keygen");
    cmd.args(["-q", "-t", "ed25519", "-C", "gh0st-files", "-f"])
        .arg(&key)
        .env("SSH_ASKPASS", exe)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(super::askpass::ENV, askpass);
    detach(&mut cmd);
    p.auto = Some(passphrase.to_string());
    let r = run(p, &mut cmd, None, None, &mut |_, _| Ok(()));
    p.auto = None;
    let (code, _, err) = r?;
    if code != 0 {
        let _ = std::fs::remove_file(&key);
        return Err(format!("ssh-keygen failed: {}", err.trim()));
    }
    Ok(())
}

/// No terminal for the child: ssh and ssh-keygen must use askpass.
pub fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trash_keeps_names_and_info() {
        let dir = std::env::temp_dir().join(format!("gh0st-files-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(dir.join("a/x y.txt"), b"1").unwrap();
        trash(&dir, &dir.join("a/x y.txt")).unwrap();
        std::fs::write(dir.join("a/x y.txt"), b"2").unwrap();
        trash(&dir, &dir.join("a/x y.txt")).unwrap();
        let t = dir.join(".local/share/Trash");
        assert!(t.join("files/x y.txt").exists());
        assert!(t.join("files/x y 2.txt").exists());
        let info = std::fs::read_to_string(t.join("info/x y 2.txt.trashinfo")).unwrap();
        assert!(info.contains("/a/x%20y.txt"), "{info}");
        assert!(info.contains("DeletionDate="));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered("a.tar", 2), "a 2.tar");
        assert_eq!(numbered("notes", 3), "notes 3");
        assert_eq!(numbered(".bashrc", 2), ".bashrc 2");
    }
}
