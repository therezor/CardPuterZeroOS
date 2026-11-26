//! Root mode: the worker gh0st-helper starts as root after the user's
//! password. It serves local files only. In the user's own folders it acts
//! as the user, so files it makes there are the user's, and a program of
//! the user's can't swap a folder under it to reach what only root may.
//!
//! What is whose is decided per call, from the path's folders: a folder
//! owned by the user, or one its group or anyone may write to, makes all
//! below it the user's, because the user could change what is there. A
//! folder only root can change can't be swapped mid-walk, so root acts
//! there.

use super::local::{self, Local};
use super::{Fs, Prog, Pump, Res};
use crate::model::Entry;
use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The environment variable gh0st-helper passes: the user's `uid:gid`.
pub const ENV: &str = "GH0ST_FILES_ROOT";

/// Links followed while deciding whose a path is, before giving up.
const MAX_LINKS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Root,
    User,
}

/// Whose `path` is, for a user `uid`: decided by every folder it is in,
/// from `/` down to its own folder, read with lstat. The path itself may not
/// exist yet, and its own owner doesn't count: changing a name is its
/// folder's say.
pub fn whose(path: &Path, uid: u32) -> Who {
    whose_in(path, uid, 0)
}

fn whose_in(path: &Path, uid: u32, links: u32) -> Who {
    if links > MAX_LINKS {
        return Who::User;
    }
    let mut at = PathBuf::from("/");
    let mut parts = path.components().skip(1).peekable();
    while let Some(c) = parts.next() {
        if !theirs_cant_change(&at, uid) {
            return Who::User;
        }
        let next = at.join(c);
        let Ok(m) = std::fs::symlink_metadata(&next) else {
            // Not there yet: its folder decides, and it was root's.
            return Who::Root;
        };
        if m.file_type().is_symlink() && parts.peek().is_some() {
            // A link only root could have put here: whose is where it goes.
            let Ok(to) = std::fs::read_link(&next) else {
                return Who::User;
            };
            let mut rest = at.join(to);
            rest.extend(parts);
            return whose_in(&normal(&rest), uid, links + 1);
        }
        at = next;
    }
    Who::Root
}

/// True when only root can change what is in folder `dir`.
fn theirs_cant_change(dir: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(dir)
        .is_ok_and(|m| m.uid() != uid && m.mode() & 0o022 == 0 && m.uid() == 0)
}

/// `a/./b/../c` as `a/c`, without touching the disk.
fn normal(p: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(n) => out.push(n),
            _ => {}
        }
    }
    out
}

/// The user the worker acts for: ids and groups, read once at start.
pub struct Ident {
    pub uid: u32,
    gid: u32,
    groups: Vec<libc::gid_t>,
}

impl Ident {
    /// From `uid:gid`. The groups come from the password database, as at
    /// a login.
    pub fn parse(s: &str) -> Option<Ident> {
        let (u, g) = s.split_once(':')?;
        let (uid, gid) = (u.parse().ok()?, g.parse().ok()?);
        if uid == 0 {
            return None;
        }
        let name = gh0st_core::password::user_name(uid)?;
        Some(Ident {
            uid,
            gid,
            groups: groups_of(&name, gid),
        })
    }

    /// Runs `f` as `who`. As the user, the worker takes the user's ids and
    /// groups for the call and root's back after; a switch that fails runs
    /// nothing, and one back that fails ends the worker.
    pub fn run<T>(&self, who: Who, f: impl FnOnce() -> Res<T>) -> Res<T> {
        if who == Who::Root {
            return f();
        }
        // SAFETY: plain id syscalls on this single-threaded process; the
        // groups slice outlives the call.
        let down = unsafe {
            libc::setgroups(self.groups.len() as _, self.groups.as_ptr()) == 0
                && libc::setegid(self.gid) == 0
                && libc::seteuid(self.uid) == 0
        };
        let r = if down {
            f()
        } else {
            Err("Couldn't act as you".into())
        };
        // SAFETY: as above; root's own ids and its one group back.
        let up = unsafe {
            libc::seteuid(0) == 0 && libc::setegid(0) == 0 && libc::setgroups(1, [0].as_ptr()) == 0
        };
        if !up {
            std::process::abort();
        }
        r
    }

    /// For a name: the folders it is in decide.
    fn who_in(&self, path: &Path) -> Who {
        whose(path, self.uid)
    }

    /// For what is inside a folder: the folder itself counts too.
    fn who_inside(&self, dir: &Path) -> Who {
        whose(&dir.join(".gh0st"), self.uid)
    }
}

