//! Shared pieces every gh0st app draws: the top bar, the action bar and its
//! chips, the hint bar, tiles, rows, fields, cards, toasts. Each one draws
//! into a rect it is given and takes every size and colour from the theme.
//! docs/design.md says how they are used.

use crate::canvas::Canvas;
use crate::color::{Color, mix};
use crate::font::Font;
use crate::geom::Rect;
use crate::icon::{self, IconSize};
use crate::image::Image;
use crate::theme::{self, Theme, Tone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    None,
    /// Signal 0-4 bars.
    Wifi(u8),
    Ethernet,
    /// This device is the access point.
    Hotspot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// Ten rows tall, so the bolt fits inside without cutting the outline.
const BATTERY_W: i32 = 17;
const BATTERY_H: i32 = 10;

/// The charging bolt, as tall as the room inside the outline. Drawn by
/// hand: the smallest baked icon is 12 px.
const BOLT_W: i32 = 5;
const BOLT_H: i32 = BATTERY_H - 2;
const BOLT: [u8; (BOLT_W * BOLT_H) as usize] = pixel_mask(
    b"\
..###\
.###.\
.##..\
#####\
..##.\
.##..\
.#...\
#....",
);

const fn pixel_mask<const N: usize>(art: &[u8; N]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        if art[i] == b'#' {
            out[i] = 255;
        }
        i += 1;
    }
    out
}

pub struct Status<'a> {
    /// Open apps after the mark, in launch order.
    pub dock: &'a [Visual<'a>],
    /// The filled slot: 0 is the mark, 1 the first app. `None` fills none.
    pub front: Option<usize>,
    /// The page's icon, before its name. Empty draws the gh0st mark.
    pub icon: &'a str,
    /// The page's name, after the dock. Empty on home.
    pub title: &'a str,
    /// After the title in Caption: the parent page, a step, a file. The
    /// first thing to go when the bar is full.
    pub context: &'a str,
    /// Sessions of the app, the front one underlined.
    pub tabs: &'a [&'a str],
    pub front_tab: usize,
    /// One word of the app's own after a dot, in a tone: "REC".
    pub item: Option<(&'a str, Tone)>,
    pub clock: &'a str,
    pub battery: Option<Battery>,
    pub net: Net,
    /// The controller is up.
    pub bluetooth: bool,
    /// Notifications waiting: a bell and the count. 0 draws nothing.
    pub notices: u8,
}

/// The shell's mark, where pt35 has its skull.
pub const LOGO: &str = "gh0st-components";

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

/// The body for the home grid and the switcher: the side margins and the
/// smaller `grid_v` above and below.
pub fn grid(t: &Theme) -> Rect {
    let b = body(t);
    Rect::new(
        b.x + t.m.margin,
        b.y + t.m.grid_v,
        b.w - 2 * t.m.margin,
        b.h - 2 * t.m.grid_v,
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
    crate::lint::text(f, r, s);
    let y = r.y + (r.h + f.cap()) / 2;
    f.draw_elided(c, r.x, y, s, r.w, color)
}

pub fn text_right(c: &mut Canvas, f: Font, right: i32, r: Rect, s: &str, color: Color) -> i32 {
    let w = f.measure(s);
    text_in(c, f, Rect::new(right - w, r.y, w, r.h), s, color);
    right - w
}

/// As [`text_right`], but no wider than `max`: past it the text is cut.
fn value_right(
    c: &mut Canvas,
    f: Font,
    right: i32,
    r: Rect,
    s: &str,
    color: Color,
    max: i32,
) -> i32 {
    let w = f.measure(s).min(max);
    text_in(c, f, Rect::new(right - w, r.y, w, r.h), s, color);
    right - w
}

pub fn text_centre(c: &mut Canvas, f: Font, r: Rect, s: &str, color: Color) {
    let w = f.measure(s).min(r.w);
    text_in(c, f, Rect::new(r.x + (r.w - w) / 2, r.y, w, r.h), s, color);
}

/// Labels that are not selected: softer than `fg`, clearer than `muted`.
pub fn soft(t: &Theme) -> Color {
    t.c.soft()
}

/// The top bar: where you are on the left, the device on the right. When
/// the title does not fit, the context goes first, then Bluetooth, then the
/// title shortens; the clock and battery always stay.
pub fn status_bar(c: &mut Canvas, t: &Theme, s: &Status) {
    if !draw_status(c, t, s, s.bluetooth) {
        draw_status(c, t, s, false);
    }
}

/// False when the title needs Bluetooth's room and `bluetooth` may give it.
fn draw_status(c: &mut Canvas, t: &Theme, s: &Status, bluetooth: bool) -> bool {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.clear(r, t.c.bg);
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
        Net::None => {
            x -= 12;
            icon::draw(c, "wifi-off", IconSize::Tiny, x, inner.y + 2, t.c.muted);
            x -= 8;
        }
        Net::Hotspot => {
            x -= 12;
            icon::draw(c, "radio-tower", IconSize::Tiny, x, inner.y + 2, t.c.fg);
            x -= 8;
        }
    }
    if s.notices > 0 {
        let n = Count::new(s.notices);
        x = text_right(c, t.f.small_bold, x, inner, n.as_str(), t.c.accent) - 13;
        icon::draw(c, "bell", IconSize::Tiny, x, inner.y + 2, t.c.accent);
        x -= 8;
    }
    if bluetooth {
        x -= 12;
        icon::draw(c, "bluetooth", IconSize::Tiny, x, inner.y + 2, t.c.fg);
        x -= 7;
    }
    if let Some((word, tone)) = s.item {
        let col = t.c.tone(tone);
        let tx = text_right(c, t.f.small_bold, x, inner, word, col);
        c.circle((tx - 6) as f32, (inner.h / 2) as f32 + 0.5, 2.5, col);
        x = tx - 9 - 8;
    }
    let end = x;

    // Rounded down, so slot ink shares rows with the clock's digits.
    let y = (inner.h - t.m.slot_h) / 2;
    let step = t.m.slot_w + t.m.slot_gap;
    // The mark is the size of the icons in the slots, and accent while home
    // is in front.
    let logo = t.m.slot_icon;
    let home = s.front == Some(0);
    let (mark, ink) = if s.icon.is_empty() {
        (
            LOGO,
            if home || s.dock.is_empty() {
                t.c.accent
            } else {
                t.c.muted
            },
        )
    } else {
        (s.icon, t.c.accent)
    };
    icon::draw(
        c,
        mark,
        IconSize::Tiny,
        t.m.pad,
        y + (t.m.slot_h - logo) / 2,
        ink,
    );
    let side = (t.m.slot_w - logo) / 2;
    if home {
        front_mark(c, t, t.m.pad - side, t.m.slot_w);
    }
    // Ink to ink, the mark and every icon are the same distance apart.
    let mut sx = t.m.pad + logo + t.m.slot_gap + side;

    // Every app gets a slot while they fit; past that the last one counts
    // the rest.
    let fit = ((end - sx + t.m.slot_gap) / step).max(0) as usize;
    let n = s.dock.len();
    let shown = if n <= fit { n } else { fit.saturating_sub(1) };
    for (i, v) in s.dock.iter().take(shown).enumerate() {
        let r = Rect::new(sx, y, t.m.slot_w, t.m.slot_h);
        slot_visual(c, t, r, v);
        if s.front == Some(i + 1) {
            front_mark(c, t, r.x, r.w);
        } else {
            // Behind: the same picture, faded into the bar.
            c.fill_alpha(r, t.c.bg, 110);
        }
        sx += step;
    }
    if shown < n {
        let r = Rect::new(sx, y, t.m.slot_w, t.m.slot_h);
        let mut buf = String::with_capacity(4);
        buf.push('+');
        buf.push_str(&(n - shown).min(99).to_string());
        text_centre(c, t.f.small_bold, r, &buf, t.c.muted);
        sx += step;
    }
    for (i, tab) in s.tabs.iter().enumerate() {
        let r = Rect::new(sx, y, t.m.slot_w, t.m.slot_h);
        let col = if i == s.front_tab { t.c.fg } else { t.c.muted };
        text_centre(c, t.f.small_bold, r, tab, col);
        if i == s.front_tab {
            front_mark(c, t, r.x, r.w);
        }
        sx += step;
    }
    if s.title.is_empty() {
        return true;
    }
    let x = sx + t.m.slot_gap;
    let room = end - x;
    let tw = t.f.small_bold.measure(s.title);
    if tw > room && bluetooth {
        return false;
    }
    let title = Rect::new(x, inner.y, tw.min(room), inner.h);
    let after = text_in(c, t.f.small_bold, title, s.title, t.c.fg);
    if !s.context.is_empty() {
        let cx = after + 6;
        if cx + t.f.small.measure(s.context) <= end {
            text_in(
                c,
                t.f.small,
                Rect::new(cx, inner.y, end - cx, inner.h),
                s.context,
                t.c.muted,
            );
        }
    }
    true
}

