//! Keys from raw evdev codes, without libinput or xkbcommon. The TCA8418
//! driver sends letters and digits as US codes, and its symbol layer as reused
//! codes that a table turns into characters (stock `cp0_keyboard_keymap.c`).
//! Fn is held like Shift. Arrows are Fn+F/Z/X/C, Home is Fn+K.

/// Linux input codes this keyboard sends.
pub mod code {
    pub const ESC: u16 = 1;
    pub const BACKSPACE: u16 = 14;
    pub const TAB: u16 = 15;
    pub const ENTER: u16 = 28;
    pub const LEFTCTRL: u16 = 29;
    pub const LEFTSHIFT: u16 = 42;
    pub const RIGHTSHIFT: u16 = 54;
    pub const LEFTALT: u16 = 56;
    pub const SPACE: u16 = 57;
    pub const CAPSLOCK: u16 = 58;
    pub const F10: u16 = 68;
    pub const RIGHTCTRL: u16 = 97;
    pub const RIGHTALT: u16 = 100;
    /// Fn+K. Opens the app switcher.
    pub const HOME: u16 = 102;
    /// Fn+H. Opens the app's menu.
    pub const HELP: u16 = 138;
    pub const UP: u16 = 103;
    pub const LEFT: u16 = 105;
    pub const RIGHT: u16 = 106;
    pub const DOWN: u16 = 108;
    pub const DELETE: u16 = 111;
    pub const MUTE: u16 = 113;
    pub const VOLUMEDOWN: u16 = 114;
    pub const VOLUMEUP: u16 = 115;
    pub const SYSRQ: u16 = 99;
    pub const COMPOSE: u16 = 127;
    pub const PRINT: u16 = 210;
    pub const BRIGHTNESSDOWN: u16 = 224;
    pub const BRIGHTNESSUP: u16 = 225;
    pub const FN: u16 = 464;
    /// What the CardputerZero's Fn and Sym keys send (KEY_DVD, KEY_CD). The
    /// kernel driver applies their layers itself, so Fn+F arrives as Up.
    pub const FN_M5: u16 = 389;
    pub const SYM_M5: u16 = 383;
    pub const F: u16 = 33;
    pub const X: u16 = 45;
    pub const Z: u16 = 44;
    pub const C: u16 = 46;
}

/// Delay and rate of software repeat, from stock.
pub const REPEAT_DELAY_MS: u64 = 500;
pub const REPEAT_RATE_MS: u64 = 50;
/// A missed Fn release must not leave Fn stuck: stock clears it after 10 s.
const FN_STALE_MS: u64 = 10_000;

/// The stock symbol layer: these codes type these characters, overriding US.
const SYM_DEFAULT: &[(u16, char)] = &[
    (26, '!'),
    (27, '@'),
    (39, '#'),
    (40, '$'),
    (41, '%'),
    (43, '^'),
    (51, '&'),
    (52, '*'),
    (53, '('),
    (94, ')'),
    (55, '~'),
    (69, '`'),
    (70, '_'),
    (71, '-'),
    (72, '+'),
    (73, '='),
    (74, '['),
    (75, ']'),
    (76, '{'),
    (77, '}'),
    (79, ';'),
    (80, ':'),
    (81, '\''),
    (82, '"'),
    (83, '<'),
    (85, '>'),
    (86, '\\'),
    (89, '|'),
    (90, ','),
    (91, '.'),
    (92, '/'),
    (93, '?'),
];

pub const SYM_MAP_PATH: &str = "/usr/share/keymaps/tca8418_keypad_m5stack_keymap.map";

/// X keysym names the map file uses, for the characters it can hold.
const SYM_NAMES: &[(&str, char)] = &[
    ("exclam", '!'),
    ("at", '@'),
    ("numbersign", '#'),
    ("dollar", '$'),
    ("percent", '%'),
    ("asciicircum", '^'),
    ("ampersand", '&'),
    ("asterisk", '*'),
    ("parenleft", '('),
    ("parenright", ')'),
    ("asciitilde", '~'),
    ("grave", '`'),
    ("underscore", '_'),
    ("minus", '-'),
    ("plus", '+'),
    ("equal", '='),
    ("bracketleft", '['),
    ("bracketright", ']'),
    ("braceleft", '{'),
    ("braceright", '}'),
    ("semicolon", ';'),
    ("colon", ':'),
    ("apostrophe", '\''),
    ("quotedbl", '"'),
    ("less", '<'),
    ("greater", '>'),
    ("backslash", '\\'),
    ("bar", '|'),
    ("comma", ','),
    ("period", '.'),
    ("slash", '/'),
    ("question", '?'),
    ("space", ' '),
];

