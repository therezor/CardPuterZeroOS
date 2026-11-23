//! The notifications panel: every notice, newest first, opened from 5 in
//! the switcher or from home's menu. Enter opens the notice's app, 8 or Del
//! clears one, 4 held clears them all (as in the switcher), Esc goes back.

use crate::hold::{HeldKey, HoldTick};
use crate::home::{self, Icon, Size};
use gh0st_core::notify::{self, Notice};
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Chip, Look, Message, Status};

/// A notice with the app it came from, as home shows that app.
#[derive(Clone)]
pub struct Row {
    pub notice: Notice,
    /// The app's name, or the poster's when no app answers to it.
    pub name: String,
    pub icon: Icon,
}

#[derive(Default)]
pub struct Panel {
    rows: Vec<Row>,
    selected: usize,
    top: usize,
    hold: HeldKey,
}

impl Panel {
    /// New rows, keeping the cursor on the same notice.
    pub fn set(&mut self, rows: Vec<Row>) {
        let keep = self.selected().map(|r| r.notice.id());
        self.rows = rows;
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.notice.id() == id))
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        if self.rows.is_empty() {
            self.hold.cancel();
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Back to the newest, as the panel opens.
    pub fn reset(&mut self) {
        self.selected = 0;
        self.top = 0;
        self.hold.cancel();
    }

    pub fn move_by(&mut self, d: i32) {
        self.hold.cancel();
        let n = self.rows.len() as i32;
        if n == 0 {
            return;
        }
        self.selected = (self.selected as i32 + d).clamp(0, n - 1) as usize;
    }

    /// Starts the hold that clears every notice, while key `code` is down.
    pub fn hold_all(&mut self, code: u16, now: u64) {
        if !self.rows.is_empty() {
            self.hold.start(code, now);
        }
    }

    pub fn holding(&self) -> bool {
        self.hold.holding()
    }

    pub fn cancel_hold(&mut self) {
        self.hold.cancel();
    }

    pub fn let_go(&mut self, code: u16) -> bool {
        self.hold.let_go(code)
    }

    pub fn hold_at(&self) -> Option<u64> {
        self.hold.at()
    }

    pub fn hold_tick(&mut self, now: u64) -> HoldTick {
        self.hold.tick(now)
    }

    fn rows_on_screen(t: &Theme) -> usize {
        (widgets::body(t).h / row_h(t)).max(1) as usize
    }

    pub fn draw(&mut self, c: &mut Canvas, t: &Theme, status: &Status, now_secs: u64) {
        c.clear(widgets::body(t), t.c.bg);
        widgets::status_bar(c, t, status);
        let none = Chip::default();
        if self.rows.is_empty() {
            let m = Message {
                icon: "bell",
                tone: None,
                text: "No notifications",
                note: "Apps and updates leave them here",
                keys: &[],
            };
            widgets::message(c, t, widgets::body(t), &m);
            widgets::action_bar(c, t, &[none; 5]);
            return;
        }
        let per = Self::rows_on_screen(t);
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + per {
            self.top = self.selected + 1 - per;
        }
        let b = widgets::body(t);
        let h = row_h(t);
        let y0 = b.y + (b.h - per as i32 * h) / 2;
        for (i, row) in self.rows.iter().enumerate().skip(self.top).take(per) {
            let r = Rect::new(
                t.m.margin,
                y0 + (i - self.top) as i32 * h,
                b.w - 2 * t.m.margin,
                h,
            );
            draw_row(c, t, r, row, now_secs, i == self.selected);
        }
        let track = Rect::new(t.m.margin, y0, b.w - 2 * t.m.margin, per as i32 * h);
        widgets::scrollbar(c, t, track, self.top, per, self.rows.len());
        let all = Chip {
            icon: "copy-x",
            word: "All",
            look: match self.hold.fill() {
                Some(pc) => Look::Hold(pc),
                None => Look::Danger,
            },
        };
        let clear = Chip {
            icon: "check",
            word: "Clear",
            look: Look::Normal,
        };
        widgets::action_bar(c, t, &[all, none, none, none, clear]);
    }
}

fn row_h(t: &Theme) -> i32 {
    t.m.app_row_h
}

/// The app's badge, the title, the body under it, the age on the right.
fn draw_row(c: &mut Canvas, t: &Theme, r: Rect, row: &Row, now_secs: u64, selected: bool) {
    if selected {
        widgets::selection(c, t, r);
    }
    let b = t.m.badge_small;
    let br = Rect::new(r.x + 6, r.y + (r.h - b) / 2, b, b);
    let v = home::visual(t, &row.icon, &row.name, Size::Small);
    widgets::badge(c, t, br, &v, selected);
    let age = notify::age(row.notice.time, now_secs);
    let line = Rect::new(r.x, r.y + 2, r.w - 8, 16);
    let right = widgets::text_right(c, t.f.small, r.right() - 8, line, &age, t.c.muted) - 8;
    let x = br.right() + 10;
    let title = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { widgets::soft(t) };
    title.draw_elided(c, x, r.y + 15, &row.notice.title, right - x, col);
    let body = if row.notice.body.is_empty() {
        row.name.as_str()
    } else {
        row.notice.body.as_str()
    };
    t.f.small
        .draw_elided(c, x, r.y + 29, body, r.right() - 8 - x, t.c.muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tag: &str) -> Row {
        Row {
            notice: Notice {
                app: "a".into(),
                tag: tag.into(),
                title: tag.into(),
                ..Notice::default()
            },
            name: "A".into(),
            icon: Icon::Letter { tint: 0 },
        }
    }

    #[test]
    fn new_rows_keep_the_cursor_on_its_notice() {
        let mut p = Panel::default();
        p.set(vec![row("x"), row("y"), row("z")]);
        p.move_by(2);
        p.set(vec![row("new"), row("x"), row("y"), row("z")]);
        assert_eq!(p.selected().unwrap().notice.tag, "z");
        p.set(vec![row("x")]);
        assert_eq!(p.selected().unwrap().notice.tag, "x");
        p.set(Vec::new());
        assert!(p.selected().is_none());
    }

    #[test]
    fn nothing_to_clear_nothing_to_hold() {
        let mut p = Panel::default();
        p.hold_all(5, 0);
        assert!(!p.holding());
        p.set(vec![row("x")]);
        p.hold_all(5, 0);
        assert!(p.holding());
        p.move_by(1);
        assert!(!p.holding());
    }
}
