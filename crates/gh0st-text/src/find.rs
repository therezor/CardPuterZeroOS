//! Find, a slice at a time: a search through a gigabyte runs between key
//! presses and stops at Esc.

use crate::buf::{Buffer, scan};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// The next match starts here.
    Hit(u64),
    /// Searched the whole text: none.
    Miss,
    /// Not done: call again.
    More,
}

pub struct Finder {
    needle: Vec<u8>,
    /// Letters match whatever their case.
    fold: bool,
    /// Where the search started; it wraps round to here.
    start: u64,
    at: u64,
    wrapped: bool,
    /// The last bytes of the chunk before, so a match across chunks counts.
    carry: Vec<u8>,
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
            carry: Vec::new(),
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
        let mut window = std::mem::take(&mut self.carry);
        let base = self.at - window.len() as u64;
        b.for_each(self.at..end, |c| {
            window.extend_from_slice(c);
            true
        })?;
        if let Some(i) = find(&window, &self.needle, self.fold) {
            let hit = base + i as u64;
            return Ok(Found::Hit(hit));
        }
        self.at = end;
        let keep = window.len().min(n - 1);
        self.carry = window[window.len() - keep..].to_vec();
        if end < limit {
            return Ok(Found::More);
        }
        if self.wrapped || self.start == 0 {
            return Ok(Found::Miss);
        }
        // Round to the top; a match may straddle where it began.
        self.wrapped = true;
        self.at = 0;
        self.carry.clear();
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
    let mut i = 0;
    while i + n <= hay.len() {
        let next = if fold && first.is_ascii_alphabetic() {
            let lo = scan::memchr(first, &hay[i..]);
            let up = scan::memchr(first.to_ascii_uppercase(), &hay[i..]);
            match (lo, up) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        } else {
            scan::memchr(first, &hay[i..])
        };
        let p = i + next?;
        if p + n > hay.len() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(b: &mut Buffer, needle: &str, from: u64, fold: bool, budget: u64) -> Option<u64> {
        let mut f = Finder::new(needle.as_bytes(), from, fold);
        loop {
            match f.step(b, budget).unwrap() {
                Found::Hit(at) => return Some(at),
                Found::Miss => return None,
                Found::More => {}
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
}
