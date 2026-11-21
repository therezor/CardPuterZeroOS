//! Text fields that fill a page, with labels, Tab between them and an error
//! under each.

use crate::field::{Edit, TextField};
use crate::key::Key;

struct Entry {
    label: String,
    placeholder: String,
    field: TextField,
    /// Made as a password: Tab may show it.
    secret: bool,
    error: Option<String>,
}

/// Labelled text fields on a page: a name and a password, a date and a time.
/// One has focus. Tab moves to the next; with a single password field Tab
/// shows or hides it instead, as a dialog does. A field can say what is
/// wrong with it: a red edge and a line under it, until it is typed in.
///
/// While a page shows a form, its [`Page::typing`] returns true and its bars
/// carry [`hints`](Form::hints). Draw it with [`Ui::form`].
///
/// ```
/// use gh0st_app::prelude::*;
///
/// let mut f = Form::new().text("Your name").password("Password, 4 or more");
/// f.key(Key::Char('a'));
/// f.key(Key::Tab);
/// f.key(Key::Char('x'));
/// assert_eq!(f.value(0), "a");
/// assert_eq!(f.focus(), 1);
/// f.error(1, "Too short: 4 or more");
/// assert_eq!(f.key(Key::Enter), Some(Edit::Submit));
/// ```
///
/// [`Page::typing`]: crate::Page::typing
/// [`Ui::form`]: crate::Ui::form
#[derive(Default)]
pub struct Form {
    entries: Vec<Entry>,
    focus: usize,
}

impl Form {
    pub fn new() -> Form {
        Form::default()
    }

    /// Adds a field for plain text.
    pub fn text(self, label: &str) -> Form {
        self.add(label, TextField::new(), false)
    }

    /// Adds a field that shows dots.
    pub fn password(self, label: &str) -> Form {
        self.add(label, TextField::masked(), true)
    }

    fn add(mut self, label: &str, field: TextField, secret: bool) -> Form {
        self.entries.push(Entry {
            label: label.to_string(),
            placeholder: String::new(),
            field,
            secret,
            error: None,
        });
        self
    }

    /// Fills in the last field added.
    pub fn with(mut self, text: &str) -> Form {
        if let Some(e) = self.entries.last_mut() {
            e.field.set(text);
        }
        self
    }

    /// Grey text in the last field added while it is empty.
    pub fn placeholder(mut self, text: &str) -> Form {
        if let Some(e) = self.entries.last_mut() {
            e.placeholder = text.to_string();
        }
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn value(&self, i: usize) -> &str {
        self.entries.get(i).map_or("", |e| e.field.text())
    }

    pub fn set(&mut self, i: usize, text: &str) {
        if let Some(e) = self.entries.get_mut(i) {
            e.field.set(text);
        }
    }

    /// Takes field `i`'s text out, leaving it empty. Zero a password once
    /// used: `s.into_bytes().fill(0)`.
    pub fn take(&mut self, i: usize) -> String {
        self.entries
            .get_mut(i)
            .map_or_else(String::new, |e| e.field.take())
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn set_focus(&mut self, i: usize) {
        self.focus = i.min(self.entries.len().saturating_sub(1));
    }

    /// Marks field `i` wrong and moves the focus to it.
    pub fn error(&mut self, i: usize, why: &str) {
        if let Some(e) = self.entries.get_mut(i) {
            e.error = Some(why.to_string());
            self.focus = i;
        }
    }

    pub fn clear_errors(&mut self) {
        for e in &mut self.entries {
            e.error = None;
        }
    }

    /// The single field, when it is a password: Tab shows it rather than
    /// moving.
    fn lone_secret(&self) -> Option<&Entry> {
        match self.entries.as_slice() {
            [e] if e.secret => Some(e),
            _ => None,
        }
    }

    /// Types into the focused field, moves on Tab, submits on Enter. Typing
    /// clears that field's error. Other keys return `None`.
    pub fn key(&mut self, key: Key) -> Option<Edit> {
        if key == Key::Tab {
            if self.lone_secret().is_some() {
                let f = &mut self.entries[0].field;
                f.reveal(f.is_masked());
                return Some(Edit::Changed);
            }
            if self.entries.len() < 2 {
                return None;
            }
            self.focus = (self.focus + 1) % self.entries.len();
            return Some(Edit::Changed);
        }
        let e = self.entries.get_mut(self.focus)?;
        let edit = e.field.key(key)?;
        if edit == Edit::Changed {
            e.error = None;
        }
        Some(edit)
    }

    /// The hint bar for a page with this form: Esc, Tab's job, Enter.
    pub fn hints<'a>(&self, esc: &'a str, enter: &'a str) -> Vec<(&'a str, &'a str)> {
        let mut h = vec![("Esc", esc)];
        if let Some(e) = self.lone_secret() {
            h.push(("Tab", if e.field.is_masked() { "show" } else { "hide" }));
        } else if self.entries.len() > 1 {
            h.push(("Tab", "next field"));
        }
        h.push(("Enter", enter));
        h
    }

    pub(crate) fn fields(&self) -> impl Iterator<Item = gh0st_ui::widgets::Field<'_>> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| gh0st_ui::widgets::Field {
                label: &e.label,
                text: e.field.text(),
                placeholder: &e.placeholder,
                masked: e.field.is_masked(),
                focused: i == self.focus,
                error: e.error.as_deref(),
            })
    }
}

