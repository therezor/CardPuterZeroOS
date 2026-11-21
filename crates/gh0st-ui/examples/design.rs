//! Design system mockups: every bar, control and overlay proposed for the Gh0st
//! design system, drawn at panel size and 3x into mockups/design, with a
//! captioned contact sheet per palette. The drawing here is a prototype: once
//! the look is approved it moves into gh0st-ui and the SDK. Run from the
//! workspace root:
//!
//!     cargo run -p gh0st-ui --example design

use gh0st_ui::color::{hex, mix};
use gh0st_ui::font::Font;
use gh0st_ui::icon::{self, IconSize};
use gh0st_ui::snapshot::write_png;
use gh0st_ui::theme::{self, Palette, Theme};
use gh0st_ui::widgets::{
    self, AppItem, Card, Net, Row, Status, Trailing, Visual, soft, text_centre, text_in,
    text_right,
};
use gh0st_ui::{Canvas, Color, Rect};
use std::path::PathBuf;

// ---------------------------------------------------------------- roles

/// Proposed palette tweaks: Paper's border and warning are too faint on its
/// light ground.
fn tuned(p: Palette) -> Palette {
    let mut p = p;
    if p.name == "Paper" {
        p.border = hex("#b3ab9a");
        p.warning = hex("#7d5600");
    }
    p
}

/// Proposed new role: neutral notices. Every palette gets its own.
fn info(t: &Theme) -> Color {
    match t.c.name {
        "Nord" => hex("#81a1c1"),
        _ => t.c.tints[1],
    }
}

/// Proposed new role: inert items, about 3:1 on bg.
fn disabled(t: &Theme) -> Color {
    mix(t.c.bg, t.c.muted, 180)
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Info,
    Ok,
    Warning,
    Critical,
}

fn tone(t: &Theme, k: Tone) -> Color {
    match k {
        Tone::Info => info(t),
        Tone::Ok => t.c.ok,
        Tone::Warning => t.c.warning,
        Tone::Critical => t.c.critical,
    }
}

fn tone_icon(k: Tone) -> &'static str {
    match k {
        Tone::Info => "info",
        Tone::Ok => "circle-check",
        Tone::Warning => "triangle-alert",
        Tone::Critical => "circle-x",
    }
}

// ---------------------------------------------------------------- top bar

#[derive(Clone)]
struct Top<'a> {
    /// The page you are on: its icon and name, in bold.
    icon: &'a str,
    title: &'a str,
    /// The parent, muted: the app when the page is deeper, a step, a file.
    context: &'a str,
    tabs: &'a [&'a str],
    front_tab: usize,
    /// One app status item: a word after a dot, in a tone.
    item: Option<(&'a str, Tone)>,
    bluetooth: bool,
    net: Net,
    battery: u8,
    charging: bool,
}

fn top<'a>(icon: &'a str, title: &'a str) -> Top<'a> {
    Top {
        icon,
        title,
        context: "",
        tabs: &[],
        front_tab: 0,
        item: None,
        bluetooth: false,
        net: Net::Wifi(3),
        battery: 87,
        charging: false,
    }
}

fn sub<'a>(icon: &'a str, title: &'a str, parent: &'a str) -> Top<'a> {
    let mut s = top(icon, title);
    s.context = parent;
    s
}

const CLOCK: &str = "14:32";

/// Overflow gives way in order: the context, then Bluetooth, then the title
/// shortens. Clock and battery never go.
fn top_bar(c: &mut Canvas, t: &Theme, s: &Top) {
    if !draw_top(c, t, s, s.bluetooth) {
        let mut s = s.clone();
        s.bluetooth = false;
        draw_top(c, t, &s, false);
    }
}

/// False when the title needs Bluetooth's room and `may_retry`.
fn draw_top(c: &mut Canvas, t: &Theme, s: &Top, may_retry: bool) -> bool {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.fill(r, t.c.bg);
    c.hline(0, r.w, r.bottom() - 1, t.c.border);
    let inner = Rect::new(t.m.pad, 0, r.w - 2 * t.m.pad, r.h - 1);

    let mut x = text_right(c, t.f.small_bold, inner.right(), inner, CLOCK, t.c.fg) - 9;
    x = battery(c, t, x, inner, s.battery, s.charging) - 9;
    match s.net {
        Net::Wifi(level) => {
            x -= 11;
            widgets::signal_bars(c, x, inner.y + 3, 9, level, t.c.fg, t.c.border);
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
    }
    if s.bluetooth {
        x -= 12;
        icon::draw(c, "bluetooth", IconSize::Tiny, x, inner.y + 2, t.c.fg);
        x -= 7;
    }
    if let Some((word, k)) = s.item {
        let col = tone(t, k);
        let tx = text_right(c, t.f.small_bold, x, inner, word, col);
        c.circle((tx - 6) as f32, (inner.h / 2) as f32 + 0.5, 2.5, col);
        x = tx - 9 - 8;
    }
    let end = x;

    let mut lx = t.m.pad;
    icon::draw(c, s.icon, IconSize::Tiny, lx, inner.y + 2, t.c.accent);
    lx += 12 + 6;
    if !s.tabs.is_empty() {
        lx -= 2;
        for (i, tab) in s.tabs.iter().enumerate() {
            let tr = Rect::new(lx, 1, 14, 15);
            let col = if i == s.front_tab { t.c.fg } else { t.c.muted };
            text_centre(c, t.f.small_bold, tr, tab, col);
            if i == s.front_tab {
                c.fill(Rect::new(tr.x, t.m.bar_h - 2, tr.w, 2), t.c.accent);
            }
            lx += 16;
        }
        lx += 4;
    }
    let room = end - lx;
    let tw = t.f.small_bold.measure(s.title);
    let cw = if s.context.is_empty() {
        0
    } else {
        t.f.small.measure(s.context) + 6
    };
    if tw > room && may_retry {
        return false;
    }
    let tw = tw.min(room.max(48));
    text_in(c, t.f.small_bold, Rect::new(lx, inner.y, tw, inner.h), s.title, t.c.fg);
    if cw > 0 && tw + cw <= room {
        let cx = lx + tw + 6;
        text_in(c, t.f.small, Rect::new(cx, inner.y, end - cx, inner.h), s.context, t.c.muted);
    }
    true
}

const BOLT_W: i32 = 5;
const BOLT: [&[u8; 5]; 8] = [
    b"..###", b".###.", b".##..", b"#####", b"..##.", b".##..", b".#...", b"#....",
];

fn battery(c: &mut Canvas, t: &Theme, right: i32, r: Rect, p: u8, charging: bool) -> i32 {
    let label = format!("{p}%");
    let low = !charging && p <= 5;
    let ink = if low { t.c.critical } else { t.c.fg };
    let x = text_right(c, t.f.small, right, r, &label, ink) - 4;
    let (w, h) = (17, 10);
    let bx = x - w - 2;
    let by = r.y + (r.h - h) / 2;
    let fill = if charging {
        t.c.accent
    } else if low {
        t.c.critical
    } else if p <= 20 {
        t.c.warning
    } else {
        t.c.fg
    };
    c.stroke_round_rect(Rect::new(bx, by, w, h), 1, t.c.muted);
    c.fill(Rect::new(bx + w, by + (h - 4) / 2, 2, 4), t.c.muted);
    let level = ((w - 4) * p.min(100) as i32 + 50) / 100;
    c.fill(Rect::new(bx + 2, by + 2, level.max(1), h - 4), fill);
    if charging {
        let (ox, oy) = (bx + (w - BOLT_W) / 2, by + 1);
        let room = Rect::new(bx + 1, by + 1, w - 2, h - 2);
        let on = |x: i32, y: i32| BOLT[y as usize][x as usize] == b'#';
        for y in 0..8 {
            for x in 0..BOLT_W {
                if on(x, y) {
                    c.fill(Rect::new(ox + x - 1, oy + y - 1, 3, 3).intersect(&room), t.c.bg);
                }
            }
        }
        for y in 0..8 {
            for x in 0..BOLT_W {
                if on(x, y) {
                    c.set(ox + x, oy + y, t.c.fg);
                }
            }
        }
    }
    bx
}

/// The shell's urgent word: it takes the top bar's place while shown, so the
/// page underneath never moves or hides.
fn banner(c: &mut Canvas, t: &Theme, k: Tone, text: &str) {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    let col = tone(t, k);
    c.fill(r, col);
    let w = 12 + 5 + t.f.small_bold.measure(text);
    let x = (t.m.width - w) / 2;
    icon::draw(c, tone_icon(k), IconSize::Tiny, x, r.y + 3, t.c.bg);
    text_in(c, t.f.small_bold, Rect::new(x + 17, r.y, w, r.h - 1), text, t.c.bg);
}

/// Volume or brightness keys: the level takes the top bar's place for 1.5 s.
fn hud(c: &mut Canvas, t: &Theme, icon_name: &str, label: &str, level: u8) {
    let r = Rect::new(0, 0, t.m.width, t.m.bar_h);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.bottom() - 1, t.c.accent);
    let inner = Rect::new(t.m.pad, 0, r.w - 2 * t.m.pad, r.h - 1);
    icon::draw(c, icon_name, IconSize::Tiny, inner.x, 2, t.c.accent);
    let lx = text_in(c, t.f.small_bold, Rect::new(inner.x + 18, 0, 80, inner.h), label, t.c.fg);
    let px = text_right(c, t.f.small, inner.right(), inner, &format!("{level}%"), t.c.fg);
    let bar = Rect::new(lx + 10, 7, px - 10 - lx - 10, 4);
    c.round_rect(bar, 2, t.c.border);
    c.round_rect(Rect::new(bar.x, bar.y, bar.w * level as i32 / 100, bar.h), 2, t.c.accent);
}

