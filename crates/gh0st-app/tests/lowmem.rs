//! Low memory reaches every page on the stack, the one on top first.

use gh0st_app::testing::Driver;
use gh0st_app::{Cx, Key, Page, Ui};
use std::cell::RefCell;
use std::rc::Rc;

struct Cached {
    name: &'static str,
    heard: Rc<RefCell<Vec<(&'static str, u8)>>>,
}

impl Page for Cached {
    fn title(&self) -> &str {
        self.name
    }

    fn draw(&self, _ui: &mut Ui) {}

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.push(Cached {
                name: "Above",
                heard: self.heard.clone(),
            });
        }
    }

    fn low_memory(&mut self, level: u8, _cx: &mut Cx) {
        self.heard.borrow_mut().push((self.name, level));
    }
}

#[test]
fn every_page_drops_its_caches_top_first() {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let mut d = Driver::new(Cached {
        name: "Below",
        heard: heard.clone(),
    });
    d.keys("enter");
    d.low_memory(100);
    assert_eq!(*heard.borrow(), [("Above", 100), ("Below", 100)]);
}
