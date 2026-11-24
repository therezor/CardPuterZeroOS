//! The frame ring: shared memory a helper draws pictures into and the app
//! reads, so 100 KB a frame never crosses a socket. The app makes it, and
//! the helper inherits its fd ([`crate::sidecar`]). `native/av/ring.cpp` is
//! the C++ writer.
//!
//! A 64-byte header (magic, version, slot count, pixels per slot), then the
//! slots. Each slot is a 16-byte header (a sequence number, width, height)
//! and RGB565 pixels, rows packed. The writer makes the sequence odd while
//! it writes and even when done; a reader that sees it change drops the
//! frame. The helper never rewrites the slot it announced last.

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering, fence};

const MAGIC: u32 = 0x4730_4352; // "G0CR"
const VERSION: u32 = 1;
const HEAD: usize = 64;
const SLOT_HEAD: usize = 16;

/// The whole panel: the most pixels a slot needs to hold.
pub const SCREEN_PX: usize = 320 * 170;

pub struct Ring {
    fd: OwnedFd,
    base: *mut u8,
    len: usize,
    slots: u32,
    slot_px: usize,
}

// SAFETY: the mapping belongs to the process; the seqlock orders access.
unsafe impl Send for Ring {}

fn slot_bytes(px: usize) -> usize {
    SLOT_HEAD + px * 2
}

fn memfd(name: &CStr) -> io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: a NUL-terminated name; the fd is fresh and ours.
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Hosts without memfd: a temp file, gone from the directory at once.
        let mut tpl = std::env::temp_dir()
            .join(format!("{}-XXXXXX", name.to_string_lossy()))
            .into_os_string()
            .into_encoded_bytes();
        tpl.push(0);
        // SAFETY: mkstemp fills a NUL-terminated template in place.
        let fd = unsafe { libc::mkstemp(tpl.as_mut_ptr().cast()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the same template, now a real path.
        unsafe {
            libc::unlink(tpl.as_ptr().cast());
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            Ok(OwnedFd::from_raw_fd(fd))
        }
    }
}

fn map(fd: &OwnedFd, len: usize) -> io::Result<*mut u8> {
    // SAFETY: maps `len` bytes of a file at least that long.
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if p == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(p.cast())
}

impl Ring {
    /// A new ring of `slots` slots of `slot_px` pixels each, named `name`
    /// in /proc.
    pub fn create(name: &CStr, slots: u32, slot_px: usize) -> io::Result<Ring> {
        let fd = memfd(name)?;
        let len = HEAD + slots as usize * slot_bytes(slot_px);
        // SAFETY: ftruncate on our own fd.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let base = map(&fd, len)?;
        let r = Ring {
            fd,
            base,
            len,
            slots,
            slot_px,
        };
        r.put_u32(0, MAGIC);
        r.put_u32(4, VERSION);
        r.put_u32(8, slots);
        r.put_u32(12, slot_px as u32);
        Ok(r)
    }

    /// The ring behind an inherited fd, checked.
    pub fn open(fd: OwnedFd) -> io::Result<Ring> {
        // SAFETY: fstat into a zeroed struct.
        let size = unsafe {
            let mut st: libc::stat = std::mem::zeroed();
            if libc::fstat(fd.as_raw_fd(), &mut st) != 0 {
                return Err(io::Error::last_os_error());
            }
            st.st_size as usize
        };
        if size < HEAD {
            return Err(io::Error::other("ring too small"));
        }
        let base = map(&fd, size)?;
        let mut r = Ring {
            fd,
            base,
            len: size,
            slots: 0,
            slot_px: 0,
        };
        let (slots, px) = (r.get_u32(8), r.get_u32(12) as usize);
        if r.get_u32(0) != MAGIC || r.get_u32(4) != VERSION {
            return Err(io::Error::other("not a frame ring"));
        }
        if HEAD + slots as usize * slot_bytes(px) > size {
            return Err(io::Error::other("ring header lies"));
        }
        (r.slots, r.slot_px) = (slots, px);
        Ok(r)
    }

