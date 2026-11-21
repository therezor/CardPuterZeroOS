//! Settings pages as steps of first-boot setup: their chips go on, Esc on a
//! step's own page goes back a step.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use gh0st_settings::app::{Embed, Settings};
use gh0st_settings::model::PageId;
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
    let log = Log::default();
    let steps = Steps::new("Setup")
        .step(step(PageId::Wifi, true, false))
        .step(mark(&log));
    let mut d = Driver::new(steps);
    d.keys("7");
    assert_eq!(taken(&log), ["Setup 2 of 2"]);
}

#[test]
fn esc_on_a_step_s_page_goes_back_a_step() {
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
