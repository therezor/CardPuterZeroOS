//! The app menu: Fn+H opens it, a key or Enter picks, Esc closes, and
//! Close app is always last.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::rc::Rc;

struct App {
    picked: Rc<RefCell<Vec<u32>>>,
}

impl Page for App {
    fn title(&self) -> &str {
        "App"
    }
    fn draw(&self, _: &mut Ui) {}
    fn menu(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::new(1, "Zoom in").key('t'),
            MenuItem::new(2, "Grid").key('m').value("Magnify"),
        ]
    }
    fn menu_pick(&mut self, id: u32, _: &mut Cx) {
        self.picked.borrow_mut().push(id);
    }
}

fn app() -> (Driver, Rc<RefCell<Vec<u32>>>) {
    let picked = Rc::new(RefCell::new(Vec::new()));
    let d = Driver::new(App {
        picked: picked.clone(),
    });
    (d, picked)
}

#[test]
fn a_key_picks_and_closes_the_menu() {
    let (mut d, picked) = app();
    d.keys("help");
    d.frame();
    d.keys("m");
    assert_eq!(*picked.borrow(), [2]);
    // Closed: m now goes to the page, which ignores it.
    d.keys("m");
    assert_eq!(*picked.borrow(), [2]);
}

#[test]
fn enter_picks_the_selection_and_esc_closes() {
    let (mut d, picked) = app();
    d.keys("help");
    d.frame();
    d.keys("fn+x\nenter");
    assert_eq!(*picked.borrow(), [2]);
    d.keys("help");
    d.frame();
    d.keys("esc");
    assert!(!d.quit(), "Esc closes the menu, not the page");
    assert!(picked.borrow().len() == 1);
}

#[test]
fn close_app_is_last_and_quits() {
    let (mut d, _) = app();
    d.keys("help");
    d.frame();
    d.keys("fn+f\nenter");
    assert!(d.quit());
}

#[test]
fn fn_esc_opens_it_and_plain_esc_does_not() {
    let (mut d, picked) = app();
    d.keys("fn+esc");
    d.frame();
    d.keys("t");
    assert_eq!(*picked.borrow(), [1]);
    d.keys("esc");
    assert!(d.quit(), "plain Esc goes to the page, which closes");
}
