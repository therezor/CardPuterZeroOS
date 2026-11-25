//! The terminal and the clipboard every app shares: a program copies with
//! OSC 52, and Paste never brings back what the lock cleared.

use gh0st_app::testing::Driver;
use gh0st_term::app::{Options, Term};

fn term(script: &str) -> Driver {
    let opts = Options {
        command: Some(vec!["/bin/sh".into(), "-c".into(), script.into()]),
        sysplause: false,
    };
    let mut d = Driver::new(Term::new(opts));
    d.pump(500);
    d
}

#[test]
fn paste_brings_back_nothing_the_lock_cleared() {
    // "hello" in base64, then an empty payload that would clear it.
    let mut d = term(r"printf '\033]52;c;aGVsbG8=\a\033]52;c;\a'; exec cat");
    assert_eq!(d.clipboard(), Some(&b"hello"[..]));
    assert_eq!(d.toast(), Some("Copied"));
    d.clear_clipboard();
    d.keys("fn+space\np");
    assert_eq!(d.toast(), Some("Nothing copied"));
}
