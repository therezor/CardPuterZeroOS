//! Low memory and network changes reach every page on the stack, the one
//! on top first.

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

    fn network_changed(&mut self, _cx: &mut Cx) {
        self.heard.borrow_mut().push((self.name, 0));
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

#[test]
fn a_network_change_reaches_every_page() {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let mut d = Driver::new(Cached {
        name: "Below",
        heard: heard.clone(),
    });
    d.keys("enter");
    d.network_changed();
    assert_eq!(*heard.borrow(), [("Above", 0), ("Below", 0)]);
}

#[test]
fn a_network_change_behind_another_app_is_heard_once_on_return() {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let mut d = Driver::new(Cached {
        name: "Below",
        heard: heard.clone(),
    });
    d.pause();
    d.network_changed().network_changed();
    assert!(heard.borrow().is_empty(), "nothing wakes behind");
    d.resume();
    assert_eq!(*heard.borrow(), [("Below", 0)]);
    d.pause().resume();
    assert_eq!(heard.borrow().len(), 1, "no change, nothing heard");
}
