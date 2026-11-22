//! The device, read and set directly from sysfs and procfs. Nothing here
//! needs root. Each call reads once; none of them polls.

use gh0st_core::{backlight::Backlight, battery};
use std::path::Path;

const BACKLIGHT: &str = "/sys/class/backlight";
const POWER: &str = "/sys/class/power_supply";

/// Screen brightness, 0 to 100.
pub fn brightness() -> Option<u8> {
    let b = Backlight::find(Path::new(BACKLIGHT))?;
    Some(b.percent(b.get()?))
}

/// Sets the screen brightness, 1 to 100, and returns the raw sysfs value.
/// To keep it across boots, store that value as `brightness` in the
/// settings, as stock does.
pub fn set_brightness(percent: u8) -> std::io::Result<u32> {
    let b = Backlight::find(Path::new(BACKLIGHT)).ok_or(std::io::ErrorKind::NotFound)?;
    let raw = b.raw(percent.clamp(1, 100));
    b.set(raw)?;
    Ok(raw)
}

/// The battery right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Charge left and the full charge today, in mAh, if the gauge says.
    pub charge_mah: Option<u32>,
    pub full_mah: Option<u32>,
    /// The full charge when new, in mAh.
    pub design_mah: Option<u32>,
}

impl Battery {
    /// Full charge today as a share of the design charge, 0 to 100.
    pub fn health(&self) -> Option<u8> {
        let (full, design) = (self.full_mah?, self.design_mah?);
        (design > 0).then(|| (full * 100 / design).min(100) as u8)
    }
}

pub fn battery() -> Option<Battery> {
    let dir = battery::find(Path::new(POWER))?;
    let r = battery::sample(&dir)?;
    let mah = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()
            .map(|uah| (uah / 1000) as u32)
    };
    Some(Battery {
        percent: r.percent,
        charging: r.charging,
        charge_mah: mah("charge_now"),
        full_mah: mah("charge_full"),
        design_mah: mah("charge_full_design"),
    })
}

/// Memory in use and in total, in MB, as `free` counts it.
pub fn memory() -> Option<(u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| {
        info.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    Some(((total - available) / 1024, total / 1024))
}

/// Space used and in total on the filesystem that holds `path`, in MB.
/// The image keeps `/home` on its own partition, which fills the card, so
/// `/` alone shows only the system.
pub fn storage(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs fills a zeroed struct for a NUL-terminated path.
    let st = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(c.as_ptr(), &mut st) == 0).then_some(st)
    }?;
    let block = st.f_frsize as u64;
    let total = st.f_blocks as u64 * block;
    let free = st.f_bfree as u64 * block;
    Some(((total - free) >> 20, total >> 20))
}

/// The running kernel's release, as `uname -r` prints it.
pub fn kernel() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
}

/// The gh0st version this SDK belongs to.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_is_full_over_design() {
        let b = Battery {
            percent: 80,
            charging: false,
            charge_mah: Some(1300),
            full_mah: Some(1680),
            design_mah: Some(1750),
        };
        assert_eq!(b.health(), Some(96));
    }
}
