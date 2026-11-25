//! The notice, the welcome and the training: the keyboard, then the design
//! system, tried on the real chips, rows, menu and help, with a joke a
//! lesson. Each passes on the keys it asks for. The last screen hands out
//! the reward, the Phantom palette.

use super::{Shared, power_answer, power_command, power_menu};
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::widgets::{self, Hint, Message, Row, Trailing};
use gh0st_app::gfx::{Canvas, Rect, Theme};
use gh0st_app::prelude::*;

/// Shown first while builds are a test release. False drops the screen.
pub const PRERELEASE: bool = false;

/// The training's screens: eight lessons, then the reward.
pub const LESSONS: usize = 9;
const TITLES: [&str; LESSONS] = [
    "Typing",
    "Caps lock",
    "Moving",
    "Symbols",
    "The bars",
    "Rows",
    "Menu and help",
    "Taking care",
    "Graduation",
];
/// The moving lesson's board, where the dot starts, and the ring.
const BOARD: (i32, i32) = (5, 4);
const START: (i32, i32) = (1, 1);
const RING: (i32, i32) = (3, 2);
/// The fewest moves from the start to the ring: a sober walk.
const SOBER: u32 = 3;
const PARANOIA: [&str; 3] = ["Low", "Healthy", "Max"];
/// How long the pass line shows before the next lesson.
const PASS_MS: u64 = 1100;
/// Config keys: training done unlocks Phantom; the palette to wear it.
const TOUR_DONE: &str = "gh0st.tour_done";
const PALETTE: &str = gh0st_core::config::key::PALETTE;

const NEXT_LESSON: u32 = 901;
const SKIP_TOUR: u32 = 902;
const BOOM: u32 = 903;

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
        message(
            ui,
            "triangle-alert",
            Tone::Warning,
            "A beta test build",
            "It can still break,\nand the SDK may change.",
            &[("Enter", "go on")],
        );
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
        PRERELEASE
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
        message(
            ui,
            widgets::LOGO,
            Tone::Info,
            "Welcome to Gh0st OS",
            "We know nothing about you.\nLet's keep it that way.",
            &[("Enter", "take the training")],
        );
    }
}

pub struct Lesson {
    n: usize,
    label: String,
    a: Shared,
    passed: bool,
    /// The typing lessons: what is typed so far.
    typed: String,
    /// Moving: the dot, and how many moves it took.
    dot: (i32, i32),
    moves: u32,
    /// Rows: the selected row, the hat and the paranoia.
    row: usize,
    hat: bool,
    paranoia: usize,
}

impl Lesson {
    pub fn new(n: usize, label: &str, a: Shared) -> Lesson {
        Lesson {
            n,
            label: label.into(),
            a,
            passed: false,
            typed: String::new(),
            dot: START,
            moves: 0,
            row: 1,
            hat: false,
            paranoia: 1,
        }
    }

    /// The text a typing lesson asks for.
    fn word(&self) -> Option<&'static str> {
        match self.n {
            0 => Some("opsec"),
            1 => Some("OPSEC"),
            3 => Some(FAKE_EMAIL),
            _ => None,
        }
    }

    /// The key of this lesson's Skip chip, or the reward's Later.
    fn skip_key(&self) -> u8 {
        match self.n {
            4 | 7 => 5,
            _ => 6,
        }
    }

    /// Says so, then the next lesson after a beat.
    fn pass(&mut self, said: &str, cx: &mut Cx) {
        if !self.passed {
            self.passed = true;
            cx.toast_tone(Tone::Ok, said);
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
                    let said = match self.n {
                        0 => "Sworn in. We believe you. Mostly.",
                        1 => "The neighbours heard. Fine.",
                        _ => "Perfect. We'll never write.",
                    };
                    self.pass(said, cx);
                }
            }
            Key::Char(_) => cx.toast("Typo. The van outside saw that."),
            _ => {}
        }
    }

    fn move_key(&mut self, key: Key, cx: &mut Cx) {
        let (x, y) = self.dot;
        let to = match key {
            Key::Up => (x, (y - 1).max(0)),
            Key::Down => (x, (y + 1).min(BOARD.1 - 1)),
            Key::Left => ((x - 1).max(0), y),
            Key::Right => ((x + 1).min(BOARD.0 - 1), y),
            Key::Enter if self.dot == RING => {
                let said = if self.moves <= SOBER {
                    "Sober. Suspiciously sober."
                } else {
                    "Zigzag. We'll call it evasive driving."
                };
                return self.pass(said, cx);
            }
            _ => return,
        };
        self.moves += 1;
        self.dot = to;
    }

    fn row_key(&mut self, key: Key, cx: &mut Cx) {
        let n = PARANOIA.len() as i32;
        let step = |p: usize, d: i32| (p as i32 + d).clamp(0, n - 1) as usize;
        match (key, self.row) {
            (Key::Up, _) => self.row = self.row.saturating_sub(1),
            (Key::Down, _) => self.row = (self.row + 1).min(2),
            (Key::Enter, 0) => self.hat = !self.hat,
            (Key::Enter, 2) => cx.toast("Nice try."),
            (Key::Enter | Key::Right | Key::Soft(7), 1) => self.paranoia = step(self.paranoia, 1),
            (Key::Left | Key::Soft(5), 1) => self.paranoia = step(self.paranoia, -1),
            _ => {}
        }
        if self.hat && self.paranoia == PARANOIA.len() - 1 {
            self.pass("Fashionable and unreadable.", cx);
        }
    }
}

