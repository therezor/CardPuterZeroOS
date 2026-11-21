//! Commands: what a page lets the user do, listed once. The SDK builds the
//! app menu, the Keys screen and command hints from the list, and runs a
//! command when its key is pressed. The guide's "Commands" has the rules.

use gh0st_core::keys::{self, Key as RawKey, code};
use std::borrow::Cow;

/// How a command was used. Actions get `Run`; toggles flip on any step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Enter on its menu row, or its key.
    Run,
    /// Left on its menu row, or the first of a choice's two keys.
    Prev,
    /// Right on its menu row, Enter on a choice, or a choice's only key or
    /// second key.
    Next,
    /// A key from a range such as `1-9`, counted from 0.
    Nth(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Action,
    Toggle(bool),
    Choice(Cow<'static, str>),
}

const MENU: u8 = 1;
const REPEAT: u8 = 2;
const CONFIRM: u8 = 4;
const OPENS: u8 = 8;
const OFF: u8 = 16;
const CLOSES: u8 = 32;

/// One thing the user can do: a menu row, its keys, a line on the Keys
/// screen and maybe a hint.
///
/// ```
/// use gh0st_app::Command;
///
/// const FIND: u32 = 1;
/// const SIZE: u32 = 2;
/// const SCROLL_UP: u32 = 3;
///
/// let find = Command::action(FIND, "Find").keys("n");
/// // One row, two keys: r steps back, t forward.
/// let size = Command::choice(SIZE, "Text size", "8x14").keys("r t");
/// // Keys only, live while the page is in its scroll mode.
/// let up = Command::action(SCROLL_UP, "Scroll up")
///     .keys("f")
///     .keys_only()
///     .repeats()
///     .mode("Scroll mode", true);
/// # let _ = (find, size, up);
/// ```
#[derive(Clone, Debug)]
pub struct Command {
    pub(crate) id: u32,
    pub(crate) label: Cow<'static, str>,
    pub(crate) kind: Kind,
    pub(crate) keys: &'static str,
    pub(crate) group: &'static str,
    pub(crate) mode: &'static str,
    pub(crate) live: bool,
    pub(crate) hint: &'static str,
    flags: u8,
}

impl Command {
    fn new(id: u32, label: Cow<'static, str>, kind: Kind) -> Command {
        Command {
            id,
            label,
            kind,
            keys: "",
            group: "",
            mode: "",
            live: true,
            hint: "",
            flags: MENU,
        }
    }

    /// Does something. From the menu, the menu closes first.
    pub fn action(id: u32, label: impl Into<Cow<'static, str>>) -> Command {
        Command::new(id, label.into(), Kind::Action)
    }

    /// On or off. It flips in place: the menu stays open.
    pub fn toggle(id: u32, label: impl Into<Cow<'static, str>>, on: bool) -> Command {
        Command::new(id, label.into(), Kind::Toggle(on))
    }

    /// One of a few values, shown as `‹ value ›`. Left and Right step it in
    /// place, and so does Enter.
    pub fn choice(
        id: u32,
        label: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Command {
        Command::new(id, label.into(), Kind::Choice(value.into()))
    }

    /// The keys that run it, as they read on the keyboard, split by spaces:
    /// `"n"`, `"r t"`, `"G"` (Shift+G), `"Shift+PgUp"`, `"Ctrl+s"`,
    /// `"Esc"`, `"1-9"`. A choice with two keys steps back on the first and
    /// forward on the second. The Keys screen and hints show them as the
    /// device's keys: PgUp shows as Fn+L.
    pub fn keys(mut self, spec: &'static str) -> Command {
        debug_assert!(
            spec.split(' ')
                .filter(|t| !t.is_empty())
                .all(|t| Chord::parse(t).is_some()),
            "bad key spec {spec:?}"
        );
        self.keys = spec;
        self
    }

    /// A short word for a hint bar: `("f x", "scroll")`.
    pub fn hint(mut self, label: &'static str) -> Command {
        self.hint = label;
        self
    }

    /// The page mode its keys work in, named on the Keys screen, and whether
    /// that mode is on now. Its keys do nothing while it is off; its menu
    /// row and the menu's letter for it still work.
    pub fn mode(mut self, name: &'static str, active: bool) -> Command {
        self.mode = name;
        self.live = active;
        self
    }

    /// Keys and the Keys screen only, no menu row: for moving around.
    pub fn keys_only(mut self) -> Command {
        self.flags &= !MENU;
        self
    }

    /// Runs again while its key is held. Moves and steps should; actions
    /// that open or close something should not.
    pub fn repeats(mut self) -> Command {
        self.flags |= REPEAT;
        self
    }

    /// Asks before it runs from the menu: the first Enter arms the row,
    /// the second runs it. For anything that loses work.
    pub fn confirm(mut self) -> Command {
        self.flags |= CONFIRM;
        self
    }

    /// A toggle that closes the menu once flipped, for one that switches
    /// the page into a mode the user wants to use at once.
    pub fn closes(mut self) -> Command {
        self.flags |= CLOSES;
        self
    }

    /// Opens a page or a screen: the menu row gets a chevron.
    pub fn opens(mut self) -> Command {
        self.flags |= OPENS;
        self
    }

    /// Shown dimmed and does nothing while `on` is false.
    pub fn enabled(mut self, on: bool) -> Command {
        if on {
            self.flags &= !OFF;
        } else {
            self.flags |= OFF;
        }
        self
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn in_menu(&self) -> bool {
        self.flags & MENU != 0
    }

    pub(crate) fn repeating(&self) -> bool {
        self.flags & REPEAT != 0
    }

    pub(crate) fn confirms(&self) -> bool {
        self.flags & CONFIRM != 0
    }

    pub(crate) fn opening(&self) -> bool {
        self.flags & OPENS != 0
    }

    pub(crate) fn is_on(&self) -> bool {
        self.flags & OFF == 0
    }

    /// Changes in place in the menu, rather than closing it.
    pub(crate) fn stays(&self) -> bool {
        !matches!(self.kind, Kind::Action) && self.flags & CLOSES == 0
    }

    /// A choice whose two keys step back and forward.
    fn pair(&self) -> bool {
        matches!(self.kind, Kind::Choice(_)) && self.chords().filter(|c| !c.is_range()).count() == 2
    }

    fn chords(&self) -> impl Iterator<Item = Chord> + '_ {
        self.keys.split(' ').filter_map(Chord::parse)
    }

    /// The step a key gives, if it is one of this command's.
    fn step_for(&self, k: &RawKey, typing: bool) -> Option<Step> {
        let pair = self.pair();
        let mut plain = 0;
        for c in self.chords() {
            let hit = c.matches(k, typing);
            if c.is_range() {
                if let Some(n) = hit {
                    return Some(Step::Nth(n));
                }
                continue;
            }
            if hit.is_some() {
                return Some(match self.kind {
                    Kind::Choice(_) if pair && plain == 0 => Step::Prev,
                    Kind::Choice(_) => Step::Next,
                    _ => Step::Run,
                });
            }
            plain += 1;
        }
        None
    }

    /// The keys as the device shows them: `"f x"`, `"Shift+Fn+L"`.
    pub fn keys_text(&self) -> String {
        self.keys_shown(usize::MAX)
    }

    /// The key a hint shows: the first, or both of a choice's pair.
    pub(crate) fn hint_keys(&self) -> String {
        self.keys_shown(if self.pair() { 2 } else { 1 })
    }

    fn keys_shown(&self, n: usize) -> String {
        let mut out = String::new();
        for c in self.chords().take(n) {
            if !out.is_empty() {
                out.push(' ');
            }
            c.write(&mut out);
        }
        out
    }
}

/// A key a command matched, and how to run it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: u32,
    pub step: Step,
    /// The command runs again while the key is held.
    pub repeats: bool,
    pub(crate) index: usize,
}

/// A page's commands, in the order the page adds them. The menu keeps that
/// order within each group, and groups in the order they first appear.
#[derive(Default)]
pub struct Commands {
    pub(crate) list: Vec<Command>,
    group: &'static str,
}

impl Commands {
    pub fn new() -> Commands {
        Commands::default()
    }

    /// The menu section for the commands added after this.
    pub fn group(&mut self, name: &'static str) -> &mut Commands {
        self.group = name;
        self
    }

    pub fn add(&mut self, mut c: Command) -> &mut Commands {
        c.group = self.group;
        self.list.push(c);
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.list.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The live, enabled command that key `k` runs. `typing` is true while
    /// the page takes text: plain letters, digits and symbols type then.
    pub fn lookup(&self, k: &RawKey, typing: bool) -> Option<Hit> {
        self.find(k, typing, |c| c.live && c.is_on())
    }

    /// The menu's letters: any menu row's key, live or not.
    pub(crate) fn accelerator(&self, k: &RawKey) -> Option<Hit> {
        self.find(k, false, |c| c.in_menu())
    }

    fn find(&self, k: &RawKey, typing: bool, pick: impl Fn(&Command) -> bool) -> Option<Hit> {
        self.list.iter().enumerate().find_map(|(index, c)| {
            if !pick(c) {
                return None;
            }
            c.step_for(k, typing).map(|step| Hit {
                id: c.id,
                step,
                repeats: c.repeating(),
                index,
            })
        })
    }

    /// Key and word for each live command with a hint, for a hint bar. A
    /// hint shows a command's first key, or both keys of a choice's pair.
    pub fn hints(&self) -> Vec<(String, &'static str)> {
        self.list
            .iter()
            .filter(|c| c.live && !c.hint.is_empty())
            .map(|c| (c.hint_keys(), c.hint))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    /// A letter (lowercase), digit or symbol.
    Char(char),
    Code(u16),
    /// Digits `a` to `b`.
    Digits(u8, u8),
}

/// One key with its modifiers, as a spec names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    key: K,
    shift: bool,
    ctrl: bool,
    alt: bool,
    fn_: bool,
}

/// Named keys, with what the device's keycaps call them. The Fn layer's
/// keys show as Fn and the key that sends them.
const NAMES: &[(&str, u16, &str)] = &[
    ("esc", code::ESC, "Esc"),
    ("enter", code::ENTER, "Enter"),
    ("tab", code::TAB, "Tab"),
    ("space", code::SPACE, "Space"),
    ("bksp", code::BACKSPACE, "Bksp"),
    ("up", code::UP, "Fn+F"),
    ("down", code::DOWN, "Fn+X"),
    ("left", code::LEFT, "Fn+Z"),
    ("right", code::RIGHT, "Fn+C"),
    ("pgup", code::PAGEUP, "Fn+L"),
    ("pgdn", code::PAGEDOWN, "Fn+M"),
    ("home", code::HOME, "Fn+K"),
    ("end", code::END, "Fn+N"),
    ("ins", code::INSERT, "Fn+B"),
    ("del", code::DELETE, "Fn+Bksp"),
    ("help", code::HELP, "Fn+H"),
];

/// Fn and a letter arrive as another key: Fn+F is Up.
const FN_LAYER: &[(char, u16)] = &[
    ('f', code::UP),
    ('x', code::DOWN),
    ('z', code::LEFT),
    ('c', code::RIGHT),
    ('l', code::PAGEUP),
    ('m', code::PAGEDOWN),
    ('k', code::HOME),
    ('n', code::END),
    ('b', code::INSERT),
    ('h', code::HELP),
];

/// The arrow each of F, X, Z and C stands for where letters move.
const NAV: &[(char, keys::Nav)] = &[
    ('f', keys::Nav::Up),
    ('x', keys::Nav::Down),
    ('z', keys::Nav::Left),
    ('c', keys::Nav::Right),
];

impl Chord {
    fn parse(token: &str) -> Option<Chord> {
        if token.is_empty() {
            return None;
        }
        let (mods, key) = match token.strip_suffix("++") {
            Some(m) => (m, "+"),
            None => match token.rfind('+') {
                Some(i) if i + 1 < token.len() => (&token[..i], &token[i + 1..]),
                _ => ("", token),
            },
        };
        let mut c = Chord {
            key: K::Code(0),
            shift: false,
            ctrl: false,
            alt: false,
            fn_: false,
        };
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            match m.to_ascii_lowercase().as_str() {
                "shift" => c.shift = true,
                "ctrl" => c.ctrl = true,
                "alt" => c.alt = true,
                "fn" => c.fn_ = true,
                _ => return None,
            }
        }
        let mut chars = key.chars();
        let first = chars.next()?;
        c.key = if chars.next().is_none() {
            if first.is_ascii_uppercase() {
                c.shift = true;
            }
            K::Char(first.to_ascii_lowercase())
        } else if let [a @ b'0'..=b'9', b'-', b @ b'0'..=b'9'] = key.as_bytes() {
            K::Digits(a - b'0', b - b'0')
        } else {
            let low = key.to_ascii_lowercase();
            let &(_, code, _) = NAMES.iter().find(|n| n.0 == low)?;
            K::Code(code)
        };
        // Fn and a letter: name the key the driver sends instead.
        if let (true, K::Char(ch)) = (c.fn_, c.key) {
            if let Some(&(_, code)) = FN_LAYER.iter().find(|f| f.0 == ch) {
                c.key = K::Code(code);
                c.fn_ = false;
            }
        } else if c.fn_ && c.key == K::Code(code::BACKSPACE) {
            c.key = K::Code(code::DELETE);
            c.fn_ = false;
        }
        Some(c)
    }

    fn is_range(&self) -> bool {
        matches!(self.key, K::Digits(..))
    }

    /// Some(n) when `k` is this chord: n is the place in a range, else 0.
    fn matches(&self, k: &RawKey, typing: bool) -> Option<usize> {
        if k.ctrl != self.ctrl || k.alt != self.alt {
            return None;
        }
        let plain = !self.ctrl && !self.alt && !self.fn_;
        match self.key {
            K::Code(c) => {
                let fn_ok = layered(c) || k.fn_ == self.fn_;
                (k.code == c && k.shift == self.shift && fn_ok).then_some(0)
            }
            K::Digits(a, b) => {
                if typing || !plain || k.shift || k.fn_ {
                    return None;
                }
                let d = keys::base_char(k.code)?.to_digit(10)? as u8;
                (a..=b).contains(&d).then(|| (d - a) as usize)
            }
            K::Char(ch) => {
                if typing && plain {
                    return None;
                }
                if ch.is_ascii_alphanumeric() {
                    if k.shift != self.shift {
                        return None;
                    }
                    // F, X, Z and C stand for the arrows where letters move,
                    // so Fn+F works as well as F.
                    let nav = NAV.iter().find(|n| n.0 == ch).filter(|_| plain);
                    if let Some(&(_, nav)) = nav {
                        return (k.nav(true) == Some(nav)).then_some(0);
                    }
                    let base = keys::base_char(k.code)?;
                    (base == ch && k.fn_ == self.fn_).then_some(0)
                } else {
                    // A symbol: whatever layer typed it.
                    (k.text == Some(ch)).then_some(0)
                }
            }
        }
    }

    fn write(&self, out: &mut String) {
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        match self.key {
            K::Char(c) => {
                if self.fn_ {
                    out.push_str("Fn+");
                }
                out.push(if self.shift { c.to_ascii_uppercase() } else { c });
            }
            K::Digits(a, b) => {
                out.push((b'0' + a) as char);
                out.push('-');
                out.push((b'0' + b) as char);
            }
            K::Code(c) => {
                if self.shift {
                    out.push_str("Shift+");
                }
                if self.fn_ {
                    out.push_str("Fn+");
                }
                let name = NAMES.iter().find(|n| n.1 == c).map_or("?", |n| n.2);
                out.push_str(name);
            }
        }
    }
}

/// Keys the Fn layer sends: Fn is down when they arrive, so it doesn't
/// count against a match.
fn layered(c: u16) -> bool {
    FN_LAYER.iter().any(|f| f.1 == c) || c == code::DELETE
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_core::keys::Keyboard;

    /// Presses `mods` then `c` and returns the key event for `c`.
    fn key(mods: &[u16], c: u16) -> RawKey {
        let mut kb = Keyboard::new(Vec::new());
        for &m in mods {
            kb.event(m, 1, 0);
        }
        kb.event(c, 1, 0).unwrap()
    }

    const R: u16 = 19;
    const T: u16 = 20;
    const G: u16 = 34;

    #[test]
    fn specs_read_as_the_keyboard_shows_them() {
        let c = Command::action(1, "x").keys("r Shift+PgUp G Fn+f 1-9 Esc Ctrl+s");
        assert_eq!(c.keys_text(), "r Shift+Fn+L G Fn+F 1-9 Esc Ctrl+s");
        assert!(Chord::parse("Hyper+x").is_none());
        assert_eq!(Chord::parse("Ctrl++").map(|c| c.key), Some(K::Char('+')));
    }

    #[test]
    fn a_choice_with_two_keys_steps_both_ways() {
        let mut cs = Commands::new();
        cs.add(Command::choice(1, "Text size", "8x14").keys("r t"));
        let back = cs.lookup(&key(&[], R), false).unwrap();
        let fwd = cs.lookup(&key(&[], T), false).unwrap();
        assert_eq!((back.step, fwd.step), (Step::Prev, Step::Next));
    }

    #[test]
    fn shift_tells_g_from_capital_g() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Oldest").keys("g"));
        cs.add(Command::action(2, "Newest").keys("G"));
        assert_eq!(cs.lookup(&key(&[], G), false).unwrap().id, 1);
        let shifted = key(&[code::LEFTSHIFT], G);
        assert_eq!(cs.lookup(&shifted, false).unwrap().id, 2);
    }

    #[test]
    fn letters_type_while_typing_but_named_keys_still_run() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Find").keys("n"));
        cs.add(Command::action(2, "Older").keys("Enter"));
        assert!(cs.lookup(&key(&[], 49), true).is_none());
        assert_eq!(cs.lookup(&key(&[], code::ENTER), true).unwrap().id, 2);
    }

    #[test]
    fn f_moves_up_bare_or_with_fn() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Up").keys("f"));
        assert!(cs.lookup(&key(&[], code::F), false).is_some());
        assert!(cs.lookup(&key(&[code::FN_M5], code::UP), false).is_some());
        assert!(cs.lookup(&key(&[], code::F), true).is_none());
    }

    #[test]
    fn fn_with_a_key_it_does_not_change_is_its_own_chord() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Scroll mode").keys("Fn+Space"));
        cs.add(Command::choice(2, "Size", "").keys("Fn+r Fn+t"));
        assert!(cs.lookup(&key(&[], code::SPACE), true).is_none());
        let k = key(&[code::FN_M5], code::SPACE);
        assert_eq!(cs.lookup(&k, true).unwrap().id, 1, "fires while typing");
        let t = key(&[code::FN_M5], T);
        assert_eq!(cs.lookup(&t, true).unwrap().step, Step::Next);
        assert!(cs.lookup(&key(&[], T), false).is_none(), "plain t is not Fn+t");
        assert_eq!(cs.list[0].keys_text(), "Fn+Space");
    }

    #[test]
    fn ranges_give_the_place() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Tab").keys("1-9"));
        assert_eq!(cs.lookup(&key(&[], 4), false).unwrap().step, Step::Nth(2));
        assert!(cs.lookup(&key(&[], 11), false).is_none(), "0 is out");
    }

    #[test]
    fn shift_pgup_wants_shift_and_takes_fn() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Back").keys("Shift+PgUp"));
        let k = key(&[code::LEFTSHIFT, code::FN_M5], code::PAGEUP);
        assert!(cs.lookup(&k, true).is_some());
        assert!(cs.lookup(&key(&[], code::PAGEUP), true).is_none());
    }

    #[test]
    fn modes_off_and_disabled_commands_do_not_run() {
        let mut cs = Commands::new();
        cs.add(Command::action(1, "Find").keys("n").mode("Scroll", false));
        cs.add(Command::action(2, "Paste").keys("p").enabled(false));
        assert!(cs.lookup(&key(&[], 49), false).is_none());
        assert!(cs.lookup(&key(&[], 25), false).is_none());
        assert_eq!(cs.accelerator(&key(&[], 49)).unwrap().id, 1);
    }

    #[test]
    fn hints_come_from_live_commands() {
        let mut cs = Commands::new();
        cs.add(Command::choice(1, "Scroll", "").keys("f x").hint("scroll"));
        cs.add(Command::action(2, "Find").keys("n").hint("find").mode("x", false));
        cs.add(Command::action(3, "Done").keys("Esc q").hint("done"));
        let hints = cs.hints();
        assert_eq!(hints[0], ("f x".to_string(), "scroll"));
        assert_eq!(hints[1], ("Esc".to_string(), "done"), "the first key only");
    }
}