// ---------------------------------------------------------------- action bar

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Normal,
    Primary,
    /// A toggle that is on: a tint, not a ring.
    On,
    Danger,
    DangerMain,
    Disabled,
    Pressed,
    /// Held, filling red: erase.
    HoldDanger(u8),
    /// Working on it: no second press.
    Busy,
}

#[derive(Clone, Copy)]
struct Chip<'a> {
    icon: &'a str,
    word: &'a str,
    look: Look,
    /// The line to the key: its digit runs the chip now.
    stalk: bool,
}

fn l<'a>(icon: &'a str, word: &'a str, look: Look) -> Option<Chip<'a>> {
    Some(Chip {
        icon,
        word,
        look,
        stalk: true,
    })
}

fn n<'a>(icon: &'a str, word: &'a str) -> Option<Chip<'a>> {
    l(icon, word, Look::Normal)
}

fn p<'a>(icon: &'a str, word: &'a str) -> Option<Chip<'a>> {
    l(icon, word, Look::Primary)
}

/// Key 8 everywhere: the app menu. Icon only, so its neighbour keeps a word.
fn menu() -> Option<Chip<'static>> {
    n("menu", "")
}

/// Odd, so a 10 px capital gets two clear rows above and below inside the
/// edge.
const CHIP_H: i32 = 17;
const CHIP_R: i32 = 3;
/// Space between a chip's edge and its ink, and the least it may shrink to
/// when neighbours crowd it.
const CHIP_PAD: i32 = 7;
const CHIP_PAD_MIN: i32 = 4;

/// The ink's width: icon, gap, word.
fn ink_w(t: &Theme, ch: &Chip) -> i32 {
    let busy = if ch.look == Look::Busy { t.f.small_bold.measure("\u{2026}") } else { 0 };
    let word = if ch.word.is_empty() { 0 } else { t.f.small_bold.measure(ch.word) + busy };
    match (ch.icon.is_empty(), ch.word.is_empty()) {
        (false, true) => 12,
        (true, _) => word,
        (false, false) => 12 + 3 + word,
    }
}

/// A chip's natural width: its ink with even padding.
fn chip_w(t: &Theme, ch: &Chip) -> i32 {
    ink_w(t, ch) + 2 * CHIP_PAD
}

/// Widths for a bar. Chips are centred on their keys, so each pair of
/// neighbours shares the room between their centres. A crowded pair first
/// gives up padding, in proportion to what each has spare, down to the least
/// padding; only past that does a chip lose its icon.
fn bar_widths(t: &Theme, slots: &Slots) -> [i32; 5] {
    let pitch = t.m.softkey_x[1] - t.m.softkey_x[0];
    let room = pitch - 2;
    let mut want = [0; 5];
    let mut need = [0; 5];
    for (i, s) in slots.iter().enumerate() {
        if let Some(ch) = s {
            want[i] = chip_w(t, ch);
            need[i] = ink_w(t, ch) + 2 * CHIP_PAD_MIN;
        }
    }
    // Each chip's half width, capped from the left and from the right.
    let mut half: [i32; 5] = want.map(|w| w / 2);
    for i in 0..4 {
        let (a, b) = (i, i + 1);
        if want[a] == 0 || want[b] == 0 || half[a] + half[b] <= room {
            continue;
        }
        let (na, nb) = (need[a] / 2, need[b] / 2);
        let (ha, hb) = if na + nb <= room {
            let (sa, sb) = (want[a] / 2 - na, want[b] / 2 - nb);
            let extra = room - na - nb;
            let ga = if sa + sb > 0 { extra * sa / (sa + sb) } else { extra / 2 };
            (na + ga, nb + extra - ga)
        } else {
            (room / 2, room - room / 2)
        };
        half[a] = half[a].min(ha);
        half[b] = half[b].min(hb);
    }
    half.map(|h| 2 * h)
}

fn chip_ink(c: &mut Canvas, t: &Theme, r: Rect, ch: &Chip, ink: Color) {
    let busy;
    let word = if ch.look == Look::Busy {
        busy = format!("{}\u{2026}", ch.word);
        busy.as_str()
    } else {
        ch.word
    };
    // The ink sits centred, so the padding is even on both sides.
    let x = r.x + (r.w - ink_w(t, ch)) / 2;
    let iy = r.y + 2;
    // Capitals centre in the chip less its bottom row: the edge rows are
    // the ring, and descenders get the spare row below.
    let r = Rect::new(r.x, r.y, r.w, r.h - 1);
    if word.is_empty() {
        icon::draw(c, ch.icon, IconSize::Tiny, x, iy, ink);
        return;
    }
    if ch.icon.is_empty() {
        text_in(c, t.f.small_bold, Rect::new(x, r.y, r.right() - x, r.h), word, ink);
        return;
    }
    icon::draw(c, ch.icon, IconSize::Tiny, x, iy, ink);
    let lx = x + 12 + 3;
    text_in(c, t.f.small_bold, Rect::new(lx, r.y, r.right() - lx, r.h), word, ink);
}

/// One chip centred on `cx`, its line down to `line_to` when given.
fn chip(c: &mut Canvas, t: &Theme, cx: i32, top: i32, ch: &Chip, w: i32, line_to: Option<i32>) {
    // Words never elide: a chip short of room drops its icon first.
    let bare;
    let tight = |ch: &Chip| ink_w(t, ch) + 2 * CHIP_PAD_MIN;
    let (ch, w) = if tight(ch) > w && !ch.icon.is_empty() && !ch.word.is_empty() {
        bare = Chip { icon: "", ..*ch };
        (&bare, chip_w(t, &bare).min(w))
    } else {
        (ch, w)
    };
    let r = Rect::new(cx - w / 2, top, w, CHIP_H);
    let k = &t.c;
    let rad = CHIP_R;
    let (fill, ink, line, ring) = match ch.look {
        Look::Normal => (k.surface, k.fg, k.muted, Some(k.border)),
        Look::Primary => (k.accent, k.accent_fg, k.accent, None),
        Look::On => (mix(k.bg, k.accent, 80), k.accent, k.accent, None),
        Look::Danger | Look::HoldDanger(_) => (k.surface, k.critical, k.critical, Some(k.critical)),
        Look::DangerMain => (k.critical, k.bg, k.critical, None),
        Look::Disabled => (k.bg, disabled(t), k.border, Some(k.border)),
        Look::Pressed => (k.fg, k.bg, k.fg, None),
        Look::Busy => (k.surface, k.muted, k.muted, Some(k.border)),
    };
    c.round_rect(r, rad, fill);
    if let Some(ring) = ring {
        c.stroke_round_rect(r, rad, ring);
    }
    chip_ink(c, t, r, ch, ink);
    let mut line = line;
    let held = match ch.look {
        Look::HoldDanger(pc) => Some((pc, k.critical, k.bg)),
        _ => None,
    };
    if let Some((pc, f, i)) = held {
        let old = c.clip();
        c.set_clip(Rect::new(r.x, r.y, r.w * pc as i32 / 100, r.h).intersect(&old));
        c.round_rect(r, rad, f);
        chip_ink(c, t, r, ch, i);
        c.set_clip(old);
        line = f;
    }
    if let Some(bottom) = line_to
        && ch.stalk
    {
        c.fill(Rect::new(cx - 1, r.bottom(), 2, bottom - r.bottom()), line);
    }
}

