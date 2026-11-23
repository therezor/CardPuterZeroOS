//! Colour, geometry and type in one place. Widgets read everything from here
//! and hardcode nothing, so a palette or a size changes in one line.

use crate::color::{Color, hex, mix};
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
    /// Tones: neutral notices, take care, errors and loss, done.
    pub info: Color,
    pub warning: Color,
    pub critical: Color,
    pub ok: Color,
    /// Lines, tracks, key pills and chip edges.
    pub border: Color,
    /// Badge tints for apps that are not ours, so the grid is not one colour.
    pub tints: [Color; 6],
    /// The terminal's 16 ANSI colours: black, red, green, yellow, blue,
    /// magenta, cyan, white, then the bright eight.
    pub ansi: [Color; 16],
}

impl Palette {
    /// Labels that are not selected: softer than `fg`, clearer than `muted`.
    pub fn soft(&self) -> Color {
        mix(self.muted, self.fg, 150)
    }

    /// Things that can't be used now: about 3:1 on `bg`.
    pub fn disabled(&self) -> Color {
        mix(self.bg, self.muted, 180)
    }

    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Info => self.info,
            Tone::Ok => self.ok,
            Tone::Warning => self.warning,
            Tone::Critical => self.critical,
        }
    }
}

/// What a toast, banner, tag or row icon says about itself. Apps name a tone,
/// never a colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Info,
    Ok,
    Warning,
    Critical,
}

