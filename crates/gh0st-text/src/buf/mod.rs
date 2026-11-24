//! The document: a piece table over the file as opened and a buffer of
//! everything typed since. Opening reads nothing but the size; an edit
//! touches a few pieces, never the file. Bytes stay exactly as they were:
//! text that is not UTF-8, CR LF, a missing final newline.

mod index;
pub mod scan;
mod store;
mod undo;

pub use index::Lines;
pub use store::{BLOCK, Store};

use index::Index;
use std::io;
use std::ops::Range;
use std::path::Path;
use undo::{Splice, Undo};

/// Where a piece's bytes live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Src {
    /// A file as opened: the one on disk, or one an earlier save replaced,
    /// kept while undo needs it.
    Orig(u32),
    Add,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub src: Src,
    pub off: u64,
    pub len: u64,
}

pub struct Buffer {
    stores: Vec<Store>,
    add: Vec<u8>,
    pieces: Vec<Piece>,
    /// Where each piece starts in the document.
    starts: Vec<u64>,
    len: u64,
    undo: Undo,
    index: Index,
    /// Bumped by every change, so a view knows its caches are stale.
    version: u64,
}

impl Buffer {
    /// The file at `path`. Reads its size and nothing else.
    pub fn open(path: &Path) -> io::Result<Buffer> {
        Ok(Buffer::with_store(Store::open(path)?))
    }

