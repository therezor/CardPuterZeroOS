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

struct Editor {
    got: Option<Option<Vec<PathBuf>>>,
}

impl Page for Editor {
    fn title(&self) -> &str {
        "Editor"
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.pick(gh0st_app::PickSpec::save("notes.txt").filter("Text", &["text/plain", "*.md"]));
        }
    }

    fn picked(&mut self, _pick: gh0st_app::Pick, paths: Option<Vec<PathBuf>>, _cx: &mut Cx) {
        self.got = Some(paths);
    }
}

#[test]
fn a_pick_is_answered_to_the_page_that_asked() {
    let mut d = Driver::new(Editor { got: None });
    d.keys("enter");
    let asked = d.picks();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].mode, gh0st_app::PickMode::Save);
    assert_eq!(asked[0].name, "notes.txt");
    d.answer_pick(Some(vec![PathBuf::from("/home/u/notes.txt")]));
    assert!(d.picks().is_empty());
}

/// Pushes a page that asks for a file at once.
struct Home {
    heard: std::rc::Rc<std::cell::Cell<u32>>,
}

struct Asker {
    heard: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Page for Home {
    fn title(&self) -> &str {
        "Home"
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.push(Asker {
                heard: self.heard.clone(),
            });
        }
    }
}

impl Page for Asker {
    fn title(&self) -> &str {
        "Asker"
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn start(&mut self, cx: &mut Cx) {
        if self.heard.get() == 0 {
            cx.pick(gh0st_app::PickSpec::open());
        }
    }

    fn picked(&mut self, _: gh0st_app::Pick, _: Option<Vec<PathBuf>>, _: &mut Cx) {
        self.heard.set(self.heard.get() + 10);
    }

    fn stop(&mut self, _: &mut Cx) {
        self.heard.set(self.heard.get() + 1);
    }
}

#[test]
fn a_closed_pages_pick_never_reaches_the_page_in_its_place() {
    let heard = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut d = Driver::new(Home {
        heard: heard.clone(),
    });
    d.keys("enter");
    assert_eq!(d.picks().len(), 1);
    // It closes; another page takes its place on the stack.
    d.keys("esc\nenter");
    assert_eq!(d.depth(), 2);
    assert!(d.picks().is_empty(), "the pick went with its page");
    d.answer_pick(Some(vec![PathBuf::from("/tmp/x")]));
    assert_eq!(heard.get(), 1, "only the first page's stop, no answer");
}
