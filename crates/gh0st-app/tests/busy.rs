//! Slow work: the busy card waits out a grace, then holds keys until the
//! work ends or Esc hides it.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;

#[derive(Default)]
struct Slow {
    presses: u32,
}

impl Page for Slow {
    fn title(&self) -> &str {
        "slow"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Enter => {
                self.presses += 1;
                cx.busy("Applying\u{2026}");
                cx.after(1000);
            }
            Key::Char('q') => {
                self.presses += 1;
                cx.busy("Quick");
                cx.idle();
            }
            _ => {}
        }
    }
    fn timer(&mut self, _: Timer, cx: &mut Cx) {
        cx.idle();
        cx.toast(&format!("Done after {} presses", self.presses));
    }
}

#[test]
fn the_card_waits_out_its_grace_then_holds_keys() {
    let mut d = Driver::new(Slow::default());
    d.keys("enter");
    assert_eq!(d.busy(), None, "not before the grace");
    d.keys("q");
    assert_eq!(d.busy(), None, "keys still work in the grace");
    d.keys("enter");
    d.wait(400);
    assert_eq!(d.busy(), Some("Applying\u{2026}"));
    d.keys("enter");
    d.wait(1100);
    assert_eq!(d.busy(), None);
    assert_eq!(
        d.toast(),
        Some("Done after 3 presses"),
        "the press while the card showed never reached the page"
    );
}

/// A page that waits on work, opened over one that doesn't.
struct Opener;

impl Page for Opener {
    fn title(&self) -> &str {
        "opener"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Char('o') {
            cx.push(Slow::default());
        }
    }
}

#[test]
fn closing_the_page_takes_its_card_away() {
    let mut d = Driver::new(Opener);
    // Esc inside the grace closes the page that started the work.
    d.keys("o\nenter\nesc");
    assert_eq!(d.depth(), 1);
    d.wait(400);
    assert_eq!(d.busy(), None, "the card never shows over the page below");
}

#[test]
fn quick_work_never_shows_it() {
    let mut d = Driver::new(Slow::default());
    d.keys("q");
    d.wait(500);
    assert_eq!(d.busy(), None);
}

#[test]
fn esc_hides_the_card_and_the_work_goes_on() {
    let mut d = Driver::new(Slow::default());
    d.keys("enter");
    d.wait(400);
    assert!(d.busy().is_some());
    d.keys("esc");
    assert_eq!(d.busy(), None);
    assert_eq!(d.depth(), 1, "Esc hid the card, not the page");
    d.wait(700);
    assert_eq!(d.toast(), Some("Done after 1 presses"));
}
