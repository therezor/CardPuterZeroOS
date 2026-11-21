//! The welcome screen and the tour: the keyboard, then the design system,
//! tried on the real chips, rows, menu and help. One idea a lesson; each
//! passes on the keys it asks for.

use super::{Shared, power_answer, power_command, power_menu};
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::widgets::{self, Hint, Message, Row, Trailing};
use gh0st_app::gfx::{Canvas, Rect, Theme};
use gh0st_app::prelude::*;

pub const LESSONS: usize = 7;
const TITLES: [&str; LESSONS] = [
    "Typing",
    "Moving",
    "Symbols",
    "The bars",
    "Rows",
    "Menu and help",
    "Taking care",
];
/// The moving lesson's board, and the ring to walk the dot onto.
const BOARD: (i32, i32) = (5, 4);
const RING: (i32, i32) = (3, 2);
const COLOURS: [&str; 3] = ["Mint", "Nord", "Paper"];
/// How long "That's it" shows before the next lesson.
const PASS_MS: u64 = 900;

const NEXT_LESSON: u32 = 901;
const SKIP_TOUR: u32 = 902;
const ERASE: u32 = 903;

/// Shown first while builds are pre-alpha (user, 2026-11-21). False drops
/// the screen.
pub const PRE_ALPHA: bool = true;

/// Before anything: what this build is.
pub struct Notice;

impl Page for Notice {
    fn title(&self) -> &str {
        "Before you start"
    }

    fn icon(&self) -> &str {
        "triangle-alert"
    }

    fn context(&self) -> Option<&str> {
        Some("")
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.done();
        }
    }

    /// There is nothing before it.
    fn back(&mut self, _: &mut Cx) -> bool {
        false
    }

    fn commands(&self, c: &mut Commands) {
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        power_command(id, cx);
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        power_answer(id, &answer, cx);
    }

    fn draw(&self, ui: &mut Ui) {
        let (t, area) = (*ui.theme(), ui.area());
        let keys = [Hint {
            key: "Enter",
            label: "go on",
        }];
        let m = Message {
            icon: "triangle-alert",
            tone: Some(Tone::Warning),
            text: "A pre-alpha test build",
            note: "It can be unstable,\nand the SDK and the design can still change.",
            keys: &keys,
        };
        widgets::message(ui.canvas(), &t, area, &m);
    }
}

pub struct Welcome {
    a: Shared,
}

impl Welcome {
    pub fn new(a: Shared) -> Welcome {
        Welcome { a }
    }
}

impl Page for Welcome {
    fn title(&self) -> &str {
        "Welcome"
    }

    fn context(&self) -> Option<&str> {
        Some("")
    }

    fn bars(&self) -> Bars {
        Bars::top().chip(6, Chip::new("skip-forward", "Skip"))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        let tour = match key {
            Key::Enter => true,
            Key::Soft(6) => false,
            _ => return,
        };
        self.a.borrow_mut().tour = tour;
        cx.done();
    }

    /// Back to the notice, if there is one.
    fn back(&mut self, _: &mut Cx) -> bool {
        PRE_ALPHA
    }

    fn commands(&self, c: &mut Commands) {
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        power_command(id, cx);
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        power_answer(id, &answer, cx);
    }

    fn draw(&self, ui: &mut Ui) {
        let (t, area) = (*ui.theme(), ui.area());
        let keys = [
            Hint {
                key: "Enter",
                label: "take the tour",
            },
            Hint {
                key: "6",
                label: "skip to settings",
            },
        ];
        let m = Message {
            icon: widgets::LOGO,
            tone: Some(Tone::Info),
            text: "Welcome to Gh0st OS",
            note: "A short tour of the keys, then the settings.",
            keys: &keys,
        };
        widgets::message(ui.canvas(), &t, area, &m);
    }
}

pub struct Lesson {
    n: usize,
    label: String,
    a: Shared,
    passed: bool,
    /// Typing and symbols: what is typed so far.
    typed: String,
    /// Moving: the dot.
    dot: (i32, i32),
    /// The bars: the Light chip.
    lit: bool,
    /// Rows: the selected row, the lamp and the colour.
    row: usize,
    lamp: bool,
    colour: usize,
}

impl Lesson {
    pub fn new(n: usize, label: &str, a: Shared) -> Lesson {
        Lesson {
            n,
            label: label.into(),
            a,
            passed: false,
            typed: String::new(),
            dot: (1, 1),
            lit: false,
            row: 1,
            lamp: false,
            colour: 0,
        }
    }

