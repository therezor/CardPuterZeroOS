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

    /// The tint of a row's icon square, the same for an icon everywhere, so
    /// Wi-Fi is one colour on every page. Common icons have one picked so
    /// neighbours on Settings' first page differ; the rest hash.
    pub fn icon_tint(&self, icon: &str) -> Color {
        let i = ICON_TINTS
            .iter()
            .find(|(name, _)| *name == icon)
            .map(|&(_, i)| i)
            .unwrap_or_else(|| {
                let h = icon.bytes().fold(0x811c_9dc5u32, |h, b| {
                    (h ^ b as u32).wrapping_mul(0x0100_0193)
                });
                h as usize
            });
        self.tints[i % self.tints.len()]
    }

    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Info => self.info,
            Tone::Ok => self.ok,
            Tone::Warning => self.warning,
            Tone::Critical => self.critical,
        }
    }

    /// The colour of a run of code. From the terminal's colours, which
    /// every palette already keeps readable on `bg`, so a scheme file
    /// colours code too.
    pub fn syntax(&self, role: Syntax) -> Color {
        match role {
            Syntax::Plain => self.fg,
            Syntax::Comment => self.muted,
            Syntax::Punct => self.soft(),
            Syntax::String => self.ansi[2],
            Syntax::Keyword => self.ansi[5],
            Syntax::Type => self.ansi[6],
            Syntax::Number => self.ansi[3],
            Syntax::Func => self.ansi[4],
            Syntax::Attr => self.ansi[11],
            Syntax::Heading => self.ansi[12],
            Syntax::Added => self.ansi[2],
            Syntax::Removed => self.ansi[1],
        }
    }
}

/// What a run of code or markup is, for its colour in an editor. Apps name
/// a role, never a colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Syntax {
    #[default]
    Plain,
    Comment,
    String,
    Keyword,
    Type,
    /// Numbers and constants: `42`, `true`, `None`.
    Number,
    /// Functions called or defined, and keys in data files.
    Func,
    Punct,
    /// Attributes, decorators, preprocessor lines, tags' attributes.
    Attr,
    /// Headings in prose, sections in config files.
    Heading,
    /// Lines a diff adds and removes.
    Added,
    Removed,
}

impl Syntax {
    pub const ALL: [Syntax; 12] = [
        Syntax::Plain,
        Syntax::Comment,
        Syntax::String,
        Syntax::Keyword,
        Syntax::Type,
        Syntax::Number,
        Syntax::Func,
        Syntax::Punct,
        Syntax::Attr,
        Syntax::Heading,
        Syntax::Added,
        Syntax::Removed,
    ];
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
    /// An app badge, and a dialog or toast card.
    pub badge_r: i32,
    /// How far a badge stands up off the page: the depth of its foot. A
    /// chip's foot and a row icon's are `chip_lift`.
    pub lift: i32,
    pub chip_lift: i32,
    pub card_r: i32,
    /// The tinted square behind a settings row's icon.
    pub row_icon: i32,
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
    /// An action chip, a pill over its key: its height, its corners, the
    /// space each side of its ink, and the room under it to the screen's
    /// edge.
    pub chip_h: i32,
    pub chip_r: i32,
    pub chip_pad: i32,
    pub chip_foot: i32,
    /// The short bar under a chip's word.
    pub chip_bar_w: i32,
    /// How far a pill stops short of its key's edges, so neighbouring
    /// chips read apart.
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
    /// Words over a picture sit on a dark plate this far around them.
    pub plate_pad: i32,
    /// Where a zoomed picture's view is: a map at most this big.
    pub map_w: i32,
    pub map_h: i32,
    /// A video's progress line over the buttons, and its room to them.
    pub track_h: i32,
    pub track_lift: i32,
}

