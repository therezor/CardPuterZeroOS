//! Drawing a page. Each call stacks below the last, inside the area the bars
//! leave, with the OS margins, fonts and colours. Pixel work goes through
//! [`Ui::canvas`].

use crate::field::TextField;
use crate::list::{List, Row, Trail};
use gh0st_ui::font::Font;
use gh0st_ui::icon::{self, IconSize};
use gh0st_ui::theme::Tone;
use gh0st_ui::widgets::{self, Trailing};
use gh0st_ui::{Canvas, Color, Rect, Theme};

pub struct Ui<'a> {
    c: &'a mut Canvas,
    t: &'a Theme,
    area: Rect,
    y: i32,
}

impl<'a> Ui<'a> {
    pub(crate) fn new(c: &'a mut Canvas, t: &'a Theme, area: Rect) -> Ui<'a> {
        c.set_clip(area);
        c.fill(area, t.c.bg);
        Ui {
            c,
            t,
            area,
            y: area.y + t.m.margin_v,
        }
    }

    /// The page's whole area: the screen minus the bars it asked for.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// What is left below the last thing drawn.
    pub fn rest(&self) -> Rect {
        Rect::new(
            self.area.x,
            self.y,
            self.area.w,
            (self.area.bottom() - self.y).max(0),
        )
    }

    pub fn theme(&self) -> &Theme {
        self.t
    }

    /// Raw pixels, clipped to the page's area. Coordinates are the screen's.
    pub fn canvas(&mut self) -> &mut Canvas {
        self.c
    }

    fn inner(&self) -> (i32, i32) {
        let x = self.area.x + self.t.m.margin + 3;
        (x, self.area.right() - self.t.m.margin - 3 - x)
    }

    fn line(&mut self, f: Font, h: i32, s: &str, color: Color) {
        let (x, w) = self.inner();
        widgets::text_in(self.c, f, Rect::new(x, self.y, w, h), s, color);
        self.y += h;
    }

    /// A page heading.
    pub fn heading(&mut self, s: &str) {
        let t = self.t;
        self.line(t.f.title, 24, s, t.c.fg);
    }

    /// Body text, wrapped to the width.
    pub fn text(&mut self, s: &str) {
        let t = self.t;
        self.wrapped(t.f.body, 18, s, t.c.fg);
    }

    /// Secondary text, smaller and softer, wrapped.
    pub fn note(&mut self, s: &str) {
        let t = self.t;
        self.wrapped(t.f.small, 16, s, t.c.muted);
    }

    fn wrapped(&mut self, f: Font, h: i32, s: &str, color: Color) {
        let (_, w) = self.inner();
        for line in wrap(f, s, w) {
            self.line(f, h, &line, color);
        }
    }

    /// A small accent label over a group.
    pub fn section(&mut self, s: &str) {
        let (x, w) = self.inner();
        widgets::section(self.c, self.t, Rect::new(x - 9, self.y, w + 9, 16), s);
        self.y += 16;
    }

    /// A label and a value in two columns.
    pub fn readout(&mut self, label: &str, value: &str) {
        let (x, w) = self.inner();
        widgets::readout(
            self.c,
            self.t,
            Rect::new(x - 9, self.y, w + 9, 14),
            label,
            value,
        );
        self.y += 14;
    }

    /// Empty space, in pixels.
    pub fn space(&mut self, px: i32) {
        self.y += px;
    }

    /// Large text, centred in what is left. For a number or a short word.
    pub fn big(&mut self, s: &str) {
        let r = self.rest();
        let f = self.t.f.big;
        widgets::text_centre(self.c, f, r, s, self.t.c.fg);
        self.y = r.bottom();
    }

    /// An icon and a line of text, centred in what is left. For an empty or
    /// waiting state.
    pub fn message(&mut self, icon_name: &str, s: &str) {
        let r = self.rest();
        let size = IconSize::Large as i32;
        let top = r.y + (r.h - size - 26) / 2;
        icon::draw(
            self.c,
            icon_name,
            IconSize::Large,
            r.x + (r.w - size) / 2,
            top,
            self.t.c.muted,
        );
        let text = Rect::new(r.x, top + size + 6, r.w, 20);
        widgets::text_centre(self.c, self.t.f.body, text, s, self.t.c.muted);
        self.y = r.bottom();
    }

    /// A message that fills the rest: an icon in a tone (none: muted), a line,
    /// a note, and the keys that act on it: `&[("Enter", "scan")]`.
    pub fn notice(
        &mut self,
        icon_name: &str,
        tone: Option<Tone>,
        text: &str,
        note: &str,
        keys: &[(&str, &str)],
    ) {
        let r = self.rest();
        let keys: Vec<widgets::Hint> = keys
            .iter()
            .map(|&(key, label)| widgets::Hint { key, label })
            .collect();
        let m = widgets::Message {
            icon: icon_name,
            tone,
            text,
            note,
            keys: &keys,
        };
        widgets::message(self.c, self.t, r, &m);
        self.y = r.bottom();
    }

    /// A labelled bar: how far a job has come, 0 to 100. `None` while that
    /// isn't known: a segment at `phase` (0 to 255), moved only while shown.
    pub fn progress(&mut self, label: &str, value: Option<u8>, phase: u8) {
        let (x, w) = self.inner();
        let r = Rect::new(x, self.y, w, 23);
        widgets::progress(self.c, self.t, r, label, value, phase);
        self.y = r.bottom() + 6;
    }

    /// Rows with `list`'s selection, scrolled to keep it in view. A long list
    /// fills the rest of the page; a short one leaves room below it.
    pub fn list(&mut self, list: &List, rows: &[Row]) {
        let t = self.t;
        let row_h = t.m.row_h;
        // First on the page: use the whole area and centre the rows, as the
        // shell's lists do. Below other content: start under it.
        let alone = self.y == self.area.y + t.m.margin_v;
        let rest = if alone { self.area } else { self.rest() };
        let visible = (rest.h / row_h).max(1) as usize;
        let first = list.layout(rows.len(), visible);
        let x = rest.x + t.m.margin;
        let w = rest.w - 2 * t.m.margin - 4;
        let top = if alone {
            rest.y + (rest.h - visible as i32 * row_h) / 2
        } else {
            rest.y
        };
        for (i, row) in rows.iter().enumerate().skip(first).take(visible) {
            let r = Rect::new(x, top + (i - first) as i32 * row_h, w, row_h);
            let tint = if row.dim { t.c.muted } else { t.c.accent };
            if let Some(why) = row.off {
                widgets::disabled_row(self.c, t, r, row.icon, row.label, why);
                continue;
            }
            let trailing = match row.trail {
                Trail::None | Trail::Check(false) => Trailing::None,
                Trail::Link(None) => Trailing::Chevron,
                Trail::Link(Some(v)) => Trailing::ValueChevron(v),
                Trail::Info(v) => Trailing::Value(v),
                Trail::Toggle(on) => Trailing::Toggle(on),
                Trail::Choice(v) => Trailing::Choice(v),
                Trail::Level(p) => Trailing::Level(p),
                Trail::Check(true) => Trailing::Check,
                Trail::Tag(w, tone) => Trailing::Tag(w, tone),
                Trail::Signal(level, secure) => Trailing::Signal { level, secure },
            };
            let w_row = widgets::Row {
                icon: (!row.icon.is_empty()).then_some(row.icon),
                tint,
                label: row.label,
                trailing,
            };
            widgets::list_row(self.c, t, r, &w_row, i == list.selected());
        }
        let track = Rect::new(x, top, w, visible as i32 * row_h);
        widgets::scrollbar(self.c, t, track, first, visible, rows.len());
        self.y = top + visible.min(rows.len()) as i32 * row_h + t.m.margin_v;
    }

    /// A text field, with a caret when `focused`.
    pub fn field(&mut self, field: &TextField, placeholder: &str, focused: bool) {
        let (x, w) = self.inner();
        let r = Rect::new(x - 3, self.y, w + 6, 24);
        widgets::text_field(
            self.c,
            self.t,
            r,
            field.text(),
            field.is_masked(),
            focused,
            placeholder,
        );
        self.y += 28;
    }
}

/// Greedy word wrap. A word wider than the line is left to elide.
fn wrap(f: Font, s: &str, w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if f.measure(&candidate) <= w || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_on_spaces() {
        let t = Theme::default();
        let lines = wrap(t.f.body, "one two three four five six seven", 80);
        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|l| t.f.body.measure(l) <= 80 || !l.contains(' '))
        );
        assert_eq!(wrap(t.f.body, "a\nb", 300), ["a", "b"]);
    }
}
