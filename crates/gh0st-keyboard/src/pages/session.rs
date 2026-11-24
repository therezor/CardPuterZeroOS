//! Typing on a host, in one of five modes: Type, Mouse, Media, Slides and
//! Snippets. Fn+Space opens a picker of the five from any of them. Type
//! and Mouse take every key raw; Esc still goes back, and Fn+Esc sends
//! Esc to the host, as in the terminal.

use gh0st_app::gfx::Rect;
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::widgets::{self, Hint, Pill};
use gh0st_app::prelude::*;
use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::code;

use crate::app::{Nav, SHOW_TYPED, Screen, Shared, Target};
use crate::engine::{Echo, Named};
use crate::link::{Advert, Event, UsbState};
use crate::pages::{media, mouse::Mouse, slides::Slides, snippets::Snippets};
use crate::shortcuts;
use crate::words;

pub const TYPE: usize = 0;
pub const MOUSE: usize = 1;
pub const MEDIA: usize = 2;
pub const SLIDES: usize = 3;
pub const SNIPPETS: usize = 4;

const PICK: u32 = 100;
const MODE: u32 = 101; // to MODE + 4
const SHOW: u32 = 110;
const UNLINK: u32 = 111;

/// The config key for the mode used last.
const LAST_MODE: &str = "keyboard.mode";

/// How many sends the echo line keeps.
const ECHO: usize = 48;

pub struct Session {
    title: String,
    mode: usize,
    /// The mode picker, open, and the tile on it.
    picker: Option<usize>,
    echo: Vec<Echo>,
    mouse: Mouse,
    slides: Slides,
    snippets: Snippets,
    /// Opened in a given mode: the last one used doesn't replace it.
    fixed: bool,
}

impl Session {
    pub fn new(s: &Shared) -> Session {
        Session {
            title: s.target_name(),
            mode: TYPE,
            picker: None,
            echo: Vec::new(),
            mouse: Mouse::default(),
            slides: Slides::new(),
            snippets: Snippets::new(),
            fixed: false,
        }
    }

    /// Opens in `mode`: for mockups and tests.
    pub fn in_mode(s: &Shared, mode: usize) -> Session {
        Session {
            mode,
            fixed: true,
            ..Session::new(s)
        }
    }

    fn raw_mode(&self) -> bool {
        matches!(self.mode, TYPE | MOUSE)
    }

    /// Leaves the current mode cleanly: keys up, timers stopped.
    fn leave_mode(&mut self, s: &mut Shared, cx: &mut Cx) {
        s.release();
        self.mouse.stop(cx);
        self.slides.pause(cx);
        self.snippets.stop(s, cx);
    }

    fn set_mode(&mut self, s: &mut Shared, mode: usize, cx: &mut Cx) {
        if mode != self.mode {
            self.leave_mode(s, cx);
            self.mode = mode;
            cx.settings().set_str(LAST_MODE, words::MODES[mode]);
        }
        self.picker = None;
    }

    fn picker_key(&mut self, s: &mut Shared, k: PickKey, cx: &mut Cx) {
        let Some(at) = self.picker else { return };
        match k {
            PickKey::Digit(d) if (1..=5).contains(&d) => self.set_mode(s, d - 1, cx),
            PickKey::Up => self.picker = Some(gh0st_app::cycle(at, -1, 5)),
            PickKey::Down => self.picker = Some(gh0st_app::cycle(at, 1, 5)),
            PickKey::Enter => self.set_mode(s, at, cx),
            _ => {}
        }
        cx.redraw();
    }

    /// Why the host can't take keys now, as a message, or None when it can.
    fn not_live(&self, s: &Shared) -> Option<(&'static str, String, &'static str)> {
        if s.live() {
            return None;
        }
        let name = &self.title;
        Some(match &s.target {
            Some(Target::Usb) => match s.usb.state() {
                UsbState::HostMode => (
                    "usb",
                    words::usb_note(UsbState::HostMode).into(),
                    words::SLAVE_NOTE,
                ),
                UsbState::Asleep => ("usb", words::usb_note(UsbState::Asleep).into(), ""),
                st => ("usb", words::usb_note(st).into(), words::CABLE_NOTE),
            },
            _ => {
                let bt = s.bt.as_ref();
                if bt.is_some_and(|b| b.wanted() == Advert::Reconnect) {
                    ("bluetooth", words::waiting_for(name), words::WAKE_IT)
                } else {
                    ("bluetooth", words::waiting_for(name), words::NOT_BACK)
                }
            }
        })
    }