    /// A new document holding `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> Buffer {
        let mut b = Buffer::with_store(Store::empty());
        if !bytes.is_empty() {
            b.add.extend_from_slice(bytes);
            b.pieces.push(Piece {
                src: Src::Add,
                off: 0,
                len: bytes.len() as u64,
            });
            b.starts.push(0);
            b.len = bytes.len() as u64;
        }
        b
    }

    fn with_store(store: Store) -> Buffer {
        let len = store.len();
        let mut b = Buffer {
            stores: vec![store],
            add: Vec::new(),
            pieces: Vec::new(),
            starts: Vec::new(),
            len,
            undo: Undo::default(),
            index: Index::default(),
            version: 0,
        };
        if len > 0 {
            b.pieces.push(Piece {
                src: Src::Orig(0),
                off: 0,
                len,
            });
            b.starts.push(0);
        }
        b
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Pieces in the table: a measure of how fragmented editing has left it.
    pub fn pieces(&self) -> usize {
        self.pieces.len()
    }

    /// Bytes typed or pasted so far, kept for undo.
    pub fn added(&self) -> usize {
        self.add.len()
    }

    /// Lets the block cache go.
    pub fn shrink(&mut self) {
        for s in &mut self.stores {
            s.shrink();
        }
    }

    /// The piece holding `off`, and where it starts. `off == len` gives the
    /// piece count.
    fn find(&self, off: u64) -> (usize, u64) {
        let i = self.starts.partition_point(|&s| s <= off);
        if i == 0 {
            return (0, 0);
        }
        let i = i - 1;
        let p = self.pieces[i];
        if off < self.starts[i] + p.len {
            (i, self.starts[i])
        } else {
            (i + 1, self.starts[i] + p.len)
        }
    }

    /// Calls `f` with the document's bytes in `range`, a chunk at a time,
    /// until it returns false. A file that shrank on disk ends early with
    /// an error.
    pub fn for_each(
        &mut self,
        range: Range<u64>,
        mut f: impl FnMut(&[u8]) -> bool,
    ) -> io::Result<()> {
        let end = range.end.min(self.len);
        let mut off = range.start;
        if off >= end {
            return Ok(());
        }
        let (mut i, mut start) = self.find(off);
        while off < end && i < self.pieces.len() {
            let p = self.pieces[i];
            let take_to = end.min(start + p.len);
            while off < take_to {
                let at = p.off + (off - start);
                let want = (take_to - off) as usize;
                let chunk: &[u8] = match p.src {
                    Src::Add => &self.add[at as usize..at as usize + want],
                    Src::Orig(g) => {
                        let c = self.stores[g as usize].chunk(at)?;
                        if c.is_empty() {
                            return Err(changed());
                        }
                        &c[..c.len().min(want)]
                    }
                };
                off += chunk.len() as u64;
                if !f(chunk) {
                    return Ok(());
                }
            }
            start += p.len;
            i += 1;
        }
        Ok(())
    }

    /// As [`for_each`](Buffer::for_each), backwards from `range.end`: each
    /// chunk ends where the one before it began.
    pub fn for_each_back(
        &mut self,
        range: Range<u64>,
        mut f: impl FnMut(&[u8]) -> bool,
    ) -> io::Result<()> {
        let mut end = range.end.min(self.len);
        let start = range.start;
        while end > start {
            let (i, pstart) = self.find(end - 1);
            let p = self.pieces[i];
            let from = start.max(pstart);
            let at_end = p.off + (end - pstart);
            let want = (end - from) as usize;
            let chunk: &[u8] = match p.src {
                Src::Add => &self.add[at_end as usize - want..at_end as usize],
                Src::Orig(g) => {
                    let c = self.stores[g as usize].chunk_before(at_end)?;
                    if c.is_empty() {
                        return Err(changed());
                    }
                    &c[c.len().saturating_sub(want)..]
                }
            };
            end -= chunk.len() as u64;
            if !f(chunk) {
                return Ok(());
            }
        }
        Ok(())
    }

    /// The bytes in `range`, for short ranges: a line, a selection.
    pub fn bytes(&mut self, range: Range<u64>) -> io::Result<Vec<u8>> {
        let mut out =
            Vec::with_capacity(range.end.saturating_sub(range.start).min(1 << 20) as usize);
        self.for_each(range, |c| {
            out.extend_from_slice(c);
            true
        })?;
        Ok(out)
    }

    /// The first `b` at or after `from`, searching no further than `limit`.
    pub fn find_byte(&mut self, b: u8, from: u64, limit: u64) -> io::Result<Option<u64>> {
        let mut off = from;
        let mut hit = None;
        self.for_each(from..limit, |c| match scan::memchr(b, c) {
            Some(i) => {
                hit = Some(off + i as u64);
                false
            }
            None => {
                off += c.len() as u64;
                true
            }
        })?;
        Ok(hit)
    }

    /// The last `b` before `before`, searching back no further than `limit`.
    pub fn rfind_byte(&mut self, b: u8, before: u64, limit: u64) -> io::Result<Option<u64>> {
        let mut end = before.min(self.len);
        let mut hit = None;
        self.for_each_back(limit..before, |c| {
            let start = end - c.len() as u64;
            match scan::memrchr(b, c) {
                Some(i) => {
                    hit = Some(start + i as u64);
                    false
                }
                None => {
                    end = start;
                    true
                }
            }
        })?;
        Ok(hit)
    }

    /// Where the line holding `off` starts.
    pub fn line_start(&mut self, off: u64) -> io::Result<u64> {
        Ok(self.rfind_byte(b'\n', off, 0)?.map_or(0, |p| p + 1))
    }

    /// Where the line holding `off` ends: its newline, or the end.
    pub fn line_end(&mut self, off: u64) -> io::Result<u64> {
        let len = self.len;
        Ok(self.find_byte(b'\n', off, len)?.unwrap_or(len))
    }

    /// True when the first block holds a NUL: a binary file, shown as hex.
    pub fn looks_binary(&mut self) -> io::Result<bool> {
        let mut nul = false;
        self.for_each(0..BLOCK as u64, |c| {
            nul = c.contains(&0);
            !nul
        })?;
        Ok(nul)
    }

    /// Puts `bytes` at `at`.
    pub fn insert(&mut self, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.replace(at..at, bytes)
    }

    /// Takes out `range`.
    pub fn delete(&mut self, range: Range<u64>) -> io::Result<()> {
        self.replace(range, &[])
    }

    /// Takes out `range` and puts `bytes` there, as one change.
    pub fn replace(&mut self, range: Range<u64>, bytes: &[u8]) -> io::Result<()> {
        let at = range.start.min(self.len);
        let end = range.end.clamp(at, self.len);
        if at == end && bytes.is_empty() {
            return Ok(());
        }
        let old_nl = self.newlines(at..end)?;
        let new = if bytes.is_empty() {
            Vec::new()
        } else {
            let off = self.add.len() as u64;
            self.add.extend_from_slice(bytes);
            vec![Piece {
                src: Src::Add,
                off,
                len: bytes.len() as u64,
            }]
        };
        let new_nl = Some(scan::count(b'\n', bytes) as u64);
        let old = self.splice(at, end - at, &new);
        self.index
            .edit(at, end - at, old_nl, bytes.len() as u64, new_nl);
        self.undo.record(Splice {
            at,
            old,
            old_len: end - at,
            old_nl,
            new,
            new_len: bytes.len() as u64,
            new_nl,
        });
        Ok(())
    }

    /// Newlines in `range`, when the index can say so cheaply: inside what
    /// it has read, or short.
    fn newlines(&mut self, range: Range<u64>) -> io::Result<Option<u64>> {
        if range.is_empty() {
            return Ok(Some(0));
        }
        if range.end - range.start <= 1 << 20 {
            let mut n = 0;
            self.for_each(range, |c| {
                n += scan::count(b'\n', c) as u64;
                true
            })?;
            return Ok(Some(n));
        }
        match (self.line_of(range.start)?, self.line_of(range.end)?) {
            (Some(a), Some(b)) => Ok(Some(b - a)),
            _ => Ok(None),
        }
    }

    /// Replaces `len` bytes at `at` with `new`, returning the pieces taken
    /// out. Keeps `starts` in step.
    fn splice(&mut self, at: u64, len: u64, new: &[Piece]) -> Vec<Piece> {
        self.version += 1;
        let end = at + len;
        // Typing at the end of the last thing typed grows that piece.
        if len == 0 && new.len() == 1 && new[0].src == Src::Add && at > 0 {
            let (i, s) = self.find(at - 1);
            if let Some(p) = self.pieces.get_mut(i) {
                if p.src == Src::Add && s + p.len == at && p.off + p.len == new[0].off {
                    p.len += new[0].len;
                    self.len += new[0].len;
                    for st in &mut self.starts[i + 1..] {
                        *st += new[0].len;
                    }
                    return Vec::new();
                }
            }
        }
        let split = |b: &mut Buffer, off: u64| -> usize {
            let (i, s) = b.find(off);
            if i < b.pieces.len() && s < off {
                let p = b.pieces[i];
                let left = off - s;
                b.pieces[i].len = left;
                b.pieces.insert(
                    i + 1,
                    Piece {
                        src: p.src,
                        off: p.off + left,
                        len: p.len - left,
                    },
                );
                b.starts.insert(i + 1, off);
                i + 1
            } else {
                i
            }
        };
        let a = split(self, at);
        let b = if len == 0 { a } else { split(self, end) };
        let old: Vec<Piece> = self.pieces.splice(a..b, new.iter().copied()).collect();
        let new_len: u64 = new.iter().map(|p| p.len).sum();
        self.len = self.len - len + new_len;
        self.starts.truncate(a);
        let mut s = at;
        for p in &self.pieces[a..] {
            self.starts.push(s);
            s += p.len;
        }
        old
    }

    /// Ends the current undo step: the next edit starts a new one. Typing
    /// runs into one step until the cursor jumps or a command runs.
    pub fn seal(&mut self) {
        self.undo.seal();
    }

    /// Takes back the last step. Returns where it happened, for the cursor.
    pub fn undo(&mut self) -> Option<u64> {
        let group = self.undo.take_undo()?;
        let mut at = None;
        for s in group.iter().rev() {
            self.splice(s.at, s.new_len, &s.old);
            self.index
                .edit(s.at, s.new_len, s.new_nl, s.old_len, s.old_nl);
            at = Some(s.at + s.old_len);
        }
        self.undo.push_redo(group);
        at
    }

    /// Does again what [`undo`](Buffer::undo) took back.
    pub fn redo(&mut self) -> Option<u64> {
        let group = self.undo.take_redo()?;
        let mut at = None;
        for s in &group {
            self.splice(s.at, s.old_len, &s.new);
            self.index
                .edit(s.at, s.old_len, s.old_nl, s.new_len, s.new_nl);
            at = Some(s.at + s.new_len);
        }
        self.undo.push_done(group);
        at
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Reads on with the line index for about `budget` bytes. True once it
    /// has read the whole document.
    pub fn index_step(&mut self, budget: u64) -> io::Result<bool> {
        let mut ix = std::mem::take(&mut self.index);
        let r = ix.step(self, budget);
        self.index = ix;
        r
    }

    /// How many lines, and whether that is exact or a guess from what the
    /// index has read so far.
    pub fn lines(&self) -> Lines {
        self.index.lines(self.len)
    }

    /// How far the index has read, of the document's length.
    pub fn indexed(&self) -> u64 {
        self.index.done()
    }

    /// The line (from 0) that `off` is on, once the index has read that far.
    pub fn line_of(&mut self, off: u64) -> io::Result<Option<u64>> {
        let ix = std::mem::take(&mut self.index);
        let r = ix.line_of(self, off);
        self.index = ix;
        r
    }

    /// Where line `n` (from 0) starts, once the index has read that far.
    pub fn line_offset(&mut self, n: u64) -> io::Result<Option<u64>> {
        let ix = std::mem::take(&mut self.index);
        let r = ix.offset_of(self, n);
        self.index = ix;
        r
    }
}

fn changed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "the file changed on disk")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            if n == 0 { 0 } else { self.next() as u64 % n }
        }
    }

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gh0st-text-buf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn all(b: &mut Buffer) -> Vec<u8> {
        let len = b.len();
        b.bytes(0..len).unwrap()
    }

    fn lines_model(v: &[u8]) -> u64 {
        v.iter().filter(|&&c| c == b'\n').count() as u64 + 1
    }

    /// Random edits, undos and redos against a plain `Vec<u8>`, with the
    /// index read part way and then to the end.
    #[test]
    fn random_edits_match_a_plain_vector() {
        let mut rng = Rng(12345);
        let base: Vec<u8> = (0..300_000u32)
            .map(|i| {
                if i % 37 == 0 {
                    b'\n'
                } else {
                    b'a' + (i % 26) as u8
                }
            })
            .collect();
        let p = temp("random", &base);
        let mut b = Buffer::open(&p).unwrap();
        let mut model = base.clone();
        let mut history: Vec<Vec<u8>> = vec![model.clone()];
        let mut at_step = 0usize;
        b.index_step(100_000).unwrap();
        for step in 0..2000 {
            match rng.below(10) {
                0..=3 => {
                    let at = rng.below(model.len() as u64 + 1);
                    let n = rng.below(20) as usize;
                    let ins: Vec<u8> = (0..n)
                        .map(|_| if rng.below(5) == 0 { b'\n' } else { b'X' })
                        .collect();
                    b.insert(at, &ins).unwrap();
                    model.splice(at as usize..at as usize, ins);
                }
                4..=6 => {
                    let at = rng.below(model.len() as u64);
                    let n = rng.below(50).min(model.len() as u64 - at);
                    b.delete(at..at + n).unwrap();
                    model.drain(at as usize..(at + n) as usize);
                }
                7 => {
                    let at = rng.below(model.len() as u64);
                    let n = rng.below(10).min(model.len() as u64 - at);
                    b.replace(at..at + n, b"r\nr").unwrap();
                    model.splice(at as usize..(at + n) as usize, b"r\nr".iter().copied());
                }
                8 => {
                    b.seal();
                }
                _ => {
                    b.seal();
                    history.truncate(at_step + 1);
                    history.push(model.clone());
                    at_step = history.len() - 1;
                }
            }
            if step % 97 == 0 {
                b.index_step(50_000).unwrap();
            }
            assert_eq!(b.len(), model.len() as u64);
        }
        assert_eq!(all(&mut b), model);
        while !b.index_step(1 << 20).unwrap() {}
        let l = b.lines();
        assert!(l.exact);
        assert_eq!(l.count, lines_model(&model));
        // Every line start the index gives is a real line start.
        let mut starts = vec![0u64];
        starts.extend(
            model
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c == b'\n')
                .map(|(i, _)| i as u64 + 1),
        );
        for n in (0..starts.len()).step_by(starts.len() / 50 + 1) {
            assert_eq!(
                b.line_offset(n as u64).unwrap(),
                Some(starts[n]),
                "line {n}"
            );
            assert_eq!(b.line_of(starts[n]).unwrap(), Some(n as u64));
        }
        // Undo all the way back to the file.
        while b.undo().is_some() {}
        assert_eq!(all(&mut b), base);
        while b.redo().is_some() {}
        assert_eq!(all(&mut b), model);
        let _ = history;
    }

    #[test]
    fn undo_takes_back_a_typing_run_in_one_step() {
        let mut b = Buffer::from_bytes(b"hello\n");
        for (i, c) in b" world".iter().enumerate() {
            b.insert(5 + i as u64, &[*c]).unwrap();
        }
        assert_eq!(all(&mut b), b"hello world\n");
        assert_eq!(b.pieces(), 3);
        b.seal();
        b.delete(0..6).unwrap();
        assert_eq!(all(&mut b), b"world\n");
        assert_eq!(b.undo(), Some(6));
        assert_eq!(all(&mut b), b"hello world\n");
        assert_eq!(b.undo(), Some(5));
        assert_eq!(all(&mut b), b"hello\n");
        assert_eq!(b.undo(), None);
        b.redo();
        b.redo();
        assert_eq!(all(&mut b), b"world\n");
    }

    #[test]
    fn bytes_stay_exact() {
        let raw = b"crlf\r\nbad \xff\xfe\nhalf \xe2\x82\nno newline";
        let p = temp("exact", raw);
        let mut b = Buffer::open(&p).unwrap();
        assert_eq!(all(&mut b), raw);
        b.insert(0, b"x").unwrap();
        b.delete(0..1).unwrap();
        assert_eq!(all(&mut b), raw);
        assert!(!b.looks_binary().unwrap());
        assert!(Buffer::from_bytes(b"a\0b").looks_binary().unwrap());
    }

    #[test]
    fn lines_are_found_both_ways() {
        let mut b = Buffer::from_bytes(b"one\ntwo\n\nfour");
        assert_eq!(b.line_start(5).unwrap(), 4);
        assert_eq!(b.line_end(5).unwrap(), 7);
        assert_eq!(b.line_start(8).unwrap(), 8);
        assert_eq!(b.line_end(9).unwrap(), 13);
        assert_eq!(b.line_start(0).unwrap(), 0);
        assert_eq!(b.rfind_byte(b'\n', 4, 0).unwrap(), Some(3));
        assert_eq!(b.rfind_byte(b'\n', 3, 0).unwrap(), None);
    }

    #[test]
    fn a_file_that_shrinks_reads_as_changed() {
        let p = temp("shrinks", &vec![b'a'; 3 * BLOCK]);
        let mut b = Buffer::open(&p).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(10)
            .unwrap();
        let len = b.len();
        let e = b.bytes(0..len).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn the_index_guesses_then_counts() {
        let text: Vec<u8> = b"0123456789\n".repeat(100_000);
        let p = temp("guess", &text);
        let mut b = Buffer::open(&p).unwrap();
        assert!(!b.lines().exact);
        b.index_step(200_000).unwrap();
        let guess = b.lines();
        assert!(!guess.exact);
        assert!(
            guess.count > 90_000 && guess.count < 110_000,
            "{}",
            guess.count
        );
        while !b.index_step(200_000).unwrap() {}
        assert_eq!(b.lines().count, 100_001);
        assert_eq!(b.line_offset(99_999).unwrap(), Some(99_999 * 11));
        assert_eq!(b.line_offset(100_000).unwrap(), Some(1_100_000));
        assert_eq!(b.line_offset(100_001).unwrap(), None);
    }
}