type Slots<'a> = [Option<Chip<'a>>; 5];

/// The under-screen action bar: a chip above each of keys 4 to 8.
fn action_bar(c: &mut Canvas, t: &Theme, slots: &Slots) {
    let h = t.m.height;
    c.fill(Rect::new(0, h - t.m.hint_h, t.m.width, t.m.hint_h), t.c.bg);
    let top = h - 2 - CHIP_H;
    let w = bar_widths(t, slots);
    for (i, s) in slots.iter().enumerate() {
        if let Some(ch) = s {
            chip(c, t, t.m.softkey_x[i], top, ch, w[i], Some(h));
        }
    }
}

/// While a field has focus the digits type, so no chip can run: the chips
/// give way to this bar, which names the keys that still work. The main key's
/// pill is filled, as the main chip is. Pills are 15 px with two clear rows
/// above and below, inside the bar's 20.
fn hint_bar(c: &mut Canvas, t: &Theme, hints: &[(&str, &str)], right: Option<(&str, &str)>) {
    let r = Rect::new(0, t.m.height - t.m.hint_h, t.m.width, t.m.hint_h);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.y, t.c.border);
    let last = hints.len().saturating_sub(1);
    let mut x = t.m.pad;
    for (i, &(keys, label)) in hints.iter().enumerate() {
        x = pills(c, t, x, r.y + 3, keys, i == last) + 4;
        x = text_in(c, t.f.small, Rect::new(x, r.y + 3, 120, 15), label, t.c.muted) + 12;
    }
    // The menu hint goes on the right while it fits; the page's own keys
    // come first.
    if let Some((keys, label)) = right {
        let lw = t.f.small.measure(label);
        let lx = r.right() - t.m.pad - lw;
        if lx - 4 - pills_w(t, keys) < x {
            return;
        }
        text_in(c, t.f.small, Rect::new(lx, r.y + 3, lw, 15), label, t.c.muted);
        let w = pills_w(t, keys);
        pills(c, t, lx - 4 - w, r.y + 3, keys, false);
    }
}

fn pill_w(t: &Theme, key: &str) -> i32 {
    (t.f.small_bold.measure(key) + 10).max(15)
}

fn pills_w(t: &Theme, keys: &str) -> i32 {
    keys.split(' ').map(|k| pill_w(t, k) + 3).sum::<i32>() - 3
}

/// Key pills from `x`; returns the right edge. `main` fills them in accent.
fn pills(c: &mut Canvas, t: &Theme, x: i32, y: i32, keys: &str, main: bool) -> i32 {
    let (fill, ink) = if main { (t.c.accent, t.c.accent_fg) } else { (t.c.border, t.c.fg) };
    let mut x = x;
    for k in keys.split(' ') {
        let pr = Rect::new(x, y, pill_w(t, k), 15);
        c.round_rect(pr, t.m.radius_small, fill);
        text_in(c, t.f.small_bold, Rect::new(pr.x + 5, pr.y, pr.w - 5, pr.h), k, ink);
        x = pr.right() + 3;
    }
    x - 3
}

const MENU_HINT: Option<(&str, &str)> = Some(("Fn h", "help"));

/// Enter and Esc are named, never given chips: a line of key pills with what
/// they do, for cards and messages. The last pill is the main one.
fn key_row(c: &mut Canvas, t: &Theme, x: i32, y: i32, hints: &[(&str, &str)]) -> i32 {
    let last = hints.len().saturating_sub(1);
    let mut x = x;
    for (i, &(keys, label)) in hints.iter().enumerate() {
        x = pills(c, t, x, y, keys, i == last) + 4;
        x = text_in(c, t.f.small, Rect::new(x, y, 120, 15), label, t.c.muted) + 12;
    }
    x - 12
}

fn key_row_w(t: &Theme, hints: &[(&str, &str)]) -> i32 {
    hints
        .iter()
        .map(|&(k, l)| pills_w(t, k) + 4 + t.f.small.measure(l) + 12)
        .sum::<i32>()
        - 12
}

// ---------------------------------------------------------------- content

fn frame(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    c
}

fn rows_from(t: &Theme, y: i32, row_h: i32, i: usize) -> Rect {
    Rect::new(t.m.margin, y + i as i32 * row_h, t.m.width - 2 * t.m.margin, row_h)
}

fn row_select(c: &mut Canvas, t: &Theme, r: Rect) {
    c.round_rect(r, t.m.radius, t.c.surface);
    c.round_rect(Rect::new(r.x, r.y + 4, 3, r.h - 8), 1, t.c.accent);
}

fn row<'a>(icon: &'a str, tint: Color, label: &'a str, trailing: Trailing<'a>) -> Row<'a> {
    Row {
        icon: Some(icon),
        tint,
        label,
        trailing,
    }
}

#[derive(Clone, Copy)]
enum Extra<'a> {
    Check,
    Tag(&'a str, Color),
}

fn row_extra(c: &mut Canvas, t: &Theme, r: Rect, rw: &Row, sel: bool, extra: Extra) {
    widgets::list_row(c, t, r, rw, sel);
    let right = r.right() - 8;
    match extra {
        Extra::Check => {
            icon::draw(c, "check", IconSize::Small, right - 14, r.y + (r.h - 14) / 2, t.c.accent);
        }
        Extra::Tag(word, col) => {
            tag(c, t, right, r, word, col);
        }
    }
}

fn tag(c: &mut Canvas, t: &Theme, right: i32, r: Rect, word: &str, col: Color) -> i32 {
    let w = t.f.small_bold.measure(word) + 8;
    let tr = Rect::new(right - w, r.y + (r.h - 13) / 2, w, 13);
    c.round_rect(tr, t.m.radius_small, mix(t.c.bg, col, 60));
    text_centre(c, t.f.small_bold, tr, word, col);
    tr.x
}

fn row_disabled(c: &mut Canvas, t: &Theme, r: Rect, icon_name: &str, label: &str, note: &str) {
    let ink = disabled(t);
    icon::draw(c, icon_name, IconSize::Small, r.x + 9, r.y + (r.h - 14) / 2, ink);
    let x = r.x + 9 + 14 + 9;
    let nx = text_right(c, t.f.small, r.right() - 8, r, note, ink);
    text_in(c, t.f.body, Rect::new(x, r.y, nx - 8 - x, r.h), label, ink);
}

#[allow(clippy::too_many_arguments)]
fn row_two(
    c: &mut Canvas,
    t: &Theme,
    r: Rect,
    icon_name: &str,
    label: &str,
    note: &str,
    signal: Option<u8>,
    sel: bool,
) {
    if sel {
        row_select(c, t, r);
    }
    icon::draw(c, icon_name, IconSize::Small, r.x + 9, r.y + (r.h - 14) / 2, t.c.accent);
    let x = r.x + 9 + 14 + 9;
    let mut right = r.right() - 8;
    if let Some(level) = signal {
        widgets::signal_bars(c, right - 11, r.y + (r.h - 9) / 2, 9, level, t.c.fg, t.c.border);
        right -= 11 + 8;
    }
    let f = if sel { t.f.body_bold } else { t.f.body };
    let col = if sel { t.c.fg } else { soft(t) };
    text_in(c, f, Rect::new(x, r.y + 2, right - x, 16), label, col);
    text_in(c, t.f.small, Rect::new(x, r.y + 17, right - x, 13), note, t.c.muted);
}

