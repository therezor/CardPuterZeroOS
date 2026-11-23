//! Bakes fonts and icons into `.rodata` so nothing is parsed or rasterized at
//! run time. A glyph or icon costs a page fault the first time it is drawn,
//! and nothing before that.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, VariableFont, point};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Light text on a dark panel reads thin with linear coverage. This lifts the
/// mid tones a little, which stands in for the hinting ab_glyph does not do.
const COVERAGE_GAMMA: f32 = 1.25;

/// Mono text on the panel blurs where a stem spans two pixels at half
/// coverage. Coverage below this goes to nothing and above one minus it to
/// full, so stems keep a hard core and only a thin fringe.
const MONO_SHARPEN: f32 = 0.2;

/// Inter below this em (Caption and Body) blurs the same way, its stems a
/// pixel wide. Each glyph moves at most a quarter pixel (a half spaces
/// letters unevenly) and gets no gamma lift, which widens the fringe. The
/// sharpen is hard but keeps a fringe: with none, Inter's curves go jagged.
const SMALL_SANS_EM: f32 = 18.0;
const SMALL_SANS_SHARPEN: f32 = 0.22;
/// Small sans text still reads soft, its stem edges at mid grey. An
/// S-curve on coverage pushes fringes toward the background and cores toward
/// the ink: 0 is linear, 1 a full smoothstep.
const SMALL_SANS_CONTRAST: f32 = 1.0;

const TEXT: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0xa0, 0x17f),
    (0x400, 0x45f),
    (0x490, 0x491),
    (0x2010, 0x2027),
    (0x2030, 0x203a),
    (0x20ac, 0x20ac),
    (0x2116, 0x2116),
    (0x2190, 0x2195),
    (0x21b5, 0x21b5),
    (0x2212, 0x2212),
    (0x2318, 0x2318),
    (0x23ce, 0x23ce),
    (0x25cf, 0x25cf),
];

/// The lock's clock: "14:32", "2:32" on a 12-hour clock, or "--:--" before
/// the first read.
const CLOCK: &[(u32, u32)] = &[(0x20, 0x20), (0x2d, 0x2d), (0x30, 0x3a)];

const MONO: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0xa0, 0x17f),
    (0x400, 0x45f),
    (0x490, 0x491),
    (0x2010, 0x2027),
    (0x2190, 0x2195),
    (0x2500, 0x25ff),
];

/// The terminal's faces: the mono set plus Greek and a few signs. The
/// terminal draws box drawing and blocks itself, so they join at any size.
const TERM: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0xa0, 0x17f),
    (0x370, 0x3ff),
    (0x400, 0x45f),
    (0x490, 0x491),
    (0x2010, 0x2027),
    (0x2030, 0x203a),
    (0x20ac, 0x20ac),
    (0x2116, 0x2116),
    (0x2190, 0x2195),
    (0x21b5, 0x21b5),
    (0x2212, 0x2212),
    (0x25a0, 0x25ff),
];

/// The terminal's face, Terminus, OFL. The BDF files are kept as published
/// (the licence reserves the name for unmodified fonts); only the glyphs in
/// `TERM` are baked.
const BDF_FACES: &[(&str, &str)] = &[
    ("TERMINUS_12", "terminus/ter-u12n.bdf"),
    ("TERMINUS_12_BOLD", "terminus/ter-u12b.bdf"),
    ("TERMINUS_14", "terminus/ter-u14n.bdf"),
    ("TERMINUS_14_BOLD", "terminus/ter-u14b.bdf"),
    ("TERMINUS_16", "terminus/ter-u16n.bdf"),
    ("TERMINUS_16_BOLD", "terminus/ter-u16b.bdf"),
    ("TERMINUS_18", "terminus/ter-u18n.bdf"),
    ("TERMINUS_18_BOLD", "terminus/ter-u18b.bdf"),
    ("TERMINUS_20", "terminus/ter-u20n.bdf"),
    ("TERMINUS_20_BOLD", "terminus/ter-u20b.bdf"),
];

