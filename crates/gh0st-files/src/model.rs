//! What a side shows: where it is, what is there, and how it is sorted.

use std::path::{Path, PathBuf};

/// A folder's listing stops here, with a row that says so: a bigger folder
/// costs more memory than a phone-sized screen can show.
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    /// A symlink to a folder: opens as one.
    LinkDir,
    /// A symlink to a file, or one that points nowhere.
    Link,
    /// A device, socket or pipe.
    Other,
}

impl Kind {
    pub fn is_dir(self) -> bool {
        matches!(self, Kind::Dir | Kind::LinkDir)
    }
}

/// One name in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: i64,
    /// The permission bits, 0 when the source has none.
    pub mode: u32,
    /// Where a symlink points.
    pub link: Option<String>,
}

impl Entry {
    pub fn dir(name: &str) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::Dir,
            size: 0,
            mtime: 0,
            mode: 0o755,
            link: None,
        }
    }

    pub fn file(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            kind: Kind::File,
            size,
            mtime: 0,
            mode: 0o644,
            link: None,
        }
    }

    pub fn sort(&self) -> Type {
        if self.kind.is_dir() {
            Type::Folder
        } else {
            Type::of(&self.name)
        }
    }
}

/// What a file is, from its name: its icon, and what Enter does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type {
    Folder,
    Archive,
    Text,
    Code,
    Image,
    Audio,
    Video,
    Other,
}

impl Type {
    pub fn of(name: &str) -> Type {
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        if archive_ext(&lower) {
            return Type::Archive;
        }
        match ext {
            "txt" | "md" | "log" | "csv" | "ini" | "conf" | "cfg" | "desktop" | "service"
            | "srt" | "nfo" => Type::Text,
            "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "sh" | "js" | "ts" | "json" | "toml"
            | "yaml" | "yml" | "xml" | "html" | "css" | "lua" | "go" | "java" => Type::Code,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => Type::Image,
            "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" => Type::Audio,
            "mp4" | "mkv" | "webm" | "avi" | "mov" => Type::Video,
            _ if ext.is_empty() && matches!(lower.as_str(), "readme" | "license" | "makefile") => {
                Type::Text
            }
            _ => Type::Other,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Type::Folder => "folder",
            Type::Archive => "file-archive",
            Type::Text => "file-text",
            Type::Code => "file-code",
            Type::Image => "file-image",
            Type::Audio => "file-music",
            Type::Video => "file-video",
            Type::Other => "file",
        }
    }

    /// Opens in the text viewer.
    pub fn readable(self) -> bool {
        matches!(self, Type::Text | Type::Code)
    }
}

fn archive_ext(lower: &str) -> bool {
    const EXTS: [&str; 17] = [
        ".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst",
        ".tar.lz4", ".7z", ".rar", ".iso", ".gz", ".xz", ".bz2", ".cpio",
    ];
    EXTS.iter()
        .any(|e| lower.ends_with(e) && lower.len() > e.len())
}

/// Where a side is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    /// The list of places and servers: a side's root.
    Places,
    Local(PathBuf),
    /// A folder on a saved server, by the server's name.
    Remote {
        server: String,
        path: String,
    },
    /// A folder inside an archive: the archive's own place, and the path
    /// in it, "" at its top.
    Archive {
        file: Box<Loc>,
        inner: String,
    },
}

impl Loc {
    /// The folder above, or None at the top of the place it started from.
    pub fn up(&self, base: &Loc) -> Option<Loc> {
        if self == base {
            return None;
        }
        match self {
            Loc::Places => None,
            Loc::Local(p) => p.parent().map(|p| Loc::Local(p.to_path_buf())),
            Loc::Remote { server, path } => {
                let up = parent(path)?;
                Some(Loc::Remote {
                    server: server.clone(),
                    path: up,
                })
            }
            Loc::Archive { file, inner } => match parent(inner) {
                Some(up) => Some(Loc::Archive {
                    file: file.clone(),
                    inner: up,
                }),
                None => file.parent_folder(),
            },
        }
    }

    /// The name of this folder in the one above, to select it after going
    /// up.
    pub fn leaf(&self) -> Option<String> {
        match self {
            Loc::Places => None,
            Loc::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
            Loc::Remote { path, .. } => last(path),
            Loc::Archive { file, inner } => last(inner).or_else(|| file.leaf()),
        }
    }

    /// The folder that holds this file.
    pub fn parent_folder(&self) -> Option<Loc> {
        match self {
            Loc::Local(p) => p.parent().map(|p| Loc::Local(p.to_path_buf())),
            Loc::Remote { server, path } => Some(Loc::Remote {
                server: server.clone(),
                path: parent(path)?,
            }),
            _ => None,
        }
    }

