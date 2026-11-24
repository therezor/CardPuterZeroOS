//! The engine on key sequences as the typing page gets them.

use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::code;
use gh0st_keyboard::engine::{Echo, Engine, LatchKey, Named, Options};
use gh0st_keyboard::hid::layout::US;
use gh0st_keyboard::hid::report::{Mouse, Report};
use gh0st_keyboard::hid::usage::{consumer, key, mods};
use gh0st_keyboard::host::System;

const A: u16 = 30;
const C: u16 = 46;
const E: u16 = 18;
const ONE: u16 = 2;

fn engine(system: System) -> Engine {
    Engine::new(Options {
        system,
        layout: &US,
        latch: LatchKey::FnAlt,
        media_keys: false,
    })
}

fn raw(code: u16, state: KeyState) -> RawKey {
    RawKey {
        code,
        state,
        ch: None,
        text: None,
        shift: false,
        ctrl: false,
        alt: false,
        fn_: false,
    }
}

fn down(code: u16) -> RawKey {
    raw(code, KeyState::Press)
}

fn up(code: u16) -> RawKey {
    raw(code, KeyState::Release)
}

fn with_fn(mut k: RawKey) -> RawKey {
    k.fn_ = true;
    k
}

fn with_text(mut k: RawKey, c: char) -> RawKey {
    k.text = Some(c);
    k
}

fn kbd(m: u8, keys: &[u8]) -> Report {
    let mut r = [m, 0, 0, 0, 0, 0, 0, 0];
    r[2..2 + keys.len()].copy_from_slice(keys);
    Report::Keyboard(r)
}

/// Feeds keys and returns every report they sent.
fn run(e: &mut Engine, keys: &[RawKey]) -> Vec<Report> {
    let mut out = Vec::new();
    for k in keys {
        e.key(k, &mut out);
    }
    out
}

#[test]
fn a_letter_goes_by_place() {
    let mut e = engine(System::Windows);
    let out = run(&mut e, &[down(A), up(A)]);
    assert_eq!(out, [kbd(0, &[key::A]), kbd(0, &[])]);
}

#[test]
fn shift_and_ctrl_go_with_the_key() {
    let mut e = engine(System::Windows);
    let out = run(
        &mut e,
        &[down(code::LEFTCTRL), down(C), up(C), up(code::LEFTCTRL)],
    );
    assert_eq!(
        out,
        [
            kbd(mods::LCTRL, &[]),
            kbd(mods::LCTRL, &[key::C]),
            kbd(mods::LCTRL, &[]),
            kbd(0, &[]),
        ]
    );
    let out = run(&mut e, &[down(code::LEFTSHIFT), down(ONE)]);
    assert_eq!(out[1], kbd(mods::LSHIFT, &[0x1e]));
}

#[test]
fn repeats_send_nothing_the_host_repeats() {
    let mut e = engine(System::Windows);
    let out = run(
        &mut e,
        &[down(A), raw(A, KeyState::Repeat), raw(A, KeyState::Repeat)],
    );
    assert_eq!(out, [kbd(0, &[key::A])]);
}

#[test]
fn a_sym_character_sets_its_own_shift() {
    let mut e = engine(System::Windows);
    // Sym+? types '!' as code 26, which is '[' by place: it must go as
    // Shift+1, with Shift added though none is held.
    let out = run(&mut e, &[with_text(down(26), '!'), up(26)]);
    assert_eq!(out, [kbd(mods::LSHIFT, &[0x1e]), kbd(0, &[])]);
    // '/' needs no Shift: a held Shift is taken off for it.
    let out = run(
        &mut e,
        &[down(code::LEFTSHIFT), with_text(down(92), '/'), up(92)],
    );
    assert_eq!(
        out,
        [
            kbd(mods::LSHIFT, &[]),
            kbd(0, &[0x38]),
            kbd(mods::LSHIFT, &[])
        ]
    );
}

#[test]
fn ctrl_stays_on_a_sym_character() {
    let mut e = engine(System::Windows);
    let out = run(&mut e, &[down(code::LEFTCTRL), with_text(down(92), '/')]);
    assert_eq!(out[1], kbd(mods::LCTRL, &[0x38]));
}

#[test]
fn alt_is_command_on_apple() {
    let mut e = engine(System::Mac);
    let out = run(&mut e, &[down(code::LEFTALT), down(C)]);
    assert_eq!(out[1], kbd(mods::LGUI, &[key::C]));
    let mut e = engine(System::Windows);
    let out = run(&mut e, &[down(code::LEFTALT), down(C)]);
    assert_eq!(out[1], kbd(mods::LALT, &[key::C]));
}

#[test]
fn the_latch_gives_the_next_key_the_host_key() {
    let mut e = engine(System::Windows);
    // Fn then Alt: the driver holds Fn until Alt comes up.
    let out = run(
        &mut e,
        &[with_fn(down(code::LEFTALT)), with_fn(up(code::LEFTALT))],
    );
    assert!(out.is_empty());
    assert!(e.latched());
    let out = run(&mut e, &[down(E), up(E), down(E)]);
    assert_eq!(
        out,
        [kbd(mods::LGUI, &[key::E]), kbd(0, &[]), kbd(0, &[key::E])]
    );
    assert!(!e.latched());
}

