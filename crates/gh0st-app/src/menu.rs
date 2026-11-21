//! The app menu and Help. Key 8 opens the menu in every app, Fn+H opens
//! Help. The SDK builds both from the commands of the page in front, and
//! adds Help and Close app to the menu. The rules are in docs/design.md.

use crate::command::{Command, Commands, Hit, Kind, Step};
use gh0st_core::keys::{Key as RawKey, Nav, State, code};
use gh0st_ui::widgets::{self, Trailing};
use gh0st_ui::{Canvas, Rect, Theme};
use std::cell::Cell;

/// Ids of the rows the SDK adds. Pages keep their ids below these.
pub(crate) const HELP: u32 = u32::MAX - 1;
pub(crate) const CLOSE_APP: u32 = u32::MAX;

/// What a key in the menu asks of the runtime.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Nothing,
    Close,
    /// Run a page command; `close` shuts the menu first.
    Run { id: u32, step: Step, close: bool },
    Help,
    Quit,
}

enum Line {
    Head(&'static str),
    /// A menu row, by its place in `rows`.
    Item(usize),
}

pub(crate) struct Menu {
    pub owner: usize,
    cmds: Commands,
    /// The menu's rows in display order, as indices into `cmds`.
    rows: Vec<usize>,
    lines: Vec<Line>,
    sel: usize,
    /// The selected row asks first, and Enter was pressed once.
    armed: bool,
    /// The last row changed in place.
    changed: Option<u32>,
    first: Cell<usize>,
}

impl Menu {
    pub fn new(owner: usize, cmds: Commands, select: Option<u32>) -> Menu {
        let mut m = Menu {
            owner,
            cmds: Commands::new(),
            rows: Vec::new(),
            lines: Vec::new(),
            sel: 0,
            armed: false,
            changed: None,
            first: Cell::new(0),
        };
        m.refresh(cmds);
        if let Some(id) = select {
            m.select_id(id);
        }
        m
    }

    /// New commands from the page, after a change: the selection stays on
    /// the same command.
    pub fn refresh(&mut self, mut cmds: Commands) {
        let keep = self.selected().map(|c| c.id);
        let grouped = cmds.iter().any(|c| c.in_menu() && !c.group.is_empty());
        cmds.group(if grouped { "App" } else { "" });
        cmds.add(Command::action(HELP, "Help").opens());
        cmds.add(Command::action(CLOSE_APP, "Close app").confirm());
        self.cmds = cmds;
        self.rows.clear();
        self.lines.clear();
        let mut groups: Vec<&'static str> = Vec::new();
        for c in self.cmds.iter().filter(|c| c.in_menu()) {
            if !groups.contains(&c.group) {
                groups.push(c.group);
            }
        }
        for g in groups {
            if !g.is_empty() {
                self.lines.push(Line::Head(g));
            }
            for (i, c) in self.cmds.list.iter().enumerate() {
                if c.in_menu() && c.group == g {
                    self.lines.push(Line::Item(self.rows.len()));
                    self.rows.push(i);
                }
            }
        }
        self.sel = self.sel.min(self.rows.len() - 1);
        if let Some(id) = keep {
            self.select_id(id);
        }
    }

    fn select_id(&mut self, id: u32) {
        if let Some(r) = self.rows.iter().position(|&i| self.cmds.list[i].id == id) {
            self.sel = r;
        }
    }

    fn selected(&self) -> Option<&Command> {
        self.rows.get(self.sel).map(|&i| &self.cmds.list[i])
    }

    pub fn selected_id(&self) -> Option<u32> {
        self.selected().map(|c| c.id)
    }

    /// The setting changed in place last, if the selection is still on it.
    pub fn changed(&self) -> Option<u32> {
        self.changed.filter(|&id| self.selected_id() == Some(id))
    }

    pub fn key(&mut self, k: &RawKey) -> Out {
        let out = self.key_inner(k);
        if let Out::Run { id, close: false, .. } = out {
            self.changed = Some(id);
        }
        out
    }

    fn key_inner(&mut self, k: &RawKey) -> Out {
        if k.state == State::Release {
            return Out::Nothing;
        }
        let nav = (!k.ctrl && !k.alt).then(|| k.nav(true)).flatten();
        if let Some(nav) = nav {
            let n = self.rows.len();
            match nav {
                Nav::Up => self.sel = (self.sel + n - 1) % n,
                Nav::Down => self.sel = (self.sel + 1) % n,
                Nav::Left => return self.step(Step::Prev),
                Nav::Right => return self.step(Step::Next),
            }
            self.armed = false;
            return Out::Nothing;
        }
        if k.state == State::Repeat {
            return Out::Nothing;
        }
        match k.code {
            code::ESC if self.armed => {
                self.armed = false;
                return Out::Nothing;
            }
            code::ESC => return Out::Close,
            code::ENTER => return self.pick(Step::Run),
            _ => {}
        }
        match self.cmds.accelerator(k) {
            Some(hit) => self.accelerate(hit),
            None => Out::Nothing,
        }
    }

