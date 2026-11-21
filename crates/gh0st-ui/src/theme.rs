//! Colour, geometry and type in one place. Widgets read everything from here
//! and hardcode nothing, so a palette or a size changes in one line.

use crate::color::{Color, hex};
use crate::font::{self, Font};

#[derive(Clone, Copy)]
pub struct Palette {
    pub name: &'static str,
    pub bg: Color,
    /// Bars, rows and tiles that sit on `bg`.
    pub surface: Color,
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    /// Text on an `accent` fill.
    pub accent_fg: Color,
    pub warning: Color,
    pub critical: Color,
    pub ok: Color,
    pub border: Color,
    /// Badge tints for built-in apps, so the grid is not one colour.
    pub tints: [Color; 6],
}

#[derive(Clone, Copy)]
pub struct Metrics {
    pub width: i32,
    pub height: i32,
    pub bar_h: i32,
    pub hint_h: i32,
    /// Text inset inside the bars.
    pub pad: i32,
    /// Content inset from the screen edges, left and right.
    pub margin: i32,
    /// Content inset below the status bar and above the hint bar.
    pub margin_v: i32,
    /// Space between tiles.
    pub gap: i32,
    /// Badges and rows.
    pub radius: i32,
    /// Selection around a tile, so it reads as one shape with the badge.
    pub radius_large: i32,
    /// Key pills.
    pub radius_small: i32,
    pub cols: i32,
    pub rows: i32,
    /// App badge on a grid tile.
    pub badge: i32,
    /// App badge on a list row.
    pub badge_small: i32,
    /// Home list row: badge, name and a second line.
    pub app_row_h: i32,
    /// Settings row: icon, label and a value.
    pub row_h: i32,
    /// A readout line on Network and About.
    pub line_h: i32,
    /// Top-bar slots: the mark, then one per open app.
    pub slot_w: i32,
    pub slot_h: i32,
    pub slot_gap: i32,
    /// An app's picture inside a slot.
    pub slot_icon: i32,
    /// Switcher cards across the body.
    pub cards: i32,
    /// Centres of the five bottom slots. The screen sits over the number keys
    /// 4 to 8, so each slot is above its key. Stock's guide gives these.
    pub softkey_x: [i32; 5],
}

#[derive(Clone, Copy)]
pub struct Fonts {
    pub small: Font,
    pub small_bold: Font,
    pub body: Font,
    pub body_bold: Font,
    pub title: Font,
    pub big: Font,
    pub mono: Font,
    pub mono_bold: Font,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub c: Palette,
    pub m: Metrics,
    pub f: Fonts,
}

/// pt35's Mint on charcoal, kept so both devices look like one family.
pub const MINT: Palette = Palette {
    name: "Mint",
    bg: hex("#0a1310"),
    surface: hex("#14231d"),
    fg: hex("#dcf3e5"),
    muted: hex("#7aa48e"),
    accent: hex("#3ddc97"),
    accent_fg: hex("#06130d"),
    warning: hex("#e8c468"),
    critical: hex("#ef6b73"),
    ok: hex("#3ddc97"),
    border: hex("#254236"),
    tints: [
        hex("#3ddc97"),
        hex("#5fd0e8"),
        hex("#e8c468"),
        hex("#ef6b73"),
        hex("#b48ef0"),
        hex("#f0a35e"),
    ],
};

pub const NORD: Palette = Palette {
    name: "Nord",
    bg: hex("#2e3440"),
    surface: hex("#3b4252"),
    fg: hex("#eceff4"),
    muted: hex("#a3adc2"),
    accent: hex("#88c0d0"),
    accent_fg: hex("#2e3440"),
    warning: hex("#ebcb8b"),
    critical: hex("#bf616a"),
    ok: hex("#a3be8c"),
    border: hex("#4c566a"),
    tints: [
        hex("#88c0d0"),
        hex("#a3be8c"),
        hex("#ebcb8b"),
        hex("#bf616a"),
        hex("#b48ead"),
        hex("#d08770"),
    ],
};

pub const PAPER: Palette = Palette {
    name: "Paper",
    bg: hex("#f4f1ea"),
    surface: hex("#e8e3d8"),
    fg: hex("#1f2421"),
    muted: hex("#5e665f"),
    accent: hex("#1f7a55"),
    accent_fg: hex("#f4f1ea"),
    warning: hex("#9a6a00"),
    critical: hex("#b3363f"),
    ok: hex("#1f7a55"),
    border: hex("#c9c2b3"),
    tints: [
        hex("#1f7a55"),
        hex("#1d6f8a"),
        hex("#9a6a00"),
        hex("#b3363f"),
        hex("#6b4fa0"),
        hex("#b0582a"),
    ],
};

pub const PALETTES: &[Palette] = &[MINT, NORD, PAPER];

/// A palette by its name, any case: the value of `gh0st.palette`.
pub fn palette(name: &str) -> Option<Palette> {
    PALETTES
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .copied()
}

/// 320x170: an 18 px bar on top, an 18 px hint bar below, 134 px between. The
/// grid inside that is 4x2 tiles of 74x60 with 4 px gaps and a 6/5 px margin.
/// Bar slots are 14x14, so a 12 px icon keeps a 1 px side and top pad.
pub const CARDPUTER: Metrics = Metrics {
    width: 320,
    height: 170,
    bar_h: 18,
    hint_h: 18,
    pad: 8,
    margin: 6,
    margin_v: 5,
    gap: 4,
    radius: 3,
    radius_large: 4,
    radius_small: 2,
    cols: 4,
    rows: 2,
    badge: 36,
    badge_small: 24,
    app_row_h: 33,
    row_h: 26,
    line_h: 15,
    slot_w: 14,
    slot_h: 14,
    slot_gap: 4,
    slot_icon: 12,
    cards: 3,
    softkey_x: [49, 105, 161, 217, 273],
};

pub const FONTS: Fonts = Fonts {
    small: &font::SANS_13,
    small_bold: &font::SANS_13_BOLD,
    body: &font::SANS_15,
    body_bold: &font::SANS_15_BOLD,
    title: &font::SANS_17_BOLD,
    big: &font::SANS_24_BOLD,
    mono: &font::MONO_10,
    mono_bold: &font::MONO_10_BOLD,
};

impl Theme {
    pub const fn new(c: Palette) -> Theme {
        Theme {
            c,
            m: CARDPUTER,
            f: FONTS,
        }
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::new(MINT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::contrast;

    #[test]
    fn every_palette_keeps_text_readable() {
        for p in PALETTES {
            assert!(contrast(p.fg, p.bg) >= 7.0, "{} fg", p.name);
            assert!(contrast(p.fg, p.surface) >= 4.5, "{} fg on surface", p.name);
            assert!(contrast(p.muted, p.bg) >= 3.0, "{} muted", p.name);
            assert!(
                contrast(p.accent_fg, p.accent) >= 4.5,
                "{} accent text",
                p.name
            );
        }
    }
}