fn wrap(f: Font, s: &str, w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if f.measure(&next) > w && !line.is_empty() {
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

fn message(c: &mut Canvas, t: &Theme, icon_name: &str, ink: Color, text: &str, note: &str, keys: &[(&str, &str)]) {
    let b = widgets::body(t);
    let top = b.y + 10;
    icon::draw(c, icon_name, IconSize::Large, (t.m.width - 36) / 2, top, ink);
    let tr = Rect::new(b.x + 12, top + 36 + 8, b.w - 24, 18);
    text_centre(c, t.f.body_bold, tr, text, t.c.fg);
    let nr = Rect::new(b.x + 12, tr.bottom() + 1, b.w - 24, 15);
    text_centre(c, t.f.small, nr, note, t.c.muted);
    if !keys.is_empty() {
        let w = key_row_w(t, keys);
        key_row(c, t, (t.m.width - w) / 2, nr.bottom() + 8, keys);
    }
}

fn progress(c: &mut Canvas, t: &Theme, y: i32, label: &str, value: Option<u8>) {
    let r = Rect::new(15, y, t.m.width - 30, 16);
    let right = match value {
        Some(v) => text_right(c, t.f.small, r.right(), r, &format!("{v}%"), t.c.muted) - 8,
        None => r.right(),
    };
    text_in(c, t.f.small, Rect::new(r.x, r.y, right - r.x, r.h), label, t.c.fg);
    let bar = Rect::new(r.x, r.bottom() + 3, r.w, 4);
    c.round_rect(bar, 2, t.c.border);
    let part = match value {
        Some(v) => Rect::new(bar.x, bar.y, bar.w * v as i32 / 100, bar.h),
        None => Rect::new(bar.x + bar.w * 30 / 100, bar.y, bar.w / 4, bar.h),
    };
    c.round_rect(part, 2, t.c.accent);
}

fn field_label(c: &mut Canvas, t: &Theme, y: i32, s: &str, col: Color) {
    text_in(c, t.f.small, Rect::new(15, y, t.m.width - 30, 13), s, col);
}

fn field(c: &mut Canvas, t: &Theme, y: i32, text: &str, masked: bool, focused: bool, error: bool) {
    let r = Rect::new(12, y, t.m.width - 24, 22);
    widgets::text_field(c, t, r, text, masked, focused, "");
    if error {
        c.stroke_round_rect(r, t.m.radius, t.c.critical);
    }
}

/// Covers the content, not the bars: the chips stay readable and live.
fn scrim(c: &mut Canvas, t: &Theme) {
    c.fill_alpha(widgets::body(t), t.c.bg, 160);
}

fn card(c: &mut Canvas, t: &Theme, r: Rect, title: &str, edge: Color) -> Rect {
    c.round_rect(r, t.m.radius_large, t.c.surface);
    c.stroke_round_rect(r, t.m.radius_large, edge);
    let head = Rect::new(r.x + 12, r.y + 6, r.w - 24, 18);
    text_in(c, t.f.body_bold, head, title, t.c.fg);
    Rect::new(r.x + 12, head.bottom() + 4, r.w - 24, r.bottom() - head.bottom() - 10)
}

fn toast(c: &mut Canvas, t: &Theme, y: i32, k: Tone, text: &str) {
    toast_held(c, t, y, k, text, None);
}

/// A toast for a key that must be held: a bar along its foot fills as the
/// hold goes on.
fn toast_held(c: &mut Canvas, t: &Theme, y: i32, k: Tone, text: &str, held: Option<u8>) {
    let w = (12 + 4 + t.f.small_bold.measure(text) + 18).min(t.m.width - 2 * t.m.margin);
    let r = Rect::new((t.m.width - w) / 2, y, w, 20);
    c.round_rect(r, t.m.radius_large, t.c.surface);
    c.stroke_round_rect(r, t.m.radius_large, mix(t.c.border, tone(t, k), 110));
    icon::draw(c, tone_icon(k), IconSize::Tiny, r.x + 8, r.y + 4, tone(t, k));
    let tx = r.x + 8 + 12 + 5;
    text_in(c, t.f.small_bold, Rect::new(tx, r.y, r.right() - 8 - tx, r.h), text, t.c.fg);
    if let Some(pc) = held {
        let bar = Rect::new(r.x + 4, r.bottom() - 4, r.w - 8, 2);
        c.fill(bar, t.c.border);
        c.fill(Rect::new(bar.x, bar.y, bar.w * pc as i32 / 100, bar.h), tone(t, k));
    }
}

/// Above the chips, clear of their lines.
fn toast_y(t: &Theme) -> i32 {
    t.m.height - t.m.hint_h - 20 - 4
}

fn toast_over_bar(c: &mut Canvas, t: &Theme, text: &str) {
    let r = Rect::new(0, t.m.height - t.m.hint_h, t.m.width, t.m.hint_h);
    c.fill(r, t.c.surface);
    c.hline(0, r.w, r.y, t.c.accent);
    text_centre(c, t.f.small_bold, r, text, t.c.fg);
}

fn blit(dst: &mut Canvas, src: &Canvas, ox: i32, oy: i32) {
    for y in 0..src.height() {
        for x in 0..src.width() {
            dst.set(ox + x, oy + y, src.get(x, y));
        }
    }
}

// ---------------------------------------------------------------- screens

fn settings_rows(t: &Theme) -> Vec<Row<'static>> {
    let a = t.c.accent;
    vec![
        row("wifi", a, "Wi-Fi", Trailing::ValueChevron("HomeNet")),
        row("bluetooth", a, "Bluetooth", Trailing::Toggle(true)),
        row("palette", a, "Palette", Trailing::Choice("Mint")),
        row("sun", a, "Brightness", Trailing::Level(80)),
        row("volume-2", a, "Sound", Trailing::ValueChevron("60%")),
    ]
}

fn settings(t: &Theme, sel: usize, slots: Slots) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &top("settings", "Settings"));
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, rw) in settings_rows(t).iter().enumerate() {
        widgets::list_row(&mut c, t, rows_from(t, area.y, t.m.row_h, i), rw, i == sel);
    }
    widgets::scrollbar(&mut c, t, area, 0, 5, 9);
    action_bar(&mut c, t, &slots);
    c
}

/// Settings' own keys: Enter opens, Esc goes back, n finds.
fn link_slots() -> Slots<'static> {
    [None, n("search", "Find"), None, None, menu()]
}

fn roles(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("palette", "Colour roles", t.c.name));
    let cells: [(&str, Color); 14] = [
        ("bg", t.c.bg),
        ("surface", t.c.surface),
        ("border", t.c.border),
        ("fg", t.c.fg),
        ("soft", soft(t)),
        ("muted", t.c.muted),
        ("disabled", disabled(t)),
        ("accent", t.c.accent),
        ("accent_fg", t.c.accent_fg),
        ("info", info(t)),
        ("ok", t.c.ok),
        ("warning", t.c.warning),
        ("critical", t.c.critical),
        ("tints", t.c.tints[0]),
    ];
    let (w, h) = (74, 31);
    for (i, &(name, col)) in cells.iter().enumerate() {
        let (x, y) = (6 + (i as i32 % 4) * (w + 4), 22 + (i as i32 / 4) * (h + 2));
        if name == "tints" {
            for (j, tint) in t.c.tints.iter().enumerate() {
                c.round_rect(Rect::new(x + j as i32 * 12, y, 11, 16), 2, *tint);
            }
        } else {
            let sw = Rect::new(x, y, w, 16);
            c.round_rect(sw, t.m.radius, col);
            c.stroke_round_rect(sw, t.m.radius, t.c.border);
        }
        text_in(&mut c, t.f.small, Rect::new(x + 1, y + 17, w, 13), name, t.c.muted);
    }
    c
}

fn type_scale(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &top("tag", "Type and icons"));
    let mut y = 22;
    let lines: [(Font, &str, Color, i32); 7] = [
        (t.f.big, "Display 24", t.c.fg, 26),
        (t.f.title, "Title 17", t.c.fg, 20),
        (t.f.body_bold, "Body strong 15", t.c.fg, 18),
        (t.f.body, "Body 15", soft(t), 18),
        (t.f.small_bold, "Label 13", t.c.accent, 16),
        (t.f.small, "Caption 13", t.c.muted, 16),
        (t.f.mono, "Mono 10  ~/code $ ls", t.c.fg, 14),
    ];
    for (f, s, col, h) in lines {
        text_in(&mut c, f, Rect::new(12, y, 180, h), s, col);
        y += h;
    }
    let sizes = [
        (IconSize::Tiny, "12 bars"),
        (IconSize::Small, "14 rows"),
        (IconSize::Bar, "16"),
        (IconSize::Medium, "26 tiles"),
        (IconSize::Large, "36 large"),
    ];
    let mut y = 22;
    for (s, label) in sizes {
        let px = s as i32;
        icon::draw(&mut c, "settings", s, 206, y, t.c.accent);
        text_in(&mut c, t.f.small, Rect::new(248, y, 70, px.max(13)), label, t.c.muted);
        y += px + 4;
    }
    c
}

fn top_rows(t: &Theme, bars: &[(&str, Top)]) -> Canvas {
    let mut c = frame(t);
    let mut y = 0;
    for (caption, b) in bars {
        text_in(&mut c, t.f.small, Rect::new(8, y, 300, 13), caption, t.c.muted);
        y += 13;
        let mut bar = Canvas::new(t.m.width, t.m.bar_h);
        bar.fill(bar.bounds(), t.c.bg);
        top_bar(&mut bar, t, b);
        blit(&mut c, &bar, 0, y);
        y += t.m.bar_h + 3;
    }
    c
}

