//! Something the shell draws over a running app: the ESC hint, the battery
//! countdown, a toast. It remembers the pixels underneath and puts them back
//! when it goes, except where the app has drawn since.

use gh0st_ui::Canvas;
use gh0st_ui::display::Display;
use gh0st_ui::geom::Rect;

pub struct Overlay {
    area: Rect,
    under: Canvas,
    drawn: Canvas,
}

impl Overlay {
    /// Reads the panel, lets `paint` draw over a copy, and sends the rows
    /// `paint` says it covered.
    pub fn show(d: &mut dyn Display, under: Canvas, paint: impl FnOnce(&mut Canvas) -> Rect) -> Overlay {
        let mut under = under;
        d.read_into(&mut under);
        let mut drawn = under.clone();
        let area = paint(&mut drawn).intersect(&drawn.bounds());
        d.flush(&drawn, area);
        Overlay { area, under, drawn }
    }

    /// The panel as it was before, for a screenshot or a saved frame.
    pub fn under(&self) -> &Canvas {
        &self.under
    }

    /// Puts back what was underneath. A pixel the app changed since keeps
    /// the app's colour. Returns the canvas for reuse.
    pub fn hide(self, d: &mut dyn Display) -> Canvas {
        let Overlay {
            area,
            under,
            drawn,
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
