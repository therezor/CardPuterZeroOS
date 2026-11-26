//! The power dialog: Restart on 5 and Off on 7, each a chip held
//! 0.7 s, as the design system holds an action that closes every app. Esc
//! held on home opens it, and so does Power in home's menu. Esc cancels.
//! No I/O here: the shell runs the helper.

use crate::hold::{HeldKey, HoldTick};
use gh0st_core::keys::{Key, State, code};
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Chip, Hint, Look, Status};

const TITLE: &str = "Restart or power off?";
const BODY: &str =
    "Open apps close first. After a power off, turn the side switch off and on to start.";

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    /// A key let go too soon: say to hold it.
    Hint(&'static str),
    Close,
    /// Held to the end: restart (true) or power off.
    Run(bool),
}

#[derive(Default)]
pub struct Power {
    hold: HeldKey,
    /// What the running hold is for: restart (true) or power off.
    restart: bool,
}

impl Power {
    pub fn key(&mut self, k: &Key, now: u64) -> Action {
        if k.state == State::Release {
            if self.hold.let_go(k.code) {
                return Action::Hint(if self.restart {
                    "Hold to restart"
                } else {
                    "Hold to power off"
                });
            }
            return Action::None;
        }
        if k.state != State::Press {
            return Action::None;
        }
        if self.hold.holding() {
            // Any other key lets the hold go.
            self.hold.cancel();
            return Action::Redraw;
        }
        if k.is(code::ESC) {
            return Action::Close;
        }
        match k.ch {
            Some(d @ ('5' | '7')) => {
                self.restart = d == '5';
                self.hold.start(k.code, now);
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    /// The fill grows, and at its end the caller checks the key is still
    /// down before it runs.
    pub fn tick(&mut self, now: u64) -> (Action, Option<u16>) {
        match self.hold.tick(now) {
            HoldTick::Idle => (Action::None, None),
            HoldTick::Fill => (Action::Redraw, None),
            HoldTick::Done(key) => (Action::Run(self.restart), Some(key)),
        }
    }

    pub fn hold_at(&self) -> Option<u64> {
        self.hold.at()
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, status: &Status) {
        let all = c.bounds();
        c.clear(all, t.c.bg);
        widgets::status_bar(c, t, status);
        let area = widgets::body(t);
        let w = t.m.width - 2 * t.m.dialog_margin;
        let inner = w - 2 * t.m.card_inset;
        let lines = crate::prompt::wrap(t.f.small, BODY, inner, 3);
        let body_h = lines.len() as i32 * t.m.line_h + t.m.card_gap + t.m.pill_h;
        let h = widgets::card_h(t, body_h);
        let room_h = t.m.height - t.m.hint_h - area.y;
        let r = Rect::new(t.m.dialog_margin, area.y + (room_h - h).max(0) / 2, w, h);
        let body = widgets::card(c, t, r, TITLE, t.c.critical);
        let mut y = body.y;
        for line in &lines {
            let lr = Rect::new(body.x, y, body.w, t.m.line_h);
            widgets::text_in(c, t.f.small, lr, line, t.c.soft());
            y += t.m.line_h;
        }
        let hints = [Hint {
            key: "Esc",
            label: "cancel",
        }];
        widgets::key_row(c, t, body.x, body.bottom() - t.m.pill_h, &hints);
        let look = |restart: bool| match self.hold.fill() {
            Some(pc) if self.restart == restart => Look::Hold(pc),
            _ => Look::Danger,
        };
        let none = Chip::default();
        let restart = Chip {
            icon: "",
            word: "Restart",
            look: look(true),
        };
        let off = Chip {
            icon: "",
            word: "Off",
            look: look(false),
        };
        widgets::action_bar(c, t, &[none, restart, none, off, none]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hold::HOLD_MS;

    fn raw(code: u16, ch: Option<char>, state: State) -> Key {
        Key {
            code,
            state,
            ch,
            text: ch,
            shift: false,
            ctrl: false,
            alt: false,
            fn_: false,
        }
    }

    fn key(ch: char, code: u16, state: State) -> Key {
        raw(code, Some(ch), state)
    }

    const FIVE: u16 = 6;
    const SEVEN: u16 = 8;

    #[test]
    fn a_short_press_says_to_hold() {
        let mut p = Power::default();
        assert_eq!(p.key(&key('7', SEVEN, State::Press), 0), Action::Redraw);
        assert_eq!(
            p.key(&key('7', SEVEN, State::Release), 100),
            Action::Hint("Hold to power off")
        );
        assert_eq!(p.hold_at(), None);
    }

    #[test]
    fn a_full_hold_runs_once() {
        let mut p = Power::default();
        p.key(&key('5', FIVE, State::Press), 1000);
        assert_eq!(p.tick(1040).0, Action::Redraw);
        assert_eq!(p.tick(1000 + HOLD_MS), (Action::Run(true), Some(FIVE)));
        assert_eq!(p.tick(1000 + HOLD_MS + 40), (Action::None, None));
        assert_eq!(
            p.key(&key('5', FIVE, State::Release), 1800),
            Action::None,
            "the release after a done hold says nothing"
        );
    }

    #[test]
    fn esc_closes_and_enter_does_nothing() {
        let mut p = Power::default();
        assert_eq!(
            p.key(&raw(code::ENTER, None, State::Press), 0),
            Action::None
        );
        assert_eq!(p.key(&raw(code::ESC, None, State::Press), 0), Action::Close);
    }

    #[test]
    fn another_key_lets_the_hold_go() {
        let mut p = Power::default();
        p.key(&key('7', SEVEN, State::Press), 0);
        assert_eq!(p.key(&key('q', 16, State::Press), 100), Action::Redraw);
        assert_eq!(p.tick(HOLD_MS), (Action::None, None));
    }
}
