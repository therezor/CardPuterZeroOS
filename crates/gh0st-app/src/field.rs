//! A one-line text input.

use crate::key::Key;

/// What a key did to a [`TextField`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Changed,
    /// Enter.
    Submit,
}

/// A line of text the user types. While a page has one focused, its
/// [`Page::typing`](crate::Page::typing) should return true, so letters and
/// digits type instead of moving or pressing soft keys.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    text: String,
    masked: bool,
}

impl TextField {
    pub fn new() -> TextField {
        TextField::default()
    }

    /// Shows dots instead of the text, for passwords.
    pub fn masked() -> TextField {
        TextField {
            text: String::new(),
            masked: true,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// Shows or hides a masked field's text.
    pub fn reveal(&mut self, show: bool) {
        self.masked = !show;
    }

    /// Types, deletes and submits. Other keys return `None`.
    pub fn key(&mut self, key: Key) -> Option<Edit> {
        match key {
            Key::Char(c) => self.text.push(c),
            Key::Backspace => {
                self.text.pop()?;
            }
            Key::Enter => return Some(Edit::Submit),
            _ => return None,
        }
        Some(Edit::Changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_deletes_and_submits() {
        let mut f = TextField::new();
        assert_eq!(f.key(Key::Char('h')), Some(Edit::Changed));
        f.key(Key::Char('i'));
        f.key(Key::Backspace);
        assert_eq!(f.text(), "h");
        assert_eq!(f.key(Key::Enter), Some(Edit::Submit));
        assert_eq!(f.key(Key::Up), None);
    }
}
