//! Something the shell draws over a running app: the ESC hint, the battery
//! countdown, a toast. It remembers the pixels underneath and puts them back
//! when it goes, except where the app has drawn since. An app that redraws
//! all the time would paint over it, so `refresh` lays it on top again.

use gh0st_ui::Canvas;
use gh0st_ui::display::Display;
use gh0st_ui::geom::Rect;

/// How often an overlay checks that an app has not painted over it.
pub const REFRESH_MS: u64 = 100;

pub struct Overlay {
    area: Rect,
    under: Canvas,
    drawn: Canvas,
    /// Pixels the overlay itself set, as opposed to ones it left alone.
    painted: Vec<bool>,
}

impl Overlay {
    /// Reads the panel, lets `paint` draw over a copy, and sends the rows
    /// `paint` says it covered.
    pub fn show(
        d: &mut dyn Display,
        under: Canvas,
        paint: impl FnOnce(&mut Canvas) -> Rect,
    ) -> Overlay {
        let mut under = under;
        d.read_into(&mut under);
        let mut drawn = under.clone();
        let area = paint(&mut drawn).intersect(&drawn.bounds());
        let mut painted = Vec::with_capacity((area.w * area.h).max(0) as usize);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                painted.push(drawn.get(x, y) != under.get(x, y));
            }
        }
        d.flush(&drawn, area);
        Overlay {
            area,
            under,
            drawn,
            painted,
        }
    }

    /// Lays the overlay on top again where the app has drawn over it. The
    /// app's new pixels become the ones `hide` puts back.
    pub fn refresh(&mut self, d: &mut dyn Display) {
        let mut now = self.drawn.clone();
        d.read_into(&mut now);
        let mut lost = false;
        let mut i = 0;
        for y in self.area.y..self.area.bottom() {
            for x in self.area.x..self.area.right() {
                let p = now.get(x, y);
                if p != self.drawn.get(x, y) {
                    self.under.set(x, y, p);
                    if self.painted[i] {
                        now.set(x, y, self.drawn.get(x, y));
                        lost = true;
                    } else {
                        self.drawn.set(x, y, p);
                    }
                }
                i += 1;
            }
        }
        if lost {
            d.flush(&now, self.area);
        }
    }

    /// The panel as it was before, for a screenshot or a saved frame.
    pub fn under(&self) -> &Canvas {
        &self.under
    }

    /// Puts back what was underneath. A pixel the app changed since keeps
    /// the app's colour. Returns the canvas for reuse.
    pub fn hide(self, d: &mut dyn Display) -> Canvas {
        let Overlay {
            area, under, drawn, ..
        } = self;
        let mut now = drawn.clone();
        d.read_into(&mut now);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if now.get(x, y) == drawn.get(x, y) {
                    now.set(x, y, under.get(x, y));
                }
            }
        }
        d.flush(&now, area);
        under
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_ui::display::Frames;

    #[test]
    fn refresh_puts_the_overlay_back_and_keeps_the_new_app_pixels_for_later() {
        let dir = std::env::temp_dir().join(format!("gh0st-overlay-r-{}", std::process::id()));
        let mut d = Frames::new(dir.clone(), 8, 4).unwrap();
        let mut app = Canvas::new(8, 4);
        app.fill(app.bounds(), 0x1111);
        d.flush_all(&app);
        let mut o = Overlay::show(&mut d, Canvas::new(8, 4), |c| {
            c.fill(Rect::new(0, 0, 4, 1), 0xffff);
            Rect::new(0, 0, 8, 2)
        });
        // The app repaints everything in a new colour.
        let mut repaint = Canvas::new(8, 4);
        repaint.fill(repaint.bounds(), 0x3333);
        d.flush(&repaint, repaint.bounds());
        o.refresh(&mut d);
        let mut now = Canvas::new(8, 4);
        d.read_into(&mut now);
        assert_eq!(now.get(1, 0), 0xffff, "the overlay is back on top");
        assert_eq!(
            now.get(6, 1),
            0x3333,
            "what it never covered stays the app's"
        );
        o.hide(&mut d);
        d.read_into(&mut now);
        assert_eq!(now.get(1, 0), 0x3333, "hide restores the newest app pixels");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hiding_restores_the_app_except_what_it_drew_meanwhile() {
        let dir = std::env::temp_dir().join(format!("gh0st-overlay-{}", std::process::id()));
        let mut d = Frames::new(dir.clone(), 8, 4).unwrap();
        let mut app = Canvas::new(8, 4);
        app.fill(app.bounds(), 0x1111);
        d.flush_all(&app);
        let o = Overlay::show(&mut d, Canvas::new(8, 4), |c| {
            let r = Rect::new(0, 0, 8, 2);
            c.fill(r, 0xffff);
            r
        });
        // The app redraws one pixel under the overlay.
        let mut now = Canvas::new(8, 4);
        d.read_into(&mut now);
        now.set(3, 1, 0x2222);
        d.flush(&now, now.bounds());
        o.hide(&mut d);
        let mut after = Canvas::new(8, 4);
        d.read_into(&mut after);
        assert_eq!(after.get(0, 0), 0x1111, "put back");
        assert_eq!(after.get(3, 1), 0x2222, "the app's newer pixel stays");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