/// What is in front: an accent line on the bar's bottom edge, under it.
fn front_mark(c: &mut Canvas, t: &Theme, x: i32, w: i32) {
    c.fill(Rect::new(x, t.m.bar_h - 2, w, 2), t.c.accent);
}

fn slot_visual(c: &mut Canvas, t: &Theme, r: Rect, v: &Visual) {
    let s = t.m.slot_icon;
    let (x, y) = r.centre(s, s);
    match v {
        Visual::System { icon: name } => {
            icon::draw(c, name, IconSize::Tiny, x, y, t.c.accent);
        }
        Visual::Badge { icon: name, tint } => {
            icon::draw(c, name, IconSize::Tiny, x, y, *tint);
        }
        Visual::Image(img) => {
            let (x, y) = r.centre(img.w, img.h);
            c.image(x, y, img);
        }
        Visual::Letter { ch, tint } => {
            let mut buf = [0u8; 4];
            let up = ch.to_uppercase().next().unwrap_or(*ch);
            text_centre(c, t.f.small_bold, r, up.encode_utf8(&mut buf), *tint);
        }
    }
}

/// Battery outline with fill and percentage, right-aligned to `right`.
/// Returns its left edge.
fn battery(c: &mut Canvas, t: &Theme, right: i32, r: Rect, b: Battery) -> i32 {
    let label = Percent::new(b.percent);
    let low = !b.charging && b.percent <= 5;
    let ink = if low { t.c.critical } else { t.c.fg };
    let x = text_right(c, t.f.small, right, r, label.as_str(), ink) - 4;
    let (w, h) = (BATTERY_W, BATTERY_H);
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
    c.fill(Rect::new(bx + w, by + (h - 4) / 2, 2, 4), t.c.muted);
    let inner = w - 4;
    let level = ((inner * b.percent.min(100) as i32) + 50) / 100;
    c.fill(Rect::new(bx + 2, by + 2, level.max(1), h - 4), fill);
    if b.charging {
        let (ox, oy) = (bx + (w - BOLT_W) / 2, by + 1);
        // A background ring parts the bolt from the fill. It stays inside
        // the outline.
        let room = Rect::new(bx + 1, by + 1, w - 2, h - 2);
        for (i, _) in BOLT.iter().enumerate().filter(|(_, a)| **a != 0) {
            let (x, y) = (ox + i as i32 % BOLT_W, oy + i as i32 / BOLT_W);
            c.fill(Rect::new(x - 1, y - 1, 3, 3).intersect(&room), t.c.bg);
        }
        c.mask8(ox, oy, BOLT_W, BOLT_H, &BOLT, t.c.fg);
    }
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

/// A count of up to 9, then "9+", without allocating.
struct Count {
    buf: [u8; 2],
    len: usize,
}

impl Count {
    fn new(n: u8) -> Count {
        if n > 9 {
            Count {
                buf: *b"9+",
                len: 2,
            }
        } else {
            Count {
                buf: [b'0' + n, 0],
                len: 1,
            }
        }
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

/// The bar while a field has focus, when digits type and no chip can run:
/// the keys that still work, in pills, with what they do. Enter's pill is
/// filled, as a main chip is. `right` goes on the far side while it fits:
/// "Fn h help".
pub fn hint_bar(c: &mut Canvas, t: &Theme, hints: &[Hint], right: Option<Hint>) {
    let r = hint_rect(t);
    c.clear(r, t.c.bg);
    let y = r.y + (r.h - t.m.pill_h) / 2;
    let end = key_row(c, t, t.m.pad, y, hints);
    if let Some(h) = right {
        let lw = t.f.small.measure(h.label);
        let lx = r.right() - t.m.pad - lw;
        let kx = lx - 4 - pills_w(t, h.key);
        if kx >= end + 12 {
            pills(c, t, kx, y, h.key, Pill::Normal);
            text_in(
                c,
                t.f.small,
                Rect::new(lx, y, lw, t.m.pill_h),
                h.label,
                t.c.muted,
            );
        }
    }
}

/// Key pills with what they do, from `x`, top at `y`: in the hint bar, in a
/// card, under a message. Enter's pill is filled. Returns the right edge.
pub fn key_row(c: &mut Canvas, t: &Theme, x: i32, y: i32, hints: &[Hint]) -> i32 {
    let mut x = x;
    for (i, h) in hints.iter().enumerate() {
        if i > 0 {
            x += 12;
        }
        let style = if h.key == "Enter" {
            Pill::Main
        } else {
            Pill::Normal
        };
        x = pills(c, t, x, y, h.key, style) + 4;
        let lr = Rect::new(x, y, t.f.small.measure(h.label), t.m.pill_h);
        x = text_in(c, t.f.small, lr, h.label, t.c.muted);
    }
    x
}

/// The width `key_row` draws.
pub fn key_row_w(t: &Theme, hints: &[Hint]) -> i32 {
    hints
        .iter()
        .map(|h| pills_w(t, h.key) + 4 + t.f.small.measure(h.label))
        .sum::<i32>()
        + 12 * (hints.len() as i32 - 1).max(0)
}

/// How a chip looks. docs/design.md says when each is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Normal,
    /// The page's main action when it is not on Enter.
    Main,
    /// A toggle that is on.
    On,
    Danger,
    /// Danger, armed: filled.
    DangerMain,
    /// Can't be used now: the key does nothing.
    Disabled,
    /// While its key is down.
    Pressed,
    /// A main or on chip while its key is down: as Pressed, keeping its
    /// accent word.
    MainPressed,
    /// Held to confirm, filling red: 0 to 100.
    Hold(u8),
    /// Working: no second press.
    Busy,
}

/// An action chip over one of keys 4 to 8: an icon, one word, or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chip<'a> {
    pub icon: &'a str,
    pub word: &'a str,
    pub look: Look,
}

impl Chip<'_> {
    pub fn is_empty(&self) -> bool {
        self.icon.is_empty() && self.word.is_empty()
    }
}

/// Icon, gap, word.
fn chip_ink_w(t: &Theme, ch: &Chip) -> i32 {
    let busy = if ch.look == Look::Busy {
        t.f.chip.measure("\u{2026}")
    } else {
        0
    };
    let word = if ch.word.is_empty() {
        0
    } else {
        t.f.chip.measure(ch.word) + busy
    };
    // A word stands alone; only a chip with none shows its icon.
    if ch.word.is_empty() && !ch.icon.is_empty() {
        IconSize::Tiny as i32
    } else {
        word
    }
}

/// A chip's natural width: its ink with even padding.
pub fn chip_w(t: &Theme, ch: &Chip) -> i32 {
    chip_ink_w(t, ch) + 2 * t.m.chip_pad
}

/// Widths for a bar: every chip as wide as its key, less a gap each side,
/// so neighbouring pills read apart.
pub fn bar_widths(t: &Theme, chips: &[Chip; 5]) -> [i32; 5] {
    let key = t.m.softkey_x[1] - t.m.softkey_x[0] - 2 * t.m.chip_inset;
    chips.map(|ch| if ch.is_empty() { 0 } else { key })
}

/// Draws the icon in `icon_ink` and the word in `ink`, centred together.
fn chip_ink(c: &mut Canvas, t: &Theme, r: Rect, ch: &Chip, ink: Color, icon_ink: Color) {
    let busy;
    let word = if ch.look == Look::Busy {
        busy = format!("{}\u{2026}", ch.word);
        busy.as_str()
    } else {
        ch.word
    };
    let inner = r;
    // Centred, so the padding is even on both sides.
    let x = inner.x + (inner.w - chip_ink_w(t, ch)) / 2;
    let s = IconSize::Tiny as i32;
    // A row lower than centred: Lucide's ink starts a row above the
    // capitals and ends on their baseline.
    let iy = inner.y + (inner.h - s) / 2 + 1;
    if word.is_empty() {
        icon::draw(c, ch.icon, IconSize::Tiny, x, iy, icon_ink);
        return;
    }
    let mut lx = x;
    if !ch.icon.is_empty() {
        icon::draw(c, ch.icon, IconSize::Tiny, x, iy, icon_ink);
        lx += s + 3;
    }
    let tr = Rect::new(lx, inner.y, inner.right() - lx, inner.h);
    text_in(c, t.f.chip, tr, word, ink);
}

/// One chip: its word over a short bar, centred on `cx` over its key, no
/// fill behind. The bar says what the word is: muted for most, the text
/// colour for the main action, red for danger. Pressed, the bar turns
/// accent and nothing moves. `w` is the room it may take.
pub fn chip(c: &mut Canvas, t: &Theme, cx: i32, top: i32, ch: &Chip, w: i32) {
    chip_on(c, t, cx, top, ch, w, false);
}

