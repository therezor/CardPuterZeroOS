//! Steps, and an app that can't close: what first-boot setup runs on.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use gh0st_app::{Options, Steps};
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

/// A step: Enter is done, s opens a sub-page, t starts a timer.
struct Step {
    label: String,
    log: Log,
}

impl Page for Step {
    fn title(&self) -> &str {
        "Step"
    }
    fn context(&self) -> Option<&str> {
        Some(&self.label)
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, _: &mut Cx) {
        self.log.borrow_mut().push(self.label.clone());
    }
    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Enter => cx.done(),
            Key::Char('s') => cx.push(Sub),
            Key::Char('t') => {
                cx.after(500);
            }
            Key::Char('q') => cx.quit(),
            _ => {}
        }
    }
    fn timer(&mut self, _: Timer, _: &mut Cx) {
        let fired = format!("timer on {}", self.label);
        self.log.borrow_mut().push(fired);
    }
}

/// A page a step opens; Enter is done for it alone.
struct Sub;

impl Page for Sub {
    fn title(&self) -> &str {
        "Sub"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.done();
        }
    }
}

fn step(log: &Log) -> impl FnMut(&str) -> Step + 'static {
    let log = log.clone();
    move |label| Step {
        label: label.into(),
        log: log.clone(),
    }
}

fn three(log: &Log, middle: bool) -> Steps {
    let l = log.clone();
    Steps::new("Setup")
        .step(step(log))
        .optional(move |label| {
            middle.then(|| Step {
                label: label.into(),
                log: l.clone(),
            })
        })
        .step(step(log))
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

#[test]
fn done_goes_on_and_esc_goes_back() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, true));
    assert_eq!(taken(&log), ["Setup 1 of 3"]);
    d.keys("enter\nenter");
    assert_eq!(taken(&log), ["Setup 2 of 3", "Setup 3 of 3"]);
    d.keys("esc");
    assert_eq!(taken(&log), ["Setup 2 of 3"]);
    assert_eq!(d.depth(), 2, "Steps and one step");
}

#[test]
fn a_step_that_does_not_apply_is_skipped_both_ways() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, false));
    d.keys("enter");
    assert_eq!(taken(&log), ["Setup 1 of 3", "Setup 3 of 3"]);
    d.keys("esc");
    assert_eq!(taken(&log), ["Setup 1 of 3"]);
}

#[test]
fn esc_on_the_first_step_makes_it_again() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, true));
    taken(&log);
    d.keys("esc").wait(800);
    assert_eq!(taken(&log), ["Setup 1 of 3"]);
    assert!(!d.quit());
}

#[test]
fn after_the_last_step_steps_closes() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, true));
    d.keys("enter\nenter\nenter");
    assert!(d.quit());
}

#[test]
fn a_sub_page_done_is_not_the_step_done() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, true));
    taken(&log);
    d.keys("type s\nenter");
    assert_eq!(d.depth(), 2, "the sub-page closed");
    assert!(taken(&log).is_empty(), "no step opened");
    d.keys("enter");
    assert_eq!(taken(&log), ["Setup 2 of 3"]);
}

#[test]
fn a_closed_step_s_timer_never_fires() {
    let log = Log::default();
    let mut d = Driver::new(three(&log, true));
    d.keys("type t\nenter").wait(600);
    assert_eq!(taken(&log), ["Setup 1 of 3", "Setup 2 of 3"]);
}

#[test]
fn from_resumes_and_on_step_reports_each_step() {
    let log = Log::default();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    let steps = three(&log, true)
        .from(1)
        .on_step(move |i| s.borrow_mut().push(i));
    let mut d = Driver::new(steps);
    d.keys("enter\nesc\nesc");
    assert_eq!(*seen.borrow(), [1, 2, 1, 0]);
}

#[test]
fn an_app_that_cant_close_stays_on_the_last_step() {
    let log = Log::default();
    let fixed = Options { closable: false };
    let mut d = Driver::with(three(&log, true), fixed);
    d.keys("enter\nenter");
    taken(&log);
    d.keys("enter");
    assert!(!d.quit());
    assert_eq!(taken(&log), ["Setup 3 of 3"], "made again");
    assert_eq!(d.depth(), 2);
}

#[test]
fn from_past_the_end_opens_the_last_step_when_it_cant_close() {
    let log = Log::default();
    let fixed = Options { closable: false };
    let _d = Driver::with(three(&log, false).from(9), fixed);
    assert_eq!(taken(&log), ["Setup 3 of 3"]);
}

/// A first page with nothing on it but a way to quit.
struct Plain;

impl Page for Plain {
    fn title(&self) -> &str {
        "Plain"
    }
    fn draw(&self, _: &mut Ui) {}
    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Char('q') {
            cx.quit();
        }
    }
}

#[test]
fn an_app_that_cant_close_ignores_a_held_esc() {
    let mut d = Driver::with(Plain, Options { closable: false });
    d.keys("down esc").wait(800);
    assert!(!d.quit());
    assert_eq!(d.toast(), None, "no Hold to exit");
}

#[test]
fn its_menu_has_no_close_app() {
    // Help, then Close app, which asks for a second Enter.
    let menu = "8\nx\nenter\nenter";
    let mut d = Driver::new(Plain);
    d.keys(menu);
    assert!(d.quit(), "an app that can close");
    let mut d = Driver::with(Plain, Options { closable: false });
    d.keys(menu);
    assert!(!d.quit());
}

#[test]
fn cx_quit_still_ends_it() {
    let mut d = Driver::with(Plain, Options { closable: false });
    d.keys("type q");
    assert!(d.quit());
}

#[test]
fn each_part_counts_its_own_steps() {
    let log = Log::default();
    let steps = Steps::new("")
        .step(step(&log))
        .part("Tour")
        .step(step(&log))
        .step(step(&log))
        .part("Setup")
        .step(step(&log));
    let mut d = Driver::new(steps);
    d.keys("enter\nenter\nenter");
    assert_eq!(
        taken(&log),
        ["", "Tour 1 of 2", "Tour 2 of 2", "Setup 1 of 1"]
    );
}
