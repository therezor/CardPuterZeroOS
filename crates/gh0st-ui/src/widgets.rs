//! Shared pieces every gh0st app draws: the status bar, the hint bar, tiles,
//! rows, fields and dialogs. Each one draws into a rect it is given and takes
//! every size and colour from the theme.

use crate::canvas::Canvas;
use crate::color::{Color, mix};
use crate::font::Font;
use crate::geom::Rect;
use crate::icon::{self, IconSize};
use crate::image::Image;
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    None,
    /// Signal 0-4 bars.
    Wifi(u8),
    Ethernet,
}

#[derive(Clone, Copy, Debug)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

pub struct Status<'a> {
    /// Open apps after the skull, in launch order.
    pub dock: &'a [Visual<'a>],
    /// The filled slot: 0 is the skull, 1 the first app. `None` fills none.
    pub front: Option<usize>,
    /// The page's name, after the dock. Empty on home.
    pub title: &'a str,
    pub clock: &'a str,
    pub battery: Option<Battery>,
    pub net: Net,
}

/// The shell's mark: a skull, as on pt35, drawn smooth.
pub const SKULL: &str = "gh0st-skull";

/// Body area between the two bars.
pub fn body(t: &Theme) -> Rect {
    Rect::new(0, t.m.bar_h, t.m.width, t.m.height - t.m.bar_h - t.m.hint_h)
}

/// Body area minus the margins: where rows and tiles go.
pub fn content(t: &Theme) -> Rect {
    let b = body(t);
    Rect::new(
        b.x + t.m.margin,
        b.y + t.m.margin_v,
        b.w - 2 * t.m.margin,
        b.h - 2 * t.m.margin_v,
    )
}

/// The body cut into as many `row_h` rows as fit, centred vertically, inside
/// the side margins. Returns the area and the row count.
pub fn rows_area(t: &Theme, row_h: i32) -> (Rect, usize) {
    let b = body(t);
    let n = (b.h / row_h).max(1);
    let used = n * row_h;
    let r = Rect::new(
        b.x + t.m.margin,
        b.y + (b.h - used) / 2,
        b.w - 2 * t.m.margin,
        used,
    );
    (r, n as usize)
}

/// Draws `s` in `r`, centred vertically on the font's capitals.
pub fn text_in(c: &mut Canvas, f: Font, r: Rect, s: &str, color: Color) -> i32 {
    let y = r.y + (r.h + f.cap()) / 2;
    f.draw_elided(c, r.x, y, s, r.w, color)
}

pub fn text_right(c: &mut Canvas, f: Font, right: i32, r: Rect, s: &str, color: Color) -> i32 {
    let w = f.measure(s);
    text_in(c, f, Rect::new(right - w, r.y, w, r.h), s, color);
    right - w
}

pub fn text_centre(c: &mut Canvas, f: Font, r: Rect, s: &str, color: Color) {
    let w = f.measure(s).min(r.w);
    text_in(c, f, Rect::new(r.x + (r.w - w) / 2, r.y, w, r.h), s, color);
}

/// Labels that are not selected: softer than `fg`, clearer than `muted`.
pub fn soft(t: &Theme) -> Color {
    mix(t.c.muted, t.c.fg, 150)
}