struct Face {
    name: &'static str,
    file: &'static str,
    em: f32,
    wght: f32,
    ranges: &'static [(u32, u32)],
    /// Scale the em so the x-height lands on a whole pixel. Crisper tops on
    /// lower-case letters. Off for mono, where the cell width must stay exact.
    snap_x_height: bool,
}

impl Face {
    fn small_sans(&self) -> bool {
        self.snap_x_height && self.em < SMALL_SANS_EM
    }

    /// Mono and small Inter are sharpened; large Inter keeps its smooth
    /// edges.
    fn sharpen(&self) -> f32 {
        match (self.snap_x_height, self.small_sans()) {
            (false, _) => MONO_SHARPEN,
            (true, true) => SMALL_SANS_SHARPEN,
            (true, false) => 0.0,
        }
    }

    /// How far, in eighths of a pixel, a glyph may move to fit the grid.
    fn reach(&self) -> i32 {
        match (self.snap_x_height, self.small_sans()) {
            (false, _) => 4,
            (true, true) => 2,
            (true, false) => 0,
        }
    }

    fn gamma(&self) -> f32 {
        if self.small_sans() {
            1.0
        } else {
            COVERAGE_GAMMA
        }
    }

    /// Coverage after the sharpen and gamma: the small sans S-curve.
    fn curve(&self, c: f32) -> f32 {
        if !self.small_sans() {
            return c;
        }
        let s = c * c * (3.0 - 2.0 * c);
        c + SMALL_SANS_CONTRAST * (s - c)
    }
}

/// Inter steps in whole-pixel x-heights: 15 px gives an 8 px x-height, 17
/// gives 9, 19 gives 10. Sizes between snap to the same face. 13 px (7) is
/// too small to read on the panel. The small faces are a step heavier than
/// Inter's named weights, 450 and 650: thinner stems read faint on the dark
/// panel.
const FACES: &[Face] = &[
    face("SANS_15", "Inter.ttf", 15.0, 450.0, TEXT, true),
    face("SANS_15_BOLD", "Inter.ttf", 15.0, 650.0, TEXT, true),
    face("SANS_17", "Inter.ttf", 17.0, 450.0, TEXT, true),
    face("SANS_17_BOLD", "Inter.ttf", 17.0, 650.0, TEXT, true),
    face("SANS_19_BOLD", "Inter.ttf", 19.0, 600.0, TEXT, true),
    face("SANS_26_BOLD", "Inter.ttf", 26.0, 600.0, TEXT, true),
    face("SANS_50_BOLD", "Inter.ttf", 50.0, 600.0, CLOCK, true),
    // A 7 px cell exactly: JetBrains Mono's advance is 0.6 em, and at 12 px
    // a 7.2 px cell would space letters unevenly.
    face(
        "MONO_12",
        "JetBrainsMono.ttf",
        35.0 / 3.0,
        400.0,
        MONO,
        false,
    ),
    face(
        "MONO_12_BOLD",
        "JetBrainsMono.ttf",
        35.0 / 3.0,
        700.0,
        MONO,
        false,
    ),
];

const fn face(
    name: &'static str,
    file: &'static str,
    em: f32,
    wght: f32,
    ranges: &'static [(u32, u32)],
    snap_x_height: bool,
) -> Face {
    Face {
        name,
        file,
        em,
        wght,
        ranges,
        snap_x_height,
    }
}

/// Icon sizes in pixels, with the stroke width each one is drawn at. Lucide is
/// drawn on a 24 grid with a 2 unit stroke; at 14 px that is a blurry 1.2 px.
const ICON_SIZES: &[(u32, f32)] = &[(12, 1.2), (14, 1.2), (16, 1.2), (26, 2.0), (36, 2.6)];

/// Icons this size and under sit beside Terminus, so they are drawn near
/// it: a thin stroke moved onto the pixel grid, then sharpened.
const PIXEL_ICON_MAX: u32 = 16;

/// Coverage under this goes to nothing and over one minus it to full,
/// then an S-curve: straight strokes keep a hard core, curves a thin
/// fringe.
const PIXEL_ICON_SHARPEN: f32 = 0.15;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", root.join("fonts").display());
    println!("cargo:rerun-if-changed={}", root.join("icons").display());
    println!("cargo:rerun-if-changed=build.rs");

    bake_fonts(&root.join("fonts"), &out);
    bake_icons(&root.join("icons"), &out);
}

