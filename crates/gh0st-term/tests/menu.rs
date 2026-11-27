//! An Esc tap while typing opens the menu. Its first row sends Esc to the
//! program, and Close app is there too. A hold exits, as in every app.

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

#[test]
fn esc_opens_the_menu_and_send_esc_reaches_the_program() {
    let out = std::env::temp_dir().join(format!("gh0st-term-esc-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let script = format!("stty raw -echo; exec cat > '{}'", out.display());
    let mut d = term(&script);
    d.keys("esc").wait(1000);
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    assert!(d.menu_open(), "Esc opens the menu");
    assert_ne!(d.toast(), Some("Hold to exit"));
    assert!(!d.quit());
    d.keys("enter").pump(300);
    assert!(!d.menu_open());
    assert_eq!(std::fs::read(&out).ok().as_deref(), Some(&b"\x1b"[..]));
    let _ = std::fs::remove_file(&out);
}

#[test]
fn esc_held_closes_with_the_same_toast_as_every_app() {
    let mut d = term("exec cat");
    d.wait(3000);
    d.keys("down esc").wait(300);
    assert_eq!(d.toast(), Some("Hold to exit"));
    assert!(!d.menu_open());
    d.wait(500);
    assert!(d.quit());
}

#[test]
fn fn_esc_sends_esc_while_typing() {
    let out = std::env::temp_dir().join(format!("gh0st-term-fnesc-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let script = format!("stty raw -echo; exec cat > '{}'", out.display());
    let mut d = term(&script);
    d.keys("fn+esc").pump(300);
    assert!(!d.menu_open());
    assert_eq!(std::fs::read(&out).ok().as_deref(), Some(&b"\x1b"[..]));
    let _ = std::fs::remove_file(&out);
}