fn groups_of(name: &str, gid: u32) -> Vec<libc::gid_t> {
    let Ok(c) = CString::new(name) else {
        return vec![gid];
    };
    let mut n: libc::c_int = 64;
    let mut v: Vec<libc::gid_t> = vec![0; n as usize];
    // SAFETY: getgrouplist fills at most n entries of v and says how many.
    let r = unsafe { libc::getgrouplist(c.as_ptr(), gid as _, v.as_mut_ptr().cast(), &mut n) };
    if r < 0 {
        return vec![gid];
    }
    v.truncate(n.max(0) as usize);
    v
}

/// The card, as root, with the user's folders as the user.
pub struct RootLocal {
    pub ident: Ident,
    local: Local,
}

impl RootLocal {
    pub fn new(ident: Ident) -> RootLocal {
        RootLocal {
            ident,
            local: Local,
        }
    }

    /// Up to `max` bytes of a plain file, for the viewer.
    pub fn read(&self, path: &Path, max: u64) -> Res<Vec<u8>> {
        let file = self.ident.run(self.ident.who_in(path), || {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                .open(path)
                .map_err(|e| local::words(&e))
        })?;
        // Checked on the file opened, not the name: a pipe would block.
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Err("Only a plain file opens here".into());
        }
        let mut buf = Vec::new();
        file.take(max)
            .read_to_end(&mut buf)
            .map_err(|e| local::words(&e))?;
        Ok(buf)
    }

    /// One file, the source opened as whoever its folder belongs to and
    /// the copy made as whoever the target's does. A failed copy leaves no
    /// half file.
    fn copy(&self, p: &mut Pump, from: &Path, to: &Path, prog: &mut Prog) -> Res<()> {
        let id = &self.ident;
        // A link is copied as a link by the walk; here one is refused, not
        // followed.
        let mut src = id.run(id.who_in(from), || {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(from)
                .map_err(|e| local::words(&e))
        })?;
        if !src.metadata().is_ok_and(|m| m.is_file()) {
            return Err("Only plain files copy as root".into());
        }
        let mode = src.metadata().map(|m| m.mode() & 0o777).unwrap_or(0o644);
        let made = id.who_in(to);
        let mut dst = id.run(made, || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .mode(mode)
                .open(to)
                .map_err(|e| local::words(&e))
        })?;
        let r = (|| {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = src.read(&mut buf).map_err(|e| local::words(&e))?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).map_err(|e| local::words(&e))?;
                prog.add(p, n as u64)?;
            }
            dst.sync_all().map_err(|e| local::words(&e))?;
            let _ = dst.set_permissions(std::fs::Permissions::from_mode(mode));
            Ok(())
        })();
        if r.is_err() {
            let _ = id.run(made, || {
                std::fs::remove_file(to).map_err(|e| local::words(&e))
            });
        }
        r
    }
}

impl Fs for RootLocal {
    fn list(&mut self, p: &mut Pump, path: &str) -> Res<Vec<Entry>> {
        let who = self.ident.who_inside(Path::new(path));
        let local = &mut self.local;
        self.ident.run(who, || local.list(p, path))
    }

    fn stat(&mut self, p: &mut Pump, path: &str) -> Res<Option<Entry>> {
        let who = self.ident.who_in(Path::new(path));
        let local = &mut self.local;
        self.ident.run(who, || local.stat(p, path))
    }

    fn get(&mut self, p: &mut Pump, path: &str, to: &Path, prog: &mut Prog) -> Res<()> {
        self.copy(p, Path::new(path), to, prog)
    }

    fn put(&mut self, p: &mut Pump, from: &Path, path: &str, prog: &mut Prog) -> Res<()> {
        self.copy(p, from, Path::new(path), prog)
    }

    fn mkdir(&mut self, p: &mut Pump, path: &str) -> Res<()> {
        let who = self.ident.who_in(Path::new(path));
        let local = &mut self.local;
        self.ident.run(who, || local.mkdir(p, path))
    }

    fn remove(&mut self, p: &mut Pump, path: &str, dir: bool) -> Res<()> {
        let who = self.ident.who_in(Path::new(path));
        let local = &mut self.local;
        self.ident.run(who, || local.remove(p, path, dir))
    }

    fn rename(&mut self, p: &mut Pump, from: &str, to: &str) -> Res<()> {
        let (a, b) = (
            self.ident.who_in(Path::new(from)),
            self.ident.who_in(Path::new(to)),
        );
        // Two owners: neither may do both halves, so the walk copies (each
        // side as its owner) and then removes the source as its owner.
        if a != b {
            return Err("That's on another drive".into());
        }
        let local = &mut self.local;
        self.ident.run(a, || local.rename(p, from, to))
    }

    // No one-step tree removal: the walk removes each entry as whoever its
    // folder belongs to.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_and_doubles_fold_away() {
        assert_eq!(normal(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normal(Path::new("/../x")), PathBuf::from("/x"));
    }

