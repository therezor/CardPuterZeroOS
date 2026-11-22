//! An app that keeps the screen on, and lets it go.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;

struct Player;

impl Page for Player {
    fn title(&self) -> &str {
        "player"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Char('y') => cx.keep_awake(true),
            Key::Char('u') => cx.keep_awake(false),
            _ => {}
        }
    }
}

#[test]
fn keep_awake_follows_the_page() {
    let mut d = Driver::new(Player);
    assert!(!d.awake());
    d.keys("y");
    assert!(d.awake());
    d.keys("y\nu");
    assert!(!d.awake());
}
