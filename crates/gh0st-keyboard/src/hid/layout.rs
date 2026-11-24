//! Characters to keys. A HID keyboard sends places, not characters, so to
//! type "@" the app must say which place and which modifiers give "@". The
//! Cardputer is a US keyboard and tells every host so: one layout, US.
//! Letters, digits and the keyboard's own keys go by place and never come
//! here; the Sym layer's characters and typed text do.

use super::usage::{key, mods};

/// One key press that types a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub usage: u8,
    pub mods: u8,
}

/// Usage, then the character it types plain and with Shift.
const ROWS: &[(u8, char, char)] = &[
    (0x1e, '1', '!'),
    (0x1f, '2', '@'),
    (0x20, '3', '#'),
    (0x21, '4', '$'),
    (0x22, '5', '%'),
    (0x23, '6', '^'),
    (0x24, '7', '&'),
    (0x25, '8', '*'),
    (0x26, '9', '('),
    (0x27, '0', ')'),
    (0x2d, '-', '_'),
    (0x2e, '=', '+'),
    (0x2f, '[', '{'),
    (0x30, ']', '}'),
    (0x31, '\\', '|'),
    (0x33, ';', ':'),
    (0x34, '\'', '"'),
    (0x35, '`', '~'),
    (0x36, ',', '<'),
    (0x37, '.', '>'),
    (0x38, '/', '?'),
];

/// The key that types `c` on a US layout, if any.
pub fn stroke(c: char) -> Option<Stroke> {
    let plain = |usage| Some(Stroke { usage, mods: 0 });
    match c {
        ' ' => return plain(key::SPACE),
        '\n' => return plain(key::ENTER),
        '\t' => return plain(key::TAB),
        'a'..='z' => return plain(key::A + (c as u8 - b'a')),
        'A'..='Z' => {
            return Some(Stroke {
                usage: key::A + (c as u8 - b'A'),
                mods: mods::LSHIFT,
            });
        }
        _ => {}
    }
    ROWS.iter().find_map(|&(usage, p, s)| {
        if c == p {
            Some(Stroke { usage, mods: 0 })
        } else if c == s {
            Some(Stroke {
                usage,
                mods: mods::LSHIFT,
            })
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_go_by_their_us_place() {
        assert_eq!(
            stroke('@'),
            Some(Stroke {
                usage: 0x1f,
                mods: mods::LSHIFT
            })
        );
        assert_eq!(stroke('/').map(|s| s.mods), Some(0));
        assert_eq!(
            stroke('A'),
            Some(Stroke {
                usage: key::A,
                mods: mods::LSHIFT
            })
        );
        assert_eq!(stroke('\n').map(|s| s.usage), Some(key::ENTER));
        assert_eq!(stroke('é'), None);
    }

    /// Every printable ASCII character has a key, and no two share one.
    #[test]
    fn ascii_types_once() {
        let mut seen = std::collections::HashMap::new();
        for c in (0x20u8..0x7f).map(char::from) {
            let s = stroke(c).unwrap_or_else(|| panic!("no key for {c:?}"));
            if let Some(other) = seen.insert((s.usage, s.mods), c) {
                panic!("{c:?} and {other:?} on one key");
            }
        }
    }
}
