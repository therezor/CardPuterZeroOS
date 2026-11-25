//! Memory and CPU for the switcher. Read only while it shows, and cheap: no
//! scan of `/proc`. Every app process descends from the shell, which is
//! their subreaper, so the walk starts at the shell's own children and
//! enters only the trees of the groups asked for.

use crate::procfs::{self, ProcStat};
use std::fs;

/// A baseline older than this is dropped: CPU over a long gap (the screen
/// was off, say) would not mean "now".
const STALE_MS: u64 = 5000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AppUsage {
    /// Proportional memory of every process, swap included.
    pub mem_kb: u64,
    /// Share of all cores since the last sample, 0 to 100. `None` on the
    /// first sample of a group.
    pub cpu: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemUsage {
    pub used_kb: u64,
    pub total_kb: u64,
    /// Share of all cores busy since the last sample.
    pub cpu: Option<f32>,
}

pub struct Sample {
    pub system: SystemUsage,
    /// One per group asked for, in the same order.
    pub apps: Vec<AppUsage>,
}

struct Base {
    at: u64,
    all: u64,
    idle: u64,
    /// Ticks per process group.
    groups: Vec<(i32, u64)>,
}

/// The system's memory alone: two short reads, cheap enough to do before
/// a frame. CPU needs two samples, so it is `None` here.
pub fn memory() -> Option<SystemUsage> {
    let (total_kb, avail_kb) = procfs::meminfo()?;
    Some(SystemUsage {
        used_kb: total_kb.saturating_sub(avail_kb),
        total_kb,
        cpu: None,
    })
}

#[derive(Default)]
pub struct Sampler {
    last: Option<Base>,
}

impl Sampler {
    /// Forgets the baseline, so the next CPU figures start fresh.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Reads the system and each process group in `groups`. A group's
    /// processes are its members plus their descendants: a terminal's inner
    /// shell runs in a group of its own. `None` without `/proc`.
    pub fn sample(&mut self, now: u64, groups: &[i32]) -> Option<Sample> {
        let (total_kb, avail_kb) = procfs::meminfo()?;
        let (all, idle) = procfs::cpu_times()?;
        let (ticks, mem) = if groups.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            walk(groups, std::process::id() as i32, children, stat, |p| {
                procfs::pss_kb(p).unwrap_or(0)
            })
        };
        let base = self
            .last
            .take()
            .filter(|b| now.saturating_sub(b.at) <= STALE_MS && all > b.all);
        let share = |busy: u64, b: &Base| (busy as f32 * 100.0 / (all - b.all) as f32).min(100.0);
        let apps = groups
            .iter()
            .enumerate()
            .map(|(i, g)| AppUsage {
                mem_kb: mem[i],
                cpu: base.as_ref().and_then(|b| {
                    let before = b.groups.iter().find(|(p, _)| p == g)?.1;
                    Some(share(ticks[i].saturating_sub(before), b))
                }),
            })
            .collect();
        let cpu = base.as_ref().map(|b| {
            let spent = all - b.all;
            share(spent.saturating_sub(idle.saturating_sub(b.idle)), b)
        });
        self.last = Some(Base {
            at: now,
            all,
            idle,
            groups: groups.iter().copied().zip(ticks).collect(),
        });
        Some(Sample {
            system: SystemUsage {
                used_kb: total_kb.saturating_sub(avail_kb),
                total_kb,
                cpu,
            },
            apps,
        })
    }
}

/// Ticks and memory per group, from the tree under `root`. Children of
/// `root` outside the groups (the shell's helpers) are not entered.
fn walk(
    groups: &[i32],
    root: i32,
    children: impl Fn(i32) -> Vec<i32>,
    stat: impl Fn(i32) -> Option<ProcStat>,
    pss: impl Fn(i32) -> u64,
) -> (Vec<u64>, Vec<u64>) {
    let mut ticks = vec![0; groups.len()];
    let mut mem = vec![0; groups.len()];
    let mut stack: Vec<(usize, ProcStat)> = children(root)
        .into_iter()
        .filter_map(&stat)
        .filter_map(|p| Some((groups.iter().position(|&g| g == p.pgrp)?, p)))
        .collect();
    while let Some((g, p)) = stack.pop() {
        ticks[g] += p.ticks;
        mem[g] += pss(p.pid);
        stack.extend(
            children(p.pid)
                .into_iter()
                .filter_map(&stat)
                .map(|c| (g, c)),
        );
    }
    (ticks, mem)
}

fn stat(pid: i32) -> Option<ProcStat> {
    procfs::parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Children of every thread of `pid`. Needs `CONFIG_PROC_CHILDREN`.
fn children(pid: i32) -> Vec<i32> {
    let mut out = Vec::new();
    for t in fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(s) = fs::read_to_string(t.path().join("children")) {
            out.extend(s.split_whitespace().filter_map(|p| p.parse::<i32>().ok()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32, pgrp: i32, ticks: u64) -> ProcStat {
        ProcStat {
            pid,
            state: 'S',
            ppid,
            pgrp,
            ticks,
        }
    }

    #[test]
    fn a_group_counts_its_members_and_descendants_in_other_groups() {
        // Shell 1. App 10 forks 11, which moved to a group of its own (a
        // terminal's inner shell) and forked 12. 20 is an orphan of app 10,
        // now the shell's child. 30 is another app, 40 a shell helper.
        let tree = [
            proc(10, 1, 10, 5),
            proc(11, 10, 11, 7),
            proc(12, 11, 11, 1),
            proc(20, 1, 10, 2),
            proc(30, 1, 30, 100),
            proc(40, 1, 40, 900),
        ];
        let children = |pid| {
            tree.iter()
                .filter(|p| p.ppid == pid)
                .map(|p| p.pid)
                .collect()
        };
        let stat = |pid| tree.iter().find(|p| p.pid == pid).copied();
        let (ticks, mem) = walk(&[10, 30], 1, children, stat, |pid| pid as u64);
        assert_eq!(ticks, [15, 100]);
        assert_eq!(mem, [10 + 11 + 12 + 20, 30]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_tree_is_walked_into_a_child_in_its_own_session() {
        // Another test's waitpid(-1) would take this child's exit.
        let _one = crate::child::CHILD_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new("/bin/sh")
            .args(["-c", "setsid sleep 2 & exec sleep 2"])
            .process_group(0)
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pgid = c.id() as i32;
        let me = std::process::id() as i32;
        let (_, count) = walk(&[pgid], me, children, stat, |_| 1);
        let mem = Sampler::default().sample(0, &[pgid]).unwrap().apps[0].mem_kb;
        let _ = c.kill();
        let _ = c.wait();
        assert_eq!(count, [2], "the leader and its setsid child");
        assert!(mem > 0);
    }

    #[test]
    fn cpu_shows_from_the_second_sample_on() {
        let mut s = Sampler::default();
        // Groups that do not exist read as empty; without /proc there is no sample.
        let Some(first) = s.sample(0, &[999_990]) else {
            return;
        };
        assert_eq!(first.apps[0].cpu, None);
        assert_eq!(first.system.cpu, None);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let second = s.sample(60, &[999_990]).unwrap();
        // Every core ticked since the first, unless the clock is very coarse.
        if second.system.cpu.is_some() {
            assert_eq!(
                second.apps[0],
                AppUsage {
                    mem_kb: 0,
                    cpu: Some(0.0)
                }
            );
        }
        assert!(second.system.used_kb <= second.system.total_kb);
        assert_eq!(s.sample(60 + STALE_MS + 1, &[]).unwrap().system.cpu, None);
    }
}
