//! Settings, a screen of the shell. Pages are built from a snapshot of the
//! system (`pages::State`) each time it changes, and drawn with the shared
//! rows, so this file holds only the model and its drawing.

pub mod pages;

use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Hint, Row, Status, Trailing};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageId {
    Root,
    Wifi,
    Network,
    Bluetooth,
    Display,
    Sound,
    Appearance,
    Apps,
    System,
    /// Regions, then the cities of one region.
    TimeZone(Option<String>),
    Privacy,
    About,
}

/// A value a row changes in place: Enter flips a toggle, Z and C step the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    Brightness,
    DarkTime,
    Volume,
    Mute,
    Layout,
    Palette,
    Animations,
    Clock,
    /// Shown on home, by `.desktop` file name.
    AppShown(String),
    WifiRadio,
    Bluetooth,
    NetworkTime,
    Ssh,
    Logs,
}

/// Something Enter starts that is not a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Join(String),
    JoinHidden,
    Device(String),
    BtScan,
    SetZone(String),
    Password,
    Restart,
    PowerOff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Open(PageId),
    Set(Setting),
    Run(Command),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Chevron,
    Text(String),
    TextChevron(String),
    Toggle(bool),
    Choice(String),
    Level(u8),
    Signal { level: u8, secure: bool },
}

pub struct Item {
    /// `Some("")` keeps the icon column empty, so marked and unmarked rows
    /// line up.
    pub icon: Option<&'static str>,
    pub label: String,
    pub value: Value,
    pub act: Act,
    /// Drawn in the critical colour: power off.
    pub danger: bool,
}

impl Item {
    pub fn new(icon: &'static str, label: impl Into<String>, value: Value, act: Act) -> Item {
        Item {
            icon: Some(icon),
            label: label.into(),
            value,
            act,
            danger: false,
        }
    }
}

pub enum Line {
    Section(String),
    Readout(String, String),
}

pub enum Body {
    List(Vec<Item>),
    /// Readouts with no cursor: Up and Down scroll.
    Info(Vec<Line>),
}

pub struct Page {
    pub id: PageId,
    pub title: String,
    pub body: Body,
    /// Extra keys this page takes, shown before Esc.
    pub keys: Vec<(&'static str, &'static str)>,
}

impl Page {
    pub fn len(&self) -> usize {
        match &self.body {
            Body::List(items) => items.len(),
            Body::Info(lines) => lines.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn item(&self, i: usize) -> Option<&Item> {
        match &self.body {
            Body::List(items) => items.get(i),
            Body::Info(_) => None,
        }
    }
}

/// Where the cursor sits on one page of the stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub selected: usize,
    pub top: usize,
}

impl Cursor {
    /// Moves by `d` rows over `len`, keeping `per` rows on screen. An info
    /// page has no cursor, so it scrolls instead.
    pub fn step(&mut self, d: i32, len: usize, per: usize, info: bool) {
        if len == 0 {
            return;
        }
        if info {
            let max = len.saturating_sub(per) as i64;
            self.top = (self.top as i64 + d as i64).clamp(0, max) as usize;
            return;
        }
        self.selected = (self.selected as i64 + d as i64).clamp(0, len as i64 - 1) as usize;
        self.fit(len, per);
    }

    pub fn fit(&mut self, len: usize, per: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + per {
            self.top = self.selected + 1 - per;
        }
        self.top = self.top.min(len.saturating_sub(per));
    }
}

/// A box over the page that takes every key until it closes.
pub enum Dialog {
    /// One line of text: a password, a network name.
    Text {
        title: String,
        placeholder: &'static str,
        text: String,
        masked: bool,
    },
    Confirm {
        title: String,
        line: String,
        yes: &'static str,
    },
}

/// Rows of a list page that fit the body.
pub fn rows(t: &Theme) -> (Rect, usize) {
    widgets::rows_area(t, t.m.row_h)
}

/// Lines of an info page that fit the body.
pub fn lines(t: &Theme) -> (Rect, usize) {
    widgets::rows_area(t, t.m.line_h)
}

pub fn draw(
    c: &mut Canvas,
    t: &Theme,
    status: &Status,
    page: &Page,
    cur: Cursor,
    dialog: Option<&Dialog>,
) {
    c.fill(widgets::body(t), t.c.bg);
    widgets::status_bar(c, t, status);
    match &page.body {
        Body::List(items) => {
            let (area, n) = rows(t);
            let w = area.w - 4;
            for (slot, i) in (cur.top..items.len()).take(n).enumerate() {
                let it = &items[i];
                let r = Rect::new(area.x, area.y + slot as i32 * t.m.row_h, w, t.m.row_h);
                widgets::list_row(c, t, r, &row(t, it), i == cur.selected);
            }
            let track = Rect::new(area.x, area.y, w, area.h);
            widgets::scrollbar(c, t, track, cur.top, n, items.len());
            if items.is_empty() {
                widgets::text_centre(c, t.f.body, widgets::content(t), "Nothing here", t.c.muted);
            }
        }
        Body::Info(lines_) => {
            let (area, n) = lines(t);
            let w = area.w - 4;
            for (slot, line) in lines_.iter().skip(cur.top).take(n).enumerate() {
                let r = Rect::new(area.x, area.y + slot as i32 * t.m.line_h, w, t.m.line_h);
                match line {
                    Line::Section(s) => widgets::section(c, t, r, s),
                    Line::Readout(k, v) => widgets::readout(c, t, r, k, v),
                }
            }
            let track = Rect::new(area.x, area.y, w, area.h);
            widgets::scrollbar(c, t, track, cur.top, n, lines_.len());
        }
    }
    let item = page.item(cur.selected);
    widgets::hint_bar(c, t, &hints(page, item), None);
    if let Some(d) = dialog {
        draw_dialog(c, t, d);
    }
}

fn row<'a>(t: &Theme, it: &'a Item) -> Row<'a> {
    let trailing = match &it.value {
        Value::None => Trailing::None,
        Value::Chevron => Trailing::Chevron,
        Value::Text(s) => Trailing::Value(s),
        Value::TextChevron(s) => Trailing::ValueChevron(s),
        Value::Toggle(on) => Trailing::Toggle(*on),
        Value::Choice(s) => Trailing::Choice(s),
        Value::Level(p) => Trailing::Level(*p),
        Value::Signal { level, secure } => Trailing::Signal {
            level: *level,
            secure: *secure,
        },
    };
    Row {
        icon: it.icon,
        tint: if it.danger { t.c.critical } else { t.c.accent },
        label: &it.label,
        trailing,
    }
}