#[derive(Clone, Copy)]
pub struct Fonts {
    /// Caption and Label: Terminus, a bitmap face, crisp at this size.
    pub small: Font,
    pub small_bold: Font,
    pub body: Font,
    pub body_bold: Font,
    pub title: Font,
    pub big: Font,
    pub mono: Font,
    pub mono_bold: Font,
    /// A chip's word: a smaller bitmap face, so a word of seven letters
    /// fits over its key.
    pub chip: Font,
    /// The lock screen's time: digits and a colon only.
    pub clock: Font,
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

/// Catppuccin's dark flavour: pastels on blue-grey.
pub const CATPPUCCIN_MOCHA: Palette = Palette {
    name: "Catppuccin Mocha",
    bg: hex("#1e1e2e"),
    surface: hex("#313244"),
    fg: hex("#cdd6f4"),
    muted: hex("#9399b2"),
    accent: hex("#cba6f7"),
    accent_fg: hex("#11111b"),
    info: hex("#89b4fa"),
    warning: hex("#f9e2af"),
    critical: hex("#f38ba8"),
    ok: hex("#a6e3a1"),
    border: hex("#45475a"),
    tints: [
        hex("#cba6f7"),
        hex("#89b4fa"),
        hex("#f9e2af"),
        hex("#f38ba8"),
        hex("#a6e3a1"),
        hex("#fab387"),
    ],
    ansi: [
        hex("#45475a"),
        hex("#f38ba8"),
        hex("#a6e3a1"),
        hex("#f9e2af"),
        hex("#89b4fa"),
        hex("#f5c2e7"),
        hex("#94e2d5"),
        hex("#a6adc8"),
        // Overlay 1, not Surface 2: bright black reads on the base.
        hex("#7f849c"),
        hex("#f37799"),
        hex("#89d88b"),
        hex("#ebd391"),
        hex("#74a8fc"),
        hex("#f2aede"),
        hex("#6bd7ca"),
        hex("#bac2de"),
    ],
};

/// Catppuccin's light flavour.
pub const CATPPUCCIN_LATTE: Palette = Palette {
    name: "Catppuccin Latte",
    bg: hex("#eff1f5"),
    surface: hex("#e6e9ef"),
    fg: hex("#4c4f69"),
    muted: hex("#6c6f85"),
    accent: hex("#8839ef"),
    accent_fg: hex("#eff1f5"),
    info: hex("#1e66f5"),
    // Yellow and green darkened a step, so they read on the light ground.
    warning: hex("#b77418"),
    critical: hex("#d20f39"),
    ok: hex("#3b9328"),
    border: hex("#bcc0cc"),
    tints: [
        hex("#8839ef"),
        hex("#1e66f5"),
        hex("#df8e1d"),
        hex("#d20f39"),
        hex("#40a02b"),
        hex("#fe640b"),
    ],
    // Green, yellow, pink and teal darkened a step for the light ground.
    // White and bright white are Overlay 2 and Subtext 1: text, not ground.
    ansi: [
        hex("#5c5f77"),
        hex("#d20f39"),
        hex("#3e9b2a"),
        hex("#be7919"),
        hex("#1e66f5"),
        hex("#c764ad"),
        hex("#179299"),
        hex("#7c7f93"),
        hex("#6c6f85"),
        hex("#de293e"),
        hex("#409a36"),
        hex("#b77b23"),
        hex("#456eff"),
        hex("#c466a6"),
        hex("#2a959e"),
        hex("#5c5f77"),
    ],
};

/// Dracula: purple and pink on blue-grey.
pub const DRACULA: Palette = Palette {
    name: "Dracula",
    bg: hex("#282a36"),
    // BG Light, not Selection, so the accent reads on a badge.
    surface: hex("#343746"),
    fg: hex("#f8f8f2"),
    // Comment lifted a step, so hints and disabled rows read.
    muted: hex("#8894ba"),
    accent: hex("#bd93f9"),
    accent_fg: hex("#282a36"),
    info: hex("#8be9fd"),
    warning: hex("#ffb86c"),
    critical: hex("#ff5555"),
    ok: hex("#50fa7b"),
    border: hex("#44475a"),
    tints: [
        hex("#bd93f9"),
        hex("#8be9fd"),
        hex("#f1fa8c"),
        hex("#ff5555"),
        hex("#ff79c6"),
        hex("#ffb86c"),
    ],
    ansi: [
        hex("#21222c"),
        hex("#ff5555"),
        hex("#50fa7b"),
        hex("#f1fa8c"),
        hex("#bd93f9"),
        hex("#ff79c6"),
        hex("#8be9fd"),
        hex("#f8f8f2"),
        hex("#6272a4"),
        hex("#ff6e6e"),
        hex("#69ff94"),
        hex("#ffffa5"),
        hex("#d6acff"),
        hex("#ff92df"),
        hex("#a4ffff"),
        hex("#ffffff"),
    ],
};

/// Gruvbox's warm retro dark.
pub const GRUVBOX_DARK: Palette = Palette {
    name: "Gruvbox Dark",
    bg: hex("#282828"),
    surface: hex("#3c3836"),
    fg: hex("#ebdbb2"),
    muted: hex("#a89984"),
    accent: hex("#fe8019"),
    accent_fg: hex("#282828"),
    info: hex("#83a598"),
    warning: hex("#fabd2f"),
    critical: hex("#fb4934"),
    ok: hex("#b8bb26"),
    border: hex("#504945"),
    tints: [
        hex("#fe8019"),
        hex("#83a598"),
        hex("#fabd2f"),
        hex("#fb4934"),
        hex("#d3869b"),
        hex("#8ec07c"),
    ],
    ansi: [
        hex("#3c3836"),
        // Red lifted a step to read on bg0.
        hex("#d23c36"),
        hex("#98971a"),
        hex("#d79921"),
        hex("#458588"),
        hex("#b16286"),
        hex("#689d6a"),
        hex("#a89984"),
        hex("#928374"),
        hex("#fb4934"),
        hex("#b8bb26"),
        hex("#fabd2f"),
        hex("#83a598"),
        hex("#d3869b"),
        hex("#8ec07c"),
        hex("#ebdbb2"),
    ],
};

/// Gruvbox's light, on its cream.
pub const GRUVBOX_LIGHT: Palette = Palette {
    name: "Gruvbox Light",
    bg: hex("#fbf1c7"),
    surface: hex("#ebdbb2"),
    fg: hex("#3c3836"),
    // fg3, so disabled rows still read.
    muted: hex("#665c54"),
    accent: hex("#af3a03"),
    accent_fg: hex("#fbf1c7"),
    info: hex("#076678"),
    // Faded yellow darkened a step to read on bg1.
    warning: hex("#a76d12"),
    critical: hex("#9d0006"),
    ok: hex("#79740e"),
    border: hex("#bdae93"),
    tints: [
        hex("#af3a03"),
        hex("#076678"),
        hex("#b57614"),
        hex("#9d0006"),
        hex("#8f3f71"),
        hex("#427b58"),
    ],
    // Neutral green, yellow and aqua darkened a step for the light ground.
    ansi: [
        hex("#fbf1c7"),
        hex("#cc241d"),
        hex("#8c8b18"),
        hex("#b27f1b"),
        hex("#458588"),
        hex("#b16286"),
        hex("#619263"),
        hex("#7c6f64"),
        hex("#928374"),
        hex("#9d0006"),
        hex("#79740e"),
        hex("#b57614"),
        hex("#076678"),
        hex("#8f3f71"),
        hex("#427b58"),
        hex("#3c3836"),
    ],
};

/// Rosé Pine: muted rose and gold on deep violet.
pub const ROSE_PINE: Palette = Palette {
    name: "Rosé Pine",
    bg: hex("#191724"),
    surface: hex("#26233a"),
    fg: hex("#e0def4"),
    muted: hex("#908caa"),
    accent: hex("#ebbcba"),
    accent_fg: hex("#191724"),
    info: hex("#c4a7e7"),
    warning: hex("#f6c177"),
    critical: hex("#eb6f92"),
    ok: hex("#9ccfd8"),
    border: hex("#403d52"),
    tints: [
        hex("#ebbcba"),
        hex("#9ccfd8"),
        hex("#f6c177"),
        hex("#eb6f92"),
        hex("#c4a7e7"),
        hex("#31748f"),
    ],
    ansi: [
        hex("#26233a"),
        hex("#eb6f92"),
        hex("#31748f"),
        hex("#f6c177"),
        hex("#9ccfd8"),
        hex("#c4a7e7"),
        hex("#ebbcba"),
        hex("#e0def4"),
        hex("#6e6a86"),
        hex("#eb6f92"),
        hex("#31748f"),
        hex("#f6c177"),
        hex("#9ccfd8"),
        hex("#c4a7e7"),
        hex("#ebbcba"),
        hex("#e0def4"),
    ],
};

/// Rosé Pine's light, on warm paper.
pub const ROSE_PINE_DAWN: Palette = Palette {
    name: "Rosé Pine Dawn",
    bg: hex("#faf4ed"),
    surface: hex("#f2e9e1"),
    fg: hex("#464261"),
    // Subtle, foam and gold darkened a step, so they read on the paper.
    muted: hex("#736f8c"),
    accent: hex("#286983"),
    accent_fg: hex("#faf4ed"),
    info: hex("#907aa9"),
    warning: hex("#b27728"),
    critical: hex("#b4637a"),
    ok: hex("#538e99"),
    border: hex("#dfdad9"),
    tints: [
        hex("#286983"),
        hex("#56949f"),
        hex("#ea9d34"),
        hex("#b4637a"),
        hex("#907aa9"),
        hex("#d7827e"),
    ],
    ansi: [
        hex("#f2e9e1"),
        hex("#b4637a"),
        hex("#286983"),
        hex("#be7f2a"),
        hex("#56949f"),
        hex("#907aa9"),
        hex("#c47673"),
        hex("#575279"),
        hex("#8f8a9b"),
        hex("#b4637a"),
        hex("#286983"),
        hex("#be7f2a"),
        hex("#56949f"),
        hex("#907aa9"),
        hex("#c47673"),
        hex("#575279"),
    ],
};

/// Solarized on its light base.
pub const SOLARIZED_LIGHT: Palette = Palette {
    name: "Solarized Light",
    bg: hex("#fdf6e3"),
    surface: hex("#eee8d5"),
    // base02, not base00: body text needs 7:1.
    fg: hex("#073642"),
    muted: hex("#586e75"),
    // Blue, cyan, green and yellow darkened a step to read on base2.
    accent: hex("#1b679c"),
    accent_fg: hex("#fdf6e3"),
    info: hex("#26938a"),
    warning: hex("#a77e00"),
    critical: hex("#dc322f"),
    ok: hex("#798b00"),
    border: hex("#93a1a1"),
    tints: [
        hex("#268bd2"),
        hex("#2aa198"),
        hex("#b58900"),
        hex("#dc322f"),
        // Violet darkened a step, so a badge's ink reads on it.
        hex("#6569b8"),
        hex("#cb4b16"),
    ],
    // Green, yellow, cyan and the light greys darkened a step. Solarized's
    // whites are this ground, so base00 and base01 stand in.
    ansi: [
        hex("#073642"),
        hex("#dc322f"),
        hex("#809300"),
        hex("#ae8400"),
        hex("#268bd2"),
        hex("#d33682"),
        hex("#289b92"),
        hex("#657b83"),
        hex("#002b36"),
        hex("#cb4b16"),
        hex("#586e75"),
        hex("#657b83"),
        hex("#7e8e90"),
        hex("#6c71c4"),
        hex("#838f8f"),
        hex("#586e75"),
    ],
};

/// Tokyo Night: neon blues on navy.
pub const TOKYO_NIGHT: Palette = Palette {
    name: "Tokyo Night",
    bg: hex("#1a1b26"),
    surface: hex("#292e42"),
    fg: hex("#c0caf5"),
    // dark5 lifted a step, so disabled rows read.
    muted: hex("#7b82a8"),
    accent: hex("#7aa2f7"),
    accent_fg: hex("#1a1b26"),
    info: hex("#7dcfff"),
    warning: hex("#e0af68"),
    critical: hex("#f7768e"),
    ok: hex("#9ece6a"),
    border: hex("#3b4261"),
    tints: [
        hex("#7aa2f7"),
        hex("#7dcfff"),
        hex("#e0af68"),
        hex("#f7768e"),
        hex("#bb9af7"),
        hex("#ff9e64"),
    ],
    ansi: [
        hex("#15161e"),
        hex("#f7768e"),
        hex("#9ece6a"),
        hex("#e0af68"),
        hex("#7aa2f7"),
        hex("#bb9af7"),
        hex("#7dcfff"),
        hex("#a9b1d6"),
        // Terminal black lifted, so bright black reads on the night bg.
        hex("#636983"),
        hex("#ff899d"),
        hex("#9fe044"),
        hex("#faba4a"),
        hex("#8db0ff"),
        hex("#c7a9ff"),
        hex("#a4daff"),
        hex("#c0caf5"),
    ],
};

/// Ours first, then the popular schemes by name. Each keeps its upstream
/// colours except where a role moved to pass the contrast rules; those say so.
pub const PALETTES: &[Palette] = &[
    MINT,
    NORD,
    PAPER,
    CATPPUCCIN_LATTE,
    CATPPUCCIN_MOCHA,
    DRACULA,
    GRUVBOX_DARK,
    GRUVBOX_LIGHT,
    ROSE_PINE,
    ROSE_PINE_DAWN,
    SOLARIZED_LIGHT,
    TOKYO_NIGHT,
];

/// Built-ins a picker lists only once earned: Phantom, after setup's
/// training (`gh0st.tour_done`). `gh0st.palette` finds them any time.
pub const EARNED: &[Palette] = &[PHANTOM];

/// A built-in palette by its name, any case, with or without the é.
pub fn palette(name: &str) -> Option<Palette> {
    let fold = |s: &str| s.replace('é', "e").to_ascii_lowercase();
    let name = fold(name);
    PALETTES
        .iter()
        .chain(EARNED)
        .find(|p| fold(p.name) == name)
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
        .map(|(k, v)| (k.trim(), v))
        .collect();
    // A base is a name of one or more words, up to any comment.
    let base = lines
        .iter()
        .find(|(k, _)| *k == "base")
        .map(|(_, v)| {
            let v = v.split('#').next().unwrap_or("").trim();
            palette(v).ok_or(format!("no built-in palette {v}"))
        })
        .transpose()?
        .unwrap_or(MINT);
    let mut p = base;
    p.name = intern(name);
    for (k, v) in lines {
        if k == "base" {
            continue;
        }
        // A colour is the first word: what follows it is a comment.
        let v = v.split_whitespace().next().unwrap_or("");
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
    radius: 4,
    radius_large: 6,
    radius_small: 2,
    badge_r: 5,
    lift: 2,
    chip_lift: 1,
    card_r: 7,
    row_icon: 20,
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
    // A 16 px pill two rows above the edge, 48 px wide over a 56 px key.
    chip_h: 16,
    chip_r: 4,
    chip_pad: 7,
    chip_foot: 2,
    chip_bar_w: 18,
    chip_inset: 4,
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
    plate_pad: 5,
    map_w: 40,
    map_h: 30,
    track_h: 3,
    track_lift: 6,
};

/// Which of the six tints a common icon takes. The order of Settings' first
/// page decides it: no two neighbours share one.
const ICON_TINTS: &[(&str, usize)] = &[
    ("wifi", 1),
    ("radio-tower", 4),
    ("network", 5),
    ("bluetooth", 1),
    ("sun", 2),
    ("monitor", 2),
    ("volume-2", 3),
    ("palette", 4),
    ("layout-grid", 0),
    ("settings", 5),
    ("clock", 1),
    ("globe", 5),
    ("keyboard", 2),
    ("user", 4),
    ("lock", 3),
    ("shield", 0),
    ("power", 3),
    ("download", 0),
    ("refresh-cw", 0),
    ("info", 1),
    ("cpu", 5),
    ("memory-stick", 5),
    ("hard-drive", 2),
    ("usb", 4),
    ("square-terminal", 0),
    // Files: a folder looks the same in every list, each kind of file its own.
    ("folder", 1),
    ("folder-open", 1),
    ("folder-up", 1),
    ("file", 2),
    ("file-text", 2),
    ("file-code", 2),
    ("file-image", 0),
    ("file-music", 4),
    ("file-video", 3),
    ("file-archive", 5),
    ("link", 1),
];

/// Light and shade mixed into a fill so it reads as lit from above: a
/// lighter top, a darker foot, a bright rim. On every palette.
pub const GLOSS_LIGHT: Color = crate::color::rgb(255, 255, 255);
pub const GLOSS_SHADE: Color = crate::color::rgb(0, 0, 0);

/// The top of a lit fill.
pub fn lit(c: Color) -> Color {
    mix(c, GLOSS_LIGHT, 40)
}

/// The foot of a lit fill.
pub fn shade(c: Color) -> Color {
    mix(c, GLOSS_SHADE, 50)
}

/// How far the foot under a raised thing is darkened.
pub const FOOT: u8 = 80;

/// The edge under a raised thing that makes it stand up: a chip, a badge.
pub fn foot(c: Color) -> Color {
    mix(c, GLOSS_SHADE, FOOT)
}

/// Ink on a badge or icon square of `tint`: dark or light, whichever reads
/// better across the lit fill, top to foot.
pub fn on_tint(tint: Color) -> Color {
    use crate::color::contrast;
    let dark = mix(tint, GLOSS_SHADE, 215);
    let light = mix(tint, GLOSS_LIGHT, 225);
    let worst = |ink: Color| contrast(ink, lit(tint)).min(contrast(ink, shade(tint)));
    if worst(dark) >= worst(light) {
        dark
    } else {
        light
    }
}

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

/// The editor: how strongly a selection, a find match and the cursor's
/// line tint the page, out of 255. A selection and a match take the accent
/// and warning colours, the cursor's line `surface`.
pub const TEXT_SELECT: u8 = 90;
pub const TEXT_MATCH: u8 = 120;
pub const TEXT_LINE: u8 = 150;
/// Space between the editor's line numbers and its text, in px.
pub const TEXT_GUTTER_GAP: i32 = 4;
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
    small: &font::TERMINUS_16,
    small_bold: &font::TERMINUS_16_BOLD,
    body: &font::SANS_17,
    body_bold: &font::SANS_17_BOLD,
    title: &font::SANS_19_BOLD,
    big: &font::SANS_26_BOLD,
    mono: &font::MONO_12,
    mono_bold: &font::MONO_12_BOLD,
    chip: &font::TERMINUS_12_BOLD,
    clock: &font::SANS_50_BOLD,
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
        let warm = parse_palette("base = gruvbox dark # warm\n", "Warm").unwrap();
        assert_eq!(warm.bg, GRUVBOX_DARK.bg);
        let dawn = parse_palette("base = Rose Pine Dawn\n", "Dawn").unwrap();
        assert_eq!(dawn.bg, ROSE_PINE_DAWN.bg);
        assert!(find("../etc/passwd").is_none());
    }

