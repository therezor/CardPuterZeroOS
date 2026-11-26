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

/// Tab held this long locks: on home and in the switcher at once, over an
/// app when Tab comes up.
pub const LOCK_HOLD_MS: u64 = 3000;
/// Over an app, when the banner says to keep holding.
pub const LOCK_HINT_MS: u64 = 1500;

/// Tab held over an app, timed from the raw keys: the app gets Tab too. Any
/// other key down spoils it until Tab comes up again.
#[derive(Clone, Copy, Debug, Default)]
pub struct TabHold {
    since: Option<u64>,
    hinted: bool,
    done: bool,
}

/// What a tick of the Tab hold asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabStep {
    /// Show "Hold Tab to lock".
    pub hint: bool,
    /// Held long enough: lock once every key is up.
    pub lock: bool,
}

impl TabHold {
    /// A raw key event: `value` 1 down, 0 up, 2 a repeat. `alone`: no
    /// other key is down. True if a hint showing should go.
    pub fn key(&mut self, code: u16, value: i32, now: u64, alone: bool) -> bool {
        use gh0st_core::keys::code::TAB;
        match (code == TAB, value) {
            (_, 2) => false,
            (true, 1) if alone => {
                *self = TabHold {
                    since: Some(now),
                    ..TabHold::default()
                };
                false
            }
            (true, 0) | (_, 1) => {
                // Once the lock is due it stays due: its banner says to let go.
                let hide = self.hinted && !self.done;
                *self = TabHold::default();
                hide
            }
            _ => false,
        }
    }

    pub fn update(&mut self, now: u64) -> TabStep {
        let Some(t) = self.since else {
            return TabStep::default();
        };
        if !self.done && now >= t + LOCK_HOLD_MS {
            self.done = true;
            return TabStep {
                lock: true,
                ..TabStep::default()
            };
        }
        if !self.hinted && now >= t + LOCK_HINT_MS {
            self.hinted = true;
            return TabStep {
                hint: true,
                ..TabStep::default()
            };
        }
        TabStep::default()
    }

    /// The next hint or lock, only while Tab is down.
    pub fn next_at(&self) -> Option<u64> {
        let t = self.since?;
        match (self.hinted, self.done) {
            (_, true) => None,
            (true, false) => Some(t + LOCK_HOLD_MS),
            (false, false) => Some(t + LOCK_HINT_MS),
        }
    }

    pub fn reset(&mut self) {
        *self = TabHold::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_core::keys::code::TAB;

    #[test]
    fn tab_held_hints_then_locks_once() {
        let mut h = TabHold::default();
        assert_eq!(h.next_at(), None);
        h.key(TAB, 1, 1000, true);
        assert_eq!(h.next_at(), Some(1000 + LOCK_HINT_MS));
        assert_eq!(h.update(2000), TabStep::default());
        assert!(h.update(1000 + LOCK_HINT_MS).hint);
        assert_eq!(h.next_at(), Some(1000 + LOCK_HOLD_MS));
        assert!(h.update(1000 + LOCK_HOLD_MS).lock);
        assert_eq!(h.update(9000), TabStep::default(), "once");
        assert_eq!(h.next_at(), None);
        assert!(
            !h.key(TAB, 0, 9000, true),
            "the let-go banner stays for the lock"
        );
    }

    #[test]
    fn another_key_spoils_the_hold_until_tab_comes_up() {
        let mut h = TabHold::default();
        h.key(TAB, 1, 0, true);
        h.update(LOCK_HINT_MS);
        assert!(h.key(16, 1, 1600, false), "the hint goes");
        assert_eq!(h.update(LOCK_HOLD_MS), TabStep::default());
        assert_eq!(h.next_at(), None);
        let mut chord = TabHold::default();
        chord.key(TAB, 1, 0, false);
        assert_eq!(
            chord.next_at(),
            None,
            "Tab with another key down never starts"
        );
    }

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
