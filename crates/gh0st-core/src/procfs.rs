//! Small readers for `/proc`: process stat lines, memory and CPU times.
//! Each parser takes text, so the tests run on any host. Without `/proc`
//! (a host run) the readers return nothing.

use std::fs;
use std::io::Read;

/// One `/proc/PID/stat` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcStat {
    pub pid: i32,
    pub state: char,
    pub ppid: i32,
    pub pgrp: i32,
    /// User plus system time, in clock ticks.
    pub ticks: u64,
}

/// Parses a stat line. The name field can hold spaces and brackets, so
/// parsing starts after its last `)`.
pub fn parse_stat(stat: &str) -> Option<ProcStat> {
    let close = stat.rfind(')')?;
    let pid = stat.split_whitespace().next()?.parse().ok()?;
    let mut f = stat[close + 1..].split_whitespace();
    let state = f.next()?.chars().next()?;
    let ppid = f.next()?.parse().ok()?;
    let pgrp = f.next()?.parse().ok()?;
    // session, tty_nr, tpgid, flags, minflt, cminflt, majflt, cmajflt.
    let mut f = f.skip(8);
    let utime: u64 = f.next()?.parse().ok()?;
    let stime: u64 = f.next()?.parse().ok()?;
    Some(ProcStat {
        pid,
        state,
        ppid,
        pgrp,
        ticks: utime + stime,
    })
}

/// Whether process `pid` is in group `pgid` or descends from a process
/// in it: an app, or a program it started (the terminal's shell, a
/// script). Walks up at most 32 parents.
pub fn belongs_to(pid: i32, pgid: i32) -> bool {
    let mut at = pid;
    for _ in 0..32 {
        let Some(st) = fs::read_to_string(format!("/proc/{at}/stat"))
            .ok()
            .and_then(|s| parse_stat(&s))
        else {
            return false;
        };
        if st.pgrp == pgid || st.pid == pgid {
            return true;
        }
        if st.ppid <= 1 {
            return false;
        }
        at = st.ppid;
    }
    false
}

/// Every process, read one stat file at a time. Processes that exit
/// mid-scan are skipped.
pub fn processes() -> impl Iterator<Item = ProcStat> {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let pid = name
                .to_str()
                .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))?;
            parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
        })
}

/// Proportional memory in kB, swapped pages included: Linux here swaps to
/// zram, so a paused app can sit mostly in swap. `None` for a process we
/// may not read, such as one run through sudo.
pub fn pss_kb(pid: i32) -> Option<u64> {
    parse_rollup(&fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?)
}

pub fn parse_rollup(text: &str) -> Option<u64> {
    let (mut pss, mut swap): (Option<u64>, u64) = (None, 0);
    for line in text.lines() {
        let mut f = line.split_whitespace();
        match f.next() {
            Some("Pss:") => pss = f.next()?.parse().ok(),
            Some("SwapPss:") => swap = f.next()?.parse().ok()?,
            _ => {}
        }
    }
    Some(pss? + swap)
}

/// The start of a file, read into a stack buffer: the lines we need come
/// first, and `/proc/stat` runs to kilobytes of interrupt counts.
fn head(path: &str, f: impl FnOnce(&str) -> Option<(u64, u64)>) -> Option<(u64, u64)> {
    let mut buf = [0u8; 256];
    let n = fs::File::open(path).ok()?.read(&mut buf).ok()?;
    f(std::str::from_utf8(&buf[..n]).ok()?)
}

/// Total and available memory in kB.
pub fn meminfo() -> Option<(u64, u64)> {
    head("/proc/meminfo", parse_meminfo)
}

pub fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
    };
    Some((field("MemTotal:")?, field("MemAvailable:")?))
}

/// Ticks spent by every core since boot, all and idle.
pub fn cpu_times() -> Option<(u64, u64)> {
    head("/proc/stat", parse_cpu)
}

pub fn parse_cpu(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().next()?.strip_prefix("cpu ")?;
    // user nice system idle iowait irq softirq steal. Guest time is already
    // in user and nice.
    let (mut all, mut idle, mut n) = (0u64, 0u64, 0);
    for (i, v) in line.split_whitespace().take(8).enumerate() {
        let v: u64 = v.parse().ok()?;
        all += v;
        if i == 3 || i == 4 {
            idle += v;
        }
        n += 1;
    }
    if n < 4 {
        return None;
    }
    Some((all, idle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_lines_read_past_odd_names() {
        let line = "123 (we (ird) x) T 1 120 120 0 -1 4194560 10 0 0 0 40 2 0 0 20 0 1 0";
        assert_eq!(
            parse_stat(line),
            Some(ProcStat {
                pid: 123,
                state: 'T',
                ppid: 1,
                pgrp: 120,
                ticks: 42,
            })
        );
        assert_eq!(parse_stat("garbage"), None);
        assert_eq!(parse_stat("123 (x) S 1 120"), None);
    }

    #[test]
    fn rollup_adds_swapped_pages_and_ignores_the_pss_breakdown() {
        let text = "\
55c0-7ffd ---p 00000000 00:00 0 [rollup]
Rss:                4120 kB
Pss:                2048 kB
Pss_Anon:           1024 kB
Pss_File:           1000 kB
SwapPss:             512 kB
";
        assert_eq!(parse_rollup(text), Some(2560));
        assert_eq!(parse_rollup("Rss: 4 kB\n"), None);
    }

    #[test]
    fn meminfo_gives_total_and_available() {
        let text = "MemTotal:  360000 kB\nMemFree: 20000 kB\nMemAvailable: 140000 kB\n";
        assert_eq!(parse_meminfo(text), Some((360000, 140000)));
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn cpu_line_sums_all_and_idle_with_iowait() {
        let text = "cpu  100 5 50 800 20 3 2 0 9 0\ncpu0 1 2 3 4\n";
        assert_eq!(parse_cpu(text), Some((980, 820)));
        assert_eq!(parse_cpu("intr 1 2\n"), None);
    }
}
