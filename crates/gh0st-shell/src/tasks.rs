//! Open apps. Each is a process group: at most one in front, the rest
//! paused with SIGSTOP. A paused app keeps its last frame here, so the panel
//! shows it again before the app runs.

use crate::home::Icon;
use crate::switcher::{self, Card};
use gh0st_core::child;
use gh0st_core::esc::KILL_GRACE_MS;
use gh0st_core::link::Link;
use gh0st_core::usage::AppUsage;
use gh0st_ui::Canvas;

/// How often to look for a leftover group member once the leader has gone:
/// such a process may not be our child, so its exit sends no SIGCHLD.
pub const GROUP_RECHECK_MS: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Front,
    Paused,
    /// Left running, as Settings > Apps allows.
    Background,
    /// Sent SIGTERM at this time; SIGKILL follows after the grace.
    Closing {
        since: u64,
        killed: bool,
    },
}

pub struct Task {
    /// The app's `.desktop` file name.
    pub id: String,
    pub name: String,
    pub icon: Icon,
    pub pgid: i32,
    pub state: State,
    pub leader_done: bool,
    /// What the panel showed when it was paused. About 106 KB.
    pub pixels: Option<Canvas>,
    /// When it was last in front, for the switcher's starting card.
    pub used: u64,
    /// Memory for the switcher, read once per pause: a stopped app's
    /// figure hardly moves. Re-read each sample while it runs behind.
    pub mem_kb: Option<u64>,
    /// The shell's end of the app's link.
    pub link: Option<Link>,
    /// The app answered `hello`: it speaks the link.
    pub linked: bool,
    /// `X-Gh0st-Background` in its `.desktop` file.
    pub background: bool,
    /// Running behind and told to `pause`: it draws nothing and reads no
    /// keys, so the shell needs neither its pixels nor a guard.
    pub quiet: bool,
    /// Sent `awake 1`: the screen stays on while it is in front.
    pub awake: bool,
    /// Holds the keyboard itself or can't be paused: the shell can't see
    /// its keys or stop its drawing, so the screen stays on until it
    /// leaves the front.
    pub keeps_on: bool,
}

impl Task {
    /// Can come to the front: paused or running behind.
    pub fn resumable(&self) -> bool {
        matches!(self.state, State::Paused | State::Background)
    }
}

#[derive(Default)]
pub struct Tasks {
    /// In launch order: the dock and the switcher show this order.
    list: Vec<Task>,
}