/// A chip; with `shadow`, its word, icon and bar cast a 1 px shadow, so it
/// reads straight on a picture with nothing behind it.
fn chip_on(c: &mut Canvas, t: &Theme, cx: i32, top: i32, ch: &Chip, w: i32, shadow: bool) {
    // A chip with a word shows the word alone: only a chip with no word
    // shows its icon.
    let bare;
    let ch = if !ch.icon.is_empty() && !ch.word.is_empty() {
        bare = Chip { icon: "", ..*ch };
        &bare
    } else {
        ch
    };
    let k = &t.c;
    let r = Rect::new(cx - w / 2, top, w, t.m.chip_h);
    // Word, icon, bar and its height.
    let quiet = mix(k.bg, k.fg, 110);
    let (ink, icon_ink, bar, bar_h) = match ch.look {
        Look::Normal => (k.fg, k.accent, quiet, 2),
        Look::Main | Look::On => (k.accent, k.accent, k.fg, 2),
        Look::Danger | Look::Hold(_) => (k.critical, k.critical, k.critical, 2),
        Look::DangerMain => (k.critical, k.critical, k.critical, 3),
        Look::Disabled => (k.disabled(), k.disabled(), k.border, 2),
        Look::Pressed => (k.fg, k.accent, k.accent, 2),
        Look::MainPressed => (k.accent, k.accent, k.accent, 2),
        Look::Busy => (k.muted, k.muted, k.muted, 2),
    };
    let foot = top + t.m.chip_h + t.m.chip_foot;
    if shadow {
        let plate = theme::PICTURE_PLATE;
        let sr = Rect::new(r.x + 1, r.y + 1, r.w, r.h);
        chip_ink(c, t, sr, ch, plate, plate);
        let bw = t.m.chip_bar_w;
        c.fill(Rect::new(cx - bw / 2 + 1, foot - 1, bw, 2), plate);
    }
    chip_ink(c, t, r, ch, ink, icon_ink);
    // The bar's foot sits `chip_foot` under the chip, wherever it is drawn.
    let bw = t.m.chip_bar_w;
    if let Look::Hold(pc) = ch.look {
        // Held, the bar runs the chip's width and fills as the hold goes.
        let track = Rect::new(cx - w / 2, foot - bar_h, w, bar_h);
        c.fill(track, k.border);
        let done = w * pc.min(100) as i32 / 100;
        c.fill(Rect::new(track.x, track.y, done, bar_h), k.critical);
        return;
    }
    c.fill(Rect::new(cx - bw / 2, foot - bar_h, bw, bar_h), bar);
}

/// The action bar: a tab over each of keys 4 to 8 that has one, underlined
/// along the screen's bottom edge right above its key. No fill behind them.
pub fn action_bar(c: &mut Canvas, t: &Theme, chips: &[Chip; 5]) {
    lint_chips(chips);
    c.clear(hint_rect(t), t.c.bg);
    action_chips(c, t, chips, false);
}

/// The action bar over a picture: nothing behind the chips, so the
/// picture shows whole. Each chip casts a shadow instead.
pub fn action_bar_over(c: &mut Canvas, t: &Theme, chips: &[Chip; 5]) {
    lint_chips(chips);
    action_chips(c, t, chips, true);
}

/// A dialog's held yes stands alone on 7, as design.md puts it: the
/// placement rule is for a page's own chips.
fn lint_chips(chips: &[Chip; 5]) {
    let on = [1, 2, 3].map(|i| !chips[i].is_empty());
    let held_yes = on == [false, false, true]
        && matches!(
            chips[3].look,
            Look::Danger | Look::DangerMain | Look::Hold(_)
        );
    if !held_yes {
        crate::lint::chips(on);
    }
}

fn action_chips(c: &mut Canvas, t: &Theme, chips: &[Chip; 5], shadow: bool) {
    let top = t.m.height - t.m.chip_foot - t.m.chip_h;
    let w = bar_widths(t, chips);
    for (i, ch) in chips.iter().enumerate().filter(|(_, ch)| !ch.is_empty()) {
        chip_on(c, t, t.m.softkey_x[i], top, ch, w[i], shadow);
    }
}

/// The one chip of a full-screen page: over the right rail's foot, on the
/// page's own background there.
pub fn corner_chip(c: &mut Canvas, t: &Theme, ch: &Chip) {
    let w = t.m.rail_w;
    c.fill(
        Rect::new(t.m.width - w, t.m.height - t.m.hint_h, w, t.m.hint_h),
        t.c.bg,
    );
    let top = t.m.height - t.m.chip_foot - t.m.chip_h;
    chip(c, t, t.m.width - w / 2, top, ch, w - 2 * t.m.chip_inset);
}