/// The top bar, as on pt35: the skull, a slot for each open app, the page's
/// name, then network, battery and clock on the right.
pub fn status_bar(c: &mut Canvas, t: &Theme, s: &Status) {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.fill(r, t.c.bg);
    c.hline(0, r.w, r.bottom() - 1, t.c.border);
    let inner = Rect::new(t.m.pad, 0, r.w - 2 * t.m.pad, r.h - 1);

    let mut x = text_right(c, t.f.small_bold, inner.right(), inner, s.clock, t.c.fg) - 9;
    if let Some(b) = s.battery {
        x = battery(c, t, x, inner, b) - 9;
    }
    match s.net {
        Net::Wifi(level) => {
            x -= 11;
            signal_bars(c, x, inner.y + 3, 9, level, t.c.fg, t.c.border);
            x -= 8;
        }
        Net::Ethernet => {
            x -= 14;
            icon::draw(c, "ethernet-port", IconSize::Small, x, inner.y + 1, t.c.fg);
            x -= 8;
        }
        Net::None => {}
    }
    let end = x;

    let y = (inner.h - t.m.slot_h + 1) / 2;
    let step = t.m.slot_w + t.m.slot_gap;
    // The skull has no box: it is accent while home is in front.
    let s16 = IconSize::Bar as i32;
    let ink = if s.front == Some(0) {
        t.c.accent
    } else {
        t.c.muted
    };
    icon::draw(
        c,
        SKULL,
        IconSize::Bar,
        t.m.pad - 2,
        (inner.h - s16) / 2,
        ink,
    );
    let mut sx = t.m.pad - 2 + s16 + 2 * t.m.slot_gap;

    // Every app gets a slot while they fit; past that the last one counts
    // the rest.
    let fit = ((end - sx + t.m.slot_gap) / step).max(0) as usize;
    let n = s.dock.len();
    let shown = if n <= fit { n } else { fit.saturating_sub(1) };
    for (i, v) in s.dock.iter().take(shown).enumerate() {
        let r = Rect::new(sx, y, t.m.slot_w, t.m.slot_h);
        let front = s.front == Some(i + 1);
        let ink = slot(c, t, r, front);
        slot_visual(c, t, r, v, front, ink);
        sx += step;
    }
    if shown < n {
        let r = Rect::new(sx, y, t.m.slot_w, t.m.slot_h);
        let ink = slot(c, t, r, false);
        let mut buf = String::with_capacity(4);
        buf.push('+');
        buf.push_str(&(n - shown).min(99).to_string());
        text_centre(c, t.f.small_bold, r, &buf, ink);
        sx += step;
    }
    if !s.title.is_empty() {
        let x = sx + t.m.slot_gap;
        let title = Rect::new(x, inner.y, end - x, inner.h);
        text_in(c, t.f.small_bold, title, s.title, t.c.fg);
    }
}

/// A bar slot: filled when its app is in front, outlined when not. Returns
/// the ink for what goes inside.
fn slot(c: &mut Canvas, t: &Theme, r: Rect, front: bool) -> Color {
    if front {
        c.round_rect(r, t.m.radius, t.c.accent);
        t.c.accent_fg
    } else {
        c.stroke_round_rect(r, t.m.radius, t.c.border);
        t.c.muted
    }
}

fn slot_visual(c: &mut Canvas, t: &Theme, r: Rect, v: &Visual, front: bool, ink: Color) {
    let s = t.m.slot_icon;
    let (x, y) = r.centre(s, s);
    match v {
        Visual::Badge { icon: name, tint } => {
            let col = if front { ink } else { *tint };
            icon::draw(c, name, IconSize::Tiny, x, y, col);
        }
        Visual::Image(img) => {
            let (x, y) = r.centre(img.w, img.h);
            c.image(x, y, img);
        }
        Visual::Letter { ch, tint } => {
            let col = if front { ink } else { *tint };
            let mut buf = [0u8; 4];
            let up = ch.to_uppercase().next().unwrap_or(*ch);
            text_centre(c, t.f.small_bold, r, up.encode_utf8(&mut buf), col);
        }
    }
}

/// Battery outline with fill and percentage, right-aligned to `right`.
/// Returns its left edge.
fn battery(c: &mut Canvas, t: &Theme, right: i32, r: Rect, b: Battery) -> i32 {
    let label = Percent::new(b.percent);
    let x = text_right(c, t.f.small, right, r, label.as_str(), t.c.fg) - 4;
    let (w, h) = (15, 8);
    let bx = x - w - 2;
    let by = r.y + (r.h - h) / 2;
    let fill = if b.charging {
        t.c.accent
    } else if b.percent <= 5 {
        t.c.critical
    } else if b.percent <= 20 {
        t.c.warning
    } else {
        t.c.fg
    };
    c.stroke_round_rect(Rect::new(bx, by, w, h), 1, t.c.muted);
    c.fill(Rect::new(bx + w, by + 2, 2, h - 4), t.c.muted);
    let inner = w - 4;
    let level = ((inner * b.percent.min(100) as i32) + 50) / 100;
    c.fill(Rect::new(bx + 2, by + 2, level.max(1), h - 4), fill);
    bx
}

