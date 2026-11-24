//! Edits not saved yet, kept on disk so a crash, a kill or a flat battery
//! loses at most the last two seconds of typing. One file per document in
//! the data folder: a header naming the file as it was opened, then one
//! record per change. Each record carries its length and a CRC, so a write
//! cut short loses only itself.

use crate::buf::Change;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"gh0stJ1\n";
const REPLACE: u8 = 1;
const LOST: u8 = 2;
/// A record bigger than this is not trusted.
const RECORD_MAX: usize = 64 << 20;

/// The file a journal was kept for, as it was when the edits began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: i64,
}

impl Stamp {
    pub fn of(path: &Path) -> Option<Stamp> {
        let m = std::fs::metadata(path).ok()?;
        Some(Stamp {
            dev: m.dev(),
            ino: m.ino(),
            size: m.size(),
            mtime_ns: m.mtime() * 1_000_000_000 + m.mtime_nsec(),
        })
    }
}

/// What a journal held.
#[derive(Debug, PartialEq, Eq)]
pub struct Recovered {
    pub stamp: Stamp,
    pub changes: Vec<Change>,
}

impl Recovered {
    /// The edits fit the file as it is now: same file, untouched since.
    pub fn fits(&self, path: &Path) -> bool {
        Stamp::of(path).as_ref() == Some(&self.stamp)
    }

    /// The text that was typed or pasted, each piece on its own line: what
    /// can be kept when the edits no longer fit the file.
    pub fn typed(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // Where the run being typed ends: keys typed one after another.
        let mut run_end = None;
        for c in &self.changes {
            let Change::Replace { at, len, bytes } = c else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            let new_run = *len != 0 || run_end != Some(*at);
            if new_run && !out.is_empty() && !out.ends_with(b"\n") {
                out.push(b'\n');
            }
            out.extend_from_slice(bytes);
            run_end = Some(at + bytes.len() as u64);
        }
        if !out.is_empty() && !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        out
    }
}

/// Where the journal of `file` lives in `data`.
pub fn path_for(data: &Path, file: &Path) -> PathBuf {
    // FNV-1a over the path: a stable name, no separators.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in file.as_os_str().as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    data.join("journal").join(format!("{h:016x}"))
}

pub struct Journal {
    path: PathBuf,
    of: PathBuf,
    file: Option<File>,
    /// Records not on disk yet.
    pending: Vec<u8>,
}

impl Journal {
    /// The journal of `of`, kept in `data`. Nothing is written until there
    /// is something to keep.
    pub fn new(data: &Path, of: &Path) -> Journal {
        Journal {
            path: path_for(data, of),
            of: of.to_path_buf(),
            file: None,
            pending: Vec::new(),
        }
    }

    pub fn add(&mut self, c: &Change) {
        let mut body = Vec::new();
        match c {
            Change::Replace { at, len, bytes } => {
                body.push(REPLACE);
                body.extend_from_slice(&at.to_le_bytes());
                body.extend_from_slice(&len.to_le_bytes());
                body.extend_from_slice(bytes);
            }
            Change::Lost => body.push(LOST),
        }
        frame(&mut self.pending, &body);
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Writes what is pending and makes it last: fsync.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        if self.file.is_none() {
            let Some(stamp) = Stamp::of(&self.of) else {
                // A file not on disk yet has nothing to fit edits to.
                self.pending.clear();
                return Ok(());
            };
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)?;
            let mut head = MAGIC.to_vec();
            let mut h = Vec::new();
            for n in [stamp.dev, stamp.ino, stamp.size] {
                h.extend_from_slice(&n.to_le_bytes());
            }
            h.extend_from_slice(&stamp.mtime_ns.to_le_bytes());
            h.extend_from_slice(self.of.as_os_str().as_bytes());
            frame(&mut head, &h);
            f.write_all(&head)?;
            self.file = Some(f);
        }
        if let Some(f) = &mut self.file {
            f.write_all(&self.pending)?;
            f.sync_data()?;
        }
        self.pending.clear();
        Ok(())
    }

    /// The edits were saved or thrown away: the journal goes.
    pub fn discard(&mut self) {
        self.pending.clear();
        self.file = None;
        let _ = std::fs::remove_file(&self.path);
    }

    /// Carries on the journal found on disk, as after a restore.
    pub fn resume(&mut self) {
        self.file = OpenOptions::new().append(true).open(&self.path).ok();
    }
}

