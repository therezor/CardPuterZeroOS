//! HID report descriptors for the keyboard app. Bluetooth gets one map
//! with a report ID per kind; USB gets three functions with no IDs, so a
//! PC's firmware can use the keyboard and the mouse in their boot forms.
//! Here, not in the app, because the root helper builds the USB gadget
//! from them.

/// Report IDs in the Bluetooth map.
pub const ID_KEYBOARD: u8 = 1;
pub const ID_CONSUMER: u8 = 2;
pub const ID_MOUSE: u8 = 3;

macro_rules! keyboard {
    ($($id:tt)*) => {
        [
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x06, // Usage (Keyboard)
            0xa1, 0x01, // Collection (Application)
            $($id)*
            0x05, 0x07, //   Usage Page (Keyboard)
            0x19, 0xe0, 0x29, 0xe7, // Usage Min/Max: the 8 modifiers
            0x15, 0x00, 0x25, 0x01, // Logical 0..1
            0x75, 0x01, 0x95, 0x08, // 8 x 1 bit
            0x81, 0x02, //   Input (Data, Var, Abs)
            0x75, 0x08, 0x95, 0x01, // 1 x 8 bits
            0x81, 0x01, //   Input (Const): reserved
            0x05, 0x08, //   Usage Page (LEDs)
            0x19, 0x01, 0x29, 0x05, // Num, Caps, Scroll, Compose, Kana
            0x75, 0x01, 0x95, 0x05, // 5 x 1 bit
            0x91, 0x02, //   Output (Data, Var, Abs)
            0x75, 0x03, 0x95, 0x01, // 1 x 3 bits
            0x91, 0x01, //   Output (Const): padding
            0x05, 0x07, //   Usage Page (Keyboard)
            0x19, 0x00, 0x29, 0x65, // Usage 0..101
            0x15, 0x00, 0x25, 0x65, // Logical 0..101
            0x75, 0x08, 0x95, 0x06, // 6 x 8 bits
            0x81, 0x00, //   Input (Data, Array)
            0xc0,       // End Collection
        ]
    };
}

macro_rules! consumer {
    ($($id:tt)*) => {
        [
            0x05, 0x0c, // Usage Page (Consumer)
            0x09, 0x01, // Usage (Consumer Control)
            0xa1, 0x01, // Collection (Application)
            $($id)*
            0x19, 0x00, 0x2a, 0xff, 0x03, // Usage 0..0x3ff
            0x15, 0x00, 0x26, 0xff, 0x03, // Logical 0..0x3ff
            0x75, 0x10, 0x95, 0x01, // 1 x 16 bits
            0x81, 0x00, //   Input (Data, Array)
            0xc0,
        ]
    };
}

macro_rules! mouse {
    ($($id:tt)*) => {
        [
            0x05, 0x01, // Usage Page (Generic Desktop)
            0x09, 0x02, // Usage (Mouse)
            0xa1, 0x01, // Collection (Application)
            $($id)*
            0x09, 0x01, //   Usage (Pointer)
            0xa1, 0x00, //   Collection (Physical)
            0x05, 0x09, //     Usage Page (Buttons)
            0x19, 0x01, 0x29, 0x05, // Buttons 1..5
            0x15, 0x00, 0x25, 0x01, // Logical 0..1
            0x75, 0x01, 0x95, 0x05, // 5 x 1 bit
            0x81, 0x02, //     Input (Data, Var, Abs)
            0x75, 0x03, 0x95, 0x01, // 1 x 3 bits
            0x81, 0x01, //     Input (Const): padding
            0x05, 0x01, //     Usage Page (Generic Desktop)
            0x09, 0x30, 0x09, 0x31, 0x09, 0x38, // X, Y, Wheel
            0x15, 0x81, 0x25, 0x7f, // Logical -127..127
            0x75, 0x08, 0x95, 0x03, // 3 x 8 bits
            0x81, 0x06, //     Input (Data, Var, Rel)
            0x05, 0x0c, //     Usage Page (Consumer)
            0x0a, 0x38, 0x02, // Usage (AC Pan)
            0x15, 0x81, 0x25, 0x7f, // Logical -127..127
            0x75, 0x08, 0x95, 0x01, // 1 x 8 bits
            0x81, 0x06, //     Input (Data, Var, Rel)
            0xc0,       //   End Collection
            0xc0,       // End Collection
        ]
    };
}

/// The Bluetooth report map: keyboard, consumer control and mouse, each
/// with its own report ID. iOS mishandles modifiers when a keyboard and a
/// mouse share one report.
pub fn ble_map() -> Vec<u8> {
    let mut map = Vec::with_capacity(200);
    map.extend_from_slice(&keyboard!(0x85, ID_KEYBOARD,));
    map.extend_from_slice(&consumer!(0x85, ID_CONSUMER,));
    map.extend_from_slice(&mouse!(0x85, ID_MOUSE,));
    map
}

/// USB's boot keyboard: 8-byte reports in, 1 LED byte out.
pub const USB_KEYBOARD: &[u8] = &keyboard!();
/// USB's mouse: 5-byte reports, the first three in boot form.
pub const USB_MOUSE: &[u8] = &mouse!();
/// USB's consumer control: one 16-bit usage.
pub const USB_CONSUMER: &[u8] = &consumer!();

#[cfg(test)]
mod tests {
    use super::*;

    /// Walks the short items and checks collections balance and every item
    /// has its data bytes.
    fn well_formed(d: &[u8]) -> bool {
        let (mut i, mut depth) = (0usize, 0i32);
        while i < d.len() {
            let size = match d[i] & 3 {
                3 => 4,
                n => n as usize,
            };
            match d[i] & 0xfc {
                0xa0 => depth += 1,
                0xc0 => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return false;
            }
            i += 1 + size;
        }
        i == d.len() && depth == 0
    }

    #[test]
    fn descriptors_are_well_formed() {
        assert!(well_formed(&ble_map()));
        assert!(well_formed(USB_KEYBOARD));
        assert!(well_formed(USB_MOUSE));
        assert!(well_formed(USB_CONSUMER));
    }

    #[test]
    fn the_ble_map_names_each_report_id_once() {
        let map = ble_map();
        let ids: Vec<u8> = map
            .windows(2)
            .filter(|w| w[0] == 0x85)
            .map(|w| w[1])
            .collect();
        assert_eq!(ids, [ID_KEYBOARD, ID_CONSUMER, ID_MOUSE]);
        assert!(!USB_KEYBOARD.contains(&0x85));
    }
}
