//! Where shots go, what they are called, and how much room is left. Names
//! are stock Camera's: `CAM_YYYYmmdd_HHMMSS.jpg`, and `VID_` for video.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// What a photo takes on the card, about: 8 MP at quality 95.
pub const PHOTO_MB: u64 = 4;
/// What a minute of video takes, about: 12 Mb/s and AAC.
pub const VIDEO_MB_PER_MIN: u64 = 94;
/// Below this, shooting stops: the card must keep room to boot.
pub const MIN_FREE_MB: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    Video,
}

impl Kind {
    pub fn of(p: &Path) -> Option<Kind> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Kind::Photo),
            "mp4" => Some(Kind::Video),
            _ => None,
        }
    }
}

/// `CAM_20261123_143005`, from seconds since 1970.
pub fn stem(prefix: &str, epoch: i64) -> String {
    let l = gh0st_core::rt::local_at(epoch);
    format!(
        "{prefix}_{:04}{:02}{:02}_{:02}{:02}{:02}",
        l.year,
        l.month + 1,
        l.day,
        l.hour,
        l.minute,
        epoch.rem_euclid(60)
    )
}

/// A path in `dir` no file has yet: `_2`, `_3` and on after a clash, for
/// two shots in one second.
pub fn fresh(dir: &Path, prefix: &str, ext: &str, epoch: i64) -> PathBuf {
    let base = stem(prefix, epoch);
    let mut n = 1;
    loop {
        let name = match n {
            1 => format!("{base}.{ext}"),
            _ => format!("{base}_{n}.{ext}"),
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A shot on the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    pub kind: Kind,
    /// Seconds since 1970.
    pub when: i64,
    pub bytes: u64,
}

/// Every photo and video in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Shot> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<Shot> = rd
        .filter_map(|e| {
            let e = e.ok()?;
            let path = e.path();
            let kind = Kind::of(&path)?;
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            let when = m
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some(Shot {
                path,
                kind,
                when,
                bytes: m.len(),
            })
        })
        .collect();
    v.sort_by(|a, b| b.when.cmp(&a.when).then(b.path.cmp(&a.path)));
    v
}

/// Free MB on the card that holds `dir`, or its nearest parent that exists.
pub fn free_mb(dir: &Path) -> Option<u64> {
    let mut p = dir;
    while !p.exists() {
        p = p.parent()?;
    }
    let (used, total) = gh0st_app::system::storage(p)?;
    Some(total.saturating_sub(used))
}

/// "1.2k" photos, or "42 min" of video, as the rail shows it.
pub fn left(free: u64, kind: Kind) -> String {
    let room = free.saturating_sub(MIN_FREE_MB);
    match kind {
        Kind::Photo => match room / PHOTO_MB {
            n if n >= 10_000 => format!("{}k", n / 1000),
            n if n >= 1000 => format!("{}.{}k", n / 1000, n % 1000 / 100),
            n => n.to_string(),
        },
        Kind::Video => match room / VIDEO_MB_PER_MIN {
            m if m >= 600 => format!("{}h", m / 60),
            m => format!("{m}m"),
        },
    }
}

/// Where the helper keeps a video's picture: `~/.cache/gh0st-camera`,
/// out of the photos folder (`posterFor` in camd/main.cpp).
pub fn poster(video: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let stem = video.file_stem()?;
    let mut name = stem.to_os_string();
    name.push(".jpg");
    Some(PathBuf::from(home).join(".cache/gh0st-camera").join(name))
}

/// A JPEG's width and height, from its frame header: the first 128 KB
/// hold it, past any EXIF.
pub fn jpeg_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut b = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(128 << 10)
        .read_to_end(&mut b)
        .ok()?;
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xff {
            return None;
        }
        let marker = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        // SOF0 to SOF15, but DHT, JPG and DAC share the range.
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// "3.9 MB".
pub fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_shot_in_the_same_second_gets_a_suffix() {
        let dir = std::env::temp_dir().join(format!("gh0st-camera-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = fresh(&dir, "CAM", "jpg", 1_795_000_000);
        std::fs::write(&a, b"x").unwrap();
        let b = fresh(&dir, "CAM", "jpg", 1_795_000_000);
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("_2.jpg"));
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("CAM_20")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn room_reads_short() {
        assert_eq!(left(64 + 4 * 1234, Kind::Photo), "1.2k");
        assert_eq!(left(64 + 4 * 12, Kind::Photo), "12");
        assert_eq!(left(10, Kind::Photo), "0");
        assert_eq!(left(64 + 94 * 42, Kind::Video), "42m");
        assert_eq!(left(64 + 94 * 700, Kind::Video), "11h");
        assert_eq!(size(4_089_446), "3.9 MB");
    }

    #[test]
    fn only_photos_and_videos_are_listed() {
        assert_eq!(Kind::of(Path::new("a/CAM_1.JPG")), Some(Kind::Photo));
        assert_eq!(Kind::of(Path::new("VID_1.mp4")), Some(Kind::Video));
        assert_eq!(Kind::of(Path::new("CAM_1.dng")), None);
        assert_eq!(Kind::of(Path::new("CAM_1.jpg.part")), None);
    }
}
