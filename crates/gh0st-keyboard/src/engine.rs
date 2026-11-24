//! Keys in, HID reports out. Pure: no fds, no clock. The pages feed it the
//! raw keys of the typing page and the actions of the other modes; a link
//! sends what it returns.
//!
//! - Letters, digits and named keys go by place (`usage::by_code`), with the
//!   modifiers held. The Sym layer's characters go through the host's layout,
//!   with Shift and AltGr set to suit the character.
//! - Alt sends Command to an Apple host. The latch key gives the host key
//!   the keyboard lacks (Win, Super, Search, or Option on Apple) to the next
//!   key; pressed twice it taps that key alone.
//! - Fn+Tab sends Home: Fn+K is the OS's.
//! - Keys the OS acts on are dropped. The host repeats held keys itself, so
//!   the SDK's repeats are dropped too.

use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::{code, media_key, os_key};

use crate::hid::layout::Layout;
use crate::hid::report::{Mouse, Report};
use crate::hid::usage::{self, key, mods};
use crate::host::System;

/// Which key latches the host key the keyboard lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatchKey {
    /// Fn then Alt.
    FnAlt,
    /// Fn then Enter.
    FnEnter,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub system: System,
    pub layout: &'static Layout,
    pub latch: LatchKey,
    /// Send the media keys (Fn+Q, W, E) to the host. Off while the OS acts
    /// on them.
    pub media_keys: bool,
}

