//! The picture behind home and the lock screen. It is baked once, when the
//! shell starts or the picture or palette changes, into RGB565 buffers the
//! size of the screen: home's dimmed, the lock's brighter, and a blurred
//! frost the panes show. A frame then copies; it never blends a picture.
//!
//! A picture of the user's own is cropped to the screen when it is picked
//! (`import`), reading the source a row at a time, and only that crop is
//! kept. A huge source never sits in memory.

use crate::canvas::Backdrop;
use crate::color::{Color, contrast, rgb, to_rgb8};
use crate::theme::Palette;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

/// The built-in pictures, drawn from the palette's colours.
pub const BUILTINS: &[&str] = &["aurora", "contour"];

/// `gh0st.wallpaper` for the user's own picture, kept at [`picture_path`].
pub const OWN: &str = "picture";
/// `gh0st.wallpaper` for no picture: the palette's `bg`.
pub const OFF: &str = "off";

/// What a canvas shows, as a picture to bake: a frozen screen behind a
/// question about it.
pub fn picture_of(c: &crate::canvas::Canvas) -> Picture {
    let px = c
        .pixels()
        .iter()
        .map(|&p| {
            let [r, g, b] = to_rgb8(p);
            [r as f32, g as f32, b as f32]
        })
        .collect();
    Picture {
        w: c.width() as usize,
        h: c.height() as usize,
        px,
    }
}

/// Where the crop of the user's picture lives, beside config.json.
pub fn picture_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("wallpaper.png")
}

/// A picture at the screen's size, 8 bits a channel, before it is dimmed.
#[derive(Clone)]
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
}

/// Home's and the lock's grounds. They share one frost.
pub struct Baked {
    pub home: Rc<Backdrop>,
    pub lock: Rc<Backdrop>,
}

fn f3(c: Color) -> [f32; 3] {
    let [r, g, b] = to_rgb8(c);
    [r as f32, g as f32, b as f32]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A built-in picture by name, in the palette's colours.
pub fn builtin(name: &str, p: &Palette, w: usize, h: usize) -> Option<Picture> {
    let px = match name {
        "aurora" => aurora(p, w, h),
        "contour" => contour(p, w, h),
        _ => return None,
    };
    Some(Picture { w, h, px })
}

/// Soft ribbons of the accent and two tints over a deep ground.
fn aurora(p: &Palette, w: usize, h: usize) -> Vec<[f32; 3]> {
    let deep = lerp3(f3(p.bg), [0.0; 3], 0.35);
    let cols = [f3(p.accent), f3(p.tints[1]), f3(p.tints[4])];
    // Each ribbon's centre and width change only across: work them out
    // once a column.
    let bands: Vec<[(f32, f32); 3]> = (0..w)
        .map(|x| {
            let fx = x as f32 / w as f32;
            std::array::from_fn(|i| {
                let k = i as f32;
                let centre = 0.30
                    + 0.16 * k
                    + 0.10 * (fx * 5.0 + k * 1.7).sin()
                    + 0.05 * (fx * 11.0 - k).sin();
                (centre, 0.10 + 0.05 * ((fx * 3.0 + k * 2.3).sin() + 1.0))
            })
        })
        .collect();
    let mut v = Vec::with_capacity(w * h);
    for y in 0..h {
        let fy = y as f32 / h as f32;
        let ground = lerp3(deep, f3(p.bg), fy);
        for (x, band) in bands.iter().enumerate() {
            let fade = 0.55 + 0.45 * (1.0 - x as f32 / w as f32);
            let mut c = ground;
            for (i, col) in cols.iter().enumerate() {
                let (centre, width) = band[i];
                let a = (1.0 - smooth((fy - centre).abs() / width)) * (0.55 - 0.12 * i as f32);
                c = lerp3(c, *col, (a * fade).clamp(0.0, 1.0));
            }
            v.push(c);
        }
    }
    v
}

/// Lines of a height map over a gradient, in the accent.
fn contour(p: &Palette, w: usize, h: usize) -> Vec<[f32; 3]> {
    let a = f3(p.bg);
    let b = lerp3(a, f3(p.tints[1]), 0.25);
    let line = f3(p.accent);
    let mut v = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / 80.0, y as f32 / 80.0);
            let z = (fx * 1.3 + 0.7).sin() * (fy * 1.7 + 0.3).cos()
                + 0.6 * (fx * 0.7 - fy * 1.1 + 2.0).sin()
                + 0.3 * (fx * 2.9 + fy * 2.3).sin();
            let d = ((z * 4.0).rem_euclid(1.0) - 0.5).abs() * 2.0;
            let ink = (1.0 - smooth((1.0 - d) / 0.12)) * 0.65;
            let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
            v.push(lerp3(lerp3(a, b, t), line, ink));
        }
    }
    v
}

