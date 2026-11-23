//! First-boot setup (docs/setup.md): the welcome, a tour of the keys and the
//! design system, then the settings, and the hand-over to the shell. It runs
//! as root before any user session, as `gh0st-settings --setup`, and can't
//! be closed.

mod account;
mod clock;
mod finish;
mod state;
pub mod tour;

use crate::app::{Embed, Settings};
use crate::model::PageId;
use gh0st_app::prelude::*;
use gh0st_app::{Options, run_with};
use gh0st_core::backlight::Backlight;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use state::Answers;

/// What the steps share.
pub type Shared = Rc<RefCell<Answers>>;

/// The root helper, through pkexec as Settings calls it. Setup runs as
/// root, so its gated verbs ask no password.
use gh0st_app::helper::{PATH as HELPER, failure};

/// The account's step: the notice, the welcome, the training, then five
/// settings before it. A resume never starts past it: the password isn't
/// saved.
const ACCOUNT: usize = 2 + tour::LESSONS + 5;

/// A system file setup reads or writes, under `$GH0ST_SETUP_ROOT` when set,
/// for a host run.
fn path(p: &str) -> PathBuf {
    match std::env::var_os("GH0ST_SETUP_ROOT") {
        Some(root) => PathBuf::from(root).join(p.trim_start_matches('/')),
        None => PathBuf::from(p),
    }
}

pub fn run() -> ! {
    light_up();
    let answers = Rc::new(RefCell::new(state::load()));
    run_with(steps(answers), Options { closable: false })
}

/// The welcome, the tour, then the settings, each part counted on its own.
pub fn steps(a: Shared) -> Steps {
    let at = a.borrow().step.min(ACCOUNT);
    let welcome = a.clone();
    let mut s = Steps::new("")
        .optional(|_| tour::ALPHA.then_some(tour::Notice))
        .step(move |_| tour::Welcome::new(welcome.clone()))
        .part("Training");
    for n in 0..tour::LESSONS {
        let a = a.clone();
        s = s.optional(move |label| {
            let tour = a.borrow().tour;
            tour.then(|| tour::Lesson::new(n, label, a.clone()))
        });
    }
    let (acct, fin, save) = (a.clone(), a.clone(), a);
    s.part("Setup")
        .step(embed(PageId::Country(None), true, false))
        .step(embed(PageId::CountryZones, true, false))
        .step(embed(PageId::Privacy, false, true))
        .step(embed(PageId::Wifi, true, false))
        .optional(|label| (!clock::synced()).then(|| clock::DateTime::new(label)))
        .step(move |label| account::Account::new(label, acct.clone()))
        .step(embed(PageId::Look, false, true))
        .step(move |label| finish::Finish::new(label, fin.clone()))
        .from(at)
        .on_step(move |i| {
            let mut a = save.borrow_mut();
            a.step = i;
            state::save(&a);
        })
}

fn embed(page: PageId, skip: bool, done: bool) -> impl FnMut(&str) -> Settings + 'static {
    move |label| {
        Settings::embedded(Embed {
            page: page.clone(),
            label: label.into(),
            skip,
            done,
        })
    }
}

/// No shell runs yet to light the panel.
fn light_up() {
    if let Some(bl) = Backlight::find(Path::new("/sys/class/backlight")) {
        if bl.get().is_none_or(|r| bl.percent(r) < 30) {
            let _ = bl.set(bl.raw(80));
        }
    }
}

/// Setup's menu on the pages it draws itself: Power off. Embedded Settings
/// adds the same.
const POWER_OFF: u32 = 900;

fn power_menu(c: &mut Commands) {
    c.group("Setup");
    c.add(Command::action(POWER_OFF, "Power off"));
}

/// Asks before powering off. True if `id` was Power off.
fn power_command(id: u32, cx: &mut Cx) -> bool {
    if id != POWER_OFF {
        return false;
    }
    let d = Dialog::confirm("Power off?", "Setup goes on at the next start.").yes("power off");
    cx.ask(POWER_OFF, d);
    true
}

/// True if the dialog was Power off's.
fn power_answer(id: u32, answer: &Answer, cx: &mut Cx) -> bool {
    if id != POWER_OFF {
        return false;
    }
    if matches!(answer, Answer::Yes(_)) {
        cx.spawn(&["pkexec", HELPER, "power", "off"]);
    }
    true
}
