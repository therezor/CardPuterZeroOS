//! Line numbers for a document of any size. A mark every 64 KB or so says
//! which line starts there: 16 K marks for a gigabyte, where a mark per
//! line would take hundreds of megabytes. The rest is counted when asked,
//! from the nearest mark. The index reads the document a slice at a time,
//! so opening a big file shows its first screen at once.

use super::{Buffer, scan};
use std::io;

/// How far apart marks are, in bytes.
const SPACING: u64 = 64 * 1024;

/// A line count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lines {
    pub count: u64,
    /// False while the index is still reading: `count` is a guess from the
    /// part read so far.
    pub exact: bool,
}

#[derive(Default)]
pub(crate) struct Index {
    /// (offset, line) of line starts, ascending. The first is (0, 0).
    marks: Vec<(u64, u64)>,
    /// Read up to here.
    done: u64,
    /// Newlines before `done`.
    done_line: u64,
}

impl Index {
    pub fn done(&self) -> u64 {
        self.done
    }

    fn last_mark(&self) -> (u64, u64) {
        self.marks.last().copied().unwrap_or((0, 0))
    }

    pub fn lines(&self, len: u64) -> Lines {
        if self.done >= len {
            return Lines {
                count: self.done_line + 1,
                exact: true,
            };
        }
        let count = if self.done == 0 {
            1
        } else {
            ((self.done_line as u128 * len as u128) / self.done as u128) as u64 + 1
        };
        Lines {
            count,
            exact: false,
        }
    }

    /// Reads on for about `budget` bytes. True when it reached the end.
    pub fn step(&mut self, b: &mut Buffer, budget: u64) -> io::Result<bool> {
        if self.marks.is_empty() {
            self.marks.push((0, 0));
        }
        let end = b.len().min(self.done.saturating_add(budget.max(1)));
        let mut off = self.done;
        let mut line = self.done_line;
        let mut last = self.last_mark();
        let marks = &mut self.marks;
        let r = b.for_each(self.done..end, |c| {
            let mut rest = c;
            let mut at = off;
            while !rest.is_empty() {
                let due = last.0 + SPACING;
                if at + (rest.len() as u64) < due {
                    line += scan::count(b'\n', rest) as u64;
                    at += rest.len() as u64;
                    break;
                }
                // Count up to the spacing, then mark the next line start.
                let skip = due.saturating_sub(at) as usize;
                line += scan::count(b'\n', &rest[..skip]) as u64;
                at += skip as u64;
                rest = &rest[skip..];
                match scan::memchr(b'\n', rest) {
                    Some(i) => {
                        line += 1;
                        at += i as u64 + 1;
                        rest = &rest[i + 1..];
                        last = (at, line);
                        marks.push(last);
                    }
                    None => {
                        at += rest.len() as u64;
                        break;
                    }
                }
            }
            off = at;
            true
        });
        // What was read counts, even when the disk stopped the slice.
        self.done = off;
        self.done_line = line;
        r?;
        Ok(self.done >= b.len())
    }

    /// The line `off` is on, if the index has read that far.
    pub fn line_of(&self, b: &mut Buffer, off: u64) -> io::Result<Option<u64>> {
        if off > self.done {
            return Ok(None);
        }
        let i = self.marks.partition_point(|&(o, _)| o <= off);
        let (mo, ml) = if i == 0 { (0, 0) } else { self.marks[i - 1] };
        let mut n = ml;
        b.for_each(mo..off, |c| {
            n += scan::count(b'\n', c) as u64;
            true
        })?;
        Ok(Some(n))
    }

    /// Where line `n` starts, if the index has read that far.
    pub fn offset_of(&self, b: &mut Buffer, n: u64) -> io::Result<Option<u64>> {
        if n > self.done_line {
            return Ok(None);
        }
        if n == 0 {
            return Ok(Some(0));
        }
        let i = self.marks.partition_point(|&(_, l)| l <= n);
        let (mut at, ml) = if i == 0 { (0, 0) } else { self.marks[i - 1] };
        let mut left = n - ml;
        let mut found = (left == 0).then_some(at);
        let limit = self.done;
        b.for_each(at..limit, |c| {
            let mut rest = c;
            while left > 0 {
                match scan::memchr(b'\n', rest) {
                    Some(i) => {
                        left -= 1;
                        at += i as u64 + 1;
                        rest = &rest[i + 1..];
                        if left == 0 {
                            found = Some(at);
                            return false;
                        }
                    }
                    None => {
                        at += rest.len() as u64;
                        return true;
                    }
                }
            }
            false
        })?;
        Ok(found)
    }

    /// `old_len` bytes at `at`, holding `old_nl` newlines, became `new_len`
    /// bytes holding `new_nl`. A count of None is not known: the index
    /// forgets from `at` on and reads that part again.
    pub fn edit(
        &mut self,
        at: u64,
        old_len: u64,
        old_nl: Option<u64>,
        new_len: u64,
        new_nl: Option<u64>,
    ) {
        if at > self.done || self.marks.is_empty() {
            return;
        }
        let end = at + old_len;
        let (Some(old_nl), Some(new_nl), true) = (old_nl, new_nl, end <= self.done) else {
            // Start again from the last line start at or before `at`.
            self.marks.retain(|&(o, _)| o <= at);
            let (o, l) = self.last_mark();
            self.done = o;
            self.done_line = l;
            return;
        };
        // A mark inside what went away, or right after it, may no longer be
        // a line start.
        self.marks.retain(|&(o, _)| o <= at || o > end);
        for m in &mut self.marks {
            if m.0 > at {
                m.0 = m.0 - old_len + new_len;
                m.1 = m.1 - old_nl + new_nl;
            }
        }
        self.done = self.done - old_len + new_len;
        self.done_line = self.done_line - old_nl + new_nl;
    }
}