fn top_a(t: &Theme) -> Canvas {
    let mut tabs = top("square-terminal", "zsh");
    tabs.tabs = &["1", "2", "3"];
    tabs.front_tab = 1;
    tabs.context = "~/code";
    let mut bt = top("pencil", "Notes");
    bt.bluetooth = true;
    let mut cable = top("pencil", "Notes");
    cable.net = Net::Ethernet;
    cable.battery = 64;
    cable.charging = true;
    top_rows(
        t,
        &[
            ("First page: app icon and name; Wi-Fi, battery, clock", top("settings", "Settings")),
            ("Deeper: the page in bold, its parent muted", sub("wifi", "Wi-Fi", "Settings")),
            ("Tabs for sessions, the front one underlined", tabs),
            ("Bluetooth on", bt),
            ("On a cable, charging", cable),
        ],
    )
}

fn top_b(t: &Theme) -> Canvas {
    let mut rec = top("mic", "Recorder");
    rec.item = Some(("REC", Tone::Critical));
    let mut over = sub("pencil", "Holiday plans and packing list.txt", "Notes");
    over.bluetooth = true;
    let mut low = top("settings", "Settings");
    low.battery = 4;
    low.net = Net::None;
    let mut warn = top("settings", "Settings");
    warn.battery = 15;
    top_rows(
        t,
        &[
            ("App item: one word in a tone", rec),
            ("Overflow: context, then Bluetooth, then title", over),
            ("Battery at 4%, no network", low),
            ("Battery at 15%", warn),
        ],
    )
}

fn chip_states(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &top("tag", "Chip states"));
    let rows: [[(Option<Chip>, &str); 5]; 3] = [
        [
            (n("search", "Find"), "normal"),
            (p("arrow-right", "Next"), "main"),
            (l("sun", "Flash", Look::On), "on"),
            (l("trash-2", "Forget", Look::Danger), "danger"),
            (l("trash-2", "Erase", Look::HoldDanger(40)), "hold red"),
        ],
        [
            (l("check", "Join", Look::Disabled), "disabled"),
            (l("search", "Find", Look::Pressed), "pressed"),
            (l("square", "Stop", Look::Danger), "danger"),
            (l("check", "Join", Look::Busy), "busy"),
            (menu(), "icon only"),
        ],
        [
            (n("chevron-left", "Prev"), "z"),
            (n("chevron-right", "Next"), "c"),
            (n("search", "Find"), "s"),
            (n("eye", "Show"), "Tab"),
            (menu(), "Fn 8"),
        ],
    ];
    let captions = ["", "", "Other keys that run the same chip"];
    for (ri, row) in rows.iter().enumerate() {
        let y = 24 + ri as i32 * 38;
        if !captions[ri].is_empty() {
            text_in(&mut c, t.f.small, Rect::new(8, y - 2, 300, 12), captions[ri], t.c.muted);
        }
        let cy = y + if captions[ri].is_empty() { 0 } else { 11 };
        let slots: Slots = row.map(|(ch, _)| ch);
        let w = bar_widths(t, &slots);
        for (i, (ch, cap)) in row.iter().enumerate() {
            let ch = ch.unwrap();
            let cx = t.m.softkey_x[i];
            chip(&mut c, t, cx, cy, &ch, w[i], None);
            let cr = Rect::new(cx - 28, cy + CHIP_H + 1, 56, 13);
            text_centre(&mut c, t.f.small, cr, cap, t.c.muted);
        }
    }
    action_bar(&mut c, t, &link_slots());
    c
}

fn rows_a(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("cpu", "System", "Settings"));
    let a = t.c.accent;
    let (y, h) = (20, t.m.row_h);
    widgets::list_row(&mut c, t, rows_from(t, y, h, 0), &row("info", a, "Version", Trailing::Value("0.4.1")), false);
    row_extra(&mut c, t, rows_from(t, y, h, 1), &row("globe", a, "English (UK)", Trailing::None), false, Extra::Check);
    row_extra(&mut c, t, rows_from(t, y, h, 2), &row("download", a, "Updates", Trailing::None), false, Extra::Tag("3", t.c.accent));
    row_disabled(&mut c, t, rows_from(t, y, h, 3), "usb", "USB gadget", "needs a cable");
    widgets::list_row(&mut c, t, rows_from(t, y, h, 4), &row("trash-2", t.c.critical, "Forget network", Trailing::Alert("Press again")), true);
    action_bar(&mut c, t, &[None, None, l("trash-2", "Forget", Look::DangerMain), None, menu()]);
    c
}

fn rows_b(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("wifi", "Wi-Fi", "Settings"));
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Connected");
    y += 15;
    row_two(&mut c, t, rows_from(t, y, 33, 0), "wifi", "HomeNet", "Connected, 5 GHz, 192.168.1.42", Some(3), true);
    y += 33 + 2;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Nearby");
    y += 15;
    let a = t.c.accent;
    widgets::list_row(&mut c, t, rows_from(t, y, t.m.row_h, 0), &row("wifi", a, "Office-5G", Trailing::Signal { level: 4, secure: true }), false);
    widgets::list_row(&mut c, t, rows_from(t, y, t.m.row_h, 1), &row("wifi", a, "Cafe Guest", Trailing::Signal { level: 2, secure: false }), false);
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    widgets::scrollbar(&mut c, t, area, 0, 4, 7);
    action_bar(
        &mut c,
        t,
        &[None, n("refresh-cw", "Scan"), l("trash-2", "Forget", Look::Danger), None, menu()],
    );
    c
}

fn picker(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("palette", "Palette", "Settings"));
    let a = t.c.accent;
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, name) in ["Mint", "Nord", "Paper"].iter().enumerate() {
        let r = rows_from(t, area.y, t.m.row_h, i);
        let rw = row("palette", a, name, Trailing::None);
        if i == 0 {
            row_extra(&mut c, t, r, &rw, false, Extra::Check);
        } else {
            widgets::list_row(&mut c, t, r, &rw, i == 1);
        }
    }
    action_bar(&mut c, t, &[None, None, None, None, menu()]);
    c
}

fn form(t: &Theme, rule_b: bool) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("user", "Account", "Setup 7 of 10"));
    field_label(&mut c, t, 21, "Your name on this device", t.c.muted);
    field(&mut c, t, 35, "alex", false, false, false);
    field_label(&mut c, t, 62, "Password, 4 or more", t.c.muted);
    field(&mut c, t, 76, "abc", true, true, true);
    icon::draw(&mut c, "triangle-alert", IconSize::Tiny, 15, 103, t.c.critical);
    text_in(&mut c, t.f.small, Rect::new(31, 101, 260, 16), "Too short: 4 or more", t.c.critical);
    let _ = rule_b;
    hint_bar(&mut c, t, &[("Esc", "back"), ("Tab", "next field"), ("Enter", "next")], MENU_HINT);
    c
}

fn wifi_behind(c: &mut Canvas, t: &Theme) {
    top_bar(c, t, &sub("wifi", "Wi-Fi", "Settings"));
    let a = t.c.accent;
    let names = ["HomeNet", "Office-5G", "Cafe Guest", "Neighbours", "Library"];
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, s) in names.iter().enumerate() {
        let rw = row("wifi", a, s, Trailing::Signal { level: 3, secure: true });
        widgets::list_row(c, t, rows_from(t, area.y, t.m.row_h, i), &rw, i == 1);
    }
}

/// Field, then a 6 px gap, a 15 px line of pills or a note, and 6 px again
/// to the card's edge.
fn join_card(c: &mut Canvas, t: &Theme) -> Rect {
    scrim(c, t);
    let body = card(c, t, Rect::new(22, 34, 276, 80), "Join \u{201c}Office-5G\u{201d}", t.c.accent);
    let fr = Rect::new(body.x, body.y + 2, body.w, 22);
    widgets::text_field(c, t, fr, "hunter22", true, true, "");
    fr
}

fn dialog_chips(t: &Theme) -> Canvas {
    let mut c = frame(t);
    wifi_behind(&mut c, t);
    let fr = join_card(&mut c, t);
    let note = "Password for a WPA2 network";
    text_in(&mut c, t.f.small, Rect::new(fr.x, fr.bottom() + 6, fr.w, 15), note, t.c.muted);
    hint_bar(&mut c, t, &[("Esc", "cancel"), ("Tab", "show"), ("Enter", "join")], MENU_HINT);
    c
}