    /// The rules every shipped palette keeps, one line per break.
    pub(super) fn lint(p: &Palette) -> Vec<String> {
        let mut out = Vec::new();
        let mut need = |what: String, got: f32, min: f32| {
            if got < min {
                out.push(format!("{} {what}: {got:.2} < {min}", p.name));
            }
        };
        need("fg".into(), contrast(p.fg, p.bg), 7.0);
        need("fg on surface".into(), contrast(p.fg, p.surface), 4.5);
        need("muted".into(), contrast(p.muted, p.bg), 3.0);
        need("disabled".into(), contrast(p.disabled(), p.bg), 2.8);
        for tone in [Tone::Info, Tone::Ok, Tone::Warning, Tone::Critical] {
            need(
                format!("{tone:?} on surface"),
                contrast(p.tone(tone), p.surface),
                3.0,
            );
        }
        need("accent text".into(), contrast(p.accent_fg, p.accent), 4.5);
        for (i, tint) in p.tints.iter().chain([&p.accent]).enumerate() {
            // Icons and large letters: 3:1, as for graphics.
            let ink = on_tint(*tint);
            let got = contrast(ink, lit(*tint)).min(contrast(ink, shade(*tint)));
            need(format!("ink on tint {i}"), got, 3.0);
        }
        // Black is a background colour in the terminal; the rest is text.
        for (i, &c) in p.ansi.iter().enumerate().filter(|&(i, _)| i != 0) {
            need(format!("ansi {i}"), contrast(c, p.bg), 3.0);
        }
        for role in Syntax::ALL {
            need(
                format!("{role:?} code"),
                contrast(p.syntax(role), p.bg),
                3.0,
            );
        }
        out
    }

    /// The schemes we ship as files meet the built-ins' rules, not only
    /// the looser check every file gets.
    #[test]
    fn shipped_schemes_keep_text_readable() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/palettes");
        let mut bad = Vec::new();
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str()?.strip_suffix(".palette"))
            else {
                continue;
            };
            n += 1;
            match parse_palette(&std::fs::read_to_string(&path).unwrap(), name) {
                Ok(p) => bad.extend(lint(&p)),
                Err(e) => bad.push(format!("{name}: {e}")),
            }
        }
        assert!(n >= 2, "{n} schemes");
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn every_palette_keeps_text_readable() {
        let bad: Vec<String> = PALETTES.iter().chain(EARNED).flat_map(lint).collect();
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}
