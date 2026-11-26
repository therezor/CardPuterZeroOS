//! Radio switches through `/dev/rfkill`. Stock boots with every radio
//! blocked; the user's session ACL usually allows the write, and the root
//! helper covers the rest.

use std::io::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radio {
    Wlan = 1,
    Bluetooth = 2,
}

/// `struct rfkill_event`: idx, type, op, soft, hard. Op 3 changes every
/// switch of the type.
fn event(radio: Radio, blocked: bool) -> [u8; 8] {
    [0, 0, 0, 0, radio as u8, 3, blocked as u8, 0]
}

/// Blocks or unblocks every switch of `radio`.
pub fn set(radio: Radio, blocked: bool) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/rfkill")?;
    f.write_all(&event(radio, blocked))
}

/// Whether any Wi-Fi radio is free to run: neither switch blocks it.
/// `root` is `/sys/class/rfkill`.
pub fn wlan_on(root: &std::path::Path) -> bool {
    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .any(|d| {
            read(d.join("type")).trim() == "wlan"
                && read(d.join("soft")).trim() == "0"
                && read(d.join("hard")).trim() == "0"
        })
}

/// Whether a switch of `radio` is blocked in software. `root` is
/// `/sys/class/rfkill`.
pub fn blocked(root: &std::path::Path, radio: Radio) -> bool {
    let kind = match radio {
        Radio::Wlan => "wlan",
        Radio::Bluetooth => "bluetooth",
    };
    let read = |p: std::path::PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .any(|d| read(d.join("type")).trim() == kind && read(d.join("soft")).trim() == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wlan_on_needs_both_switches_open_and_blocked_reads_soft() {
        let root = std::env::temp_dir().join(format!("gh0st-rfkill-{}", std::process::id()));
        let sw = |n: &str, kind: &str, soft: &str| {
            let d = root.join(n);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("type"), format!("{kind}\n")).unwrap();
            std::fs::write(d.join("soft"), format!("{soft}\n")).unwrap();
            std::fs::write(d.join("hard"), "0\n").unwrap();
        };
        sw("rfkill0", "bluetooth", "0");
        sw("rfkill1", "wlan", "1");
        assert!(!wlan_on(&root), "blocked");
        assert!(blocked(&root, Radio::Wlan));
        assert!(!blocked(&root, Radio::Bluetooth));
        sw("rfkill1", "wlan", "0");
        assert!(wlan_on(&root));
        assert!(!blocked(&root, Radio::Wlan));
        sw("rfkill0", "bluetooth", "1");
        assert!(blocked(&root, Radio::Bluetooth));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_event_changes_all_switches_of_one_type() {
        assert_eq!(event(Radio::Bluetooth, false), [0, 0, 0, 0, 2, 3, 0, 0]);
        assert_eq!(event(Radio::Wlan, true), [0, 0, 0, 0, 1, 3, 1, 0]);
    }
}
