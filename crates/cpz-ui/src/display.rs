//! Where frames go: the panel, or numbered PNGs when the shell runs on a
//! host for development (`cpz shell --sim DIR`).

use crate::canvas::Canvas;
use crate::fb::Framebuffer;
use crate::geom::Rect;
use std::path::PathBuf;

pub trait Display {
    /// Writes the rows `area` covers. Returns how many changed.
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize;
    /// Reads the current picture back, for a screenshot over an app.
    fn read_into(&mut self, c: &mut Canvas);
    /// The device path a child app should draw to.
    fn device(&self) -> String;
}

impl Display for Framebuffer {
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize {
        Framebuffer::flush(self, c, area)
    }

    fn read_into(&mut self, c: &mut Canvas) {
        Framebuffer::read_into(self, c)
    }

    fn device(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

/// Saves each frame that differs from the last as `frame-NNN.png`, 2x.
pub struct Frames {
    dir: PathBuf,
    count: usize,
    last: Canvas,
}

impl Frames {
    pub fn new(dir: PathBuf, w: i32, h: i32) -> std::io::Result<Frames> {
        std::fs::create_dir_all(&dir)?;
        Ok(Frames {
            dir,
            count: 0,
            last: Canvas::new(w, h),
        })
    }
}

impl Display for Frames {
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize {
        // `c` may be a strip smaller than the panel (the ESC hint), drawn at
        // the top. Copy its rows into the full picture, as the panel would.
        let area = area.intersect(&c.bounds()).intersect(&self.last.bounds());
        let w = c.width().min(self.last.width());
        let mut changed = 0;
        for y in area.y..area.bottom() {
            if c.row(y)[..w as usize] != self.last.row(y)[..w as usize] {
                changed += 1;
                self.last.copy_row_from(c, y, w);
            }
        }
        if changed > 0 {
            let path = self.dir.join(format!("frame-{:03}.png", self.count));
            self.count += 1;
            if let Err(e) = crate::snapshot::write_png(&self.last, &path, 2) {
                eprintln!("sim: {}: {e}", path.display());
            }
        }
        changed
    }

    fn read_into(&mut self, c: &mut Canvas) {
        *c = self.last.clone();
    }

    fn device(&self) -> String {
        "/dev/null".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_lands_in_the_full_frame() {
        let dir = std::env::temp_dir().join(format!("cpz-frames-{}", std::process::id()));
        let mut f = Frames::new(dir.clone(), 320, 170).unwrap();
        let mut full = Canvas::new(320, 170);
        full.fill(full.bounds(), 0x1234);
        assert_eq!(f.flush(&full, full.bounds()), 170);
        let mut strip = Canvas::new(320, 20);
        strip.fill(strip.bounds(), 0xffff);
        assert_eq!(f.flush(&strip, strip.bounds()), 20);
        assert_eq!(
            f.flush(&full, full.bounds()),
            20,
            "only the strip rows differ"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
