//! Copy and paste through the SDK: Ctrl+V types the first line copied into
//! a page that is typing, and a page copies with cx.copy.

use gh0st_app::testing::Driver;
use gh0st_app::{Cx, Edit, Key, Page, TextField, Ui};

#[derive(Default)]
struct Note {
    field: TextField,
}

impl Page for Note {
    fn title(&self) -> &str {
        "Note"
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn typing(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.field.key(key) == Some(Edit::Submit) {
            cx.copy(self.field.text().as_bytes());
        }
    }
}

#[test]
fn ctrl_v_types_the_first_line_and_enter_copies() {
    let mut d = Driver::new(Note::default());
    d.set_clipboard(b"hello\nsecond line");
    d.keys("ctrl+v\nenter");
    assert_eq!(d.clipboard(), Some(&b"hello"[..]));
}

#[test]
fn ctrl_v_skips_empty_lines_and_types_nothing_once_cleared() {
    let mut d = Driver::new(Note::default());
    d.set_clipboard(b"\n\n  \nsecret\xff!\nmore");
    d.keys("ctrl+v\nenter");
    assert_eq!(d.clipboard(), Some(&b"secret!"[..]));
    d.clear_clipboard();
    d.keys("ctrl+v\nenter");
    assert_eq!(
        d.clipboard(),
        Some(&b"secret!"[..]),
        "the field kept its text"
    );
}
