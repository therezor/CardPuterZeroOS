//! The keyboard's evdev device, read raw. A child app opens the same device
//! and must see every key, as in stock, so the shell grabs it only while no
//! app is in front and some app is paused.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

pub const DEFAULT_DEVICE: &str = "/dev/input/by-path/platform-3f804000.i2c-event";

/// `struct input_event` on 64-bit Linux.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    sec: i64,
    usec: i64,
    kind: u16,
    code: u16,
    value: i32,
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_MSC: u16 = 4;
const MSC_SCAN: u16 = 4;
/// `_IOW('E', 0x90, int)`.
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
/// `EVIOCGKEY(KEY_BITS)`: `_IOC(_IOC_READ, 'E', 0x18, KEY_BITS)`.
const KEY_BITS: usize = 96;
const EVIOCGKEY: libc::c_ulong = 0x8000_0000 | ((KEY_BITS as libc::c_ulong) << 16) | 0x4518;
/// Scan-and-sync pairs that overflow any reader's queue. 64 did on the
/// device; the queue size is the driver's guess, so leave room.
const FLUSH_PAIRS: usize = 256;

pub struct Device {
    file: std::fs::File,
    pub path: PathBuf,
    /// Opened read-write, so `flush_readers` can write.
    writable: bool,
    grabbed: bool,
}

/// The keyboard: `$LV_LINUX_KEYBOARD_DEVICE`, the stock path, then any input
/// device whose name says TCA8418 or keypad.
pub fn find() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LV_LINUX_KEYBOARD_DEVICE").map(PathBuf::from) {
        return Some(p);
    }
    if Path::new(DEFAULT_DEVICE).exists() {
        return Some(DEFAULT_DEVICE.into());
    }
    let mut events: Vec<PathBuf> = std::fs::read_dir("/sys/class/input")
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    events.sort();
    events.into_iter().find_map(|p| {
        let name = std::fs::read_to_string(p.join("device/name"))
            .ok()?
            .to_lowercase();
        let event = p.file_name()?.to_str()?.to_string();
        (event.starts_with("event") && (name.contains("tca8418") || name.contains("keypad")))
            .then(|| PathBuf::from("/dev/input").join(event))
    })
}

impl Device {
    /// Read-write if allowed, else read-only.
    pub fn open(path: &Path) -> io::Result<Device> {
        use std::os::unix::fs::OpenOptionsExt;
        let open = |write: bool| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(path)
        };
        let (file, writable) = match open(true) {
            Ok(f) => (f, true),
            Err(_) => (open(false)?, false),
        };
        Ok(Device {
            file,
            path: path.to_path_buf(),
            writable,
            grabbed: false,
        })
    }

    /// Takes every key for the shell alone, or gives them back. False when
    /// that failed: taking fails with EBUSY while an app holds the grab.
    pub fn grab(&mut self, on: bool) -> bool {
        if on == self.grabbed {
            return true;
        }
        // SAFETY: EVIOCGRAB takes an int by value on a descriptor we own.
        if unsafe { libc::ioctl(self.fd(), EVIOCGRAB as _, on as libc::c_int) } == 0 {
            self.grabbed = on;
        }
        on == self.grabbed
    }

    /// Empties the queue of every reader that is not reading, such as a
    /// paused app, so it does not replay old keys when it runs again. Each
    /// reader gets scan events no app acts on; a full queue is replaced by
    /// `SYN_DROPPED` and the newest of them. Must run ungrabbed, or only
    /// the shell would get them.
    pub fn flush_readers(&self) {
        if !self.writable || self.grabbed {
            return;
        }
        let pair = [
            InputEvent {
                kind: EV_MSC,
                code: MSC_SCAN,
                ..InputEvent::default()
            },
            InputEvent {
                kind: EV_SYN,
                ..InputEvent::default()
            },
        ];
        let batch: Vec<InputEvent> = pair.iter().copied().cycle().take(64).collect();
        let bytes = std::mem::size_of_val(batch.as_slice());
        for _ in 0..FLUSH_PAIRS / 32 {
            // SAFETY: writing whole input_event structs from a local Vec.
            unsafe { libc::write(self.fd(), batch.as_ptr() as *const libc::c_void, bytes) };
        }
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// The keys down now, as the kernel tracks them. It tracks them even
    /// while another reader holds the grab, so this sees releases the shell
    /// missed.
    pub fn pressed(&self) -> Option<Vec<u16>> {
        let mut bits = [0u8; KEY_BITS];
        // SAFETY: EVIOCGKEY writes at most KEY_BITS bytes into a local array.
        let n = unsafe { libc::ioctl(self.fd(), EVIOCGKEY as _, bits.as_mut_ptr()) };
        (n >= 0).then(|| keys_in(&bits))
    }

    /// Key events ready now, as (code, value). Never blocks.
    pub fn read_keys(&self, out: &mut Vec<(u16, i32)>) {
        let mut buf = [InputEvent::default(); 32];
        loop {
            let size = std::mem::size_of_val(&buf);
            // SAFETY: reading whole input_event structs into a local array.
            let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr() as *mut libc::c_void, size) };
            if n <= 0 {
                return;
            }
            let count = n as usize / std::mem::size_of::<InputEvent>();
            out.extend(
                buf[..count]
                    .iter()
                    .filter(|e| e.kind == EV_KEY)
                    .map(|e| (e.code, e.value)),
            );
            if (n as usize) < size {
                return;
            }
        }
    }
}

/// The key codes set in an evdev key bitmap.
fn keys_in(bits: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, &b) in bits.iter().enumerate() {
        for bit in 0..8 {
            if b & (1 << bit) != 0 {
                out.push((i * 8 + bit) as u16);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_bitmap_reads_as_codes() {
        let mut bits = [0u8; KEY_BITS];
        bits[0] = 0b10; // ESC is 1
        bits[102 / 8] |= 1 << (102 % 8); // HOME
        assert_eq!(keys_in(&bits), vec![1, 102]);
        assert!(keys_in(&[0; KEY_BITS]).is_empty());
    }
}
