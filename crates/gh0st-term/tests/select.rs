//! Select mode, Fn+V: a cursor of its own picks characters or whole
//! lines, and the text holds still while it does.

use gh0st_app::testing::Driver;
use gh0st_term::app::{Options, Term};

fn term(script: &str) -> Driver {
    let opts = Options {
        command: Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
        sysplause: false,
    };
    let mut d = Driver::new(Term::new(opts));
    d.pump(300);
    d
}

const TEXT: &str = r"printf 'alpha beta\r\ngamma delta\r\n'; exec cat";

fn copied(d: &Driver) -> String {
    String::from_utf8_lossy(d.clipboard().unwrap_or_default()).into_owned()
}

#[test]
fn characters_from_the_start_to_the_next_word() {
    let mut d = term(TEXT);
    // The head starts on the cursor, the empty third row.
    d.keys("fn+v\nf\nspace\nshift+c\nenter");
    assert_eq!(copied(&d), "gamma d");
    assert_eq!(d.toast(), Some("Copied 7 characters"));
}

#[test]
fn characters_across_rows() {
    let mut d = term(TEXT);
    d.keys("fn+space\nv\nf\nf\nc\nc\nc\nc\nc\nc\nspace\nx");
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    d.keys("enter");
    assert_eq!(copied(&d), "beta\ngamma d");
}

#[test]
fn whole_lines() {
    let mut d = term(TEXT);
    d.keys("fn+v\nf\nf\nl\nx\nenter");
    assert_eq!(copied(&d), "alpha beta\ngamma delta");
    assert_eq!(d.toast(), Some("Copied 2 lines"));
}

#[test]
fn no_start_no_copy() {
    let mut d = term(TEXT);
    d.keys("fn+v\nenter");
    assert_eq!(d.clipboard(), None);
    assert_eq!(d.toast(), Some("Space marks where it starts"));
}

#[test]
fn the_text_holds_still_while_selecting() {
    let mut d = term(r"printf 'one'; sleep 0.3; printf '\rtwo'; exec cat");
    d.keys("fn+v\nl");
    d.pump(600);
    d.keys("enter");
    assert_eq!(copied(&d), "one", "output came in while selecting");
    d.pump(300);
    d.keys("fn+v\nl\nenter");
    assert_eq!(copied(&d), "two", "output comes in after");
}

#[test]
fn a_wide_char_is_one_step() {
    let mut d = term("printf '\\346\\227\\245\\346\\234\\254\\350\\252\\236x\\r\\n'; exec cat");
    d.keys("fn+v\nf\nspace\nc\nc\nenter");
    assert_eq!(copied(&d), "日本語");
}

#[test]
fn from_history() {
    let mut d = term("i=1; while [ $i -le 40 ]; do echo line$i; i=$((i+1)); done; exec cat");
    d.keys("fn+v\ng\nspace\nx\nl\nenter");
    assert_eq!(copied(&d), "line1\nline2");
}

#[test]
fn scroll_modes_v_selects_too() {
    let mut d = term(TEXT);
    d.keys("fn+space\nv\nf\nl\nenter");
    assert_eq!(copied(&d), "gamma delta");
}

#[test]
fn esc_and_copy_go_back_to_typing() {
    let out = std::env::temp_dir().join(format!("gh0st-term-sel-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let script = format!(
        "printf 'alpha\\r\\n'; stty raw -echo; exec cat > '{}'",
        out.display()
    );
    let mut d = term(&script);
    d.keys("fn+v\nesc\nq");
    d.keys("fn+v\nf\nl\nenter\nw").pump(300);
    assert_eq!(copied(&d), "alpha");
    assert_eq!(std::fs::read(&out).ok().as_deref(), Some(&b"qw"[..]));
    let _ = std::fs::remove_file(&out);
}
