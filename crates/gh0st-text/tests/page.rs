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
    let mut d = Driver::new(Editor::open(Some(&p), Options::default()));
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
