//! A program's repaint shows whole. In synchronized output (mode 2026)
//! nothing is drawn until the block ends or times out.

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
fn a_sync_block_holds_every_frame_until_its_timeout() {
    let mut d = term(r"printf 'one\033[?2026htwo'; sleep 5");
    let before = d.frames();
    // The first frame is wanted since the start, and still waits.
    d.wait(90);
    assert_eq!(d.frames(), before, "drew inside the block");
    d.wait(100);
    assert!(d.frames() > before, "the timeout lets go");
}

#[test]
fn a_sync_block_draws_once_it_ends() {
    let mut d = term(r"printf 'one\033[?2026htwo\033[?2026l'; sleep 5");
    let before = d.frames();
    d.wait(30);
    assert!(d.frames() > before);
}

#[test]
fn plain_output_draws_within_a_frame() {
    let mut d = term(r"printf 'one'; sleep 5");
    let before = d.frames();
    d.wait(30);
    assert!(d.frames() > before);
}
