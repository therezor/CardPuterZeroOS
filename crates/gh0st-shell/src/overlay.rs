//! Something the shell draws over a running app: the ESC hint, the battery
//! countdown, a toast. gh0st's panel driver lays it over every send, so the
//! app can draw underneath and never cover it. On M5's driver the shell
//! draws it into the app's framebuffer: it remembers the pixels underneath
//! and puts them back when it goes, except where the app has drawn since,
//! and `refresh` lays it on top again after an app paints over it.

use gh0st_ui::Canvas;
use gh0st_ui::display::Display;
use gh0st_ui::geom::Rect;

/// How often an overlay the driver can't hold checks that an app has not
/// painted over it.
pub const REFRESH_MS: u64 = 100;

/// The driver's slots: one banner, one popup, so a top banner and a bottom
/// toast never merge into most of the screen.
pub const BANNER: u8 = 0;
pub const POPUP: u8 = 1;

pub struct Overlay {
    slot: u8,
    area: Rect,
    /// The driver holds it: nothing below is drawn into the app's pixels.
    held: bool,
    /// The app's pixels under `area`, as last seen.
    under: Canvas,
    /// What the panel shows: `under` with the overlay painted on.
    drawn: Canvas,
    /// Pixels the overlay itself set, as opposed to ones it left alone.
    painted: Vec<bool>,
    /// Read into on each refresh, so a refresh allocates nothing.
    scratch: Canvas,
}

impl Overlay {
    /// Reads the panel and paints over a copy. The driver takes it if it
    /// can; else the rows `paint` says it covered are sent.
    pub fn show(
        d: &mut dyn Display,
        slot: u8,
        under: Canvas,
        paint: impl Fn(&mut Canvas) -> Rect,
    ) -> Overlay {
        let mut under = under;
        d.read_into(&mut under);
        let mut o = Overlay {
            slot,
            area: Rect::new(0, 0, 0, 0),
            held: false,
            scratch: under.clone(),
            drawn: under.clone(),
            under,
            painted: Vec::new(),
        };
        o.paint(d, &paint, None);
        o
    }

    /// Paints anew in place: one send, no frame of the bare app between.
    pub fn update(&mut self, d: &mut dyn Display, paint: impl Fn(&mut Canvas) -> Rect) {
        let old = self.area;
        if self.held {
            // The panel's buffer never holds a held overlay: all of it is
            // the app's.
            d.read_into(&mut self.under);
        } else {
            self.sync(d, self.under.bounds());
        }
        self.paint(d, &paint, Some(old));
    }

