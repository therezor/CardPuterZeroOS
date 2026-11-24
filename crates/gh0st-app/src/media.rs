//! Pictures and video a helper draws, on screen: the camera's Gallery and
//! the Media app share these.
//!
//! - [`Picture`]: RGB565 read out of a [`Ring`], drawn centred.
//! - [`PictureView`]: fit or fill, zoom steps up to the picture's own
//!   pixels, panning a third at a time, and a map of where the view is.
//! - [`SeekBar`]: a video's time and progress over its picture, or a
//!   track's on a page.
//! - [`AskQueue`]: pictures asked of a helper, one in flight, so the newest
//!   view is never stuck behind a screenful of tiles.
//! - [`plate_text`]: words over a picture, on a dark plate.

use crate::frames::Ring;
use gh0st_ui::theme::{ON_PICTURE, PICTURE_PLATE};
use gh0st_ui::{Canvas, Color, Rect, Theme, widgets};
use std::collections::VecDeque;

/// A picture a helper drew: RGB565 rows, packed.
#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub px: Vec<u16>,
    pub w: u16,
    pub h: u16,
}

impl Picture {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The frame announced as `seq` in `slot`, or an empty picture when the
    /// helper has already moved on.
    pub fn read(ring: &Ring, slot: u32, seq: u32) -> Picture {
        let mut p = Picture::default();
        p.read_into(ring, slot, seq);
        p
    }

    /// As [`Picture::read`], into this one's memory. False, and unchanged
    /// sizes, when the frame is stale.
    pub fn read_into(&mut self, ring: &Ring, slot: u32, seq: u32) -> bool {
        match ring.read(slot, seq, &mut self.px) {
            Some((w, h)) => {
                (self.w, self.h) = (w, h);
                true
            }
            None => false,
        }
    }

    /// Centred in `r`, cut to it.
    pub fn draw(&self, c: &mut Canvas, r: Rect) {
        if self.is_empty() {
            return;
        }
        let x = r.x + (r.w - self.w as i32) / 2;
        let y = r.y + (r.h - self.h as i32) / 2;
        let old = c.set_clip(c.clip().intersect(&r));
        c.blit565(
            x,
            y,
            self.w as i32,
            self.h as i32,
            self.w as usize,
            &self.px,
        );
        c.set_clip(old);
    }
}

/// A dark plate under marks drawn over a picture.
pub fn plate(c: &mut Canvas, r: Rect) {
    c.fill_alpha(r, PICTURE_PLATE, 150);
}

/// Words on a plate over the picture, starting at `x`. Returns the plate.
pub fn plate_text(c: &mut Canvas, t: &Theme, x: i32, y: i32, s: &str, ink: Color) -> Rect {
    let f = t.f.small_bold;
    let p = t.m.plate_pad;
    let r = Rect::new(x, y, f.measure(s) + 2 * p, t.m.pill_h + 4);
    plate(c, r);
    widgets::text_in(c, f, Rect::new(r.x + p, r.y, r.w - 2 * p, r.h), s, ink);
    r
}

/// As [`plate_text`], ending at `right`.
pub fn plate_text_right(
    c: &mut Canvas,
    t: &Theme,
    right: i32,
    y: i32,
    s: &str,
    ink: Color,
) -> Rect {
    let w = t.f.small_bold.measure(s) + 2 * t.m.plate_pad;
    plate_text(c, t, right - w, y, s, ink)
}

/// A time as a clock shows it: 01:05, or 1:02:03 past an hour.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Where a video or a track is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeekBar {
    pub pos_ms: u64,
    pub dur_ms: u64,
    /// A stretch played over and over, A to B.
    pub repeat: Option<(u64, u64)>,
}

impl SeekBar {
    pub fn label(&self) -> String {
        if self.dur_ms > 0 {
            format!("{} / {}", clock(self.pos_ms), clock(self.dur_ms))
        } else {
            clock(self.pos_ms)
        }
    }

    fn frac(&self, ms: u64) -> f32 {
        if self.dur_ms == 0 {
            0.0
        } else {
            ms.min(self.dur_ms) as f32 / self.dur_ms as f32
        }
    }