/// Search replaces the hint bar: the query, a caret, and how to leave.
pub fn search_bar(c: &mut Canvas, t: &Theme, query: &str) {
    let r = hint_rect(t);
    c.clear(r, t.c.bg);
    let field = Rect::new(2, r.y + 1, r.w - 4, r.h - 2);
    c.pane(field, t.m.radius, t.c.surface);
    c.stroke_round_rect(field, t.m.radius, t.c.accent);
    let x = t.m.pad - 1;
    // Centred on the query's capitals, as a chip's icon is on its word.
    let s = IconSize::Small as i32;
    let cap_top = r.y + 1 + (r.h - 1 - t.f.body.cap()) / 2;
    let iy = cap_top + t.f.body.cap() / 2 - s / 2;
    icon::draw(c, "search", IconSize::Small, x, iy, t.c.accent);
    let tx = x + s + 6;
    let right = r.right() - t.m.pad;
    let cancel = Rect::new(0, r.y + 1, 0, r.h - 1);
    let lx = text_right(c, t.f.small, right, cancel, "cancel", t.c.muted);
    let pill = pills_w(t, "Esc");
    pills(
        c,
        t,
        lx - 4 - pill,
        r.y + (r.h - t.m.pill_h + 1) / 2,
        "Esc",
        Pill::Normal,
    );
    let field = Rect::new(tx, r.y + 1, lx - pill - 16 - tx, r.h - 1);
    let end = text_in(c, t.f.body, field, query, t.c.fg);
    c.fill(Rect::new(end + 2, r.y + 4, 1, r.h - 7), t.c.accent);
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
    /// A system app's symbol in `accent` on the neutral badge.
    System { icon: &'a str },
    /// A baked icon on a tinted badge.
    Badge { icon: &'a str, tint: Color },
    /// An app's own PNG, already scaled.
    Image(&'a Image),
    /// No icon at all: the name's first letter on a tinted badge.
    Letter { ch: char, tint: Color },
}

/// The badge under an app's own picture and a system app's symbol: theme
/// colours, never a tint.
pub fn neutral_fill(t: &Theme, selected: bool) -> Color {
    if selected {
        mix(t.c.surface, t.c.fg, 36)
    } else {
        mix(t.c.surface, t.c.fg, 16)
    }
}

/// A raised square: flat colour standing `lift` px up on a darker foot.
fn raised(c: &mut Canvas, r: Rect, radius: i32, fill: Color, lift: i32) {
    if lift > 0 {
        let under = Rect::new(r.x, r.y + lift, r.w, r.h);
        c.round_rect(under, radius, theme::foot(fill));
    }
    c.round_rect(r, radius, fill);
}

pub fn badge(c: &mut Canvas, t: &Theme, r: Rect, v: &Visual, selected: bool) {
    // A row card is a badge too, at its own size.
    let card = r.w >= t.m.row_card_side;
    let (rad, lift) = if card {
        // Scaled with the card, so a side card rounds like the middle one.
        (t.m.row_card_r * r.w / t.m.row_card, t.m.lift)
    } else if r.w >= 32 {
        (t.m.badge_r, t.m.lift)
    } else {
        (t.m.radius, t.m.chip_lift)
    };
    let symbol = |c: &mut Canvas, name: &str, fill: Color, ink: Color| {
        raised(c, r, rad, fill, lift);
        let size = if card {
            IconSize::Large
        } else if r.w >= 32 {
            IconSize::Medium
        } else {
            IconSize::Bar
        };
        let s = size as i32;
        let (x, y) = r.centre(s, s);
        icon::draw(c, name, size, x, y, ink);
    };
    match v {
        Visual::System { icon: name } => {
            symbol(c, name, neutral_fill(t, selected), t.c.accent);
        }
        Visual::Badge { icon: name, tint } => {
            symbol(c, name, *tint, theme::on_tint(*tint));
        }
        Visual::Letter { ch, tint } => {
            raised(c, r, rad, *tint, lift);
            let f = if r.w >= 32 { t.f.title } else { t.f.body_bold };
            let f = if card { t.f.big } else { f };
            let mut buf = [0u8; 4];
            let up = ch.to_uppercase().next().unwrap_or(*ch);
            text_centre(c, f, r, up.encode_utf8(&mut buf), theme::on_tint(*tint));
        }
        Visual::Image(img) => {
            raised(c, r, rad, neutral_fill(t, selected), lift);
            let (x, y) = r.centre(img.w, img.h);
            c.image(x, y, img);
        }
    }
}

/// The selection behind a tile, an app row or a switcher card: a pane
/// standing on a foot, as a badge does.
pub fn selection(c: &mut Canvas, t: &Theme, r: Rect) {
    let under = Rect::new(r.x, r.y + t.m.lift, r.w, r.h);
    c.round_rect(under, t.m.radius_large, theme::foot(t.c.surface));
    c.pane(r, t.m.radius_large, t.c.surface);
    // The frosted pane alone is faint over a bright wallpaper.
    c.stroke_round_rect(r, t.m.radius_large, t.c.edge());
}

/// What a grid tile or list row shows for one app.
pub struct AppItem<'a> {
    pub label: &'a str,
    /// The list layout's second line. Ignored by tiles.
    pub note: &'a str,
    pub visual: Visual<'a>,
    pub pinned: bool,
    /// A number on the badge's corner, from the app's notice: "12" updates.
    /// 0 draws none.
    pub count: u32,
    /// A word on the right of a list row: "Update", "45%". Tiles ignore it.
    pub tag: Option<(&'a str, Tone)>,
}

/// One grid cell: badge on top, name below, both centred, a pin mark in the
/// corner when pinned.
pub fn tile(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    if selected {
        selection(c, t, r);
    }
    tile_content(c, t, r, item, selected);
}

/// A tile without its selection frame, for when the frame is animating.
pub fn tile_content(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    // Badge and name as one block, with even room from the frame to the
    // badge and from the name's baseline to the frame; descenders use the
    // room below.
    let b = t.m.badge;
    let f = if selected { t.f.small_bold } else { t.f.small };
    let gap = 5;
    let block = b + gap + f.cap();
    let top = r.y + 1 + (r.h - 2 - block) / 2;
    let br = Rect::new(r.x + (r.w - b) / 2, top, b, b);
    badge(c, t, br, &item.visual, selected);
    count_badge(c, t, br, item.count);
    if item.pinned {
        let s = IconSize::Tiny as i32;
        let col = if selected { t.c.accent } else { t.c.muted };
        icon::draw(c, "pin", IconSize::Tiny, r.right() - 5 - s, r.y + 4, col);
    }
    let col = if selected { t.c.fg } else { soft(t) };
    let w = f.measure(item.label).min(r.w - 8);
    let y = br.bottom() + gap + f.cap();
    f.draw_elided(c, r.x + (r.w - w) / 2, y, item.label, r.w - 8, col);
}

/// A count on a badge's top-right corner, in accent, ringed in the
/// background so it reads over any badge. Nothing for 0; past 99, "99+".
pub fn count_badge(c: &mut Canvas, t: &Theme, badge: Rect, n: u32) {
    if n == 0 {
        return;
    }
    let text = if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    };
    let w = (t.f.small_bold.measure(&text) + 6).max(13);
    let r = Rect::new(badge.right() - w + 4, badge.y - 4, w, 13);
    c.round_rect(r.inset(-2), 8, t.c.bg);
    c.round_rect(r, 7, t.c.accent);
    text_centre(c, t.f.small_bold, r, &text, t.c.accent_fg);
}

/// A notice over whatever is in front: the app's picture and name, then
/// what it says, in a card edged in accent above `bottom`. Returns where
/// it went.
pub fn notice_toast(
    c: &mut Canvas,
    t: &Theme,
    bottom: i32,
    visual: &Visual,
    app: &str,
    title: &str,
) -> Rect {
    let s = IconSize::Tiny as i32;
    let aw = t.f.small_bold.measure(app);
    let tw = t.f.small.measure(title);
    let w = (8 + s + 5 + aw + 6 + tw + 10).min(t.m.width - 2 * t.m.margin);
    let h = t.m.toast_h;
    let r = Rect::new((t.m.width - w) / 2, bottom - h - 4, w, h);
    lifted(c, t, r, t.m.radius_large);
    slot_visual(c, t, Rect::new(r.x + 8, r.y + (h - s) / 2, s, s), visual);
    let x = r.x + 8 + s + 5;
    let x = text_in(
        c,
        t.f.small_bold,
        Rect::new(x, r.y, aw.min(r.right() - 8 - x), h),
        app,
        t.c.fg,
    ) + 6;
    if x < r.right() - 8 {
        text_in(
            c,
            t.f.small,
            Rect::new(x, r.y, r.right() - 8 - x, h),
            title,
            soft(t),
        );
    }
    r
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
}

/// A list row without its selection frame.
pub fn app_row_content(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, selected: bool) {
    let b = t.m.badge_small;
    let br = Rect::new(r.x + 6, r.y + (r.h - b) / 2, b, b);
    badge(c, t, br, &item.visual, selected);
    count_badge(c, t, br, item.count);
    let x = br.right() + 10;
    let mut right = r.right() - 10;
    if let Some((word, tone)) = item.tag {
        right = tag(c, t, right, r, word, tone) - 8;
    }
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
        let (y1, y2) = two_lines(t, r, name);
        name.draw_elided(c, x, y1, item.label, right - x, col);
        t.f.small
            .draw_elided(c, x, y2, item.note, right - x, t.c.muted);
    }
}

/// Baselines for a label and a Caption note under it, centred as one block
/// on their capitals. The gap leaves room for the label's descenders.
fn two_lines(t: &Theme, r: Rect, label: Font) -> (i32, i32) {
    let gap = 6;
    let block = label.cap() + gap + t.f.small.cap();
    let top = r.y + (r.h - block) / 2;
    (top + label.cap(), top + block)
}

/// A switcher card: a large badge, the name, and one short line under it.
pub struct Card<'a> {
    pub label: &'a str,
    /// A state worth words, such as closing. Drawn instead of `stats`.
    pub note: &'a str,
    /// Memory and CPU, in small mono figures. `Some("")` keeps the line's
    /// room, so the card does not move when they arrive.
    pub stats: Option<&'a str>,
    pub visual: Visual<'a>,
    /// A pause mark on the badge's corner.
    pub paused: bool,
}

/// A card without its selection frame: a thin edge so it reads as a card,
/// then its content.
pub fn card_content(c: &mut Canvas, t: &Theme, r: Rect, card: &Card, selected: bool) {
    // Only the selected card stands on a pane, as on home: the rest sit on
    // the page, so the one in front of you is the one that stands out.
    let b = t.m.badge;
    // Badge, name and line as one block, centred in the card.
    let has_line = !card.note.is_empty() || card.stats.is_some();
    let line_h = if has_line { 14 } else { 0 };
    let block = b + 6 + 18 + line_h;
    let br = Rect::new(r.x + (r.w - b) / 2, r.y + (r.h - block) / 2, b, b);
    badge(c, t, br, &card.visual, selected);
    if card.paused {
        let (under, ink) = if selected {
            (t.c.surface, t.c.fg)
        } else {
            (t.c.bg, soft(t))
        };
        pause_mark(c, t, br, under, ink);
    }
    let f = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { soft(t) };
    let lr = Rect::new(r.x + 4, br.bottom() + 6, r.w - 8, 18);
    text_centre(c, f, lr, card.label, col);
    let nr = Rect::new(r.x + 4, lr.bottom(), r.w - 8, line_h);
    if !card.note.is_empty() {
        text_centre(c, t.f.small, nr, card.note, t.c.muted);
    } else if let Some(stats) = card.stats {
        text_centre(c, t.f.mono, nr, stats, t.c.muted);
    }
}

/// Two bars over the badge's bottom-right corner, outlined in `under` so
/// they read over any badge. Drawn, not baked: no icon to keep in memory.
fn pause_mark(c: &mut Canvas, t: &Theme, badge: Rect, under: Color, ink: Color) {
    let (w, h, gap) = (3, 10, 2);
    let pair = Rect::new(badge.right() - 6, badge.bottom() - 8, 2 * w + gap, h);
    c.round_rect(pair.inset(-2), t.m.radius_small, under);
    for dx in [0, w + gap] {
        c.round_rect(Rect::new(pair.x + dx, pair.y, w, h), 1, ink);
    }
}

/// A share of something, such as CPU or memory: the label and value on one
/// line in mono, a thin bar under them. `share` runs from 0 to 1. Keep the
/// text ASCII: mono is not otherwise drawn by the shell, and ASCII touches
/// the fewest pages of its tables.
pub fn meter(c: &mut Canvas, t: &Theme, r: Rect, label: &str, value: &str, share: f32) {
    let f = t.f.mono;
    let bar = Rect::new(r.x, r.bottom() - 3, r.w, 3);
    let line = Rect::new(r.x, r.y, r.w, bar.y - 2 - r.y);
    text_in(c, f, line, label, t.c.muted);
    text_right(c, f, r.right(), line, value, soft(t));
    c.round_rect(bar, 1, t.c.border);
    let fill = (bar.w as f32 * share.clamp(0.0, 1.0)).round() as i32;
    if fill > 0 {
        c.round_rect(Rect::new(bar.x, bar.y, fill, bar.h), 1, t.c.accent);
    }
}

