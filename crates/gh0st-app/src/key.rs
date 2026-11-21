//! Keys as an app reads them. The SDK turns raw evdev codes into these, with
//! the CardputerZero's layers already applied.

use gh0st_core::keys::{self, State, code};

/// One key press. Held keys repeat as more presses of the same key; the SDK
/// sends releases to no one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Fn+F, or F when the page is not typing.
    Up,
    /// Fn+X, or X when the page is not typing.
    Down,
    /// Fn+Z, or Z when the page is not typing.
    Left,
    /// Fn+C, or C when the page is not typing.
    Right,
    Enter,
    /// Esc, or 4 when not typing, as template apps have it. The SDK goes
    /// back a page on it unless [`Page::back`] says no; on the first page,
    /// held 0.7 s, it closes the app.
    ///
    /// [`Page::back`]: crate::Page::back
    Back,
    Tab,
    Backspace,
    Delete,
    /// A number key 5 to 7 under a chip the page shows.
    Soft(u8),
    /// A typed character, with Shift, Caps and the Sym layer applied.
    Char(char),
    /// Ctrl and a letter or digit: `Ctrl('s')`.
    Ctrl(char),
    /// Alt and a letter or digit.
    Alt(char),
    /// Any other key, by Linux input code. The brightness and volume keys
    /// arrive here after the OS has acted on them.
    Other(u16),
}

/// Input codes for [`Key::Other`].
pub use gh0st_core::keys::code as codes;

/// Maps a raw key to what the page sees. `typing` keeps letters and digits as
/// characters; `chips` says which of keys 5 to 7 have a chip.
pub(crate) fn map(raw: &keys::Key, typing: bool, chips: [bool; 3]) -> Option<Key> {
    if raw.state == State::Release || is_modifier(raw.code) {
        return None;
    }
    let repeat = raw.state == State::Repeat;
    if let Some(nav) = raw.nav(!typing && !raw.ctrl && !raw.alt) {
        return Some(match nav {
            keys::Nav::Up => Key::Up,
            keys::Nav::Down => Key::Down,
            keys::Nav::Left => Key::Left,
            keys::Nav::Right => Key::Right,
        });
    }
    let key = match raw.code {
        code::ESC => Key::Back,
        code::ENTER => Key::Enter,
        code::TAB => Key::Tab,
        code::BACKSPACE => Key::Backspace,
        code::DELETE => Key::Delete,
        c if raw.ctrl => Key::Ctrl(keys::base_char(c)?),
        c if raw.alt => Key::Alt(keys::base_char(c)?),
        c => match raw.ch {
            Some('4') if !typing => Key::Back,
            Some(d @ '5'..='7') if !typing && chips[(d as u8 - b'5') as usize] => {
                Key::Soft(d as u8 - b'0')
            }
            Some(ch) => Key::Char(ch),
            None => Key::Other(c),
        },
    };
    let repeats = matches!(key, Key::Backspace | Key::Delete | Key::Char(_));
    (!repeat || repeats).then_some(key)
}

fn is_modifier(c: u16) -> bool {
    matches!(
        c,
        code::LEFTSHIFT
            | code::RIGHTSHIFT
            | code::LEFTCTRL
            | code::RIGHTCTRL
            | code::LEFTALT
            | code::RIGHTALT
            | code::CAPSLOCK
            | code::FN
            | code::FN_M5
            | code::SYM_M5
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_core::keys::Keyboard;

    fn press(kb: &mut Keyboard, c: u16) -> keys::Key {
        kb.event(c, 1, 0).unwrap()
    }

    #[test]
    fn letters_move_unless_typing() {
        let mut kb = Keyboard::new(Vec::new());
        let x = press(&mut kb, code::X);
        assert_eq!(map(&x, false, [false; 3]), Some(Key::Down));
        assert_eq!(map(&x, true, [false; 3]), Some(Key::Char('x')));
    }

    #[test]
    fn digits_run_chips_only_where_one_shows_and_four_is_esc() {
        let mut kb = Keyboard::new(Vec::new());
        let six = press(&mut kb, 7);
        let chips = [false, true, false];
        assert_eq!(map(&six, false, chips), Some(Key::Soft(6)));
        assert_eq!(map(&six, true, chips), Some(Key::Char('6')));
        let five = press(&mut kb, 6);
        assert_eq!(map(&five, false, chips), Some(Key::Char('5')));
        let four = press(&mut kb, 5);
        assert_eq!(map(&four, false, chips), Some(Key::Back));
        assert_eq!(map(&four, true, chips), Some(Key::Char('4')));
    }

    #[test]
    fn ctrl_letters_and_esc() {
        let mut kb = Keyboard::new(Vec::new());
        press(&mut kb, code::LEFTCTRL);
        let s = press(&mut kb, 31);
        assert_eq!(map(&s, false, [false; 3]), Some(Key::Ctrl('s')));
        let esc = press(&mut kb, code::ESC);
        assert_eq!(map(&esc, false, [false; 3]), Some(Key::Back));
    }
}
