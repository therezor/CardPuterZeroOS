//! A key held to confirm, as the SDK's held chip: the chip fills, and at
//! the end the action runs. The switcher's All and the notifications'
//! All use it. It lasts only while the key is down.

/// How long the key is held.
pub const HOLD_MS: u64 = 700;
/// A frame of the fill.
pub const HOLD_FRAME_MS: u64 = 40;

/// What a tick did to the hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldTick {
    Idle,
    /// The fill grew: redraw.
    Fill,
    /// Held to the end by key `code`.
    Done(u16),
}

#[derive(Clone, Copy, Debug)]
struct Run {
    code: u16,
    start: u64,
    next: u64,
    pc: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HeldKey {
    run: Option<Run>,
}

impl HeldKey {
    /// Starts while key `code` is down.
    pub fn start(&mut self, code: u16, now: u64) {
        self.run = Some(Run {
            code,
            start: now,
            next: now + HOLD_FRAME_MS,
            pc: 0,
        });
    }

    pub fn holding(&self) -> bool {
        self.run.is_some()
    }

    pub fn cancel(&mut self) {
        self.run = None;
    }

    /// How full, 0 to 100, while held.
    pub fn fill(&self) -> Option<u8> {
        self.run.map(|r| r.pc)
    }

    /// Key `code` came up. True if that ended a hold before it was done.
    pub fn let_go(&mut self, code: u16) -> bool {
        let early = self.run.is_some_and(|r| r.code == code);
        if early {
            self.run = None;
        }
        early
    }

    /// When the hold next needs a tick: its next frame, or its end.
    pub fn at(&self) -> Option<u64> {
        self.run.map(|r| r.next.min(r.start + HOLD_MS))
    }

    pub fn tick(&mut self, now: u64) -> HoldTick {
        let Some(r) = self.run.as_mut() else {
            return HoldTick::Idle;
        };
        if now >= r.start + HOLD_MS {
            let code = r.code;
            self.run = None;
            return HoldTick::Done(code);
        }
        if now < r.next {
            return HoldTick::Idle;
        }
        r.pc = ((now - r.start) * 100 / HOLD_MS) as u8;
        r.next = now + HOLD_FRAME_MS;
        HoldTick::Fill
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR: u16 = 5;

    #[test]
    fn a_hold_fills_then_ends_and_no_deadline_outlives_it() {
        let mut h = HeldKey::default();
        h.start(FOUR, 1000);
        assert_eq!(h.at(), Some(1000 + HOLD_FRAME_MS));
        assert_eq!(h.tick(1020), HoldTick::Idle);
        assert_eq!(h.tick(1040), HoldTick::Fill);
        assert_eq!(h.fill(), Some(5));
        assert_eq!(h.tick(1000 + HOLD_MS), HoldTick::Done(FOUR));
        assert!(!h.holding());
        assert_eq!(h.at(), None);
        assert!(!h.let_go(FOUR));
    }

    #[test]
    fn only_its_own_key_lets_go() {
        let mut h = HeldKey::default();
        h.start(FOUR, 0);
        assert!(!h.let_go(FOUR + 1));
        assert!(h.let_go(FOUR));
        assert_eq!(h.at(), None);
    }
}