impl Drop for Form {
    fn drop(&mut self) {
        for e in &mut self.entries {
            e.field.take().into_bytes().fill(0);
        }
    }
}

/// Labels and focus only: a field's text stays out of logs.
impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let labels: Vec<&str> = self.entries.iter().map(|e| e.label.as_str()).collect();
        f.debug_struct("Form")
            .field("fields", &labels)
            .field("focus", &self.focus)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(f: &mut Form, s: &str) {
        for c in s.chars() {
            f.key(Key::Char(c));
        }
    }

    #[test]
    fn tab_moves_between_fields_and_round() {
        let mut f = Form::new().text("Name").password("Password");
        typed(&mut f, "al");
        f.key(Key::Tab);
        typed(&mut f, "pw");
        assert_eq!((f.value(0), f.value(1)), ("al", "pw"));
        f.key(Key::Tab);
        assert_eq!(f.focus(), 0);
        assert_eq!(
            f.hints("back", "next"),
            [("Esc", "back"), ("Tab", "next field"), ("Enter", "next")]
        );
    }

    #[test]
    fn tab_on_a_lone_password_shows_and_hides_it() {
        let mut f = Form::new().password("Again");
        assert_eq!(f.hints("back", "next")[1], ("Tab", "show"));
        f.key(Key::Tab);
        assert!(!f.fields().next().unwrap().masked);
        assert_eq!(f.hints("back", "next")[1], ("Tab", "hide"));
        f.key(Key::Tab);
        assert!(f.fields().next().unwrap().masked);
    }

    #[test]
    fn a_lone_text_field_has_no_tab() {
        let f = Form::new().text("Name");
        assert_eq!(f.hints("back", "next"), [("Esc", "back"), ("Enter", "next")]);
    }

    #[test]
    fn an_error_takes_the_focus_and_typing_clears_it() {
        let mut f = Form::new().text("Name").password("Password");
        f.error(1, "Too short");
        assert_eq!(f.focus(), 1);
        assert_eq!(f.fields().nth(1).unwrap().error, Some("Too short"));
        typed(&mut f, "x");
        assert_eq!(f.fields().nth(1).unwrap().error, None);
    }

    #[test]
    fn take_empties_a_field_and_debug_hides_text() {
        let mut f = Form::new().password("Password").with("hunter22");
        assert!(!format!("{f:?}").contains("hunter22"));
        assert_eq!(f.take(0), "hunter22");
        assert_eq!(f.value(0), "");
    }
}
