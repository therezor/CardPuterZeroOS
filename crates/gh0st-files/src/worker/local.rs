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
        // A private file stays private while it copies.
        let mut dst = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(mode)
                .open(to)
                .map_err(|e| words(&e))?
        };
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
        // All of it: a copy, a move or a delete must not stop short.
        model::list_local(Path::new(path), usize::MAX)
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

/// Moves `path` to the freedesktop Trash under `home`.
pub fn trash(home: &Path, path: &Path) -> Res<()> {
    match gh0st_core::trash::put(home, path) {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            Err("This drive has no Trash: delete it for good instead".into())
        }
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
            Err("Can't move this to the Trash".into())
        }
        Err(e) => Err(words(&e)),
    }
}

pub use gh0st_core::trash::numbered;

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
    fn the_trash_says_why_it_cant() {
        let dir = std::env::temp_dir().join(format!("gh0st-files-trash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            trash(&dir, Path::new("/")).unwrap_err(),
            "Can't move this to the Trash"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