    /// Lays the overlay on top again where the app has drawn over it. The
    /// app's new pixels become the ones `hide` puts back. Nothing to do
    /// when the driver holds it.
    pub fn refresh(&mut self, d: &mut dyn Display) {
        if self.held {
            return;
        }
        let area = self.area;
        d.read_rect(&mut self.scratch, area);
        let mut lost = false;
        let mut i = 0;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let p = self.scratch.get(x, y);
                if p != self.drawn.get(x, y) {
                    self.under.set(x, y, p);
                    if self.painted[i] {
                        lost = true;
                    } else {
                        self.drawn.set(x, y, p);
                    }
                }
                i += 1;
            }
        }
        if lost {
            d.flush(&self.drawn, area);
        }
    }

    /// Whether `refresh` has work: only when the shell draws it itself.
    pub fn needs_refresh(&self) -> bool {
        !self.held
    }

    /// Takes the overlay out of a picture read from the panel, for a
    /// screenshot. A held overlay was never in it.
    pub fn patch_under(&self, shot: &mut Canvas) {
        if self.held {
            return;
        }
        let mut i = 0;
        for y in self.area.y..self.area.bottom() {
            for x in self.area.x..self.area.right() {
                if self.painted[i] && shot.get(x, y) == self.drawn.get(x, y) {
                    shot.set(x, y, self.under.get(x, y));
                }
                i += 1;
            }
        }
    }

    /// Puts back what was underneath. A pixel the app changed since keeps
    /// the app's colour. Returns a canvas for reuse.
    pub fn hide(self, d: &mut dyn Display) -> Canvas {
        let Overlay {
            slot,
            area,
            held,
            mut under,
            drawn,
            mut scratch,
            ..
        } = self;
        if held {
            d.overlay_clear(slot);
            return under;
        }
        d.read_rect(&mut scratch, area);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if scratch.get(x, y) != drawn.get(x, y) {
                    under.set(x, y, scratch.get(x, y));
                }
            }
        }
        d.flush(&under, area);
        under
    }

    /// Brings `under` up to date inside `area`: wherever the panel no
    /// longer shows the overlay's own pixel, it shows the app's.
    fn sync(&mut self, d: &mut dyn Display, area: Rect) {
        d.read_rect(&mut self.scratch, area);
        let area = area.intersect(&self.under.bounds());
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let p = self.scratch.get(x, y);
                let ours = self.area.contains(x, y)
                    && self.painted[self.index(x, y)]
                    && p == self.drawn.get(x, y);
                if !ours {
                    self.under.set(x, y, p);
                }
            }
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        ((y - self.area.y) * self.area.w + (x - self.area.x)) as usize
    }

    /// Paints over `under`, works out which pixels are the overlay's, and
    /// hands it to the driver or sends it. `old` is the area it covered
    /// before, whose rows go back to the app's in the same send.
    fn paint(
        &mut self,
        d: &mut dyn Display,
        paint: &impl Fn(&mut Canvas) -> Rect,
        old: Option<Rect>,
    ) {
        self.drawn.clone_from(&self.under);
        let area = paint(&mut self.drawn).intersect(&self.drawn.bounds());
        // Painted again over the inverse: a pixel that differs from neither
        // background is still the overlay's if it matches the app's colour.
        self.scratch.clone_from(&self.under);
        invert(&mut self.scratch, area);
        let inverse = paint(&mut self.scratch).intersect(&self.scratch.bounds());
        let area = area.union(&inverse);
        self.painted.clear();
        let mut px = Vec::with_capacity((area.w.max(0) * area.h.max(0)) as usize);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let u = self.under.get(x, y);
                self.painted
                    .push(self.drawn.get(x, y) != u || self.scratch.get(x, y) != !u);
                px.push(self.drawn.get(x, y));
            }
        }
        self.area = area;
        let mask: Option<Vec<u8>> = (!self.painted.iter().all(|&p| p))
            .then(|| self.painted.iter().map(|&p| p as u8).collect());
        if d.overlay_set(self.slot, area, &px, mask.as_deref()) {
            self.held = true;
            return;
        }
        if self.held {
            // The driver took it before and refuses now: take it back.
            d.overlay_clear(self.slot);
            self.held = false;
        }
        let rows = old.map_or(area, |o| o.union(&area));
        d.flush(&self.drawn, rows);
    }
}

