//! The editor page driven by keys, in virtual time.

use gh0st_app::testing::Driver;
use gh0st_text::app::{Editor, KeyModel, Options};
use std::path::{Path, PathBuf};

fn temp(name: &str, text: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gh0st-text-page-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

fn open(p: &Path, keys: KeyModel) -> Driver {
    let opts = Options {
        keys: Some(keys),
        data: Some(data()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(p), opts));
    d.wait(100);
    d
}

#[test]
fn typing_and_saving_write_the_file() {
    let p = temp("save.txt", b"one\ntwo\n");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("fn+x\nfn+n\ntype  more\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"one\ntwo more\n");
    assert_eq!(d.toast(), Some("Saved"));
    assert!(d.lint().is_empty());
}

#[test]
fn undo_and_redo_reach_the_saved_text() {
    let p = temp("undo.txt", b"abc\n");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("type xyz\nctrl+z\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"abc\n");
    d.keys("ctrl+y\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"xyzabc\n");
}

#[test]
fn closing_with_changes_asks_to_save() {
    let p = temp("quit.txt", b"keep\n");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("type x\nesc");
    assert_eq!(d.dialog().map(|(t, _)| t), Some("Save quit.txt?"));
    d.keys("enter");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"xkeep\n");
    assert!(d.quit());
}

#[test]
fn view_mode_moves_and_enter_edits() {
    let p = temp("modal.txt", b"first\nsecond\n");
    let mut d = open(&p, KeyModel::Modal);
    // Letters move in View, and type nothing.
    d.keys("x\nc\nenter\ntype Z\nesc\nctrl+s");
    d.wait(100);
    // Ctrl+S works from View too.
    d.keys("ctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"first\nsZecond\n");
}

#[test]
fn find_goes_round_to_the_top() {
    let p = temp("find.txt", b"alpha\nbeta\nalpha again\n");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("ctrl+f\ntype again\nenter");
    d.wait(100);
    d.keys("esc\nctrl+f\ntype alpha\nenter");
    d.wait(100);
    // No alpha after "again": round to the top.
    d.keys("esc\ntype X\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"Xalpha\nbeta\nalpha again\n");
}

#[test]
fn a_binary_file_opens_read_only_as_hex() {
    let p = temp("bin.dat", b"\x00\x01\x02text");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("type abc\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"\x00\x01\x02text");
    assert!(d.lint().is_empty());
}

#[test]
fn an_idle_editor_wakes_for_nothing() {
    let mut big = Vec::new();
    while big.len() < 3 << 20 {
        big.extend_from_slice(b"a line of a log that goes on\n");
    }
    let p = temp("idle.log", &big);
    let mut d = open(&p, KeyModel::Modeless);
    // Let the index finish.
    d.wait(5_000);
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "{wakes}: the clock's minute and nothing more");
}

#[test]
fn a_file_opens_to_read_and_a_key_changes_nothing() {
    let p = temp("read.txt", b"one\ntwo\n");
    let opts = Options {
        data: Some(data()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(&p), opts));
    d.wait(100);
    d.keys("type xyz");
    d.keys("esc");
    assert!(d.dialog().is_none(), "nothing to save");
    assert_eq!(std::fs::read(&p).unwrap(), b"one\ntwo\n");
}

#[test]
fn quitting_with_changes_saves_keeps_editing_or_discards_held() {
    let p = temp("three.txt", b"one\n");
    let mut d = open(&p, KeyModel::Modal);
    let keys = |d: &mut Driver, script: &str| {
        for k in script.lines() {
            d.keys(k);
            d.wait(50);
        }
    };
    // Edit, type, back to reading, then Esc asks.
    keys(&mut d, "enter\ntype x\nesc\nesc");
    assert_eq!(d.dialog().map(|x| x.0), Some("Save three.txt?"));
    // Esc keeps editing.
    keys(&mut d, "esc");
    assert!(d.dialog().is_none() && !d.quit());
    // A tap on 7 discards nothing; held, the changes go and the page closes.
    keys(&mut d, "esc\n7");
    assert!(!d.quit());
    assert!(d.dialog().is_some(), "still asking");
    d.keys("down 7").wait(900).keys("up 7");
    assert!(d.quit());
    assert_eq!(std::fs::read(&p).unwrap(), b"one\n");
}

fn data() -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-text-data-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn editing(p: &Path) -> Driver {
    let opts = Options {
        keys: Some(KeyModel::Modeless),
        data: Some(data()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(p), opts));
    d.wait(100);
    d
}

#[test]
fn enter_after_an_open_bracket_indents_and_puts_the_closer_below() {
    let p = temp("indent.rs", b"fn a() {}\n");
    let mut d = editing(&p);
    d.keys("ctrl+e\nfn+z\nenter\ntype x;\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"fn a() {\n    x;\n}\n");
}

#[test]
fn a_closer_on_a_blank_line_steps_back_one_indent() {
    let p = temp("dedent.rs", b"fn a() {\n    x;\n");
    let mut d = editing(&p);
    d.keys("fn+x\nctrl+e\nenter\ntype }\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"fn a() {\n    x;\n}\n");
}

#[test]
fn comment_toggles_the_lines_in_one_undo() {
    let p = temp("toggle.py", b"a = 1\n  b = 2\n\nc = 3\n");
    let mut d = editing(&p);
    // Select from the first line into the last, then toggle.
    d.keys("shift+fn+x\nshift+fn+x\nshift+fn+x\nshift+fn+c\nctrl+k\nctrl+s");
    d.wait(100);
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"# a = 1\n#   b = 2\n\n# c = 3\n"
    );
    d.keys("ctrl+k\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"a = 1\n  b = 2\n\nc = 3\n");
    d.keys("ctrl+z\nctrl+s");
    d.wait(100);
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"# a = 1\n#   b = 2\n\n# c = 3\n"
    );
}

fn wrapping(p: &Path, wrap: bool) -> Driver {
    let opts = Options {
        keys: Some(KeyModel::Modeless),
        data: Some(data()),
        wrap: Some(wrap),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(p), opts));
    d.wait(100);
    d
}

#[test]
fn down_moves_by_rows_on_screen_when_long_lines_wrap() {
    let mut text = vec![b'a'; 120];
    text.extend_from_slice(b"\nb\n");
    let p = temp("wrap.txt", &text);
    let mut d = wrapping(&p, true);
    d.keys("fn+x\ntype X\nctrl+s");
    d.wait(100);
    let got = std::fs::read(&p).unwrap();
    let x = got.iter().position(|&b| b == b'X').unwrap();
    assert!(
        (40..70).contains(&x),
        "X went into the long line's second row, at {x}"
    );
    assert!(got.ends_with(b"\nb\n"));
    assert!(d.lint().is_empty());

    let p = temp("nowrap.txt", &text);
    let mut d = wrapping(&p, false);
    d.keys("fn+x\ntype X\nctrl+s");
    d.wait(100);
    assert!(std::fs::read(&p).unwrap().ends_with(b"\nXb\n"));
}

#[test]
fn the_end_of_a_wrapped_file_comes_into_view() {
    let mut text = Vec::new();
    for _ in 0..40 {
        text.extend_from_slice(&[b'w'; 150]);
        text.push(b'\n');
    }
    text.extend_from_slice(b"last\n");
    let p = temp("wraptail.txt", &text);
    let mut d = wrapping(&p, true);
    // To the end of the file: the last line must be drawn.
    d.keys("ctrl+down");
    d.wait(50);
    assert!(d.lint().is_empty());
    d.keys("type Z\nctrl+s");
    d.wait(100);
    assert!(std::fs::read(&p).unwrap().ends_with(b"last\nZ"));
}

#[test]
fn four_files_open_switch_and_close_one_at_a_time() {
    let a = temp("one.txt", b"one\n");
    let b = temp("two.txt", b"two\n");
    let mut d = editing(&a);
    d.open(&[b.as_path()]);
    d.keys("type B\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&b).unwrap(), b"Btwo\n");
    // Back to the first, type there; Alt+2 shows the second again.
    d.keys("ctrl+tab\ntype A\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&a).unwrap(), b"Aone\n");
    d.keys("alt+2\ntype C");
    // Closing a file with changes asks; held 7 drops them and shows the other.
    d.keys("ctrl+w");
    assert_eq!(d.dialog().map(|x| x.0), Some("Save two.txt?"));
    d.keys("down 7").wait(900).keys("up 7");
    assert!(!d.quit(), "one file is still open");
    assert_eq!(std::fs::read(&b).unwrap(), b"Btwo\n");
    d.keys("type Z\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&a).unwrap(), b"AZone\n");
    // Opening one that is open shows it, no second copy.
    d.open(&[a.as_path()]);
    d.keys("ctrl+w");
    assert!(d.quit(), "the last file closed the app");
}