impl Tasks {
    pub fn add(&mut self, t: Task) {
        self.list.push(t);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Task> {
        self.list.iter_mut()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.list.iter_mut().find(|t| t.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn front(&self) -> Option<&Task> {
        self.list.iter().find(|t| t.state == State::Front)
    }

    pub fn front_mut(&mut self) -> Option<&mut Task> {
        self.list.iter_mut().find(|t| t.state == State::Front)
    }

    pub fn any_paused(&self) -> bool {
        self.list.iter().any(|t| t.state != State::Front)
    }

    /// The background app used last: the one a stray pixel most likely
    /// came from.
    /// The newest app running behind that may draw over the shell: one
    /// told to pause on the link draws nothing.
    pub fn background_mut(&mut self) -> Option<&mut Task> {
        self.list
            .iter_mut()
            .filter(|t| t.state == State::Background && !t.quiet)
            .max_by_key(|t| t.used)
    }

    /// The reaper saw `pid` exit.
    pub fn exited(&mut self, pid: i32) {
        if let Some(t) = self.list.iter_mut().find(|t| t.pgid == pid) {
            t.leader_done = true;
        }
    }

    /// Asks an app to exit.
    pub fn close(&mut self, id: &str, now: u64) {
        if let Some(t) = self.get_mut(id) {
            child::terminate(t.pgid);
            t.state = State::Closing {
                since: now,
                killed: false,
            };
            t.pixels = None;
        }
    }

    /// Asks every app to exit, for a shell that is stopping.
    pub fn close_all(&mut self, now: u64) {
        let ids: Vec<String> = self.list.iter().map(|t| t.id.clone()).collect();
        for id in ids {
            self.close(&id, now);
        }
    }

    /// Kills apps past their grace and drops the ones whose group is gone.
    /// Returns the dropped ones.
    pub fn sweep(&mut self, now: u64) -> Vec<Task> {
        for t in &mut self.list {
            // The leader of a paused app died; what is left is stopped and
            // would never exit on its own.
            if t.leader_done && t.state == State::Paused && child::group_alive(t.pgid) {
                child::terminate(t.pgid);
                t.state = State::Closing {
                    since: now,
                    killed: false,
                };
                t.pixels = None;
            }
            if let State::Closing { since, killed } = &mut t.state {
                if !*killed && now >= *since + KILL_GRACE_MS {
                    child::kill(t.pgid);
                    *killed = true;
                }
            }
        }
        let (gone, kept): (Vec<Task>, Vec<Task>) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|t| t.leader_done && !child::group_alive(t.pgid));
        self.list = kept;
        gone
    }

    /// When `sweep` next has work that no signal will announce.
    pub fn next_at(&self, now: u64) -> Option<u64> {
        let recheck = self
            .list
            .iter()
            .any(|t| t.leader_done)
            .then_some(now + GROUP_RECHECK_MS);
        let kill = self.list.iter().find_map(|t| match t.state {
            State::Closing {
                since,
                killed: false,
            } => Some(since + KILL_GRACE_MS),
            _ => None,
        });
        gh0st_core::rt::earliest([recheck, kill])
    }

    /// What the switcher shows per app: memory as last read, with CPU from
    /// `cpu_of` for an app running behind.
    pub fn usage(&self, cpu_of: impl Fn(&str) -> Option<f32>) -> Vec<(&str, AppUsage)> {
        self.list
            .iter()
            .filter_map(|t| {
                // Zero means no member could be read (run through sudo):
                // no figure beats a wrong one.
                let mem_kb = t.mem_kb.filter(|&m| m > 0)?;
                let cpu = cpu_of(&t.id);
                Some((t.id.as_str(), AppUsage { mem_kb, cpu }))
            })
            .collect()
    }

    /// The switcher's cards, in the same order.
    pub fn cards(&self) -> Vec<Card> {
        self.list
            .iter()
            .map(|t| Card {
                id: t.id.clone(),
                name: t.name.clone(),
                icon: t.icon.clone(),
                state: match t.state {
                    State::Closing { .. } => switcher::State::Closing,
                    State::Background => switcher::State::Running,
                    _ => switcher::State::Paused,
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, pgid: i32, used: u64) -> Task {
        Task {
            id: id.into(),
            name: id.into(),
            icon: Icon::Letter { tint: 0 },
            pgid,
            state: State::Paused,
            leader_done: false,
            pixels: None,
            used,
            mem_kb: None,
            link: None,
            linked: false,
            background: false,
            quiet: false,
            awake: false,
            keeps_on: false,
        }
    }

    #[test]
    fn a_task_goes_once_its_leader_is_reaped_and_its_group_is_empty() {
        let mut t = Tasks::default();
        // No process has these ids, so their groups count as gone.
        t.add(task("a", 999_991, 5));
        t.add(task("b", 999_992, 9));
        assert!(t.sweep(0).is_empty(), "leaders still running");
        t.exited(999_991);
        assert_eq!(t.next_at(100), Some(100 + GROUP_RECHECK_MS));
        let gone = t.sweep(0);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].id, "a");
        assert_eq!(t.cards().len(), 1);
    }

    #[test]
    fn closing_kills_after_the_grace() {
        let mut t = Tasks::default();
        t.add(task("a", 999_993, 0));
        t.close("a", 1000);
        assert_eq!(t.next_at(1000), Some(1000 + KILL_GRACE_MS));
        assert_eq!(t.cards()[0].state, switcher::State::Closing);
        t.sweep(1000 + KILL_GRACE_MS);
        assert_eq!(
            t.get("a").unwrap().state,
            State::Closing {
                since: 1000,
                killed: true
            }
        );
        assert_eq!(t.next_at(5000), None);
    }
}
