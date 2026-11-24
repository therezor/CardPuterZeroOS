//! Copy, move and delete between any two places: the card and any server.
//! Folders go file by file; a name already there asks the user.

use super::local::{self, numbered};
use super::{Backends, Prog, Pump, Res, STOPPED, loc_path};
use crate::engine::{Ask, Id};
use crate::model::{Entry, Kind, Loc};
use std::os::unix::fs::MetadataExt;

/// How deep a folder copy goes: links that loop stop here.
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

/// Both on one filesystem, so a move is a rename.
fn same_place(a: &Loc, b: &Loc) -> bool {
    match (a, b) {
        (Loc::Local(x), Loc::Local(y)) => {
            let dev = |p: &std::path::Path| {
                p.parent()
                    .and_then(|d| std::fs::metadata(d).ok())
                    .map(|m| m.dev())
            };
            let y_dev = std::fs::metadata(y).ok().map(|m| m.dev());
            dev(x).is_some() && dev(x) == y_dev
        }
        (Loc::Remote { server: s, .. }, Loc::Remote { server: t, .. }) => s == t,
        _ => false,
    }
}

fn leaf(l: &Loc) -> Res<String> {
    l.leaf().ok_or_else(|| "Nothing to copy".to_string())
}

/// Bytes in `l`, all the way down.
fn size_tree(b: &mut Backends, p: &mut Pump, l: &Loc, depth: u32) -> Res<u64> {
    let Some(e) = b.stat(p, l)? else {
        return Ok(0);
    };
    if !e.kind.is_dir() || depth >= DEPTH {
        return Ok(e.size);
    }
    let mut n = 0;
    for c in b.list(p, l)? {
        n += size_tree(b, p, &l.join(&c.name), depth + 1)?;
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

/// What to do with `name` already in the target: the policy chosen for
/// all, or the user's answer.
fn conflict(p: &mut Pump, id: Id, name: &str, all: &mut Option<Policy>) -> Res<Policy> {
    if let Some(x) = *all {
        return Ok(x);
    }
    let answer = p
        .ask(id, Ask::Conflict { name: name.into() })
        .ok_or(STOPPED)?;
    let (x, for_all) = parse_policy(&answer).ok_or(STOPPED)?;
    if for_all {
        *all = Some(x);
    }
    Ok(x)
}

pub fn copy(
    b: &mut Backends,
    p: &mut Pump,
    id: Id,
    items: &[Loc],
    to: &Loc,
    remove: bool,
) -> Res<()> {
    let mut total = 0;
    for item in items {
        total += size_tree(b, p, item, 0)?;
    }
    let mut prog = Prog::new(id, total);
    let mut all = None;
    for item in items {
        if p.cancelled(id) {
            return Err(STOPPED.into());
        }
        let name = leaf(item)?;
        if item.parent_folder().as_ref() == Some(to) {
            return Err("It is in that folder already".into());
        }
        if let (Loc::Local(a), Loc::Local(t)) = (item, to) {
            if t.starts_with(a) {
                return Err("A folder can't go inside itself".into());
            }
        }
        if remove && same_place(item, to) {
            move_by_rename(b, p, &mut prog, item, to, &name, &mut all)?;
            continue;
        }
        copy_tree(b, p, &mut prog, item, to, &name, &mut all, 0)?;
        if remove {
            delete_tree(b, p, item, 0)?;
        }
    }
    Ok(())
}

fn move_by_rename(
    b: &mut Backends,
    p: &mut Pump,
    prog: &mut Prog,
    item: &Loc,
    to: &Loc,
    name: &str,
    all: &mut Option<Policy>,
) -> Res<()> {
    let size = size_tree(b, p, item, 0)?;
    let mut target = to.join(name);
    if b.stat(p, &target)?.is_some() {
        match conflict(p, prog.id, name, all)? {
            Policy::Skip => return prog.add(p, size),
            Policy::Both => target = free_name(b, p, to, name)?,
            Policy::Replace => delete_tree(b, p, &target, 0)?,
        }
    }
    prog.file(p, name);
    let (fs, from) = b.fs(p, item)?;
    fs.rename(p, &from, &loc_path(&target))?;
    prog.add(p, size)
}

#[allow(clippy::too_many_arguments)]
fn copy_tree(
    b: &mut Backends,
    p: &mut Pump,
    prog: &mut Prog,
    src: &Loc,
    dir: &Loc,
    name: &str,
    all: &mut Option<Policy>,
    depth: u32,
) -> Res<()> {
    if depth >= DEPTH {
        return Err("Folders nest too deep: a link may loop".into());
    }
    let e = b
        .stat(p, src)?
        .ok_or_else(|| format!("\u{201c}{name}\u{201d} isn't there any more"))?;
    let mut target = dir.join(name);
    let mut merge = false;
    if let Some(there) = b.stat(p, &target)? {
        match conflict(p, prog.id, name, all)? {
            Policy::Skip => {
                let size = size_tree(b, p, src, depth)?;
                return prog.add(p, size);
            }
            Policy::Both => target = free_name(b, p, dir, name)?,
            Policy::Replace if there.kind.is_dir() && e.kind.is_dir() => merge = true,
            Policy::Replace => delete_tree(b, p, &target, 0)?,
        }
    }
    if e.kind.is_dir() {
        if !merge {
            let (fs, path) = b.fs(p, &target)?;
            fs.mkdir(p, &path)?;
        }
        for c in b.list(p, src)? {
            copy_tree(
                b,
                p,
                prog,
                &src.join(&c.name),
                &target,
                &c.name,
                all,
                depth + 1,
            )?;
        }
        return Ok(());
    }
    if e.kind == Kind::Other {
        // Devices and pipes have no bytes to copy.
        return Ok(());
    }
    prog.file(p, name);
    transfer(b, p, prog, src, &target, &e)
}

/// One file from `src` to `dst`, staged on the card between two servers.
fn transfer(
    b: &mut Backends,
    p: &mut Pump,
    prog: &mut Prog,
    src: &Loc,
    dst: &Loc,
    e: &Entry,
) -> Res<()> {
    match (src, dst) {
        (_, Loc::Local(to)) => {
            let (fs, path) = b.fs(p, src)?;
            fs.get(p, &path, to, prog)
        }
        (Loc::Local(from), _) => {
            let (fs, path) = b.fs(p, dst)?;
            let r = fs.put(p, from, &path, prog);
            if r.is_err() {
                let _ = fs.remove(p, &path, false);
            }
            r
        }
        _ => {
            let stage = b.stage().join(format!("copy-{}", prog.id));
            let r = (|| {
                // The bar counts the way up.
                let _ = e;
                let mut down = Prog::silent(prog.id);
                let (fs, path) = b.fs(p, src)?;
                fs.get(p, &path, &stage, &mut down)?;
                let (fs, path) = b.fs(p, dst)?;
                let r = fs.put(p, &stage, &path, prog);
                if r.is_err() {
                    let _ = fs.remove(p, &path, false);
                }
                r
            })();
            let _ = std::fs::remove_file(&stage);
            r
        }
    }
}

/// Removes `l` and everything in it. Gone already is fine.
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

fn drop_trash_info(home: &std::path::Path, path: &std::path::Path) {
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

/// Used by extract: a staged local tree moved or copied into `to`.
pub fn bring(b: &mut Backends, p: &mut Pump, id: Id, stage: &std::path::Path, to: &Loc) -> Res<()> {
    let mut items = Vec::new();
    for de in std::fs::read_dir(stage).map_err(|e| local::words(&e))? {
        let de = de.map_err(|e| local::words(&e))?;
        items.push(Loc::Local(de.path()));
    }
    copy(b, p, id, &items, to, true)
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
}
