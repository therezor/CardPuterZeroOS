//! Dialogs: a card over the page that takes every key until it closes.
//! `Cx::ask` opens one; `Page::answer` hears how it ended.

use gh0st_core::keys::{Key as RawKey, State, code};
use gh0st_ui::widgets::{self, Hint};
use gh0st_ui::{Canvas, Rect, Theme};

/// Longest text a dialog's field takes.
const MAX: usize = 64;

/// What the card asks. Enter says yes and Esc says no, named in key pills;
/// a dialog that loses data says yes only to its chip held on 7.
///
/// ```
/// use gh0st_app::Dialog;
///
/// let join = Dialog::password("Join \u{201c}Office-5G\u{201d}").yes("join");
/// let rename = Dialog::text("Device name", "zero").value("zero");
/// let erase = Dialog::confirm("Erase everything?", "Apps, files and settings go.")
///     .yes("erase")
///     .hold();
/// # let _ = (join, rename, erase);
/// ```
#[derive(Clone, Debug)]
pub struct Dialog {
    pub(crate) title: String,
    pub(crate) line: String,
    pub(crate) field: Option<Field>,
    pub(crate) yes: String,
    pub(crate) no: String,
    pub(crate) danger: bool,
    pub(crate) hold: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub text: String,
    pub placeholder: String,
    pub masked: bool,
    /// A password: Tab shows or hides it.
    pub secret: bool,
}

/// How a dialog ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// Enter, or the held chip. The field's text, empty without one. Wipe
    /// a password once used.
    Yes(String),
    /// Esc.
    No,
}

impl Dialog {
    fn new(title: &str) -> Dialog {
        Dialog {
            title: title.into(),
            line: String::new(),
            field: None,
            yes: "ok".into(),
            no: "cancel".into(),
            danger: false,
            hold: false,
        }
    }

    /// One line of text, with `placeholder` while it is empty.
    pub fn text(title: &str, placeholder: &str) -> Dialog {
        Dialog {
            field: Some(Field {
                text: String::new(),
                placeholder: placeholder.into(),
                masked: false,
                secret: false,
            }),
            ..Dialog::new(title)
        }
    }

    /// A password: dots, and Tab shows it.
    pub fn password(title: &str) -> Dialog {
        Dialog {
            field: Some(Field {
                text: String::new(),
                placeholder: String::new(),
                masked: true,
                secret: true,
            }),
            ..Dialog::new(title)
        }
    }

    /// A question to read before deciding: Enter does it, Esc doesn't.
    pub fn confirm(title: &str, line: &str) -> Dialog {
        Dialog {
            line: line.into(),
            ..Dialog::new(title)
        }
    }

    /// The text the field starts with.
    pub fn value(mut self, text: &str) -> Dialog {
        if let Some(f) = self.field.as_mut() {
            f.text = text.chars().take(MAX).collect();
        }
        self
    }

    /// What the field shows while it is empty.
    pub fn placeholder(mut self, text: &str) -> Dialog {
        if let Some(f) = self.field.as_mut() {
            f.placeholder = text.into();
        }
        self
    }

    /// A line under the field, or the question's text.
    pub fn line(mut self, line: &str) -> Dialog {
        self.line = line.into();
        self
    }

    /// What yes does, one lowercase word: "join", "restart". "ok" if unset.
    pub fn yes(mut self, word: &str) -> Dialog {
        self.yes = word.into();
        self
    }

    /// What Esc says: "cancel", or "later" for a restart.
    pub fn no(mut self, word: &str) -> Dialog {
        self.no = word.into();
        self
    }

    /// Takes something away: a red edge.
    pub fn danger(mut self) -> Dialog {
        self.danger = true;
        self
    }

    /// Loses data that can't come back: Enter does nothing, and yes is a red
    /// chip on 7 held 0.7 s.
    pub fn hold(mut self) -> Dialog {
        self.danger = true;
        self.hold = true;
        self
    }

    /// True when it has a field: the bar shows its hints, not chips.
    pub fn typing(&self) -> bool {
        self.field.is_some()
    }

