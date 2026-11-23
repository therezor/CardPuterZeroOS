//! One-time migrations: changes an update makes once on each device, then
//! never again. A package ships them as files in [`DIR`], one per change,
//! run in name order. Each line is a step:
//!
//! ```text
//! # comment
//! install curl wget python3
//! ```
//!
//! `install` adds packages a device flashed before them lacks. They are
//! installed by hand, as apt sees it, so the user may remove them later and
//! the migration won't bring them back. A migration is done once all its
//! steps hold; its name then goes in [`done_path`]. A fresh install of
//! gh0st-base marks every migration it ships done, since a new image has
//! them already (docs/updates.md).

use gh0st_core::update;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const DIR: &str = "/usr/share/gh0st/migrations";

/// Where the migrations are, or `$GH0ST_MIGRATIONS` for tests.
pub fn dir() -> PathBuf {
    std::env::var_os("GH0ST_MIGRATIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DIR))
}

/// The names of the migrations done, one a line.
pub fn done_path() -> PathBuf {
    update::state_dir().join("migrations")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Migration {
    pub id: String,
    pub install: Vec<String>,
}

pub fn parse(id: &str, text: &str) -> Migration {
    let mut m = Migration {
        id: id.into(),
        ..Migration::default()
    };
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() == Some("install") {
            m.install.extend(words.map(String::from));
        }
    }
    m
}

/// The migrations in `dir` not named in `done`, in name order.
pub fn pending(dir: &Path, done: &Path) -> Vec<Migration> {
    let done = std::fs::read_to_string(done).unwrap_or_default();
    let done: HashSet<&str> = done.lines().map(str::trim).collect();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<Migration> = rd
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            if id.starts_with('.') || done.contains(id.as_str()) {
                return None;
            }
            Some(parse(&id, &std::fs::read_to_string(e.path()).ok()?))
        })
        .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// The packages `pending` still wants, each once, in order.
pub fn missing(pending: &[Migration], installed: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pending.iter().flat_map(|m| &m.install) {
        if !installed.contains(p) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

/// The migrations of `pending` whose steps all hold.
pub fn complete<'a>(pending: &'a [Migration], installed: &HashSet<String>) -> Vec<&'a str> {
    pending
        .iter()
        .filter(|m| m.install.iter().all(|p| installed.contains(p)))
        .map(|m| m.id.as_str())
        .collect()
}

/// Adds `ids` to the done list.
pub fn mark_done(done: &Path, ids: &[&str]) -> std::io::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut text = std::fs::read_to_string(done).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for id in ids {
        if !text.lines().any(|l| l.trim() == *id) {
            text.push_str(id);
            text.push('\n');
        }
    }
    if let Some(d) = done.parent() {
        std::fs::create_dir_all(d)?;
    }
    update::write_atomic(done, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_migration_runs_once_and_only_for_what_is_missing() {
        let root = std::env::temp_dir().join(format!("gh0st-migrate-{}", std::process::id()));
        let dir = root.join("migrations");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("0002-more"),
            "install jq curl\n# comment\nlater x\n",
        )
        .unwrap();
        std::fs::write(dir.join("0001-tools"), "install curl wget\n").unwrap();
        let done = root.join("state/migrations");

        let p = pending(&dir, &done);
        assert_eq!(
            p.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["0001-tools", "0002-more"]
        );
        let have = set(&["wget"]);
        assert_eq!(missing(&p, &have), ["curl", "jq"]);
        assert!(complete(&p, &have).is_empty());

        let have = set(&["wget", "curl"]);
        assert_eq!(complete(&p, &have), ["0001-tools"]);
        mark_done(&done, &complete(&p, &have)).unwrap();
        mark_done(&done, &["0001-tools"]).unwrap();
        assert_eq!(std::fs::read_to_string(&done).unwrap(), "0001-tools\n");

        let p = pending(&dir, &done);
        assert_eq!(p.len(), 1);
        assert_eq!(missing(&p, &have), ["jq"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
