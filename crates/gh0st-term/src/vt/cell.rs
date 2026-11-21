//! One screen cell in 8 bytes, its style inline.
//!
//! The first word holds the code point (21 bits), three layout flags, six
//! attributes and two "palette index" bits. Then fg and bg, each either a
//! palette index or RGB565. The 16 ANSI colours and the defaults stay
//! indices, so a palette change recolours them; the 256-colour cube and
//! truecolor are fixed colours and resolve to RGB565 at parse.

const CP: u32 = 0x1f_ffff;
const WIDE: u32 = 1 << 21;
const SPACER: u32 = 1 << 22;
const CLUSTER: u32 = 1 << 23;
const FG_INDEX: u32 = 1 << 30;
const BG_INDEX: u32 = 1 << 31;
/// The bits a style carries: the attributes and both index flags.
const STYLE: u32 = 0xff << 24;

/// Attribute bits, as in [`Pen::attrs`] and [`Cell::attrs`].
pub mod attr {
    pub const BOLD: u32 = 1 << 24;
    pub const DIM: u32 = 1 << 25;
    pub const ITALIC: u32 = 1 << 26;
    pub const UNDERLINE: u32 = 1 << 27;
    pub const INVERSE: u32 = 1 << 28;
    pub const STRIKE: u32 = 1 << 29;
    pub const ALL: u32 = 0x3f << 24;
}

/// The palette index that means the theme's default fg or bg.
pub const DEFAULT: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// 0 to 15 are the ANSI colours, [`DEFAULT`] the theme's fg or bg.
    Index(u8),
    Rgb565(u16),
}

impl Color {
    pub const DEFAULT: Color = Color::Index(DEFAULT);

    fn pack(self) -> (bool, u16) {
        match self {
            Color::Index(i) => (true, i as u16),
            Color::Rgb565(c) => (false, c),
        }
    }

    fn unpack(index: bool, v: u16) -> Color {
        if index {
            Color::Index(v as u8)
        } else {
            Color::Rgb565(v)
        }
    }

    /// An xterm 256-colour index: 0..15 stay palette entries, the cube and
    /// the grey ramp are fixed.
    pub fn xterm(n: u8) -> Color {
        match n {
            0..=15 => Color::Index(n),
            16..=231 => {
                let n = n - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Color::Rgb565(rgb565(level(n / 36), level(n / 6 % 6), level(n % 6)))
            }
            _ => {
                let g = 8 + 10 * (n - 232);
                Color::Rgb565(rgb565(g, g, g))
            }
        }
    }
}

pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// The style new cells get: attributes and colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pen {
    flags: u32,
    fg: u16,
    bg: u16,
}

impl Default for Pen {
    fn default() -> Pen {
        Pen::DEFAULT
    }
}

impl Pen {
    pub const DEFAULT: Pen = Pen {
        flags: FG_INDEX | BG_INDEX,
        fg: DEFAULT as u16,
        bg: DEFAULT as u16,
    };

    pub fn attrs(&self) -> u32 {
        self.flags & attr::ALL
    }

    pub fn set(&mut self, a: u32, on: bool) {
        if on {
            self.flags |= a & attr::ALL;
        } else {
            self.flags &= !(a & attr::ALL);
        }
    }

    pub fn fg(&self) -> Color {
        Color::unpack(self.flags & FG_INDEX != 0, self.fg)
    }

    pub fn bg(&self) -> Color {
        Color::unpack(self.flags & BG_INDEX != 0, self.bg)
    }

    pub fn set_fg(&mut self, c: Color) {
        let (i, v) = c.pack();
        self.flags = (self.flags & !FG_INDEX) | if i { FG_INDEX } else { 0 };
        self.fg = v;
    }

    pub fn set_bg(&mut self, c: Color) {
        let (i, v) = c.pack();
        self.flags = (self.flags & !BG_INDEX) | if i { BG_INDEX } else { 0 };
        self.bg = v;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Cell {
    w: u32,
    fg: u16,
    bg: u16,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell::BLANK
    }
}

impl Cell {
    pub const BLANK: Cell = Cell {
        w: ' ' as u32 | FG_INDEX | BG_INDEX,
        fg: DEFAULT as u16,
        bg: DEFAULT as u16,
    };

    pub fn new(ch: char, pen: &Pen) -> Cell {
        Cell {
            w: ch as u32 | (pen.flags & STYLE),
            fg: pen.fg,
            bg: pen.bg,
        }
    }

    /// An erased cell. Erase keeps the pen's background (xterm's bce) and
    /// nothing else.
    pub fn blank(pen: &Pen) -> Cell {
        Cell {
            w: ' ' as u32 | FG_INDEX | (pen.flags & BG_INDEX),
            fg: DEFAULT as u16,
            bg: pen.bg,
        }
    }

    pub fn wide(ch: char, pen: &Pen) -> Cell {
        let mut c = Cell::new(ch, pen);
        c.w |= WIDE;
        c
    }

    /// The right half of a wide char. It draws nothing but its background.
    pub fn spacer(pen: &Pen) -> Cell {
        let mut c = Cell::new(' ', pen);
        c.w |= SPACER;
        c
    }

    /// A cell whose text is cluster `id` in the terminal's cluster table.
    pub fn with_cluster(self, id: u32) -> Cell {
        Cell {
            w: (self.w & !CP) | (id & CP) | CLUSTER,
            ..self
        }
    }