impl Page for Lesson {
    fn title(&self) -> &str {
        TITLES[self.n]
    }

    fn icon(&self) -> &str {
        if self.n == 8 { "" } else { "keyboard" }
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
            0 | 1 | 3 => bars.hints([("Esc", "back"), ("Tab", "skip")]),
            4 => bars
                .chip(5, skip)
                .chip(7, Chip::new("triangle-alert", "Don't")),
            5 if self.row == 1 => bars
                .chip(5, Chip::new("chevron-left", "Prev"))
                .chip(6, skip)
                .chip(7, Chip::new("chevron-right", "Next")),
            7 => bars
                .chip(5, skip)
                .chip(7, Chip::new("", "Boom").danger().hold().runs(BOOM)),
            8 => bars.chip(6, Chip::new("clock", "Later")),
            _ => bars.chip(6, skip),
        }
    }

    /// The reward screen marks the training done: Phantom is earned.
    fn start(&mut self, cx: &mut Cx) {
        if self.n == 8 {
            cx.settings().set_bool(TOUR_DONE, true);
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
            0 | 1 | 3 => self.type_key(key, cx),
            2 => self.move_key(key, cx),
            4 if key == Key::Soft(7) => self.pass("You pressed it. Of course you did.", cx),
            5 => self.row_key(key, cx),
            8 if key == Key::Enter => {
                cx.settings().set_str(PALETTE, "Phantom");
                cx.done();
            }
            _ => {}
        }
    }

    fn timer(&mut self, _: Timer, cx: &mut Cx) {
        cx.done();
    }

    fn commands(&self, c: &mut Commands) {
        c.group("Setup");
        if self.n == 6 {
            c.add(Command::action(NEXT_LESSON, "Next lesson"));
        }
        if self.n < 8 {
            c.add(Command::action(SKIP_TOUR, "Skip training"));
        }
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        match id {
            NEXT_LESSON => self.pass("It was never secret. Act surprised.", cx),
            BOOM => self.pass("Boom. Nothing happened. Breathe.", cx),
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
        if self.n == 8 {
            return message(
                ui,
                widgets::LOGO,
                Tone::Ok,
                "Training complete",
                "You were never here.\nUnlocked: Phantom, a palette for ghosts.",
                // Later is the chip on 6; a pill would say it twice.
                &[("Enter", "wear it")],
            );
        }
        let t = *ui.theme();
        let area = ui.area();
        let r = Rect::new(
            area.x + t.m.margin,
            area.y + t.m.margin_v,
            area.w - 2 * t.m.margin,
            area.h - 2 * t.m.margin_v,
        );
        let typed = self.typed.chars().count();
        let c = ui.canvas();
        match self.n {
            0 => typing(c, &t, r, "opsec", typed, SWEAR),
            1 => typing(c, &t, r, "OPSEC", typed, MEAN_IT),
            2 => moving(c, &t, r, self.dot),
            3 => typing(c, &t, r, FAKE_EMAIL, typed, EMAIL),
            4 => bars(c, &t, r),
            5 => rows(c, &t, r, self.row, self.hat, self.paranoia),
            6 => {
                para(c, &t, r.x, r.y, r.w, MENU, t.f.body, 18);
            }
            _ => {
                para(c, &t, r.x, r.y, r.w, BOOM_TEXT, t.f.small, 15);
            }
        }
    }
}