/// What the keys do on this row, then the page's own keys, then Esc.
fn hints<'a>(page: &'a Page, item: Option<&Item>) -> Vec<Hint<'a>> {
    let mut out = Vec::new();
    let what = item.map(|i| (&i.act, &i.value));
    match what {
        Some((Act::Open(_), _)) => out.push(("Enter", "open")),
        Some((Act::Set(_), Value::Toggle(_))) => out.push(("Enter", "toggle")),
        Some((Act::Set(_), _)) => out.push(("Z C", "change")),
        Some((Act::Run(cmd), _)) => out.push((
            "Enter",
            match cmd {
                Command::Join(_) | Command::JoinHidden => "join",
                Command::Device(_) => "connect",
                Command::BtScan => "scan",
                Command::SetZone(_) => "set",
                _ => "run",
            },
        )),
        _ => {}
    }
    out.extend(page.keys.iter().copied());
    out.push(("Esc", "back"));
    out.into_iter()
        .map(|(key, label)| Hint { key, label })
        .collect()
}

fn draw_dialog(c: &mut Canvas, t: &Theme, d: &Dialog) {
    let r = Rect::new(28, 36, t.m.width - 56, 88);
    match d {
        Dialog::Text {
            title,
            placeholder,
            text,
            masked,
        } => {
            let inner = widgets::dialog(c, t, r, title, t.c.accent);
            let field = Rect::new(inner.x, inner.y + 2, inner.w, 24);
            widgets::text_field(c, t, field, text, *masked, true, placeholder);
            let keys: &[(&str, &str)] = if *masked {
                &[("Tab", "show"), ("Enter", "ok"), ("Esc", "cancel")]
            } else {
                &[("Enter", "ok"), ("Esc", "cancel")]
            };
            dialog_keys(c, t, Rect::new(inner.x, field.bottom() + 8, inner.w, 16), keys);
        }
        Dialog::Confirm { title, line, yes } => {
            let inner = widgets::dialog(c, t, r, title, t.c.critical);
            let l = Rect::new(inner.x, inner.y + 2, inner.w, 22);
            widgets::text_in(c, t.f.body, l, line, t.c.fg);
            let keys = [("Enter", *yes), ("Esc", "cancel")];
            dialog_keys(c, t, Rect::new(inner.x, l.bottom() + 10, inner.w, 16), &keys);
        }
    }
}

fn dialog_keys(c: &mut Canvas, t: &Theme, line: Rect, keys: &[(&str, &str)]) {
    let mut x = line.x;
    for (key, label) in keys {
        x = widgets::key_pill(c, t, x, line, key) + 4;
        let r = Rect::new(x, line.y + 1, line.right() - x, line.h - 1);
        x = widgets::text_in(c, t.f.small, r, label, t.c.muted) + 12;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_keeps_its_row_on_screen() {
        let mut c = Cursor::default();
        c.step(6, 10, 5, false);
        assert_eq!((c.selected, c.top), (6, 2));
        c.step(-9, 10, 5, false);
        assert_eq!((c.selected, c.top), (0, 0));
    }

    #[test]
    fn an_info_page_scrolls_without_a_cursor() {
        let mut c = Cursor::default();
        c.step(3, 12, 8, true);
        assert_eq!(c.top, 3);
        c.step(9, 12, 8, true);
        assert_eq!(c.top, 4, "never past the last screen");
    }
}
