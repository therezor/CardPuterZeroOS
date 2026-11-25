//! System sounds: the SDK asks the shell for them over the link as keys
//! move, choose and go back, and an app asks for its own.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use gh0st_core::config::key;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

struct Menu {
    list: List,
    path: Rc<RefCell<PathBuf>>,
}

impl Page for Menu {
    fn title(&self) -> &str {
        "menu"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, k: Key, cx: &mut Cx) {
        *self.path.borrow_mut() = cx.settings().config().path().to_path_buf();
        match k {
            Key::Enter => cx.push(Inner),
            Key::Char('e') => cx.sound(Sound::Error),
            Key::Char('q') => cx.settings().set_bool(key::SOUNDS, false),
            _ => {
                self.list.key(k);
            }
        }
    }
}

struct Inner;

impl Page for Inner {
    fn title(&self) -> &str {
        "inner"
    }
    fn draw(&self, _: &mut Ui) {}
}

struct Recorder;

impl Page for Recorder {
    fn title(&self) -> &str {
        "recorder"
    }
    fn draw(&self, _: &mut Ui) {}
    fn quiet(&self) -> bool {
        true
    }
    fn key(&mut self, k: Key, cx: &mut Cx) {
        if k == Key::Char('e') {
            cx.sound(Sound::Error);
        }
    }
}

struct Game;

impl Page for Game {
    fn title(&self) -> &str {
        "game"
    }
    fn draw(&self, _: &mut Ui) {}
    fn raw_keys(&self) -> bool {
        true
    }
}

fn menu() -> (Driver, Rc<RefCell<PathBuf>>) {
    let path = Rc::new(RefCell::new(PathBuf::new()));
    let page = Menu {
        list: List::new(),
        path: path.clone(),
    };
    let mut d = Driver::new(page);
    d.listen();
    // A key teaches the page its settings file; the folder is the test's.
    d.keys("j");
    std::fs::create_dir_all(path.borrow().parent().unwrap()).unwrap();
    d.heard();
    (d, path)
}

#[test]
fn moving_choosing_and_going_back_sound() {
    let (mut d, _) = menu();
    d.keys("x\nenter\nesc\ne");
    assert_eq!(
        d.heard(),
        ["sound move", "sound select", "sound back", "sound error"]
    );
}

#[test]
fn a_raw_key_page_a_quiet_one_and_a_paused_app_make_none() {
    let mut d = Driver::new(Game);
    d.listen();
    d.keys("x\nenter\nz");
    assert!(d.heard().iter().all(|m| !m.starts_with("sound")));
    let mut d = Driver::new(Recorder);
    d.listen();
    d.keys("x\nenter\ne");
    assert!(d.heard().iter().all(|m| !m.starts_with("sound")));
    let (mut d, _) = menu();
    d.pause().keys("x");
    assert!(d.heard().iter().all(|m| !m.starts_with("sound")));
}

#[test]
fn turned_off_they_stop_at_once_and_after_a_resume() {
    let (mut d, _) = menu();
    d.keys("q\nx");
    assert!(d.heard().is_empty());

    // Turned off elsewhere while the app was behind.
    let (mut d, path) = menu();
    d.keys("x");
    d.heard();
    d.pause();
    gh0st_core::config::Config::update(&path.borrow(), |c| c.set_bool(key::SOUNDS, false)).unwrap();
    d.resume().keys("x");
    assert!(d.heard().iter().all(|m| !m.starts_with("sound")));
}
