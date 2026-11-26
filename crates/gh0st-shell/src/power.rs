//! The power dialog: Restart on 6, a chip held 0.7 s, as the design system
//! holds an action that closes every app. Esc held on home opens it, and so
//! does Power in home's menu. Esc cancels. There is no Off: the side switch
//! powers off, through a full shutdown (gh0st-power), and a power off from
//! here would only halt with the power still on. No I/O here: the shell
//! runs the helper.
//!
//! Also the last frame, [`bye`]: the panel keeps it until the power goes.

use crate::hold::{HeldKey, HoldTick};
use gh0st_core::keys::{Key, State, code};
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Chip, Hint, Look, Status};

const TITLE: &str = "Restart?";
const BODY: &str = "Open apps close first. To power off, turn the side switch off.";

/// The screen while the system goes down: the mark, the name, and
/// restarting or shutting down.
pub fn bye(c: &mut Canvas, t: &Theme, restart: bool) {
    let screen = c.bounds();
    c.clear(screen, t.c.bg);
    let m = widgets::Message {
        icon: widgets::LOGO,
        tone: Some(gh0st_ui::theme::Tone::Info),
        text: "Gh0st OS",
        note: if restart {
            "Restarting\u{2026}"
        } else {
            "Shutting down\u{2026}"
        },
        keys: &[],
    };
    widgets::message(c, t, screen, &m);
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    /// A key let go too soon: say to hold it.
    Hint(&'static str),
    Close,
    /// Restart held to the end.
    Run,
}

#[derive(Default)]
pub struct Power {
    hold: HeldKey,
}

impl Power {
    pub fn key(&mut self, k: &Key, now: u64) -> Action {
        if k.state == State::Release {
            if self.hold.let_go(k.code) {
                return Action::Hint("Hold to restart");
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
        if k.ch == Some('6') {
            self.hold.start(k.code, now);
            return Action::Redraw;
        }
        Action::None
    }

    /// The fill grows, and at its end the caller checks the key is still
    /// down before it runs.
    pub fn tick(&mut self, now: u64) -> (Action, Option<u16>) {
        match self.hold.tick(now) {
            HoldTick::Idle => (Action::None, None),
            HoldTick::Fill => (Action::Redraw, None),
            HoldTick::Done(key) => (Action::Run, Some(key)),
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
        let restart = Chip {
            icon: "",
            word: "Restart",
            look: self.hold.fill().map_or(Look::Danger, Look::Hold),
        };
        let none = Chip::default();
        widgets::action_bar(c, t, &[none, none, restart, none, none]);
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

    const SIX: u16 = 7;

    #[test]
    fn a_short_press_says_to_hold() {
        let mut p = Power::default();
        assert_eq!(p.key(&key('6', SIX, State::Press), 0), Action::Redraw);
        assert_eq!(
            p.key(&key('6', SIX, State::Release), 100),
            Action::Hint("Hold to restart")
        );
        assert_eq!(p.hold_at(), None);
    }

    #[test]
    fn a_full_hold_runs_once() {
        let mut p = Power::default();
        p.key(&key('6', SIX, State::Press), 1000);
        assert_eq!(p.tick(1040).0, Action::Redraw);
        assert_eq!(p.tick(1000 + HOLD_MS), (Action::Run, Some(SIX)));
        assert_eq!(p.tick(1000 + HOLD_MS + 40), (Action::None, None));
        assert_eq!(
            p.key(&key('6', SIX, State::Release), 1800),
            Action::None,
            "the release after a done hold says nothing"
        );
    }

    #[test]
    fn there_is_no_power_off() {
        let mut p = Power::default();
        for (ch, code) in [('5', 6), ('7', 8)] {
            assert_eq!(p.key(&key(ch, code, State::Press), 0), Action::None);
        }
        assert_eq!(p.hold_at(), None);
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
        p.key(&key('6', SIX, State::Press), 0);
        assert_eq!(p.key(&key('q', 16, State::Press), 100), Action::Redraw);
        assert_eq!(p.tick(HOLD_MS), (Action::None, None));
    }
}