fn dialog_busy(t: &Theme) -> Canvas {
    let mut c = frame(t);
    wifi_behind(&mut c, t);
    let fr = join_card(&mut c, t);
    let note = "Joining\u{2026}";
    text_in(&mut c, t.f.small, Rect::new(fr.x, fr.bottom() + 6, 80, 15), note, t.c.muted);
    progress_line(&mut c, t, fr.bottom() + 12);
    action_bar(&mut c, t, &[None; 5]);
    c
}

fn progress_line(c: &mut Canvas, t: &Theme, y: i32) {
    let bar = Rect::new(100, y, 186, 3);
    c.round_rect(bar, 1, t.c.border);
    c.round_rect(Rect::new(bar.x + 90, bar.y, 60, bar.h), 1, t.c.accent);
}

fn dialog_danger(t: &Theme) -> Canvas {
    let mut c = settings(t, 0, [None; 5]);
    scrim(&mut c, t);
    let body = card(&mut c, t, Rect::new(22, 30, 276, 92), "Erase everything?", t.c.critical);
    let text = "Apps, files and settings are deleted. This can't be undone.";
    for (i, line) in wrap(t.f.body, text, body.w).iter().enumerate() {
        text_in(&mut c, t.f.body, Rect::new(body.x, body.y + i as i32 * 18, body.w, 18), line, soft(t));
    }
    key_row(&mut c, t, body.x, body.y + 40, &[("Esc", "cancel")]);
    let hint = "hold 7 to erase";
    let hx = body.right() - t.f.small.measure(hint);
    text_in(&mut c, t.f.small, Rect::new(hx, body.y + 40, body.w, 15), hint, t.c.muted);
    action_bar(&mut c, t, &[None, None, None, l("trash-2", "Erase", Look::HoldDanger(40)), None]);
    c
}

fn dialog_three(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Normal);
    scrim(&mut c, t);
    let body = card(&mut c, t, Rect::new(22, 30, 276, 92), "Save changes?", t.c.accent);
    let text = "Packing list has edits you haven't saved.";
    for (i, line) in wrap(t.f.body, text, body.w).iter().enumerate() {
        text_in(&mut c, t.f.body, Rect::new(body.x, body.y + i as i32 * 18, body.w, 18), line, soft(t));
    }
    key_row(&mut c, t, body.x, body.y + 40, &[("Esc", "cancel"), ("Enter", "save")]);
    action_bar(&mut c, t, &[None, None, l("trash-2", "Discard", Look::Danger), None, None]);
    c
}

fn app_menu(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("menu", "Menu", "Settings"));
    let a = t.c.accent;
    let (x, w, kw) = (6, t.m.width - 12, 40);
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(x, y, w, 15), "Settings");
    y += 15;
    let rows: [(Row, &str); 2] = [
        (row("search", a, "Search", Trailing::None), "n"),
        (row("layout-grid", a, "Top bar", Trailing::Toggle(true)), "b"),
    ];
    for (i, (rw, k)) in rows.iter().enumerate() {
        widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), rw, k, kw, i == 0, false);
        y += 22;
    }
    widgets::section(&mut c, t, Rect::new(x, y, w, 15), "App");
    y += 15;
    let keys = row("keyboard", a, "Help", Trailing::Chevron);
    widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), &keys, "Fn h", kw, false, false);
    y += 22;
    let close = row("circle-x", t.c.critical, "Close app", Trailing::None);
    widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), &close, "", kw, false, false);
    action_bar(&mut c, t, &[None, None, None, None, l("menu", "", Look::On)]);
    c
}

fn keys_sheet(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("keyboard", "Help", "Settings"));
    let (x, w, kw) = (6, t.m.width - 12, 62);
    let mut y = 19;
    let lines: [(&str, &str, &str); 7] = [
        ("#", "Under the screen", ""),
        ("5 n", "Find", ""),
        ("8", "Menu", ""),
        ("#", "Keys", ""),
        ("Esc", "Back; hold to exit", ""),
        ("Enter", "Open", ""),
        ("Fn k", "Switch apps", ""),
    ];
    for (k, label, v) in lines {
        if k == "#" {
            widgets::section(&mut c, t, Rect::new(x, y, w, 16), label);
            y += 16;
        } else {
            widgets::keys_line(&mut c, t, Rect::new(x, y, w, 19), k, kw, label, v);
            y += 19;
        }
    }
    widgets::scrollbar(&mut c, t, Rect::new(x, 20, w, 130), 0, 8, 13);
    action_bar(&mut c, t, &[None; 5]);
    c
}

fn toasts(t: &Theme) -> Canvas {
    let mut c = settings(t, 0, link_slots());
    scrim(&mut c, t);
    let items = [
        (24, Tone::Ok, "Saved to Screenshots"),
        (50, Tone::Info, "Hold to exit"),
        (76, Tone::Warning, "Battery at 15%"),
        (102, Tone::Critical, "Couldn't join Office-5G"),
    ];
    for (y, k, s) in items {
        toast(&mut c, t, y, k, s);
    }
    c
}

fn toggle_slots() -> Slots<'static> {
    link_slots()
}

fn toast_float(t: &Theme) -> Canvas {
    let mut c = settings(t, 1, toggle_slots());
    toast(&mut c, t, toast_y(t), Tone::Ok, "Bluetooth on");
    c
}

fn toast_bar(t: &Theme) -> Canvas {
    let mut c = settings(t, 1, toggle_slots());
    toast_over_bar(&mut c, t, "Bluetooth on");
    c
}

const NOTES: [(&str, &str); 4] = [
    ("Shopping", "Today"),
    ("Packing list", "Mon"),
    ("Ideas", "Nov 12"),
    ("Wi-Fi codes", "Oct 3"),
];

fn notes_list(c: &mut Canvas, t: &Theme) {
    let a = t.c.accent;
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, (s, v)) in NOTES.iter().enumerate() {
        let rw = row("pencil", a, s, Trailing::Value(v));
        widgets::list_row(c, t, rows_from(t, area.y, t.m.row_h, i), &rw, i == 0);
    }
}

fn notes_first(t: &Theme, _exit_look: Look) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &top("pencil", "Notes"));
    notes_list(&mut c, t);
    action_bar(
        &mut c,
        t,
        &[None, n("plus", "New"), None, None, menu()],
    );
    c
}

fn esc_tap(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Normal);
    toast_held(&mut c, t, toast_y(t), Tone::Info, "Hold to exit", Some(0));
    c
}

fn esc_hold(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Normal);
    toast_held(&mut c, t, toast_y(t), Tone::Info, "Hold to exit", Some(60));
    c
}

fn esc_hung(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Pressed);
    banner(&mut c, t, Tone::Warning, "Not responding. Hold Esc to close it");
    c
}

fn low_battery(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Normal);
    banner(&mut c, t, Tone::Critical, "Battery 3%: powering off in 30 s");
    c
}

fn volume(t: &Theme) -> Canvas {
    let mut c = notes_first(t, Look::Normal);
    hud(&mut c, t, "volume-2", "Volume", 60);
    c
}

fn progress_screen(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("download", "Updates", "Settings"));
    progress(&mut c, t, 26, "Downloading gh0st 0.5", Some(42));
    let note = "18 of 43 MB, about a minute left";
    text_in(&mut c, t.f.small, Rect::new(15, 50, 290, 13), note, t.c.muted);
    progress(&mut c, t, 76, "Checking the download", None);
    let note = "Moves only while it is on screen";
    text_in(&mut c, t.f.small, Rect::new(15, 100, 290, 13), note, t.c.muted);
    action_bar(&mut c, t, &[None, None, l("square", "Stop", Look::Danger), None, menu()]);
    c
}

fn empty(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let mut tb = sub("wifi", "Wi-Fi", "Settings");
    tb.net = Net::None;
    top_bar(&mut c, t, &tb);
    let note = "Move closer to the router, then scan again.";
    message(&mut c, t, "wifi-off", t.c.muted, "No networks found", note, &[("Enter", "scan")]);
    action_bar(&mut c, t, &[None, None, None, None, menu()]);
    c
}

fn error(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let mut tb = sub("wifi", "Wi-Fi", "Settings");
    tb.net = Net::None;
    top_bar(&mut c, t, &tb);
    let note = "The password was not accepted.";
    message(&mut c, t, "circle-x", t.c.critical, "Couldn't join Office-5G", note, &[("Enter", "try again")]);
    action_bar(&mut c, t, &[None, None, n("pencil", "Edit"), None, menu()]);
    c
}

