//! The lock screen's keys. It rests until Tab, then Enter, so a device in a
//! pocket types nothing; only then does a password field take keys. No I/O
//! here: the shell runs the check and keeps the try count.

use gh0st_core::keys::{Key, State, code};
use gh0st_core::password::wipe;

/// Enter must follow Tab within this.
pub const TAB_ENTER_MS: u64 = 2000;
/// The field goes back to rest after this without a key.
pub const FIELD_IDLE_MS: u64 = 10_000;
const MAX_PASSWORD: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    /// Tab then Enter with no password to ask.
    Unlock,
    /// Check this password. The caller wipes it.
    Check(Vec<u8>),
}

#[derive(Default)]
pub struct Lock {
    /// The password field is open.
    field: bool,
    tab_at: Option<u64>,
    buf: Vec<u8>,
    /// A check is running: the field takes no keys.
    pub checking: bool,
    /// The last check said no.
    pub wrong: bool,
    last_key: u64,
}

impl Lock {
    /// Back to rest, the field wiped.
    pub fn rest(&mut self) {
        self.field = false;
        self.tab_at = None;
        self.checking = false;
        self.wrong = false;
        self.clear();
    }

    pub fn field_open(&self) -> bool {
        self.field
    }

    pub fn typed(&self) -> usize {
        std::str::from_utf8(&self.buf).map_or(self.buf.len(), |s| s.chars().count())
    }