/// How much of a glyph sits on half-covered pixels: what blurs it.
fn blur(font: &FontRef, scale: PxScale, id: ab_glyph::GlyphId, at: ab_glyph::Point) -> f32 {
    let mut sum = 0.0;
    if let Some(o) = font.outline_glyph(id.with_scale_and_position(scale, at)) {
        o.draw(|_, _, c| {
            let c = c.clamp(0.0, 1.0);
            sum += c * (1.0 - c);
        });
    }
    sum
}

/// Where a mono face sits within a pixel so the most bars land on whole
/// pixels: the offset, in eighths, with the least blur over ASCII. Hinting
/// does this in a font renderer; ab_glyph has none.
fn grid_fit(font: &FontRef, scale: PxScale) -> ab_glyph::Point {
    let mut best = (f32::MAX, point(0.0, 0.0));
    for i in 0..8 {
        for j in 0..8 {
            let at = point(i as f32 / 8.0, j as f32 / 8.0);
            let b: f32 = ('!'..='~')
                .map(|ch| blur(font, scale, font.glyph_id(ch), at))
                .sum();
            if b < best.0 {
                best = (b, at);
            }
        }
    }
    best.1
}

/// Each glyph then moves sideways, up to `reach` eighths of a pixel, to put
/// its own stems on pixels: m's three stems can't all fit the face's offset.
/// The cell stays put; only the ink inside it moves.
fn glyph_fit(
    font: &FontRef,
    scale: PxScale,
    id: ab_glyph::GlyphId,
    face: ab_glyph::Point,
    reach: i32,
) -> ab_glyph::Point {
    let mut best = (f32::MAX, face);
    for i in -reach..reach {
        let at = point(face.x + i as f32 / 8.0, face.y);
        let b = blur(font, scale, id, at);
        // Ties keep the face's offset, so glyphs don't drift for nothing.
        if b < best.0 - 1e-3 || (b - best.0).abs() <= 1e-3 && i == 0 {
            best = (b, at);
        }
    }
    best.1
}

fn bake_fonts(dir: &Path, out: &Path) {
    let mut src = String::new();
    for f in FACES {
        let data = std::fs::read(dir.join(f.file)).expect("font file");
        let mut font = FontRef::try_from_slice(&data).expect("valid font");
        font.set_variation(b"wght", f.wght);
        let upem = font.units_per_em().unwrap_or(1000.0);
        let mut em = f.em;
        if f.snap_x_height {
            if let Some(xh) = x_height(&font) {
                let target = (xh / upem * em).round().max(1.0);
                em = target * upem / xh;
            }
        }
        font.set_variation(b"opsz", em.clamp(14.0, 32.0));
        // ab_glyph's scale is the ascent-to-descent height, not the em.
        let scale = PxScale::from(em * font.height_unscaled() / upem);
        let scaled = font.as_scaled(scale);

        let at = if f.snap_x_height {
            point(0.0, 0.0)
        } else {
            grid_fit(&font, scale)
        };
        let mut bits: Vec<u8> = Vec::new();
        let mut nibbles: Vec<u8> = Vec::new();
        let mut glyphs = String::new();
        for &(lo, hi) in f.ranges {
            for cp in lo..=hi {
                let Some(ch) = char::from_u32(cp) else {
                    continue;
                };
                let id = font.glyph_id(ch);
                if id.0 == 0 {
                    continue;
                }
                let adv = (scaled.h_advance(id) * 64.0).round() as u32;
                let at = match f.reach() {
                    0 => at,
                    r => glyph_fit(&font, scale, id, at, r),
                };
                let glyph = id.with_scale_and_position(scale, at);
                let (w, h, left, top, off) = match font.outline_glyph(glyph) {
                    Some(outline) => {
                        let b = outline.px_bounds();
                        let (w, h) = (b.width() as usize, b.height() as usize);
                        let mut cov = vec![0f32; w * h];
                        outline.draw(|x, y, c| cov[y as usize * w + x as usize] = c);
                        let off = nibbles.len();
                        let (t, gamma) = (f.sharpen(), f.gamma());
                        for c in cov {
                            let c = ((c - t) / (1.0 - 2.0 * t)).clamp(0.0, 1.0);
                            let c = f.curve(c.powf(1.0 / gamma));
                            nibbles.push((c * 15.0).round() as u8);
                        }
                        (w, h, b.min.x as i32, b.min.y as i32, off)
                    }
                    None => (0, 0, 0, 0, nibbles.len()),
                };
                assert!(w < 256 && h < 256 && adv < 65536);
                writeln!(
                    glyphs,
                    "    g({cp}, {w}, {h}, {left}, {top}, {adv}, {off}),"
                )
                .unwrap();
            }
        }
        for pair in nibbles.chunks(2) {
            bits.push(pair[0] << 4 | pair.get(1).copied().unwrap_or(0));
        }
        let file = format!("font_{}.bin", f.name.to_lowercase());
        std::fs::write(out.join(&file), &bits).unwrap();

        let ascent = scaled.ascent().round() as i32;
        let descent = scaled.descent().round() as i32;
        let line = (scaled.ascent() - scaled.descent() + scaled.line_gap()).ceil() as i32;
        writeln!(
            src,
            "pub static {name}: BakedFont = BakedFont {{\n    ascent: {ascent},\n    descent: {descent},\n    line: {line},\n    em: {em:.3},\n    glyphs: &[\n{glyphs}    ],\n    bits: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{file}\")),\n}};\n",
            name = f.name,
        )
        .unwrap();
    }
    let nerd = std::fs::read(dir.join(NERD)).expect("nerd font");
    let nerd = FontRef::try_from_slice(&nerd).expect("valid nerd font");
    for &(name, file) in BDF_FACES {
        let text = std::fs::read_to_string(dir.join(file)).expect("bdf file");
        bake_bdf(name, &text, out, &mut src);
        if !name.ends_with("_BOLD") {
            bake_term_icons(&nerd, name, &text, out, &mut src);
        }
    }
    std::fs::write(out.join("fonts.rs"), src).unwrap();
}

