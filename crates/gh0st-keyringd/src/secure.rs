//! Memory for secrets and the process hardening the daemon starts with.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set once an `mlock` failed, so the daemon logs it once.
static UNLOCKED_PAGES: AtomicBool = AtomicBool::new(false);

pub fn mlock_failed() -> bool {
    UNLOCKED_PAGES.load(Ordering::Relaxed)
}

/// Fills `buf` from the kernel's CSPRNG.
pub fn random(buf: &mut [u8]) {
    #[cfg(target_os = "linux")]
    {
        let mut done = 0;
        while done < buf.len() {
            // SAFETY: getrandom writes into the rest of a live buffer.
            let n =
                unsafe { libc::getrandom(buf[done..].as_mut_ptr().cast(), buf.len() - done, 0) };
            if n > 0 {
                done += n as usize;
            } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                // No entropy source means no keys: stopping beats weak ones.
                std::process::abort();
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        // SAFETY: arc4random_buf fills a live buffer and cannot fail.
        unsafe { libc::arc4random_buf(buf.as_mut_ptr().cast(), buf.len()) };
    }
}

pub fn random_array<const N: usize>() -> [u8; N] {
    let mut a = [0u8; N];
    random(&mut a);
    a
}

/// Bytes on pages of their own: locked in RAM, left out of core dumps,
/// zeroed before they go back.
pub struct Locked {
    ptr: *mut u8,
    cap: usize,
    len: usize,
}

impl Locked {
    pub fn new(data: &[u8]) -> Locked {
        let page = page_size();
        let cap = data.len().max(1).div_ceil(page) * page;
        // SAFETY: an anonymous private mapping of `cap` bytes.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                cap,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            // Out of address space: as fatal as a failed allocation.
            std::process::abort();
        }
        // SAFETY: the mapping is ours and `cap` long.
        unsafe {
            if libc::mlock(ptr, cap) != 0 {
                UNLOCKED_PAGES.store(true, Ordering::Relaxed);
            }
            #[cfg(target_os = "linux")]
            libc::madvise(ptr, cap, libc::MADV_DONTDUMP);
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast(), data.len());
        }
        Locked {
            ptr: ptr.cast(),
            cap,
            len: data.len(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `len` bytes from `ptr` were written in `new`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Clone for Locked {
    fn clone(&self) -> Locked {
        Locked::new(self.bytes())
    }
}

impl std::fmt::Debug for Locked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Locked({} bytes)", self.len)
    }
}

impl PartialEq for Locked {
    fn eq(&self, o: &Locked) -> bool {
        self.bytes() == o.bytes()
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        // SAFETY: the mapping is ours; zero it, then hand it back.
        unsafe {
            gh0st_dbus::wipe(std::slice::from_raw_parts_mut(self.ptr, self.cap));
            libc::munlock(self.ptr.cast(), self.cap);
            libc::munmap(self.ptr.cast(), self.cap);
        }
    }
}

fn page_size() -> usize {
    // SAFETY: sysconf has no preconditions.
    let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if n > 0 { n as usize } else { 4096 }
}

/// No core dumps, no ptrace from our own UID, no /proc/<pid>/mem.
pub fn harden() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit and prctl with plain values.
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
        #[cfg(target_os = "linux")]
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_keeps_its_bytes() {
        let s = Locked::new(b"hunter2");
        assert_eq!(s.bytes(), b"hunter2");
        assert_eq!(s.clone(), s);
        assert_eq!(Locked::new(b"").bytes(), b"");
        let big = vec![7u8; 10_000];
        assert_eq!(Locked::new(&big).bytes(), &big[..]);
    }

    #[test]
    fn random_is_not_constant() {
        assert_ne!(random_array::<32>(), random_array::<32>());
    }
}
