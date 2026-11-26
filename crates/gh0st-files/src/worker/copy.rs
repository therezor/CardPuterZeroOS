//! Copy, move and delete between any two places: the card and any server.
//! Folders go file by file; a name already there asks the user.
//!
//! A move deletes only what arrived: a skipped file keeps its source, and
//! the folders around it. Links inside a folder are not followed: on the
//! card they are made again as links, elsewhere they are left out. Only an
//! item the user picked may be a link that is copied through.

use super::local::{self, numbered};
use super::{Backends, Prog, Pump, Res, STOPPED, loc_path};
use crate::engine::{Ask, Id};
use crate::model::{Entry, Kind, Loc};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// How deep a folder copy goes.
const DEPTH: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Replace,
    Skip,
    Both,
}

fn parse_policy(s: &str) -> Option<(Policy, bool)> {
    let (word, all) = match s.split_once(' ') {
        Some((w, a)) => (w, a == "all"),
        None => (s, false),
    };
    let p = match word {
        "replace" => Policy::Replace,
        "skip" => Policy::Skip,
        "both" => Policy::Both,
        _ => return None,
    };
    Some((p, all))
}

/// A path as the system finally sees it, so a link and its target compare
/// equal. Missing parts stay as written.
fn canon(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    match (p.parent(), p.file_name()) {
        (Some(d), Some(n)) => canon(d).join(n),
        _ => p.to_path_buf(),
    }
}

/// `a` is `of`, or inside it.
fn inside(a: &Loc, of: &Loc) -> bool {
    match (a, of) {
        (Loc::Local(x), Loc::Local(y)) => canon(x).starts_with(canon(y)),
        (Loc::Remote { server: s, path: x }, Loc::Remote { server: t, path: y }) => {
            let (x, y) = (x.trim_end_matches('/'), y.trim_end_matches('/'));
            s == t && (x == y || x.starts_with(&format!("{y}/")) || y.is_empty())
        }
        _ => false,
    }
}

/// The same folder, links aside.
fn same(a: &Loc, b: &Loc) -> bool {
    inside(a, b) && inside(b, a)
}

/// Both on one filesystem, so a move is a rename.
fn same_place(a: &Loc, b: &Loc) -> bool {
    match (a, b) {
        (Loc::Local(x), Loc::Local(y)) => {
            let dev = |p: &Path| std::fs::symlink_metadata(p).ok().map(|m| m.dev());
            let x_dev = x.parent().and_then(dev);
            x_dev.is_some() && x_dev == dev(y)
        }
        (Loc::Remote { server: s, .. }, Loc::Remote { server: t, .. }) => s == t,
        _ => false,
    }
}

fn leaf(l: &Loc) -> Res<String> {
    l.leaf().ok_or_else(|| "Nothing to copy".to_string())
}

fn is_link(e: &Entry) -> bool {
    matches!(e.kind, Kind::Link | Kind::LinkDir)
}

/// Bytes in `l`, all the way down. Links count only when followed.
fn size_tree(
    b: &mut Backends,
    p: &mut Pump,
    id: Id,
    l: &Loc,
    depth: u32,
    follow: bool,
) -> Res<u64> {
    p.wait(&[], 0);
    if p.cancelled(id) {
        return Err(STOPPED.into());
    }
    let Some(e) = b.stat(p, l)? else {
        return Ok(0);
    };
    if is_link(&e) && !follow {
        return Ok(0);
    }
    if !e.kind.is_dir() || depth >= DEPTH {
        return Ok(e.size);
    }
    let mut n = 0;
    for c in b.list(p, l)? {
        n += size_tree(b, p, id, &l.join(&c.name), depth + 1, false)?;
    }
    Ok(n)
}

/// A name in `dir` that isn't taken: "a 2.txt", "a 3.txt".
fn free_name(b: &mut Backends, p: &mut Pump, dir: &Loc, name: &str) -> Res<Loc> {
    for n in 2..1000 {
        let l = dir.join(&numbered(name, n));
        if b.stat(p, &l)?.is_none() {
            return Ok(l);
        }
    }
    Err("No free name".into())
}

/// One copy or move under way.
struct Run<'a> {
    id: Id,
    prog: &'a mut Prog,
    /// The answer chosen for every clash.
    all: Option<Policy>,
    remove: bool,
}

