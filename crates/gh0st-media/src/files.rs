//! Folders and the media in them: what a name looks like it holds, a
//! folder's listing, and the places the start page offers. What a file
//! really is comes from the helper's probe; the name only sorts and badges
//! tiles before that.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// What a file looks like from its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Folder,
    Picture,
    Video,
    Sound,
}

const PICTURES: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "apng", "gif", "webp", "bmp", "tif", "tiff", "tga", "ppm", "pgm",
    "pbm", "pnm", "qoi", "ico", "jp2", "j2k", "jxl", "svg", "heic", "heif", "avif", "psd", "exr",
    "hdr", "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2",
    "pef", "srw", "3fr", "erf", "kdc", "mos", "mrw", "x3f", "iiq", "rwl",
];
const VIDEOS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "ts", "m2ts", "mts", "mpg", "mpeg", "ogv", "flv",
    "3gp", "3g2", "wmv", "asf", "vob",
];
const SOUNDS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "opus", "wav", "aiff", "aif", "wma", "ape",
    "wv", "alac", "mka", "dsf",
];

/// None for a file that is no media.
pub fn look_of(path: &Path) -> Option<Look> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let ext = ext.as_str();
    if PICTURES.contains(&ext) {
        Some(Look::Picture)
    } else if VIDEOS.contains(&ext) {
        Some(Look::Video)
    } else if SOUNDS.contains(&ext) {
        Some(Look::Sound)
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub look: Look,
    /// Seconds since the epoch, modified.
    pub when: i64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Newest,
    Name,
}

impl Sort {
    pub fn word(self) -> &'static str {
        match self {
            Sort::Newest => "Newest",
            Sort::Name => "Name",
        }
    }
}

/// A folder's subfolders, by name, then its media in `sort`'s order.
/// Hidden names are left out.
pub fn list(dir: &Path, sort: Sort) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    let mut media = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        // Through links, as Files shows them.
        let Ok(md) = std::fs::metadata(&path) else {
            continue;
        };
        let when = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        if md.is_dir() {
            folders.push(Entry {
                path,
                name,
                look: Look::Folder,
                when,
                bytes: 0,
            });
        } else if let Some(look) = look_of(&path) {
            media.push(Entry {
                path,
                name,
                look,
                when,
                bytes: md.len(),
            });
        }
    }
    let by_name = |a: &Entry, b: &Entry| natural(&a.name, &b.name);
    folders.sort_by(by_name);
    match sort {
        Sort::Name => media.sort_by(by_name),
        Sort::Newest => media.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| by_name(a, b))),
    }
    folders.extend(media);
    folders
}

/// Names as people count: "track 2" before "track 10", case aside.
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                let mut m = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
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

/// The picture a folder's tile shows: its cover, else its newest picture.
pub fn folder_face(dir: &Path) -> Option<PathBuf> {
    for n in [
        "cover.jpg",
        "cover.png",
        "folder.jpg",
        "folder.png",
        "front.jpg",
    ] {
        let p = dir.join(n);
        if p.is_file() {
            return Some(p);
        }
    }
    list(dir, Sort::Newest)
        .into_iter()
        .find(|e| e.look == Look::Picture || e.look == Look::Video)
        .map(|e| e.path)
}

/// A place on the start page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: &'static str,
    pub icon: &'static str,
    pub path: PathBuf,
}

/// An XDG user folder (`PICTURES`, `VIDEOS`, `MUSIC`), from
/// user-dirs.dirs, else its usual name in home.
pub fn user_dir(home: &Path, key: &str, fallback: &str) -> PathBuf {
    let conf = home.join(".config/user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(conf) {
        let want = format!("XDG_{key}_DIR=");
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix(&want) {
                let v = v.trim_matches('"');
                let p = match v.strip_prefix("$HOME") {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(v),
                };
                if p.is_absolute() {
                    return p;
                }
            }
        }
    }
    home.join(fallback)
}

pub fn places(home: &Path) -> Vec<Place> {
    let pictures = user_dir(home, "PICTURES", "Pictures");
    vec![
        Place {
            name: "Camera",
            icon: "camera",
            path: pictures.join("DCIM/Camera"),
        },
        Place {
            name: "Pictures",
            icon: "images",
            path: pictures,
        },
        Place {
            name: "Videos",
            icon: "video",
            path: user_dir(home, "VIDEOS", "Videos"),
        },
        Place {
            name: "Music",
            icon: "music",
            path: user_dir(home, "MUSIC", "Music"),
        },
        Place {
            name: "Home",
            icon: "house",
            path: home.to_path_buf(),
        },
    ]
}

/// A size as people read it: 4.1 MB.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_count_as_people_do() {
        let mut v = vec![
            "track 10.mp3",
            "Track 2.mp3",
            "track 1.mp3",
            "b.mp3",
            "A.mp3",
        ];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(
            v,
            [
                "A.mp3",
                "b.mp3",
                "track 1.mp3",
                "Track 2.mp3",
                "track 10.mp3"
            ]
        );
    }

    #[test]
    fn a_folder_lists_folders_then_media_and_skips_the_rest() {
        let d = std::env::temp_dir().join(format!("gh0st-media-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Trip")).unwrap();
        for n in ["b.jpg", "a.mkv", "notes.txt", ".hidden.png", "song.flac"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        let names: Vec<String> = list(&d, Sort::Name).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Trip", "a.mkv", "b.jpg", "song.flac"]);
        assert_eq!(look_of(Path::new("x.HEIC")), Some(Look::Picture));
        assert_eq!(look_of(Path::new("x.txt")), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn user_dirs_follow_the_xdg_file() {
        let home = std::env::temp_dir().join(format!("gh0st-media-home-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::write(
            home.join(".config/user-dirs.dirs"),
            "XDG_PICTURES_DIR=\"$HOME/Photos\"\n",
        )
        .unwrap();
        assert_eq!(user_dir(&home, "PICTURES", "Pictures"), home.join("Photos"));
        assert_eq!(user_dir(&home, "MUSIC", "Music"), home.join("Music"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