/// "87%" without allocating: the bar redraws every minute.
struct Percent {
    buf: [u8; 4],
    len: usize,
}

impl Percent {
    fn new(p: u8) -> Percent {
        let p = p.min(100);
        let mut buf = [0u8; 4];
        let mut len = 0;
        if p >= 100 {
            buf[len] = b'1';
            len += 1;
        }
        if p >= 10 {
            buf[len] = b'0' + (p / 10 % 10);
            len += 1;
        }
        buf[len] = b'0' + p % 10;
        buf[len + 1] = b'%';
        Percent { buf, len: len + 2 }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Four rising bars, `level` of them lit, bottom at `y + h`.
pub fn signal_bars(c: &mut Canvas, x: i32, y: i32, h: i32, level: u8, on: Color, off: Color) {
    for i in 0..4 {
        let bh = 3 + i * (h - 3) / 3;
        let col = if (i as u8) < level { on } else { off };
        c.fill(Rect::new(x + i * 3, y + h - bh, 2, bh), col);
    }
}

pub struct Hint<'a> {
    pub key: &'a str,
    pub label: &'a str,
}

fn hint_rect(t: &Theme) -> Rect {
    Rect::new(0, t.m.height - t.m.hint_h, t.m.width, t.m.hint_h)
}

/// Bottom bar: key pills with what they do, and page dots on the right.
pub fn hint_bar(c: &mut Canvas, t: &Theme, hints: &[Hint], pages: Option<(usize, usize)>) {
    let r = hint_rect(t);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.y, t.c.border);
    let mut x = t.m.pad;
    for h in hints {
        x = key_pill(c, t, x, r, h.key) + 4;
        let label = Rect::new(x, r.y + 1, r.right() - x, r.h - 1);
        x = text_in(c, t.f.small, label, h.label, t.c.muted) + 12;
    }
    if let Some((page, count)) = pages {
        page_dots(c, t, r.right() - t.m.pad, r.y + r.h / 2 + 1, page, count);
    }
}

/// Search replaces the hint bar: the query, a caret, and how to leave.
pub fn search_bar(c: &mut Canvas, t: &Theme, query: &str) {
    let r = hint_rect(t);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.y, t.c.accent);
    let x = t.m.pad - 1;
    icon::draw(c, "search", IconSize::Small, x, r.y + 1, t.c.accent);
    let tx = x + IconSize::Small as i32 + 6;
    let right = r.right() - t.m.pad;
    let cancel = Rect::new(0, r.y + 1, 0, r.h - 1);
    let lx = text_right(c, t.f.small, right, cancel, "cancel", t.c.muted);
    let pill = t.f.small_bold.measure("Esc") + 7;
    key_pill(c, t, lx - 4 - pill, r, "Esc");
    let field = Rect::new(tx, r.y + 1, lx - pill - 16 - tx, r.h - 1);
    let end = text_in(c, t.f.body, field, query, t.c.fg);
    c.fill(Rect::new(end + 2, r.y + 4, 1, r.h - 7), t.c.accent);
}

pub fn key_pill(c: &mut Canvas, t: &Theme, x: i32, r: Rect, key: &str) -> i32 {
    let w = (t.f.small_bold.measure(key) + 7).max(13);
    let pill = Rect::new(x, r.y + 3, w, r.h - 5);
    c.round_rect(pill, t.m.radius_small, t.c.border);
    text_centre(c, t.f.small_bold, pill, key, t.c.fg);
    pill.right()
}

pub fn page_dots(c: &mut Canvas, t: &Theme, right: i32, cy: i32, page: usize, count: usize) {
    if count < 2 {
        return;
    }
    for i in 0..count {
        let x = right - ((count - 1 - i) as i32) * 8 - 3;
        let col = if i == page { t.c.accent } else { t.c.border };
        c.circle(x as f32, cy as f32, 2.0, col);
    }
}

