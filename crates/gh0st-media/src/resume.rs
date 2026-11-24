//! Where long clips stopped, so a film goes on from there. Only clips of
//! five minutes or more, only between their first 10 s and their last
//! 30 s, and only the newest 64: one small text file in the app's data.

use std::path::{Path, PathBuf};

/// Shorter clips start from the top.
pub const LONG_MS: u64 = 5 * 60 * 1000;
const KEEP: usize = 64;
const HEAD_MS: u64 = 10_000;
const TAIL_MS: u64 = 30_000;

pub struct Resume {
    file: Option<PathBuf>,
    /// (key of the path, position), newest first.
    list: Vec<(u64, u64)>,
    dirty: bool,
}

fn key(p: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in p.as_os_str().as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Resume {
    /// Nothing kept: for tests, or without a data folder.
    pub fn none() -> Resume {
        Resume {
            file: None,
            list: Vec::new(),
            dirty: false,
        }
    }

    pub fn load(file: PathBuf) -> Resume {
        let list = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let (k, ms) = l.split_once(' ')?;
                Some((u64::from_str_radix(k, 16).ok()?, ms.parse().ok()?))
            })
            .take(KEEP)
            .collect();
        Resume {
            file: Some(file),
            list,
            dirty: false,
        }
    }

    /// Where `path` stopped, if it is worth going on from.
    pub fn get(&self, path: &Path) -> Option<u64> {
        let k = key(path);
        self.list.iter().find(|(x, _)| *x == k).map(|&(_, ms)| ms)
    }

    /// `path`, `dur_ms` long, stopped at `pos_ms`. Near either end, or a
    /// short clip, forgets it instead.
    pub fn set(&mut self, path: &Path, pos_ms: u64, dur_ms: u64) {
        let k = key(path);
        let before = self.list.len();
        self.list.retain(|(x, _)| *x != k);
        let keep = dur_ms >= LONG_MS && pos_ms >= HEAD_MS && pos_ms + TAIL_MS < dur_ms;
        if keep {
            self.list.insert(0, (k, pos_ms));
            self.list.truncate(KEEP);
        }
        self.dirty |= keep || self.list.len() != before;
    }

    /// Writes the list if it changed.
    pub fn save(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        if !self.dirty {
            return;
        }
        let text: String = self
            .list
            .iter()
            .map(|(k, ms)| format!("{k:016x} {ms}\n"))
            .collect();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, file).is_ok() {
            self.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_clips_away_from_their_ends_are_kept_and_the_rest_forgotten() {
        let mut r = Resume::none();
        let film = Path::new("/v/film.mkv");
        r.set(film, 600_000, 5_400_000);
        assert_eq!(r.get(film), Some(600_000));
        r.set(film, 5_390_000, 5_400_000);
        assert_eq!(r.get(film), None, "at its end it starts over");
        r.set(film, 3_000, 5_400_000);
        assert_eq!(r.get(film), None, "at its start, nothing to keep");
        r.set(Path::new("/v/clip.mp4"), 60_000, 95_000);
        assert_eq!(
            r.get(Path::new("/v/clip.mp4")),
            None,
            "short clips start over"
        );
    }

    #[test]
    fn the_newest_64_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("gh0st-resume-{}", std::process::id()));
        let file = dir.join("positions");
        let mut r = Resume::load(file.clone());
        for i in 0..70u64 {
            r.set(
                &PathBuf::from(format!("/v/{i}.mkv")),
                60_000 + i,
                LONG_MS * 2,
            );
        }
        r.save();
        let r = Resume::load(file);
        assert_eq!(r.get(Path::new("/v/69.mkv")), Some(60_069));
        assert_eq!(r.get(Path::new("/v/6.mkv")), Some(60_006));
        assert_eq!(r.get(Path::new("/v/5.mkv")), None, "the oldest went");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
