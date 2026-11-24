//! Mouse mode: the arrow keys move the pointer, faster the longer they
//! are held. A timer runs only while a move key is down.

use gh0st_app::gfx::Rect;
use gh0st_app::gfx::widgets::{self, Hint, Pill};
use gh0st_app::prelude::*;
use gh0st_app::{KeyState, RawKey};
use gh0st_core::keys::code;

use crate::app::Shared;
use crate::hid::report::Mouse as Report;

/// A move every frame of the panel, near enough.
pub const TICK_MS: u64 = 16;
/// Pixels per tick: where a move starts, how fast it grows each second
/// held, and the most.
const START: f32 = 2.0;
const GROW: f32 = 14.0;
const MOST: f32 = 24.0;

const R: u16 = 19;
const P: u16 = 25;
const V: u16 = 47;

#[derive(Default)]
pub struct Mouse {
    /// Up, down, left, right.
    held: [bool; 4],
    buttons: u8,
    fast: bool,
    timer: Option<Timer>,
    /// Ticks since the move began: the longer, the faster.
    ticks: u32,
    /// What a tick moved less than a pixel, kept for the next.
    rest: (f32, f32),
}

fn dir(c: u16) -> Option<usize> {
    match c {
        code::F | code::UP => Some(0),
        code::X | code::DOWN => Some(1),
        code::Z | code::LEFT => Some(2),
        code::C | code::RIGHT => Some(3),
        _ => None,
    }
}

impl Mouse {
    pub fn moving(&self) -> bool {
        self.held.iter().any(|&h| h)
    }

    fn report(&self, dx: i8, dy: i8, wheel: i8) -> Report {
        Report {
            buttons: self.buttons,
            dx,
            dy,
            wheel,
            pan: 0,
        }
    }

    fn send(&self, s: &mut Shared, r: Report) {
        let mut out = Vec::new();
        s.engine.mouse(r, &mut out);
        s.send(&out);
    }

    pub fn raw(&mut self, s: &mut Shared, k: &RawKey, cx: &mut Cx) {
        let down = k.state != KeyState::Release;
        let press = k.state == KeyState::Press;
        match k.code {
            code::LEFTSHIFT | code::RIGHTSHIFT => self.fast = down,
            code::SPACE | code::ENTER | code::BACKSPACE | P if k.state != KeyState::Repeat => {
                let bit = match k.code {
                    code::BACKSPACE => 2,
                    P => 4,
                    _ => 1,
                };
                if down {
                    self.buttons |= bit;
                } else {
                    self.buttons &= !bit;
                }
                self.send(s, self.report(0, 0, 0));
            }
            R | V => {
                // The SDK repeats a held key: each repeat scrolls a notch.
                if down {
                    self.send(s, self.report(0, 0, if k.code == R { 1 } else { -1 }));
                }
            }
            c => {
                let Some(d) = dir(c) else { return };
                if k.state == KeyState::Repeat {
                    return;
                }
                self.held[d] = press;
                if self.moving() && self.timer.is_none() {
                    self.ticks = 0;
                    self.rest = (0.0, 0.0);
                    self.step(s);
                    self.timer = Some(cx.every(TICK_MS));
                } else if !self.moving()
                    && let Some(t) = self.timer.take()
                {
                    cx.cancel(t);
                }
            }
        }
        cx.redraw();
    }

    /// One tick: a move as far as the keys held say.
    pub fn step(&mut self, s: &mut Shared) {
        let held = self.ticks as f32 * TICK_MS as f32 / 1000.0;
        self.ticks = self.ticks.saturating_add(1);
        let mut speed = (START + GROW * held).min(MOST);
        if self.fast {
            speed *= 2.0;
        }
        let axis = |neg: bool, pos: bool| (pos as i32 - neg as i32) as f32;
        let x = axis(self.held[2], self.held[3]) * speed + self.rest.0;
        let y = axis(self.held[0], self.held[1]) * speed + self.rest.1;
        let (dx, dy) = (
            x.trunc().clamp(-127.0, 127.0),
            y.trunc().clamp(-127.0, 127.0),
        );
        self.rest = (x - dx, y - dy);
        if dx != 0.0 || dy != 0.0 {
            self.send(s, self.report(dx as i8, dy as i8, 0));
        }
    }

