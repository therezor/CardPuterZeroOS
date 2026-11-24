//! Find, a slice at a time: a search through a gigabyte runs between key
//! presses and stops at Esc.

use crate::buf::{Buffer, scan};
use std::io;

/// Bytes a step searches. On the device a megabyte took up to 20 ms; a
/// quarter of that keeps a slice near its 8 ms.
pub const STEP: u64 = 256 << 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// The next match starts here.
    Hit(u64),
    /// Searched the whole text: none.
    Miss,
    /// Not done: call again.
    More,
    /// The disk is behind: call again a little later.
    Wait,
}

pub struct Finder {
    needle: Vec<u8>,
    /// Letters match whatever their case.
    fold: bool,
    /// Where the search started; it wraps round to here.
    start: u64,
    at: u64,
    wrapped: bool,
    /// What a step searches. Between steps it holds the last bytes of the
    /// chunk before, so a match across chunks counts; it keeps its room.
    window: Vec<u8>,
}

impl Finder {
    /// Searches forward for `needle` from `from`, round to the top and back.
    pub fn new(needle: &[u8], from: u64, fold: bool) -> Finder {
        let needle = if fold {
            needle.to_ascii_lowercase()
        } else {
            needle.to_vec()
        };
        Finder {
            needle,
            fold,
            start: from,
            at: from,
            wrapped: false,
            window: Vec::new(),
        }
    }

    pub fn needle(&self) -> &[u8] {
        &self.needle
    }

    /// How far it has got: bytes searched.
    pub fn searched(&self, len: u64) -> u64 {
        if self.wrapped {
            len - self.start + self.at
        } else {
            self.at - self.start
        }
    }

    /// Searches about `budget` bytes.
    pub fn step(&mut self, b: &mut Buffer, budget: u64) -> io::Result<Found> {
        let n = self.needle.len();
        if n == 0 {
            return Ok(Found::Miss);
        }
        let len = b.len();
        let limit = if self.wrapped {
            self.start.min(len)
        } else {
            len
        };
        let end = limit.min(self.at.saturating_add(budget.max(1)));
        let mut window = std::mem::take(&mut self.window);
        let base = self.at - window.len() as u64;
        let kept = window.len();
        let r = b.nowait(|b| {
            b.for_each(self.at..end, |c| {
                window.extend_from_slice(c);
                true
            })
        });
        // The disk is behind: search what came, and wait for the rest.
        let waiting = matches!(&r, Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        if !waiting {
            r?;
        }
        let end = if waiting {
            self.at + (window.len() - kept) as u64
        } else {
            end
        };
        if let Some(i) = find(&window, &self.needle, self.fold) {
            self.window = window;
            return Ok(Found::Hit(base + i as u64));
        }
        self.at = end;
        let keep = window.len().min(n - 1);
        window.drain(..window.len() - keep);
        self.window = window;
        if waiting {
            return Ok(Found::Wait);
        }
        if end < limit {
            return Ok(Found::More);
        }
        if self.wrapped || self.start == 0 {
            return Ok(Found::Miss);
        }
        // Round to the top; a match may straddle where it began.
        self.wrapped = true;
        self.at = 0;
        self.window.clear();
        self.start = (self.start + n as u64 - 1).min(len);
        Ok(Found::More)
    }
}

/// The first `needle` in `hay`. `needle` is lower case when folding.
pub fn find(hay: &[u8], needle: &[u8], fold: bool) -> Option<usize> {
    let n = needle.len();
    if n == 0 || hay.len() < n {
        return None;
    }
    let first = needle[0];
    let both = fold && first.is_ascii_alphabetic();
    // Where each case of the first byte comes next. A letter common in one
    // case and missing in the other is looked for once, not at every
    // candidate: that made a folded search quadratic.
    let (mut lo, mut up) = (None, None);
    let mut i = 0;
    while i + n <= hay.len() {
        let mut p = next_at(first, hay, i, &mut lo);
        if both {
            p = p.min(next_at(first.to_ascii_uppercase(), hay, i, &mut up));
        }
        if p == usize::MAX || p + n > hay.len() {
            return None;
        }
        let w = &hay[p..p + n];
        if (fold && w.eq_ignore_ascii_case(needle)) || (!fold && w == needle) {
            return Some(p);
        }
        i = p + 1;
    }
    None
}

/// The next `c` at or after `i`, or usize::MAX for none, kept in `at`
/// until the search passes it.
fn next_at(c: u8, hay: &[u8], i: usize, at: &mut Option<usize>) -> usize {
    match *at {
        Some(p) if p >= i => p,
        _ => {
            let p = scan::memchr(c, &hay[i..]).map_or(usize::MAX, |p| i + p);
            *at = Some(p);
            p
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(b: &mut Buffer, needle: &str, from: u64, fold: bool, budget: u64) -> Option<u64> {
        let mut f = Finder::new(needle.as_bytes(), from, fold);
        loop {
            match f.step(b, budget).unwrap() {
                Found::Hit(at) => return Some(at),
                Found::Miss => return None,
                Found::More | Found::Wait => {}
            }
        }
    }

    #[test]
    fn finds_across_slices_and_wraps_round() {
        let text = format!("{}needle{}Needle", "x".repeat(1000), "y".repeat(1000));
        let mut b = Buffer::from_bytes(text.as_bytes());
        for budget in [1, 3, 7, 100, 1 << 20] {
            assert_eq!(
                run(&mut b, "needle", 0, false, budget),
                Some(1000),
                "{budget}"
            );
            assert_eq!(
                run(&mut b, "needle", 1001, false, budget),
                Some(1000),
                "{budget}"
            );
            assert_eq!(
                run(&mut b, "needle", 1001, true, budget),
                Some(2006),
                "{budget}"
            );
            assert_eq!(run(&mut b, "absent", 500, true, budget), None, "{budget}");
        }
        // A match straddling the start is still found on the way round.
        assert_eq!(run(&mut b, "needle", 1003, false, 5), Some(1000));
    }

    #[test]
    fn folding_finds_what_a_plain_scan_finds() {
        let hay = b"xLl-lL-LLzz-lLz-llZZq-LZZQ";
        for needle in ["l", "ll", "lz", "lzz", "lzzq", "-l", "q", "zz"] {
            let want = hay
                .windows(needle.len())
                .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()));
            assert_eq!(find(hay, needle.as_bytes(), true), want, "{needle}");
        }
    }

    #[test]
    fn a_folded_miss_reads_a_slice_once() {
        // Every byte a candidate, the other case nowhere: each look for it
        // once read to the end, a quadratic minute for 1 MB.
        let hay = vec![b'l'; 1 << 20];
        let t = std::time::Instant::now();
        assert_eq!(find(&hay, b"lzzq", true), None);
        assert!(t.elapsed().as_millis() < 1000, "{:?}", t.elapsed());
    }
}