    /// The code point, or the cluster id when [`is_cluster`](Cell::is_cluster).
    pub fn code(&self) -> u32 {
        self.w & CP
    }

    pub fn ch(&self) -> char {
        char::from_u32(self.code()).unwrap_or(' ')
    }

    pub fn is_wide(&self) -> bool {
        self.w & WIDE != 0
    }

    pub fn is_spacer(&self) -> bool {
        self.w & SPACER != 0
    }

    pub fn is_cluster(&self) -> bool {
        self.w & CLUSTER != 0
    }

    pub fn attrs(&self) -> u32 {
        self.w & attr::ALL
    }

    pub fn fg(&self) -> Color {
        Color::unpack(self.w & FG_INDEX != 0, self.fg)
    }

    pub fn bg(&self) -> Color {
        Color::unpack(self.w & BG_INDEX != 0, self.bg)
    }

    /// The style as a pen, for erasing next to this cell.
    pub fn pen(&self) -> Pen {
        Pen {
            flags: self.w & STYLE,
            fg: self.fg,
            bg: self.bg,
        }
    }

    /// The style packed for history runs: style bits, fg, bg.
    pub fn style_key(&self) -> (u8, u16, u16) {
        ((self.w >> 24) as u8, self.fg, self.bg)
    }

    pub fn from_style_key(ch: char, key: (u8, u16, u16)) -> Cell {
        Cell {
            w: ch as u32 | ((key.0 as u32) << 24),
            fg: key.1,
            bg: key.2,
        }
    }

    /// The cell as one word: the code point in the low 21 bits, then the
    /// flags, attributes, fg and bg.
    #[inline]
    pub fn bits(&self) -> u64 {
        self.w as u64 | (self.fg as u64) << 32 | (self.bg as u64) << 48
    }

    /// The bits that say what a cell looks like, past its code point: the
    /// layout flags, the style and both colours.
    pub const LOOK: u64 = !(CP as u64);

    /// One printable ASCII char in one cell: history stores it as one byte.
    pub fn is_ascii(&self) -> bool {
        (0x20..0x7f).contains(&self.code()) && self.w & (WIDE | SPACER | CLUSTER) == 0
    }

    /// A space that shows nothing: no background, no line through it.
    pub fn is_blank(&self) -> bool {
        self.code() == ' ' as u32
            && !self.is_cluster()
            && !self.is_spacer()
            && self.bg() == Color::DEFAULT
            && self.attrs() & (attr::INVERSE | attr::UNDERLINE | attr::STRIKE) == 0
    }
}

/// Text of cells that hold a base char and combining marks. Interned and
/// bounded: past the cap, new marks are dropped.
#[derive(Default)]
pub struct Clusters {
    list: Vec<Box<str>>,
}

const CLUSTERS_MAX: usize = 1024;
const CLUSTER_BYTES_MAX: usize = 32;

impl Clusters {
    pub fn get(&self, id: u32) -> &str {
        self.list.get(id as usize).map_or(" ", |s| s)
    }

    /// The id of `s`, interning it if there is room.
    pub fn intern(&mut self, s: &str) -> Option<u32> {
        if let Some(i) = self.list.iter().position(|x| &**x == s) {
            return Some(i as u32);
        }
        if self.list.len() >= CLUSTERS_MAX || s.len() > CLUSTER_BYTES_MAX {
            return None;
        }
        self.list.push(s.into());
        Some(self.list.len() as u32 - 1)
    }

    /// Appends a cell's text to `out`.
    pub fn push_text(&self, cell: &Cell, out: &mut String) {
        if cell.is_cluster() {
            out.push_str(self.get(cell.code()));
        } else {
            out.push(cell.ch());
        }
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<Cell>(), 8);
    }

    #[test]
    fn style_survives_the_cell() {
        let mut pen = Pen::DEFAULT;
        pen.set(attr::BOLD | attr::UNDERLINE, true);
        pen.set_fg(Color::Index(3));
        pen.set_bg(Color::Rgb565(0x1234));
        let c = Cell::wide('漢', &pen);
        assert_eq!(c.ch(), '漢');
        assert!(c.is_wide());
        assert_eq!(c.attrs(), attr::BOLD | attr::UNDERLINE);
        assert_eq!(c.fg(), Color::Index(3));
        assert_eq!(c.bg(), Color::Rgb565(0x1234));
        assert_eq!(Cell::from_style_key('x', c.style_key()).pen(), c.pen());
    }

    #[test]
    fn erase_keeps_only_the_background() {
        let mut pen = Pen::DEFAULT;
        pen.set(attr::INVERSE, true);
        pen.set_fg(Color::Index(1));
        pen.set_bg(Color::Index(4));
        let c = Cell::blank(&pen);
        assert_eq!(c.attrs(), 0);
        assert_eq!(c.fg(), Color::DEFAULT);
        assert_eq!(c.bg(), Color::Index(4));
        assert!(!c.is_blank());
        assert!(Cell::blank(&Pen::DEFAULT).is_blank());
    }

    #[test]
    fn xterm_cube_and_greys() {
        assert_eq!(Color::xterm(1), Color::Index(1));
        assert_eq!(Color::xterm(16), Color::Rgb565(0));
        assert_eq!(Color::xterm(231), Color::Rgb565(0xffff));
        assert_eq!(Color::xterm(232), Color::Rgb565(rgb565(8, 8, 8)));
    }
}
