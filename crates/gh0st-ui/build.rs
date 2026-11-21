//! Bakes fonts and icons into `.rodata` so nothing is parsed or rasterized at
//! run time. A glyph or icon costs a page fault the first time it is drawn,
//! and nothing before that.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, VariableFont, point};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Light text on a dark panel reads thin with linear coverage. This lifts the
/// mid tones a little, which stands in for the hinting ab_glyph does not do.
const COVERAGE_GAMMA: f32 = 1.25;

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

const MONO: &[(u32, u32)] = &[
    (0x20, 0x7e),
    (0xa0, 0x17f),
    (0x400, 0x45f),
    (0x490, 0x491),
    (0x2010, 0x2027),
    (0x2190, 0x2195),
    (0x2500, 0x25ff),
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

/// Inter steps in whole-pixel x-heights: 13 px gives a 7 px x-height, 15 gives
/// 8, 17 gives 9. Sizes between snap to the same face.
const FACES: &[Face] = &[
    face("SANS_13", "Inter.ttf", 13.0, 400.0, TEXT, true),
    face("SANS_13_BOLD", "Inter.ttf", 13.0, 600.0, TEXT, true),
    face("SANS_15", "Inter.ttf", 15.0, 400.0, TEXT, true),
    face("SANS_15_BOLD", "Inter.ttf", 15.0, 600.0, TEXT, true),
    face("SANS_17_BOLD", "Inter.ttf", 17.0, 600.0, TEXT, true),
    face("SANS_24_BOLD", "Inter.ttf", 24.0, 600.0, TEXT, true),
    face("MONO_10", "JetBrainsMono.ttf", 10.0, 400.0, MONO, false),
    face(
        "MONO_10_BOLD",
        "JetBrainsMono.ttf",
        10.0,
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
const ICON_SIZES: &[(u32, f32)] = &[(12, 1.4), (14, 1.5), (16, 1.6), (26, 2.0), (36, 2.6)];

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", root.join("fonts").display());
    println!("cargo:rerun-if-changed={}", root.join("icons").display());
    println!("cargo:rerun-if-changed=build.rs");

    bake_fonts(&root.join("fonts"), &out);
    bake_icons(&root.join("icons"), &out);
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
                let glyph = id.with_scale_and_position(scale, point(0.0, 0.0));
                let (w, h, left, top, off) = match font.outline_glyph(glyph) {
                    Some(outline) => {
                        let b = outline.px_bounds();
                        let (w, h) = (b.width() as usize, b.height() as usize);
                        let mut cov = vec![0f32; w * h];
                        outline.draw(|x, y, c| cov[y as usize * w + x as usize] = c);
                        let off = nibbles.len();
                        for c in cov {
                            let c = c.clamp(0.0, 1.0).powf(1.0 / COVERAGE_GAMMA);
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
    std::fs::write(out.join("fonts.rs"), src).unwrap();
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
            let units = stroke_px * 24.0 / size as f32;
            let svg = svg.replace(
                "stroke-width=\"2\"",
                &format!("stroke-width=\"{units:.3}\""),
            );
            let svg = svg.replace("currentColor", "#000");
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
                .expect("valid svg");
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
            let s = size as f32 / 24.0;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(s, s),
                &mut pixmap.as_mut(),
            );
            let off = blob.len();
            blob.extend(pixmap.pixels().iter().map(|p| p.alpha()));
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
