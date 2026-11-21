//! Commands and the app menu, on a page that is not the terminal: Fn+Esc
//! opens the menu, settings change in place, actions close it, and the keys
//! run the same commands.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::rc::Rc;

const SIZE: u32 = 1;
const NIGHT: u32 = 2;
const FIND: u32 = 3;
const DOWN: u32 = 4;
const EDIT: u32 = 5;

type Log = Rc<RefCell<Vec<(u32, Step)>>>;

struct Reader {
    log: Log,
    size: usize,
    night: bool,
    /// A mode where letters type: the edit field.
    editing: bool,
    typed: Rc<RefCell<String>>,
}

const SIZES: &[&str] = &["Small", "Medium", "Large"];

impl Page for Reader {
    fn title(&self) -> &str {
        "Reader"
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text(SIZES[self.size]);
    }

    fn bars(&self) -> Bars {
        Bars::top().command_hints()
    }

    fn typing(&self) -> bool {
        self.editing
    }

    fn commands(&self, c: &mut Commands) {
        let reading = !self.editing;
        c.group("View");
        c.add(Command::choice(SIZE, "Text size", SIZES[self.size]).keys("r t"));
        c.add(Command::toggle(NIGHT, "Night", self.night).keys("b"));
        c.add(Command::action(FIND, "Find").keys("n").mode("Reading", reading));
        c.add(
            Command::action(DOWN, "Scroll down")
                .keys("x")
                .keys_only()
                .repeats()
                .mode("Reading", reading)
                .hint("scroll"),
        );
        c.group("Edit");
        c.add(Command::action(EDIT, "Edit").keys("e").mode("Reading", reading));
    }

    fn command(&mut self, id: u32, step: Step, _: &mut Cx) {
        self.log.borrow_mut().push((id, step));
        match (id, step) {
            (SIZE, Step::Prev) => self.size = self.size.saturating_sub(1),
            (SIZE, _) => self.size = (self.size + 1).min(SIZES.len() - 1),
            (NIGHT, _) => self.night = !self.night,
            (EDIT, _) => self.editing = true,
            _ => {}
        }
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        if let Key::Char(c) = key {
            self.typed.borrow_mut().push(c);
        }
    }
}

struct App {
    d: Driver,
    log: Log,
    typed: Rc<RefCell<String>>,
}

fn app() -> App {
    let log = Log::default();
    let typed = Rc::new(RefCell::new(String::new()));
    let d = Driver::new(Reader {
        log: log.clone(),
        size: 1,
        night: false,
        editing: false,
        typed: typed.clone(),
    });
    App { d, log, typed }
}

impl App {
    fn ids(&self) -> Vec<u32> {
        self.log.borrow().iter().map(|e| e.0).collect()
    }
}

#[test]
fn keys_run_commands_without_the_menu() {
    let mut a = app();
    a.d.keys("n\nt\nr");
    assert_eq!(
        *a.log.borrow(),
        [(FIND, Step::Run), (SIZE, Step::Next), (SIZE, Step::Prev)]
    );
}

#[test]
fn a_choice_steps_in_place_and_the_menu_stays() {
    let mut a = app();
    a.d.keys("fn+esc");
    a.d.frame();
    // Text size is the first row: Right steps it, the menu stays open, so
    // t steps it again through the menu.
    a.d.keys("fn+c\nt\nenter");
    assert_eq!(a.ids(), [SIZE, SIZE, SIZE]);
    a.d.keys("esc");
    assert!(!a.d.quit(), "Esc closes the menu first");
    a.d.keys("esc");
    assert!(a.d.quit(), "then Esc reaches the page");
}

#[test]
fn a_toggle_flips_in_place_on_enter() {
    let mut a = app();
    a.d.keys("help\nfn+x\nenter\nenter");
    assert_eq!(a.ids(), [NIGHT, NIGHT]);
    a.d.keys("fn+esc");
    a.d.keys("b");
    assert_eq!(a.ids(), [NIGHT, NIGHT, NIGHT], "closed: b runs it directly");
}

#[test]
fn an_action_closes_the_menu_then_runs() {
    let mut a = app();
    a.d.keys("fn+esc\nn");
    assert_eq!(a.ids(), [FIND]);
    a.d.keys("esc");
    assert!(a.d.quit(), "the menu had closed: Esc went to the page");
}

#[test]
fn a_mode_that_is_off_keeps_its_menu_row() {
    let mut a = app();
    a.d.keys("e");
    assert_eq!(a.ids(), [EDIT]);
    // Editing: n types, and Find runs only from the menu.
    a.d.keys("n");
    assert_eq!(*a.typed.borrow(), "n");
    a.d.keys("fn+esc\nn");
    assert_eq!(a.ids(), [EDIT, FIND]);
}

#[test]
fn close_app_asks_once_more() {
    let mut a = app();
    a.d.keys("fn+esc\nfn+f\nenter");
    assert!(!a.d.quit());
    a.d.keys("enter");
    assert!(a.d.quit());
}

#[test]
fn h_opens_keys_and_esc_goes_back_to_the_menu() {
    let mut a = app();
    a.d.keys("fn+esc\nh\nesc\nn");
    assert_eq!(a.ids(), [FIND], "back in the menu, n picked Find");
}

#[test]
fn the_menu_reopens_on_a_setting_just_changed() {
    let mut a = app();
    a.d.keys("fn+esc\nfn+x\nenter\nfn+esc");
    a.d.keys("fn+esc\nenter");
    assert_eq!(a.ids(), [NIGHT, NIGHT]);
}

#[test]
fn after_an_action_the_menu_opens_at_the_top() {
    let mut a = app();
    a.d.keys("fn+esc\nfn+x\nenter\nfn+x\nenter");
    assert_eq!(a.ids(), [NIGHT, FIND]);
    a.d.keys("fn+esc\nenter");
    assert_eq!(a.ids(), [NIGHT, FIND, SIZE], "the first row, not Find");
}

#[test]
fn a_held_key_stops_once_its_command_changed_the_mode() {
    let mut a = app();
    a.d.keys("down e");
    a.d.wait(800);
    a.d.keys("up e");
    assert_eq!(a.ids(), [EDIT]);
    assert_eq!(*a.typed.borrow(), "", "e's repeats typed nothing");
}

#[test]
fn moves_repeat_while_held() {
    let mut a = app();
    a.d.keys("down x");
    a.d.wait(800);
    a.d.keys("up x");
    assert!(a.ids().len() > 3, "{:?}", a.ids());
}

/// `GH0ST_SNAP_DIR=dir cargo test -p gh0st-app --test menu` saves the menu
/// and the Keys screen as PNGs, for a look by eye.
#[test]
fn snapshots() {
    let Some(dir) = std::env::var_os("GH0ST_SNAP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut a = app();
    a.d.save(&dir.join("page-hints.png")).unwrap();
    a.d.keys("fn+esc");
    a.d.save(&dir.join("menu-1.png")).unwrap();
    a.d.keys("fn+x");
    a.d.save(&dir.join("menu-2.png")).unwrap();
    a.d.keys("fn+f\nfn+f\nenter");
    a.d.save(&dir.join("menu-3.png")).unwrap();
    a.d.keys("esc\nh");
    a.d.save(&dir.join("menu-keys.png")).unwrap();
}
