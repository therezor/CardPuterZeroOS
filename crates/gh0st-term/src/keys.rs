//! Key events to the bytes a program reads, as xterm sends them.

use crate::vt::Modes;
use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::code;

const F1: u16 = 59;
const F11: u16 = 87;
const F12: u16 = 88;
const PAGEUP: u16 = 104;
const END: u16 = 107;
const PAGEDOWN: u16 = 109;
const INSERT: u16 = 110;
pub const HELP: u16 = 138;

/// Keys the OS acts on. A program never sees them: Fn+K opens the
/// switcher, Help opens the app menu, the rest are volume, brightness, media
/// and screenshots.
fn os_key(c: u16) -> bool {
    matches!(
        c,
        code::HOME
            | HELP
            | code::MUTE
            | code::VOLUMEDOWN
            | code::VOLUMEUP
            | code::BRIGHTNESSDOWN
            | code::BRIGHTNESSUP
            | code::SYSRQ
            | code::PRINT
            | 163..=166
    )
}

/// xterm's modifier parameter: 1 plus Shift 1, Alt 2, Ctrl 4.
fn mods(k: &RawKey) -> u8 {
    1 + k.shift as u8 + 2 * k.alt as u8 + 4 * k.ctrl as u8
}

/// The control code Ctrl turns `c` into, if any.
fn control(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '7' | '/' | '-' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

/// `CSI 1;m X` with modifiers, else `CSI X`, or `SS3 X` when `ss3`.
fn cursor_key(out: &mut Vec<u8>, m: u8, ss3: bool, x: u8) {
    if m > 1 {
        out.extend_from_slice(format!("\x1b[1;{m}").as_bytes());
    } else if ss3 {
        out.extend_from_slice(b"\x1bO");
        out.push(x);
        return;
    } else {
        out.extend_from_slice(b"\x1b[");
    }
    out.push(x);
}

/// `CSI n ~`, or `CSI n;m ~` with modifiers.
fn tilde(out: &mut Vec<u8>, m: u8, n: u8) {
    if m > 1 {
        out.extend_from_slice(format!("\x1b[{n};{m}~").as_bytes());
    } else {
        out.extend_from_slice(format!("\x1b[{n}~").as_bytes());
    }
}

/// Appends what `k` sends to `out`. Releases and OS keys send nothing.
/// `alt_esc`: Alt sends ESC before the key, as most programs expect.
pub fn encode(k: &RawKey, modes: &Modes, alt_esc: bool, out: &mut Vec<u8>) {
    if k.state == KeyState::Release || os_key(k.code) {
        return;
    }
    let m = mods(k);
    let esc = |out: &mut Vec<u8>| {
        if k.alt && alt_esc {
            out.push(0x1b);
        }
    };
    match k.code {
        code::ESC => {
            esc(out);
            out.push(0x1b);
        }
        code::ENTER => {
            esc(out);
            out.push(b'\r');
        }
        code::TAB if k.shift => out.extend_from_slice(b"\x1b[Z"),
        code::TAB => {
            esc(out);
            out.push(b'\t');
        }
        code::BACKSPACE => {
            esc(out);
            out.push(if k.ctrl { 0x08 } else { 0x7f });
        }
        code::UP => cursor_key(out, m, modes.app_cursor, b'A'),
        code::DOWN => cursor_key(out, m, modes.app_cursor, b'B'),
        code::RIGHT => cursor_key(out, m, modes.app_cursor, b'C'),
        code::LEFT => cursor_key(out, m, modes.app_cursor, b'D'),
        END => cursor_key(out, m, modes.app_cursor, b'F'),
        INSERT => tilde(out, m, 2),
        code::DELETE => tilde(out, m, 3),
        PAGEUP => tilde(out, m, 5),
        PAGEDOWN => tilde(out, m, 6),
        c @ F1..=62 => cursor_key(out, m, true, b'P' + (c - F1) as u8),
        c @ 63..=code::F10 => {
            let n = [15, 17, 18, 19, 20, 21][(c - 63) as usize];
            tilde(out, m, n);
        }
        F11 => tilde(out, m, 23),
        F12 => tilde(out, m, 24),
        _ => {
            let Some(c) = k.text else {
                return;
            };
            esc(out);
            match control(c).filter(|_| k.ctrl) {
                Some(b) => out.push(b),
                None => {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
            }
        }
    }
}

/// Text to paste: wrapped in bracketed-paste marks when the program asked,
/// with newlines as Enter.
pub fn paste(text: &str, modes: &Modes, out: &mut Vec<u8>) {
    if modes.bracketed_paste {
        out.extend_from_slice(b"\x1b[200~");
    }
    // A pasted end mark must not end the paste early.
    let clean = text
        .replace("\x1b[201~", "")
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    out.extend_from_slice(clean.as_bytes());
    if modes.bracketed_paste {
        out.extend_from_slice(b"\x1b[201~");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16, text: Option<char>) -> RawKey {
        RawKey {
            code,
            state: KeyState::Press,
            ch: text,
            text,
            shift: false,
            ctrl: false,
            alt: false,
            fn_: false,
        }
    }

    fn send(k: RawKey, modes: &Modes) -> Vec<u8> {
        let mut out = Vec::new();
        encode(&k, modes, true, &mut out);
        out
    }

    #[test]
    fn chars_and_control_codes() {
        let m = Modes::default();
        assert_eq!(send(key(30, Some('a')), &m), b"a");
        let ctrl = |c| RawKey {
            ctrl: true,
            ch: None,
            ..key(0, Some(c))
        };
        assert_eq!(send(ctrl('c'), &m), [3]);
        assert_eq!(send(ctrl('['), &m), [0x1b]);
        assert_eq!(send(ctrl(' '), &m), [0]);
        let alt = RawKey {
            alt: true,
            ..key(30, Some('a'))
        };
        assert_eq!(send(alt, &m), b"\x1ba");
        assert_eq!(send(key(code::ENTER, None), &m), b"\r");
        assert_eq!(send(key(code::BACKSPACE, None), &m), [0x7f]);
    }

    #[test]
    fn arrows_follow_the_cursor_mode_and_modifiers() {
        let mut m = Modes::default();
        assert_eq!(send(key(code::UP, None), &m), b"\x1b[A");
        m.app_cursor = true;
        assert_eq!(send(key(code::UP, None), &m), b"\x1bOA");
        let ctrl = RawKey {
            ctrl: true,
            ..key(code::RIGHT, None)
        };
        assert_eq!(send(ctrl, &m), b"\x1b[1;5C");
    }

    #[test]
    fn function_and_editing_keys() {
        let m = Modes::default();
        assert_eq!(send(key(F1, None), &m), b"\x1bOP");
        assert_eq!(send(key(code::F10, None), &m), b"\x1b[21~");
        assert_eq!(send(key(F12, None), &m), b"\x1b[24~");
        assert_eq!(send(key(code::DELETE, None), &m), b"\x1b[3~");
        let shift_tab = RawKey {
            shift: true,
            ..key(code::TAB, None)
        };
        assert_eq!(send(shift_tab, &m), b"\x1b[Z");
    }

    #[test]
    fn os_keys_and_releases_send_nothing() {
        let m = Modes::default();
        assert!(send(key(code::HOME, None), &m).is_empty());
        assert!(send(key(code::VOLUMEUP, None), &m).is_empty());
        let up = RawKey {
            state: KeyState::Release,
            ..key(30, Some('a'))
        };
        assert!(send(up, &m).is_empty());
    }

    #[test]
    fn bracketed_paste_cannot_be_ended_early() {
        let m = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };
        let mut out = Vec::new();
        paste("a\n\x1b[201~b", &m, &mut out);
        assert_eq!(out, b"\x1b[200~a\rb\x1b[201~");
    }
}
