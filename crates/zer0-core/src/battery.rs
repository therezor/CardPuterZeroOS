//! The battery: a reading from power_supply sysfs, and the low-battery flow
//! from stock (`low_battery_flow.hpp`): a reminder at 20% or less, a warning
//! at 5% or less, a 15 s countdown and power off at 3% or less. Charging
//! cancels all of it.
//!
//! One change from stock: stock skips 5% exactly (no warning), which reads as
//! an accident. Here 4% and 5% both warn.

use std::path::{Path, PathBuf};

pub const SHUTDOWN_DELAY_MS: u64 = 15_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub percent: u8,
    pub charging: bool,
}

/// Finds the battery once: the first `type=Battery` supply, a `bq27*` one
/// preferred. `root` is `/sys/class/power_supply`, or a fake tree in tests.
pub fn find(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            read(&p.join("type")).as_deref() == Some("Battery") && p.join("capacity").exists()
        })
        .collect();
    found.sort();
    let bq = found.iter().position(|p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().contains("bq27"))
    });
    bq.map(|i| found.swap_remove(i))
        .or_else(|| found.into_iter().next())
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Reads `capacity` and `status`. `None` if either is unreadable.
pub fn sample(dir: &Path) -> Option<Reading> {
    let percent = read(&dir.join("capacity"))?.parse::<u8>().ok()?.min(100);
    let charging = matches!(read(&dir.join("status"))?.as_str(), "Charging" | "Full");
    Some(Reading { percent, charging })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    None,
    /// Shown once per discharge, at 20% or less.
    Reminder(u8),
    Low(u8),
    /// Power off when the countdown runs out, unless charging starts.
    Countdown,
}

#[derive(Clone, Copy, Debug)]
pub struct LowBattery {
    warning: Warning,
    reminded: bool,
    countdown_from: u64,
    shutdown_sent: bool,
}

impl Default for LowBattery {
    fn default() -> Self {
        LowBattery {
            warning: Warning::None,
            reminded: false,
            countdown_from: 0,
            shutdown_sent: false,
        }
    }
}

impl LowBattery {
    pub fn update(&mut self, r: Option<Reading>, now: u64) -> Warning {
        let Some(r) = r else {
            // An unreadable gauge must never power the device off.
            self.warning = Warning::None;
            return self.warning;
        };
        if r.charging || r.percent > 20 {
            self.reminded = false;
            self.shutdown_sent = false;
            self.warning = Warning::None;
        } else if r.percent > 5 {
            self.warning = if self.reminded {
                Warning::None
            } else {
                Warning::Reminder(r.percent)
            };
            self.reminded = true;
        } else if r.percent > 3 {
            self.warning = Warning::Low(r.percent);
        } else if self.warning != Warning::Countdown {
            self.warning = Warning::Countdown;
            self.countdown_from = now;
            self.shutdown_sent = false;
        }
        self.warning
    }

    pub fn warning(&self) -> Warning {
        self.warning
    }

    pub fn dismiss(&mut self) {
        if matches!(self.warning, Warning::Reminder(_) | Warning::Low(_)) {
            self.warning = Warning::None;
        }
    }

    /// Whole seconds left on the countdown, rounded up.
    pub fn seconds_left(&self, now: u64) -> u64 {
        if self.warning != Warning::Countdown {
            return 0;
        }
        let gone = now.saturating_sub(self.countdown_from);
        SHUTDOWN_DELAY_MS.saturating_sub(gone).div_ceil(1000)
    }

    /// True once, when the countdown has run out.
    pub fn take_shutdown(&mut self, now: u64) -> bool {
        let due = self.warning == Warning::Countdown
            && !self.shutdown_sent
            && now.saturating_sub(self.countdown_from) >= SHUTDOWN_DELAY_MS;
        self.shutdown_sent |= due;
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(percent: u8, charging: bool) -> Option<Reading> {
        Some(Reading { percent, charging })
    }

    #[test]
    fn the_reminder_shows_once_per_discharge() {
        let mut lb = LowBattery::default();
        assert_eq!(lb.update(r(50, false), 0), Warning::None);
        assert_eq!(lb.update(r(20, false), 0), Warning::Reminder(20));
        assert_eq!(lb.update(r(19, false), 0), Warning::None);
        lb.update(r(19, true), 0);
        assert_eq!(lb.update(r(18, false), 0), Warning::Reminder(18));
    }

    #[test]
    fn three_percent_counts_down_fifteen_seconds_then_shuts_down_once() {
        let mut lb = LowBattery::default();
        assert_eq!(lb.update(r(5, false), 0), Warning::Low(5));
        assert_eq!(lb.update(r(3, false), 1000), Warning::Countdown);
        lb.update(r(2, false), 5000);
        assert_eq!(lb.seconds_left(5000), 11);
        assert!(!lb.take_shutdown(15_999));
        assert!(lb.take_shutdown(16_000));
        assert!(!lb.take_shutdown(17_000));
    }

    #[test]
    fn plugging_in_cancels_the_countdown_and_no_reading_never_shuts_down() {
        let mut lb = LowBattery::default();
        lb.update(r(2, false), 0);
        assert_eq!(lb.update(r(2, true), 1000), Warning::None);
        assert!(!lb.take_shutdown(20_000));
        assert_eq!(lb.update(None, 0), Warning::None);
    }

    #[test]
    fn sysfs_reading_prefers_the_bq27_gauge() {
        let root = std::env::temp_dir().join(format!("zer0-bat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, kind, cap, status) in [
            ("ac", "Mains", "", ""),
            ("a-bat", "Battery", "40", "Discharging"),
            ("bq27220-0", "Battery", "87", "Charging"),
        ] {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("type"), format!("{kind}\n")).unwrap();
            if !cap.is_empty() {
                std::fs::write(d.join("capacity"), format!("{cap}\n")).unwrap();
                std::fs::write(d.join("status"), format!("{status}\n")).unwrap();
            }
        }
        let dir = find(&root).unwrap();
        assert!(dir.ends_with("bq27220-0"));
        assert_eq!(
            sample(&dir),
            Some(Reading {
                percent: 87,
                charging: true
            })
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