impl Run<'_> {
    /// What to do with `name` already in the target.
    fn conflict(&mut self, p: &mut Pump, name: &str) -> Res<Policy> {
        if let Some(x) = self.all {
            return Ok(x);
        }
        let answer = p
            .ask(self.id, Ask::Conflict { name: name.into() })
            .ok_or(STOPPED)?;
        let (x, for_all) = parse_policy(&answer).ok_or(STOPPED)?;
        if for_all {
            self.all = Some(x);
        }
        Ok(x)
    }
}

/// Copies `items` into `to`; with `remove`, a move. `follow` copies
/// through an item that is itself a link (not for a move, not for what an
/// archive brought).
pub fn copy(
    b: &mut Backends,
    p: &mut Pump,
    id: Id,
    items: &[Loc],
    to: &Loc,
    remove: bool,
    follow: bool,
) -> Res<()> {
    let follow = follow && !remove;
    let mut total = 0;
    for item in items {
        if inside(to, item) {
            return Err("A folder can't go inside itself".into());
        }
        total += size_tree(b, p, id, item, 0, follow)?;
    }
    let mut prog = Prog::new(id, total);
    let mut run = Run {
        id,
        prog: &mut prog,
        all: None,
        remove,
    };
    for item in items {
        if p.cancelled(id) {
            return Err(STOPPED.into());
        }
        let name = leaf(item)?;
        let parent = item.parent_folder();
        if parent.as_ref().is_some_and(|d| same(d, to)) {
            if remove {
                return Err("It is in that folder already".into());
            }
            // A copy into its own folder: "photo 2.png" beside it.
            let fresh = leaf(&free_name(b, p, to, &name)?)?;
            node(b, p, &mut run, item, to, &fresh, 0, follow)?;
            continue;
        }
        if remove && same_place(item, to) {
            move_by_rename(b, p, &mut run, item, to, &name)?;
            continue;
        }
        node(b, p, &mut run, item, to, &name, 0, follow)?;
    }
    Ok(())
}

/// A move on one filesystem: a rename, unless a folder meets a folder
/// (they merge) or the rename crosses a mount after all.
fn move_by_rename(
    b: &mut Backends,
    p: &mut Pump,
    run: &mut Run,
    item: &Loc,
    to: &Loc,
    name: &str,
) -> Res<()> {
    let size = size_tree(b, p, run.id, item, 0, false)?;
    let mut target = to.join(name);
    if let Some(there) = b.stat(p, &target)? {
        let src = b.stat(p, item)?.ok_or("It isn't there any more")?;
        match run.conflict(p, name)? {
            Policy::Skip => return run.prog.add(p, size),
            Policy::Both => target = free_name(b, p, to, name)?,
            Policy::Replace if there.kind == Kind::Dir && src.kind == Kind::Dir => {
                // Merge, as a copy would: only what arrives leaves.
                merge(b, p, run, item, &target, 0)?;
                return Ok(());
            }
            Policy::Replace => {
                if inside(item, &target) {
                    return Err(format!(
                        "Replacing \u{201c}{name}\u{201d} would delete what moves"
                    ));
                }
                // As root, the walk below decides whose each step is.
                if let (Loc::Local(a), Loc::Local(t), None) = (item, &target, &b.root) {
                    if there.kind != Kind::Dir {
                        // One step: the old file goes as the new one lands.
                        run.prog.file(p, name);
                        std::fs::rename(a, t).map_err(|e| local::words(&e))?;
                        return run.prog.add(p, size);
                    }
                }
                delete_tree(b, p, &target, 0)?;
            }
        }
    }
    run.prog.file(p, name);
    let (fs, from) = b.fs(p, item)?;
    match fs.rename(p, &from, &loc_path(&target)) {
        Ok(()) => run.prog.add(p, size),
        // Two mounts on one device: copy, then delete what arrived.
        Err(e) if e == "That's on another drive" => {
            let name = leaf(&target)?;
            node(b, p, run, item, to, &name, 0, false).map(|_| ())
        }
        Err(e) => Err(e),
    }
}