    /// A key on the lock screen. `password`: Tab then Enter opens the field
    /// instead of unlocking. `waiting`: wrong tries ran out for now, so the
    /// field takes no password.
    pub fn key(&mut self, k: &Key, now: u64, password: bool, waiting: bool) -> Action {
        if !self.field {
            return self.rest_key(k, now, password);
        }
        if k.state == State::Release || self.checking {
            return Action::None;
        }
        self.last_key = now;
        if k.is(code::ESC) && k.state == State::Press {
            self.rest();
            return Action::Redraw;
        }
        if waiting {
            return Action::None;
        }
        if k.is(code::ENTER) {
            if k.state != State::Press || self.buf.is_empty() {
                return Action::None;
            }
            self.checking = true;
            self.wrong = false;
            return Action::Check(std::mem::take(&mut self.buf));
        }
        if k.is(code::BACKSPACE) {
            if let Some(i) = std::str::from_utf8(&self.buf)
                .ok()
                .and_then(|s| s.char_indices().last().map(|(i, _)| i))
            {
                wipe(&mut self.buf[i..]);
                self.buf.truncate(i);
            }
            return Action::Redraw;
        }
        match k.ch {
            Some(ch) if !ch.is_control() => {
                let mut b = [0u8; 4];
                let s = ch.encode_utf8(&mut b);
                if self.buf.len() + s.len() <= MAX_PASSWORD {
                    self.buf.extend_from_slice(s.as_bytes());
                }
                wipe(&mut b);
                self.wrong = false;
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    fn rest_key(&mut self, k: &Key, now: u64, password: bool) -> Action {
        if k.state != State::Press {
            return Action::None;
        }
        if k.is(code::TAB) {
            self.tab_at = Some(now);
            return Action::None;
        }
        let tab = self.tab_at.take();
        if !k.is(code::ENTER) || !tab.is_some_and(|t| now - t <= TAB_ENTER_MS) {
            return Action::None;
        }
        if !password {
            return Action::Unlock;
        }
        self.field = true;
        self.last_key = now;
        Action::Redraw
    }

    /// The check answered.
    pub fn checked(&mut self, ok: bool, now: u64) {
        self.checking = false;
        self.last_key = now;
        if ok {
            self.rest();
        } else {
            self.wrong = true;
        }
    }

    /// When the open field goes back to rest.
    pub fn next_at(&self) -> Option<u64> {
        (self.field && !self.checking).then_some(self.last_key + FIELD_IDLE_MS)
    }

    /// Closes a field left alone. True when it did.
    pub fn tick(&mut self, now: u64) -> bool {
        if self.next_at().is_some_and(|t| now >= t) {
            self.rest();
            return true;
        }
        false
    }

    fn clear(&mut self) {
        wipe(&mut self.buf);
        self.buf.clear();
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16, state: State, ch: Option<char>) -> Key {
        Key {
            code,
            state,
            ch,
            text: ch,
            shift: false,
            ctrl: false,
            alt: false,
            fn_: false,
        }
    }

    fn press(code: u16) -> Key {
        key(code, State::Press, None)
    }

    fn typed(l: &mut Lock, s: &str, now: u64) {
        for ch in s.chars() {
            l.key(&key(0, State::Press, Some(ch)), now, true, false);
        }
    }

    #[test]
    fn rest_takes_no_key_but_tab_then_enter() {
        let mut l = Lock::default();
        typed(&mut l, "hunter2", 0);
        assert_eq!(l.key(&press(code::ENTER), 10, true, false), Action::None);
        assert!(!l.field_open(), "a pocket types nothing");
        l.key(&press(code::TAB), 20, true, false);
        assert_eq!(l.key(&press(code::ENTER), 30, true, false), Action::Redraw);
        assert!(l.field_open());
        assert_eq!(l.typed(), 0, "keys at rest were dropped");
    }

    #[test]
    fn tab_then_enter_unlocks_without_a_password() {
        let mut l = Lock::default();
        l.key(&press(code::TAB), 0, false, false);
        assert_eq!(
            l.key(&press(code::ENTER), 100, false, false),
            Action::Unlock
        );
    }

    #[test]
    fn enter_must_follow_tab_at_once() {
        let mut l = Lock::default();
        l.key(&press(code::TAB), 0, false, false);
        assert_eq!(
            l.key(&press(code::ENTER), TAB_ENTER_MS + 1, false, false),
            Action::None
        );
        l.key(&press(code::TAB), 5000, false, false);
        l.key(&key(0, State::Press, Some('x')), 5001, false, false);
        assert_eq!(
            l.key(&press(code::ENTER), 5002, false, false),
            Action::None,
            "a key between"
        );
    }

    #[test]
    fn the_field_checks_what_was_typed() {
        let mut l = Lock::default();
        l.key(&press(code::TAB), 0, true, false);
        l.key(&press(code::ENTER), 1, true, false);
        typed(&mut l, "pass", 2);
        l.key(&press(code::BACKSPACE), 3, true, false);
        typed(&mut l, "sé", 4);
        assert_eq!(l.typed(), 5);
        assert_eq!(
            l.key(&press(code::ENTER), 5, true, false),
            Action::Check("passé".as_bytes().to_vec())
        );
        assert!(l.checking);
        typed(&mut l, "x", 6);
        assert_eq!(l.typed(), 0, "no keys while checking");
        l.checked(false, 7);
        assert!(l.wrong && l.field_open());
        l.checked(true, 8);
        assert!(!l.field_open());
    }

    #[test]
    fn a_wait_takes_no_password_but_esc_still_goes_back() {
        let mut l = Lock::default();
        l.key(&press(code::TAB), 0, true, false);
        l.key(&press(code::ENTER), 1, true, false);
        l.key(&key(0, State::Press, Some('a')), 2, true, true);
        assert_eq!(l.typed(), 0);
        assert_eq!(l.key(&press(code::ESC), 3, true, true), Action::Redraw);
        assert!(!l.field_open());
    }

    #[test]
    fn a_field_left_alone_goes_back_to_rest() {
        let mut l = Lock::default();
        l.key(&press(code::TAB), 0, true, false);
        l.key(&press(code::ENTER), 100, true, false);
        typed(&mut l, "abc", 200);
        assert_eq!(l.next_at(), Some(200 + FIELD_IDLE_MS));
        assert!(!l.tick(200 + FIELD_IDLE_MS - 1));
        assert!(l.tick(200 + FIELD_IDLE_MS));
        assert!(!l.field_open());
        assert_eq!(l.typed(), 0);
    }
}
