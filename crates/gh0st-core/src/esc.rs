//! Holding ESC closes a running app, as in stock (`cp0_esc_exit_policy.hpp`):
//! a hint at 0.5 s, SIGTERM to the app's group at 3 s, SIGKILL 3 s later.
//! Pure timing, so the event loop arms one timer for `next_at` and nothing
//! wakes while ESC is up.

/// After an app's own 0.7 s hold to exit, so an app that answers closes
/// before the shell says anything.
pub const HINT_MS: u64 = 1500;
pub const TERMINATE_MS: u64 = 3000;
pub const KILL_GRACE_MS: u64 = 3000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub show_hint: bool,
    pub hide_hint: bool,
    pub terminate: bool,
    pub kill: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EscHold {
    pressed_at: Option<u64>,
    hint: bool,
    terminated_at: Option<u64>,
    killed: bool,
}

impl EscHold {
    pub fn update(&mut self, now: u64, esc_down: bool) -> Decision {
        let mut d = Decision::default();
        if let Some(t) = self.terminated_at {
            if !self.killed && now.saturating_sub(t) >= KILL_GRACE_MS {
                self.killed = true;
                d.kill = true;
            }
            return d;
        }
        if !esc_down {
            d.hide_hint = self.hint;
            self.hint = false;
            self.pressed_at = None;
            return d;
        }
        let since = *self.pressed_at.get_or_insert(now);
        let held = now.saturating_sub(since);
        if !self.hint && held >= HINT_MS {
            self.hint = true;
            d.show_hint = true;
        }
        if held >= TERMINATE_MS {
            self.terminated_at = Some(now);
            d.terminate = true;
        }
        d
    }

    /// When `update` next has something to do, if anything.
    pub fn next_at(&self) -> Option<u64> {
        if let Some(t) = self.terminated_at {
            return (!self.killed).then_some(t + KILL_GRACE_MS);
        }
        let p = self.pressed_at?;
        Some(if self.hint {
            p + TERMINATE_MS
        } else {
            p + HINT_MS
        })
    }

    pub fn hint_visible(&self) -> bool {
        self.hint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_shows_the_hint_then_terminates_then_kills() {
        let mut e = EscHold::default();
        assert_eq!(e.update(1000, true), Decision::default());
        assert_eq!(e.next_at(), Some(2500));
        assert!(e.update(2500, true).show_hint);
        assert_eq!(e.next_at(), Some(4000));
        assert!(e.update(4000, true).terminate);
        assert!(!e.update(6999, true).kill);
        assert!(
            e.update(7000, false).kill,
            "the grace runs even after release"
        );
        assert_eq!(e.next_at(), None);
    }

    #[test]
    fn letting_go_early_hides_the_hint_and_starts_over() {
        let mut e = EscHold::default();
        e.update(0, true);
        e.update(1600, true);
        assert!(e.update(1700, false).hide_hint);
        assert_eq!(e.next_at(), None);
        e.update(1800, true);
        assert!(
            !e.update(4500, true).terminate,
            "the clock restarted at 1800"
        );
    }
}