    fn accelerate(&mut self, hit: Hit) -> Out {
        if let Some(r) = self.rows.iter().position(|&i| i == hit.index) {
            if r != self.sel {
                self.armed = false;
            }
            self.sel = r;
        }
        self.pick(hit.step)
    }

    /// Left or Right: steps a toggle or a choice in place.
    fn step(&mut self, step: Step) -> Out {
        match self.selected() {
            Some(c) if c.stays() && c.is_on() => Out::Run {
                id: c.id,
                step,
                close: false,
            },
            _ => Out::Nothing,
        }
    }

    /// Enter or a row's key.
    fn pick(&mut self, step: Step) -> Out {
        let Some(c) = self.selected() else {
            return Out::Nothing;
        };
        if !c.is_on() {
            return Out::Nothing;
        }
        let id = c.id;
        let close = !c.stays();
        match c.kind {
            Kind::Action => {
                if c.confirms() && !self.armed {
                    self.armed = true;
                    return Out::Nothing;
                }
                self.armed = false;
                match id {
                    HELP => Out::Help,
                    CLOSE_APP => Out::Quit,
                    _ => Out::Run {
                        id,
                        step: Step::Run,
                        close: true,
                    },
                }
            }
            Kind::Toggle(_) => Out::Run { id, step, close },
            Kind::Choice(_) => Out::Run {
                id,
                step: if step == Step::Run { Step::Next } else { step },
                close,
            },
        }
    }

    fn line_h(&self, t: &Theme, l: &Line) -> i32 {
        match l {
            Line::Head(_) => t.m.menu_head_h,
            Line::Item(_) => t.m.menu_row_h,
        }
    }

