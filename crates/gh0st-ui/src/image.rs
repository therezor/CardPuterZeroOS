//! App icons. `.desktop` icons are 100x100 RGBA PNGs; a tile shows 36 px. So
//! an icon is decoded once, area-averaged down, and kept as RGB565 plus alpha.

use crate::color::{Color, rgb};
use std::path::Path;

pub struct Image {
    pub w: i32,
    pub h: i32,
    pub px: Vec<Color>,
    pub alpha: Vec<u8>,
}

impl Image {
    /// Decodes a PNG and scales it to fit `size` x `size`, keeping its aspect.
    pub fn load_png(path: &Path, size: u32) -> Result<Image, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        decode(std::io::BufReader::new(file), size).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// As [`load_png`](Image::load_png), from PNG bytes in memory: one icon
    /// of the Store's pack.
    pub fn from_png(bytes: &[u8], size: u32) -> Result<Image, String> {
        decode(std::io::Cursor::new(bytes), size)
    }
}

fn decode<R: std::io::BufRead + std::io::Seek>(r: R, size: u32) -> Result<Image, String> {
    let mut dec = png::Decoder::new(r);
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let rgba = to_rgba(&buf[..info.buffer_size()], info.color_type);
    Ok(scale_rgba(&rgba, info.width, info.height, size))
}

fn to_rgba(buf: &[u8], ct: png::ColorType) -> Vec<[u8; 4]> {
    match ct {
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        _ => buf.iter().map(|&g| [g, g, g, 255]).collect(),
    }
}

/// Box filter in premultiplied space, so transparent edges do not bleed dark
/// fringes into the result.
fn scale_rgba(src: &[[u8; 4]], sw: u32, sh: u32, size: u32) -> Image {
    let s = size as f32 / sw.max(sh) as f32;
    let dw = ((sw as f32 * s).round() as u32).max(1);
    let dh = ((sh as f32 * s).round() as u32).max(1);
    let mut px = Vec::with_capacity((dw * dh) as usize);
    let mut alpha = Vec::with_capacity((dw * dh) as usize);
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let [r, g, b, a] = src[(y * sw + x) as usize];
                    let a32 = a as u32;
                    acc[0] += r as u32 * a32;
                    acc[1] += g as u32 * a32;
                    acc[2] += b as u32 * a32;
                    acc[3] += a32;
                }
            }
            let n = (x1 - x0) * (y1 - y0);
            let a = acc[3];
            if a == 0 {
                px.push(0);
                alpha.push(0);
            } else {
                let div = |v: u32| (v / a.max(1)) as u8;
                px.push(rgb(div(acc[0]), div(acc[1]), div(acc[2])));
                alpha.push((a / n) as u8);
            }
        }
    }
    Image {
        w: dw as i32,
        h: dh as i32,
        px,
        alpha,
    }
}
