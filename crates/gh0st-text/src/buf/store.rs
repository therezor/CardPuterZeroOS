//! The file as opened: read with `pread` through a small cache of blocks.
//!
//! Not `mmap`: a mapped file that another program truncates kills the
//! reader with SIGBUS. Here a short read just comes back short, and the
//! editor says the file changed.
//!
//! Work that reads far ahead (the line index, find, save) may read without
//! waiting while a reader process brings the file into memory (`warm`): it
//! takes only what is there and gets `WouldBlock` for the rest. An SD card
//! reads 20 MB a second and stalls now and then, so a slice that waited for
//! it would freeze the page.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// One block of the file. 64 KB keeps a page of lines in one read.
pub const BLOCK: usize = 64 * 1024;
/// Blocks kept: 4 MB at most, whatever the file's size.
const SLOTS: usize = 64;

pub struct Store {
    file: Option<File>,
    len: u64,
    slots: Vec<Slot>,
    clock: u64,
    /// Reads take only what the page cache holds.
    nowait: bool,
}

struct Slot {
    /// Which block, by index.
    block: u64,
    data: Vec<u8>,
    used: u64,
}

impl Store {
    pub fn open(path: &Path) -> io::Result<Store> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Store {
            file: Some(file),
            len,
            slots: Vec::new(),
            clock: 0,
            nowait: false,
        })
    }

    /// No file: a new document.
    pub fn empty() -> Store {
        Store {
            file: None,
            len: 0,
            slots: Vec::new(),
            clock: 0,
            nowait: false,
        }
    }

    /// Reads from here on take only what is in memory: a block that is not
    /// fails with `WouldBlock`.
    pub fn set_nowait(&mut self, on: bool) {
        self.nowait = on;
    }

    /// The size the file had when it opened.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bytes from `off` to the end of its block. Shorter at the end of the
    /// file, and empty past it or once the file has shrunk under us.
    pub fn chunk(&mut self, off: u64) -> io::Result<&[u8]> {
        if off >= self.len {
            return Ok(&[]);
        }
        let block = off / BLOCK as u64;
        let i = self.slot(block)?;
        let at = (off - block * BLOCK as u64) as usize;
        let data = &self.slots[i].data;
        Ok(data.get(at..).unwrap_or(&[]))
    }

    /// Bytes from the start of `end - 1`'s block up to `end`: for reading
    /// backwards. Empty at 0.
    pub fn chunk_before(&mut self, end: u64) -> io::Result<&[u8]> {
        let end = end.min(self.len);
        if end == 0 {
            return Ok(&[]);
        }
        let block = (end - 1) / BLOCK as u64;
        let i = self.slot(block)?;
        let to = (end - block * BLOCK as u64) as usize;
        let data = &self.slots[i].data;
        // Shorter than asked: the file shrank, and these bytes are not the
        // ones before `end`.
        Ok(data.get(..to).unwrap_or(&[]))
    }

    /// Lets the cache go, for when memory runs short.
    pub fn shrink(&mut self) {
        self.slots = Vec::new();
    }

    fn slot(&mut self, block: u64) -> io::Result<usize> {
        self.clock += 1;
        if let Some(i) = self.slots.iter().position(|s| s.block == block) {
            self.slots[i].used = self.clock;
            return Ok(i);
        }
        let i = if self.slots.len() < SLOTS {
            self.slots.push(Slot {
                block,
                data: Vec::with_capacity(BLOCK),
                used: 0,
            });
            self.slots.len() - 1
        } else {
            let (i, _) = self
                .slots
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.used)
                .unwrap_or((0, &self.slots[0]));
            i
        };
        let slot = &mut self.slots[i];
        slot.block = block;
        slot.used = self.clock;
        slot.data.resize(BLOCK, 0);
        let start = block * BLOCK as u64;
        let want = (self.len - start).min(BLOCK as u64) as usize;
        let mut got = 0;
        if let Some(f) = &self.file {
            while got < want {
                let r = if self.nowait {
                    read_nowait(f, &mut slot.data[got..want], start + got as u64)
                } else {
                    f.read_at(&mut slot.data[got..want], start + got as u64)
                };
                match r {
                    Ok(0) => break,
                    Ok(n) => got += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        slot.data.clear();
                        slot.block = u64::MAX;
                        return Err(e);
                    }
                }
            }
        }
        slot.data.truncate(got);
        Ok(i)
    }
}

/// `pread` that takes only what the page cache holds: `preadv2` with
/// `RWF_NOWAIT`. A kernel or file system without it reads as usual.
#[cfg(target_os = "linux")]
fn read_nowait(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    const RWF_NOWAIT: libc::c_int = 0x8;
    let iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // SAFETY: one iovec over a live buffer. On 64-bit the offset is one
    // argument, and the high half is ignored.
    let n = unsafe {
        libc::syscall(
            libc::SYS_preadv2,
            f.as_raw_fd(),
            &iov as *const libc::iovec,
            1,
            off as libc::c_long,
            0 as libc::c_long,
            RWF_NOWAIT,
        )
    };
    if n >= 0 {
        return Ok(n as usize);
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EAGAIN) => Err(io::ErrorKind::WouldBlock.into()),
        Some(libc::EOPNOTSUPP | libc::EINVAL | libc::ENOSYS) => f.read_at(buf, off),
        _ => Err(e),
    }
}

#[cfg(not(target_os = "linux"))]
fn read_nowait(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    f.read_at(buf, off)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gh0st-text-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_across_blocks_and_keeps_at_most_four_megabytes() {
        let bytes: Vec<u8> = (0..BLOCK * 70 + 5).map(|i| (i % 251) as u8).collect();
        let p = temp("blocks", &bytes);
        let mut s = Store::open(&p).unwrap();
        let mut off = 0u64;
        while off < s.len() {
            let c = s.chunk(off).unwrap().to_vec();
            assert!(!c.is_empty());
            assert_eq!(c[..], bytes[off as usize..off as usize + c.len()]);
            off += c.len() as u64;
        }
        assert_eq!(off, bytes.len() as u64);
        assert!(s.slots.len() <= SLOTS);
        let back = s.chunk_before(BLOCK as u64 + 3).unwrap();
        assert_eq!(back, &bytes[BLOCK..BLOCK + 3]);
    }

    #[test]
    fn a_file_cut_short_reads_short_not_a_crash() {
        let p = temp("short", &[b'x'; 3 * BLOCK]);
        let mut s = Store::open(&p).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(BLOCK as u64 + 10)
            .unwrap();
        assert_eq!(s.chunk(0).unwrap().len(), BLOCK);
        assert_eq!(s.chunk(BLOCK as u64).unwrap().len(), 10);
        assert!(s.chunk(2 * BLOCK as u64).unwrap().is_empty());
    }
}