    /// The line itself in `r`: done in the accent, the rest on `rest`, an
    /// A-B stretch marked above it.
    fn line(&self, c: &mut Canvas, t: &Theme, r: Rect, rest: Color, alpha: u8) {
        c.fill_alpha(r, rest, alpha);
        if self.dur_ms == 0 {
            return;
        }
        let done = (r.w as f32 * self.frac(self.pos_ms)) as i32;
        c.fill(Rect::new(r.x, r.y, done, r.h), t.c.accent);
        if let Some((a, b)) = self.repeat {
            let xa = r.x + (r.w as f32 * self.frac(a)) as i32;
            let xb = r.x + (r.w as f32 * self.frac(b)) as i32;
            c.fill(Rect::new(xa, r.y - 2, (xb - xa).max(1), 1), t.c.accent);
        }
    }

    /// Over a full-screen picture: the time top right, the line just above
    /// the buttons.
    pub fn draw_over(&self, c: &mut Canvas, t: &Theme) {
        let m = t.m.margin;
        plate_text_right(c, t, t.m.width - m, t.m.margin_v, &self.label(), ON_PICTURE);
        if self.dur_ms == 0 {
            return;
        }
        let foot = t.m.height - t.m.hint_h;
        let track = Rect::new(m, foot - t.m.track_lift, t.m.width - 2 * m, t.m.track_h);
        self.line(c, t, track, PICTURE_PLATE, 150);
    }

    /// On a page, across `r`: the line, then the time under its left end
    /// and the length under its right.
    pub fn draw_in(&self, c: &mut Canvas, t: &Theme, r: Rect) {
        let track = Rect::new(r.x, r.y, r.w, t.m.track_h);
        self.line(c, t, track, t.c.muted, 90);
        let words = Rect::new(r.x, r.y + t.m.track_h + 2, r.w, t.m.line_h);
        let soft = widgets::soft(t);
        widgets::text_in(c, t.f.small, words, &clock(self.pos_ms), soft);
        if self.dur_ms > 0 {
            widgets::text_right(c, t.f.small, r.right(), words, &clock(self.dur_ms), soft);
        }
    }
}

/// Zoom steps past the base view. The last step is always one picture
/// pixel per screen pixel.
const ZOOMS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

/// A picture larger than the screen, seen through it: fitted whole or
/// filling the screen, then closer in steps, moved a third of the view at a
/// time. The helper is asked for exactly what the screen shows
/// ([`PictureView::zoom`] and [`PictureView::centre`]), so nothing larger
/// than the screen ever comes over.
#[derive(Clone, Debug)]
pub struct PictureView {
    screen: (i32, i32),
    size: Option<(u32, u32)>,
    step: usize,
    centre: (f32, f32),
    fill: bool,
}

impl PictureView {
    /// A view `w` x `h` pixels big.
    pub fn new(w: i32, h: i32) -> PictureView {
        PictureView {
            screen: (w, h),
            size: None,
            step: 0,
            centre: (0.5, 0.5),
            fill: false,
        }
    }

