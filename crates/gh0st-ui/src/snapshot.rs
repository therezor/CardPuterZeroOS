//! Writes the back buffer to a PNG. Used for screenshots on the device and
//! for mockups and golden tests on any host.

use crate::canvas::Canvas;
use crate::color::to_rgb8;
use std::path::Path;

/// Saves `c` scaled up `scale` times with nearest-neighbour, so each panel
/// pixel stays a sharp square.
pub fn write_png(c: &Canvas, path: &Path, scale: u32) -> std::io::Result<()> {
    let (w, h) = (c.width() as u32, c.height() as u32);
    let (sw, sh) = (w * scale, h * scale);
    let mut data = Vec::with_capacity((sw * sh * 3) as usize);
    for y in 0..sh {
        let row = c.row((y / scale) as i32);
        for x in 0..sw {
            data.extend_from_slice(&to_rgb8(row[(x / scale) as usize]));
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, sw, sh);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(&data).map_err(std::io::Error::other)
}