/// The most a source may hold, so a decode stays bounded whatever the file
/// says: 64 MB.
const DECODE_LIMIT: usize = 64 << 20;

/// Reads a PNG and crops it to cover `w` x `h`, averaging each output pixel
/// over the source pixels under it. Rows are read one at a time, so memory
/// stays near a row of the source plus the result.
pub fn import(src: impl Read, w: usize, h: usize) -> Result<Picture, String> {
    let limits = png::Limits {
        bytes: DECODE_LIMIT,
    };
    let mut dec = png::Decoder::new_with_limits(std::io::BufReader::new(src), limits);
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let (sw, sh) = {
        let i = reader.info();
        (i.width as usize, i.height as usize)
    };
    if sw == 0 || sh == 0 {
        return Err("empty picture".into());
    }
    let ch = match reader.output_color_type().0 {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("indexed colour after expand".into()),
    };
    let pixel = |row: &[u8], x: usize| -> [f32; 3] {
        let p = &row[x * ch..];
        if ch >= 3 {
            [p[0] as f32, p[1] as f32, p[2] as f32]
        } else {
            [p[0] as f32; 3]
        }
    };
    // The crop: the largest window of the screen's shape, centred.
    let s = (sw as f64 / w as f64).min(sh as f64 / h as f64);
    let (ox, oy) = (
        (sw as f64 - w as f64 * s) / 2.0,
        (sh as f64 - h as f64 * s) / 2.0,
    );
    let col_of = |x: usize| ((x as f64 + 0.5 - ox) / s).floor();
    let mut sum = vec![[0f32; 3]; w * h];
    let mut n = vec![0u32; w * h];
    let mut add = |y: usize, row: &[u8]| {
        let oyf = ((y as f64 + 0.5 - oy) / s).floor();
        if oyf < 0.0 || oyf >= h as f64 {
            return;
        }
        let j = oyf as usize;
        for x in 0..sw {
            let i = col_of(x);
            if i < 0.0 || i >= w as f64 {
                continue;
            }
            let k = j * w + i as usize;
            let p = pixel(row, x);
            for c in 0..3 {
                sum[k][c] += p[c];
            }
            n[k] += 1;
        }
    };
    if reader.info().interlaced {
        // Passes arrive out of order, so take the frame whole: the limit
        // above still bounds it.
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        for (y, row) in buf.chunks(info.line_size).take(sh).enumerate() {
            add(y, row);
        }
    } else {
        let mut y = 0;
        while let Some(row) = reader.next_row().map_err(|e| e.to_string())? {
            add(y, row.data());
            y += 1;
        }
    }
    // A source smaller than the screen leaves gaps: fill each from the
    // nearest row or column above or left that has one.
    let mut px = vec![[0f32; 3]; w * h];
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            px[k] = if n[k] > 0 {
                let d = n[k] as f32;
                [sum[k][0] / d, sum[k][1] / d, sum[k][2] / d]
            } else if i > 0 {
                px[k - 1]
            } else if j > 0 {
                px[k - w]
            } else {
                [0.0; 3]
            };
        }
    }
    Ok(Picture { w, h, px })
}

/// The crop as a PNG, to keep beside config.json.
pub fn encode(p: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, p.w as u32, p.h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> =
        p.px.iter()
            .flat_map(|c| c.map(|v| v.clamp(0.0, 255.0) as u8))
            .collect();
    if let Ok(mut wr) = enc.write_header() {
        let _ = wr.write_image_data(&data);
    }
    out
}

/// Reads a kept crop back. Anything but the screen's size is refused, so a
/// file swapped in by hand can't cost more than the crop would.
pub fn load(path: &Path, w: usize, h: usize) -> Option<Picture> {
    let head = png::Decoder::new(std::fs::File::open(path).ok()?)
        .read_info()
        .ok()?;
    let i = head.info();
    if (i.width as usize, i.height as usize) != (w, h) {
        return None;
    }
    import(std::fs::File::open(path).ok()?, w, h).ok()
}

/// What a dither can take from a contrast: each channel moves a step.
const DITHER_SLACK: f32 = 0.3;