/// What a tile or row shows as its picture.
pub enum Visual<'a> {
    /// A baked icon on a tinted badge.
    Badge { icon: &'a str, tint: Color },
    /// An app's own PNG, already scaled.
    Image(&'a Image),
    /// No icon at all: the name's first letter on a tinted badge.
    Letter { ch: char, tint: Color },
}

pub fn badge(c: &mut Canvas, t: &Theme, r: Rect, v: &Visual, selected: bool) {
    let tinted = |tint: Color| {
        if selected {
            (tint, t.c.bg)
        } else {
            (mix(t.c.bg, tint, 46), tint)
        }
    };
    match v {
        Visual::Badge { icon: name, tint } => {
            let (fill, ink) = tinted(*tint);
            c.round_rect(r, t.m.radius, fill);
            let size = if r.w >= 32 {
                IconSize::Medium
            } else {
                IconSize::Small
            };
            let s = size as i32;
            let (x, y) = r.centre(s, s);
            icon::draw(c, name, size, x, y, ink);
        }
        Visual::Letter { ch, tint } => {
            let (fill, ink) = tinted(*tint);
            c.round_rect(r, t.m.radius, fill);
            let f = if r.w >= 32 { t.f.title } else { t.f.body_bold };
            let mut buf = [0u8; 4];
            let up = ch.to_uppercase().next().unwrap_or(*ch);
            text_centre(c, f, r, up.encode_utf8(&mut buf), ink);
        }
        Visual::Image(img) => {
            let fill = if selected {
                mix(t.c.surface, t.c.fg, 36)
            } else {
                mix(t.c.bg, t.c.surface, 200)
            };
            c.round_rect(r, t.m.radius, fill);
            let (x, y) = r.centre(img.w, img.h);
            c.image(x, y, img);
        }
    }
}

/// The selection behind a tile or app row: a raised fill and an accent edge.
pub fn selection(c: &mut Canvas, t: &Theme, r: Rect) {
    c.round_rect(r, t.m.radius_large, t.c.surface);
}

/// The selection's outline. Drawn after the content, so an icon the frame
/// passes over never hides it.
pub fn selection_ring(c: &mut Canvas, t: &Theme, r: Rect) {
    c.stroke_round_rect(r, t.m.radius_large, t.c.accent);
}

/// What a grid tile or list row shows for one app.
pub struct AppItem<'a> {
    pub label: &'a str,
    /// The list layout's second line. Ignored by tiles.
    pub note: &'a str,
    pub visual: Visual<'a>,
    pub pinned: bool,
}

/// One grid cell: badge on top, name below, both centred, a pin mark in the
/// corner when pinned.
pub fn tile(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    if selected {
        selection(c, t, r);
    }
    tile_content(c, t, r, item, selected);
    if selected {
        selection_ring(c, t, r);
    }
}

/// A tile without its selection frame, for when the frame is animating.
pub fn tile_content(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    let b = t.m.badge;
    let br = Rect::new(r.x + (r.w - b) / 2, r.y + 6, b, b);
    badge(c, t, br, &item.visual, selected);
    if item.pinned {
        pin_mark(c, t, r.right() - 4, r.y + 4, selected);
    }
    let f = if selected { t.f.small_bold } else { t.f.small };
    let col = if selected { t.c.fg } else { soft(t) };
    let lr = Rect::new(
        r.x + 3,
        br.bottom() + 4,
        r.w - 6,
        r.bottom() - br.bottom() - 6,
    );
    let w = f.measure(item.label).min(lr.w);
    let y = lr.y + f.cap() + 1;
    f.draw_elided(c, lr.x + (lr.w - w) / 2, y, item.label, lr.w, col);
}

/// The pin icon with its right edge at `right`.
fn pin_mark(c: &mut Canvas, t: &Theme, right: i32, y: i32, selected: bool) {
    let s = IconSize::Small as i32;
    let col = if selected { t.c.accent } else { t.c.muted };
    icon::draw(c, "pin", IconSize::Small, right - s, y, col);
}

