//! Files as the file chooser: what it hands back for each kind of question.

use gh0st_app::secret::Keyring;
use gh0st_app::testing::Driver;
use gh0st_files::app::{Files, Options, Picker};
use gh0st_files::fake::Fake;
use gh0st_files::pick::{Filter, Mode, Pattern, Spec};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

type Answer = Rc<RefCell<Option<Option<Vec<PathBuf>>>>>;

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> Home {
    let d = std::env::temp_dir().join(format!("gh0st-files-pick-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for f in [
        "Music/a.flac",
        "Music/b.flac",
        "Music/notes.txt",
        "Docs/plan.md",
    ] {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
    }
    Home(d)
}

fn chooser(h: &Home, spec: Spec) -> (Driver, Answer) {
    let answer: Answer = Rc::new(RefCell::new(None));
    let a = answer.clone();
    let page = Files::with(Options {
        home: h.0.clone(),
        data: Some(h.0.join(".data")),
        engine: Box::new(Fake::new()),
        keyring: Keyring::over(None),
        pick: Some(Picker {
            spec,
            done: Box::new(move |p, _| *a.borrow_mut() = Some(p)),
        }),
    });
    let mut d = Driver::new(page);
    d.frame();
    (d, answer)
}

fn keys(d: &mut Driver, script: &str) {
    for line in script.lines() {
        d.keys(line);
        d.frame();
    }
}

fn sound() -> Filter {
    Filter {
        name: "Sound".into(),
        patterns: vec![Pattern::Mime("audio/*".into())],
    }
}

#[test]
fn open_picks_the_file_under_the_cursor_and_hides_other_types() {
    let h = home("one");
    let mut spec = Spec::new(Mode::Open, "Media");
    spec.filters = vec![sound()];
    spec.filter = Some(0);
    let (mut d, answer) = chooser(&h, spec);
    // Docs, Music: Music holds a.flac, b.flac; notes.txt is hidden.
    keys(&mut d, "x\nenter\nx\nenter");
    assert_eq!(*answer.borrow(), Some(Some(vec![h.0.join("Music/b.flac")])));
    assert!(d.quit());
}

#[test]
fn open_many_picks_the_marks() {
    let h = home("many");
    let mut spec = Spec::new(Mode::Open, "Media");
    spec.multiple = true;
    let (mut d, answer) = chooser(&h, spec);
    keys(&mut d, "x\nenter\nspace\nspace\nenter");
    let got = answer.borrow().clone().unwrap().unwrap();
    assert_eq!(got, [h.0.join("Music/a.flac"), h.0.join("Music/b.flac")]);
}

#[test]
fn save_asks_for_a_name_and_asks_before_replacing() {
    let h = home("save");
    let mut spec = Spec::new(Mode::Save, "Text");
    spec.name = "new.txt".into();
    spec.folder = Some(h.0.join("Docs"));
    let (mut d, answer) = chooser(&h, spec);
    keys(&mut d, "6");
    assert_eq!(d.dialog().map(|x| x.0), Some("Save as"));
    keys(&mut d, "enter");
    assert_eq!(*answer.borrow(), Some(Some(vec![h.0.join("Docs/new.txt")])));

    let mut spec = Spec::new(Mode::Save, "Text");
    spec.folder = Some(h.0.join("Docs"));
    let (mut d, answer) = chooser(&h, spec);
    keys(&mut d, "enter");
    assert_eq!(d.dialog().map(|x| x.0), Some("Replace plan.md?"));
    keys(&mut d, "esc");
    assert!(answer.borrow().is_none(), "kept the file");
    keys(&mut d, "enter\nenter");
    assert_eq!(*answer.borrow(), Some(Some(vec![h.0.join("Docs/plan.md")])));
}

#[test]
fn folder_mode_shows_folders_and_chooses_the_one_you_are_in() {
    let h = home("folder");
    let (mut d, answer) = chooser(&h, Spec::new(Mode::Folder, "Media"));
    keys(&mut d, "x\nenter");
    assert!(answer.borrow().is_none(), "Enter goes in");
    keys(&mut d, "6");
    assert_eq!(*answer.borrow(), Some(Some(vec![h.0.join("Music")])));
}

#[test]
fn esc_at_the_first_folder_goes_to_places_not_away() {
    let h = home("places");
    let (mut d, answer) = chooser(&h, Spec::new(Mode::Open, "Text"));
    keys(&mut d, "esc");
    assert!(!d.quit());
    assert!(answer.borrow().is_none());
}

#[test]
fn the_chooser_is_one_pane_tab_and_the_other_side_are_not_there() {
    let h = home("one-pane");
    let (mut d, answer) = chooser(&h, Spec::new(Mode::Open, "Text"));
    // Into Docs, then Tab: still Docs, and Enter picks from it.
    keys(&mut d, "enter\ntab\nenter");
    assert_eq!(*answer.borrow(), Some(Some(vec![h.0.join("Docs/plan.md")])));
}