    fn draw_type(&self, s: &Shared, ui: &mut Ui) {
        let a = ui.area();
        let t = *ui.theme();
        let system = s.system();
        let c = ui.canvas();
        // Where the keys go, small, on top.
        let link = if s.target == Some(Target::Usb) {
            "usb"
        } else {
            "bluetooth"
        };
        let top = Rect::new(
            a.x + t.m.pad,
            a.y + t.m.margin_v,
            a.w - 2 * t.m.pad,
            t.m.line_h,
        );
        let iw = icon::draw(c, link, IconSize::Small, top.x, top.y + 2, t.c.accent);
        let r = Rect::new(top.x + iw + t.m.gap, top.y, top.w - iw, top.h);
        widgets::text_in(c, t.f.small, r, &words::typing_on(&self.title), t.c.muted);
        // What was sent, newest on the right, in a well.
        let well = Rect::new(
            a.x + t.m.pad,
            top.bottom() + t.m.gap,
            a.w - 2 * t.m.pad,
            t.m.app_row_h + t.m.pad,
        );
        c.round_rect(well, t.m.radius_large, t.c.surface);
        let inner = Rect::new(well.x + t.m.pad, well.y, well.w - 2 * t.m.pad, well.h);
        draw_echo(c, &t, inner, &self.echo, system, s.show_typed);
        // The latch, while it holds the host's key for the next one.
        let y = well.bottom() + t.m.gap;
        if s.engine.latched() {
            let end = widgets::pills(c, &t, well.x, y, system.latch_name(), Pill::Main);
            let r = Rect::new(end + t.m.gap * 2, y, well.right() - end, t.m.pill_h);
            widgets::text_in(c, t.f.small, r, words::LATCHED, t.c.muted);
        }
        let hints = [
            Hint {
                key: "Fn\u{a0}Space",
                label: "modes",
            },
            Hint {
                key: "Fn\u{a0}Esc",
                label: "Esc",
            },
            Hint {
                key: "Fn\u{a0}h",
                label: "keys",
            },
        ];
        let w = widgets::key_row_w(&t, &hints);
        widgets::key_row(
            c,
            &t,
            a.x + (a.w - w) / 2,
            a.bottom() - t.m.pill_h - t.m.margin_v,
            &hints,
        );
    }

    /// The modes as a list, the one in use selected, each with its digit.
    fn draw_picker(&self, ui: &mut Ui, at: usize) {
        let digits = ["1", "2", "3", "4", "5"];
        let rows: Vec<Row> = (0..words::MODES.len())
            .map(|i| Row::tag(words::MODES[i], digits[i], Tone::Info).icon(words::MODE_ICONS[i]))
            .collect();
        let mut list = List::new();
        list.select(at);
        ui.list(&list, &rows);
    }
}

enum PickKey {
    Digit(usize),
    Up,
    Down,
    Enter,
    Other,
}

/// The echo line: characters as text, combinations and named keys as
/// pills, laid out from the newest back until the well is full.
fn draw_echo(
    c: &mut gh0st_app::gfx::Canvas,
    t: &gh0st_app::gfx::theme::Theme,
    r: Rect,
    echo: &[Echo],
    system: crate::host::System,
    show: bool,
) {
    enum Tok {
        Text(String),
        Pill(String),
    }
    let mut toks: Vec<Tok> = Vec::new();
    for e in echo {
        match *e {
            Echo::Char(ch) => {
                let ch = if show { ch } else { '\u{2022}' };
                match toks.last_mut() {
                    Some(Tok::Text(s)) => s.push(ch),
                    _ => toks.push(Tok::Text(ch.to_string())),
                }
            }
            Echo::Combo { mods, key } => {
                let mut words = shortcuts::mod_words(system, mods);
                let k = match key {
                    Named::Char(ch) if show => ch.to_ascii_uppercase().to_string(),
                    Named::Char(_) => "\u{2022}".into(),
                    Named::Key(k) => k.to_string(),
                    Named::None => String::new(),
                };
                if !k.is_empty() {
                    words.push(k);
                }
                // A pill per combination: its words joined so they stay one.
                toks.push(Tok::Pill(words.join("\u{a0}")));
            }
            Echo::Media(_) => toks.push(Tok::Pill("Media".into())),
        }
    }
    let width = |tok: &Tok| match tok {
        Tok::Text(s) => t.f.body.measure(s),
        Tok::Pill(p) => widgets::pills_w(t, p) + t.m.gap,
    };
    // From the newest back, as many as fit; a long text keeps its end.
    let mut room = r.w;
    let mut first = toks.len();
    let mut cut = String::new();
    while first > 0 {
        let w = width(&toks[first - 1]);
        if w <= room {
            room -= w;
            first -= 1;
            continue;
        }
        if let Tok::Text(s) = &toks[first - 1] {
            cut = widgets::tail_fit(t.f.body, s, room).to_string();
        }
        break;
    }
    let mid = r.y + r.h / 2;
    let mut x = r.x;
    let text_y = |f: gh0st_app::gfx::font::Font| mid + f.cap() / 2;
    if !cut.is_empty() {
        x = t.f.body.draw(c, x, text_y(t.f.body), &cut, t.c.fg);
    }
    for tok in &toks[first..] {
        match tok {
            Tok::Text(s) => x = t.f.body.draw(c, x, text_y(t.f.body), s, t.c.fg),
            Tok::Pill(p) => {
                let end =
                    widgets::pills(c, t, x + t.m.gap / 2, mid - t.m.pill_h / 2, p, Pill::Normal);
                x = end + t.m.gap / 2;
            }
        }
    }
    // The caret.
    c.fill(
        Rect::new(x + 1, mid - t.m.line_h / 2, 2, t.m.line_h),
        t.c.accent,
    );
}

