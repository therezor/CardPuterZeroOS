//! Characters to keys, for the layout the host has set. A HID keyboard
//! sends places, not characters: to type "@" the app must know which place
//! and which modifiers give "@" on the host. Letters, digits and the
//! keyboard's own keys go by place and never come here. The Sym layer's
//! characters and typed text do.

use super::usage::{key, mods};

/// One key press that types a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    pub usage: u8,
    pub mods: u8,
    /// A dead key: the host waits for the next key. A space after it types
    /// the character alone.
    pub dead: bool,
}

pub struct Layout {
    /// Stored in the host's profile.
    pub id: &'static str,
    pub name: &'static str,
    /// a to z by usage, 0x04 to 0x1d.
    letters: &'static str,
    /// Usage, then the characters it types plain, with Shift, with AltGr,
    /// with Shift and AltGr. `\0` is none.
    rows: &'static [(u8, &'static str)],
    dead: &'static [char],
}

const LEVEL: [u8; 4] = [0, mods::LSHIFT, mods::RALT, mods::LSHIFT | mods::RALT];

impl Layout {
    pub fn stroke(&self, c: char) -> Option<Stroke> {
        let plain = |usage| {
            Some(Stroke {
                usage,
                mods: 0,
                dead: false,
            })
        };
        match c {
            ' ' => return plain(key::SPACE),
            '\n' => return plain(key::ENTER),
            '\t' => return plain(key::TAB),
            _ => {}
        }
        let dead = self.dead.contains(&c);
        for &(usage, chars) in self.rows {
            if let Some(level) = chars.chars().position(|x| x == c && x != '\0') {
                return Some(Stroke {
                    usage,
                    mods: LEVEL[level],
                    dead,
                });
            }
        }
        let lower = c.to_ascii_lowercase();
        let i = self.letters.chars().position(|x| x == lower)?;
        Some(Stroke {
            usage: key::A + i as u8,
            mods: if c.is_ascii_uppercase() {
                mods::LSHIFT
            } else {
                0
            },
            dead: false,
        })
    }
}

const QWERTY: &str = "abcdefghijklmnopqrstuvwxyz";

pub static US: Layout = Layout {
    id: "us",
    name: "US",
    letters: QWERTY,
    rows: &[
        (0x1e, "1!"),
        (0x1f, "2@"),
        (0x20, "3#"),
        (0x21, "4$"),
        (0x22, "5%"),
        (0x23, "6^"),
        (0x24, "7&"),
        (0x25, "8*"),
        (0x26, "9("),
        (0x27, "0)"),
        (0x2d, "-_"),
        (0x2e, "=+"),
        (0x2f, "[{"),
        (0x30, "]}"),
        (0x31, "\\|"),
        (0x33, ";:"),
        (0x34, "'\""),
        (0x35, "`~"),
        (0x36, ",<"),
        (0x37, ".>"),
        (0x38, "/?"),
    ],
    dead: &[],
};

/// Every layout a host can pick, US first: it is also by place.
pub static LAYOUTS: &[&Layout] = &[&US];

/// The layout with this id, else US.
pub fn by_id(id: &str) -> &'static Layout {
    LAYOUTS.iter().copied().find(|l| l.id == id).unwrap_or(&US)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_types_by_place() {
        let s = US.stroke('@').unwrap();
        assert_eq!((s.usage, s.mods), (0x1f, mods::LSHIFT));
        assert_eq!(US.stroke('/').unwrap().mods, 0);
        assert_eq!(
            US.stroke('A').unwrap(),
            Stroke {
                usage: key::A,
                mods: mods::LSHIFT,
                dead: false
            }
        );
        assert_eq!(US.stroke('\n').unwrap().usage, key::ENTER);
        assert_eq!(US.stroke('é'), None);
    }

    #[test]
    fn unknown_ids_fall_back_to_us() {
        assert_eq!(by_id("xx").id, "us");
    }
}