const READER: &str = "The panel sits right above the number row, so an action on screen is drawn above the key that runs it. Press the key under a chip and that chip runs. Enter and Esc need no chip: Enter opens or does the main thing, Esc goes back one step, and holding Esc on the first screen closes the app. Key 8 opens the menu, and Fn and H list every key.";

fn reader(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("info", "Keys under the screen", "Help 2 of 5"));
    let b = widgets::body(t);
    for (i, line) in wrap(t.f.body, READER, b.w - 30).iter().take(7).enumerate() {
        let r = Rect::new(15, b.y + 3 + i as i32 * 18, b.w - 30, 18);
        text_in(&mut c, t.f.body, r, line, t.c.fg);
    }
    widgets::scrollbar(&mut c, t, Rect::new(6, b.y + 2, 306, 128), 1, 7, 30);
    action_bar(
        &mut c,
        t,
        &[None, n("chevron-up", "PgUp"), n("chevron-down", "PgDn"), None, menu()],
    );
    c
}

fn find(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &sub("search", "Find", "Home"));
    let fr = Rect::new(12, 21, t.m.width - 24, 22);
    widgets::text_field(&mut c, t, fr, "re", false, true, "");
    let a = t.c.accent;
    let hits = [
        ("mic", "Recorder", "Sound and voice notes"),
        ("settings", "Settings", "Wi-Fi, display, about"),
        ("download", "Updates", "In Settings"),
    ];
    for (i, (ic, s, v)) in hits.iter().enumerate() {
        let r = rows_from(t, 46, 34, i);
        let r = Rect::new(r.x, r.y, r.w, if i == 2 { 26 } else { 34 });
        if i < 2 {
            row_two(&mut c, t, r, ic, s, v, None, i == 0);
        } else {
            widgets::list_row(&mut c, t, r, &row(ic, a, s, Trailing::None), false);
        }
    }
    hint_bar(&mut c, t, &[("Esc", "cancel"), ("Enter", "open")], MENU_HINT);
    c
}

fn level_row(t: &Theme) -> Canvas {
    settings(t, 3, [None, n("minus", "Less"), n("plus", "More"), None, menu()])
}