impl Tone {
    /// The icon a toast or banner of this tone shows.
    pub fn icon(self) -> &'static str {
        match self {
            Tone::Info => "info",
            Tone::Ok => "circle-check",
            Tone::Warning => "triangle-alert",
            Tone::Critical => "circle-x",
        }
    }
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
    /// The same for the home grid and the switcher's cards, which need the
    /// height.
    pub grid_v: i32,
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
    /// An app menu row, and a section heading in it.
    pub menu_row_h: i32,
    pub menu_head_h: i32,
    /// A line of the Keys screen, and the widest its key column grows.
    pub keys_line_h: i32,
    pub keys_col_max: i32,
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
    /// An action chip, a tab on the screen's bottom edge: its height, its
    /// top corners, the space each side of its ink, and the least that may
    /// shrink to when neighbours crowd it.
    pub chip_h: i32,
    pub chip_r: i32,
    pub chip_pad: i32,
    /// How far a chip's bar, wash and fill stop short of each side, so
    /// neighbouring chips read apart.
    pub chip_inset: i32,
    /// A key pill in the hint bar, a card or a message.
    pub pill_h: i32,
    pub toast_h: i32,
    /// A card's space from its edge to the ink, top and bottom, and between
    /// its blocks: title, text, field, keys.
    pub card_pad: i32,
    pub card_gap: i32,
    /// A card's ink from its sides, and a dialog card's space from the
    /// screen's sides.
    pub card_inset: i32,
    pub dialog_margin: i32,
    /// What a scrolled list gives up on its right for the scrollbar.
    pub scrollbar_w: i32,
    /// A form's text field, its label above, a line under it that says
    /// what is wrong, and the space between two fields.
    pub field_h: i32,
    pub field_label_h: i32,
    pub field_note_h: i32,
    pub field_gap: i32,
    /// The lock screen: its password field's width, and the space between
    /// the lock, the time, the date and the field.
    pub lock_field_w: i32,
    pub lock_gap: i32,
    /// A full-screen page's side rails, beside a 4:3 picture as tall as the
    /// screen. The menu chip of `Bars::full` stands in the right one.
    pub rail_w: i32,
    /// Between the tiles of a grid of pictures (`gh0st_app::Tiles`).
    pub tile_gap: i32,
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
    info: hex("#5fd0e8"),
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
    ansi: [
        hex("#1f2b26"),
        hex("#ef6b73"),
        hex("#3ddc97"),
        hex("#e8c468"),
        hex("#5fa8e8"),
        hex("#b48ef0"),
        hex("#5fd0e8"),
        hex("#c8d8cf"),
        hex("#4f665a"),
        hex("#ff8f96"),
        hex("#72f0b8"),
        hex("#f5d88a"),
        hex("#8cc4f5"),
        hex("#cdb0f7"),
        hex("#8ce2f2"),
        hex("#f2fbf6"),
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
    info: hex("#81a1c1"),
    warning: hex("#ebcb8b"),
    // Nord's red lifted a step, so a danger word reads on `surface`.
    critical: hex("#d0737c"),
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
    ansi: [
        hex("#3b4252"),
        hex("#bf616a"),
        hex("#a3be8c"),
        hex("#ebcb8b"),
        hex("#81a1c1"),
        hex("#b48ead"),
        hex("#88c0d0"),
        hex("#e5e9f0"),
        hex("#7b88a1"),
        hex("#d08770"),
        hex("#b9d2a2"),
        hex("#f0d9a8"),
        hex("#9cb7d3"),
        hex("#c7a5c0"),
        hex("#8fbcbb"),
        hex("#eceff4"),
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
    info: hex("#1d6f8a"),
    // Darker than pt35's, so "Press again" and warning icons read on the
    // light ground.
    warning: hex("#7d5600"),
    critical: hex("#b3363f"),
    ok: hex("#1f7a55"),
    border: hex("#b3ab9a"),
    tints: [
        hex("#1f7a55"),
        hex("#1d6f8a"),
        hex("#9a6a00"),
        hex("#b3363f"),
        hex("#6b4fa0"),
        hex("#b0582a"),
    ],
    ansi: [
        hex("#1f2421"),
        hex("#b3363f"),
        hex("#1f7a55"),
        hex("#8a5f00"),
        hex("#1d5f9a"),
        hex("#6b4fa0"),
        hex("#1d6f8a"),
        hex("#8f8a7f"),
        hex("#5e665f"),
        hex("#d0454f"),
        hex("#2a9a6b"),
        hex("#a87400"),
        hex("#2b74b8"),
        hex("#8463bd"),
        hex("#2a8aa8"),
        hex("#6f6a60"),
    ],
};

/// Near-black and pale violet: the reward for finishing setup's training,
/// "for people who were never here".
pub const PHANTOM: Palette = Palette {
    name: "Phantom",
    bg: hex("#0b0a10"),
    surface: hex("#17151f"),
    fg: hex("#e9e4f5"),
    muted: hex("#9a90b5"),
    accent: hex("#c3a6ff"),
    accent_fg: hex("#140f22"),
    info: hex("#8fb8ff"),
    warning: hex("#e8c468"),
    critical: hex("#ff7a8a"),
    ok: hex("#8fe3b0"),
    border: hex("#2c2838"),
    tints: [
        hex("#c3a6ff"),
        hex("#8fb8ff"),
        hex("#e8c468"),
        hex("#ff7a8a"),
        hex("#8fe3b0"),
        hex("#f0a35e"),
    ],
    ansi: [
        hex("#1d1a26"),
        hex("#ff7a8a"),
        hex("#8fe3b0"),
        hex("#e8c468"),
        hex("#8fb8ff"),
        hex("#c3a6ff"),
        hex("#7fd8e6"),
        hex("#d6d0e6"),
        hex("#736b8f"),
        hex("#ff9daa"),
        hex("#aef0c8"),
        hex("#f3d98c"),
        hex("#b0ceff"),
        hex("#d9c6ff"),
        hex("#a3e6f0"),
        hex("#f6f3fc"),
    ],
};

pub const PALETTES: &[Palette] = &[MINT, NORD, PAPER];

/// Built-ins a picker lists only once earned: Phantom, after setup's
/// training (`gh0st.tour_done`). `gh0st.palette` finds them any time.
pub const EARNED: &[Palette] = &[PHANTOM];

/// A built-in palette by its name, any case.
pub fn palette(name: &str) -> Option<Palette> {
    PALETTES
        .iter()
        .chain(EARNED)
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .copied()
}

/// Where custom colour schemes live, the user's first: `<name>.palette`.
pub fn palette_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::Path::new(&home).join(".config/cardputerzero/palettes"));
    }
    dirs.push("/usr/share/gh0st/palettes".into());
    dirs
}

/// The palette `gh0st.palette` names, or Mint.
pub fn named(name: Option<&str>) -> Palette {
    name.and_then(find).unwrap_or(MINT)
}

/// The value of `gh0st.palette`: a built-in, else a scheme file by that
/// name. None if there is neither or the file is unreadable on screen.
pub fn find(name: &str) -> Option<Palette> {
    if let Some(p) = palette(name) {
        return Some(p);
    }
    if name.is_empty() || name.contains(['/', '.']) {
        return None;
    }
    palette_dirs().iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join(format!("{name}.palette"))).ok()?;
        parse_palette(&text, name).ok()
    })
}

/// Every palette there is: the built-ins, then each scheme file that loads,
/// by name. For a picker.
pub fn all() -> Vec<Palette> {
    let mut out = PALETTES.to_vec();
    for d in palette_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                n.strip_suffix(".palette").map(str::to_string)
            })
            .collect();
        names.sort();
        for n in names {
            if out.iter().any(|p| p.name.eq_ignore_ascii_case(&n)) {
                continue;
            }
            if let Some(p) = find(&n) {
                out.push(p);
            }
        }
    }
    out
}