    /// The text a typing lesson asks for.
    fn word(&self) -> Option<&'static str> {
        match self.n {
            0 => Some("Hello"),
            2 => Some("me@home.net"),
            _ => None,
        }
    }

    /// The key of this lesson's Skip chip.
    fn skip_key(&self) -> u8 {
        match self.n {
            3 | 6 => 5,
            _ => 6,
        }
    }

    fn pass(&mut self, cx: &mut Cx) {
        if !self.passed {
            self.passed = true;
            cx.toast_tone(Tone::Ok, "That's it");
            cx.after(PASS_MS);
        }
    }

    fn type_key(&mut self, key: Key, cx: &mut Cx) {
        let Some(word) = self.word() else {
            return;
        };
        match key {
            Key::Tab => cx.done(),
            Key::Backspace => {
                self.typed.pop();
            }
            Key::Char(c) if word[self.typed.len()..].starts_with(c) => {
                self.typed.push(c);
                if self.typed == word {
                    self.pass(cx);
                }
            }
            _ => {}
        }
    }

    fn move_key(&mut self, key: Key, cx: &mut Cx) {
        let (x, y) = self.dot;
        self.dot = match key {
            Key::Up => (x, (y - 1).max(0)),
            Key::Down => (x, (y + 1).min(BOARD.1 - 1)),
            Key::Left => ((x - 1).max(0), y),
            Key::Right => ((x + 1).min(BOARD.0 - 1), y),
            Key::Enter if self.dot == RING => {
                self.pass(cx);
                self.dot
            }
            _ => self.dot,
        };
    }

    fn row_key(&mut self, key: Key, cx: &mut Cx) {
        let step = |c: usize, d: i32| (c as i32 + d).rem_euclid(COLOURS.len() as i32) as usize;
        match (key, self.row) {
            (Key::Up, _) => self.row = self.row.saturating_sub(1),
            (Key::Down, _) => self.row = (self.row + 1).min(2),
            (Key::Enter, 0) => self.lamp = !self.lamp,
            (Key::Enter, 2) => cx.toast("That opens a page"),
            (Key::Enter | Key::Right | Key::Soft(7), 1) => self.colour = step(self.colour, 1),
            (Key::Left | Key::Soft(5), 1) => self.colour = step(self.colour, -1),
            _ => {}
        }
        if self.lamp && self.colour != 0 {
            self.pass(cx);
        }
    }
}

impl Page for Lesson {
    fn title(&self) -> &str {
        TITLES[self.n]
    }

    fn icon(&self) -> &str {
        "keyboard"
    }