    /// A new picture, `size` its own pixels as shown (after its rotation),
    /// or None while unknown: back to the whole picture, centred.
    pub fn show(&mut self, size: Option<(u32, u32)>) {
        self.size = size;
        self.step = 0;
        self.centre = (0.5, 0.5);
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    /// Fill: the screen covered, edges cut off. Off: the whole picture.
    pub fn set_fill(&mut self, on: bool) {
        self.fill = on;
        self.step = 0;
        self.clamp();
    }

    pub fn fill(&self) -> bool {
        self.fill
    }

    fn fit_scale(&self) -> Option<f32> {
        let (w, h) = self.size?;
        let (sw, sh) = (self.screen.0 as f32, self.screen.1 as f32);
        Some((sw / w as f32).min(sh / h as f32))
    }

    /// The base view's scale: fitted, or filling.
    fn base_scale(&self) -> Option<f32> {
        let (w, h) = self.size?;
        let (sw, sh) = (self.screen.0 as f32, self.screen.1 as f32);
        Some(if self.fill {
            (sw / w as f32).max(sh / h as f32)
        } else {
            (sw / w as f32).min(sh / h as f32)
        })
    }

    /// The zoom steps this picture has, from the base view.
    pub fn zooms(&self) -> Vec<f32> {
        let Some(base) = self.base_scale() else {
            return vec![1.0];
        };
        let most = 1.0 / base;
        let mut z: Vec<f32> = ZOOMS.iter().copied().filter(|&z| z < most * 0.9).collect();
        if z.is_empty() {
            z.push(1.0);
        }
        if most > 1.05 {
            z.push(most);
        }
        z
    }

    /// Steps in from the base view: 0 is the base.
    pub fn step(&self) -> usize {
        self.step
    }

    pub fn zoomed(&self) -> bool {
        self.step > 0
    }

    pub fn can_zoom_in(&self) -> bool {
        self.step + 1 < self.zooms().len()
    }

    pub fn can_zoom_out(&self) -> bool {
        self.step > 0
    }

    /// One step closer. False when it is already at the picture's pixels.
    pub fn zoom_in(&mut self) -> bool {
        if !self.can_zoom_in() {
            return false;
        }
        self.step += 1;
        self.clamp();
        true
    }

    pub fn zoom_out(&mut self) -> bool {
        if self.step == 0 {
            return false;
        }
        self.step -= 1;
        self.clamp();
        true
    }

    /// Back to the base view, centred.
    pub fn reset(&mut self) {
        self.step = 0;
        self.centre = (0.5, 0.5);
    }

    /// The zoom to ask the helper for: how many times closer than fitted.
    pub fn zoom(&self) -> f32 {
        let z = self.zooms();
        let rel = z[self.step.min(z.len() - 1)];
        match (self.base_scale(), self.fit_scale()) {
            (Some(b), Some(f)) if f > 0.0 => rel * b / f,
            _ => rel,
        }
    }

    /// Where the view looks, 0 to 1 across the picture each way.
    pub fn centre(&self) -> (f32, f32) {
        self.centre
    }

    /// How much of the picture the screen shows, 0 to 1 each way.
    pub fn view_frac(&self) -> (f32, f32) {
        let (Some((w, h)), Some(fit)) = (self.size, self.fit_scale()) else {
            return (1.0, 1.0);
        };
        let scale = (fit * self.zoom()).min(1.0);
        (
            (self.screen.0 as f32 / scale / w as f32).min(1.0),
            (self.screen.1 as f32 / scale / h as f32).min(1.0),
        )
    }

    /// True when some of the picture is off screen, so the arrows look
    /// around.
    pub fn moves(&self) -> bool {
        let (fw, fh) = self.view_frac();
        fw < 0.999 || fh < 0.999
    }

    fn clamp(&mut self) {
        let (fw, fh) = self.view_frac();
        self.centre.0 = self.centre.0.clamp(fw / 2.0, 1.0 - fw / 2.0);
        self.centre.1 = self.centre.1.clamp(fh / 2.0, 1.0 - fh / 2.0);
    }

    /// Looks a third of the view along: `dx` and `dy` are -1, 0 or 1.
    /// False when it is already at that edge.
    pub fn pan(&mut self, dx: i32, dy: i32) -> bool {
        let before = self.centre;
        let (fw, fh) = self.view_frac();
        self.centre.0 += dx as f32 * fw / 3.0;
        self.centre.1 += dy as f32 * fh / 3.0;
        self.clamp();
        self.centre != before
    }

    /// The zoom's word, once zoomed: "2×", or "1:1" at the picture's own
    /// pixels.
    pub fn label(&self) -> Option<String> {
        if self.step == 0 {
            return None;
        }
        let z = self.zooms();
        let at = z[self.step.min(z.len() - 1)];
        let most = 1.0 / self.base_scale()?;
        Some(if (at - most).abs() < 0.01 {
            "1:1".to_string()
        } else {
            format!("{}\u{d7}", at)
        })
    }

    /// The zoom's word and a map of where the view is, top right under `y`.
    /// Nothing when the whole picture shows.
    pub fn draw_marks(&self, c: &mut Canvas, t: &Theme, y: i32) {
        if !self.moves() {
            return;
        }
        let right = t.m.width - t.m.margin;
        let mut top = y;
        if let Some(label) = self.label() {
            top = plate_text_right(c, t, right, y, &label, ON_PICTURE).bottom() + t.m.plate_pad;
        }
        let Some((w, h)) = self.size else {
            return;
        };
        // The map keeps the picture's shape inside map_w x map_h.
        let k = (t.m.map_w as f32 / w as f32).min(t.m.map_h as f32 / h as f32);
        let mw = ((w as f32 * k) as i32).max(6);
        let mh = ((h as f32 * k) as i32).max(4);
        let map = Rect::new(right - mw, top, mw, mh);
        plate(c, map);
        let (fw, fh) = self.view_frac();
        let v = Rect::new(
            map.x + ((self.centre.0 - fw / 2.0) * map.w as f32) as i32,
            map.y + ((self.centre.1 - fh / 2.0) * map.h as f32) as i32,
            ((fw * map.w as f32) as i32).max(2),
            ((fh * map.h as f32) as i32).max(2),
        );
        c.fill_alpha(v, ON_PICTURE, 160);
    }
}

/// Pictures asked of a helper that answers in order, one at a time: the one
/// in flight is always the next answer. Pushing while one is out queues it;
/// [`AskQueue::answered`] then hands back the one that came and
/// [`AskQueue::take_next`] the one to send.
#[derive(Debug)]
pub struct AskQueue<A> {
    waiting: VecDeque<A>,
    asking: Option<A>,
}

impl<A> Default for AskQueue<A> {
    fn default() -> AskQueue<A> {
        AskQueue {
            waiting: VecDeque::new(),
            asking: None,
        }
    }
}

impl<A> AskQueue<A> {
    pub fn new() -> AskQueue<A> {
        AskQueue::default()
    }