/// Reads a scheme: one `role = #rrggbb` a line, `#` comments. `base = nord`
/// starts from a built-in (Mint if not given); any role left out keeps the
/// base's. Roles: bg surface fg muted accent accent_fg info warning critical
/// ok border, tint1 to tint6, ansi0 to ansi15. Refused when its text would
/// not read on its own background.
pub fn parse_palette(text: &str, name: &str) -> Result<Palette, String> {
    let lines: Vec<(&str, &str)> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        // The value is the first word: what follows it is a comment.
        .map(|(k, v)| (k.trim(), v.split_whitespace().next().unwrap_or("")))
        .collect();
    let base = lines
        .iter()
        .find(|(k, _)| *k == "base")
        .map(|(_, v)| palette(v).ok_or(format!("no built-in palette {v}")))
        .transpose()?
        .unwrap_or(MINT);
    let mut p = base;
    p.name = intern(name);
    for (k, v) in lines {
        if k == "base" {
            continue;
        }
        let c = crate::color::parse_hex(v).ok_or(format!("{k}: {v} is not #rrggbb"))?;
        let slot = match k {
            "bg" => &mut p.bg,
            "surface" => &mut p.surface,
            "fg" => &mut p.fg,
            "muted" => &mut p.muted,
            "accent" => &mut p.accent,
            "accent_fg" => &mut p.accent_fg,
            "info" => &mut p.info,
            "warning" => &mut p.warning,
            "critical" => &mut p.critical,
            "ok" => &mut p.ok,
            "border" => &mut p.border,
            _ => match k.strip_prefix("tint").and_then(|n| n.parse::<usize>().ok()) {
                Some(n @ 1..=6) => &mut p.tints[n - 1],
                _ => match k.strip_prefix("ansi").and_then(|n| n.parse::<usize>().ok()) {
                    Some(n @ 0..=15) => &mut p.ansi[n],
                    // Unknown keys are left for newer versions.
                    _ => continue,
                },
            },
        };
        *slot = c;
    }
    use crate::color::contrast;
    if contrast(p.fg, p.bg) < 4.5 || contrast(p.fg, p.surface) < 3.0 {
        return Err("fg does not read on bg or surface".into());
    }
    if contrast(p.accent_fg, p.accent) < 3.0 {
        return Err("accent_fg does not read on accent".into());
    }
    Ok(p)
}

/// A scheme's name for the life of the process. Each name is kept once, so
/// loading the same scheme again costs nothing.
fn intern(name: &str) -> &'static str {
    use std::sync::Mutex;
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(n);
    n
}

/// 320x170: an 18 px bar on top, a 20 px action bar below, 132 px between:
/// five 26 px rows. The grid inside that is 4x2 tiles of 74x61 with 4 px gaps
/// and a 6/3 px margin.
/// Bar slots are 14x14, so a 12 px icon keeps a 1 px side and top pad.
pub const CARDPUTER: Metrics = Metrics {
    width: 320,
    height: 170,
    bar_h: 18,
    hint_h: 20,
    pad: 8,
    margin: 6,
    margin_v: 5,
    grid_v: 3,
    gap: 4,
    radius: 3,
    radius_large: 4,
    radius_small: 2,
    cols: 4,
    rows: 2,
    badge: 34,
    badge_small: 24,
    app_row_h: 40,
    row_h: 26,
    line_h: 18,
    menu_row_h: 22,
    menu_head_h: 18,
    keys_line_h: 17,
    keys_col_max: 92,
    slot_w: 14,
    slot_h: 14,
    slot_gap: 4,
    slot_icon: 12,
    cards: 3,
    softkey_x: [49, 105, 161, 217, 273],
    // A top edge, two clear rows, a 10 px capital with its shading, then
    // room for descenders and the 3 px tick that points at the key. One row
    // of the 20 px bar stays clear above it.
    chip_h: 19,
    chip_r: 3,
    chip_pad: 7,
    chip_inset: 3,
    pill_h: 15,
    toast_h: 20,
    card_pad: 10,
    card_gap: 6,
    card_inset: 12,
    dialog_margin: 22,
    scrollbar_w: 4,
    field_h: 22,
    field_label_h: 17,
    field_note_h: 18,
    field_gap: 6,
    lock_field_w: 172,
    lock_gap: 7,
    // (320 - 170 * 4 / 3) / 2: what a 226x170 picture leaves each side.
    rail_w: 47,
    tile_gap: 3,
};

/// The panel while the screen is off. The backlight never goes fully out,
/// so this is what shows, on every palette.
pub const PANEL_OFF: Color = crate::color::rgb(0, 0, 0);

/// Marks over a photo or a video: light ink on a dark plate on every
/// palette, since the picture's colours are its own.
pub const ON_PICTURE: Color = crate::color::rgb(255, 255, 255);
pub const PICTURE_PLATE: Color = crate::color::rgb(0, 0, 0);

