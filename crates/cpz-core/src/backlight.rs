//! Panel backlight through sysfs, as stock: `brightness` and
//! `max_brightness` under `/sys/class/backlight/backlight`, or the first
//! backlight there. The config keeps the raw value, as stock does.

use std::path::{Path, PathBuf};

pub struct Backlight {
    dir: PathBuf,
    pub max: u32,
}

impl Backlight {
    pub fn find(root: &Path) -> Option<Backlight> {
        let preferred = root.join("backlight");
        let dir = if preferred.join("brightness").exists() {
            preferred
        } else {
            let mut all: Vec<PathBuf> = std::fs::read_dir(root)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .collect();
            all.sort();
            all.into_iter().find(|p| p.join("brightness").exists())?
        };
        let max = read(&dir.join("max_brightness"))
            .filter(|&m| m > 0)
            .unwrap_or(100);
        Some(Backlight { dir, max })
    }

    pub fn get(&self) -> Option<u32> {
        read(&self.dir.join("brightness"))
    }

    pub fn set(&self, raw: u32) -> std::io::Result<()> {
        std::fs::write(self.dir.join("brightness"), raw.min(self.max).to_string())
    }

    pub fn percent(&self, raw: u32) -> u8 {
        ((raw.min(self.max) * 100 + self.max / 2) / self.max) as u8
    }

    pub fn raw(&self, percent: u8) -> u32 {
        (percent.min(100) as u32 * self.max).div_ceil(100)
    }

    /// One 10% step up or down from `raw`, never to 0: a dark panel with the
    /// device on reads as off.
    pub fn step(&self, raw: u32, up: bool) -> u32 {
        let p = self.percent(raw) as i32 + if up { 10 } else { -10 };
        let p = (p / 10 * 10).clamp(10, 100) as u8;
        self.raw(p)
    }
}

fn read(p: &Path) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_tens_of_percent_and_never_dark() {
        let b = Backlight {
            dir: PathBuf::new(),
            max: 255,
        };
        assert_eq!(b.percent(b.step(b.raw(50), true)), 60);
        assert_eq!(b.percent(b.step(b.raw(10), false)), 10);
        assert_eq!(b.percent(b.step(b.raw(100), true)), 100);
    }
}