    /// The first line to draw, so the selection and its heading show.
    fn scroll(&self, t: &Theme, h: i32) -> usize {
        let at = self
            .lines
            .iter()
            .position(|l| matches!(l, Line::Item(r) if *r == self.sel))
            .unwrap_or(0);
        let mut first = self.first.get().min(at);
        if first == at && first > 0 && matches!(self.lines[first - 1], Line::Head(_)) {
            first -= 1;
        }
        let span = |a: usize| -> i32 { self.lines[a..=at].iter().map(|l| self.line_h(t, l)).sum() };
        while first < at && span(first) > h {
            first += 1;
        }
        // Fill the panel: no gap below the last line.
        loop {
            let below: i32 = self.lines[first..].iter().map(|l| self.line_h(t, l)).sum();
            if first == 0 || below + self.line_h(t, &self.lines[first - 1]) > h {
                break;
            }
            first -= 1;
        }
        self.first.set(first);
        first
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, panel: Rect) {
        c.set_clip(panel);
        c.fill(panel, t.c.bg);
        let x = panel.x + t.m.margin;
        let w = panel.w - 2 * t.m.margin - 4;
        let top = panel.y + 2;
        let h = panel.h - 4;
        let first = self.scroll(t, h);
        let keys: Vec<String> = self
            .rows
            .iter()
            .map(|&i| self.cmds.list[i].keys_text())
            .collect();
        let keys_w = keys
            .iter()
            .map(|k| widgets::pills_w(t, k))
            .max()
            .unwrap_or(0)
            .min(t.m.keys_col_max);
        let mut y = top;
        let mut shown = 0;
        for line in &self.lines[first..] {
            let lh = self.line_h(t, line);
            if y + lh > top + h {
                break;
            }
            let r = Rect::new(x, y, w, lh);
            match *line {
                Line::Head(g) => widgets::section(c, t, Rect::new(x, y + 2, w, lh - 2), g),
                Line::Item(n) => {
                    let cmd = &self.cmds.list[self.rows[n]];
                    let selected = n == self.sel;
                    let trailing = match &cmd.kind {
                        _ if selected && self.armed => Trailing::Alert("Press again"),
                        Kind::Action if cmd.opening() => Trailing::Chevron,
                        Kind::Action => Trailing::None,
                        Kind::Toggle(on) => Trailing::Toggle(*on),
                        Kind::Choice(v) => Trailing::Choice(v),
                    };
                    let row = widgets::Row {
                        icon: None,
                        tint: t.c.accent,
                        label: &cmd.label,
                        trailing,
                    };
                    let dim = !cmd.is_on();
                    widgets::menu_row(c, t, r, &row, &keys[n], keys_w, selected, dim);
                }
            }
            y += lh;
            shown += 1;
        }
        let track = Rect::new(x, top, w, h);
        widgets::scrollbar(c, t, track, first, shown, self.lines.len());
    }
}

/// A line of the Keys screen.
enum Entry {
    Head(&'static str),
    Key {
        keys: String,
        label: String,
        value: String,
    },
}

/// What a key in Help asks.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SheetOut {
    Nothing,
    Back,
    /// 8: the menu, so pages that take every key can still reach it.
    Menu,
}

/// Every key of the page in front, by the mode it works in, and the keys
/// that work everywhere.
pub(crate) struct Sheet {
    lines: Vec<Entry>,
    first: usize,
    visible: Cell<usize>,
    /// The menu it came from, to go back to.
    pub menu: Option<Menu>,
}

/// Keys the OS and the SDK give every app.
const EVERYWHERE: &[(&str, &str)] = &[
    ("Fn k", "Switch apps"),
    ("Fn j", "Screenshot"),
    ("Fn Esc", "The Esc key itself"),
    ("Fn h", "This help"),
];

impl Sheet {
    /// `under`: the chips on screen, by key, then the menu. `keys`: what Esc
    /// and Enter do here. Then the page's commands, then the keys that
    /// work everywhere.
    pub fn new<'a>(
        under: Vec<(String, String)>,
        keys: Vec<(String, String)>,
        cmds: impl Iterator<Item = &'a Command>,
        menu: Option<Menu>,
    ) -> Sheet {
        let cmds: Vec<&Command> = cmds.filter(|c| !c.keys.is_empty()).collect();
        let section = |c: &Command| if c.mode.is_empty() { c.group } else { c.mode };
        let mut order: Vec<&'static str> = Vec::new();
        for c in &cmds {
            if !order.contains(&section(c)) {
                order.push(section(c));
            }
        }
        let mut lines = Vec::new();
        for (head, list) in [("Under the screen", under), ("Keys", keys)] {
            if list.is_empty() {
                continue;
            }
            lines.push(Entry::Head(head));
            for (keys, label) in list {
                lines.push(Entry::Key {
                    keys,
                    label,
                    value: String::new(),
                });
            }
        }
        for s in order {
            if !s.is_empty() || !lines.is_empty() {
                lines.push(Entry::Head(if s.is_empty() { "Other" } else { s }));
            }
            for c in cmds.iter().filter(|c| section(c) == s) {
                let value = match &c.kind {
                    Kind::Action => String::new(),
                    Kind::Toggle(on) => if *on { "On" } else { "Off" }.to_string(),
                    Kind::Choice(v) => v.to_string(),
                };
                lines.push(Entry::Key {
                    keys: c.keys_text(),
                    label: c.label.to_string(),
                    value,
                });
            }
        }
        lines.push(Entry::Head("Everywhere"));
        for &(keys, label) in EVERYWHERE {
            lines.push(Entry::Key {
                keys: keys.into(),
                label: label.into(),
                value: String::new(),
            });
        }
        Sheet {
            lines,
            first: 0,
            visible: Cell::new(1),
            menu,
        }
    }