    /// The host's own tree: `/` and `/usr` are root's and closed to others
    /// on any test machine, `/tmp` is open to all, and the test's own
    /// folders are its user's.
    #[test]
    fn whose_follows_the_folders_down() {
        // SAFETY: getuid has no arguments.
        let me = unsafe { libc::getuid() };
        if me == 0 {
            return;
        }
        assert_eq!(whose(Path::new("/usr/bin/env"), me), Who::Root);
        assert_eq!(whose(Path::new("/usr/no-such-thing"), me), Who::Root);
        // A name right in my home, under root's /home or /Users: my folder
        // decides, one level down from root's.
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let theirs = home.parent().is_some_and(|p| theirs_cant_change(p, me));
            let mine = std::fs::metadata(&home).is_ok_and(|m| m.uid() == me);
            if theirs && mine {
                assert_eq!(whose(&home.join("new"), me), Who::User, "right in my home");
                let ident = Ident {
                    uid: me,
                    gid: 0,
                    groups: Vec::new(),
                };
                assert_eq!(
                    ident.who_in(&home.join("new")),
                    Who::User,
                    "a new name in my home"
                );
                assert_eq!(ident.who_inside(&home), Who::User, "listing my home");
            }
        }
        let tmp = std::env::temp_dir().join(format!("gh0st-whose-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("mine")).unwrap();
        assert_eq!(
            whose(&tmp.join("mine/file"), me),
            Who::User,
            "below my own folder"
        );
        // A link in my folder pointing at root's tree is still under my
        // folder, which decides first.
        let _ = std::os::unix::fs::symlink("/usr/bin", tmp.join("mine/to-usr"));
        assert_eq!(whose(&tmp.join("mine/to-usr/env"), me), Who::User);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_root_link_is_followed_to_where_it_goes() {
        // SAFETY: getuid has no arguments.
        let me = unsafe { libc::getuid() };
        if me == 0 {
            return;
        }
        // On Linux /bin is a root link to /usr/bin; elsewhere it is a
        // folder. Either way it is root's.
        assert_eq!(whose(Path::new("/bin/sh"), me), Who::Root);
    }

    /// Only as root, as on the board: `sudo` the test binary. Nobody
    /// (65534) stands in for the user.
    #[test]
    fn as_root_the_users_folders_are_worked_as_the_user() {
        // SAFETY: geteuid has no arguments.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let ident = Ident {
            uid: 65534,
            gid: 65534,
            groups: vec![65534],
        };
        // /run is root's and closed to others, as /home is: the user's own
        // folder right inside it must count as the user's.
        let top = PathBuf::from(format!("/run/gh0st-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        std::fs::create_dir_all(top.join("theirs")).unwrap();
        std::fs::create_dir_all(top.join("open")).unwrap();
        std::fs::set_permissions(&top, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(top.join("open"), std::fs::Permissions::from_mode(0o777)).unwrap();
        let theirs = CString::new(top.join("theirs").to_string_lossy().as_bytes()).unwrap();
        // SAFETY: chown on a path this test made.
        unsafe { libc::chown(theirs.as_ptr(), 65534, 65534) };
        assert_eq!(whose(&top.join("theirs/f"), 65534), Who::User);
        assert_eq!(whose(&top.join("f"), 65534), Who::Root, "root's own folder");
        assert_eq!(whose(&top.join("open/f"), 65534), Who::User, "open to all");
        let mut root = RootLocal::new(ident);
        let mut p = Pump::new(crate::proto::Conn::new(crate::proto::pair().unwrap().0));
        let made = top.join("theirs/new");
        root.mkdir(&mut p, &made.to_string_lossy()).unwrap();
        assert_eq!(
            std::fs::metadata(&made).unwrap().uid(),
            65534,
            "made as the user"
        );
        // A root-only file: the user can't read it, so as the user the
        // worker can't either.
        let secret = top.join("open/secret");
        std::fs::write(&secret, "x").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            root.read(&secret, 10).is_err(),
            "in an open folder root acts as the user"
        );
        // Root's folder to the user's: neither may do both halves, so the
        // move copies and deletes instead.
        let mine = top.join("mine");
        std::fs::write(&mine, "x").unwrap();
        let to = top.join("theirs/moved");
        let r = root.rename(&mut p, &mine.to_string_lossy(), &to.to_string_lossy());
        assert_eq!(r.unwrap_err(), "That's on another drive");
        assert!(mine.exists() && !to.exists());
        // SAFETY: geteuid has no arguments.
        assert_eq!(unsafe { libc::geteuid() }, 0, "root again after");
        std::fs::remove_dir_all(&top).unwrap();
    }

    #[test]
    fn an_ident_for_root_itself_is_refused() {
        assert!(Ident::parse("0:0").is_none());
        assert!(Ident::parse("x:1").is_none());
        assert!(Ident::parse("1000").is_none());
    }
}