impl Screen for Session {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> &str {
        words::MODE_ICONS[self.mode]
    }

    fn context(&self) -> Option<&str> {
        Some(words::MODES[self.mode])
    }

    fn about(&self) -> &str {
        match self.mode {
            TYPE => words::TYPE_ABOUT,
            MOUSE => words::MOUSE_ABOUT,
            _ => "",
        }
    }

    fn bars(&self, s: &Shared) -> Bars {
        if self.picker.is_some() {
            return if self.raw_mode() {
                Bars::top_only()
            } else {
                Bars::top().hints([("1-5", "pick"), ("Enter", "open"), ("Esc", "close")])
            };
        }
        match self.mode {
            TYPE | MOUSE => Bars::top_only(),
            SLIDES => self.slides.chips(Slides::bars()),
            SNIPPETS => self.snippets.bars(s),
            _ => Bars::top(),
        }
    }

    fn raw_keys(&self) -> bool {
        self.raw_mode()
    }

    fn typing(&self) -> bool {
        self.picker.is_some() && !self.raw_mode()
    }

    fn start(&mut self, s: &mut Shared, cx: &mut Cx) {
        let last = cx.settings().config().get_str(LAST_MODE).map(String::from);
        if let Some(i) = last.and_then(|m| words::MODES.iter().position(|x| *x == m))
            && !self.fixed
        {
            self.mode = i;
        }
        s.show_typed = cx.settings().config().get_bool(SHOW_TYPED).unwrap_or(true);
    }

    fn stop(&mut self, s: &mut Shared, cx: &mut Cx) {
        self.leave_mode(s, cx);
    }

    fn covered(&mut self, s: &mut Shared, cx: &mut Cx) {
        s.release();
        self.mouse.stop(cx);
    }

    fn back(&mut self, s: &mut Shared, cx: &mut Cx) -> bool {
        if self.picker.take().is_some() {
            return false;
        }
        if self.snippets.sending() {
            self.snippets.stop(s, cx);
            return false;
        }
        true
    }

    fn draw(&self, s: &Shared, ui: &mut Ui) {
        if let Some(at) = self.picker {
            return self.draw_picker(ui, at);
        }
        if let Some((icon_name, line, note)) = self.not_live(s).filter(|_| self.mode != SNIPPETS) {
            let keys: &[(&str, &str)] = if note == words::NOT_BACK {
                &[("Enter", "try again")]
            } else {
                &[]
            };
            ui.notice(icon_name, None, &line, note, keys);
        } else {
            match self.mode {
                TYPE => self.draw_type(s, ui),
                MOUSE => self.mouse.draw(ui),
                MEDIA => media::draw(ui),
                SLIDES => self.slides.draw(ui),
                _ => self.snippets.draw(s, ui),
            }
        }
    }

    fn raw(&mut self, s: &mut Shared, k: &RawKey, cx: &mut Cx) -> Nav {
        if self.picker.is_some() {
            if k.state == KeyState::Press {
                let pk = match k.code {
                    2..=6 => PickKey::Digit((k.code - 1) as usize),
                    code::UP | code::F => PickKey::Up,
                    code::DOWN | code::X => PickKey::Down,
                    code::ENTER => PickKey::Enter,
                    _ => PickKey::Other,
                };
                self.picker_key(s, pk, cx);
            }
            return Nav::Stay;
        }
        if !s.live() {
            // Nothing to type on: Enter asks the host back.
            if k.state == KeyState::Press && k.code == code::ENTER {
                retry(s);
                cx.redraw();
            }
            return Nav::Stay;
        }
        if self.mode == MOUSE {
            self.mouse.raw(s, k, cx);
            return Nav::Stay;
        }
        let was = s.engine.latched();
        let mut out = Vec::new();
        let echo = s.engine.key(k, &mut out);
        s.send(&out);
        if let Some(e) = echo {
            self.echo.push(e);
            if self.echo.len() > ECHO {
                self.echo.drain(..self.echo.len() - ECHO);
            }
            cx.redraw();
        } else if was != s.engine.latched() {
            cx.redraw();
        }
        Nav::Stay
    }