    pub fn key(&mut self, k: &RawKey) -> SheetOut {
        if k.state == State::Release {
            return SheetOut::Nothing;
        }
        let page = self.visible.get().saturating_sub(1).max(1);
        let last = self.lines.len().saturating_sub(self.visible.get());
        match (!k.ctrl && !k.alt).then(|| k.nav(true)).flatten() {
            Some(Nav::Up) => self.first = self.first.saturating_sub(1),
            Some(Nav::Down) => self.first = (self.first + 1).min(last),
            Some(Nav::Left) => self.first = self.first.saturating_sub(page),
            Some(Nav::Right) => self.first = (self.first + page).min(last),
            None if k.state == State::Press && matches!(k.code, code::ESC | code::ENTER) => {
                return SheetOut::Back;
            }
            None if k.state == State::Press && k.ch == Some('8') => return SheetOut::Menu,
            None => {}
        }
        SheetOut::Nothing
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, panel: Rect) {
        c.set_clip(panel);
        c.fill(panel, t.c.bg);
        let x = panel.x + t.m.margin;
        let w = panel.w - 2 * t.m.margin - 4;
        let top = panel.y + 3;
        let lh = t.m.keys_line_h;
        let visible = ((panel.h - 6) / lh).max(1) as usize;
        self.visible.set(visible);
        let first = self.first.min(self.lines.len().saturating_sub(visible));
        let keys_w = self
            .lines
            .iter()
            .filter_map(|l| match l {
                Entry::Key { keys, .. } => Some(widgets::pills_w(t, keys)),
                Entry::Head(_) => None,
            })
            .max()
            .unwrap_or(0)
            .min(t.m.keys_col_max);
        for (i, line) in self.lines.iter().skip(first).take(visible).enumerate() {
            let r = Rect::new(x, top + i as i32 * lh, w, lh);
            match line {
                Entry::Head(s) => widgets::section(c, t, r, s),
                Entry::Key { keys, label, value } => {
                    widgets::keys_line(c, t, r, keys, keys_w, label, value)
                }
            }
        }
        let track = Rect::new(x, top, w, visible as i32 * lh);
        widgets::scrollbar(c, t, track, first, visible, self.lines.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_core::keys::Keyboard;

    fn press(c: u16) -> RawKey {
        Keyboard::new(Vec::new()).event(c, 1, 0).unwrap()
    }

    fn menu() -> Menu {
        let mut cs = Commands::new();
        cs.group("View");
        cs.add(Command::action(1, "Scroll mode"));
        cs.add(Command::choice(2, "Text size", "8x14").keys("r t"));
        cs.add(Command::toggle(3, "Top bar", true).keys("b"));
        cs.add(Command::action(4, "Scroll up").keys("f").keys_only());
        Menu::new(0, cs, None)
    }

    #[test]
    fn rows_are_grouped_and_end_with_help_and_close() {
        let m = menu();
        let ids: Vec<u32> = m.rows.iter().map(|&i| m.cmds.list[i].id).collect();
        assert_eq!(ids, [1, 2, 3, HELP, CLOSE_APP]);
        assert!(matches!(m.lines[0], Line::Head("View")));
        assert!(matches!(m.lines[4], Line::Head("App")));
    }

    #[test]
    fn choices_step_in_place_and_actions_close() {
        let mut m = menu();
        assert_eq!(
            m.key(&press(19)),
            Out::Run {
                id: 2,
                step: Step::Prev,
                close: false
            }
        );
        assert_eq!(m.selected_id(), Some(2), "a row's key selects it");
        assert_eq!(
            m.key(&press(code::RIGHT)),
            Out::Run {
                id: 2,
                step: Step::Next,
                close: false
            }
        );
        m.key(&press(code::UP));
        assert_eq!(
            m.key(&press(code::ENTER)),
            Out::Run {
                id: 1,
                step: Step::Run,
                close: true
            }
        );
    }

    #[test]
    fn close_app_asks_first() {
        let mut m = menu();
        m.key(&press(code::UP));
        assert_eq!(m.key(&press(code::ENTER)), Out::Nothing);
        assert!(m.armed);
        assert_eq!(m.key(&press(code::ESC)), Out::Nothing, "Esc disarms");
        assert_eq!(m.key(&press(code::ESC)), Out::Close);
        m.key(&press(code::ENTER));
        assert_eq!(m.key(&press(code::ENTER)), Out::Quit);
    }

    #[test]
    fn the_help_row_opens_help() {
        let mut m = menu();
        m.key(&press(code::UP));
        m.key(&press(code::UP));
        assert_eq!(m.key(&press(code::ENTER)), Out::Help);
    }

    #[test]
    fn help_starts_with_the_chips_and_groups_by_mode() {
        let mut cs = Commands::new();
        cs.group("View");
        cs.add(Command::action(1, "Find").keys("n").mode("Scroll mode", false));
        cs.add(Command::action(2, "Back").keys("Shift+PgUp").mode("Typing", true));
        cs.add(Command::action(3, "No keys"));
        let under = vec![("5".to_string(), "Find".to_string())];
        let keys = vec![("Esc".to_string(), "Back".to_string())];
        let s = Sheet::new(under, keys, cs.iter(), None);
        let heads: Vec<&str> = s
            .lines
            .iter()
            .filter_map(|l| match l {
                Entry::Head(h) => Some(*h),
                _ => None,
            })
            .collect();
        assert_eq!(
            heads,
            ["Under the screen", "Keys", "Scroll mode", "Typing", "Everywhere"]
        );
    }

    #[test]
    fn the_widest_row_fits() {
        let t = Theme::default();
        let mut c = Canvas::new(t.m.width, t.m.height);
        let mut cs = Commands::new();
        cs.add(Command::choice(1, "Magnify grid", "132x43").keys("r t"));
        let m = Menu::new(0, cs, None);
        let panel = Rect::new(0, t.m.bar_h, t.m.width, t.m.height - t.m.bar_h - t.m.hint_h);
        m.draw(&mut c, &t, panel);
        let label = t.f.body_bold.measure("Magnify grid");
        let value = t.f.small.measure("132x43") + 2 * t.f.small_bold.measure("\u{203a}") + 12;
        let row = t.m.width - 2 * t.m.margin - 4 - 9 - 8;
        assert!(label + 8 + value + 6 + widgets::pills_w(&t, "r t") + 6 <= row);
    }
}