    fn context(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn typing(&self) -> bool {
        self.word().is_some()
    }

    fn bars(&self) -> Bars {
        let skip = Chip::new("skip-forward", "Skip");
        let bars = Bars::top();
        match self.n {
            0 | 2 => bars.hints([("Esc", "back"), ("Tab", "skip")]),
            3 => bars
                .chip(5, skip)
                .chip(7, Chip::new("sun", "Light").on(self.lit)),
            4 if self.row == 1 => bars
                .chip(5, Chip::new("chevron-left", "Prev"))
                .chip(6, skip)
                .chip(7, Chip::new("chevron-right", "Next")),
            6 => bars
                .chip(5, skip)
                .chip(7, Chip::new("", "Erase").danger().hold().runs(ERASE)),
            _ => bars.chip(6, skip),
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.passed {
            return;
        }
        if key == Key::Soft(self.skip_key()) {
            return cx.done();
        }
        match self.n {
            0 | 2 => self.type_key(key, cx),
            1 => self.move_key(key, cx),
            3 if key == Key::Soft(7) => {
                self.lit = !self.lit;
                self.pass(cx);
            }
            4 => self.row_key(key, cx),
            _ => {}
        }
    }

    fn timer(&mut self, _: Timer, cx: &mut Cx) {
        cx.done();
    }

    fn commands(&self, c: &mut Commands) {
        c.group("Setup");
        if self.n == 5 {
            c.add(Command::action(NEXT_LESSON, "Next lesson"));
        }
        c.add(Command::action(SKIP_TOUR, "Skip the tour"));
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        match id {
            NEXT_LESSON | ERASE => self.pass(cx),
            SKIP_TOUR => {
                self.a.borrow_mut().tour = false;
                cx.done();
            }
            _ => {
                power_command(id, cx);
            }
        }
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        power_answer(id, &answer, cx);
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        let r = Rect::new(
            area.x + t.m.margin,
            area.y + t.m.margin_v,
            area.w - 2 * t.m.margin,
            area.h - 2 * t.m.margin_v,
        );
        let c = ui.canvas();
        match self.n {
            0 => typing(c, &t, r, "Hello", self.typed.len(), TYPING),
            1 => moving(c, &t, r, self.dot),
            2 => typing(c, &t, r, "me@home.net", self.typed.len(), SYMBOLS),
            3 => bars(c, &t, r),
            4 => rows(c, &t, r, self.row, self.lamp, self.colour),
            5 => {
                para(c, &t, r.x, r.y, r.w, MENU, t.f.body, 18);
            }
            _ => {
                para(c, &t, r.x, r.y, r.w, CARE, t.f.small, 15);
            }
        }
    }
}

const TYPING: &str =
    "Press Shift, then the letter. Twice for caps lock. While you type, the bar shows keys.";
const SYMBOLS: &str = "Sym, then a key, types the symbol printed on it.";
const BARS: &str = "A word over a number key says what that key does. Press 7.";
const ROWS: &str = "The right side says what a row does.";
const MENU: &str = "8 opens the menu. Fn and H list every key on the screen. Esc closes them, one at a time. Press 8, then pick Next lesson.";
const CARE: &str = "What loses data is never on Enter: press it twice, or hold its key. Hold 7 to try. Hold Esc to leave an app, Fn and K to switch.";

/// Wrapped text from `y`; returns the y under it.
#[allow(clippy::too_many_arguments)]
fn para(c: &mut Canvas, t: &Theme, x: i32, y: i32, w: i32, s: &str, f: Font, lh: i32) -> i32 {
    let mut y = y;
    let mut line = String::new();
    let flush = |c: &mut Canvas, line: &mut String, y: &mut i32| {
        let lr = Rect::new(x + 3, *y, w - 6, lh);
        widgets::text_in(c, f, lr, line, widgets::soft(t));
        *y += lh;
        line.clear();
    };
    for word in s.split(' ') {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if f.measure(&next) > w - 6 && !line.is_empty() {
            flush(c, &mut line, &mut y);
            line.push_str(word);
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        flush(c, &mut line, &mut y);
    }
    y
}

/// The word to type, the typed part lit, a caret after it, then the note.
fn typing(c: &mut Canvas, t: &Theme, r: Rect, word: &str, typed: usize, note: &str) {
    let f = t.f.big;
    let y = r.y + 8;
    let mut x = r.x + (r.w - f.measure(word)) / 2;
    let base = y + f.cap();
    for (i, ch) in word.chars().enumerate() {
        let col = if i < typed { t.c.accent } else { t.c.border };
        x = f.draw(c, x, base, &ch.to_string(), col);
        if i + 1 == typed {
            c.fill(Rect::new(x + 1, y - 2, 2, f.cap() + 6), t.c.accent);
        }
    }
    para(c, t, r.x, y + 34, r.w, note, t.f.small, 15);
}

/// The board with the dot and the ring, and the keys that move.
fn moving(c: &mut Canvas, t: &Theme, r: Rect, dot: (i32, i32)) {
    let cell = 22;
    let board = Rect::new(r.x + 4, r.y + 2, cell * BOARD.0, cell * BOARD.1);
    for gy in 0..BOARD.1 {
        for gx in 0..BOARD.0 {
            let cr = Rect::new(
                board.x + gx * cell + 2,
                board.y + gy * cell + 2,
                cell - 4,
                cell - 4,
            );
            c.round_rect(cr, 3, t.c.surface);
        }
    }
    let centre = |(gx, gy): (i32, i32)| {
        (
            (board.x + gx * cell + cell / 2) as f32,
            (board.y + gy * cell + cell / 2) as f32,
        )
    };
    let (tx, ty) = centre(RING);
    c.circle(tx, ty, 7.0, t.c.accent);
    c.circle(tx, ty, 5.0, t.c.surface);
    let (dx, dy) = centre(dot);
    c.circle(dx, dy, 5.0, t.c.fg);
    let right = board.right() + 10;
    for (i, (keys, what)) in [("f", "up"), ("x", "down"), ("z", "left"), ("c", "right")]
        .iter()
        .enumerate()
    {
        let kr = Rect::new(right - 9, r.y + i as i32 * 19, r.right() - right + 9, 19);
        widgets::keys_line(c, t, kr, keys, 22, what, "");
    }
    let note = Rect::new(right, r.y + 4 * 19 + 2, r.right() - right, 15);
    widgets::text_in(c, t.f.small, note, "Typing? Add Fn.", t.c.muted);
}

/// Callouts up to the top bar's two ends, then the chips.
fn bars(c: &mut Canvas, t: &Theme, r: Rect) {
    let ink = t.c.accent;
    c.fill(Rect::new(r.x + 10, t.m.bar_h, 1, 7), ink);
    let lr = Rect::new(r.x + 3, r.y + 6, 120, 14);
    widgets::text_in(c, t.f.small, lr, "where you are", ink);
    c.fill(Rect::new(r.right() - 30, t.m.bar_h, 1, 7), ink);
    let rr = Rect::new(r.right() - 160, r.y + 6, 157, 14);
    widgets::text_right(c, t.f.small, rr.right(), rr, "clock, battery, network", ink);
    para(c, t, r.x, r.y + 28, r.w, BARS, t.f.body, 18);
}

/// Three rows whose right sides differ: a switch, a choice, a page.
fn rows(c: &mut Canvas, t: &Theme, r: Rect, sel: usize, lamp: bool, colour: usize) {
    let y = para(c, t, r.x, r.y, r.w, ROWS, t.f.small, 15) + 2;
    let a = t.c.accent;
    let items = [
        Row {
            icon: Some("sun"),
            tint: a,
            label: "Lamp",
            trailing: Trailing::Toggle(lamp),
        },
        Row {
            icon: Some("palette"),
            tint: a,
            label: "Colour",
            trailing: Trailing::Choice(COLOURS[colour]),
        },
        Row {
            icon: Some("settings"),
            tint: a,
            label: "More",
            trailing: Trailing::Chevron,
        },
    ];
    for (i, it) in items.iter().enumerate() {
        let rr = Rect::new(r.x - 2, y + i as i32 * t.m.row_h, r.w + 4, t.m.row_h);
        widgets::list_row(c, t, rr, it, i == sel);
    }
}