/// One home list row: badge, name, a second line when there is one, and a
/// pin mark when pinned.
pub fn app_row(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    if selected {
        selection(c, t, r);
    }
    app_row_content(c, t, r, item, selected);
    if selected {
        selection_ring(c, t, r);
    }
}

/// A list row without its selection frame.
pub fn app_row_content(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    let b = t.m.badge_small;
    let br = Rect::new(r.x + 6, r.y + (r.h - b) / 2, b, b);
    badge(c, t, br, &item.visual, selected);
    let x = br.right() + 10;
    let mut right = r.right() - 10;
    if item.pinned {
        let s = IconSize::Small as i32;
        pin_mark(c, t, right, r.y + (r.h - s) / 2, selected);
        right -= s + 6;
    }
    let name = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { soft(t) };
    if item.note.is_empty() {
        text_in(c, name, Rect::new(x, r.y, right - x, r.h), item.label, col);
    } else {
        name.draw_elided(c, x, r.y + 15, item.label, right - x, col);
        t.f.small
            .draw_elided(c, x, r.y + 29, item.note, right - x, t.c.muted);
    }
}

/// A switcher card: a large badge, the name and a short state line.
pub struct Card<'a> {
    pub label: &'a str,
    pub note: &'a str,
    pub visual: Visual<'a>,
}

/// A card without its selection frame: a thin edge so it reads as a card,
/// then its content.
pub fn card_content(c: &mut Canvas, t: &Theme, r: Rect, card: &Card, selected: bool) {
    if !selected {
        c.stroke_round_rect(r, t.m.radius_large, t.c.border);
    }
    let b = t.m.badge;
    // Badge, name and state as one block, centred in the card.
    let block = b + 6 + 18 + if card.note.is_empty() { 0 } else { 15 };
    let br = Rect::new(r.x + (r.w - b) / 2, r.y + (r.h - block) / 2, b, b);
    badge(c, t, br, &card.visual, selected);
    let f = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { soft(t) };
    let lr = Rect::new(r.x + 4, br.bottom() + 6, r.w - 8, 18);
    text_centre(c, f, lr, card.label, col);
    let nr = Rect::new(r.x + 4, lr.bottom(), r.w - 8, 15);
    text_centre(c, t.f.small, nr, card.note, t.c.muted);
}