/// Parses `keycode N = keysym` lines, at most 64, as stock does. A loaded file
/// replaces the built-in table entirely.
pub fn parse_sym_map(text: &str) -> Vec<(u16, char)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() == 64 {
            break;
        }
        let body = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = body.strip_prefix("keycode") else {
            continue;
        };
        let Some((n, sym)) = rest.split_once('=') else {
            continue;
        };
        let Ok(n) = n.trim().parse::<u16>() else {
            continue;
        };
        let sym = sym.split_whitespace().next().unwrap_or("");
        let ch = SYM_NAMES
            .iter()
            .find(|(name, _)| *name == sym)
            .map(|&(_, c)| c)
            .or_else(|| {
                let mut cs = sym.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => Some(c),
                    _ => None,
                }
            });
        if let Some(c) = ch {
            out.push((n, c));
        }
    }
    out
}

/// US letters, digits and the few punctuation keys the table does not cover.
fn us(code: u16, shift: bool) -> Option<char> {
    const ROWS: &[(u16, &str, &str)] = &[
        (2, "1234567890", "!@#$%^&*()"),
        (16, "qwertyuiop", "QWERTYUIOP"),
        (30, "asdfghjkl", "ASDFGHJKL"),
        (44, "zxcvbnm", "ZXCVBNM"),
    ];
    for &(start, plain, shifted) in ROWS {
        let Some(i) = code.checked_sub(start) else {
            continue;
        };
        let i = i as usize;
        if i < plain.len() {
            let s = if shift { shifted } else { plain };
            return s.chars().nth(i);
        }
    }
    match code {
        code::SPACE => Some(' '),
        12 => Some(if shift { '_' } else { '-' }),
        13 => Some(if shift { '+' } else { '=' }),
        _ => None,
    }
}

const US_ROWS: &[(u16, &str)] = &[
    (2, "1234567890"),
    (16, "qwertyuiop"),
    (30, "asdfghjkl"),
    (44, "zxcvbnm"),
];

/// The code that types `c` without Shift. For key scripts and tests.
/// The letter or digit a key types without Shift: Ctrl+S reads as `s`.
pub fn base_char(code: u16) -> Option<char> {
    us(code, false)
}

pub fn code_for_char(c: char) -> Option<u16> {
    if c == ' ' {
        return Some(code::SPACE);
    }
    US_ROWS
        .iter()
        .find_map(|&(start, row)| row.find(c).map(|i| start + i as u16))
        .or_else(|| SYM_DEFAULT.iter().find(|&&(_, s)| s == c).map(|&(k, _)| k))
}