/// What a press sent, for the typing page's echo line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echo {
    /// A character, with no modifier but Shift.
    Char(char),
    /// A key or a combination: the modifiers as sent, and the key's name or
    /// character.
    Combo { mods: u8, key: Named },
    /// A media key.
    Media(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Char(char),
    Key(&'static str),
    /// A modifier alone, as the latch pressed twice sends.
    None,
}

#[derive(Clone, Copy)]
struct Held {
    code: u16,
    usage: u8,
    /// Modifiers this key needs on and off, for a character.
    set: u8,
    clear: u8,
    /// The latch it took.
    latch: u8,
}

pub struct Engine {
    o: Options,
    mods_held: Vec<u16>,
    held: Vec<Held>,
    latch: u8,
    /// Presses the latch took: their releases send nothing.
    latch_codes: Vec<u16>,
    media_code: Option<u16>,
    kbd: [u8; 8],
    consumer: u16,
    buttons: u8,
}

impl Engine {
    pub fn new(o: Options) -> Engine {
        Engine {
            o,
            mods_held: Vec::new(),
            held: Vec::new(),
            latch: 0,
            latch_codes: Vec::new(),
            media_code: None,
            kbd: [0; 8],
            consumer: 0,
            buttons: 0,
        }
    }

    pub fn options(&self) -> Options {
        self.o
    }

    /// New options, for another host. Release everything first.
    pub fn set_options(&mut self, o: Options) {
        self.o = o;
    }

    /// The modifier the latch holds for the next key, if any.
    pub fn latched(&self) -> bool {
        self.latch != 0
    }

    /// Whether the host thinks a key or a button is down.
    pub fn holding(&self) -> bool {
        self.kbd != [0; 8] || self.consumer != 0 || self.buttons != 0
    }

    fn latch_bit(&self) -> u8 {
        if self.o.system.apple() {
            mods::RALT
        } else {
            mods::LGUI
        }
    }

    fn mod_bit(&self, c: u16) -> u8 {
        match c {
            code::LEFTSHIFT | code::RIGHTSHIFT => mods::LSHIFT,
            code::LEFTCTRL | code::RIGHTCTRL => mods::LCTRL,
            code::LEFTALT | code::RIGHTALT if self.o.system.apple() => mods::LGUI,
            code::LEFTALT | code::RIGHTALT => mods::LALT,
            _ => 0,
        }
    }

    fn physical(&self) -> u8 {
        self.mods_held.iter().fold(0, |m, &c| m | self.mod_bit(c))
    }

    /// The keyboard report for what is held now.
    fn report(&self) -> [u8; 8] {
        let mut m = self.physical();
        for h in &self.held {
            m |= h.latch;
        }
        if let Some(h) = self.held.last() {
            m = (m & !h.clear) | h.set;
        }
        let mut r = [m, 0, 0, 0, 0, 0, 0, 0];
        let mut n = 2;
        for h in &self.held {
            if n < 8 && !r[2..n].contains(&h.usage) {
                r[n] = h.usage;
                n += 1;
            }
        }
        r
    }

    fn send_kbd(&mut self, out: &mut Vec<Report>) {
        let r = self.report();
        if r != self.kbd {
            self.kbd = r;
            out.push(Report::Keyboard(r));
        }
    }

    fn send_consumer(&mut self, u: u16, out: &mut Vec<Report>) {
        if u != self.consumer {
            self.consumer = u;
            out.push(Report::Consumer(u));
        }
    }

    /// One key from the typing page. Returns what the press sent.
    pub fn key(&mut self, k: &RawKey, out: &mut Vec<Report>) -> Option<Echo> {
        let c = k.code;
        let press = match k.state {
            KeyState::Press => true,
            KeyState::Release => false,
            KeyState::Repeat => return None,
        };
        if matches!(c, code::FN | code::FN_M5 | code::SYM_M5) {
            return None;
        }
        if !press && let Some(i) = self.latch_codes.iter().position(|&x| x == c) {
            self.latch_codes.swap_remove(i);
            return None;
        }
        let latch_key = match self.o.latch {
            LatchKey::FnAlt => matches!(c, code::LEFTALT | code::RIGHTALT),
            LatchKey::FnEnter => c == code::ENTER,
        };
        if press && k.fn_ && latch_key {
            self.latch_codes.push(c);
            return self.press_latch(out);
        }
        if self.mod_bit(c) != 0 {
            if press {
                self.mods_held.push(c);
            } else if let Some(i) = self.mods_held.iter().position(|&x| x == c) {
                self.mods_held.swap_remove(i);
            }
            self.send_kbd(out);
            return None;
        }
        if self.media_code == Some(c) && !press {
            self.media_code = None;
            self.send_consumer(0, out);
            return None;
        }
        if !press {
            if self.held.iter().any(|h| h.code == c) {
                self.held.retain(|h| h.code != c);
                self.send_kbd(out);
            }
            return None;
        }
        if self.o.media_keys
            && media_key(c)
            && let Some(u) = usage::media(c)
        {
            self.media_code = Some(c);
            self.send_consumer(u, out);
            return Some(Echo::Media(u));
        }
        if os_key(c) {
            return None;
        }
        let (usage, set, clear, named) = if c == code::TAB && k.fn_ {
            (key::HOME, 0, 0, Named::Key("Home"))
        } else if let Some(u) = usage::by_code(c) {
            (u, 0, 0, name(u, k.text))
        } else if let Some(ch) = k.text
            && let Some(s) = self.o.layout.stroke(ch)
        {
            (s.usage, s.mods, mods::SHIFT | mods::RALT, Named::Char(ch))
        } else {
            return None;
        };
        let latch = std::mem::take(&mut self.latch);
        self.held.retain(|h| h.code != c);
        self.held.push(Held {
            code: c,
            usage,
            set,
            clear,
            latch,
        });
        self.send_kbd(out);
        let sent = self.kbd[0] & !mods::SHIFT;
        Some(match named {
            Named::Char(ch) if sent == 0 => Echo::Char(ch),
            key => Echo::Combo {
                mods: self.kbd[0],
                key,
            },
        })
    }

    fn press_latch(&mut self, out: &mut Vec<Report>) -> Option<Echo> {
        let bit = self.latch_bit();
        if self.latch == 0 {
            self.latch = bit;
            return None;
        }
        self.latch = 0;
        let m = self.physical() | bit;
        out.push(Report::Keyboard([m, 0, 0, 0, 0, 0, 0, 0]));
        self.kbd = [m, 0, 0, 0, 0, 0, 0, 0];
        self.send_kbd(out);
        Some(Echo::Combo {
            mods: m,
            key: Named::None,
        })
    }

    /// Everything up: before a dialog, on pause, when the link drops, and
    /// when the app stops. A host never keeps a key the user let go.
    pub fn release_all(&mut self, out: &mut Vec<Report>) {
        self.held.clear();
        self.mods_held.clear();
        self.latch_codes.clear();
        self.latch = 0;
        self.media_code = None;
        self.send_kbd(out);
        self.send_consumer(0, out);
        if self.buttons != 0 {
            self.buttons = 0;
            out.push(Report::Mouse(Mouse::default()));
        }
    }

    /// A combination pressed and let go, for a shortcut or a slide key.
    pub fn tap(&mut self, mods: u8, usage: u8, out: &mut Vec<Report>) {
        self.release_all(out);
        let r = [mods, 0, usage, 0, 0, 0, 0, 0];
        out.push(Report::Keyboard(r));
        out.push(Report::Keyboard([0; 8]));
        self.kbd = [0; 8];
    }

    /// A consumer key pressed and let go: play, volume, next.
    pub fn media_tap(&mut self, usage: u16, out: &mut Vec<Report>) {
        self.send_consumer(usage, out);
        self.send_consumer(0, out);
    }

    /// A consumer key held, or let go with 0, for volume held down.
    pub fn media_hold(&mut self, usage: u16, out: &mut Vec<Report>) {
        self.send_consumer(usage, out);
    }

    /// Mouse buttons as they are now, and a move.
    pub fn mouse(&mut self, m: Mouse, out: &mut Vec<Report>) {
        let moves = m.dx != 0 || m.dy != 0 || m.wheel != 0 || m.pan != 0;
        if moves || m.buttons != self.buttons {
            self.buttons = m.buttons;
            out.push(Report::Mouse(m));
        }
    }

    pub fn buttons(&self) -> u8 {
        self.buttons
    }
}

/// Text as key presses, for a snippet. Each character is a press and a
/// release; a dead key gets a space after it. Returns how many characters
/// the layout can't type: they are left out.
pub fn text(layout: &Layout, s: &str, out: &mut Vec<Report>) -> usize {
    let mut missing = 0;
    for ch in s.chars() {
        let Some(st) = layout.stroke(ch) else {
            missing += 1;
            continue;
        };
        out.push(Report::Keyboard([st.mods, 0, st.usage, 0, 0, 0, 0, 0]));
        out.push(Report::Keyboard([0; 8]));
        if st.dead {
            out.push(Report::Keyboard([0, 0, key::SPACE, 0, 0, 0, 0, 0]));
            out.push(Report::Keyboard([0; 8]));
        }
    }
    missing
}

/// A key's name for the echo line: its character, or a word.
fn name(usage: u8, text: Option<char>) -> Named {
    if let Some(c) = text.filter(|c| !c.is_control()) {
        return Named::Char(c);
    }
    Named::Key(match usage {
        key::ENTER => "Enter",
        key::ESC => "Esc",
        key::BACKSPACE => "Bksp",
        key::TAB => "Tab",
        key::CAPS => "Caps",
        key::INSERT => "Ins",
        key::HOME => "Home",
        key::PGUP => "PgUp",
        key::DELETE => "Del",
        key::END => "End",
        key::PGDN => "PgDn",
        key::RIGHT => "→",
        key::LEFT => "←",
        key::DOWN => "↓",
        key::UP => "↑",
        key::APPLICATION => "Menu",
        u if (key::F1..=key::F1 + 9).contains(&u) => F_NAMES[(u - key::F1) as usize],
        key::F11 => "F11",
        key::F12 => "F12",
        _ => "?",
    })
}

const F_NAMES: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"];
