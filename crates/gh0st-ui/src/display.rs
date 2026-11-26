//! Where frames go: the panel, or numbered PNGs when the shell runs on a
//! host for development (`gh0st shell --sim DIR`).

use crate::canvas::Canvas;
use crate::fb::Framebuffer;
use crate::geom::Rect;
use std::path::PathBuf;

pub trait Display {
    /// Writes the rows `area` covers. Returns how many changed.
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize;
    /// Writes every row without comparing. After an app or the boot splash
    /// has drawn, the mapping may not hold what is on the glass (a DRM client
    /// bypasses it), so a compare could skip rows that need writing.
    fn flush_all(&mut self, c: &Canvas);
    /// Reads the current picture back, for a screenshot over an app.
    fn read_into(&mut self, c: &mut Canvas);
    /// Reads back only `area`; the rest of `c` is left as it was.
    fn read_rect(&mut self, c: &mut Canvas, area: Rect) {
        let mut all = c.clone();
        self.read_into(&mut all);
        let area = area.intersect(&c.bounds()).intersect(&all.bounds());
        for y in area.y..area.bottom() {
            c.copy_span_from(&all, y, area.x, area.w);
        }
    }
    /// Hands the panel driver an image to lay over every send, so an app
    /// drawing underneath never covers it. `px` is `area`'s pixels, row by
    /// row; `mask`, one byte a pixel, non-zero where it shows, or None for
    /// all of it. False if the driver can't, and the caller draws it itself.
    fn overlay_set(&mut self, slot: u8, area: Rect, px: &[u16], mask: Option<&[u8]>) -> bool {
        let _ = (slot, area, px, mask);
        false
    }
    /// Takes slot `slot` off; the driver sends the app's own pixels again.
    fn overlay_clear(&mut self, slot: u8) {
        let _ = slot;
    }
    /// The device path a child app should draw to.
    fn device(&self) -> String;
    /// Lets go of the panel before an app starts. Overlays drawn meanwhile
    /// (the ESC hint) go through whatever the app sees.
    fn suspend(&mut self) {}
    /// Takes the panel back after the app exits.
    fn resume(&mut self) {}
    /// Blocks until the last flush is on the panel. False if the display
    /// can't tell, so the caller keeps its own frame timer.
    fn present(&mut self) -> bool {
        false
    }
}

impl Display for Framebuffer {
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize {
        Framebuffer::flush(self, c, area)
    }

    fn flush_all(&mut self, c: &Canvas) {
        Framebuffer::flush_all(self, c)
    }

    fn read_into(&mut self, c: &mut Canvas) {
        Framebuffer::read_into(self, c)
    }

    fn read_rect(&mut self, c: &mut Canvas, area: Rect) {
        Framebuffer::read_rect(self, c, area)
    }

    fn overlay_set(&mut self, slot: u8, area: Rect, px: &[u16], mask: Option<&[u8]>) -> bool {
        Framebuffer::overlay_set(self, slot, area, px, mask)
    }

    fn overlay_clear(&mut self, slot: u8) {
        Framebuffer::overlay_clear(self, slot)
    }

    fn device(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    fn present(&mut self) -> bool {
        Framebuffer::present(self)
    }
}

/// Saves each frame that differs from the last as `frame-NNN.png`, 2x.
pub struct Frames {
    dir: PathBuf,
    count: usize,
    last: Canvas,
    /// As gh0st's panel driver: overlays laid over each saved frame and
    /// never read back. None: as M5's driver, which has none.
    slots: Option<[Option<Slot>; 2]>,
}

struct Slot {
    area: Rect,
    px: Vec<u16>,
    mask: Option<Vec<u8>>,
}

impl Frames {
    pub fn new(dir: PathBuf, w: i32, h: i32) -> std::io::Result<Frames> {
        std::fs::create_dir_all(&dir)?;
        Ok(Frames {
            dir,
            count: 0,
            last: Canvas::new(w, h),
            slots: None,
        })
    }

    /// Frames with overlays, as gh0st's panel driver composes them.
    pub fn composited(dir: PathBuf, w: i32, h: i32) -> std::io::Result<Frames> {
        let mut f = Frames::new(dir, w, h)?;
        f.slots = Some([None, None]);
        Ok(f)
    }

    /// What the glass shows: the app's pixels with the overlays on top.
    pub fn shown(&self) -> Canvas {
        let mut c = self.last.clone();
        for s in self.slots.iter().flatten().flatten() {
            let w = s.area.w as usize;
            for y in 0..s.area.h {
                for x in 0..s.area.w {
                    let i = y as usize * w + x as usize;
                    if s.mask.as_ref().is_none_or(|m| m[i] != 0) {
                        c.set(s.area.x + x, s.area.y + y, s.px[i]);
                    }
                }
            }
        }
        c
    }

    fn save(&mut self) {
        let path = self.dir.join(format!("frame-{:03}.png", self.count));
        self.count += 1;
        if let Err(e) = crate::snapshot::write_png(&self.shown(), &path, 2) {
            eprintln!("sim: {}: {e}", path.display());
        }
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
            self.save();
        }
        changed
    }

    fn flush_all(&mut self, c: &Canvas) {
        self.last = Canvas::new(self.last.width(), self.last.height());
        self.last.fill(self.last.bounds(), !c.get(0, 0));
        self.flush(c, c.bounds());
    }

    fn read_into(&mut self, c: &mut Canvas) {
        *c = self.last.clone();
    }

    fn overlay_set(&mut self, slot: u8, area: Rect, px: &[u16], mask: Option<&[u8]>) -> bool {
        let Some(slots) = &mut self.slots else {
            return false;
        };
        let n = (area.w.max(0) * area.h.max(0)) as usize;
        let inside = area.intersect(&self.last.bounds()) == area;
        if slot > 1 || n == 0 || !inside || px.len() != n || mask.is_some_and(|m| m.len() != n) {
            return false;
        }
        slots[slot as usize] = Some(Slot {
            area,
            px: px.to_vec(),
            mask: mask.map(<[u8]>::to_vec),
        });
        self.save();
        true
    }

    fn overlay_clear(&mut self, slot: u8) {
        if let Some(slots) = &mut self.slots {
            if slots
                .get_mut(slot as usize)
                .and_then(Option::take)
                .is_some()
            {
                self.save();
            }
        }
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
        let dir = std::env::temp_dir().join(format!("gh0st-frames-{}", std::process::id()));
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