    fn key(&mut self, s: &mut Shared, key: Key, cx: &mut Cx) -> Nav {
        if self.picker.is_some() {
            let pk = match key {
                Key::Char(c) if c.is_ascii_digit() => PickKey::Digit(c as usize - '0' as usize),
                Key::Up => PickKey::Up,
                Key::Down => PickKey::Down,
                Key::Enter => PickKey::Enter,
                _ => PickKey::Other,
            };
            self.picker_key(s, pk, cx);
            return Nav::Stay;
        }
        if self.mode == SNIPPETS {
            self.snippets.key(s, key, cx);
        } else if !s.live() && key == Key::Enter {
            retry(s);
        }
        Nav::Stay
    }

    fn commands(&self, s: &Shared, c: &mut Commands) {
        let mut pick = Command::action(PICK, words::MODES_LABEL);
        pick = if self.raw_mode() {
            pick.keys("Fn+Space")
        } else {
            pick.keys("Fn+Space Tab")
        };
        c.add(pick);
        if self.picker.is_some() {
            return;
        }
        match self.mode {
            MEDIA => media::commands(c),
            SLIDES => Slides::commands(c),
            SNIPPETS => Snippets::commands(c),
            _ => {}
        }
        c.group("Mode");
        for (i, m) in words::MODES.iter().enumerate() {
            c.add(Command::action(MODE + i as u32, *m).enabled(i != self.mode));
        }
        c.group("Host");
        if self.mode == TYPE {
            c.add(Command::toggle(SHOW, words::SHOW_TYPED, s.show_typed));
        }
        let bt = matches!(s.target, Some(Target::Bt(_)));
        c.add(Command::action(UNLINK, words::UNLINK).enabled(bt && s.live()));
    }

    fn command(&mut self, s: &mut Shared, id: u32, _: Step, cx: &mut Cx) -> Nav {
        match id {
            PICK => {
                s.release();
                self.mouse.stop(cx);
                self.picker = match self.picker {
                    Some(_) => None,
                    None => Some(self.mode),
                };
            }
            id if (MODE..MODE + 5).contains(&id) => self.set_mode(s, (id - MODE) as usize, cx),
            SHOW => {
                s.show_typed = !s.show_typed;
                cx.settings().set_bool(SHOW_TYPED, s.show_typed);
            }
            UNLINK => {
                if let Some(Target::Bt(a)) = s.target.clone() {
                    s.release();
                    let path = s.path_for(&a);
                    if let Some(bt) = &mut s.bt {
                        bt.reconnect(false);
                        bt.disconnect(&path);
                    }
                    return Nav::Pop;
                }
            }
            _ => {
                let _ = media::command(s, id, cx)
                    || self.slides.command(s, id, cx)
                    || self.snippets.command(s, id, cx);
            }
        }
        cx.redraw();
        Nav::Stay
    }

    fn answer(&mut self, s: &mut Shared, id: u32, a: Answer, cx: &mut Cx) -> Nav {
        self.snippets.answer(s, id, a, cx);
        Nav::Stay
    }

    fn timer(&mut self, s: &mut Shared, t: Timer, cx: &mut Cx) -> Nav {
        if !self.mouse.timer(s, t) && !self.snippets.timer(s, t, cx) && !self.slides.timer(t) {
            return Nav::Stay;
        }
        cx.redraw();
        Nav::Stay
    }

    fn event(&mut self, s: &mut Shared, e: &Event, cx: &mut Cx) -> Nav {
        if let Event::Disconnected(_) | Event::Listening(false) = e {
            self.mouse.stop(cx);
            self.snippets.stop(s, cx);
        }
        Nav::Stay
    }

    fn frame_ms(&self) -> u64 {
        if self.mouse.moving() {
            crate::pages::mouse::TICK_MS
        } else {
            0
        }
    }
}

/// Asks the target back: the advert for a Bluetooth host, the USB
/// keyboard on for the cable.
fn retry(s: &mut Shared) {
    match &s.target {
        Some(Target::Bt(_)) => {
            if let Some(bt) = &mut s.bt {
                bt.reconnect(true);
            }
        }
        Some(Target::Usb) => {
            if matches!(s.usb.state(), UsbState::Off | UsbState::Failed) {
                s.usb.enable(true);
            }
        }
        None => {}
    }
}
