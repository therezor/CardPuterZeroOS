//! Short eased moves shared by home and the switcher. Each runs a fixed time
//! with a cubic ease-out, and the shell draws frames only while one runs.

use gh0st_ui::geom::Rect;

/// Frame spacing while something moves. The SPI panel shows fewer, but a
/// frame that is not ready in time is simply skipped by the kernel.
pub const FRAME_MS: u64 = 20;
/// The lock screen sliding up off the top at the unlock: about 12 frames
/// at the panel's 50 Hz.
pub const UNLOCK_MS: u64 = 240;

#[derive(Clone, Copy, Debug)]
pub struct Anim<M> {
    pub start: u64,
    pub ms: u64,
    pub motion: M,
}

impl<M> Anim<M> {
    /// Eased progress 0-1: fast start, soft landing.
    pub fn progress(&self, now: u64) -> f32 {
        let p = (now.saturating_sub(self.start) as f32 / self.ms as f32).min(1.0);
        1.0 - (1.0 - p).powi(3)
    }

    /// When the next frame is due.
    pub fn next_frame(&self, now: u64) -> u64 {
        (now + FRAME_MS).min(self.start + self.ms)
    }

    pub fn done(&self, now: u64) -> bool {
        now >= self.start + self.ms
    }
}

pub fn lerp(a: i32, b: i32, p: f32) -> i32 {
    a + ((b - a) as f32 * p).round() as i32
}

pub fn lerp_rect(a: Rect, b: Rect, p: f32) -> Rect {
    Rect::new(
        lerp(a.x, b.x, p),
        lerp(a.y, b.y, p),
        lerp(a.w, b.w, p),
        lerp(a.h, b.h, p),
    )
}
