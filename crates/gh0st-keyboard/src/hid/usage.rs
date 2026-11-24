//! HID usages, and the keyboard's Linux codes that map to them by place.

use gh0st_core::keys::code;

/// Modifier bits in a keyboard report's first byte.
pub mod mods {
    pub const LCTRL: u8 = 0x01;
    pub const LSHIFT: u8 = 0x02;
    pub const LALT: u8 = 0x04;
    pub const LGUI: u8 = 0x08;
    pub const RCTRL: u8 = 0x10;
    pub const RSHIFT: u8 = 0x20;
    /// AltGr on PC layouts, Option on a Mac.
    pub const RALT: u8 = 0x40;
    pub const RGUI: u8 = 0x80;
    pub const SHIFT: u8 = LSHIFT | RSHIFT;
}

/// Keyboard page usages the app names.
pub mod key {
    pub const A: u8 = 0x04;
    pub const B: u8 = 0x05;
    pub const C: u8 = 0x06;
    pub const D: u8 = 0x07;
    pub const E: u8 = 0x08;
    pub const F: u8 = 0x09;
    pub const L: u8 = 0x0f;
    pub const N: u8 = 0x11;
    pub const P: u8 = 0x13;
    pub const Q: u8 = 0x14;
    pub const R: u8 = 0x15;
    pub const S: u8 = 0x16;
    pub const T: u8 = 0x17;
    pub const V: u8 = 0x19;
    pub const W: u8 = 0x1a;
    pub const X: u8 = 0x1b;
    pub const Y: u8 = 0x1c;
    pub const Z: u8 = 0x1d;
    pub const N3: u8 = 0x20;
    pub const N4: u8 = 0x21;
    pub const N5: u8 = 0x22;
    pub const ENTER: u8 = 0x28;
    pub const ESC: u8 = 0x29;
    pub const BACKSPACE: u8 = 0x2a;
    pub const TAB: u8 = 0x2b;
    pub const SPACE: u8 = 0x2c;
    pub const CAPS: u8 = 0x39;
    pub const F1: u8 = 0x3a;
    pub const F4: u8 = 0x3d;
    pub const F5: u8 = 0x3e;
    pub const F11: u8 = 0x44;
    pub const F12: u8 = 0x45;
    pub const PRINT: u8 = 0x46;
    pub const INSERT: u8 = 0x49;
    pub const HOME: u8 = 0x4a;
    pub const PGUP: u8 = 0x4b;
    pub const DELETE: u8 = 0x4c;
    pub const END: u8 = 0x4d;
    pub const PGDN: u8 = 0x4e;
    pub const RIGHT: u8 = 0x4f;
    pub const LEFT: u8 = 0x50;
    pub const DOWN: u8 = 0x51;
    pub const UP: u8 = 0x52;
    pub const APPLICATION: u8 = 0x65;
}

/// Consumer page usages.
pub mod consumer {
    pub const BRIGHTNESS_UP: u16 = 0x006f;
    pub const BRIGHTNESS_DOWN: u16 = 0x0070;
    pub const NEXT: u16 = 0x00b5;
    pub const PREV: u16 = 0x00b6;
    pub const STOP: u16 = 0x00b7;
    pub const PLAY_PAUSE: u16 = 0x00cd;
    pub const MUTE: u16 = 0x00e2;
    pub const VOLUME_UP: u16 = 0x00e9;
    pub const VOLUME_DOWN: u16 = 0x00ea;
    /// Android's Back and iOS's Home: AC Back and AC Home.
    pub const AC_HOME: u16 = 0x0223;
    pub const AC_BACK: u16 = 0x0224;
}

/// Usages of the keys this keyboard has, by Linux code. Letters and digits
/// go by place, as on any US keyboard: the host's layout picks the
/// character. The Sym layer's codes are not here; they go by character.
pub fn by_code(c: u16) -> Option<u8> {
    const QWERTY: [u8; 10] = [0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13];
    const ASDF: [u8; 9] = [0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f];
    const ZXCV: [u8; 7] = [0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10];
    Some(match c {
        code::ESC => key::ESC,
        2..=11 => 0x1e + (c - 2) as u8,
        code::BACKSPACE => key::BACKSPACE,
        code::TAB => key::TAB,
        16..=25 => QWERTY[(c - 16) as usize],
        code::ENTER => key::ENTER,
        30..=38 => ASDF[(c - 30) as usize],
        44..=50 => ZXCV[(c - 44) as usize],
        code::SPACE => key::SPACE,
        code::CAPSLOCK => key::CAPS,
        59..=68 => key::F1 + (c - 59) as u8,
        87 => key::F11,
        88 => key::F12,
        code::UP => key::UP,
        code::DOWN => key::DOWN,
        code::LEFT => key::LEFT,
        code::RIGHT => key::RIGHT,
        code::PAGEUP => key::PGUP,
        code::PAGEDOWN => key::PGDN,
        code::END => key::END,
        code::INSERT => key::INSERT,
        code::DELETE => key::DELETE,
        code::COMPOSE => key::APPLICATION,
        _ => return None,
    })
}

/// The media keys on Fn+Q, W and E (and Stop), as consumer usages.
pub fn media(c: u16) -> Option<u16> {
    Some(match c {
        163 => consumer::NEXT,
        164 => consumer::PLAY_PAUSE,
        165 => consumer::PREV,
        166 => consumer::STOP,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_digits_go_by_place() {
        assert_eq!(by_code(30), Some(key::A));
        assert_eq!(by_code(44), Some(key::Z));
        assert_eq!(by_code(21), Some(key::Y));
        assert_eq!(by_code(2), Some(0x1e)); // 1
        assert_eq!(by_code(11), Some(0x27)); // 0
        assert_eq!(by_code(50), Some(0x10)); // m
        assert_eq!(by_code(68), Some(0x43)); // F10
    }

    #[test]
    fn os_and_symbol_codes_have_no_place() {
        assert_eq!(by_code(code::HOME), None);
        assert_eq!(by_code(26), None); // the Sym layer's '!'
        assert_eq!(by_code(code::VOLUMEUP), None);
    }
}