fn home(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let dock = [
        Visual::System {
            icon: "square-terminal",
        },
        Visual::Letter {
            ch: 'm',
            tint: t.c.tints[5],
        },
    ];
    let status = Status {
        dock: &dock,
        front: Some(0),
        title: "",
        clock: CLOCK,
        battery: Some(widgets::Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
        bluetooth: true,
    };
    widgets::status_bar(&mut c, t, &status);
    let items = [
        ("Terminal", Visual::System { icon: "square-terminal" }),
        ("Settings", Visual::System { icon: "settings" }),
        ("Store", Visual::Badge { icon: "shopping-bag", tint: t.c.tints[2] }),
        ("Camera", Visual::Badge { icon: "camera", tint: t.c.tints[3] }),
        ("Files", Visual::Badge { icon: "folder", tint: t.c.tints[2] }),
        ("Music", Visual::Letter { ch: 'm', tint: t.c.tints[5] }),
        ("Recorder", Visual::Badge { icon: "mic", tint: t.c.tints[3] }),
        ("Notes", Visual::Badge { icon: "pencil", tint: t.c.tints[4] }),
    ];
    let b = widgets::body(t);
    let area = Rect::new(b.x + t.m.margin, b.y + 3, b.w - 2 * t.m.margin, b.h - 6);
    let tw = (area.w - 3 * t.m.gap) / 4;
    let th = (area.h - t.m.gap) / 2;
    for (i, (label, v)) in items.into_iter().enumerate() {
        let r = Rect::new(
            area.x + (i as i32 % 4) * (tw + t.m.gap),
            area.y + (i as i32 / 4) * (th + t.m.gap),
            tw,
            th,
        );
        let item = AppItem {
            label,
            note: "",
            visual: v,
            pinned: i < 2,
        };
        tile(&mut c, t, r, &item, i == 1);
    }
    widgets::scrollbar(&mut c, t, area, 0, 2, 3);
    action_bar(
        &mut c,
        t,
        &[None, n("search", "Find"), n("arrow-left-right", "Apps"), None, menu()],
    );
    c
}

/// A home tile, spaced: badge and name as one block centred in the tile, the
/// pin inside the frame's corner, clear of the ring.
fn tile(c: &mut Canvas, t: &Theme, r: Rect, item: &AppItem, sel: bool) {
    if sel {
        widgets::selection(c, t, r);
    }
    let b = 34;
    let f = if sel { t.f.small_bold } else { t.f.small };
    // Even space from the ring to the badge and from the name's baseline to
    // the ring; descenders use the space below.
    let block = b + 5 + f.cap();
    let top = r.y + 1 + (r.h - 2 - block) / 2;
    let br = Rect::new(r.x + (r.w - b) / 2, top, b, b);
    widgets::badge(c, t, br, &item.visual, sel);
    if item.pinned {
        let col = if sel { t.c.accent } else { t.c.muted };
        icon::draw(c, "pin", IconSize::Tiny, r.right() - 5 - 12, r.y + 4, col);
    }
    let col = if sel { t.c.fg } else { soft(t) };
    let w = f.measure(item.label).min(r.w - 8);
    f.draw_elided(c, r.x + (r.w - w) / 2, br.bottom() + 5 + f.cap(), item.label, r.w - 8, col);
    if sel {
        widgets::selection_ring(c, t, r);
    }
}

fn switcher(t: &Theme) -> Canvas {
    switcher_with(t, false)
}

/// Close all asks first, in a window; Close on one app does not.
fn switcher_close_all(t: &Theme) -> Canvas {
    let mut c = switcher_with(t, true);
    scrim(&mut c, t);
    let body = card(&mut c, t, Rect::new(22, 30, 276, 92), "Close all apps?", t.c.critical);
    let text = "Terminal and Settings close. Work you haven't saved is lost.";
    for (i, line) in wrap(t.f.body, text, body.w).iter().enumerate() {
        text_in(&mut c, t.f.body, Rect::new(body.x, body.y + i as i32 * 18, body.w, 18), line, soft(t));
    }
    key_row(&mut c, t, body.x, body.y + 40, &[("Esc", "cancel"), ("Enter", "close all")]);
    action_bar(&mut c, t, &[None; 5]);
    c
}

fn switcher_with(t: &Theme, dialog: bool) -> Canvas {
    let mut c = frame(t);
    let dock = [
        Visual::System {
            icon: "square-terminal",
        },
        Visual::System { icon: "settings" },
    ];
    let status = Status {
        dock: &dock,
        front: Some(1),
        title: "",
        clock: CLOCK,
        battery: Some(widgets::Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
        bluetooth: false,
    };
    widgets::status_bar(&mut c, t, &status);
    let b = widgets::content(t);
    let w = (b.w - 2 * t.m.gap) / 3;
    let cards = [
        Card { label: "Home", note: "", stats: None, visual: Visual::System { icon: "layout-grid" }, paused: false },
        Card { label: "Terminal", note: "", stats: Some("12M 1%"), visual: Visual::System { icon: "square-terminal" }, paused: false },
        Card { label: "Settings", note: "", stats: Some("6M 0%"), visual: Visual::System { icon: "settings" }, paused: true },
    ];
    for (i, cd) in cards.iter().enumerate() {
        let r = Rect::new(b.x + i as i32 * (w + t.m.gap), b.y, w, b.h);
        if i == 1 {
            widgets::selection(&mut c, t, r);
            widgets::selection_ring(&mut c, t, r);
        }
        widgets::card_content(&mut c, t, r, cd, i == 1);
    }
    if !dialog {
        action_bar(
            &mut c,
            t,
            &[
                None,
                l("circle-x", "Close", Look::Danger),
                l("copy-x", "All", Look::Danger),
                None,
                None,
            ],
        );
    }
    c
}

const TERM: [&str; 9] = [
    "rezor@zero ~/code $ ls",
    "assets  crates  docs  packaging  scripts",
    "rezor@zero ~/code $ cargo build --release",
    "   Compiling gh0st-ui v0.4.1",
    "   Compiling gh0st-app v0.4.1",
    "    Finished release in 41.2s",
    "rezor@zero ~/code $ git status --short",
    " M docs/design.md",
    "rezor@zero ~/code $ ",
];

fn terminal(t: &Theme, scroll: bool) -> Canvas {
    let mut c = frame(t);
    let mut tb = top("square-terminal", "zsh");
    tb.tabs = &["1", "2"];
    tb.context = if scroll { "scrolling" } else { "~/code" };
    top_bar(&mut c, t, &tb);
    let lh = 12;
    let bottom = if scroll { t.m.height - t.m.hint_h } else { t.m.height };
    let n_lines = ((bottom - 20) / lh) as usize;
    let first = TERM.len().saturating_sub(n_lines);
    for (i, line) in TERM[first..].iter().enumerate() {
        let y = 20 + i as i32 * lh;
        let r = Rect::new(4, y, 312, lh);
        if scroll && i == 3 {
            c.fill(r, mix(t.c.bg, t.c.accent, 60));
        }
        let col = if line.starts_with("rezor") { t.c.accent } else { t.c.fg };
        text_in(&mut c, t.f.mono, r, line, col);
        if !scroll && i + first == TERM.len() - 1 {
            let x = 4 + t.f.mono.measure(line);
            c.fill(Rect::new(x, y + 1, 6, lh - 2), t.c.fg);
        }
    }
    if scroll {
        action_bar(&mut c, t, &[None, n("search", "Find"), n("copy", "Copy"), None, menu()]);
    } else {
        toast(&mut c, t, t.m.height - 24, Tone::Info, "Fn Esc sends Esc. Fn h: help");
    }
    c
}

fn camera(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top_bar(&mut c, t, &top("camera", "Camera"));
    let b = widgets::body(t);
    for y in b.y..b.bottom() {
        let k = ((y - b.y) * 255 / b.h) as u8;
        c.hline(0, t.m.width, y, mix(hex("#2a4a6a"), hex("#c08a50"), k));
    }
    let f = Rect::new(130, 55, 60, 60);
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let x = if dx == 0 { f.x } else { f.right() - 10 };
        let y = if dy == 0 { f.y } else { f.bottom() - 2 };
        c.fill(Rect::new(x, y, 10, 2), t.c.fg);
        let x = if dx == 0 { f.x } else { f.right() - 2 };
        let y = if dy == 0 { f.y } else { f.bottom() - 10 };
        c.fill(Rect::new(x, y, 2, 10), t.c.fg);
    }
    action_bar(
        &mut c,
        t,
        &[None, n("zoom-out", "Out"), n("zoom-in", "In"), n("images", "Album"), menu()],
    );
    c
}

// ---------------------------------------------------------------- output

fn sheet(t: &Theme, screens: &[(&str, &str, Canvas)]) -> Canvas {
    let (w, h, gap, cap, cols) = (t.m.width, t.m.height, 10, 18, 3);
    let n = screens.len() as i32;
    let rows_n = (n + cols - 1) / cols;
    let mut s = Canvas::new(cols * (w + gap) + gap, rows_n * (h + gap + cap) + gap);
    s.fill(s.bounds(), t.c.surface);
    for (i, (name, caption, c)) in screens.iter().enumerate() {
        let ox = gap + (i as i32 % cols) * (w + gap);
        let oy = gap + (i as i32 / cols) * (h + gap + cap);
        let label = format!("{name}  {caption}");
        text_in(&mut s, t.f.small_bold, Rect::new(ox, oy, w, cap - 3), &label, t.c.fg);
        blit(&mut s, c, ox, oy + cap);
        s.stroke_round_rect(Rect::new(ox - 1, oy + cap - 1, w + 2, h + 2), 1, t.c.border);
    }
    s
}

/// Screens docs/design.md shows.
const DOC: [&str; 14] = [
    "01-roles",
    "03-top-a",
    "05-chips",
    "06-list",
    "08-list-choice",
    "14-typing",
    "16-dialog-typing",
    "19-dialog-danger",
    "21-menu",
    "22-help",
    "23-toasts",
    "27-esc-hold",
    "36-home",
    "37-switcher",
];

type Screen = (&'static str, &'static str, fn(&Theme) -> Canvas, bool);

fn main() {
    let out = PathBuf::from("mockups/design");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    // (name, caption, draw, in every palette)
    let all: &[Screen] = &[
        ("01-roles", "colour roles", roles, true),
        ("02-type", "type and icon scale", type_scale, false),
        ("03-top-a", "top bar: what can show", top_a, false),
        ("04-top-b", "top bar: app item, overflow, battery", top_b, false),
        ("05-chips", "chip states", chip_states, true),
        ("06-list", "Enter opens, Esc back: no chips for them", |t| settings(t, 0, link_slots()), true),
        ("08-list-choice", "choice row: 5 Prev, 6 Next", |t| settings(t, 2, [None, n("chevron-left", "Prev"), n("chevron-right", "Next"), None, menu()]), false),
        ("09-picker", "Enter on a choice opens the full list", picker, false),
        ("10-list-level", "level row: 5 Less, 6 More", level_row, false),
        ("12-rows-a", "rows: value, check, tag, disabled, armed", rows_a, true),
        ("13-rows-b", "rows: sections, two lines, signal", rows_b, false),
        ("14-typing", "typing: a key-hint bar replaces the chips", |t| form(t, false), true),
        ("16-dialog-typing", "dialog with a field: hint bar", dialog_chips, true),
        ("18-dialog-busy", "after Enter: busy in the card", dialog_busy, false),
        ("19-dialog-danger", "data loss: hold 7, never Enter", dialog_danger, true),
        ("20-dialog-three", "Enter save, 6 Discard, Esc cancel", dialog_three, false),
        ("21-menu", "app menu (8): Esc goes back one level", app_menu, false),
        ("22-help", "Help (Fn h): every key", keys_sheet, false),
        ("23-toasts", "toast tones", toasts, true),
        ("24-toast-float", "VARIANT toast above the chips", toast_float, false),
        ("25-toast-bar", "VARIANT toast over the bar (today)", toast_bar, false),
        ("26-esc-tap", "Esc tapped on the first page", esc_tap, false),
        ("27-esc-hold", "Esc held: the toast fills over 0.7 s", esc_hold, false),
        ("28-esc-hung", "app hung: banner takes the top bar", esc_hung, false),
        ("29-low-battery", "system banner", low_battery, false),
        ("30-volume", "volume keys: level takes the top bar", volume, false),
        ("31-progress", "progress", progress_screen, false),
        ("32-empty", "empty: Enter does the one action", empty, false),
        ("33-error", "error state", error, false),
        ("34-reader", "long text: PgUp, PgDn", reader, false),
        ("35-find", "find while typing", find, false),
        ("36-home", "home: Find, Apps, Menu", home, true),
        ("37-switcher", "switcher: 5 closes one, 6 all", switcher, false),
        ("37b-close-all", "Close all asks first", switcher_close_all, false),
        ("38-term-typing", "terminal: Esc as everywhere, Fn Esc sends Esc", |t| terminal(t, false), false),
        ("39-term-scroll", "terminal scroll mode: chips", |t| terminal(t, true), false),
        ("40-camera", "camera: Enter shoots", camera, false),
    ];
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let mut t = Theme::new(tuned(palette));
        // Proposed: a 20 px action bar, so 16 px chips breathe. Lists keep
        // five 26 px rows in the 132 px between the bars.
        t.m.hint_h = 20;
        let mint = palette.name == "Mint";
        let screens: Vec<(&str, &str, Canvas)> = all
            .iter()
            .filter(|s| mint || s.3)
            .map(|&(name, cap, draw, _)| (name, cap, draw(&t)))
            .collect();
        let tag = palette.name.to_lowercase();
        for (name, _, c) in &screens {
            write_png(c, &out.join(format!("{tag}-{name}.png")), 1).unwrap();
            write_png(c, &out.join(format!("{tag}-{name}@3x.png")), 3).unwrap();
        }
        // The doc's pictures, at 2x so they read on a desktop screen.
        if mint {
            let docs = PathBuf::from("docs/design");
            std::fs::create_dir_all(&docs).unwrap();
            for (name, _, c) in screens.iter().filter(|s| DOC.contains(&s.0)) {
                write_png(c, &docs.join(format!("{name}.png")), 2).unwrap();
            }
        }
        let s = sheet(&t, &screens);
        write_png(&s, &out.join(format!("{tag}-sheet.png")), 1).unwrap();
        write_png(&s, &out.join(format!("{tag}-sheet@2x.png")), 2).unwrap();
    }
    println!("wrote {}", out.display());
}
