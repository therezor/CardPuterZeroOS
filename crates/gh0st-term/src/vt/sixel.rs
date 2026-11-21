//! Sixel images: the decoder for the bytes between `DCS q` and ST, and the
//! image it leaves on the screen.
//!
//! A sixel char paints six pixels of one column, in the colour selected
//! last. `$` returns to the left edge, `-` moves down six pixels, `!n`
//! repeats the next char n times, `#` selects or defines a colour, `"` gives
//! the size. Colours are RGB565 from the start, so an image costs two bytes
//! a pixel and a bit to say it was painted.

use super::cell::rgb565;

/// The VT340's first 16 colour registers, in percent. The rest start black.
const VT340: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (20, 20, 80),
    (80, 13, 13),
    (20, 80, 20),
    (80, 20, 80),
    (20, 80, 80),
    (80, 80, 20),
    (53, 53, 53),
    (26, 26, 26),
    (33, 33, 60),
    (60, 26, 26),
    (33, 60, 33),
    (60, 33, 60),
    (33, 60, 60),
    (60, 60, 33),
    (80, 80, 80),
];

fn percent(v: u32) -> u8 {
    (v.min(100) * 255 / 100) as u8
}

/// DEC's hue circle starts at blue: 0 is blue, 120 red, 240 green.
fn hls(h: u32, l: u32, s: u32) -> u16 {
    let h = ((h % 360) as f32 + 240.0) % 360.0;
    let (l, s) = (l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to8 = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    rgb565(to8(r), to8(g), to8(b))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd {
    None,
    Repeat,
    Color,
    Raster,
}

/// Pixels, row-major, and a bit per pixel for the ones painted.
pub struct Pixels {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u16>,
    painted: Vec<u64>,
}

impl Pixels {
    fn new(w: usize, h: usize) -> Pixels {
        Pixels {
            w,
            h,
            px: vec![0; w * h],
            painted: vec![0; (w * h).div_ceil(64)],
        }
    }

    /// The colour at `x`, `y`, or None where nothing was painted.
    pub fn get(&self, x: usize, y: usize) -> Option<u16> {
        let i = y * self.w + x;
        (self.painted[i / 64] & (1 << (i % 64)) != 0).then(|| self.px[i])
    }

    /// The top-left `w` x `h`, as a new buffer.
    fn crop(&self, w: usize, h: usize) -> Pixels {
        let mut out = Pixels::new(w, h);
        for y in 0..h.min(self.h) {
            for x in 0..w.min(self.w) {
                if let Some(c) = self.get(x, y) {
                    let i = y * w + x;
                    out.px[i] = c;
                    out.painted[i / 64] |= 1 << (i % 64);
                }
            }
        }
        out
    }
}

/// Decodes one sixel image. Nothing past `max_w` x `max_h` is kept, so a
/// program can't make the terminal hold more than a screen of pixels.
pub struct Decoder {
    max_w: usize,
    max_h: usize,
    /// Unpainted pixels show what is under them (P2 = 1), or the
    /// background.
    pub transparent: bool,
    buf: Pixels,
    /// The size `"` gave, if any.
    raster: Option<(usize, usize)>,
    used_w: usize,
    used_h: usize,
    x: usize,
    y: usize,
    palette: Box<[u16; 256]>,
    color: u16,
    repeat: usize,
    cmd: Cmd,
    params: [u32; 5],
    nparams: usize,
}

impl Decoder {
    pub fn new(max_w: usize, max_h: usize, transparent: bool) -> Decoder {
        let mut palette = Box::new([0u16; 256]);
        for (p, &(r, g, b)) in palette.iter_mut().zip(VT340.iter()) {
            *p = rgb565(percent(r as u32), percent(g as u32), percent(b as u32));
        }
        Decoder {
            max_w,
            max_h,
            transparent,
            buf: Pixels::new(0, 0),
            raster: None,
            used_w: 0,
            used_h: 0,
            x: 0,
            y: 0,
            color: palette[0],
            palette,
            repeat: 1,
            cmd: Cmd::None,
            params: [0; 5],
            nparams: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.cmd != Cmd::None {
                match b {
                    b'0'..=b'9' => {
                        if let Some(p) = self.params.get_mut(self.nparams - 1) {
                            *p = p.saturating_mul(10).saturating_add((b - b'0') as u32);
                        }
                        continue;
                    }
                    b';' => {
                        self.nparams += 1;
                        if let Some(p) = self.params.get_mut(self.nparams - 1) {
                            *p = 0;
                        }
                        continue;
                    }
                    _ => self.end_cmd(),
                }
            }
            match b {
                b'?'..=b'~' => self.sixel(b - b'?'),
                b'$' => self.x = 0,
                b'-' => {
                    self.x = 0;
                    self.y += 6;
                }
                b'!' => self.start(Cmd::Repeat),
                b'#' => self.start(Cmd::Color),
                b'"' => self.start(Cmd::Raster),
                _ => {}
            }
        }
    }

    fn start(&mut self, cmd: Cmd) {
        self.cmd = cmd;
        self.params = [0; 5];
        self.nparams = 1;
    }

    fn end_cmd(&mut self) {
        let p = self.params;
        match self.cmd {
            Cmd::Repeat => self.repeat = (p[0] as usize).clamp(1, self.max_w.max(1)),
            Cmd::Color => {
                let reg = (p[0] % 256) as usize;
                if self.nparams >= 5 {
                    self.palette[reg] = match p[1] {
                        1 => hls(p[2], p[3], p[4]),
                        _ => rgb565(percent(p[2]), percent(p[3]), percent(p[4])),
                    };
                }
                self.color = self.palette[reg];
            }
            Cmd::Raster if self.nparams >= 4 => {
                let (w, h) = (p[2] as usize, p[3] as usize);
                self.raster = Some((w.min(self.max_w), h.min(self.max_h)));
                if self.used_w == 0 {
                    self.grow(w, h);
                }
            }
            _ => {}
        }
        self.cmd = Cmd::None;
    }

    /// Makes room for `w` x `h`, within the limits.
    fn grow(&mut self, w: usize, h: usize) {
        let (w, h) = (w.min(self.max_w), h.min(self.max_h));
        if w <= self.buf.w && h <= self.buf.h {
            return;
        }
        // Doubling keeps an image with no size from copying on every band.
        let w = w.max(self.buf.w).max((self.buf.w * 2).min(self.max_w));
        let h = h.max(self.buf.h).max((self.buf.h * 2).min(self.max_h));
        self.buf = self.buf.crop(w, h);
    }

    fn sixel(&mut self, bits: u8) {
        let n = std::mem::replace(&mut self.repeat, 1);
        let x0 = self.x;
        self.x = self.x.saturating_add(n);
        if bits == 0 || x0 >= self.max_w || self.y >= self.max_h {
            return;
        }
        let x1 = self.x.min(self.max_w);
        let rows = (self.max_h - self.y).min(6);
        self.grow(x1, self.y + rows);
        let (w, color) = (self.buf.w, self.color);
        for i in 0..rows {
            if bits & (1 << i) == 0 {
                continue;
            }
            let y = self.y + i;
            self.used_h = self.used_h.max(y + 1);
            let row = y * w;
            self.buf.px[row + x0..row + x1].fill(color);
            for j in row + x0..row + x1 {
                self.buf.painted[j / 64] |= 1 << (j % 64);
            }
        }
        self.used_w = self.used_w.max(x1);
    }

    /// The image, cut to what was painted or the size it gave, whichever
    /// is larger. None if it is empty.
    pub fn finish(mut self) -> Option<Pixels> {
        if self.cmd != Cmd::None {
            self.end_cmd();
        }
        let (rw, rh) = self.raster.unwrap_or((0, 0));
        let (w, h) = (self.used_w.max(rw), self.used_h.max(rh));
        if w == 0 || h == 0 {
            return None;
        }
        Some(if (w, h) == (self.buf.w, self.buf.h) {
            self.buf
        } else {
            self.buf.crop(w, h)
        })
    }
}

/// An image on the screen, pinned to the cells it covers.
pub struct Image {
    /// The top-left cell. The row goes negative as the screen scrolls up.
    pub row: isize,
    pub col: usize,
    /// Cells covered.
    pub cols: usize,
    pub rows: usize,
    /// The cell size the image was laid out in: it is scaled from this to
    /// the text size on screen.
    pub cell: (usize, usize),
    pub pixels: Pixels,
    pub transparent: bool,
    /// Cells text has covered since: they show the text.
    holes: Vec<bool>,
    left: usize,
}

impl Image {
    pub fn new(
        pixels: Pixels,
        transparent: bool,
        row: isize,
        col: usize,
        cell: (usize, usize),
    ) -> Image {
        let cols = pixels.w.div_ceil(cell.0);
        let rows = pixels.h.div_ceil(cell.1);
        Image {
            row,
            col,
            cols,
            rows,
            cell,
            pixels,
            transparent,
            holes: vec![false; cols * rows],
            left: cols * rows,
        }
    }

    pub fn bytes(&self) -> usize {
        self.pixels.px.len() * 2
    }

    /// Whether cell `k` of row `j` of the image still shows it.
    pub fn shows(&self, j: usize, k: usize) -> bool {
        !self.holes[j * self.cols + k]
    }

    /// Text covered cells `x0..x1` of screen row `y`. False once nothing
    /// of the image shows.
    pub fn punch(&mut self, y: usize, x0: usize, x1: usize) -> bool {
        let j = y as isize - self.row;
        if j >= 0 && (j as usize) < self.rows {
            let (a, b) = (x0.max(self.col), x1.min(self.col + self.cols));
            for k in a.saturating_sub(self.col)..b.saturating_sub(self.col) {
                let h = &mut self.holes[j as usize * self.cols + k];
                if !*h {
                    *h = true;
                    self.left -= 1;
                }
            }
        }
        self.left > 0
    }

    /// The image's screen rows, as a range.
    pub fn span(&self) -> (isize, isize) {
        (self.row, self.row + self.rows as isize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(s: &str) -> Pixels {
        let mut d = Decoder::new(100, 100, true);
        d.feed(s.as_bytes());
        d.finish().unwrap()
    }

    const RED: u16 = 0xf800;

    #[test]
    fn a_band_paints_six_rows_in_the_colour_chosen() {
        let p = decode("#1;2;100;0;0#1!3~");
        assert_eq!((p.w, p.h), (3, 6));
        assert!((0..6).all(|y| (0..3).all(|x| p.get(x, y) == Some(RED))));
    }

    #[test]
    fn dollar_returns_and_dash_moves_down() {
        // Bit 0 at x 0, then bit 1 over it, then a band down.
        let p = decode("#1;2;100;0;0@$A-@");
        assert_eq!((p.w, p.h), (1, 7));
        assert_eq!(p.get(0, 0), Some(RED));
        assert_eq!(p.get(0, 1), Some(RED));
        assert_eq!(p.get(0, 2), None);
        assert_eq!(p.get(0, 6), Some(RED));
    }

    #[test]
    fn the_size_given_holds_even_where_nothing_is_painted() {
        let p = decode("\"1;1;8;10#1;2;100;0;0@");
        assert_eq!((p.w, p.h), (8, 10));
        assert_eq!(p.get(7, 9), None);
    }

    #[test]
    fn nothing_past_the_limit_is_kept() {
        let mut d = Decoder::new(4, 6, true);
        d.feed(b"\"1;1;5000;5000#1;2;100;0;0!9000~-~");
        let p = d.finish().unwrap();
        assert_eq!((p.w, p.h), (4, 6));
    }

    #[test]
    fn hls_hue_starts_at_blue() {
        assert_eq!(hls(0, 50, 100), 0x001f);
        assert_eq!(hls(120, 50, 100), RED);
    }

    #[test]
    fn punching_every_cell_hides_the_image() {
        let mut img = Image::new(decode("!12~-!12~"), true, 0, 0, (6, 6));
        assert_eq!((img.cols, img.rows), (2, 2));
        assert!(img.punch(0, 0, 2));
        assert!(!img.shows(0, 1) && img.shows(1, 0));
        assert!(!img.punch(1, 0, 5));
    }
}
