//! The GATT objects BlueZ serves for us: HID over GATT (service 0x1812)
//! and the Battery service (0x180f). BlueZ already serves GAP, GATT,
//! Device Information and the PnP ID.

use gh0st_dbus::Value;
use gh0st_dbus::objects::Tree;

use crate::hid::desc;

pub const ROOT: &str = "/org/gh0st/keyboard";
pub const HID: &str = "/org/gh0st/keyboard/hid";
pub const INFO: &str = "/org/gh0st/keyboard/hid/info";
pub const MAP: &str = "/org/gh0st/keyboard/hid/map";
pub const CONTROL: &str = "/org/gh0st/keyboard/hid/control";
pub const MODE: &str = "/org/gh0st/keyboard/hid/mode";
pub const KEYBOARD: &str = "/org/gh0st/keyboard/hid/keyboard";
pub const LEDS: &str = "/org/gh0st/keyboard/hid/leds";
pub const CONSUMER: &str = "/org/gh0st/keyboard/hid/consumer";
pub const MOUSE: &str = "/org/gh0st/keyboard/hid/mouse";
pub const BOOT_IN: &str = "/org/gh0st/keyboard/hid/boot_in";
pub const BOOT_OUT: &str = "/org/gh0st/keyboard/hid/boot_out";
pub const BATTERY: &str = "/org/gh0st/keyboard/battery";
pub const LEVEL: &str = "/org/gh0st/keyboard/battery/level";
pub const ADVERT: &str = "/org/gh0st/keyboard/advert";
pub const AGENT: &str = "/org/gh0st/keyboard/agent";

pub const SERVICE: &str = "org.bluez.GattService1";
pub const CHAR: &str = "org.bluez.GattCharacteristic1";
pub const DESC: &str = "org.bluez.GattDescriptor1";
pub const ADV: &str = "org.bluez.LEAdvertisement1";
pub const AGENT_IFACE: &str = "org.bluez.Agent1";

/// A Bluetooth SIG 16-bit UUID in full.
pub fn uuid(short: u16) -> String {
    format!("0000{short:04x}-0000-1000-8000-00805f9b34fb")
}

pub const HID_UUID: u16 = 0x1812;
/// The advert's appearance: a keyboard.
pub const APPEARANCE: u16 = 0x03c1;

/// HID Information: HID 1.11, no country, remote wake and normally
/// connectable.
const HID_INFO: [u8; 4] = [0x11, 0x01, 0x00, 0x03];

/// Reads of the HID service need an encrypted link: a host must pair
/// before it can see the keyboard, as iOS expects.
const READ: &str = "encrypt-read";

fn service(t: &mut Tree, path: &str, short: u16) {
    t.add(
        path,
        SERVICE,
        vec![
            ("UUID", Value::str(uuid(short))),
            ("Primary", Value::Bool(true)),
            ("Includes", Value::paths::<&str>([])),
        ],
    );
}

fn chr(t: &mut Tree, path: &str, service: &str, short: u16, flags: &[&str], value: Vec<u8>) {
    t.add(
        path,
        CHAR,
        vec![
            ("UUID", Value::str(uuid(short))),
            ("Service", Value::path(service)),
            ("Flags", Value::strs(flags.iter().copied())),
            ("Value", Value::Bytes(value)),
        ],
    );
}

/// A Report characteristic and its Report Reference: the report ID and
/// 1 for input, 2 for output.
fn report(t: &mut Tree, path: &str, flags: &[&str], len: usize, id: u8, kind: u8) {
    chr(t, path, HID, 0x2a4d, flags, vec![0; len]);
    t.add(
        &format!("{path}/ref"),
        DESC,
        vec![
            ("UUID", Value::str(uuid(0x2908))),
            ("Characteristic", Value::path(path)),
            ("Flags", Value::strs([READ])),
            ("Value", Value::Bytes(vec![id, kind])),
        ],
    );
}

pub fn tree() -> Tree {
    let mut t = Tree::new(ROOT);
    service(&mut t, HID, HID_UUID);
    chr(&mut t, INFO, HID, 0x2a4a, &[READ], HID_INFO.to_vec());
    chr(&mut t, MAP, HID, 0x2a4b, &[READ], desc::ble_map());
    chr(
        &mut t,
        CONTROL,
        HID,
        0x2a4c,
        &["write-without-response"],
        vec![0],
    );
    // 1: report protocol. A host that wants boot protocol writes 0.
    chr(
        &mut t,
        MODE,
        HID,
        0x2a4e,
        &[READ, "write-without-response"],
        vec![1],
    );
    let input = [READ, "notify"];
    report(&mut t, KEYBOARD, &input, 8, desc::ID_KEYBOARD, 1);
    let output = [READ, "encrypt-write", "write-without-response"];
    report(&mut t, LEDS, &output, 1, desc::ID_KEYBOARD, 2);
    report(&mut t, CONSUMER, &input, 2, desc::ID_CONSUMER, 1);
    report(&mut t, MOUSE, &input, 5, desc::ID_MOUSE, 1);
    chr(&mut t, BOOT_IN, HID, 0x2a22, &input, vec![0; 8]);
    chr(&mut t, BOOT_OUT, HID, 0x2a32, &output, vec![0]);
    service(&mut t, BATTERY, 0x180f);
    t.add(
        LEVEL,
        CHAR,
        vec![
            ("UUID", Value::str(uuid(0x2a19))),
            ("Service", Value::path(BATTERY)),
            ("Flags", Value::strs(["read", "notify"])),
            ("Value", Value::Bytes(vec![100])),
        ],
    );
    t.add(AGENT, AGENT_IFACE, vec![]);
    t
}

/// The advert's properties. Only the pairing advert is discoverable and
/// carries the name; a bonded host finds its keyboard without either.
pub fn advert(t: &mut Tree, pairing: bool, name: &str, timeout: u16) {
    let mut props = vec![
        ("Type", Value::str("peripheral")),
        ("ServiceUUIDs", Value::strs([uuid(HID_UUID)])),
        ("Appearance", Value::U16(APPEARANCE)),
        ("Discoverable", Value::Bool(pairing)),
        ("Timeout", Value::U16(timeout)),
    ];
    if pairing {
        props.push(("LocalName", Value::str(name)));
    }
    t.add(ADVERT, ADV, props);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_report_has_its_reference() {
        let t = tree();
        for (path, id, kind) in [
            (KEYBOARD, 1u8, 1u8),
            (LEDS, 1, 2),
            (CONSUMER, 2, 1),
            (MOUSE, 3, 1),
        ] {
            let r = t.get(&format!("{path}/ref"), DESC, "Value").unwrap();
            assert_eq!(r, &Value::Bytes(vec![id, kind]), "{path}");
        }
        assert_eq!(uuid(0x1812), "00001812-0000-1000-8000-00805f9b34fb");
    }

    #[test]
    fn only_the_pairing_advert_is_found_by_name() {
        let mut t = tree();
        advert(&mut t, false, "zero", 60);
        assert!(t.get(ADVERT, ADV, "LocalName").is_none());
        assert_eq!(
            t.get(ADVERT, ADV, "Discoverable"),
            Some(&Value::Bool(false))
        );
        advert(&mut t, true, "zero", 180);
        assert_eq!(t.get(ADVERT, ADV, "LocalName"), Some(&Value::str("zero")));
    }
}