fn invert(c: &mut Canvas, area: Rect) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let p = c.get(x, y);
            c.set(x, y, !p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_ui::display::Frames;

    fn frames(name: &str, composited: bool) -> (Frames, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("gh0st-overlay-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = if composited {
            Frames::composited(dir.clone(), 8, 4)
        } else {
            Frames::new(dir.clone(), 8, 4)
        };
        (d.unwrap(), dir)
    }

    fn app(d: &mut Frames, colour: u16) {
        let mut c = Canvas::new(8, 4);
        c.fill(c.bounds(), colour);
        d.flush(&c, c.bounds());
    }

    fn read(d: &mut Frames) -> Canvas {
        let mut c = Canvas::new(8, 4);
        d.read_into(&mut c);
        c
    }

    fn bar(colour: u16) -> impl Fn(&mut Canvas) -> Rect {
        move |c: &mut Canvas| {
            let r = Rect::new(0, 0, 4, 1);
            c.fill(r, colour);
            r
        }
    }

    fn pngs(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |d| d.count())
    }

    #[test]
    fn refresh_puts_the_overlay_back_and_keeps_the_new_app_pixels_for_later() {
        let (mut d, dir) = frames("r", false);
        app(&mut d, 0x1111);
        let mut o = Overlay::show(&mut d, BANNER, Canvas::new(8, 4), |c| {
            c.fill(Rect::new(0, 0, 4, 1), 0xffff);
            Rect::new(0, 0, 8, 2)
        });
        assert!(o.needs_refresh(), "Frames without slots is M5's driver");
        // The app repaints everything in a new colour.
        app(&mut d, 0x3333);
        o.refresh(&mut d);
        let now = read(&mut d);
        assert_eq!(now.get(1, 0), 0xffff, "the overlay is back on top");
        assert_eq!(
            now.get(6, 1),
            0x3333,
            "what it never covered stays the app's"
        );
        assert_eq!(now.get(6, 3), 0x3333, "rows below it are not touched");
        o.hide(&mut d);
        assert_eq!(
            read(&mut d).get(1, 0),
            0x3333,
            "hide restores the newest app pixels"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hiding_restores_the_app_except_what_it_drew_meanwhile() {
        let (mut d, dir) = frames("h", false);
        app(&mut d, 0x1111);
        let o = Overlay::show(&mut d, BANNER, Canvas::new(8, 4), |c| {
            let r = Rect::new(0, 0, 8, 2);
            c.fill(r, 0xffff);
            r
        });
        // The app redraws one pixel under the overlay.
        let mut now = read(&mut d);
        now.set(3, 1, 0x2222);
        d.flush(&now, now.bounds());
        o.hide(&mut d);
        let after = read(&mut d);
        assert_eq!(after.get(0, 0), 0x1111, "put back");
        assert_eq!(after.get(3, 1), 0x2222, "the app's newer pixel stays");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_update_is_one_send_with_no_bare_app_between() {
        let (mut d, dir) = frames("u", false);
        app(&mut d, 0x1111);
        let mut o = Overlay::show(&mut d, POPUP, Canvas::new(8, 4), bar(0xffff));
        let before = pngs(&dir);
        o.update(&mut d, |c| {
            let r = Rect::new(0, 0, 2, 1);
            c.fill(r, 0xaaaa);
            r
        });
        assert_eq!(pngs(&dir), before + 1, "one frame, not hide then show");
        let now = read(&mut d);
        assert_eq!(now.get(1, 0), 0xaaaa);
        assert_eq!(
            now.get(3, 0),
            0x1111,
            "what the old one covered is the app's again"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_overlay_pixel_the_colour_of_the_app_is_still_the_overlays() {
        let (mut d, dir) = frames("same", false);
        app(&mut d, 0xffff);
        let mut o = Overlay::show(&mut d, BANNER, Canvas::new(8, 4), bar(0xffff));
        app(&mut d, 0x3333);
        o.refresh(&mut d);
        assert_eq!(read(&mut d).get(2, 0), 0xffff, "laid back on top");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_driver_holds_it_and_the_app_draws_underneath() {
        let (mut d, dir) = frames("held", true);
        app(&mut d, 0x1111);
        let mut o = Overlay::show(&mut d, BANNER, Canvas::new(8, 4), bar(0xffff));
        assert!(!o.needs_refresh(), "no timer while the driver holds it");
        assert_eq!(
            read(&mut d).get(1, 0),
            0x1111,
            "the app's buffer is untouched"
        );
        app(&mut d, 0x3333);
        o.refresh(&mut d);
        assert_eq!(
            d.shown().get(1, 0),
            0xffff,
            "on the glass over the new frame"
        );
        assert_eq!(d.shown().get(6, 0), 0x3333);
        let mut shot = read(&mut d);
        o.patch_under(&mut shot);
        assert_eq!(shot.get(1, 0), 0x3333, "a screenshot has no overlay");
        o.update(&mut d, bar(0xaaaa));
        assert_eq!(d.shown().get(1, 0), 0xaaaa);
        o.hide(&mut d);
        assert_eq!(d.shown().get(1, 0), 0x3333, "the app's newest pixels");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_banner_and_a_popup_are_up_together() {
        let (mut d, dir) = frames("two", true);
        app(&mut d, 0x1111);
        let b = Overlay::show(&mut d, BANNER, Canvas::new(8, 4), bar(0xffff));
        let p = Overlay::show(&mut d, POPUP, Canvas::new(8, 4), |c| {
            let r = Rect::new(0, 3, 8, 1);
            c.fill(r, 0xaaaa);
            r
        });
        let shown = d.shown();
        assert_eq!((shown.get(0, 0), shown.get(0, 3)), (0xffff, 0xaaaa));
        b.hide(&mut d);
        assert_eq!(d.shown().get(0, 0), 0x1111);
        assert_eq!(d.shown().get(0, 3), 0xaaaa, "the popup stays");
        p.hide(&mut d);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