/// Copies `src` to `dir/name`. True when all of it arrived, so a move may
/// delete it.
#[allow(clippy::too_many_arguments)]
fn node(
    b: &mut Backends,
    p: &mut Pump,
    run: &mut Run,
    src: &Loc,
    dir: &Loc,
    name: &str,
    depth: u32,
    follow: bool,
) -> Res<bool> {
    if depth >= DEPTH {
        return Err("Folders nest too deep".into());
    }
    let e = b
        .stat(p, src)?
        .ok_or_else(|| format!("\u{201c}{name}\u{201d} isn't there any more"))?;
    if is_link(&e) && !follow {
        return link(b, p, run, src, dir, name, &e);
    }
    let mut target = dir.join(name);
    let mut onto = false;
    if let Some(there) = b.stat(p, &target)? {
        match run.conflict(p, name)? {
            Policy::Skip => {
                let size = size_tree(b, p, run.id, src, depth, follow)?;
                run.prog.add(p, size)?;
                return Ok(false);
            }
            Policy::Both => target = free_name(b, p, dir, name)?,
            Policy::Replace if there.kind.is_dir() && e.kind.is_dir() => onto = true,
            Policy::Replace => {
                if inside(src, &target) {
                    return Err(format!(
                        "Replacing \u{201c}{name}\u{201d} would delete what is copied"
                    ));
                }
                delete_tree(b, p, &target, 0)?;
            }
        }
    }
    if e.kind.is_dir() {
        if !onto {
            let (fs, path) = b.fs(p, &target)?;
            fs.mkdir(p, &path)?;
        }
        return merge(b, p, run, src, &target, depth);
    }
    if e.kind == Kind::Other {
        // Devices and pipes have no bytes to copy, and stay.
        return Ok(false);
    }
    run.prog.file(p, name);
    transfer(b, p, run.prog, src, &target)?;
    if run.remove {
        let (fs, path) = b.fs(p, src)?;
        fs.remove(p, &path, false)?;
    }
    Ok(true)
}

/// Everything in the folder `src` into the folder `target`. A move then
/// removes `src`, if all of it arrived.
fn merge(
    b: &mut Backends,
    p: &mut Pump,
    run: &mut Run,
    src: &Loc,
    target: &Loc,
    depth: u32,
) -> Res<bool> {
    let mut whole = true;
    for c in b.list(p, src)? {
        let done = node(
            b,
            p,
            run,
            &src.join(&c.name),
            target,
            &c.name,
            depth + 1,
            false,
        )?;
        whole &= done;
    }
    if run.remove && whole {
        let real_dir = b.stat(p, src)?.is_some_and(|e| e.kind == Kind::Dir);
        if real_dir {
            let (fs, path) = b.fs(p, src)?;
            fs.remove(p, &path, true)?;
        }
    }
    Ok(whole)
}

/// A link inside a folder: made again on the card, left out elsewhere.
fn link(
    b: &mut Backends,
    p: &mut Pump,
    run: &mut Run,
    src: &Loc,
    dir: &Loc,
    name: &str,
    e: &Entry,
) -> Res<bool> {
    let (Loc::Local(from), Loc::Local(_)) = (src, dir) else {
        return Ok(false);
    };
    let to_where = e.link.clone().or_else(|| {
        std::fs::read_link(from)
            .ok()
            .map(|t| t.to_string_lossy().into_owned())
    });
    let Some(points) = to_where else {
        return Ok(false);
    };
    let mut target = dir.join(name);
    if b.stat(p, &target)?.is_some() {
        match run.conflict(p, name)? {
            Policy::Skip => return Ok(false),
            Policy::Both => target = free_name(b, p, dir, name)?,
            Policy::Replace => delete_tree(b, p, &target, 0)?,
        }
    }
    let Loc::Local(t) = &target else {
        return Ok(false);
    };
    // As root, each as whoever its folder belongs to: a link is made, never
    // followed.
    b.act(t, || {
        std::os::unix::fs::symlink(&points, t).map_err(|e| local::words(&e))
    })?;
    if run.remove {
        b.act(from, || {
            std::fs::remove_file(from).map_err(|e| local::words(&e))
        })?;
    }
    Ok(true)
}

