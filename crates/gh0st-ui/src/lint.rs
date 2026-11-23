//! The design rules, checked while a test draws: text whose ink doesn't fit
//! its box, a page's text under the bars, faces under 15 px, and chips off
//! their keys (`docs/design.md`).
//! Text past the screen's edge is not checked: cards and scrolled tiles
//! leave it on purpose, and the canvas clips them. Off until a test calls [`start`];
//! release builds keep none of it.

use crate::font::Font;
use crate::geom::Rect;
use std::cell::RefCell;

/// The faces allowed under 15 px (design.md, Type): Mono's figures and the
/// chips' words. The
/// terminal draws its grid without these widgets.
fn small_ok(f: Font) -> bool {
    [
        &crate::font::MONO_12,
        &crate::font::MONO_12_BOLD,
        &crate::font::TERMINUS_12_BOLD,
    ]
    .iter()
    .any(|m| std::ptr::eq(*m, f))
}

/// The 15 px faces are baked to 15 px from ascent to descent, which is a
/// 14.7 px em; anything smaller is under the floor.
const FLOOR_EM: f32 = 14.5;

/// How far ink may reach past its box: a descender into the line gap.
const SPILL: i32 = 2;

thread_local! {
    static FOUND: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    static AREA: std::cell::Cell<Option<Rect>> = const { std::cell::Cell::new(None) };
}

/// The page's own area while it draws, so its text must stay off the bars.
/// None while the bars and overlays draw.
pub fn area(r: Option<Rect>) {
    AREA.with(|a| a.set(r));
}

/// Starts checking on this thread.
pub fn start() {
    FOUND.with(|f| *f.borrow_mut() = Some(Vec::new()));
}

/// What was found since [`start`] or the last take. Checking goes on.
pub fn take() -> Vec<String> {
    FOUND.with(|f| {
        f.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

/// Stops checking on this thread.
pub fn stop() {
    FOUND.with(|f| *f.borrow_mut() = None);
}

fn on() -> bool {
    cfg!(debug_assertions) && FOUND.with(|f| f.borrow().is_some())
}

fn found(msg: String) {
    FOUND.with(|f| {
        if let Some(v) = f.borrow_mut().as_mut() {
            if !v.contains(&msg) {
                v.push(msg);
            }
        }
    });
}

/// `s` in face `f`, centred on its capitals in `r`.
pub(crate) fn text(f: Font, r: Rect, s: &str) {
    if !on() || s.trim().is_empty() {
        return;
    }
    // Where text_in puts it: the capitals centred in the box. Descenders
    // and accents may reach into the line gap, as type does, but no more.
    let base = r.y + (r.h + f.cap()) / 2;
    let (up, down) = f.ink(s);
    let spill = (r.y - (base - up)).max(base + down - r.bottom());
    if spill > SPILL {
        found(format!(
            "{s:?}: its ink leaves its {} px box by {spill} px",
            r.h
        ));
    }
    if f.em < FLOOR_EM && !small_ok(f) {
        found(format!(
            "{s:?}: a {:.1} em face, under the 15 px floor",
            f.em
        ));
    }
}

/// Any text, drawn with its baseline at `base`: on a page, it stays off
/// the bars.
pub(crate) fn drawn(f: &crate::font::BakedFont, base: i32, s: &str) {
    if !on() || s.trim().is_empty() {
        return;
    }
    let Some(a) = AREA.with(|a| a.get()) else {
        return;
    };
    let (up, down) = f.ink(s);
    let under = (a.y - (base - up)).max(base + down - a.bottom());
    if under > SPILL {
        found(format!("{s:?}: {under} px under the bars"));
    }
}

/// The chips over keys 5, 6 and 7: one goes on 6, two on 5 and 7.
pub(crate) fn chips(on_key: [bool; 3]) {
    if !on() {
        return;
    }
    let ok = matches!(
        on_key,
        [false, false, false] | [false, true, false] | [true, false, true] | [true, true, true]
    );
    if !ok {
        let keys: Vec<String> = (5..8)
            .zip(on_key)
            .filter(|(_, on)| *on)
            .map(|(k, _)| k.to_string())
            .collect();
        found(format!(
            "chips on {}: one goes on 6, two on 5 and 7",
            keys.join(" and ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::theme::Theme;
    use crate::widgets::{self, Chip};

    #[test]
    fn it_finds_what_breaks_the_rules_and_only_that() {
        let t = Theme::default();
        let mut c = Canvas::new(t.m.width, t.m.height);
        start();
        widgets::text_in(&mut c, t.f.small, Rect::new(0, 0, 100, 18), "fits", t.c.fg);
        assert!(take().is_empty());
        widgets::text_in(
            &mut c,
            t.f.small,
            Rect::new(0, 0, 100, 10),
            "squeezed",
            t.c.fg,
        );
        widgets::text_in(&mut c, t.f.body, Rect::new(0, 0, 100, 12), "Typing", t.c.fg);
        widgets::text_in(&mut c, t.f.mono, Rect::new(0, 0, 100, 18), "12 MB", t.c.fg);
        let found = take();
        assert_eq!(found.len(), 2, "{found:#?}");
        let chip = Chip {
            icon: "",
            word: "Check",
            ..Chip::default()
        };
        let none = Chip::default();
        widgets::action_bar(&mut c, &t, &[none, none, chip, none, none]);
        widgets::action_bar(&mut c, &t, &[none, chip, none, chip, none]);
        assert!(take().is_empty(), "one on 6, a pair on 5 and 7");
        widgets::action_bar(&mut c, &t, &[none, chip, chip, none, none]);
        assert_eq!(take().len(), 1);
        stop();
    }
}
