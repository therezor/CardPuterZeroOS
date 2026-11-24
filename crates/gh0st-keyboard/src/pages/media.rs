//! Media mode: play, tracks, volume, and a phone camera's shutter (its
//! volume-up key). Every key is a command, so Help lists them.

use gh0st_app::gfx::Rect;
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::widgets::{self, Hint};
use gh0st_app::prelude::*;

use crate::app::Shared;
use crate::hid::usage::consumer;
use crate::words;

pub const PLAY: u32 = 200;
pub const PREV: u32 = 201;
pub const NEXT: u32 = 202;
pub const UP: u32 = 203;
pub const DOWN: u32 = 204;
pub const MUTE: u32 = 205;
pub const SHUTTER: u32 = 206;

pub fn commands(c: &mut Commands) {
    c.group("Media");
    c.add(Command::action(PLAY, words::PLAY).keys("Enter Space"));
    c.add(
        Command::action(PREV, "Previous track")
            .keys("z")
            .chip(5, "rewind", words::PREV),
    );
    c.add(
        Command::action(MUTE, words::MUTE)
            .keys("m")
            .chip(6, "volume-x", words::MUTE),
    );
    c.add(
        Command::action(NEXT, "Next track")
            .keys("c")
            .chip(7, "skip-forward", words::NEXT),
    );
    c.add(Command::action(UP, words::VOLUME_UP).keys("f").repeats());
    c.add(
        Command::action(DOWN, words::VOLUME_DOWN)
            .keys("x")
            .repeats(),
    );
    c.add(Command::action(SHUTTER, words::SHUTTER).keys("s"));
}

/// Runs a media command. False when `id` isn't one.
pub fn command(s: &mut Shared, id: u32, cx: &mut Cx) -> bool {
    let usage = match id {
        PLAY => consumer::PLAY_PAUSE,
        PREV => consumer::PREV,
        NEXT => consumer::NEXT,
        UP | SHUTTER => consumer::VOLUME_UP,
        DOWN => consumer::VOLUME_DOWN,
        MUTE => consumer::MUTE,
        _ => return false,
    };
    let mut out = Vec::new();
    s.engine.media_tap(usage, &mut out);
    if !s.send(&out) {
        cx.toast(words::NOT_LIVE);
    }
    true
}

pub fn draw(ui: &mut Ui) {
    let a = ui.area();
    let t = *ui.theme();
    let c = ui.canvas();
    // Three large marks: back, play, forward.
    let size = IconSize::Large as i32;
    let step = a.w / 4;
    let y = a.y + t.m.margin_v + t.m.gap;
    for (i, name) in ["rewind", "play", "skip-forward"].iter().enumerate() {
        let x = a.x + step * (i as i32 + 1) - size / 2;
        let ink = if i == 1 { t.c.accent } else { t.c.soft() };
        icon::draw(c, name, IconSize::Large, x, y, ink);
    }
    let r = Rect::new(a.x, y + size + t.m.gap, a.w, t.m.line_h);
    widgets::text_centre(c, t.f.body, r, words::PLAY, t.c.fg);
    let hints = [
        Hint {
            key: "Enter",
            label: "play",
        },
        Hint {
            key: "f x",
            label: "volume",
        },
        Hint {
            key: "s",
            label: "shutter",
        },
    ];
    let w = widgets::key_row_w(&t, &hints);
    widgets::key_row(c, &t, a.x + (a.w - w) / 2, r.bottom() + t.m.pad, &hints);
}