    /// Queues an ask.
    pub fn push(&mut self, a: A) {
        self.waiting.push_back(a);
    }

    /// Queues an ask ahead of the rest: what the screen shows now.
    pub fn push_front(&mut self, a: A) {
        self.waiting.push_front(a);
    }

    /// Drops waiting asks `keep` says no to. The one in flight stays.
    pub fn retain(&mut self, keep: impl FnMut(&A) -> bool) {
        self.waiting.retain(keep);
    }

    /// True if an ask like this waits or is in flight.
    pub fn has(&self, mut like: impl FnMut(&A) -> bool) -> bool {
        self.asking.as_ref().is_some_and(&mut like) || self.waiting.iter().any(like)
    }

    pub fn asking(&self) -> Option<&A> {
        self.asking.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.asking.is_some()
    }

    /// The next ask to send, now in flight, or None while one is out.
    pub fn take_next(&mut self) -> Option<&A> {
        if self.asking.is_some() {
            return None;
        }
        self.asking = self.waiting.pop_front();
        self.asking.as_ref()
    }

    /// The ask an answer belongs to.
    pub fn answered(&mut self) -> Option<A> {
        self.asking.take()
    }

    /// Everything dropped: the helper is gone.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.asking = None;
    }

    pub fn len(&self) -> usize {
        self.waiting.len() + self.asking.is_some() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photo_zooms_to_its_own_pixels_and_no_further() {
        let mut v = PictureView::new(320, 170);
        v.show(Some((3280, 2464)));
        let z = v.zooms();
        assert_eq!(&z[..4], &[1.0, 2.0, 4.0, 8.0]);
        assert!((z[4] - 2464.0 / 170.0).abs() < 0.01);
        while v.zoom_in() {}
        assert_eq!(v.label().as_deref(), Some("1:1"));
        let (fw, fh) = v.view_frac();
        assert!((fw - 320.0 / 3280.0).abs() < 0.001 && (fh - 170.0 / 2464.0).abs() < 0.001);
    }

    #[test]
    fn a_small_picture_has_no_steps() {
        let mut v = PictureView::new(320, 170);
        v.show(Some((100, 80)));
        assert_eq!(v.zooms(), [1.0]);
        assert!(!v.zoom_in());
        assert!(!v.moves());
    }

    #[test]
    fn fill_cuts_a_panorama_to_the_screen_and_pans_along_it() {
        let mut v = PictureView::new(320, 170);
        v.show(Some((30000, 800)));
        assert!(!v.moves(), "fitted, the whole strip shows");
        v.set_fill(true);
        assert!(v.moves());
        // Filled, the strip is as tall as the screen: zoom is height over width.
        let fit = 320.0 / 30000.0;
        let fill = 170.0 / 800.0;
        assert!((v.zoom() - fill / fit).abs() < 0.01);
        assert_eq!(v.centre(), (0.5, 0.5));
        assert!(v.pan(-1, 0));
        assert!(v.centre().0 < 0.5);
        // At the left edge it stops.
        while v.pan(-1, 0) {}
        let (fw, _) = v.view_frac();
        assert!((v.centre().0 - fw / 2.0).abs() < 1e-4);
    }

    #[test]
    fn one_ask_flies_at_a_time() {
        let mut q = AskQueue::new();
        q.push(1);
        q.push(2);
        assert_eq!(q.take_next(), Some(&1));
        assert_eq!(q.take_next(), None);
        assert!(q.has(|&a| a == 2));
        q.retain(|&a| a != 2);
        assert!(!q.has(|&a| a == 2) && q.has(|&a| a == 1));
        assert_eq!(q.answered(), Some(1));
        assert_eq!(q.take_next(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn clocks_read_as_on_a_player() {
        assert_eq!(clock(65_000), "01:05");
        assert_eq!(clock(3_723_000), "1:02:03");
    }
}