    /// The bar while a field has focus. The runtime draws it; mockups may.
    pub fn hints(&self) -> Vec<(&str, &str)> {
        let mut h = vec![("Esc", self.no.as_str())];
        if let Some(f) = self.field.as_ref().filter(|f| f.secret) {
            h.push(("Tab", if f.masked { "show" } else { "hide" }));
        }
        if !self.hold {
            h.push(("Enter", self.yes.as_str()));
        }
        h
    }
}

/// What a key does to an open dialog.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Out {
    Nothing,
    Yes,
    No,
}

impl Dialog {
    pub(crate) fn key(&mut self, k: &RawKey) -> Out {
        if k.state == State::Release {
            return Out::Nothing;
        }
        match k.code {
            code::ESC if k.state == State::Press => return Out::No,
            code::ENTER if k.state == State::Press && !self.hold => return Out::Yes,
            _ => {}
        }
        let Some(f) = self.field.as_mut() else {
            return Out::Nothing;
        };
        match k.code {
            code::TAB if f.secret && k.state == State::Press => f.masked = !f.masked,
            code::BACKSPACE => {
                f.text.pop();
            }
            _ => {
                if let Some(c) = k.ch.filter(|c| !c.is_control() && !k.ctrl && !k.alt) {
                    if f.text.chars().count() < MAX {
                        f.text.push(c);
                    }
                }
            }
        }
        Out::Nothing
    }

    /// The answer's text, taken out so the dialog keeps no copy.
    pub(crate) fn take_text(&mut self) -> String {
        self.field
            .as_mut()
            .map(|f| std::mem::take(&mut f.text))
            .unwrap_or_default()
    }

    /// The card, over `area` dimmed. The runtime draws it, and the bars;
    /// mockups may call this.
    pub fn draw(&self, c: &mut Canvas, t: &Theme, area: Rect) {
        widgets::scrim(c, t, area);
        let edge = if self.danger { t.c.critical } else { t.c.accent };
        let w = t.m.width - 44;
        let lines = if self.line.is_empty() {
            Vec::new()
        } else {
            wrap(t, &self.line, w - 24)
        };
        let lines_h = lines.len().min(3) as i32 * 18;
        // The blocks, top to bottom: a field, text, and, unless a field has
        // the hint bar, the keys.
        let field_h = if self.field.is_some() { 22 } else { 0 };
        let keys_h = if self.field.is_some() { 0 } else { t.m.pill_h };
        let blocks = [field_h, lines_h, keys_h];
        let gaps = blocks.iter().filter(|&&b| b > 0).count().saturating_sub(1) as i32;
        let body_h = blocks.iter().sum::<i32>() + gaps * t.m.card_gap;
        let h = widgets::card_h(t, body_h);
        let r = Rect::new(22, area.y + (area.h - h).max(0) / 2, w, h);
        let body = widgets::card(c, t, r, &self.title, edge);
        let mut y = body.y;
        if let Some(f) = &self.field {
            let fr = Rect::new(body.x, y, body.w, 22);
            widgets::text_field(c, t, fr, &f.text, f.masked, true, &f.placeholder);
            y = fr.bottom() + t.m.card_gap;
        }
        let ink = if self.field.is_some() { t.c.muted } else { t.c.soft() };
        let font = if self.field.is_some() { t.f.small } else { t.f.body };
        for line in lines.iter().take(3) {
            widgets::text_in(c, font, Rect::new(body.x, y, body.w, 18), line, ink);
            y += 18;
        }
        if self.field.is_none() {
            let y = body.bottom() - t.m.pill_h;
            let mut hints = vec![Hint {
                key: "Esc",
                label: &self.no,
            }];
            if !self.hold {
                hints.push(Hint {
                    key: "Enter",
                    label: &self.yes,
                });
            }
            widgets::key_row(c, t, body.x, y, &hints);
            if self.hold {
                let say = format!("hold 7 to {}", self.yes);
                let sw = t.f.small.measure(&say);
                let sr = Rect::new(body.right() - sw, y, sw, t.m.pill_h);
                widgets::text_in(c, t.f.small, sr, &say, t.c.muted);
            }
        }
    }
}

fn wrap(t: &Theme, s: &str, w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if t.f.body.measure(&next) > w && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