/// One file from `src` to `dst`, staged on the card between two servers.
/// A failed or stopped copy leaves no half file behind.
fn transfer(b: &mut Backends, p: &mut Pump, prog: &mut Prog, src: &Loc, dst: &Loc) -> Res<()> {
    match (src, dst) {
        (_, Loc::Local(to)) => {
            let (fs, path) = b.fs(p, src)?;
            fs.get(p, &path, to, prog)
        }
        (Loc::Local(from), _) => {
            let (fs, path) = b.fs(p, dst)?;
            let r = fs.put(p, from, &path, prog);
            if r.is_err() {
                p.hold_cancel = true;
                let _ = fs.remove(p, &path, false);
                p.hold_cancel = false;
            }
            r
        }
        _ => {
            let stage = b
                .stage()
                .join(format!("copy-{}-{}", std::process::id(), prog.id));
            let r = (|| {
                // The bar counts the way up.
                let mut down = Prog::silent(prog.id);
                let (fs, path) = b.fs(p, src)?;
                fs.get(p, &path, &stage, &mut down)?;
                let (fs, path) = b.fs(p, dst)?;
                let r = fs.put(p, &stage, &path, prog);
                if r.is_err() {
                    p.hold_cancel = true;
                    let _ = fs.remove(p, &path, false);
                    p.hold_cancel = false;
                }
                r
            })();
            let _ = std::fs::remove_file(&stage);
            r
        }
    }
}

/// Removes `l` and everything in it; a link goes, not what it points to.
/// Gone already is fine.
pub fn delete_tree(b: &mut Backends, p: &mut Pump, l: &Loc, depth: u32) -> Res<()> {
    let Some(e) = b.stat(p, l)? else {
        return Ok(());
    };
    if let Loc::Local(path) = l {
        // The Trash's own files have info files beside them.
        drop_trash_info(&b.home, path);
    }
    if e.kind != Kind::Dir {
        let (fs, path) = b.fs(p, l)?;
        return fs.remove(p, &path, false);
    }
    {
        let (fs, path) = b.fs(p, l)?;
        if let Some(r) = fs.remove_tree(p, &path) {
            return r;
        }
    }
    if depth >= DEPTH {
        return Err("Folders nest too deep".into());
    }
    for c in b.list(p, l)? {
        if p.cancelled(p.current.as_ref().map_or(0, |c| c.0)) {
            return Err(STOPPED.into());
        }
        delete_tree(b, p, &l.join(&c.name), depth + 1)?;
    }
    let (fs, path) = b.fs(p, l)?;
    fs.remove(p, &path, true)
}

fn drop_trash_info(home: &Path, path: &Path) {
    let files = home.join(".local/share/Trash/files");
    if path.parent() == Some(files.as_path()) {
        if let Some(n) = path.file_name() {
            let info = home
                .join(".local/share/Trash/info")
                .join(format!("{}.trashinfo", n.to_string_lossy()));
            let _ = std::fs::remove_file(info);
        }
    }
}

/// Used by extract: a staged local tree moved into `to`. Links in it are
/// never followed: an archive can't reach past what it holds.
pub fn bring(b: &mut Backends, p: &mut Pump, id: Id, stage: &Path, to: &Loc) -> Res<()> {
    let mut items = Vec::new();
    for de in std::fs::read_dir(stage).map_err(|e| local::words(&e))? {
        let de = de.map_err(|e| local::words(&e))?;
        items.push(Loc::Local(de.path()));
    }
    copy(b, p, id, &items, to, true, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_parse_with_all() {
        assert_eq!(parse_policy("replace"), Some((Policy::Replace, false)));
        assert_eq!(parse_policy("both all"), Some((Policy::Both, true)));
        assert_eq!(parse_policy("nope"), None);
    }

    #[test]
    fn inside_reads_servers_and_links() {
        let r = |p: &str| Loc::Remote {
            server: "s".into(),
            path: p.into(),
        };
        assert!(inside(&r("/a/b"), &r("/a")));
        assert!(inside(&r("/a"), &r("/a/")));
        assert!(!inside(&r("/ab"), &r("/a")));
        let dir = std::env::temp_dir().join(format!("gh0st-files-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink("real", dir.join("link")).unwrap();
        assert!(same(
            &Loc::Local(dir.join("link")),
            &Loc::Local(dir.join("real"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
