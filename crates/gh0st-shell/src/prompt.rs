//! The keyring's questions, asked on a shell screen while the shell holds
//! the keyboard: no app can read what is typed here. A queue, one shown at
//! a time. No I/O: the shell checks passwords and tells the daemon.

use std::collections::VecDeque;

use gh0st_core::keyring::Answer;
use gh0st_core::keys::{Key, State, code};
use gh0st_core::password::wipe;
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Chip, Hint, Look, Status};

/// As the lock screen takes it.
const MAX_PASSWORD: usize = 512;
/// A new question takes no answer this soon: a key meant for the app
/// before it can't approve what the app asked for.
pub const ARM_MS: u64 = 600;

/// Who wants the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A daemon prompt, by its id.
    Daemon(u64),
    /// The vault refused the login password: the old one, to rewrap.
    Stale,
    /// The lock password is off, so nothing opened the keyring: ask once.
    Open,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Password { reason: String, retry: bool },
    Approve { app: String, what: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub origin: Origin,
    pub kind: Kind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    /// Check this with `unix_chkpwd`, then send it on. The caller wipes it.
    Check(Vec<u8>),
    /// The old password goes straight to the daemon: the system no longer
    /// knows it.
    Old(Vec<u8>),
    Answer(u64, Answer),
    /// Esc: no answer.
    Dismiss(Origin),
}

#[derive(Default)]
pub struct Prompts {
    queue: VecDeque<Ask>,
    buf: Vec<u8>,
    /// A check is running: no keys but Esc.
    pub checking: bool,
    /// The last check said no.
    pub wrong: bool,
    /// The front question, and when it was first drawn.
    seen: Option<(Origin, u64)>,
}

impl Prompts {
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn current(&self) -> Option<&Ask> {
        self.queue.front()
    }

    /// Queues an ask. One from the same origin replaces it in place: a
    /// retry, or a daemon prompt that moved on to its next question.
    pub fn push(&mut self, a: Ask) {
        let front = self.queue.front().is_some_and(|q| q.origin == a.origin);
        if let Some(q) = self.queue.iter_mut().find(|q| q.origin == a.origin) {
            if let (Kind::Password { retry: true, .. }, true) = (&a.kind, front) {
                self.wrong = true;
            }
            *q = a;
            if front {
                self.checking = false;
                self.clear();
            }
            return;
        }
        self.queue.push_back(a);
    }

    /// The daemon withdrew prompt `id`.
    pub fn cancel(&mut self, id: u64) {
        self.remove(&Origin::Daemon(id));
    }

    pub fn remove(&mut self, origin: &Origin) {
        let front = self.queue.front().is_some_and(|q| &q.origin == origin);
        self.queue.retain(|q| &q.origin != origin);
        if front {
            self.reset();
        }
    }

    /// The front one is answered.
    pub fn done(&mut self) {
        self.queue.pop_front();
        self.reset();
    }

    /// Everything goes: the shell or the daemon went away.
    pub fn clear_all(&mut self) {
        self.queue.clear();
        self.reset();
    }

    fn reset(&mut self) {
        self.checking = false;
        self.wrong = false;
        self.clear();
    }

    fn clear(&mut self) {
        wipe(&mut self.buf);
        self.buf.clear();
    }

    /// The check of the front password answered no.
    pub fn checked_wrong(&mut self) {
        self.checking = false;
        self.wrong = true;
    }

    /// The screen drew the front question at `now`.
    pub fn seen(&mut self, now: u64) {
        let Some(front) = self.queue.front() else {
            return;
        };
        if self.seen.as_ref().is_none_or(|(o, _)| *o != front.origin) {
            self.seen = Some((front.origin.clone(), now));
        }
    }

    pub fn typed(&self) -> usize {
        std::str::from_utf8(&self.buf).map_or(self.buf.len(), |s| s.chars().count())
    }

    pub fn key(&mut self, k: &Key, now: u64) -> Action {
        let Some(a) = self.queue.front() else {
            return Action::None;
        };
        if k.state == State::Release {
            return Action::None;
        }
        if k.is(code::ESC) && k.state == State::Press {
            let o = a.origin.clone();
            return Action::Dismiss(o);
        }
        // Not drawn yet, or only just: only Esc counts.
        let armed = self
            .seen
            .as_ref()
            .is_some_and(|(o, t)| *o == a.origin && now >= t + ARM_MS);
        if !armed {
            return Action::None;
        }
        match &a.kind {
            Kind::Approve { .. } => {
                if k.state != State::Press {
                    return Action::None;
                }
                let Origin::Daemon(id) = a.origin else {
                    return Action::None;
                };
                // On chips, not Enter: Enter is the key a typing user hits
                // without looking.
                match k.ch {
                    Some('5') => Action::Answer(id, Answer::Once),
                    Some('7') => Action::Answer(id, Answer::Always),
                    _ => Action::None,
                }
            }
            Kind::Password { .. } => {
                if self.checking {
                    return Action::None;
                }
                if k.is(code::ENTER) {
                    if k.state != State::Press || self.buf.is_empty() {
                        return Action::None;
                    }
                    let pw = std::mem::take(&mut self.buf);
                    self.wrong = false;
                    if a.origin == Origin::Stale {
                        return Action::Old(pw);
                    }
                    self.checking = true;
                    return Action::Check(pw);
                }
                if k.is(code::BACKSPACE) {
                    if let Some(i) = std::str::from_utf8(&self.buf)
                        .ok()
                        .and_then(|s| s.char_indices().last().map(|(i, _)| i))
                    {
                        wipe(&mut self.buf[i..]);
                        self.buf.truncate(i);
                    }
                    return Action::Redraw;
                }
                match k.ch {
                    Some(ch) if !ch.is_control() => {
                        let mut b = [0u8; 4];
                        let s = ch.encode_utf8(&mut b);
                        if self.buf.len() + s.len() <= MAX_PASSWORD {
                            // All room up front: a grown buffer would free
                            // a copy of the password unwiped.
                            if self.buf.is_empty() {
                                self.buf.reserve_exact(MAX_PASSWORD);
                            }
                            self.buf.extend_from_slice(s.as_bytes());
                        }
                        wipe(&mut b);
                        self.wrong = false;
                        Action::Redraw
                    }
                    _ => Action::None,
                }
            }
        }
    }
}

