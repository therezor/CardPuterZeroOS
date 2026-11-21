//! bluetoothctl's output read into Settings state. The shell runs it as
//! jobs; this only parses.

use super::pages::Device;

/// The one command Settings runs to learn everything: the adapter, then the
/// paired, connected and known devices, between markers.
pub const QUERY: &str = "bluetoothctl show; echo @@paired; bluetoothctl devices Paired; \
echo @@connected; bluetoothctl devices Connected; echo @@all; bluetoothctl devices";

/// Drops the colour codes bluetoothctl prints even into a pipe.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// A MAC as bluetoothctl writes it, so it is safe to pass on.
pub fn is_mac(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':')
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn device_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim().split("Device ").nth(1)?;
    let (mac, name) = rest.split_once(' ').unwrap_or((rest, ""));
    is_mac(mac).then(|| (mac.to_string(), name.trim().to_string()))
}

/// The adapter's power and the devices, paired and connected first. A
/// device with no name (bluetoothctl shows its MAC with dashes) is left out
/// unless it is paired.
pub fn parse_query(out: &str) -> (bool, Vec<Device>) {
    let out = plain(out);
    let mut powered = false;
    let (mut paired, mut connected) = (Vec::new(), Vec::new());
    let mut devices: Vec<Device> = Vec::new();
    let mut section = "";
    for line in out.lines() {
        match line.trim() {
            "@@paired" | "@@connected" | "@@all" => {
                section = line.trim();
                continue;
            }
            l if section.is_empty() && l.starts_with("Powered:") => powered = l.ends_with("yes"),
            _ => {}
        }
        let Some((mac, name)) = device_line(line) else {
            continue;
        };
        match section {
            "@@paired" => paired.push(mac),
            "@@connected" => connected.push(mac),
            "@@all" if !devices.iter().any(|d| d.mac == mac) => {
                let unnamed = name.is_empty() || name == mac.replace(':', "-");
                devices.push(Device {
                    paired: paired.contains(&mac),
                    connected: connected.contains(&mac),
                    name: if unnamed { String::new() } else { name },
                    mac,
                });
            }
            _ => {}
        }
    }
    devices.retain(|d| d.paired || !d.name.is_empty());
    devices.sort_by_key(|d| (!d.connected, !d.paired, d.name.to_lowercase()));
    (powered, devices)
}

/// A failed bluetoothctl run, in a few words.
pub fn failure(out: &str) -> String {
    let out = plain(out);
    out.lines()
        .map(str::trim)
        .find(|l| l.starts_with("Failed") || l.contains("org.bluez.Error"))
        .map(|l| l.trim_end_matches('.').to_string())
        .unwrap_or_else(|| "Bluetooth did not answer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "Controller F0:40:AF:90:9A:93 (public)\n\tName: RZR-ZERO\n\tPowered: yes\n@@paired\nDevice 41:42:A1:00:12:9F Buds Pro\n@@connected\nDevice 41:42:A1:00:12:9F Buds Pro\n@@all\n[\u{1b}[0;92mNEW\u{1b}[0m] Device 79:B0:B1:8A:6D:17 79-B0-B1-8A-6D-17\nDevice 80:E1:26:52:5A:52 Flipper RZR\nDevice 41:42:A1:00:12:9F Buds Pro\n";

    #[test]
    fn power_and_devices_with_the_connected_one_first_and_unnamed_ones_gone() {
        let (powered, d) = parse_query(OUT);
        assert!(powered);
        let names: Vec<&str> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Buds Pro", "Flipper RZR"]);
        assert!(d[0].connected && d[0].paired);
        assert!(!d[1].paired);
    }

    #[test]
    fn macs_are_checked_before_they_reach_a_command() {
        assert!(is_mac("80:E1:26:52:5A:52"));
        assert!(!is_mac("80:E1:26:52:5A:5;"));
        assert!(!is_mac("80:E1:26:52:5A"));
    }

    #[test]
    fn failures_keep_the_bluez_reason() {
        assert_eq!(
            failure("Attempting to pair\nFailed to pair: org.bluez.Error.AuthenticationFailed\n"),
            "Failed to pair: org.bluez.Error.AuthenticationFailed"
        );
    }
}