    /// `name` inside this folder.
    pub fn join(&self, name: &str) -> Loc {
        match self {
            Loc::Places => Loc::Places,
            Loc::Local(p) => Loc::Local(p.join(name)),
            Loc::Remote { server, path } => Loc::Remote {
                server: server.clone(),
                path: join(path, name),
            },
            Loc::Archive { file, inner } => Loc::Archive {
                file: file.clone(),
                inner: if inner.is_empty() {
                    name.into()
                } else {
                    join(inner, name)
                },
            },
        }
    }

    pub fn is_archive(&self) -> bool {
        matches!(self, Loc::Archive { .. })
    }

    pub fn is_remote(&self) -> bool {
        match self {
            Loc::Remote { .. } => true,
            Loc::Archive { file, .. } => file.is_remote(),
            _ => false,
        }
    }

    /// How the top bar names it: "~/Music", "/etc", "nas:/backup",
    /// "old.zip/docs".
    pub fn show(&self, home: &Path) -> String {
        match self {
            Loc::Places => "Places".into(),
            Loc::Local(p) => match p.strip_prefix(home) {
                Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => p.display().to_string(),
            },
            Loc::Remote { server, path } => format!("{server}:{path}"),
            Loc::Archive { file, inner } => {
                let name = file.leaf().unwrap_or_default();
                if inner.is_empty() {
                    name
                } else {
                    format!("{name}/{inner}")
                }
            }
        }
    }
}

fn parent(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    match t.rsplit_once('/') {
        Some(("", _)) => Some("/".into()),
        Some((up, _)) => Some(up.into()),
        None => Some(String::new()),
    }
}

fn last(path: &str) -> Option<String> {
    let t = path.trim_end_matches('/');
    let name = t.rsplit('/').next().unwrap_or(t);
    (!name.is_empty()).then(|| name.to_string())
}

fn join(path: &str, name: &str) -> String {
    if path.ends_with('/') {
        format!("{path}{name}")
    } else {
        format!("{path}/{name}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Size,
    Date,
    Type,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Name, Sort::Size, Sort::Date, Sort::Type];

    pub fn word(self) -> &'static str {
        match self {
            Sort::Name => "Name",
            Sort::Size => "Size",
            Sort::Date => "Date",
            Sort::Type => "Type",
        }
    }

    pub fn parse(s: &str) -> Sort {
        Sort::ALL
            .into_iter()
            .find(|x| x.word().eq_ignore_ascii_case(s))
            .unwrap_or(Sort::Name)
    }
}

/// Folders first, then by `sort`; names break ties, case aside. Size and
/// date put the biggest and newest first.
pub fn sort(entries: &mut [Entry], by: Sort) {
    entries.sort_by(|a, b| {
        let name = || natural(&a.name, &b.name);
        b.kind
            .is_dir()
            .cmp(&a.kind.is_dir())
            .then_with(|| match by {
                Sort::Name => name(),
                Sort::Size => b.size.cmp(&a.size).then_with(name),
                Sort::Date => b.mtime.cmp(&a.mtime).then_with(name),
                Sort::Type => a.sort().cmp(&b.sort()).then_with(name),
            })
    });
}

