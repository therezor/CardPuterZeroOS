//! A page opens files and is handed them.

use gh0st_app::testing::Driver;
use gh0st_app::{Cx, Key, Page, Ui};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Viewer {
    seen: Vec<PathBuf>,
}

impl Page for Viewer {
    fn title(&self) -> &str {
        "Viewer"
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn opened(&mut self, paths: &[PathBuf], cx: &mut Cx) {
        self.seen.extend_from_slice(paths);
        cx.toast(&format!("{} files", self.seen.len()));
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Enter => cx.open_file(Path::new("/tmp/notes.txt")),
            Key::Char('w') => cx.open_with("text", Path::new("/tmp/a b.png")),
            _ => {}
        }
    }
}

#[test]
fn handed_files_reach_the_page_on_top() {
    let mut d = Driver::new(Viewer::default());
    d.open(&[Path::new("/tmp/one.txt")]);
    assert_eq!(d.toast(), Some("1 files"));
    d.open(&[Path::new("/tmp/two.txt"), Path::new("/tmp/three.txt")]);
    assert_eq!(d.toast(), Some("3 files"));
}

#[test]
fn a_page_asks_to_open_a_file_in_its_app_or_a_named_one() {
    let mut d = Driver::new(Viewer::default());
    d.keys("enter\nw");
    assert_eq!(
        d.opened_requests(),
        [
            (None, PathBuf::from("/tmp/notes.txt")),
            (Some("text".to_string()), PathBuf::from("/tmp/a b.png")),
        ]
    );
}
