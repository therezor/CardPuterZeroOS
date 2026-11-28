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
    /// A portal request of `gh0st session`, by its id and what it asks:
    /// `account` or `wallpaper`, the verb of the answer.
    Portal(u64, &'static str),
    /// An app's sudo, through `gh0st-askpass`, by the helper's id.
    Sudo(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Password {
        reason: String,
        retry: bool,
    },
    Approve {
        app: String,
        what: String,
    },
    /// A crosshair over the frozen screen; Enter picks the colour under it.
    PickColor,
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
    /// An approval, for a daemon prompt or a portal request.
    Answer(Origin, Answer),
    /// Esc: no answer.
    Dismiss(Origin),
    /// The colour under this point was picked.
    Color(Origin, (i32, i32)),
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
    /// The colour picker's crosshair; None is the middle.
    at: Option<(i32, i32)>,
    /// The screen's size, for the crosshair.
    pub size: (i32, i32),
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

    /// Whether a question from `origin` waits.
    pub fn has(&self, origin: &Origin) -> bool {
        self.queue.iter().any(|q| &q.origin == origin)
    }

    /// The daemon withdrew prompt `id`.
    pub fn cancel(&mut self, id: u64) {
        self.remove(&Origin::Daemon(id));
    }

    /// `gh0st session` withdrew portal request `id`.
    pub fn cancel_portal(&mut self, id: u64) {
        let o = self.queue.iter().find_map(|q| match q.origin {
            Origin::Portal(i, v) if i == id => Some(Origin::Portal(i, v)),
            _ => None,
        });
        if let Some(o) = o {
            self.remove(&o);
        }
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

    /// Keeps only the asks whose origin `keep` takes: a daemon went away,
    /// and its questions with it.
    pub fn retain(&mut self, keep: impl Fn(&Origin) -> bool) {
        let front = self.queue.front().is_some_and(|q| !keep(&q.origin));
        self.queue.retain(|q| keep(&q.origin));
        if front {
            self.reset();
        }
    }

    /// Where the colour picker's crosshair is.
    pub fn at(&self) -> (i32, i32) {
        self.at.unwrap_or((self.size.0 / 2, self.size.1 / 2))
    }

    fn reset(&mut self) {
        self.at = None;
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
        // The crosshair moves at once, held keys repeating; Shift steps 10.
        if a.kind == Kind::PickColor {
            let step = if k.shift { 10 } else { 1 };
            let (x, y) = self.at();
            let to = match k.code {
                code::UP => (x, y - step),
                code::DOWN => (x, y + step),
                code::LEFT => (x - step, y),
                code::RIGHT => (x + step, y),
                _ => (x, y),
            };
            if to != (x, y) {
                let (w, h) = self.size;
                self.at = Some((to.0.clamp(0, w - 1), to.1.clamp(0, h - 1)));
                return Action::Redraw;
            }
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
                if !matches!(a.origin, Origin::Daemon(_) | Origin::Portal(..)) {
                    return Action::None;
                }
                // On chips, not Enter: Enter is the key a typing user hits
                // without looking.
                match k.ch {
                    Some('5') => Action::Answer(a.origin.clone(), Answer::Once),
                    Some('7') => Action::Answer(a.origin.clone(), Answer::Always),
                    _ => Action::None,
                }
            }
            Kind::PickColor => {
                if k.is(code::ENTER) && k.state == State::Press {
                    return Action::Color(a.origin.clone(), self.at());
                }
                Action::None
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
    /// The colour picker's crosshair.
    pub at: (i32, i32),
}

/// The crosshair: a gap around the point, then an arm each way.
const RETICLE_GAP: i32 = 3;
const RETICLE_ARM: i32 = 8;

/// The top bar's icon and title over a question from `origin`.
pub fn bar(origin: &Origin) -> (&'static str, &'static str) {
    match origin {
        Origin::Portal(_, "account") => ("user", "Your name"),
        Origin::Portal(_, "wallpaper") => ("images", "Wallpaper"),
        Origin::Portal(_, "shot") => ("camera", "Screenshot"),
        Origin::Portal(_, "color") => ("palette", "Pick a colour"),
        Origin::Sudo(_) => ("shield", "Root"),
        _ => ("key-round", "Keyring"),
    }
}

impl View<'_> {
    fn title(&self) -> &str {
        match (&self.ask.origin, &self.ask.kind) {
            (Origin::Stale, _) => "Update the keyring",
            (Origin::Sudo(_), _) => "Allow root?",
            (_, Kind::Password { reason, .. }) if reason.contains(" wants to show ") => {
                "Show a secret"
            }
            (_, Kind::Password { .. }) => "Unlock the keyring",
            (Origin::Portal(_, "wallpaper"), _) => "Use as wallpaper?",
            (Origin::Portal(_, "shot"), _) => "Share this screenshot?",
            (_, Kind::Approve { .. }) => "Allow access?",
            (_, Kind::PickColor) => "Pick a colour",
        }
    }
}

/// Splits `s` into lines of at most `w` pixels in `f`, at most `max`.
pub(crate) fn wrap(f: gh0st_ui::font::Font, s: &str, w: i32, max: usize) -> Vec<String> {
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
    if v.ask.kind == Kind::PickColor {
        return draw_picker(c, t, v.at);
    }
    let all = c.bounds();
    c.clear(all, t.c.bg);
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
        (Origin::Portal(_, "wallpaper" | "shot"), Kind::Approve { app, .. }) => (
            wrap(t.f.body, &format!("{app} asks."), inner, 1),
            false,
            None,
        ),
        (_, Kind::Approve { app, what }) => (
            wrap(t.f.body, &format!("{app} wants to {what}."), inner, 3),
            false,
            None,
        ),
        (_, Kind::PickColor) => (Vec::new(), false, None),
    };
    // A check under way is no error: a right password never shows red.
    let note = if v.checking {
        Some(("Checking\u{2026}", false))
    } else if v.wrong {
        Some(("Wrong password", true))
    } else {
        note.map(|n| (n, true))
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
    // The wallpaper asked about is behind the question: keep it in view.
    let y = match v.ask.origin {
        Origin::Portal(_, "wallpaper" | "shot") => room.y + (room.h - h).max(0),
        _ => room.y + (room.h - h).max(0) / 2,
    };
    let r = Rect::new(t.m.dialog_margin, y, w, h);
    let body = widgets::card(c, t, r, v.title(), t.c.accent);
    let mut y = body.y;
    if field {
        let fr = Rect::new(body.x, y, body.w, t.m.field_h);
        let dots = "\u{2022}".repeat(v.typed);
        widgets::text_field(c, t, fr, &dots, true, !v.checking, "Password");
        y = match note {
            Some((why, true)) => widgets::field_error(c, t, fr, why),
            Some((word, false)) => widgets::field_note(c, t, fr, word),
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
        let enter = match v.ask.origin {
            Origin::Stale => "update",
            Origin::Sudo(_) => "allow",
            _ => "unlock",
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

/// The frozen screen, whole, with a crosshair and what is under it: a
/// swatch, its hex, and the keys. The card keeps away from the crosshair.
fn draw_picker(c: &mut Canvas, t: &Theme, at: (i32, i32)) {
    let all = c.bounds();
    c.clear(all, t.c.bg);
    let under = c.get(at.0, at.1);
    let ink = if gh0st_ui::color::luminance(under) > 0.5 {
        t.c.bg
    } else {
        t.c.fg
    };
    let (x, y) = at;
    let (g, a) = (RETICLE_GAP, RETICLE_GAP + RETICLE_ARM);
    c.hline(x - a, x - g, y, ink);
    c.hline(x + g, x + a, y, ink);
    c.vline(x, y - a, y - g, ink);
    c.vline(x, y + g, y + a, ink);
    let [r, gr, b] = gh0st_ui::color::to_rgb8(under);
    let hex = format!("#{r:02X}{gr:02X}{b:02X}");
    let hints = [
        Hint {
            key: "Enter",
            label: "pick",
        },
        Hint {
            key: "Esc",
            label: "cancel",
        },
    ];
    let pad = t.m.card_inset;
    let sw = t.m.pill_h;
    let text_w = t.f.body.measure(&hex);
    let w = pad + sw + pad + text_w + pad + widgets::key_row_w(t, &hints) + pad;
    let h = sw + 2 * pad;
    let top = all.y + t.m.dialog_margin;
    let bottom = all.bottom() - h - t.m.dialog_margin;
    let card_y = if y > all.h / 2 { top } else { bottom };
    let card = Rect::new((all.w - w) / 2, card_y, w, h);
    c.round_rect(card, t.m.radius, t.c.surface);
    c.stroke_round_rect(card, t.m.radius, t.c.border);
    let swatch = Rect::new(card.x + pad, card.y + pad, sw, sw);
    c.round_rect(swatch, t.m.radius_small, under);
    c.stroke_round_rect(swatch, t.m.radius_small, t.c.border);
    let tx = swatch.right() + pad;
    widgets::text_in(
        c,
        t.f.body,
        Rect::new(tx, swatch.y, text_w, sw),
        &hex,
        t.c.fg,
    );
    widgets::key_row(c, t, tx + text_w + pad, swatch.y, &hints);
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
    fn a_daemon_that_died_takes_only_its_own_questions() {
        let mut p = Prompts::default();
        p.push(pw(Origin::Daemon(3)));
        p.push(Ask {
            origin: Origin::Portal(1, "account"),
            kind: Kind::Approve {
                app: "Chat".into(),
                what: "see your name".into(),
            },
        });
        p.push(approve(4));
        p.seen(1000);
        p.key(&key(30, Some('a')), T);
        p.retain(|o| matches!(o, Origin::Portal(..)));
        assert_eq!(
            p.current().map(|a| &a.origin),
            Some(&Origin::Portal(1, "account"))
        );
        assert_eq!(p.typed(), 0, "what was typed went with its question");
        p.retain(|o| !matches!(o, Origin::Portal(..)));
        assert!(p.is_empty());
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
            Action::Answer(Origin::Daemon(9), Answer::Once)
        );
        assert_eq!(
            p.key(&key(8, Some('7')), T),
            Action::Answer(Origin::Daemon(9), Answer::Always)
        );
        p.push(Ask {
            origin: Origin::Portal(4, "account"),
            kind: Kind::Approve {
                app: "Chat".into(),
                what: "see your name".into(),
            },
        });
        p.done();
        p.seen(T);
        assert_eq!(
            p.key(&key(6, Some('5')), T + ARM_MS),
            Action::Answer(Origin::Portal(4, "account"), Answer::Once)
        );
        assert_eq!(p.key(&key(30, Some('a')), T), Action::None);
    }

    #[test]
    fn the_colour_crosshair_moves_at_once_and_enter_picks() {
        let mut p = Prompts::default();
        p.size = (320, 170);
        p.push(Ask {
            origin: Origin::Portal(5, "color"),
            kind: Kind::PickColor,
        });
        p.seen(1000);
        assert_eq!(p.at(), (160, 85));
        assert_eq!(p.key(&key(code::LEFT, None), 1001), Action::Redraw);
        let mut shifted = key(code::DOWN, None);
        shifted.shift = true;
        p.key(&shifted, 1001);
        assert_eq!(p.at(), (159, 95));
        for _ in 0..400 {
            p.key(&key(code::RIGHT, None), 1001);
        }
        assert_eq!(p.at(), (319, 95), "kept on the screen");
        assert_eq!(
            p.key(&key(code::ENTER, None), 1001),
            Action::None,
            "not armed"
        );
        assert_eq!(
            p.key(&key(code::ENTER, None), T),
            Action::Color(Origin::Portal(5, "color"), (319, 95))
        );
    }

    #[test]
    fn a_check_under_way_shows_no_red() {
        let t = Theme::new(gh0st_ui::theme::named(None));
        let ask = pw(Origin::Open);
        let draw = |checking: bool, wrong: bool| {
            let mut c = Canvas::new(t.m.width, t.m.height);
            let s = Status {
                dock: &[],
                front: None,
                icon: "key-round",
                title: "Keyring",
                context: "",
                tabs: &[],
                front_tab: 0,
                item: None,
                clock: "20:45",
                battery: None,
                net: gh0st_ui::widgets::Net::Wifi(3),
                bluetooth: false,
                notices: 0,
            };
            let v = View {
                ask: &ask,
                typed: 4,
                checking,
                wrong,
                at: (0, 0),
            };
            draw(&mut c, &t, &s, &v);
            c.pixels().iter().filter(|&&p| p == t.c.critical).count()
        };
        assert_eq!(draw(true, false), 0, "a right password never flashes red");
        assert!(draw(false, true) > 0, "a wrong one does");
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
            Action::Answer(Origin::Daemon(2), Answer::Once)
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