#[derive(Clone, Copy)]
pub enum Trailing<'a> {
    None,
    Chevron,
    Value(&'a str),
    ValueChevron(&'a str),
    Toggle(bool),
    /// A word in the warning colour: "Enter again" on a row that asks first.
    Alert(&'a str),
    /// A setting with a few values, changed with left and right.
    Choice(&'a str),
    /// A 0-100 level, changed with left and right: a short bar and a percent.
    Level(u8),
    /// Wi-Fi: signal bars and a lock for a secured network.
    Signal {
        level: u8,
        secure: bool,
    },
    /// The one picked, in a list of choices.
    Check,
    /// A count or a word in a tone: "3", "New".
    Tag(&'a str, Tone),
}

#[derive(Clone, Copy)]
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
        row_selection(c, t, r);
    }
    row_body(c, t, r, row, selected, false, true);
}

/// A row whose icon stands alone in its tint, with no tile behind it: for
/// long lists of files, where a tile on every row is noise.
pub fn plain_row(c: &mut Canvas, t: &Theme, r: Rect, row: &Row, selected: bool) {
    if selected {
        row_selection(c, t, r);
    }
    // In the tile's column, so labels line up with tiled rows.
    if let Some(name) = row.icon.filter(|n| !n.is_empty()) {
        let s = IconSize::Bar as i32;
        let col = Rect::new(r.x + 4, r.y, t.m.row_icon, r.h);
        let (ix, iy) = col.centre(s, s);
        icon::draw(c, name, IconSize::Bar, ix, iy, row.tint);
    }
    let body = Row {
        icon: row.icon.map(|_| ""),
        ..*row
    };
    row_body(c, t, r, &body, selected, false, true);
}

/// A selected row: `surface` standing on a 1 px foot.
fn row_selection(c: &mut Canvas, t: &Theme, r: Rect) {
    let under = Rect::new(r.x, r.y + t.m.chip_lift, r.w, r.h);
    c.round_rect(under, t.m.radius, theme::foot(t.c.surface));
    c.round_rect(r, t.m.radius, t.c.surface);
}

/// A row picked for an action on many: a pale accent wash. Drawn before the
/// row, under the selection when it is also selected.
pub fn marked_row(c: &mut Canvas, t: &Theme, r: Rect) {
    c.round_rect(r, t.m.radius, mix(t.c.bg, t.c.accent, 48));
}

/// A selected menu row: an accent wash, as the menu is itself a surface.
fn menu_selection(c: &mut Canvas, t: &Theme, r: Rect) {
    c.round_rect(r, t.m.radius, mix(t.c.surface, t.c.accent, 70));
}

/// A row of an app menu: a list row with its keys in pills on the right,
/// in a column `keys_w` wide so values line up. A `dim` row can't be used
/// now.
#[allow(clippy::too_many_arguments)]
pub fn menu_row(
    c: &mut Canvas,
    t: &Theme,
    r: Rect,
    row: &Row,
    keys: &str,
    keys_w: i32,
    selected: bool,
    dim: bool,
) {
    if selected {
        menu_selection(c, t, r);
    }
    let right = r.right() - 8;
    if !keys.is_empty() {
        let w = pills_w(t, keys);
        let style = if dim { Pill::Dim } else { Pill::Normal };
        pills(c, t, right - w, r.y + (r.h - t.m.pill_h) / 2, keys, style);
    }
    let gap = if keys_w > 0 { keys_w + 6 } else { 0 };
    let body = Rect::new(r.x, r.y, r.w - gap, r.h);
    row_body(c, t, body, row, selected, dim, false);
}

/// How a key pill is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pill {
    Normal,
    /// The row can't be used now.
    Dim,
    /// The key that does the main thing: Enter.
    Main,
}

/// The width of `keys` as pills: one pill per space-separated key.
pub fn pills_w(t: &Theme, keys: &str) -> i32 {
    let mut w = 0;
    for k in keys.split(' ').filter(|k| !k.is_empty()) {
        if w > 0 {
            w += 3;
        }
        w += pill_w(t, k);
    }
    w
}

fn pill_w(t: &Theme, key: &str) -> i32 {
    (t.f.small_bold.measure(key) + 10).max(t.m.pill_h)
}

/// Draws `keys` as pills from `x`, top at `y`. Returns the right edge.
pub fn pills(c: &mut Canvas, t: &Theme, x: i32, y: i32, keys: &str, style: Pill) -> i32 {
    let (fill, ink) = match style {
        Pill::Normal => (t.c.border, t.c.fg),
        Pill::Dim => (t.c.border, t.c.muted),
        Pill::Main => (t.c.accent, t.c.accent_fg),
    };
    let mut x = x;
    for k in keys.split(' ').filter(|k| !k.is_empty()) {
        let pill = Rect::new(x, y, pill_w(t, k), t.m.pill_h);
        // A keycap: flat, standing on its foot.
        let pill = Rect::new(pill.x, pill.y, pill.w, pill.h - t.m.chip_lift);
        raised(c, pill, t.m.radius_small, fill, t.m.chip_lift);
        text_centre(
            c,
            t.f.small_bold,
            Rect::new(pill.x, pill.y, pill.w, pill.h - 1),
            k,
            ink,
        );
        x = pill.right() + 3;
    }
    x - 3
}

/// A line of a Keys screen: the keys in pills, what they do, and a value on
/// the right. Labels start `keys_w` in, so they line up, unless the keys
/// need more room.
pub fn keys_line(
    c: &mut Canvas,
    t: &Theme,
    r: Rect,
    keys: &str,
    keys_w: i32,
    label: &str,
    value: &str,
) {
    let x = r.x + 9;
    let y = r.y + (r.h - t.m.pill_h) / 2;
    // Keys wider than the column push their label right, never under it.
    let end = pills(c, t, x, y, keys, Pill::Normal);
    let mut right = r.right() - 8;
    if !value.is_empty() {
        right = text_right(c, t.f.small, right, r, value, t.c.muted) - 8;
    }
    let lx = (x + keys_w).max(end) + 8;
    text_in(
        c,
        t.f.small,
        Rect::new(lx, r.y, right - lx, r.h),
        label,
        t.c.fg,
    );
}

/// A settings row's icon on a lit square of `fill`, at the row's left, or
/// just its room when `name` is empty. Returns where the label starts.
fn icon_tile(c: &mut Canvas, t: &Theme, r: Rect, name: &str, fill: Color, ink: Color) -> i32 {
    let s = t.m.row_icon;
    let sq = Rect::new(r.x + 4, r.y + (r.h - s) / 2, s, s);
    if !name.is_empty() {
        raised(c, sq, t.m.radius, fill, t.m.chip_lift);
        let i = IconSize::Tiny as i32;
        let (ix, iy) = sq.centre(i, i);
        icon::draw(c, name, IconSize::Tiny, ix, iy, ink);
    }
    sq.right() + 9
}

/// A row's icon, label and what is on its right. With `tile`, the icon
/// sits on a lit square of its tint, as a settings row's does.
fn row_body(c: &mut Canvas, t: &Theme, r: Rect, row: &Row, selected: bool, dim: bool, tile: bool) {
    let mut x = r.x + 9;
    if let Some(name) = row.icon {
        if tile {
            x = icon_tile(c, t, r, name, row.tint, theme::on_tint(row.tint));
        } else {
            let s = IconSize::Small as i32;
            icon::draw(c, name, IconSize::Small, x, r.y + (r.h - s) / 2, row.tint);
            x += s + 9;
        }
    }
    let mut right = r.right() - 8;
    let muted = t.c.muted;
    // A value gives way to the label: the label keeps its width, and a
    // value too long for the rest is cut, down to a floor.
    let label_f = if selected { t.f.body_bold } else { t.f.body };
    let value_max = |right: i32| (right - x - label_f.measure(row.label) - 12).max(48);
    match row.trailing {
        Trailing::None => {}
        Trailing::Chevron => {
            right = chevron(c, right, r, muted);
        }
        Trailing::Value(v) => {
            right = value_right(c, t.f.small, right, r, v, muted, value_max(right)) - 8;
        }
        Trailing::ValueChevron(v) => {
            right = chevron(c, right, r, muted) - 3;
            right = value_right(c, t.f.small, right, r, v, muted, value_max(right)) - 8;
        }
        Trailing::Toggle(on) => {
            right = toggle(c, t, right, r, on) - 8;
        }
        Trailing::Alert(v) => {
            right = text_right(c, t.f.small_bold, right, r, v, t.c.warning) - 8;
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
        Trailing::Check => {
            let s = IconSize::Small as i32;
            icon::draw(
                c,
                "check",
                IconSize::Small,
                right - s,
                r.y + (r.h - s) / 2,
                t.c.accent,
            );
            right -= s + 8;
        }
        Trailing::Tag(word, tone) => {
            right = tag(c, t, right, r, word, tone) - 8;
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
    let col = match (dim, selected) {
        (true, _) => muted,
        (false, true) => t.c.fg,
        (false, false) => soft(t),
    };
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

/// A QR code for a phone camera on the left of `r`, as large as it allows,
/// and labelled values beside it for when there is no camera. Values are in
/// mono, so `l` and `1` read apart. False when the text is too long.
pub fn qr_card(c: &mut Canvas, t: &Theme, r: Rect, text: &str, lines: &[(&str, &str)]) -> bool {
    let Some(code) = crate::qr::Qr::new(text) else {
        return false;
    };
    let (_, side) = code.fit(r.h - 2 * t.m.margin_v);
    let x = r.x + t.m.margin + 3;
    code.draw(c, x, r.y + (r.h - side) / 2, side);
    let tx = x + side + 12;
    let tw = r.right() - t.m.margin - 3 - tx;
    let value_h = 18;
    let pair_h = t.m.field_label_h + value_h;
    let mut y = r.y + (r.h - lines.len() as i32 * pair_h) / 2;
    for (label, value) in lines {
        text_in(
            c,
            t.f.small,
            Rect::new(tx, y, tw, t.m.field_label_h),
            label,
            t.c.muted,
        );
        let vr = Rect::new(tx, y + t.m.field_label_h, tw, value_h);
        text_in(c, t.f.mono, vr, value, t.c.fg);
        y += pair_h;
    }
    true
}

/// Label on the left in sans, value on the right in mono: a readout.
pub fn readout(c: &mut Canvas, t: &Theme, r: Rect, label: &str, value: &str) {
    let x = r.x + 9;
    text_in(c, t.f.small, Rect::new(x, r.y, 76, r.h), label, t.c.muted);
    let vx = x + 78;
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

/// The selected tile of a grid: a ring in `accent` just outside it, two
/// pixels thick, so the picture inside stays whole.
pub fn tile_focus(c: &mut Canvas, t: &Theme, r: Rect) {
    let o = Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4);
    c.fill(Rect::new(o.x, o.y, o.w, 2), t.c.accent);
    c.fill(Rect::new(o.x, o.bottom() - 2, o.w, 2), t.c.accent);
    c.fill(Rect::new(o.x, o.y, 2, o.h), t.c.accent);
    c.fill(Rect::new(o.right() - 2, o.y, 2, o.h), t.c.accent);
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
    // Over a picture, a pane; on a card or a page, a well of `bg`.
    if c.has_backdrop() {
        c.pane(r, t.m.radius, t.c.surface);
    } else {
        c.round_rect(r, t.m.radius, t.c.bg);
    }
    let edge = if cursor { t.c.accent } else { t.c.border };
    c.stroke_round_rect(r, t.m.radius, edge);
    let inner = Rect::new(r.x + 8, r.y, r.w - 16, r.h);
    let f = t.f.body;
    let end = if text.is_empty() {
        text_in(c, f, inner, placeholder, t.c.muted);
        inner.x
    } else if masked {
        // As many dots as fit: a long password doesn't run off the field.
        let n = text.chars().count().min((inner.w / 8).max(0) as usize);
        let mut x = inner.x;
        for _ in 0..n {
            c.circle(x as f32 + 3.0, (r.y + r.h / 2) as f32, 2.5, t.c.fg);
            x += 8;
        }
        x
    } else if cursor && f.measure(text) > inner.w {
        // Typing goes on at the end, so a long text shows its tail.
        let tail = tail_fit(f, text, inner.w - f.measure("\u{2026}"));
        text_in(c, f, inner, &format!("\u{2026}{tail}"), t.c.fg)
    } else {
        text_in(c, f, inner, text, t.c.fg)
    };
    if cursor {
        c.fill(Rect::new(end + 1, r.y + 5, 1, r.h - 10), t.c.accent);
    }
}

/// The longest end of `s` that fits in `w`.
pub fn tail_fit(f: Font, s: &str, w: i32) -> &str {
    let mut start = s.len();
    for (i, _) in s.char_indices().rev() {
        if f.measure(&s[i..]) > w {
            break;
        }
        start = i;
    }
    &s[start..]
}

/// A form field as [`form_field`] draws it.
pub struct Field<'a> {
    pub label: &'a str,
    pub text: &'a str,
    pub placeholder: &'a str,
    pub masked: bool,
    pub focused: bool,
    /// What is wrong with it: a red edge, and this line under it.
    pub error: Option<&'a str>,
}

/// A labelled text field from `r`'s top, `r.w` wide: the label in Caption,
/// the field, and its error line. Returns the height it took.
pub fn form_field(c: &mut Canvas, t: &Theme, r: Rect, f: &Field) -> i32 {
    let lr = Rect::new(r.x + 3, r.y, r.w - 6, t.m.field_label_h - 1);
    text_in(c, t.f.small, lr, f.label, t.c.muted);
    let fr = Rect::new(r.x, r.y + t.m.field_label_h, r.w, t.m.field_h);
    text_field(c, t, fr, f.text, f.masked, f.focused, f.placeholder);
    let Some(why) = f.error else {
        return fr.bottom() - r.y;
    };
    field_error(c, t, fr, why) - r.y
}

/// A passing word under the field at `fr`, such as "Checking…": muted,
/// the field's edge left alone. Nothing is wrong yet. Returns that line's
/// foot.
pub fn field_note(c: &mut Canvas, t: &Theme, fr: Rect, note: &str) -> i32 {
    let nr = Rect::new(fr.x + 3, fr.bottom() + 2, fr.w - 6, t.m.field_note_h);
    text_in(c, t.f.small, nr, note, t.c.muted);
    nr.bottom()
}

/// What is wrong with the field at `fr`: a red edge, and `why` on a line
/// under it. Returns that line's foot.
pub fn field_error(c: &mut Canvas, t: &Theme, fr: Rect, why: &str) -> i32 {
    c.stroke_round_rect(fr, t.m.radius, t.c.critical);
    let s = IconSize::Tiny as i32;
    let nr = Rect::new(fr.x + 3, fr.bottom() + 2, fr.w - 6, t.m.field_note_h);
    icon::draw(
        c,
        "triangle-alert",
        IconSize::Tiny,
        nr.x,
        nr.y + (nr.h - s) / 2,
        t.c.critical,
    );
    let tr = Rect::new(nr.x + s + 4, nr.y, nr.w - s - 4, nr.h);
    text_in(c, t.f.small, tr, why, t.c.critical);
    nr.bottom()
}

/// Dims `r` behind a card. Give it the page's area, not the bars: the
/// chips and the top bar stay readable.
pub fn scrim(c: &mut Canvas, t: &Theme, r: Rect) {
    c.fill_alpha(r, t.c.bg, 160);
}

/// A card with a title, edged in `edge`: `accent`, or `critical` for one
/// that loses data. Returns the body.
pub fn card(c: &mut Canvas, t: &Theme, r: Rect, title: &str, edge: Color) -> Rect {
    lifted(c, t, r, t.m.card_r);
    c.stroke_round_rect(r, t.m.card_r, mix(t.c.surface, edge, 120));
    let head = card_head(t, r);
    text_in(c, t.f.body_bold, head, title, t.c.fg);
    let y = head.bottom() + t.m.card_gap;
    let i = t.m.card_inset;
    Rect::new(r.x + i, y, r.w - 2 * i, r.bottom() - 1 - t.m.card_pad - y)
}

/// A card's ground: a soft shadow around it, a lit surface, a bright rim.
fn lifted(c: &mut Canvas, t: &Theme, r: Rect, rad: i32) {
    raised(c, r, rad, t.c.surface, t.m.lift);
}

/// The title's line, placed so its capitals start `card_pad` below the edge.
fn card_head(t: &Theme, r: Rect) -> Rect {
    let lead = (18 - t.f.body_bold.cap()) / 2;
    let i = t.m.card_inset;
    Rect::new(r.x + i, r.y + 1 + t.m.card_pad - lead, r.w - 2 * i, 18)
}

/// How tall a card is for a body `body_h` high: the same space from the
/// edge to the title's capitals as from the body's foot to the edge.
pub fn card_h(t: &Theme, body_h: i32) -> i32 {
    let lead = (18 - t.f.body_bold.cap()) / 2;
    1 + t.m.card_pad - lead + 18 + t.m.card_gap + body_h + t.m.card_pad + 1
}

/// A short line in a card that floats above `bottom`, with its tone's icon.
/// `held` fills a bar along its foot, 0 to 100, for a key that must be held.
/// Returns where it went.
pub fn toast(
    c: &mut Canvas,
    t: &Theme,
    bottom: i32,
    tone: Tone,
    text: &str,
    held: Option<u8>,
) -> Rect {
    let s = IconSize::Tiny as i32;
    let w = (8 + s + 5 + t.f.small_bold.measure(text) + 10).min(t.m.width - 2 * t.m.margin);
    let h = t.m.toast_h;
    let r = Rect::new((t.m.width - w) / 2, bottom - h - 4, w, h);
    let col = t.c.tone(tone);
    lifted(c, t, r, t.m.radius_large);
    icon::draw(
        c,
        tone.icon(),
        IconSize::Tiny,
        r.x + 8,
        r.y + (h - s) / 2,
        col,
    );
    let tx = r.x + 8 + s + 5;
    let tr = Rect::new(tx, r.y, r.right() - 8 - tx, h);
    text_in(c, t.f.small_bold, tr, text, t.c.fg);
    if let Some(pc) = held {
        let bar = Rect::new(r.x + 4, r.bottom() - 4, r.w - 8, 2);
        c.fill(bar, t.c.border);
        c.fill(
            Rect::new(bar.x, bar.y, bar.w * pc.min(100) as i32 / 100, 2),
            col,
        );
    }
    r
}

/// The shell's urgent word over any app, in its tone's colour. It takes the
/// top bar's place, so the page under it neither moves nor hides.
pub fn banner(c: &mut Canvas, t: &Theme, text: &str, tone: Tone) -> Rect {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.fill(r, t.c.tone(tone));
    let s = IconSize::Tiny as i32;
    let w = (s + 5 + t.f.small_bold.measure(text)).min(r.w - 2 * t.m.pad);
    let x = (r.w - w) / 2;
    icon::draw(c, tone.icon(), IconSize::Tiny, x, (r.h - 1 - s) / 2, t.c.bg);
    let tr = Rect::new(x + s + 5, 0, w - s - 5, r.h - 1);
    text_in(c, t.f.small_bold, tr, text, t.c.bg);
    r
}

/// The volume or brightness level in the top bar's place.
pub fn level_bar(c: &mut Canvas, t: &Theme, icon_name: &str, label: &str, level: u8) -> Rect {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.bottom() - 1, t.c.accent);
    let inner = Rect::new(t.m.pad, 0, r.w - 2 * t.m.pad, r.h - 1);
    let s = IconSize::Tiny as i32;
    icon::draw(
        c,
        icon_name,
        IconSize::Tiny,
        inner.x,
        (inner.h - s) / 2,
        t.c.accent,
    );
    let lr = Rect::new(inner.x + s + 6, 0, t.f.small_bold.measure(label), inner.h);
    let lx = text_in(c, t.f.small_bold, lr, label, t.c.fg);
    let p = Percent::new(level);
    let px = text_right(c, t.f.small, inner.right(), inner, p.as_str(), t.c.fg);
    let bar = Rect::new(lx + 10, (inner.h - 4) / 2, px - 10 - lx - 10, 4);
    c.round_rect(bar, 2, t.c.border);
    let fill = bar.w * level.min(100) as i32 / 100;
    if fill > 0 {
        c.round_rect(Rect::new(bar.x, bar.y, fill, bar.h), 2, t.c.accent);
    }
    r
}

/// How wide [`tag`] draws `word`.
pub fn tag_w(t: &Theme, word: &str) -> i32 {
    t.f.small_bold.measure(word) + 8
}

/// A count or a word in a tone, right-aligned to `right`. Returns its left.
pub fn tag(c: &mut Canvas, t: &Theme, right: i32, r: Rect, word: &str, tone: Tone) -> i32 {
    let col = t.c.tone(tone);
    let w = tag_w(t, word);
    let tr = Rect::new(right - w, r.y + (r.h - 13) / 2, w, 13);
    raised(c, tr, t.m.radius_small, mix(t.c.bg, col, 60), t.m.chip_lift);
    text_centre(c, t.f.small_bold, tr, word, col);
    tr.x
}

/// What a [`message`] shows.
pub struct Message<'a> {
    pub icon: &'a str,
    /// The icon's tone; none draws it muted, for an empty list.
    pub tone: Option<Tone>,
    pub text: &'a str,
    pub note: &'a str,
    /// What Enter, or another key, does here: in pills under the note.
    pub keys: &'a [Hint<'a>],
}

/// A message that fills `area`: a large icon, a line, a note, and the keys
/// that act on it. Empty lists and errors use it.
pub fn message(c: &mut Canvas, t: &Theme, area: Rect, m: &Message) {
    let (icon_name, text, note, keys) = (m.icon, m.text, m.note, m.keys);
    let ink = m.tone.map_or(t.c.muted, |k| t.c.tone(k));
    // A note breaks at each `\n`, and wraps where a line is too long.
    let w = area.w - 24;
    let lines: Vec<String> = note
        .split('\n')
        .filter(|l| !l.is_empty())
        .flat_map(|l| t.f.small.wrap(l, w))
        .collect();
    let lh = t.f.small.line;
    let rest = 8 + 18 + lh * lines.len() as i32 + if keys.is_empty() { 0 } else { 8 + t.m.pill_h };
    // A long note takes a smaller icon, so the whole message fits.
    let size = if IconSize::Large as i32 + rest <= area.h {
        IconSize::Large
    } else {
        IconSize::Medium
    };
    let s = size as i32;
    let top = area.y + (area.h - s - rest).max(0) / 2;
    icon::draw(c, icon_name, size, area.x + (area.w - s) / 2, top, ink);
    let tr = Rect::new(area.x + 12, top + s + 8, area.w - 24, 18);
    text_centre(c, t.f.body_bold, tr, text, t.c.fg);
    let mut y = tr.bottom();
    for line in &lines {
        let lr = Rect::new(tr.x, y + 1, tr.w, lh);
        text_centre(c, t.f.small, lr, line, t.c.muted);
        y += lh;
    }
    if !keys.is_empty() {
        let w = key_row_w(t, keys);
        key_row(c, t, area.x + (area.w - w) / 2, y + 8, keys);
    }
}

/// A labelled bar 4 px high: what, how far, the bar. With no value a
/// segment sits at `phase` (0 to 255) along it; move it only while shown.
pub fn progress(c: &mut Canvas, t: &Theme, r: Rect, label: &str, value: Option<u8>, phase: u8) {
    let line = Rect::new(r.x, r.y, r.w, 16);
    let right = match value {
        Some(v) => {
            let p = Percent::new(v);
            text_right(c, t.f.small, line.right(), line, p.as_str(), t.c.muted) - 8
        }
        None => line.right(),
    };
    text_in(
        c,
        t.f.small,
        Rect::new(line.x, line.y, right - line.x, line.h),
        label,
        t.c.fg,
    );
    let bar = Rect::new(r.x, line.bottom() + 3, r.w, 4);
    c.round_rect(bar, 2, t.c.border);
    let part = match value {
        Some(v) => Rect::new(bar.x, bar.y, bar.w * v.min(100) as i32 / 100, bar.h),
        None => {
            let w = bar.w / 4;
            Rect::new(bar.x + (bar.w - w) * phase as i32 / 255, bar.y, w, bar.h)
        }
    };
    if part.w > 0 {
        c.round_rect(part, 2, t.c.accent);
    }
}

/// Steps in a row, joined by a line: a dot each, in the accent once done,
/// larger for the current one with its name in bold. `r` is about 26 px tall.
pub fn stepper(c: &mut Canvas, t: &Theme, r: Rect, steps: &[&str], current: usize) {
    if steps.is_empty() {
        return;
    }
    let step_w = r.w / steps.len() as i32;
    let line_y = r.y + 5;
    for (i, name) in steps.iter().enumerate() {
        let cx = r.x + step_w * i as i32 + step_w / 2;
        if i + 1 < steps.len() {
            let col = if i < current { t.c.accent } else { t.c.border };
            c.fill(Rect::new(cx + 8, line_y, step_w - 16, 1), col);
        }
        let (x, y) = (cx as f32, line_y as f32 + 0.5);
        // Plain dots: an icon or a ring at this size is too small to read.
        let ink = if i < current {
            c.circle(x, y, 4.0, t.c.accent);
            t.c.accent
        } else if i == current {
            c.circle(x, y, 5.0, t.c.accent);
            t.c.fg
        } else {
            c.circle(x, y, 4.0, t.c.border);
            t.c.muted
        };
        let f = if i == current {
            t.f.small_bold
        } else {
            t.f.small
        };
        text_centre(
            c,
            f,
            Rect::new(cx - step_w / 2, r.y + 12, step_w, 14),
            name,
            ink,
        );
    }
}

/// Work that keys wait for: the page dimmed, a card that says what is
/// happening, and a segment moving along a bar at `phase` (0 to 255).
pub fn busy(c: &mut Canvas, t: &Theme, area: Rect, text: &str, phase: u8) {
    scrim(c, t, area);
    let w = t.m.width - 88;
    let h = card_h(t, 4);
    let r = Rect::new((t.m.width - w) / 2, area.y + (area.h - h).max(0) / 2, w, h);
    let body = card(c, t, r, text, t.c.accent);
    let bar = Rect::new(body.x, body.y, body.w, 4);
    c.round_rect(bar, 2, t.c.border);
    let seg = bar.w / 4;
    let x = bar.x + (bar.w - seg) * phase as i32 / 255;
    c.round_rect(Rect::new(x, bar.y, seg, bar.h), 2, t.c.accent);
}

/// A row that can't be used now: icon and label in `disabled`, and why on
/// its right.
pub fn disabled_row(c: &mut Canvas, t: &Theme, r: Rect, icon_name: &str, label: &str, why: &str) {
    let ink = t.c.disabled();
    let x = icon_tile(c, t, r, icon_name, t.c.border, ink);
    let wx = text_right(c, t.f.small, r.right() - 8, r, why, ink);
    text_in(c, t.f.body, Rect::new(x, r.y, wx - 8 - x, r.h), label, ink);
}

/// A row with a second line in Caption under its label, `app_row_h` high.
pub fn note_row(c: &mut Canvas, t: &Theme, r: Rect, row: &Row, note: &str, selected: bool) {
    if selected {
        row_selection(c, t, r);
    }
    let body = Row { label: "", ..*row };
    row_body(c, t, r, &body, selected, false, true);
    let x = match row.icon {
        Some(_) => r.x + 4 + t.m.row_icon + 9,
        None => r.x + 9,
    };
    let right = r.right() - 8 - trailing_w(t, &row.trailing);
    let f = if selected { t.f.body_bold } else { t.f.body };
    let col = if selected { t.c.fg } else { soft(t) };
    let (y1, y2) = two_lines(t, r, f);
    f.draw_elided(c, x, y1, row.label, right - x, col);
    t.f.small.draw_elided(c, x, y2, note, right - x, t.c.muted);
}

/// About how wide a row's right side draws, so a second line stops short.
fn trailing_w(t: &Theme, tr: &Trailing) -> i32 {
    match tr {
        Trailing::None => 0,
        Trailing::Value(v) | Trailing::Alert(v) => t.f.small.measure(v) + 8,
        Trailing::ValueChevron(v) => t.f.small.measure(v) + 8 + 17,
        Trailing::Chevron => 17,
        Trailing::Toggle(_) => 30,
        Trailing::Choice(v) => t.f.small.measure(v) + 28,
        Trailing::Level(_) => 110,
        Trailing::Signal { secure, .. } => 18 + if *secure { 18 } else { 0 },
        Trailing::Check => 22,
        Trailing::Tag(w, _) => t.f.small_bold.measure(w) + 16,
    }
}

/// What the lock screen shows.
pub struct Lock<'a> {
    pub clock: &'a str,
    pub date: &'a str,
    /// The password field, once Tab then Enter opened it: the characters
    /// typed. `None` is the resting screen, where no key types, so keys
    /// pressed in a pocket go nowhere.
    pub field: Option<usize>,
    /// The field takes keys: not while the check runs or a wait counts down.
    pub ready: bool,
    /// One line under the field: "Wrong password", "Try again in 5 s".
    pub note: Option<(Tone, &'a str)>,
    /// The note says how many notifications wait: a bell goes before it.
    pub bell: bool,
}

/// The lock screen: the time, large, the date, and how to get back in: Tab
/// then Enter, then the password when one is asked. The
/// top bar keeps the battery and the network; the caller passes no dock and
/// no clock, since the open apps stay private and the time is here.
pub fn lock_screen(c: &mut Canvas, t: &Theme, s: &Status, l: &Lock) {
    c.clear(c.bounds(), t.c.bg);
    status_bar(c, t, s);
    let b = body(t);
    let g = t.m.lock_gap;
    let clock = t.f.clock;
    // With the field open the time moves up to make room.
    let top = b.y + if l.field.is_some() { 4 } else { 12 };
    let y = top + clock.cap();
    let w = clock.measure(l.clock);
    clock.draw(c, (t.m.width - w) / 2, y, l.clock, t.c.fg);
    let y = y + g + 2 + t.f.small.cap();
    let w = t.f.small.measure(l.date);
    t.f.small.draw(c, (t.m.width - w) / 2, y, l.date, soft(t));
    let y = y + 2 * g;
    let fw = t.m.lock_field_w;
    let field = Rect::new((t.m.width - fw) / 2, y, fw, t.m.field_h);
    let note_y = match l.field {
        Some(n) => {
            let dots = "*".repeat(n);
            text_field(
                c,
                t,
                field,
                &dots,
                true,
                l.ready,
                if l.ready { "Password" } else { "" },
            );
            if matches!(l.note, Some((Tone::Critical, _))) {
                c.stroke_round_rect(field, t.m.radius, t.c.critical);
            }
            field.bottom() + 2
        }
        None => field.y,
    };
    if let Some((tone, text)) = l.note {
        let col = match tone {
            Tone::Info => soft(t),
            _ => t.c.tone(tone),
        };
        let s = IconSize::Tiny as i32;
        let tw = t.f.small.measure(text);
        // A note at rest, such as how many notifications wait, sits on a
        // pane of its own, with a bell for notifications.
        let at_rest = l.field.is_none();
        let icon_name = match tone {
            Tone::Info if l.bell => Some("bell"),
            Tone::Info => None,
            _ => Some(tone.icon()),
        };
        let total = tw + if icon_name.is_some() { s + 4 } else { 0 };
        let mut x = (t.m.width - total) / 2;
        let nr = Rect::new(x, note_y, total, t.m.field_note_h);
        if at_rest {
            let pr = Rect::new(nr.x - 9, nr.y, nr.w + 18, nr.h + 2);
            c.pane(pr, t.m.radius_large, t.c.surface);
        }
        if let Some(name) = icon_name {
            let ink = if l.bell { t.c.accent } else { col };
            icon::draw(c, name, IconSize::Tiny, x, nr.y + (nr.h - s) / 2 + 1, ink);
            x += s + 4;
        }
        text_in(c, t.f.small, Rect::new(x, nr.y + 1, tw, nr.h), text, col);
    }
    match l.field {
        Some(_) if l.ready => hint_bar(
            c,
            t,
            &[
                Hint {
                    key: "Enter",
                    label: "unlock",
                },
                Hint {
                    key: "Esc",
                    label: "back",
                },
            ],
            None,
        ),
        Some(_) => hint_bar(c, t, &[], None),
        None => {
            let hints = [
                Hint {
                    key: "Tab",
                    label: "then",
                },
                Hint {
                    key: "Enter",
                    label: "unlock",
                },
            ];
            // In the bottom bar's place: the lock has no chips.
            let w = key_row_w(t, &hints);
            let y = t.m.height - t.m.hint_h + (t.m.hint_h - t.m.pill_h) / 2;
            key_row(c, t, (t.m.width - w) / 2, y, &hints);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::contrast;
    use crate::theme::PALETTES;

    #[test]
    fn system_symbols_read_on_the_neutral_badge() {
        for p in PALETTES {
            let t = Theme::new(*p);
            for selected in [false, true] {
                let fill = neutral_fill(&t, selected);
                assert!(contrast(p.accent, fill) >= 3.0, "{} {selected}", p.name);
            }
        }
    }

    #[test]
    fn a_pressed_chip_only_changes_its_bar() {
        let pairs = [
            (Look::Normal, Look::Pressed),
            (Look::Main, Look::MainPressed),
            (Look::On, Look::MainPressed),
        ];
        for p in PALETTES {
            let t = Theme::new(*p);
            let top = t.m.height - t.m.chip_foot - t.m.chip_h;
            let bar_top = top + t.m.chip_h + t.m.chip_foot - 2;
            for (icon, word) in [("search", "Find"), ("aperture", "")] {
                for (up, down) in pairs {
                    let draw = |look| {
                        let mut c = Canvas::new(t.m.width, t.m.height);
                        let mut chips = [Chip::default(); 5];
                        chips[2] = Chip { icon, word, look };
                        action_bar(&mut c, &t, &chips);
                        c
                    };
                    let (a, b) = (draw(up), draw(down));
                    for y in 0..bar_top {
                        assert_eq!(a.row(y), b.row(y), "{} {word} {up:?}: row {y}", p.name);
                    }
                    let bar_changed = (bar_top..t.m.height).any(|y| a.row(y) != b.row(y));
                    assert!(bar_changed, "{} {word} {up:?}", p.name);
                }
            }
        }
    }

    fn bar(t: &Theme, percent: u8, charging: bool) -> Canvas {
        let mut c = Canvas::new(t.m.width, t.m.bar_h);
        let s = Status {
            dock: &[],
            front: None,
            icon: "",
            title: "",
            context: "",
            tabs: &[],
            front_tab: 0,
            item: None,
            clock: "12:00",
            battery: Some(Battery { percent, charging }),
            net: Net::Wifi(3),
            bluetooth: true,
            notices: 0,
        };
        status_bar(&mut c, t, &s);
        c
    }

    #[test]
    fn charging_draws_a_bolt_inside_the_battery_and_moves_nothing() {
        let bolt = BOLT.iter().filter(|&&a| a != 0).count();
        for p in PALETTES {
            let t = Theme::new(*p);
            // At 5% and under the percent turns critical, which this does
            // not count.
            for percent in [15, 50, 100] {
                let (on, off) = (bar(&t, percent, true), bar(&t, percent, false));
                let ink = |c: &Canvas| c.pixels().iter().filter(|&&px| px == t.c.fg).count();
                // Above 20% the fill itself is fg until charging turns it accent.
                if percent <= 20 {
                    assert_eq!(ink(&on), ink(&off) + bolt, "{} {percent}%", p.name);
                }
                let changed: Vec<i32> = (0..on.pixels().len())
                    .filter(|&i| on.pixels()[i] != off.pixels()[i])
                    .map(|i| i as i32 % t.m.width)
                    .collect();
                let span = changed.iter().max().unwrap() - changed.iter().min().unwrap();
                assert!(span < BATTERY_W, "{} {percent}%: {span}", p.name);
            }
        }
    }
}
