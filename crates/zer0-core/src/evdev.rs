//! The keyboard's evdev device, read raw and never grabbed: a child app opens
//! the same device and must see every key, as in stock.

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

const EV_KEY: u16 = 1;

pub struct Device {
    file: std::fs::File,
    pub path: PathBuf,
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
    pub fn open(path: &Path) -> io::Result<Device> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Device {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
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
