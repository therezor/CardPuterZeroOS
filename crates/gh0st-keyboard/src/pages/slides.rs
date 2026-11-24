//! Slides mode: next and back, a black screen, starting the show, and a
//! timer that ticks only while it runs and the screen is in front.

use std::time::{Duration, Instant};

use gh0st_app::gfx::Rect;
use gh0st_app::gfx::widgets::{self, Hint};
use gh0st_app::prelude::*;

use crate::app::Shared;
use crate::hid::usage::{key, mods};
use crate::host::System;
use crate::words;

pub const NEXT: u32 = 300;
pub const PREV: u32 = 301;
pub const BLACK: u32 = 302;
pub const START: u32 = 303;
pub const TIMER: u32 = 304;
pub const RESET: u32 = 305;

#[derive(Default)]
pub struct Slides {
    pub slide: i32,
    pub black: bool,
    /// Time before the current run.
    pub past: Duration,
    pub since: Option<Instant>,
    tick: Option<Timer>,
}

impl Slides {
    pub fn new() -> Slides {
        Slides {
            slide: 1,
            ..Slides::default()
        }
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    pub fn elapsed(&self) -> Duration {
        self.past + self.since.map(|t| t.elapsed()).unwrap_or_default()
    }

    pub fn bars() -> Bars {
        Bars::top()
    }

    pub fn chips(&self, b: Bars) -> Bars {
        b.chip(
            5,
            Chip::new("square", words::BLACK).on(self.black).runs(BLACK),
        )
        .chip(6, Chip::new("play", words::START).main().runs(START))
        .chip(
            7,
            Chip::new("timer", words::TIMER)
                .on(self.running())
                .runs(TIMER),
        )
    }

    pub fn commands(c: &mut Commands) {
        c.group("Slides");
        c.add(Command::action(NEXT, words::NEXT_SLIDE).keys("Enter Space c"));
        c.add(Command::action(PREV, words::PREV_SLIDE).keys("Bksp z"));
        c.add(Command::action(BLACK, "Black screen").keys("b"));
        c.add(Command::action(START, "Start the show"));
        c.add(Command::action(TIMER, "Start or pause the timer").keys("t"));
        c.add(Command::action(RESET, words::RESET_TIMER).keys("T"));
    }

    fn tap(s: &mut Shared, m: u8, usage: u8, cx: &mut Cx) {
        let mut out = Vec::new();
        s.engine.tap(m, usage, &mut out);
        if !s.send(&out) {
            cx.toast(words::NOT_LIVE);
        }
    }

    fn run_timer(&mut self, on: bool, cx: &mut Cx) {
        match (on, self.since) {
            (true, None) => {
                self.since = Some(Instant::now());
                self.tick = Some(cx.every(1000));
                cx.keep_awake(true);
            }
            (false, Some(t)) => {
                self.past += t.elapsed();
                self.since = None;
                if let Some(tick) = self.tick.take() {
                    cx.cancel(tick);
                }
                cx.keep_awake(false);
            }
            _ => {}
        }
    }

    /// Runs a slides command. False when `id` isn't one.
    pub fn command(&mut self, s: &mut Shared, id: u32, cx: &mut Cx) -> bool {
        match id {
            NEXT => {
                Slides::tap(s, 0, key::RIGHT, cx);
                self.slide += 1;
            }
            PREV => {
                Slides::tap(s, 0, key::LEFT, cx);
                self.slide = (self.slide - 1).max(1);
            }
            BLACK => {
                Slides::tap(s, 0, key::B, cx);
                self.black = !self.black;
            }
            START => {
                // Keynote's Play on a Mac, F5 everywhere else.
                match s.system() {
                    System::Mac | System::Ios => {
                        Slides::tap(s, mods::LGUI | mods::LALT, key::P, cx)
                    }
                    _ => Slides::tap(s, 0, key::F5, cx),
                }
                self.slide = 1;
                self.black = false;
                self.run_timer(true, cx);
            }
            TIMER => {
                let on = !self.running();
                self.run_timer(on, cx);
            }
            RESET => {
                self.run_timer(false, cx);
                self.past = Duration::ZERO;
            }
            _ => return false,
        }
        true
    }

    pub fn timer(&self, t: Timer) -> bool {
        self.tick == Some(t)
    }

    /// Leaving the mode or the front: the clock keeps its time but stops
    /// waking the CPU.
    pub fn pause(&mut self, cx: &mut Cx) {
        self.run_timer(false, cx);
    }

    pub fn draw(&self, ui: &mut Ui) {
        let a = ui.area();
        let t = *ui.theme();
        let c = ui.canvas();
        let secs = self.elapsed().as_secs();
        let clock = format!("{}:{:02}", secs / 60, secs % 60);
        let ink = if self.running() { t.c.fg } else { t.c.muted };
        let big = Rect::new(a.x, a.y + t.m.margin_v, a.w, t.m.row_h + t.m.pad * 2);
        widgets::text_centre(c, t.f.big, big, &clock, ink);
        let mut line = words::slide(self.slide);
        if self.black {
            line.push_str(" \u{b7} black");
        }
        let r = Rect::new(a.x, big.bottom(), a.w, t.m.line_h);
        widgets::text_centre(c, t.f.body, r, &line, t.c.soft());
        let hints = [
            Hint {
                key: "Space",
                label: "next",
            },
            Hint {
                key: "Bksp",
                label: "back",
            },
        ];
        let w = widgets::key_row_w(&t, &hints);
        widgets::key_row(c, &t, a.x + (a.w - w) / 2, r.bottom() + t.m.pad, &hints);
    }
}
