//! The editor page driven by keys, in virtual time.

use gh0st_app::portal::PickMode;
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

/// The cursor's bar shows: a column of accent below the top bar.
fn shows_cursor(d: &mut Driver) -> bool {
    let accent = gh0st_app::gfx::Theme::default().c.accent;
    let f = d.frame();
    (0..f.width()).any(|x| (20..f.height()).filter(|&y| f.get(x, y) == accent).count() >= 10)
}

#[test]
fn typing_at_a_long_wrapped_line_s_end_keeps_the_cursor_on_screen() {
    let mut text = b"top\n".to_vec();
    text.extend(std::iter::repeat_n(b'w', 3000));
    text.extend_from_slice(b"\nend\n");
    let p = temp("wrapend.txt", &text);
    let mut d = wrapping(&p, true);
    assert!(shows_cursor(&mut d));
    d.keys("fn+x\nctrl+e");
    assert!(
        shows_cursor(&mut d),
        "at the line's end, rows below the top"
    );
    for _ in 0..4 {
        d.keys(&format!("type {}", "v".repeat(40)));
        assert!(shows_cursor(&mut d), "after a new row");
    }
    d.keys("fn+x");
    assert!(shows_cursor(&mut d), "on the next line");
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

fn fresh(dir: &Path) -> Driver {
    let opts = Options {
        keys: Some(KeyModel::Modeless),
        data: Some(dir.to_path_buf()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(None, opts));
    d.wait(100);
    d
}

#[test]
fn with_no_file_open_asks_the_chooser_and_opens_what_it_picks() {
    let dir = std::env::temp_dir().join(format!("gh0st-text-start-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = temp("picked.txt", b"picked\n");
    let mut d = fresh(&dir);
    d.keys("enter");
    assert_eq!(d.picks().len(), 1, "Open asks the chooser");
    d.answer_pick(Some(vec![f.clone()]));
    d.keys("type X\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&f).unwrap(), b"Xpicked\n");
    // Next time, it is the first recent file, under Open and New.
    let mut d = fresh(&dir);
    d.keys("x\nx\nenter\ntype Y\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&f).unwrap(), b"YXpicked\n");
}

#[test]
fn a_new_file_saves_where_the_chooser_says() {
    let dir = std::env::temp_dir().join(format!("gh0st-text-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("made.txt");
    let _ = std::fs::remove_file(&target);
    let mut d = fresh(&dir);
    d.keys("x\nenter\ntype hello\nctrl+s");
    assert_eq!(d.picks().len(), 1, "a new file asks where");
    d.answer_pick(Some(vec![target.clone()]));
    d.wait(200);
    assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    // Saved, it is that file now: another save writes there with no ask.
    d.keys("type !\nctrl+s");
    d.wait(100);
    assert!(d.picks().is_empty());
    assert_eq!(std::fs::read(&target).unwrap(), b"hello!");
}

fn journaled(p: &Path, data: &Path) -> Driver {
    let opts = Options {
        keys: Some(KeyModel::Modeless),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(p), opts));
    d.wait(100);
    d
}

fn journal_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-text-jdata-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn edits_kept_when_paused_come_back_after_a_kill() {
    let data = journal_dir("kill");
    let p = temp("kept.txt", b"orig\n");
    let mut d = journaled(&p, &data);
    d.keys("type abc\nenter\nctrl+z\nctrl+y");
    // Fn+K: the app goes behind and the journal reaches the card.
    d.pause();
    drop(d);
    assert_eq!(std::fs::read(&p).unwrap(), b"orig\n", "nothing saved");
    let mut d = journaled(&p, &data);
    assert_eq!(d.dialog().map(|x| x.0), Some("Restore unsaved edits?"));
    d.keys("enter\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"abc\norig\n");
    // Saved: nothing to restore next time.
    drop(d);
    let d = journaled(&p, &data);
    assert!(d.dialog().is_none());
}

#[test]
fn a_torn_journal_restores_and_keeps_what_comes_after() {
    let data = journal_dir("torn");
    let p = temp("torn.txt", b"base\n");
    let mut d = journaled(&p, &data);
    d.keys("type one");
    d.pause();
    d.keys("type two");
    d.pause();
    drop(d);
    // Killed part way through the second write.
    let j = gh0st_text::journal::path_for(&data, &p);
    let len = std::fs::metadata(&j).unwrap().len();
    let f = std::fs::OpenOptions::new().write(true).open(&j).unwrap();
    f.set_len(len - 2).unwrap();
    drop(f);
    let mut d = journaled(&p, &data);
    d.keys("enter\nctrl+e\ntype three");
    d.pause();
    drop(d);
    let mut d = journaled(&p, &data);
    assert_eq!(d.dialog().map(|x| x.0), Some("Restore unsaved edits?"));
    d.keys("enter\nctrl+s");
    d.wait(100);
    // The torn "o" is gone; the rest, before and after, came back.
    assert_eq!(std::fs::read(&p).unwrap(), b"onetwbasethree\n");
}

#[test]
fn the_journal_follows_save_as() {
    let data = journal_dir("saveas");
    let p = temp("before.txt", b"x\n");
    let mut d = journaled(&p, &data);
    // Help, then the menu: Open, then Save as.
    d.keys("type a\nhelp\n8\nx\nenter");
    let target = data.join("after.txt");
    assert_eq!(d.picks().last().map(|s| s.mode), Some(PickMode::Save));
    assert_eq!(
        d.picks().last().map(|s| s.name.as_str()),
        Some("before.txt")
    );
    d.answer_pick(Some(vec![target.clone()]));
    d.wait(200);
    assert_eq!(std::fs::read(&target).unwrap(), b"ax\n");
    d.keys("ctrl+up\ntype b");
    d.pause();
    drop(d);
    assert!(
        journaled(&p, &data).dialog().is_none(),
        "not the old name's"
    );
    let mut d = journaled(&target, &data);
    assert_eq!(d.dialog().map(|x| x.0), Some("Restore unsaved edits?"));
    d.keys("enter\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&target).unwrap(), b"bax\n");
}

#[test]
fn edits_reach_the_card_two_seconds_after_typing() {
    let data = journal_dir("timer");
    let p = temp("timed.txt", b"t\n");
    let mut d = journaled(&p, &data);
    d.keys("type xy");
    let j = gh0st_text::journal::path_for(&data, &p);
    assert!(!j.exists());
    d.wait(2100);
    assert!(j.exists(), "written after two seconds");
    // Undone back to the file as saved: nothing left to keep.
    d.keys("ctrl+z");
    d.pause();
    assert!(!j.exists());
}

#[test]
fn edits_to_a_file_changed_since_open_as_a_new_file() {
    let data = journal_dir("changed");
    let p = temp("moved.txt", b"base\n");
    let mut d = journaled(&p, &data);
    d.keys("type kept words");
    d.pause();
    drop(d);
    std::fs::write(&p, b"someone else wrote this\n").unwrap();
    let mut d = journaled(&p, &data);
    assert_eq!(d.dialog().map(|x| x.0), Some("Keep the unsaved edits?"));
    d.keys("enter");
    // The second file holds what was typed; save it somewhere.
    d.keys("ctrl+s");
    let target = data.join("rescued.txt");
    d.answer_pick(Some(vec![target.clone()]));
    d.wait(200);
    assert_eq!(std::fs::read(&target).unwrap(), b"kept words\n");
    assert_eq!(std::fs::read(&p).unwrap(), b"someone else wrote this\n");
}

#[test]
fn cr_lf_lines_stay_cr_lf_through_moves_edits_and_new_lines() {
    let p = temp("dos.txt", b"a\r\nb\r\nlast");
    let mut d = editing(&p);
    // Down, to the end of "b", type, then a new line and more.
    d.keys("fn+x\nctrl+e\ntype c\nenter\ntype d\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nbc\r\nd\r\nlast");
    // Right at a line's end goes to the next line's start, past both.
    d.keys("ctrl+up\nctrl+e\nfn+c\ntype X\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nXbc\r\nd\r\nlast");
    // Backspace takes the X; at the line's start, the whole CR LF.
    d.keys("backspace\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"a\r\nbc\r\nd\r\nlast");
    d.keys("backspace\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"abc\r\nd\r\nlast");
    // A new line at the end, after a line with no ending, is CR LF too.
    d.keys("ctrl+down\nenter\ntype e\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"abc\r\nd\r\nlast\r\ne");
}

#[test]
fn bytes_that_are_not_utf8_save_as_they_were() {
    let p = temp("raw.txt", b"\xff\xfe ok\n\xc3(\n");
    let mut d = editing(&p);
    d.keys("ctrl+e\ntype !\nctrl+s");
    d.wait(100);
    assert_eq!(std::fs::read(&p).unwrap(), b"\xff\xfe ok!\n\xc3(\n");
    assert!(d.lint().is_empty());
}

#[test]
fn nothing_wakes_while_reading_or_editing_once_the_journal_is_written() {
    let p = temp("still.txt", b"one\ntwo\n");
    let opts = Options {
        data: Some(data()),
        ..Options::default()
    };
    let mut d = Driver::new(Editor::open(Some(&p), opts));
    d.wait(100);
    assert!(d.idle_wakes(600_000) <= 11, "View");
    d.keys("enter\ntype x");
    // The journal's one write, two seconds on; then nothing.
    d.wait(2_500);
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "Edit: {wakes}");
}

#[test]
fn low_memory_keeps_the_edits_and_the_screen() {
    let p = temp("lowmem.rs", b"fn main() {\n    let x = 1;\n}\n");
    let mut d = open(&p, KeyModel::Modeless);
    d.keys("type // a");
    d.wait(100);
    let before = d.frame().pixels().to_vec();
    d.low_memory(255).wait(100);
    assert_eq!(
        d.frame().pixels(),
        &before[..],
        "read again, drawn the same"
    );
    d.keys("ctrl+s").wait(100);
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"// afn main() {\n    let x = 1;\n}\n"
    );
}

#[test]
fn an_opened_file_goes_to_the_shared_recent_list() {
    let p = temp("recent.txt", b"hello\n");
    let d = open(&p, KeyModel::Modal);
    assert_eq!(d.recent().first(), Some(&p));
}