/// The terminal's Nerd Font icons, subset to the ones yazi's theme uses
/// (`nerd/README.md`).
const NERD: &str = "nerd/SymbolsNerdFont-icons.ttf";

/// Bakes the icons for one Terminus size as `{name}_ICONS`. Each fits a box
/// two cells wide, as programs follow an icon with a space and the terminal
/// draws it across both. Anti-aliased: at 11 px, 1-bit icons fall apart.
fn bake_term_icons(font: &FontRef, name: &str, bdf: &str, out: &Path, src: &mut String) {
    let (ascent, descent, glyphs) = parse_bdf(bdf);
    let cw = glyphs.iter().find(|g| g.cp == 'M' as u32).expect("M").adv;
    let ch = ascent + descent;
    // A pixel clear of the text on each side, and of the row below.
    let (bw, bh) = ((2 * cw - 2) as f32, (ch - 1) as f32);
    let mut ids: Vec<(u32, ab_glyph::GlyphId)> =
        font.codepoint_ids().map(|(id, c)| (c as u32, id)).collect();
    ids.sort_by_key(|&(cp, _)| cp);
    let mut nibbles: Vec<u8> = Vec::new();
    let mut table = String::new();
    for (cp, id) in ids {
        let at = |s: f32| {
            font.outline_glyph(id.with_scale_and_position(PxScale::from(s), point(0.0, 0.0)))
        };
        let Some(g) = at(ch as f32) else { continue };
        let b = g.px_bounds();
        let fit = (bw / b.width()).min(bh / b.height()).min(1.0);
        let g = at(ch as f32 * fit).expect("outline");
        let b = g.px_bounds();
        let (w, h) = (b.width() as usize, b.height() as usize);
        let mut cov = vec![0f32; w * h];
        g.draw(|x, y, c| cov[y as usize * w + x as usize] = c);
        let off = nibbles.len();
        for c in cov {
            nibbles.push((c.clamp(0.0, 1.0).powf(1.0 / COVERAGE_GAMMA) * 15.0).round() as u8);
        }
        // Centred in the box; `top` is from the baseline, at `ascent`.
        let left = (2 * cw - w as i32) / 2;
        let top = (ch - h as i32) / 2 - ascent;
        writeln!(
            table,
            "    g({cp}, {w}, {h}, {left}, {top}, {}, {off}),",
            2 * cw * 64
        )
        .unwrap();
    }
    let bits: Vec<u8> = nibbles
        .chunks(2)
        .map(|p| p[0] << 4 | p.get(1).copied().unwrap_or(0))
        .collect();
    let file = format!("font_{}_icons.bin", name.to_lowercase());
    std::fs::write(out.join(&file), &bits).unwrap();
    writeln!(
        src,
        "pub static {name}_ICONS: BakedFont = BakedFont {{\n    ascent: {ascent},\n    descent: {d},\n    line: {ch},\n    em: {ch}.0,\n    glyphs: &[\n{table}    ],\n    bits: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{file}\")),\n}};\n",
        d = -descent,
    )
    .unwrap();
}

