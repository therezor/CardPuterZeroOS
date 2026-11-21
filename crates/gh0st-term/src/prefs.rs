//! The terminal's settings page, opened from the menu or `s` in scroll mode. Every
//! change saves at once; the terminal reads them back when the page closes.

use crate::view::Family;
use gh0st_app::prelude::*;

pub mod key {
    pub const FONT: &str = "gh0st.term.font";
    pub const ZOOM: &str = "gh0st.term.zoom";
    pub const MODE: &str = "gh0st.term.mode";
    pub const WINDOW: &str = "gh0st.term.window";
    pub const BAR: &str = "gh0st.term.bar";
    pub const HISTORY: &str = "gh0st.term.history_kb";
    pub const ALT_ESC: &str = "gh0st.term.alt_esc";
    pub const CURSOR: &str = "gh0st.term.cursor";
}

pub const WINDOWS: &[&str] = &["80x24", "100x30", "120x40", "132x43"];
pub const MODES: &[&str] = &["magnify", "fit"];
pub const HISTORY_KB: &[i64] = &[256, 1024, 4096];
pub const CURSORS: &[&str] = &["block", "underline", "bar"];

const ROWS: usize = 8;

pub struct Prefs {
    list: List,
    s: Option<Settings>,
}

impl Prefs {
    pub fn new() -> Prefs {
        Prefs {
            list: List::new(),
            s: None,
        }
    }
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs::new()
    }
}

/// Steps `cur` through `all` by `d`, wrapping.
fn step<T: PartialEq + Copy>(all: &[T], cur: T, d: i32) -> T {
    let i = all.iter().position(|&x| x == cur).unwrap_or(0) as i32;
    all[(i + d).rem_euclid(all.len() as i32) as usize]
}

impl Page for Prefs {
    fn title(&self) -> &str {
        "Terminal"
    }

    fn start(&mut self, cx: &mut Cx) {
        self.s = Some(Settings::from(cx.settings()));
    }

    fn draw(&self, ui: &mut Ui) {
        let Some(s) = &self.s else {
            return;
        };
        let family = Family::from_key(&s.font);
        let faces = family.faces();
        let face = &faces[s.zoom.min(faces.len() - 1)];
        let cell = format!("{}x{}", face.regular.measure("M"), face.regular.line);
        let history = match s.history_kb {
            kb if kb >= 1024 => format!("{} MB", kb / 1024),
            kb => format!("{kb} KB"),
        };
        let rows = [
            Row::choice(
                "Font",
                match family {
                    Family::Terminus => "Terminus",
                    Family::Fixed => "Fixed",
                },
            ),
            Row::choice("Cell size", &cell),
            Row::choice("Grid", if s.mode == "fit" { "Fit" } else { "Magnify" }),
            Row::choice("Magnify grid", &s.window_size),
            Row::toggle("Top bar", s.bar),
            Row::choice("History", &history),
            Row::choice("Cursor", &s.cursor),
            Row::toggle("Alt sends Esc", s.alt_esc),
        ];
        debug_assert_eq!(rows.len(), ROWS);
        ui.list(&self.list, &rows);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        let Some(action) = self.list.key(key) else {
            return;
        };
        let (i, d) = match action {
            Action::Change(i, d) => (i, d),
            Action::Open(i) => (i, 1),
        };
        let s = cx.settings();
        let cur = Settings::from(s);
        match i {
            0 => {
                let next = if Family::from_key(&cur.font) == Family::Terminus {
                    Family::Fixed
                } else {
                    Family::Terminus
                };
                s.set_str(key::FONT, next.key());
                s.set_int(key::ZOOM, 0);
            }
            1 => {
                let n = Family::from_key(&cur.font).faces().len() as i64;
                s.set_int(key::ZOOM, (cur.zoom as i64 + d as i64).rem_euclid(n));
            }
            2 => s.set_str(key::MODE, step(MODES, cur.mode.as_str(), d)),
            3 => s.set_str(key::WINDOW, step(WINDOWS, cur.window_size.as_str(), d)),
            4 => s.set_bool(key::BAR, !cur.bar),
            5 => s.set_int(key::HISTORY, step(HISTORY_KB, cur.history_kb, d)),
            6 => s.set_str(key::CURSOR, step(CURSORS, cur.cursor.as_str(), d)),
            7 => s.set_bool(key::ALT_ESC, !cur.alt_esc),
            _ => {}
        }
        self.s = Some(Settings::from(s));
    }
}

/// The terminal's settings, with their defaults.
pub struct Settings {
    pub font: String,
    pub zoom: usize,
    /// `magnify`, `fit` or `window`.
    pub mode: String,
    pub window_size: String,
    pub bar: bool,
    pub history_kb: i64,
    pub cursor: String,
    pub alt_esc: bool,
}

impl Settings {
    pub fn from(s: &gh0st_app::Settings) -> Settings {
        let window_size = s
            .get_str(key::WINDOW)
            .filter(|w| WINDOWS.contains(w))
            .unwrap_or(WINDOWS[0])
            .to_string();
        Settings {
            font: s.get_str(key::FONT).unwrap_or("terminus").to_string(),
            zoom: s.get_int(key::ZOOM).unwrap_or(0).max(0) as usize,
            mode: s
                .get_str(key::MODE)
                .filter(|m| MODES.contains(m))
                .unwrap_or(MODES[0])
                .to_string(),
            window_size,
            bar: s.get_bool(key::BAR).unwrap_or(true),
            history_kb: s.get_int(key::HISTORY).unwrap_or(1024).clamp(64, 16 * 1024),
            cursor: s
                .get_str(key::CURSOR)
                .filter(|c| CURSORS.contains(c))
                .unwrap_or("block")
                .to_string(),
            alt_esc: s.get_bool(key::ALT_ESC).unwrap_or(true),
        }
    }
}