    /// A second fd for the same memory, for a helper to inherit.
    pub fn share(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn slots(&self) -> u32 {
        self.slots
    }

    pub fn slot_px(&self) -> usize {
        self.slot_px
    }

    fn put_u32(&self, off: usize, v: u32) {
        // SAFETY: inside the mapping, 4-aligned.
        unsafe { self.base.add(off).cast::<u32>().write(v) }
    }

    fn get_u32(&self, off: usize) -> u32 {
        // SAFETY: inside the mapping, 4-aligned.
        unsafe { self.base.add(off).cast::<u32>().read() }
    }

    fn slot_off(&self, slot: u32) -> Option<usize> {
        (slot < self.slots).then(|| HEAD + slot as usize * slot_bytes(self.slot_px))
    }

    fn seq(&self, off: usize) -> &AtomicU32 {
        // SAFETY: the slot header starts 4-aligned inside the mapping, and
        // the other side touches it only atomically.
        unsafe { &*self.base.add(off).cast::<AtomicU32>() }
    }

    /// Writes a `w` x `h` frame into `slot` through `fill`, and returns the
    /// sequence number to announce. `seq` must be even and new.
    pub fn write(&self, slot: u32, seq: u32, w: u16, h: u16, fill: impl FnOnce(&mut [u16])) {
        let Some(off) = self.slot_off(slot) else {
            return;
        };
        let n = (w as usize * h as usize).min(self.slot_px);
        let s = self.seq(off);
        s.store(seq | 1, Ordering::Relaxed);
        fence(Ordering::Release);
        // SAFETY: the slot's pixels lie inside the mapping and are 2-aligned.
        let px =
            unsafe { std::slice::from_raw_parts_mut(self.base.add(off + SLOT_HEAD).cast(), n) };
        fill(px);
        // SAFETY: header fields inside the slot.
        unsafe {
            self.base.add(off + 4).cast::<u16>().write(w);
            self.base.add(off + 6).cast::<u16>().write(h);
        }
        s.store(seq & !1, Ordering::Release);
    }

    /// Copies the frame announced as `seq` in `slot` into `out`. None when
    /// the writer has moved on: drop that frame.
    pub fn read(&self, slot: u32, seq: u32, out: &mut Vec<u16>) -> Option<(u16, u16)> {
        let off = self.slot_off(slot)?;
        let s = self.seq(off);
        if s.load(Ordering::Acquire) != seq {
            return None;
        }
        // SAFETY: header fields inside the slot.
        let (w, h) = unsafe {
            (
                self.base.add(off + 4).cast::<u16>().read(),
                self.base.add(off + 6).cast::<u16>().read(),
            )
        };
        let n = w as usize * h as usize;
        if n > self.slot_px {
            return None;
        }
        out.clear();
        // SAFETY: n pixels inside the slot; the seqlock check below drops
        // a torn copy.
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(self.base.add(off + SLOT_HEAD).cast::<u16>(), n)
        });
        fence(Ordering::Acquire);
        (s.load(Ordering::Relaxed) == seq).then_some((w, h))
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: unmaps exactly what `map` mapped.
        unsafe { libc::munmap(self.base.cast(), self.len) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> Ring {
        Ring::create(c"gh0st-test", 4, SCREEN_PX).unwrap()
    }

    #[test]
    fn a_frame_written_on_one_side_reads_on_the_other() {
        let a = ring();
        let b = Ring::open(a.share().unwrap()).unwrap();
        a.write(1, 4, 3, 2, |px| px.copy_from_slice(&[1, 2, 3, 4, 5, 6]));
        let mut out = Vec::new();
        assert_eq!(b.read(1, 4, &mut out), Some((3, 2)));
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_stale_announcement_reads_nothing() {
        let a = ring();
        a.write(0, 2, 1, 1, |px| px[0] = 7);
        a.write(0, 6, 1, 1, |px| px[0] = 8);
        let mut out = Vec::new();
        assert_eq!(a.read(0, 2, &mut out), None);
        assert_eq!(a.read(0, 6, &mut out), Some((1, 1)));
        assert_eq!(a.read(9, 6, &mut out), None);
    }

    #[test]
    fn a_file_that_is_no_ring_is_refused() {
        let fd = memfd(c"gh0st-test").unwrap();
        // SAFETY: test fd.
        unsafe { libc::ftruncate(fd.as_raw_fd(), 4096) };
        assert!(Ring::open(fd).is_err());
    }
}