/// A key by name, as a key script writes it: `enter`, `esc`, `fn`, `a`...
pub fn code_for_name(name: &str) -> Option<u16> {
    Some(match name {
        "enter" => code::ENTER,
        "esc" => code::ESC,
        "tab" => code::TAB,
        "backspace" => code::BACKSPACE,
        "space" => code::SPACE,
        "up" => code::UP,
        "down" => code::DOWN,
        "left" => code::LEFT,
        "right" => code::RIGHT,
        "fn" => code::FN,
        "home" => code::HOME,
        "shift" => code::LEFTSHIFT,
        "ctrl" => code::LEFTCTRL,
        "alt" => code::LEFTALT,
        "volup" => code::VOLUMEUP,
        "voldown" => code::VOLUMEDOWN,
        "mute" => code::MUTE,
        "brightup" => code::BRIGHTNESSUP,
        "brightdown" => code::BRIGHTNESSDOWN,
        "print" => code::PRINT,
        "help" => code::HELP,
        "pageup" => 104,
        "pagedown" => 109,
        "delete" => code::DELETE,
        "minus" => 12,
        "equal" => 13,
        _ => {
            let mut cs = name.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => return code_for_char(c),
                _ => return None,
            }
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: u16,
    pub state: State,
    /// The character this key types, if any, with Shift and Caps applied.
    /// None while Ctrl or Alt is held.
    pub ch: Option<char>,
    /// The character under the key with Shift, Caps and the symbol layer
    /// applied, even while Ctrl or Alt is held: a terminal turns Ctrl+[
    /// into ESC from it.
    pub text: Option<char>,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub fn_: bool,
}

impl Key {
    /// Movement this key means. Arrow codes and Fn+F/Z/X/C always move. In a
    /// navigation context bare F/Z/X/C do too, as in stock.
    pub fn nav(&self, bare_letters: bool) -> Option<Nav> {
        match self.code {
            code::UP => return Some(Nav::Up),
            code::DOWN => return Some(Nav::Down),
            code::LEFT => return Some(Nav::Left),
            code::RIGHT => return Some(Nav::Right),
            _ => {}
        }
        if !self.fn_ && !bare_letters {
            return None;
        }
        match self.code {
            code::F => Some(Nav::Up),
            code::X => Some(Nav::Down),
            code::Z => Some(Nav::Left),
            code::C => Some(Nav::Right),
            _ => None,
        }
    }

    pub fn is(&self, code: u16) -> bool {
        self.code == code
    }
}

pub struct Keyboard {
    sym: Vec<(u16, char)>,
    shift: u8,
    ctrl: u8,
    alt: u8,
    caps: bool,
    fn_down: Option<u64>,
    repeat: Option<(Key, u64)>,
    repeat_all: bool,
}

impl Keyboard {
    pub fn new(sym: Vec<(u16, char)>) -> Keyboard {
        Keyboard {
            sym,
            shift: 0,
            ctrl: 0,
            alt: 0,
            caps: false,
            fn_down: None,
            repeat: None,
            repeat_all: false,
        }
    }

    /// Repeat every held key but Esc and the modifiers, as a terminal
    /// wants, instead of characters, arrows and deletion only.
    pub fn set_repeat_all(&mut self, on: bool) {
        self.repeat_all = on;
    }

    /// The map file if it exists, else the built-in table.
    pub fn load() -> Keyboard {
        let sym = std::fs::read_to_string(SYM_MAP_PATH)
            .map(|s| parse_sym_map(&s))
            .unwrap_or_default();
        Keyboard::new(if sym.is_empty() {
            SYM_DEFAULT.to_vec()
        } else {
            sym
        })
    }

    /// Feeds one evdev key event (`value` 0 up, 1 down; kernel repeats, 2,
    /// are ignored: repeat is ours). Returns the key, if it is one.
    pub fn event(&mut self, code: u16, value: i32, now_ms: u64) -> Option<Key> {
        if value == 2 {
            return None;
        }
        let down = value == 1;
        if self
            .fn_down
            .is_some_and(|t| now_ms.saturating_sub(t) >= FN_STALE_MS)
        {
            self.fn_down = None;
        }
        let count = |n: &mut u8| {
            if down {
                *n = n.saturating_add(1)
            } else {
                *n = n.saturating_sub(1)
            }
        };
        match code {
            code::LEFTSHIFT | code::RIGHTSHIFT => count(&mut self.shift),
            code::LEFTCTRL | code::RIGHTCTRL => count(&mut self.ctrl),
            code::LEFTALT | code::RIGHTALT => count(&mut self.alt),
            code::CAPSLOCK if down => self.caps = !self.caps,
            code::FN | code::FN_M5 => self.fn_down = down.then_some(now_ms),
            _ => {}
        }
        let key = self.make(code, if down { State::Press } else { State::Release });
        let repeat = if self.repeat_all {
            !is_modifier(code) && code != code::ESC
        } else {
            repeats(&key)
        };
        if down && repeat {
            self.repeat = Some((
                Key {
                    state: State::Repeat,
                    ..key
                },
                now_ms + REPEAT_DELAY_MS,
            ));
        } else if self.repeat.is_some_and(|(k, _)| k.code == code) || !down && is_modifier(code) {
            self.repeat = None;
        }
        Some(key)
    }

    fn make(&self, code: u16, state: State) -> Key {
        let shift = self.shift > 0;
        let text = if is_modifier(code) {
            None
        } else if let Some(&(_, c)) = self.sym.iter().find(|(k, _)| *k == code) {
            Some(c)
        } else {
            let c = us(code, shift);
            match c {
                Some(c) if self.caps && c.is_ascii_alphabetic() => Some(if shift {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                }),
                c => c,
            }
        };
        let ch = text.filter(|_| self.ctrl == 0 && self.alt == 0);
        Key {
            code,
            state,
            ch,
            text,
            shift,
            ctrl: self.ctrl > 0,
            alt: self.alt > 0,
            fn_: self.fn_down.is_some(),
        }
    }

    /// When the held key repeats next, for the event loop's timer.
    pub fn repeat_at(&self) -> Option<u64> {
        self.repeat.map(|(_, at)| at)
    }

    /// The repeated key, if its time has come.
    pub fn repeat_due(&mut self, now_ms: u64) -> Option<Key> {
        let (key, at) = self.repeat?;
        if now_ms < at {
            return None;
        }
        self.repeat = Some((key, now_ms + REPEAT_RATE_MS));
        Some(key)
    }

    /// Forget held keys, after a child app had the keyboard.
    pub fn reset(&mut self) {
        let all = self.repeat_all;
        *self = Keyboard::new(std::mem::take(&mut self.sym));
        self.repeat_all = all;
    }
}

fn is_modifier(code: u16) -> bool {
    matches!(
        code,
        code::LEFTSHIFT
            | code::RIGHTSHIFT
            | code::LEFTCTRL
            | code::RIGHTCTRL
            | code::LEFTALT
            | code::RIGHTALT
            | code::CAPSLOCK
            | code::FN
            | code::FN_M5
            | code::SYM_M5
    )
}

/// Characters, arrows and deletion repeat. Esc, Enter and Tab never do: each
/// of them means one step.
fn repeats(k: &Key) -> bool {
    k.ch.is_some() || k.nav(false).is_some() || matches!(k.code, code::BACKSPACE | code::DELETE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> Keyboard {
        Keyboard::new(SYM_DEFAULT.to_vec())
    }

    fn press(k: &mut Keyboard, code: u16) -> Key {
        k.event(code, 1, 0).unwrap()
    }

    #[test]
    fn letters_digits_and_shift() {
        let mut k = kb();
        assert_eq!(press(&mut k, 30).ch, Some('a'));
        assert_eq!(press(&mut k, 11).ch, Some('0'));
        k.event(code::LEFTSHIFT, 1, 0);
        assert_eq!(press(&mut k, 30).ch, Some('A'));
    }

    #[test]
    fn the_symbol_table_overrides_us_punctuation() {
        let mut k = kb();
        assert_eq!(press(&mut k, 39).ch, Some('#'), "US semicolon code types #");
        assert_eq!(press(&mut k, 90).ch, Some(','));
    }

    #[test]
    fn a_map_file_replaces_the_table() {
        let map = parse_sym_map(
            "# comment\nkeycode 26 = exclam\nkeycode 90 = comma # trailing\nbad line\nkeycode 91 = Z\n",
        );
        assert_eq!(map, [(26, '!'), (90, ','), (91, 'Z')]);
    }

    #[test]
    fn fn_turns_f_z_x_c_into_arrows_and_bare_letters_only_in_nav_context() {
        let mut k = kb();
        let f = press(&mut k, code::F);
        assert_eq!(f.nav(false), None);
        assert_eq!(f.nav(true), Some(Nav::Up));
        k.event(code::FN, 1, 0);
        let z = press(&mut k, code::Z);
        assert!(z.fn_);
        assert_eq!(z.nav(false), Some(Nav::Left));
    }

    #[test]
    fn a_stuck_fn_clears_after_ten_seconds() {
        let mut k = kb();
        k.event(code::FN, 1, 0);
        assert!(k.event(code::C, 1, 9_000).unwrap().fn_);
        assert!(!k.event(code::C, 1, 10_001).unwrap().fn_);
    }

    #[test]
    fn held_keys_repeat_after_the_delay_but_esc_does_not() {
        let mut k = kb();
        k.event(30, 1, 1000);
        assert_eq!(k.repeat_at(), Some(1500));
        assert!(k.repeat_due(1499).is_none());
        assert_eq!(k.repeat_due(1500).unwrap().state, State::Repeat);
        assert_eq!(k.repeat_at(), Some(1550));
        k.event(30, 0, 1560);
        assert_eq!(k.repeat_at(), None);
        k.event(code::ESC, 1, 2000);
        assert_eq!(k.repeat_at(), None);
    }

    #[test]
    fn names_map_back_to_codes() {
        let mut k = kb();
        for c in "az09 ".chars() {
            let code = code_for_char(c).unwrap();
            assert_eq!(press(&mut k, code).ch, Some(c));
        }
        assert_eq!(code_for_name("enter"), Some(code::ENTER));
        assert_eq!(code_for_name("nope"), None);
    }

    #[test]
    fn minus_and_equal_type_on_both_layers() {
        let mut k = kb();
        assert_eq!(press(&mut k, 12).ch, Some('-'));
        k.event(code::LEFTSHIFT, 1, 0);
        assert_eq!(press(&mut k, 13).ch, Some('+'));
    }

    #[test]
    fn ctrl_keys_type_nothing() {
        let mut k = kb();
        k.event(code::LEFTCTRL, 1, 0);
        let c = press(&mut k, 46);
        assert!(c.ctrl);
        assert_eq!(c.ch, None);
    }

    #[test]
    fn text_survives_ctrl_and_alt() {
        let mut k = kb();
        k.event(code::LEFTCTRL, 1, 0);
        let b = press(&mut k, 74);
        assert_eq!((b.ch, b.text), (None, Some('[')));
        k.event(code::LEFTCTRL, 0, 0);
        k.event(code::LEFTALT, 1, 0);
        k.event(code::LEFTSHIFT, 1, 0);
        assert_eq!(press(&mut k, 30).text, Some('A'));
    }

    #[test]
    fn repeat_all_takes_enter_but_never_esc() {
        let mut k = kb();
        k.set_repeat_all(true);
        k.event(code::ENTER, 1, 0);
        assert!(k.repeat_at().is_some());
        k.event(code::ENTER, 0, 10);
        k.event(code::ESC, 1, 20);
        assert!(k.repeat_at().is_none());
        k.event(code::FN_M5, 1, 30);
        assert!(k.repeat_at().is_none());
    }
}