/// A QR code is dark on light on every palette: that is what cameras read.
pub const QR_DARK: Color = crate::color::rgb(0, 0, 0);
pub const QR_LIGHT: Color = crate::color::rgb(255, 255, 255);
/// Light modules around the code. The spec asks for 4; 2 reads on a lit
/// screen and leaves room for bigger modules.
pub const QR_QUIET: i32 = 2;

/// How far dim terminal text fades toward its background, of 255.
pub const TERM_DIM: u8 = 140;
/// The terminal's underline and bar cursors, and its scroll mark, in px.
pub const TERM_CURSOR_W: i32 = 2;
pub const TERM_MARK_W: i32 = 2;

/// A terminal zoom level: a face, its bold if baked, and Nerd Font icons
/// sized two cells wide. Without a bold the terminal draws bold by striking
/// the glyph twice.
#[derive(Clone, Copy)]
pub struct TermFace {
    pub regular: Font,
    pub bold: Option<Font>,
    pub icons: Font,
}

const fn term_face(regular: Font, bold: Option<Font>, icons: Font) -> TermFace {
    TermFace {
        regular,
        bold,
        icons,
    }
}

/// The cell size, in px, the terminal tells programs for images: Terminus
/// 12's. Images draw 1:1 at that size and scale at the others. It stays put
/// when the text size changes, so magnify sends no SIGWINCH.
pub const TERM_IMAGE_CELL: (i32, i32) = (6, 12);

/// Terminus, the terminal's face. A bitmap face, so crisp at every size:
/// anti-aliased faces blurred on the panel, as nothing hints them. Cells
/// 6x12, 8x14, 8x16, 10x18, 10x20.
pub const TERM_TERMINUS: &[TermFace] = &[
    term_face(
        &font::TERMINUS_12,
        Some(&font::TERMINUS_12_BOLD),
        &font::TERMINUS_12_ICONS,
    ),
    term_face(
        &font::TERMINUS_14,
        Some(&font::TERMINUS_14_BOLD),
        &font::TERMINUS_14_ICONS,
    ),
    term_face(
        &font::TERMINUS_16,
        Some(&font::TERMINUS_16_BOLD),
        &font::TERMINUS_16_ICONS,
    ),
    term_face(
        &font::TERMINUS_18,
        Some(&font::TERMINUS_18_BOLD),
        &font::TERMINUS_18_ICONS,
    ),
    term_face(
        &font::TERMINUS_20,
        Some(&font::TERMINUS_20_BOLD),
        &font::TERMINUS_20_ICONS,
    ),
];

pub const FONTS: Fonts = Fonts {
    small: &font::SANS_15,
    small_bold: &font::SANS_15_BOLD,
    body: &font::SANS_17,
    body_bold: &font::SANS_17_BOLD,
    title: &font::SANS_19_BOLD,
    big: &font::SANS_26_BOLD,
    mono: &font::MONO_12,
    mono_bold: &font::MONO_12_BOLD,
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
    fn a_scheme_starts_from_its_base_and_overrides_roles() {
        let p = parse_palette(
            "# dusk\nbase = nord\naccent = #f0a35e # orange\ntint2 = #112233\nwhat = #000000\n",
            "Dusk",
        )
        .unwrap();
        assert_eq!(p.name, "Dusk");
        assert_eq!(p.bg, NORD.bg);
        assert_eq!(p.accent, hex("#f0a35e"));
        assert_eq!(p.tints[1], hex("#112233"));
        assert!(
            parse_palette("bg = #dcf3e5\n", "Bad").is_err(),
            "fg on a light bg"
        );
        assert!(parse_palette("accent = red\n", "Bad").is_err());
        assert!(find("../etc/passwd").is_none());
    }

    #[test]
    fn every_palette_keeps_text_readable() {
        for p in PALETTES.iter().chain(EARNED) {
            assert!(contrast(p.fg, p.bg) >= 7.0, "{} fg", p.name);
            assert!(contrast(p.fg, p.surface) >= 4.5, "{} fg on surface", p.name);
            assert!(contrast(p.muted, p.bg) >= 3.0, "{} muted", p.name);
            assert!(contrast(p.disabled(), p.bg) >= 2.8, "{} disabled", p.name);
            for tone in [Tone::Info, Tone::Ok, Tone::Warning, Tone::Critical] {
                let c = p.tone(tone);
                assert!(
                    contrast(c, p.surface) >= 3.0,
                    "{} {tone:?} on surface",
                    p.name
                );
            }
            assert!(
                contrast(p.accent_fg, p.accent) >= 4.5,
                "{} accent text",
                p.name
            );
            // Black is a background colour in the terminal; the rest is text.
            for (i, &c) in p.ansi.iter().enumerate().filter(|&(i, _)| i != 0) {
                assert!(contrast(c, p.bg) >= 3.0, "{} ansi {i}", p.name);
            }
        }
    }
}