const SWEAR: &str = "Raise your right hand. Promise you won't hack anything. Type opsec to swear.";
const MEAN_IT: &str = "Now say it like you mean it. Press Shift twice for caps lock.";
const SOBRIETY: &str = "Field sobriety test. Walk the dot to the ring. Straight line, agent.";
/// No domain ending, so it can never be someone's address.
const FAKE_EMAIL: &str = "no@pe";
const EMAIL: &str =
    "Give us your email. Kidding. Type a fake one. Sym, then a key, types its symbol.";
const DONT: &str = "A word over a number key says what it does. See 7? It says Don't. So don't.";
const HAT: &str = "Put on your tinfoil hat. Turn paranoia up.";
const MENU: &str = "Secret menu on 8. Find Next lesson. Tell no one. Fn and H list every key.";
const BOOM_TEXT: &str = "Hold 7 to self-destruct. Relax, it's a drill. Anything that erases wants a hold, never Enter. Leaving an app is a hold too: Esc. Fn and K switch apps.";

/// A message filling the page: an icon, a line, a note, key pills.
fn message(ui: &mut Ui, icon: &str, tone: Tone, text: &str, note: &str, keys: &[(&str, &str)]) {
    let (t, area) = (*ui.theme(), ui.area());
    let keys: Vec<Hint> = keys
        .iter()
        .map(|&(key, label)| Hint { key, label })
        .collect();
    let m = Message {
        icon,
        tone: Some(tone),
        text,
        note,
        keys: &keys,
    };
    widgets::message(ui.canvas(), &t, area, &m);
}

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

/// The note, then the word to type, the typed part lit, a caret after it.
fn typing(c: &mut Canvas, t: &Theme, r: Rect, word: &str, typed: usize, note: &str) {
    let y = para(c, t, r.x, r.y, r.w, note, t.f.small, 15) + 10;
    let f = t.f.big;
    let mut x = r.x + (r.w - f.measure(word)) / 2;
    let base = y + f.cap();
    for (i, ch) in word.chars().enumerate() {
        let col = if i < typed { t.c.accent } else { t.c.border };
        x = f.draw(c, x, base, &ch.to_string(), col);
        if i + 1 == typed {
            c.fill(Rect::new(x + 1, y - 2, 2, f.cap() + 6), t.c.accent);
        }
    }
}

/// The joke, then the board with the dot and the ring, and the keys.
fn moving(c: &mut Canvas, t: &Theme, r: Rect, dot: (i32, i32)) {
    let top = para(c, t, r.x, r.y, r.w, SOBRIETY, t.f.small, 15) + 4;
    let cell = 20;
    let board = Rect::new(r.x + 4, top, cell * BOARD.0, cell * BOARD.1);
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
        let kr = Rect::new(right - 9, top + i as i32 * 17, r.right() - right + 9, 17);
        widgets::keys_line(c, t, kr, keys, 22, what, "");
    }
}

/// Callouts up to the top bar's two ends, then the joke.
fn bars(c: &mut Canvas, t: &Theme, r: Rect) {
    let ink = t.c.accent;
    c.fill(Rect::new(r.x + 10, t.m.bar_h, 1, 7), ink);
    let lr = Rect::new(r.x + 3, r.y + 6, 120, 14);
    widgets::text_in(c, t.f.small, lr, "where you are", ink);
    c.fill(Rect::new(r.right() - 30, t.m.bar_h, 1, 7), ink);
    let rr = Rect::new(r.right() - 160, r.y + 6, 157, 14);
    widgets::text_right(c, t.f.small, rr.right(), rr, "clock, battery, network", ink);
    para(c, t, r.x, r.y + 28, r.w, DONT, t.f.body, 18);
}

/// Three rows whose right sides differ: a switch, a choice, a page.
fn rows(c: &mut Canvas, t: &Theme, r: Rect, sel: usize, hat: bool, paranoia: usize) {
    let y = para(c, t, r.x, r.y, r.w, HAT, t.f.small, 15) + 2;
    let a = t.c.accent;
    let items = [
        Row {
            icon: Some("shield"),
            tint: a,
            label: "Tinfoil hat",
            trailing: Trailing::Toggle(hat),
        },
        Row {
            icon: Some("eye"),
            tint: a,
            label: "Paranoia",
            trailing: Trailing::Choice(PARANOIA[paranoia]),
        },
        Row {
            icon: Some("lock"),
            tint: a,
            label: "Secrets",
            trailing: Trailing::Chevron,
        },
    ];
    for (i, it) in items.iter().enumerate() {
        let rr = Rect::new(r.x - 2, y + i as i32 * t.m.row_h, r.w + 4, t.m.row_h);
        widgets::list_row(c, t, rr, it, i == sel);
    }
}