#[test]
fn the_latch_twice_taps_the_host_key_alone() {
    let mut e = engine(System::Windows);
    let latch = [with_fn(down(code::LEFTALT)), with_fn(up(code::LEFTALT))];
    run(&mut e, &latch);
    let mut out = Vec::new();
    let echo = e.key(&latch[0], &mut out);
    assert_eq!(out, [kbd(mods::LGUI, &[]), kbd(0, &[])]);
    assert_eq!(
        echo,
        Some(Echo::Combo {
            mods: mods::LGUI,
            key: Named::None
        })
    );
    // On Apple the latch is Option.
    let mut e = engine(System::Mac);
    run(&mut e, &latch);
    assert_eq!(run(&mut e, &[down(E)]), [kbd(mods::RALT, &[key::E])]);
}

#[test]
fn fn_enter_latches_when_chosen() {
    let mut e = Engine::new(Options {
        system: System::Linux,
        layout: &US,
        latch: LatchKey::FnEnter,
        media_keys: false,
    });
    let out = run(
        &mut e,
        &[with_fn(down(code::ENTER)), with_fn(up(code::ENTER))],
    );
    assert!(out.is_empty() && e.latched());
    // Fn+Alt is then a plain Alt.
    let mut e = engine(System::Linux);
    e.set_options(Options {
        latch: LatchKey::FnEnter,
        ..e.options()
    });
    let out = run(&mut e, &[with_fn(down(code::LEFTALT))]);
    assert_eq!(out, [kbd(mods::LALT, &[])]);
}

#[test]
fn fn_tab_is_home() {
    let mut e = engine(System::Windows);
    let out = run(&mut e, &[with_fn(down(code::TAB)), up(code::TAB)]);
    assert_eq!(out, [kbd(0, &[key::HOME]), kbd(0, &[])]);
}

#[test]
fn the_os_keys_send_nothing() {
    let mut e = engine(System::Windows);
    for c in [
        code::HOME,
        code::VOLUMEUP,
        code::MUTE,
        code::BRIGHTNESSUP,
        164,
    ] {
        assert!(run(&mut e, &[down(c), up(c)]).is_empty(), "code {c}");
    }
    for c in [code::FN_M5, code::SYM_M5] {
        assert!(run(&mut e, &[down(c), up(c)]).is_empty());
    }
}

#[test]
fn media_keys_go_to_the_host_when_on() {
    let mut e = Engine::new(Options {
        system: System::Android,
        layout: &US,
        latch: LatchKey::FnAlt,
        media_keys: true,
    });
    let out = run(&mut e, &[down(164), up(164)]);
    assert_eq!(
        out,
        [Report::Consumer(consumer::PLAY_PAUSE), Report::Consumer(0)]
    );
}

#[test]
fn six_keys_at_most() {
    let mut e = engine(System::Windows);
    let keys: Vec<RawKey> = (30..38).map(down).collect();
    let out = run(&mut e, &keys);
    let Some(Report::Keyboard(last)) = out.last() else {
        panic!("no report");
    };
    assert_eq!(&last[2..], &[0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b]);
}

#[test]
fn release_all_lets_go_of_everything() {
    let mut e = engine(System::Windows);
    let mut out = run(&mut e, &[down(code::LEFTCTRL), down(A)]);
    e.mouse(
        Mouse {
            buttons: 1,
            ..Mouse::default()
        },
        &mut out,
    );
    out.clear();
    e.release_all(&mut out);
    assert_eq!(out, [kbd(0, &[]), Report::Mouse(Mouse::default())]);
    assert!(!e.holding());
    // A key let go after the release sends nothing more.
    assert!(run(&mut e, &[up(A), up(code::LEFTCTRL)]).is_empty());
    // Nothing held: nothing to send.
    e.release_all(&mut out);
    assert_eq!(out.len(), 2);
}

#[test]
fn echoes_name_what_was_sent() {
    let mut e = engine(System::Windows);
    let mut out = Vec::new();
    assert_eq!(
        e.key(&with_text(down(A), 'a'), &mut out),
        Some(Echo::Char('a'))
    );
    e.key(&up(A), &mut out);
    e.key(&down(code::LEFTCTRL), &mut out);
    assert_eq!(
        e.key(&with_text(down(C), 'c'), &mut out),
        Some(Echo::Combo {
            mods: mods::LCTRL,
            key: Named::Char('c')
        })
    );
    assert_eq!(
        e.key(&down(code::ENTER), &mut out),
        Some(Echo::Combo {
            mods: mods::LCTRL,
            key: Named::Key("Enter")
        })
    );
}

#[test]
fn text_types_each_character_and_counts_the_rest() {
    let mut out = Vec::new();
    let missing = gh0st_keyboard::engine::text(&US, "Hi!\né", &mut out);
    assert_eq!(missing, 1);
    assert_eq!(
        out,
        [
            kbd(mods::LSHIFT, &[0x0b]),
            kbd(0, &[]),
            kbd(0, &[0x0c]),
            kbd(0, &[]),
            kbd(mods::LSHIFT, &[0x1e]),
            kbd(0, &[]),
            kbd(0, &[key::ENTER]),
            kbd(0, &[]),
        ]
    );
}

#[test]
fn a_tap_is_a_press_and_a_release() {
    let mut e = engine(System::Windows);
    let mut out = Vec::new();
    e.tap(mods::LGUI, key::L, &mut out);
    assert_eq!(out, [kbd(mods::LGUI, &[key::L]), kbd(0, &[])]);
    out.clear();
    e.media_tap(consumer::VOLUME_UP, &mut out);
    assert_eq!(
        out,
        [Report::Consumer(consumer::VOLUME_UP), Report::Consumer(0)]
    );
}