    pub fn timer(&mut self, s: &mut Shared, t: Timer) -> bool {
        if self.timer != Some(t) {
            return false;
        }
        self.step(s);
        true
    }

    /// Everything let go: on leaving the mode, Help, or a pause.
    pub fn stop(&mut self, cx: &mut Cx) {
        self.held = [false; 4];
        self.buttons = 0;
        if let Some(t) = self.timer.take() {
            cx.cancel(t);
        }
    }

    /// `iphone`: the host is an iPhone, which shows a pointer only with
    /// AssistiveTouch on.
    pub fn draw(&self, ui: &mut Ui, iphone: bool) {
        let a = ui.area();
        let t = *ui.theme();
        let c = ui.canvas();
        // A cross of keycaps on the left, lit while held.
        let cap = t.m.row_h;
        let gap = t.m.gap;
        let cx0 = a.x + t.m.pad + cap + gap;
        let cy0 = a.y + t.m.margin_v + cap + gap;
        let caps = [
            ("f", 0, cx0, cy0 - cap - gap),
            ("x", 1, cx0, cy0 + cap + gap),
            ("z", 2, cx0 - cap - gap, cy0),
            ("c", 3, cx0 + cap + gap, cy0),
        ];
        for (label, i, x, y) in caps {
            let r = Rect::new(x, y, cap, cap);
            let (fill, ink) = if self.held[i] {
                (t.c.accent, t.c.accent_fg)
            } else {
                (t.c.surface, t.c.fg)
            };
            c.round_rect(r, t.m.radius, fill);
            widgets::text_centre(c, t.f.small_bold, r, label, ink);
        }
        let pointer = Rect::new(cx0, cy0, cap, cap);
        gh0st_app::gfx::icon::draw(
            c,
            "mouse-pointer-2",
            gh0st_app::gfx::icon::IconSize::Bar,
            pointer.x + (cap - 16) / 2,
            pointer.y + (cap - 16) / 2,
            t.c.muted,
        );
        // What the other keys do, on the right.
        let x = cx0 + 2 * (cap + gap) + t.m.pad * 2;
        let lines: [(&str, &str, u8); 5] = [
            ("Space", "click, drag", 1),
            ("Bksp", "right click", 2),
            ("p", "middle click", 4),
            ("r v", "scroll", 0),
            ("Shift", "faster", 0),
        ];
        let mut y = a.y + t.m.margin_v;
        for (keys, what, bit) in lines {
            let style = if bit != 0 && self.buttons & bit != 0 {
                Pill::Main
            } else {
                Pill::Normal
            };
            let end = widgets::pills(c, &t, x, y, keys, style);
            let r = Rect::new(end + t.m.gap * 2, y, a.right() - end, t.m.pill_h);
            widgets::text_in(c, t.f.small, r, what, t.c.muted);
            y += t.m.keys_line_h + t.m.gap;
        }
        if iphone {
            let r = Rect::new(
                a.x + t.m.pad,
                a.bottom() - t.m.pill_h - t.m.margin_v,
                a.w - 2 * t.m.pad,
                t.m.pill_h,
            );
            widgets::text_in(c, t.f.small, r, crate::words::IPHONE_POINTER, t.c.warning);
            return;
        }
        let hints = [
            Hint {
                key: "Fn\u{a0}Space",
                label: "modes",
            },
            Hint {
                key: "Fn\u{a0}h",
                label: "keys",
            },
        ];
        let w = widgets::key_row_w(&t, &hints);
        widgets::key_row(
            c,
            &t,
            a.x + (a.w - w) / 2,
            a.bottom() - t.m.pill_h - t.m.margin_v,
            &hints,
        );
    }
}
