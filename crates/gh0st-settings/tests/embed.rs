//! Settings pages as steps of first-boot setup: their chips go on, Esc on a
//! step's own page goes back a step.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use gh0st_settings::app::{Embed, Settings};
use gh0st_settings::model::PageId;

mod common;
use common::stub_tools;
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

/// A plain step: logs its label as it opens, Enter is done.
struct Mark(String, Log);

impl Page for Mark {
    fn title(&self) -> &str {
        "Mark"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, _: &mut Cx) {
        self.1.borrow_mut().push(self.0.clone());
    }
    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.done();
        }
    }
}

fn mark(log: &Log) -> impl FnMut(&str) -> Mark + 'static {
    let log = log.clone();
    move |label| Mark(label.into(), log.clone())
}

fn step(page: PageId, skip: bool, done: bool) -> impl FnMut(&str) -> Settings + 'static {
    move |label| {
        Settings::embedded(Embed {
            page: page.clone(),
            label: label.into(),
            skip,
            done,
        })
    }
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

#[test]
fn done_on_six_goes_on_from_privacy() {
    stub_tools();
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(step(PageId::Privacy, false, true))
        .step(mark(&log));
    let mut d = Driver::new(steps);
    assert!(taken(&log).is_empty());
    d.keys("6");
    assert_eq!(taken(&log), ["Setup 2 of 2"]);
}

#[test]
fn skip_on_seven_goes_on_from_the_network() {
    stub_tools();
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(step(PageId::Wifi, true, false))
        .step(mark(&log));
    let mut d = Driver::new(steps);
    // The step may be turning the radio on: Skip waits for that.
    d.keys("7").jobs(2000);
    assert_eq!(taken(&log), ["Setup 2 of 2"]);
}

/// Alone, Skip takes the middle key, as any single chip does.
#[test]
fn a_lone_skip_sits_on_six() {
    stub_tools();
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(step(PageId::CountryZones, true, false))
        .step(mark(&log));
    let mut d = Driver::new(steps);
    d.keys("7");
    assert!(taken(&log).is_empty(), "7 has no chip here");
    d.keys("6");
    assert_eq!(taken(&log), ["Setup 2 of 2"]);
}

/// The Country step is a search: digits and letters type, and its first
/// row keeps the world setting, as Skip does on other steps.
#[test]
fn the_country_step_types_and_its_first_row_skips() {
    stub_tools();
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(step(PageId::Country, true, false))
        .step(mark(&log));
    let mut d = Driver::new(steps);
    d.keys("6");
    assert!(taken(&log).is_empty(), "6 types into the search");
    d.keys("enter");
    assert!(taken(&log).is_empty(), "nothing matches 6 to pick");
    d.keys("esc");
    assert!(taken(&log).is_empty(), "Esc clears what was typed first");
    d.keys("enter");
    assert_eq!(taken(&log), ["Setup 2 of 2"]);
}

#[test]
fn esc_on_a_step_s_page_goes_back_a_step() {
    stub_tools();
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(mark(&log))
        .step(step(PageId::Look, false, true));
    let mut d = Driver::new(steps);
    d.keys("enter");
    assert_eq!(taken(&log), ["Setup 1 of 2"]);
    d.keys("esc");
    assert_eq!(taken(&log), ["Setup 1 of 2"], "the first step again");
}

/// A wrong answer opens the dialog again with why under the field, and the
/// next key clears it (design.md, Text input).
#[test]
fn a_wrong_device_name_says_why_in_the_dialog() {
    stub_tools();
    let steps = Steps::new("Setup").step(step(PageId::Look, false, true));
    let mut d = Driver::new(steps);
    d.keys("enter");
    assert_eq!(d.dialog().map(|x| x.1), Some(None));
    d.keys("type bad_name\nenter");
    assert_eq!(
        d.dialog(),
        Some(("Device name", Some("Letters, digits and hyphens only")))
    );
    assert_eq!(d.toast(), None, "no toast says it twice");
    d.keys("backspace");
    assert_eq!(d.dialog().map(|x| x.1), Some(None));
}