/// One BDF glyph: its code point, advance, box and rows of bits, MSB first.
struct BdfGlyph {
    cp: u32,
    adv: i32,
    w: i32,
    h: i32,
    xo: i32,
    yo: i32,
    rows: Vec<u64>,
}

fn parse_bdf(text: &str) -> (i32, i32, Vec<BdfGlyph>) {
    let (mut ascent, mut descent) = (0, 0);
    let mut glyphs = Vec::new();
    let mut lines = text.lines();
    let num = |s: Option<&str>| s.and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    while let Some(line) = lines.next() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("FONT_ASCENT") => ascent = num(it.next()),
            Some("FONT_DESCENT") => descent = num(it.next()),
            Some("STARTCHAR") => {
                let mut g = BdfGlyph {
                    cp: u32::MAX,
                    adv: 0,
                    w: 0,
                    h: 0,
                    xo: 0,
                    yo: 0,
                    rows: Vec::new(),
                };
                let mut bitmap = false;
                for line in lines.by_ref() {
                    let mut it = line.split_whitespace();
                    match it.next() {
                        Some("ENCODING") => g.cp = num(it.next()) as u32,
                        Some("DWIDTH") => g.adv = num(it.next()),
                        Some("BBX") => {
                            g.w = num(it.next());
                            g.h = num(it.next());
                            g.xo = num(it.next());
                            g.yo = num(it.next());
                        }
                        Some("BITMAP") => bitmap = true,
                        Some("ENDCHAR") => break,
                        Some(hex) if bitmap && g.w > 0 => {
                            let bytes = (g.w as usize).div_ceil(8);
                            let v = u64::from_str_radix(hex, 16).unwrap_or(0);
                            // Left-align the row so bit 63 is the first pixel.
                            g.rows.push(v << (64 - bytes * 8));
                        }
                        _ => {}
                    }
                }
                glyphs.push(g);
            }
            _ => {}
        }
    }
    glyphs.sort_by_key(|g| g.cp);
    (ascent, descent, glyphs)
}

/// Bakes a bitmap face into the same tables as an outline one: full
/// coverage where a bit is set, trimmed to the pixels in use.
fn bake_bdf(name: &str, text: &str, out: &Path, src: &mut String) {
    let (ascent, descent, all) = parse_bdf(text);
    let mut nibbles: Vec<u8> = Vec::new();
    let mut glyphs = String::new();
    for g in all
        .iter()
        .filter(|g| TERM.iter().any(|&(lo, hi)| (lo..=hi).contains(&g.cp)))
    {
        let on = |x: i32, y: i32| g.rows[y as usize] & (1u64 << (63 - x)) != 0;
        let mut b: Option<(i32, i32, i32, i32)> = None;
        for y in 0..g.h.min(g.rows.len() as i32) {
            for x in 0..g.w {
                if on(x, y) {
                    let (x0, y0, x1, y1) = b.unwrap_or((x, y, x, y));
                    b = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
                }
            }
        }
        let off = nibbles.len();
        let (w, h, left, top) = match b {
            Some((x0, y0, x1, y1)) => {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        nibbles.push(if on(x, y) { 15 } else { 0 });
                    }
                }
                // The box's top row sits yo + h above the baseline.
                (x1 - x0 + 1, y1 - y0 + 1, g.xo + x0, -(g.yo + g.h) + y0)
            }
            None => (0, 0, 0, 0),
        };
        writeln!(
            glyphs,
            "    g({}, {w}, {h}, {left}, {top}, {}, {off}),",
            g.cp,
            g.adv * 64
        )
        .unwrap();
    }
    let bits: Vec<u8> = nibbles
        .chunks(2)
        .map(|p| p[0] << 4 | p.get(1).copied().unwrap_or(0))
        .collect();
    let file = format!("font_{}.bin", name.to_lowercase());
    std::fs::write(out.join(&file), &bits).unwrap();
    let line = ascent + descent;
    writeln!(
        src,
        "pub static {name}: BakedFont = BakedFont {{\n    ascent: {ascent},\n    descent: {d},\n    line: {line},\n    em: {line}.0,\n    glyphs: &[\n{glyphs}    ],\n    bits: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{file}\")),\n}};\n",
        d = -descent,
    )
    .unwrap();
}