impl Drop for Prompts {
    fn drop(&mut self) {
        self.clear();
    }
}

/// What a prompt screen shows, for the shell and the mockups.
pub struct View<'a> {
    pub ask: &'a Ask,
    pub typed: usize,
    pub checking: bool,
    pub wrong: bool,
}

impl View<'_> {
    fn title(&self) -> &str {
        match (&self.ask.origin, &self.ask.kind) {
            (Origin::Stale, _) => "Update the keyring",
            (_, Kind::Password { .. }) => "Unlock the keyring",
            (_, Kind::Approve { .. }) => "Allow access?",
        }
    }
}

/// Splits `s` into lines of at most `w` pixels in `f`, at most `max`.
fn wrap(f: gh0st_ui::font::Font, s: &str, w: i32, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if f.measure(&next) <= w || line.is_empty() {
            line = next;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && f.measure(&format!("{last}\u{2026}")) > w {
                last.pop();
            }
            last.push('\u{2026}');
        }
    }
    for l in &mut lines {
        while f.measure(l) > w && l.chars().count() > 1 {
            l.pop();
            l.pop();
            l.push('\u{2026}');
        }
    }
    lines
}

/// The prompt over the shell's ground: the top bar, a card, and keys.
pub fn draw(c: &mut Canvas, t: &Theme, status: &Status, v: &View) {
    widgets::status_bar(c, t, status);
    let area = widgets::body(t);
    let w = t.m.width - 2 * t.m.dialog_margin;
    let inner = w - 2 * t.m.card_inset;
    let (lines, field, note): (Vec<String>, bool, Option<&str>) = match (&v.ask.origin, &v.ask.kind)
    {
        (Origin::Stale, _) => (
            wrap(
                t.f.small,
                "Your password changed outside Settings. Type the old one.",
                inner,
                2,
            ),
            true,
            None,
        ),
        (_, Kind::Password { reason, .. }) => (wrap(t.f.small, reason, inner, 2), true, None),
        (_, Kind::Approve { app, what }) => (
            wrap(t.f.body, &format!("{app} wants to {what}."), inner, 3),
            false,
            None,
        ),
    };
    let note = if v.checking {
        Some("Checking\u{2026}")
    } else if v.wrong {
        Some("Wrong password")
    } else {
        note
    };
    let line_font = if field { t.f.small } else { t.f.body };
    let lines_h = lines.len() as i32 * t.m.line_h;
    let error_h = if field && note.is_some() {
        2 + t.m.field_note_h
    } else {
        0
    };
    let field_h = if field { t.m.field_h + error_h } else { 0 };
    let keys_h = if field { 0 } else { t.m.pill_h };
    let blocks = [field_h, lines_h, keys_h];
    let gaps = blocks.iter().filter(|&&b| b > 0).count().saturating_sub(1) as i32;
    let body_h = blocks.iter().sum::<i32>() + gaps * t.m.card_gap;
    let h = widgets::card_h(t, body_h);
    let bottom_bar = t.m.hint_h;
    let room = Rect::new(area.x, area.y, area.w, t.m.height - bottom_bar - area.y);
    let r = Rect::new(t.m.dialog_margin, room.y + (room.h - h).max(0) / 2, w, h);
    let body = widgets::card(c, t, r, v.title(), t.c.accent);
    let mut y = body.y;
    if field {
        let fr = Rect::new(body.x, y, body.w, t.m.field_h);
        let dots = "\u{2022}".repeat(v.typed);
        widgets::text_field(c, t, fr, &dots, true, !v.checking, "Password");
        y = match note {
            Some(why) => widgets::field_error(c, t, fr, why),
            None => fr.bottom(),
        } + t.m.card_gap;
    }
    let ink = if field { t.c.muted } else { t.c.soft() };
    for line in &lines {
        widgets::text_in(
            c,
            line_font,
            Rect::new(body.x, y, body.w, t.m.line_h),
            line,
            ink,
        );
        y += t.m.line_h;
    }
    if field {
        let enter = if v.ask.origin == Origin::Stale {
            "update"
        } else {
            "unlock"
        };
        let hints = [
            Hint {
                key: "Esc",
                label: "not now",
            },
            Hint {
                key: "Enter",
                label: enter,
            },
        ];
        widgets::hint_bar(c, t, &hints, None);
    } else {
        let ky = body.bottom() - t.m.pill_h;
        let hints = [Hint {
            key: "Esc",
            label: "deny",
        }];
        widgets::key_row(c, t, body.x, ky, &hints);
        let none = Chip::default();
        let once = Chip {
            icon: "check",
            word: "Once",
            look: Look::Normal,
        };
        let always = Chip {
            icon: "check-check",
            word: "Always",
            look: Look::Normal,
        };
        widgets::action_bar(c, t, &[none, once, none, always, none]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_core::keys::Key;

    fn key(code: u16, ch: Option<char>) -> Key {
        Key {
            code,
            state: State::Press,
            ch,
            text: ch,
            shift: false,
            ctrl: false,
            alt: false,
            fn_: false,
        }
    }

    fn pw(origin: Origin) -> Ask {
        Ask {
            origin,
            kind: Kind::Password {
                reason: "git wants the keyring".into(),
                retry: false,
            },
        }
    }

    fn approve(id: u64) -> Ask {
        Ask {
            origin: Origin::Daemon(id),
            kind: Kind::Approve {
                app: "Chromium".into(),
                what: "read \u{201c}github\u{201d}".into(),
            },
        }
    }

    const T: u64 = 1000 + ARM_MS;

    #[test]
    fn a_password_is_typed_then_checked() {
        let mut p = Prompts::default();
        p.push(pw(Origin::Daemon(3)));
        p.seen(1000);
        assert_eq!(p.key(&key(code::ENTER, None), T), Action::None, "empty");
        for (c, ch) in [(30, 'a'), (48, 'b')] {
            p.key(&key(c, Some(ch)), T);
        }
        p.key(&key(code::BACKSPACE, None), T);
        p.key(&key(46, Some('c')), T);
        assert_eq!(p.typed(), 2);
        assert_eq!(
            p.key(&key(code::ENTER, None), T),
            Action::Check(b"ac".to_vec())
        );
        assert!(p.checking);
        assert_eq!(
            p.key(&key(30, Some('x')), T),
            Action::None,
            "no keys while checking"
        );
        p.checked_wrong();
        assert!(p.wrong && !p.checking);
    }

    #[test]
    fn the_old_password_skips_the_check() {
        let mut p = Prompts::default();
        p.push(pw(Origin::Stale));
        p.seen(1000);
        p.key(&key(30, Some('o')), T);
        assert_eq!(
            p.key(&key(code::ENTER, None), T),
            Action::Old(b"o".to_vec())
        );
    }

    #[test]
    fn approval_is_on_chips_and_waits_to_arm() {
        let mut p = Prompts::default();
        p.push(approve(9));
        assert_eq!(p.key(&key(6, Some('5')), T), Action::None, "not drawn yet");
        p.seen(1000);
        assert_eq!(p.key(&key(6, Some('5')), 1100), Action::None, "too soon");
        assert_eq!(
            p.key(&key(code::ESC, None), 1100),
            Action::Dismiss(Origin::Daemon(9))
        );
        assert_eq!(
            p.key(&key(code::ENTER, None), T),
            Action::None,
            "Enter approves nothing"
        );
        assert_eq!(
            p.key(&key(6, Some('5')), T),
            Action::Answer(9, Answer::Once)
        );
        assert_eq!(
            p.key(&key(8, Some('7')), T),
            Action::Answer(9, Answer::Always)
        );
        assert_eq!(p.key(&key(30, Some('a')), T), Action::None);
    }

    #[test]
    fn a_new_front_question_arms_again() {
        let mut p = Prompts::default();
        p.push(approve(1));
        p.push(approve(2));
        p.seen(1000);
        p.done();
        p.seen(T);
        assert_eq!(p.key(&key(6, Some('5')), T + 1), Action::None);
        assert_eq!(
            p.key(&key(6, Some('5')), T + ARM_MS),
            Action::Answer(2, Answer::Once)
        );
    }

    #[test]
    fn the_queue_replaces_and_cancels_by_origin() {
        let mut p = Prompts::default();
        p.push(pw(Origin::Daemon(1)));
        p.push(pw(Origin::Daemon(2)));
        p.seen(1000);
        p.key(&key(30, Some('a')), T);
        // A retry of the front one: same place, field cleared, marked wrong.
        p.push(Ask {
            origin: Origin::Daemon(1),
            kind: Kind::Password {
                reason: "x".into(),
                retry: true,
            },
        });
        assert_eq!(p.current().unwrap().origin, Origin::Daemon(1));
        assert!(p.wrong && p.typed() == 0);
        p.cancel(1);
        assert_eq!(p.current().unwrap().origin, Origin::Daemon(2));
        assert!(!p.wrong);
        p.done();
        assert!(p.is_empty());
    }
}