/// The journal of `file` in `data`, if there is one worth reading. Records
/// after a torn or damaged one are not trusted, and a journal that lost
/// track of an edit gives nothing back.
pub fn read(data: &Path, file: &Path) -> Option<Recovered> {
    let bytes = std::fs::read(path_for(data, file)).ok()?;
    let rest = bytes.strip_prefix(MAGIC.as_slice())?;
    let (head, mut rest) = unframe(rest)?;
    if head.len() < 32 {
        return None;
    }
    let n = |i: usize| u64::from_le_bytes(head[i..i + 8].try_into().unwrap_or([0; 8]));
    if &head[32..] != file.as_os_str().as_bytes() {
        return None;
    }
    let stamp = Stamp {
        dev: n(0),
        ino: n(8),
        size: n(16),
        mtime_ns: n(24) as i64,
    };
    let mut changes = Vec::new();
    while let Some((body, next)) = unframe(rest) {
        rest = next;
        match body.first() {
            Some(&REPLACE) if body.len() >= 17 => {
                let at = u64::from_le_bytes(body[1..9].try_into().ok()?);
                let len = u64::from_le_bytes(body[9..17].try_into().ok()?);
                changes.push(Change::Replace {
                    at,
                    len,
                    bytes: body[17..].to_vec(),
                });
            }
            Some(&LOST) => return None,
            _ => break,
        }
    }
    (!changes.is_empty()).then_some(Recovered { stamp, changes })
}

fn frame(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32(body).to_le_bytes());
    out.extend_from_slice(body);
}

/// One framed record and what follows it, or None at a torn or damaged
/// one.
fn unframe(b: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    let crc = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    if len > RECORD_MAX {
        return None;
    }
    let body = b.get(8..8 + len)?;
    (crc32(body) == crc).then(|| (body, &b[8 + len..]))
}

/// CRC-32 (IEEE), bit by bit: journals are small and written rarely.
fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("gh0st-text-journal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn edit(at: u64, len: u64, b: &[u8]) -> Change {
        Change::Replace {
            at,
            len,
            bytes: b.to_vec(),
        }
    }

    #[test]
    fn records_come_back_in_order_and_fit_the_file() {
        let d = dir("back");
        let f = d.join("doc.txt");
        std::fs::write(&f, b"hello\n").unwrap();
        let mut j = Journal::new(&d, &f);
        j.add(&edit(0, 0, b"X"));
        j.add(&edit(3, 2, b""));
        j.flush().unwrap();
        let r = read(&d, &f).unwrap();
        assert_eq!(r.changes, [edit(0, 0, b"X"), edit(3, 2, b"")]);
        assert!(r.fits(&f));
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        j.discard();
        assert!(read(&d, &f).is_none());
    }

    #[test]
    fn a_torn_last_record_loses_only_itself() {
        let d = dir("torn");
        let f = d.join("doc.txt");
        std::fs::write(&f, b"x").unwrap();
        let mut j = Journal::new(&d, &f);
        j.add(&edit(0, 0, b"first"));
        j.add(&edit(0, 0, b"second"));
        j.flush().unwrap();
        let p = path_for(&d, &f);
        let len = std::fs::metadata(&p).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(len - 3)
            .unwrap();
        assert_eq!(read(&d, &f).unwrap().changes, [edit(0, 0, b"first")]);
    }

    #[test]
    fn a_file_changed_since_does_not_fit_and_a_lost_edit_gives_nothing() {
        let d = dir("changed");
        let f = d.join("doc.txt");
        std::fs::write(&f, b"one").unwrap();
        let mut j = Journal::new(&d, &f);
        j.add(&edit(0, 0, b"typed"));
        j.flush().unwrap();
        std::fs::write(&f, b"one and more").unwrap();
        let r = read(&d, &f).unwrap();
        assert!(!r.fits(&f));
        assert_eq!(r.typed(), b"typed\n");
        j.add(&Change::Lost);
        j.flush().unwrap();
        assert!(read(&d, &f).is_none());
    }
}