fn x_height(font: &FontRef) -> Option<f32> {
    let id = font.glyph_id('x');
    font.outline(id)
        .map(|o| (o.bounds.max.y - o.bounds.min.y).abs())
}

fn bake_icons(dir: &Path, out: &Path) {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "svg").then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();

    let mut blob: Vec<u8> = Vec::new();
    let mut arms = String::new();
    for name in &names {
        let svg = std::fs::read_to_string(dir.join(format!("{name}.svg"))).unwrap();
        for &(size, stroke_px) in ICON_SIZES {
            // Lucide's 24 grid halves cleanly to 12 but not to 14: there a
            // stroke straddles two pixels. So 14 draws the 12 px icon with
            // a pixel of room each side.
            let (s, inset) = if size == 14 {
                (0.5, 1.0)
            } else {
                (size as f32 / 24.0, 0.0)
            };
            let units = stroke_px / s;
            let svg = svg.replace(
                "stroke-width=\"2\"",
                &format!("stroke-width=\"{units:.3}\""),
            );
            let svg = svg.replace("currentColor", "#000");
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
                .expect("valid svg");
            let render = |dx: f32, dy: f32| {
                let (dx, dy) = (dx + inset, dy + inset);
                let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
                resvg::render(
                    &tree,
                    resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy),
                    &mut pixmap.as_mut(),
                );
                pixmap
                    .pixels()
                    .iter()
                    .map(|p| p.alpha())
                    .collect::<Vec<u8>>()
            };
            let alpha = if size <= PIXEL_ICON_MAX {
                // The shift, of four half-pixel ones, that leaves the least
                // grey: that is where the strokes land on whole pixels.
                let grey = |a: &[u8]| a.iter().filter(|&&v| v > 24 && v < 232).count();
                let best = [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)]
                    .into_iter()
                    .map(|(dx, dy)| render(dx, dy))
                    .min_by_key(|a| grey(a))
                    .unwrap();
                let t = PIXEL_ICON_SHARPEN;
                best.iter()
                    .map(|&v| {
                        let c = v as f32 / 255.0;
                        let c = ((c - t) / (1.0 - 2.0 * t)).clamp(0.0, 1.0);
                        let c = c * c * (3.0 - 2.0 * c);
                        (c * 255.0).round() as u8
                    })
                    .collect()
            } else {
                render(0.0, 0.0)
            };
            let off = blob.len();
            blob.extend(alpha);
            writeln!(
                arms,
                "        (\"{name}\", {size}) => Some(({off}, {size})),"
            )
            .unwrap();
        }
    }
    std::fs::write(out.join("icons.bin"), &blob).unwrap();
    let src = format!(
        "pub static ICON_BITS: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/icons.bin\"));\n\n\
         /// The names baked in, for tests and tooling.\n\
         pub const ICON_NAMES: &[&str] = &{names:?};\n\n\
         pub fn icon_slot(name: &str, size: u16) -> Option<(usize, u16)> {{\n    match (name, size) {{\n{arms}        _ => None,\n    }}\n}}\n"
    );
    std::fs::write(out.join("icons.rs"), src).unwrap();
}