/// How much of the picture to keep, mixed toward `to`, so each ink reads at
/// its ratio on the pixel that is hardest to read on, and never more than
/// `max`. Found from that one pixel, so it costs nothing to bake.
fn fit_keep(px: &[[f32; 3]], to: [f32; 3], inks: &[(Color, f32)], max: f32) -> f32 {
    let col = |c: [f32; 3]| rgb(c[0] as u8, c[1] as u8, c[2] as u8);
    let Some(&(first, _)) = inks.first() else {
        return max;
    };
    // Light text is hardest to read on the brightest pixel, dark text on the
    // darkest. A weighted sum finds it without the gamma per pixel.
    let light = crate::color::luminance(first) > 0.5;
    let y = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let pick = px.iter().max_by(|a, b| {
        let o = y(a).partial_cmp(&y(b)).unwrap_or(std::cmp::Ordering::Equal);
        if light { o } else { o.reverse() }
    });
    let Some(&worst) = pick else {
        return max;
    };
    let ok = |k: f32| {
        let c = col(lerp3(to, worst, k));
        inks.iter()
            .all(|&(ink, ratio)| contrast(ink, c) >= ratio + DITHER_SLACK)
    };
    if ok(max) {
        return max;
    }
    let (mut lo, mut hi) = (0.0f32, max);
    for _ in 0..12 {
        let mid = (lo + hi) / 2.0;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Mixes toward `to` under the bars, and for the lock behind the clock, so
/// text there reads on any picture.
fn scrims(
    px: &mut [[f32; 3]],
    w: usize,
    h: usize,
    to: [f32; 3],
    top: usize,
    foot: usize,
    halo: f32,
) {
    for y in 0..h {
        let mut a: f32 = 0.0;
        if y < top + 8 {
            a = a.max(0.65 * (1.0 - y as f32 / (top + 8) as f32));
        }
        if y + foot + 10 > h {
            a = a.max(0.7 * (y + foot + 10 - h) as f32 / (foot + 10) as f32);
        }
        for x in 0..w {
            let mut b = a;
            if halo > 0.0 {
                let dx = (x as f32 - w as f32 / 2.0) / 120.0;
                let dy = (y as f32 - h as f32 * 0.41) / 50.0;
                b = b.max(halo * (1.0 - smooth((dx * dx + dy * dy).sqrt())));
            }
            let c = &mut px[y * w + x];
            *c = lerp3(*c, to, b);
        }
    }
}

/// A box blur run three times, near a gaussian, with running sums: the
/// same cost whatever the radius.
fn blur(px: &[[f32; 3]], w: usize, h: usize, r: usize) -> Vec<[f32; 3]> {
    let mut cur = px.to_vec();
    let mut next = cur.clone();
    for _ in 0..3 {
        for (len, stride, lines, step) in [(w, 1, h, w), (h, w, w, 1)] {
            for l in 0..lines {
                let base = l * step;
                let at = |i: usize| base + i * stride;
                let mut acc = [0f32; 3];
                let mut n = 0f32;
                for i in 0..r.min(len) {
                    for c in 0..3 {
                        acc[c] += cur[at(i)][c];
                    }
                    n += 1.0;
                }
                for i in 0..len {
                    if i + r < len {
                        for c in 0..3 {
                            acc[c] += cur[at(i + r)][c];
                        }
                        n += 1.0;
                    }
                    if i > r {
                        for c in 0..3 {
                            acc[c] -= cur[at(i - r - 1)][c];
                        }
                        n -= 1.0;
                    }
                    next[at(i)] = [acc[0] / n, acc[1] / n, acc[2] / n];
                }
            }
            std::mem::swap(&mut cur, &mut next);
        }
    }
    cur
}

const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// To RGB565 with an ordered dither, so a soft gradient doesn't band.
fn to565(px: &[[f32; 3]], w: usize) -> Vec<Color> {
    px.iter()
        .enumerate()
        .map(|(i, c)| {
            let (x, y) = (i % w, i / w);
            let d = (BAYER[y & 3][x & 3] as f32 + 0.5) / 16.0;
            let q = |v: f32, levels: f32| -> u8 {
                let s = v.clamp(0.0, 255.0) / 255.0 * levels;
                let k = (s.floor() + if s.fract() > d { 1.0 } else { 0.0 }).min(levels);
                (k / levels * 255.0).round() as u8
            };
            rgb(q(c[0], 31.0), q(c[1], 63.0), q(c[2], 31.0))
        })
        .collect()
}

/// How dark home is: `fg` must read at this on every pixel, and no more
/// than this much of the picture shows. `muted` must read at 3:1 on home,
/// the lock and the frost.
const HOME_RATIO: f32 = 6.0;
const MUTED_RATIO: f32 = 3.0;
/// Text on a pane.
const FROST_RATIO: f32 = 4.5;
const HOME_MAX: f32 = 0.65;
/// The lock shows more of the picture and dims behind its clock instead.
const LOCK_RATIO: f32 = 4.5;
const LOCK_MAX: f32 = 0.85;
const LOCK_HALO: f32 = 0.35;
/// The frost: blurred this far, then mostly `surface`.
const FROST_R: usize = 7;
const FROST_KEEP: f32 = 0.28;

/// Bakes home's and the lock's grounds from `pic`, for `bar` and `foot` px
/// of bars on top and below.
pub fn bake(pic: &Picture, p: &Palette, bar: usize, foot: usize) -> Baked {
    let (w, h) = (pic.w, pic.h);
    let to = f3(p.bg);
    let dim = |keep: f32| -> Vec<[f32; 3]> { pic.px.iter().map(|c| lerp3(to, *c, keep)).collect() };
    let muted = (p.muted, MUTED_RATIO);
    let home_keep = fit_keep(&pic.px, to, &[(p.fg, HOME_RATIO), muted], HOME_MAX);
    let mut home = dim(home_keep);
    scrims(&mut home, w, h, to, bar, foot, 0.0);
    let lock_keep = fit_keep(&pic.px, to, &[(p.fg, LOCK_RATIO), muted], LOCK_MAX);
    let mut lock = dim(lock_keep);
    scrims(&mut lock, w, h, to, bar, foot, LOCK_HALO);
    let frost = frost_of(pic, p);
    Baked {
        home: Rc::new(Backdrop {
            back: to565(&home, w),
            frost: frost.clone(),
        }),
        lock: Rc::new(Backdrop {
            back: to565(&lock, w),
            frost,
        }),
    }
}

/// The frost panes show: the picture blurred and pulled toward `surface`
/// until text on it reads.
fn frost_of(pic: &Picture, p: &Palette) -> Rc<Vec<Color>> {
    let s = f3(p.surface);
    let blurred = blur(&pic.px, pic.w, pic.h, FROST_R);
    let keep = fit_keep(
        &blurred,
        s,
        &[(p.fg, FROST_RATIO), (p.muted, MUTED_RATIO)],
        FROST_KEEP,
    );
    let frost: Vec<[f32; 3]> = blurred.into_iter().map(|c| lerp3(s, c, keep)).collect();
    Rc::new(to565(&frost, pic.w))
}

/// A frozen screen to ask a question over: as it was, with scrims under
/// the bars so they read, and a frost for the card.
pub fn frozen(c: &crate::canvas::Canvas, p: &Palette, bar: usize, foot: usize) -> Rc<Backdrop> {
    let pic = picture_of(c);
    let mut back = pic.px.clone();
    scrims(&mut back, pic.w, pic.h, f3(p.bg), bar, foot, 0.0);
    Rc::new(Backdrop {
        back: to565(&back, pic.w),
        frost: frost_of(&pic, p),
    })
}

/// Bumped when the bake changes, so an old cache is never shown.
const CACHE_VERSION: u32 = 1;

/// The baked buffers as bytes, behind a line naming what they were baked
/// from, to keep in the user's cache: a start then skips the bake.
pub fn cache_bytes(b: &Baked, key: &str) -> Vec<u8> {
    let mut out = format!("gh0st-wallpaper {CACHE_VERSION} {key}\n").into_bytes();
    for buf in [&b.home.back, &b.lock.back, &*b.home.frost] {
        out.extend(buf.iter().flat_map(|c| c.to_le_bytes()));
    }
    out
}

/// The buffers back from a cache written for `key` and `n` pixels, or
/// none when it was written for anything else.
pub fn from_cache(bytes: &[u8], key: &str, n: usize) -> Option<Baked> {
    let head = format!("gh0st-wallpaper {CACHE_VERSION} {key}\n");
    let body = bytes.strip_prefix(head.as_bytes())?;
    if body.len() != 3 * 2 * n {
        return None;
    }
    let buf = |i: usize| -> Vec<Color> {
        body[i * 2 * n..(i + 1) * 2 * n]
            .chunks_exact(2)
            .map(|p| Color::from_le_bytes([p[0], p[1]]))
            .collect()
    };
    let frost = Rc::new(buf(2));
    Some(Baked {
        home: Rc::new(Backdrop {
            back: buf(0),
            frost: frost.clone(),
        }),
        lock: Rc::new(Backdrop {
            back: buf(1),
            frost,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::MINT;

    fn png_of(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let px = (0..w * h)
            .map(|i| {
                let c = f(i % w, i / w);
                [c[0] as f32, c[1] as f32, c[2] as f32]
            })
            .collect();
        encode(&Picture { w, h, px })
    }

    #[test]
    fn import_crops_a_tall_picture_to_its_middle() {
        // Top third red, middle green, bottom blue: the screen's shape is
        // wide, so the crop keeps the middle.
        let src = png_of(90, 300, |_, y| match y {
            0..100 => [255, 0, 0],
            100..200 => [0, 255, 0],
            _ => [0, 0, 255],
        });
        let p = import(&src[..], 32, 17).unwrap();
        for c in &p.px {
            assert!(c[1] > 200.0 && c[0] < 10.0 && c[2] < 10.0, "{c:?}");
        }
    }

    #[test]
    fn a_picture_smaller_than_the_screen_fills_it() {
        let src = png_of(8, 4, |x, _| [x as u8 * 30, 0, 0]);
        let p = import(&src[..], 32, 17).unwrap();
        assert_eq!(p.px.len(), 32 * 17);
        assert!(p.px.iter().all(|c| c[1] == 0.0));
    }

    #[test]
    fn a_kept_crop_of_the_wrong_size_is_refused() {
        let dir = std::env::temp_dir().join(format!("gh0st-wall-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wallpaper.png");
        std::fs::write(&path, png_of(640, 340, |_, _| [9, 9, 9])).unwrap();
        assert!(load(&path, 32, 17).is_none());
        std::fs::write(&path, png_of(32, 17, |_, _| [9, 9, 9])).unwrap();
        assert!(load(&path, 32, 17).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_reads_on_home_over_a_white_picture() {
        for p in crate::theme::PALETTES
            .iter()
            .chain(crate::theme::EARNED)
            .copied()
        {
            let white = Picture {
                w: 32,
                h: 40,
                px: vec![[255.0; 3]; 32 * 40],
            };
            let black = Picture {
                px: vec![[0.0; 3]; 32 * 40],
                ..white.clone()
            };
            for pic in [white, black] {
                let b = bake(&pic, &p, 4, 4);
                // Dithering moves a channel a step, so a little slack.
                let reads = |buf: &[Color], ink: Color, ratio: f32, what: &str| {
                    for c in buf {
                        let got = contrast(ink, *c);
                        assert!(got >= ratio, "{} {what}: {got}", p.name);
                    }
                };
                reads(&b.home.back, p.fg, HOME_RATIO, "fg on home");
                reads(&b.home.back, p.muted, MUTED_RATIO, "muted on home");
                reads(&b.lock.back, p.fg, LOCK_RATIO, "fg on the lock");
                reads(&b.lock.back, p.muted, MUTED_RATIO, "muted on the lock");
                reads(&b.home.frost, p.fg, FROST_RATIO, "fg on frost");
                reads(&b.home.frost, p.muted, MUTED_RATIO, "muted on frost");
            }
        }
    }

    #[test]
    fn a_cache_comes_back_only_for_its_own_key() {
        let pic = builtin("aurora", &MINT, 32, 17).unwrap();
        let b = bake(&pic, &MINT, 4, 4);
        let bytes = cache_bytes(&b, "aurora Mint");
        let back = from_cache(&bytes, "aurora Mint", 32 * 17).unwrap();
        assert_eq!(back.home.back, b.home.back);
        assert_eq!(back.lock.back, b.lock.back);
        assert_eq!(*back.lock.frost, *b.home.frost);
        assert!(from_cache(&bytes, "aurora Nord", 32 * 17).is_none());
        assert!(from_cache(&bytes, "aurora Mint", 33 * 17).is_none());
        assert!(from_cache(&bytes[..bytes.len() - 1], "aurora Mint", 32 * 17).is_none());
    }

    #[test]
    fn every_builtin_draws_in_every_palette() {
        for name in BUILTINS {
            for p in crate::theme::PALETTES {
                let pic = builtin(name, p, 32, 17).unwrap();
                assert!(pic.px.iter().all(|c| c.iter().all(|v| v.is_finite())));
            }
        }
        assert!(builtin("nope", &MINT, 4, 4).is_none());
    }
}