/// Names as people read them: case aside, and runs of digits by value, so
/// "track 2" comes before "track 10".
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let o = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if o.is_ne() {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o.is_ne() {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A local folder's entries, unsorted, hidden ones too, at most `cap`
/// ([`MAX_ENTRIES`] for the screen). The bool says whether more were left
/// out.
pub fn list_local(dir: &Path, cap: usize) -> std::io::Result<(Vec<Entry>, bool)> {
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    let mut more = false;
    for de in std::fs::read_dir(dir)? {
        let Ok(de) = de else { continue };
        if out.len() == cap {
            more = true;
            break;
        }
        let name = de.file_name().to_string_lossy().into_owned();
        let Ok(lmeta) = std::fs::symlink_metadata(de.path()) else {
            continue;
        };
        let ft = lmeta.file_type();
        let (kind, meta, link) = if ft.is_symlink() {
            let target = std::fs::read_link(de.path())
                .ok()
                .map(|t| t.to_string_lossy().into_owned());
            match std::fs::metadata(de.path()) {
                Ok(m) if m.is_dir() => (Kind::LinkDir, m, target),
                Ok(m) => (Kind::Link, m, target),
                Err(_) => (Kind::Link, lmeta, target),
            }
        } else if ft.is_dir() {
            (Kind::Dir, lmeta, None)
        } else if ft.is_file() {
            (Kind::File, lmeta, None)
        } else {
            (Kind::Other, lmeta, None)
        };
        out.push(Entry {
            name,
            kind,
            size: if kind.is_dir() { 0 } else { meta.len() },
            mtime: meta.mtime(),
            mode: meta.mode() & 0o7777,
            link,
        });
    }
    Ok((out, more))
}

/// "820 B", "14 kB", "3.4 MB", "28 MB", "1.2 GB".
pub fn size_text(bytes: u64) -> String {
    const K: u64 = 1000;
    let tenths = |unit: u64| {
        let t = bytes * 10 / unit;
        format!("{}.{}", t / 10, t % 10)
    };
    match bytes {
        0..K => format!("{bytes} B"),
        K..1_000_000 => format!("{} kB", bytes / K),
        1_000_000..10_000_000 => format!("{} MB", tenths(1_000_000)),
        10_000_000..1_000_000_000 => format!("{} MB", bytes / 1_000_000),
        1_000_000_000..10_000_000_000 => format!("{} GB", tenths(1_000_000_000)),
        _ => format!("{} GB", bytes / 1_000_000_000),
    }
}

/// "rwxr-xr-x".
pub fn mode_text(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// "2026-11-24 14:05", local time.
pub fn time_text(secs: i64) -> String {
    if secs <= 0 {
        return "unknown".into();
    }
    // SAFETY: localtime_r into a local struct tm.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&(secs as _), &mut tm);
        tm
    };
    format!(
        "{}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// A name a listing may hold: one part of a path, nothing that climbs or
/// jumps out of the folder.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// A name the user typed is one we can make: not empty, not "." or "..",
/// no slash or NUL, at most 255 bytes.
pub fn name_ok(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("A name can't be empty");
    }
    if name == "." || name == ".." {
        return Err("That name is taken by the system");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("A name can't have a /");
    }
    if name.len() > 255 {
        return Err("Too long");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_come_from_names() {
        assert_eq!(Type::of("backup.tar.gz"), Type::Archive);
        assert_eq!(Type::of("Song.FLAC"), Type::Audio);
        assert_eq!(Type::of("README"), Type::Text);
        assert_eq!(Type::of(".zip"), Type::Other, "a bare extension is a name");
        assert_eq!(Type::of("main.rs"), Type::Code);
        assert_eq!(Type::of("photo.jpeg").icon(), "file-image");
    }

    #[test]
    fn natural_order_reads_numbers() {
        let mut v = ["track 10", "Track 2", "track 1", "b", "A"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["A", "b", "track 1", "Track 2", "track 10"]);
    }

    #[test]
    fn folders_come_first_then_the_sort() {
        let mut v = vec![
            Entry::file("big", 900),
            Entry::dir("zeta"),
            Entry::file("small", 3),
            Entry::dir("alpha"),
        ];
        sort(&mut v, Sort::Size);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta", "big", "small"]);
    }

    #[test]
    fn up_stops_at_the_base_and_leaves_archives() {
        let home = Loc::Local("/home/me".into());
        let music = home.join("Music");
        assert_eq!(music.up(&home), Some(home.clone()));
        assert_eq!(home.up(&home), None);
        let nas = Loc::Remote {
            server: "nas".into(),
            path: "/".into(),
        };
        let deep = nas.join("a").join("b");
        assert_eq!(deep.show(Path::new("/x")), "nas:/a/b");
        assert_eq!(deep.up(&nas).unwrap().show(Path::new("/x")), "nas:/a");
        let zip = Loc::Archive {
            file: Box::new(music.join("old.zip")),
            inner: String::new(),
        };
        let inside = zip.join("docs");
        assert_eq!(inside.show(Path::new("/home/me")), "old.zip/docs");
        assert_eq!(inside.up(&home), Some(zip.clone()));
        assert_eq!(
            zip.up(&home),
            Some(music),
            "out of the archive to its folder"
        );
        assert_eq!(zip.leaf().as_deref(), Some("old.zip"));
    }

    #[test]
    fn sizes_and_modes_read_short() {
        assert_eq!(size_text(820), "820 B");
        assert_eq!(size_text(3_400_000), "3.4 MB");
        assert_eq!(size_text(28_100_000), "28 MB");
        assert_eq!(size_text(1_234_000_000), "1.2 GB");
        assert_eq!(mode_text(0o754), "rwxr-xr--");
    }

    #[test]
    fn names_are_checked() {
        assert!(name_ok("notes.txt").is_ok());
        assert!(name_ok("a/b").is_err());
        assert!(name_ok("..").is_err());
        assert!(name_ok("  ").is_err());
    }

    #[test]
    fn a_local_folder_lists_kinds_and_links() {
        let dir = std::env::temp_dir().join(format!("gh0st-files-ls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::os::unix::fs::symlink("sub", dir.join("to-sub")).unwrap();
        std::os::unix::fs::symlink("gone", dir.join("dangling")).unwrap();
        let (mut v, more) = list_local(&dir, MAX_ENTRIES).unwrap();
        assert!(!more);
        sort(&mut v, Sort::Name);
        let got: Vec<_> = v.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            got,
            [
                ("sub", Kind::Dir),
                ("to-sub", Kind::LinkDir),
                ("a.txt", Kind::File),
                ("dangling", Kind::Link)
            ]
        );
        assert_eq!(v[2].size, 5);
        assert_eq!(v[3].link.as_deref(), Some("gone"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