pub enum Trailing<'a> {
    None,
    Chevron,
    Value(&'a str),
    ValueChevron(&'a str),
    Toggle(bool),
    /// A setting with a few values, changed with left and right.
    Choice(&'a str),
    /// A 0-100 level, changed with left and right: a short bar and a percent.
    Level(u8),
    /// Wi-Fi: signal bars and a lock for a secured network.
    Signal {
        level: u8,
        secure: bool,
    },
}

pub struct Row<'a> {
    /// An empty name keeps the icon column, so rows with and without a mark
    /// line up. `None` drops the column.
    pub icon: Option<&'a str>,
    pub tint: Color,
    pub label: &'a str,
    pub trailing: Trailing<'a>,
}

/// A settings row. A bar on the left marks the selection, so dense lists stay
/// calm.
pub fn list_row(c: &mut Canvas, t: &Theme, r: Rect, row: &Row, selected: bool) {
    if selected {
        c.round_rect(r, t.m.radius, t.c.surface);
        c.round_rect(Rect::new(r.x, r.y + 4, 3, r.h - 8), 1, t.c.accent);
    }
    let mut x = r.x + 9;
    if let Some(name) = row.icon {
        let s = IconSize::Small as i32;
        icon::draw(c, name, IconSize::Small, x, r.y + (r.h - s) / 2, row.tint);
        x += s + 9;
    }
    let mut right = r.right() - 8;
    let muted = t.c.muted;
    match row.trailing {
        Trailing::None => {}
        Trailing::Chevron => {
            right = chevron(c, right, r, muted);
        }
        Trailing::Value(v) => {
            right = text_right(c, t.f.small, right, r, v, muted) - 8;
        }
        Trailing::ValueChevron(v) => {
            right = chevron(c, right, r, muted);
            right = text_right(c, t.f.small, right - 3, r, v, muted) - 8;
        }
        Trailing::Toggle(on) => {
            right = toggle(c, t, right, r, on) - 8;
        }
        Trailing::Choice(v) => {
            let arrow = if selected { t.c.accent } else { muted };
            let col = if selected { t.c.fg } else { muted };
            right = text_right(c, t.f.small_bold, right, r, "\u{203a}", arrow) - 6;
            right = text_right(c, t.f.small, right, r, v, col) - 6;
            right = text_right(c, t.f.small_bold, right, r, "\u{2039}", arrow) - 8;
        }
        Trailing::Level(p) => {
            let arrow = if selected { t.c.accent } else { muted };
            let col = if selected { t.c.fg } else { muted };
            let label = Percent::new(p);
            right = text_right(c, t.f.small_bold, right, r, "\u{203a}", arrow) - 6;
            let w = t.f.small.measure("100%");
            text_right(c, t.f.small, right, r, label.as_str(), col);
            right -= w + 6;
            let bar = Rect::new(right - 40, r.y + (r.h - 4) / 2, 40, 4);
            c.round_rect(bar, 2, t.c.border);
            let fill = (bar.w * p.min(100) as i32 + 50) / 100;
            if fill > 0 {
                let on = if selected { t.c.accent } else { muted };
                c.round_rect(Rect::new(bar.x, bar.y, fill, bar.h), 2, on);
            }
            right = bar.x - 6;
            right = text_right(c, t.f.small_bold, right, r, "\u{2039}", arrow) - 8;
        }
        Trailing::Signal { level, secure } => {
            let bars = Rect::new(right - 11, r.y + (r.h - 9) / 2, 11, 9);
            signal_bars(c, bars.x, bars.y, bars.h, level, t.c.fg, t.c.border);
            right -= 11 + 7;
            if secure {
                let s = IconSize::Small as i32;
                icon::draw(
                    c,
                    "lock",
                    IconSize::Small,
                    right - s,
                    r.y + (r.h - s) / 2,
                    muted,
                );
                right -= s + 4;
            }
        }
    }
    let f = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { soft(t) };
    text_in(c, f, Rect::new(x, r.y, right - x, r.h), row.label, col);
}

fn chevron(c: &mut Canvas, right: i32, r: Rect, color: Color) -> i32 {
    let s = IconSize::Small as i32;
    icon::draw(
        c,
        "chevron-right",
        IconSize::Small,
        right - s + 3,
        r.y + (r.h - s) / 2,
        color,
    );
    right - s
}

pub fn toggle(c: &mut Canvas, t: &Theme, right: i32, r: Rect, on: bool) -> i32 {
    let (w, h) = (22, 12);
    let tr = Rect::new(right - w, r.y + (r.h - h) / 2, w, h);
    let track = if on { t.c.accent } else { t.c.border };
    c.round_rect(tr, h / 2, track);
    let knob = if on { t.c.accent_fg } else { t.c.muted };
    let cx = if on { tr.right() - h / 2 } else { tr.x + h / 2 };
    c.circle(cx as f32, (tr.y + h / 2) as f32, 4.0, knob);
    tr.x
}

/// A small header above a group of rows.
pub fn section(c: &mut Canvas, t: &Theme, r: Rect, label: &str) {
    text_in(
        c,
        t.f.small_bold,
        Rect::new(r.x + 9, r.y, r.w - 9, r.h),
        label,
        t.c.accent,
    );
}

/// Label on the left in sans, value on the right in mono: a readout.
pub fn readout(c: &mut Canvas, t: &Theme, r: Rect, label: &str, value: &str) {
    let x = r.x + 9;
    text_in(c, t.f.small, Rect::new(x, r.y, 66, r.h), label, t.c.muted);
    let vx = x + 68;
    text_in(
        c,
        t.f.mono,
        Rect::new(vx, r.y, r.right() - vx - 8, r.h),
        value,
        t.c.fg,
    );
}

/// A thin bar just right of `r`, in the margin. Nothing when everything fits.
pub fn scrollbar(c: &mut Canvas, t: &Theme, r: Rect, first: usize, visible: usize, total: usize) {
    if total <= visible {
        return;
    }
    let track = Rect::new(r.right() + 2, r.y + 2, 2, r.h - 4);
    c.fill(track, t.c.border);
    let h = ((track.h as usize * visible) / total).max(8) as i32;
    let y = track.y + ((track.h - h) as usize * first / (total - visible)) as i32;
    c.fill(Rect::new(track.x, y, 2, h), t.c.muted);
}

/// Text input. `masked` shows dots, `cursor` draws the caret at the end.
pub fn text_field(
    c: &mut Canvas,
    t: &Theme,
    r: Rect,
    text: &str,
    masked: bool,
    cursor: bool,
    placeholder: &str,
) {
    c.round_rect(r, t.m.radius, t.c.bg);
    let edge = if cursor { t.c.accent } else { t.c.border };
    c.stroke_round_rect(r, t.m.radius, edge);
    let inner = Rect::new(r.x + 8, r.y, r.w - 16, r.h);
    let f = t.f.body;
    let end = if text.is_empty() {
        text_in(c, f, inner, placeholder, t.c.muted);
        inner.x
    } else if masked {
        let n = text.chars().count();
        let mut x = inner.x;
        for _ in 0..n {
            c.circle(x as f32 + 3.0, (r.y + r.h / 2) as f32, 2.5, t.c.fg);
            x += 8;
        }
        x
    } else {
        text_in(c, f, inner, text, t.c.fg)
    };
    if cursor {
        c.fill(Rect::new(end + 1, r.y + 5, 1, r.h - 10), t.c.accent);
    }
}

/// A raised card in the middle of the screen with a title. Returns the body.
pub fn dialog(c: &mut Canvas, t: &Theme, r: Rect, title: &str, accent: Color) -> Rect {
    c.fill_alpha(Rect::new(0, 0, t.m.width, t.m.height), t.c.bg, 150);
    c.round_rect(r, t.m.radius_large, t.c.surface);
    c.stroke_round_rect(r, t.m.radius_large, accent);
    let head = Rect::new(r.x + 12, r.y + 6, r.w - 24, 18);
    text_in(c, t.f.body_bold, head, title, t.c.fg);
    Rect::new(
        r.x + 12,
        head.bottom() + 4,
        r.w - 24,
        r.bottom() - head.bottom() - 14,
    )
}

/// A short message above the hint bar, the only answer a hotkey gets.
/// Returns where it went.
pub fn toast(c: &mut Canvas, t: &Theme, text: &str) -> Rect {
    let w = (t.f.small_bold.measure(text) + 20).min(t.m.width - 2 * t.m.margin);
    let h = 18;
    let r = Rect::new((t.m.width - w) / 2, t.m.height - t.m.hint_h - h - 6, w, h);
    c.round_rect(r, t.m.radius, t.c.surface);
    c.stroke_round_rect(r, t.m.radius, t.c.border);
    text_centre(c, t.f.small_bold, r, text, t.c.fg);
    r
}

/// A strip across the top of the panel, drawn over whatever an app shows:
/// the ESC hold hint and the low-battery countdown.
pub fn banner(c: &mut Canvas, t: &Theme, text: &str, color: Color) -> Rect {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h + 4);
    c.fill(r, color);
    text_centre(c, t.f.small_bold, r, text, t.c.bg);
    r
}

/// The lock screen: the time, large, and how to get back in.
pub fn lock_screen(c: &mut Canvas, t: &Theme, clock: &str, s: &Status) {
    c.fill(c.bounds(), t.c.bg);
    status_bar(c, t, s);
    let b = body(t);
    let w = t.f.big.measure(clock);
    t.f.big
        .draw(c, (t.m.width - w) / 2, b.y + b.h / 2 + 4, clock, t.c.fg);
    let s = IconSize::Small as i32;
    icon::draw(
        c,
        "lock",
        IconSize::Small,
        (t.m.width - s) / 2,
        b.y + b.h / 2 - 38,
        t.c.muted,
    );
    hint_bar(
        c,
        t,
        &[Hint {
            key: "Enter",
            label: "unlock",
        }],
        None,
    );
}
