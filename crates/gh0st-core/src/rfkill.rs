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
    let mut f = std::fs::OpenOptions::new().write(true).open("/dev/rfkill")?;
    f.write_all(&event(radio, blocked))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_changes_all_switches_of_one_type() {
        assert_eq!(event(Radio::Bluetooth, false), [0, 0, 0, 0, 2, 3, 0, 0]);
        assert_eq!(event(Radio::Wlan, true), [0, 0, 0, 0, 1, 3, 1, 0]);
    }
}
