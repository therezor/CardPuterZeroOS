//! What the status bar shows, read from sysfs and procfs, never by polling a
//! tool. The shell re-reads it on the minute tick and when netlink says a
//! link, address or power supply changed.

use std::os::fd::RawFd;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Net {
    None,
    /// Signal 0-4 bars.
    Wifi(u8),
    Ethernet,
    /// wlan0 is an access point. Shown over a cable, so it is never missed.
    Hotspot,
}

/// Bluetooth is on while the first controller is up, which is what BlueZ
/// powering it means. sysfs has no file for that; HCIGETDEVINFO asks the
/// kernel and needs no privilege.
#[cfg(target_os = "linux")]
pub fn bluetooth() -> bool {
    const AF_BLUETOOTH: libc::c_int = 31;
    const BTPROTO_HCI: libc::c_int = 1;
    /// `_IOR('H', 211, int)`.
    const HCIGETDEVINFO: u32 = 0x8004_48d3;
    const HCI_UP: u32 = 1;
    /// `struct hci_dev_info`: 92 bytes, `flags` at 16.
    #[repr(C)]
    struct DevInfo {
        dev_id: u16,
        name: [u8; 8],
        bdaddr: [u8; 6],
        flags: u32,
        rest: [u8; 80],
    }
    // SAFETY: a plain socket call; the fd is closed below.
    let fd = unsafe {
        libc::socket(
            AF_BLUETOOTH,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            BTPROTO_HCI,
        )
    };
    if fd < 0 {
        return false;
    }
    let mut info = DevInfo {
        dev_id: 0,
        name: [0; 8],
        bdaddr: [0; 6],
        flags: 0,
        rest: [0; 80],
    };
    // SAFETY: info is larger than the kernel's struct, which it fills.
    let ok = unsafe { libc::ioctl(fd, HCIGETDEVINFO as _, &mut info) } == 0;
    // SAFETY: fd is ours and open.
    unsafe { libc::close(fd) };
    ok && info.flags & HCI_UP != 0
}

#[cfg(not(target_os = "linux"))]
pub fn bluetooth() -> bool {
    false
}

/// The hotspot first, then a cable with link, then Wi-Fi, as on pt35.
pub fn network(sys_net: &Path, proc_wireless: &Path) -> Net {
    network_with(sys_net, proc_wireless, is_ap)
}

pub fn network_with(sys_net: &Path, proc_wireless: &Path, ap: impl Fn(&str) -> bool) -> Net {
    let Ok(rd) = std::fs::read_dir(sys_net) else {
        return Net::None;
    };
    let mut wifi_up = false;
    let mut cable = false;
    let mut hotspot = false;
    for e in rd.flatten() {
        let p = e.path();
        let up = std::fs::read_to_string(p.join("operstate"))
            .map(|s| s.trim() == "up")
            .unwrap_or(false);
        if !up || !p.join("device").exists() {
            continue;
        }
        if p.join("wireless").exists() {
            if e.file_name().to_str().is_some_and(&ap) {
                hotspot = true;
            } else {
                wifi_up = true;
            }
        } else {
            cable = true;
        }
    }
    if hotspot {
        Net::Hotspot
    } else if cable {
        Net::Ethernet
    } else if wifi_up {
        Net::Wifi(
            std::fs::read_to_string(proc_wireless)
                .ok()
                .map(|s| wifi_bars(&s))
                .unwrap_or(2),
        )
    } else {
        Net::None
    }
}

/// Whether `iface` is an access point, from the wireless extensions'
/// SIOCGIWMODE: one ioctl, no privilege, no tool to run.
#[cfg(target_os = "linux")]
pub fn is_ap(iface: &str) -> bool {
    /// `struct iwreq`: the name, then a 16-byte union whose first field is
    /// the mode.
    #[repr(C)]
    struct IwReq {
        name: [u8; 16],
        mode: u32,
        rest: [u8; 12],
    }
    const SIOCGIWMODE: u32 = 0x8B07;
    const IW_MODE_MASTER: u32 = 3;
    let bytes = iface.as_bytes();
    if bytes.len() >= 16 {
        return false;
    }
    let mut req = IwReq {
        name: [0; 16],
        mode: 0,
        rest: [0; 12],
    };
    req.name[..bytes.len()].copy_from_slice(bytes);
    // SAFETY: a plain socket call; the fd is closed below.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return false;
    }
    // SAFETY: req is an iwreq for the kernel to fill.
    let ok = unsafe { libc::ioctl(fd, SIOCGIWMODE as _, &mut req) } == 0;
    // SAFETY: fd is ours and open.
    unsafe { libc::close(fd) };
    ok && req.mode == IW_MODE_MASTER
}

#[cfg(not(target_os = "linux"))]
pub fn is_ap(_: &str) -> bool {
    false
}

/// The hotspot is up: wlan0 is an access point with link.
pub fn hotspot() -> bool {
    let up =
        std::fs::read_to_string("/sys/class/net/wlan0/operstate").is_ok_and(|s| s.trim() == "up");
    up && is_ap("wlan0")
}

/// Bars from `/proc/net/wireless` link quality, out of 70 on brcmfmac.
pub fn wifi_bars(proc_wireless: &str) -> u8 {
    let quality = proc_wireless
        .lines()
        .skip(2)
        .find_map(|l| {
            l.split_whitespace()
                .nth(2)?
                .trim_end_matches('.')
                .parse::<f32>()
                .ok()
        })
        .unwrap_or(0.0);
    match quality as u32 {
        0 => 0,
        1..=20 => 1,
        21..=40 => 2,
        41..=55 => 3,
        _ => 4,
    }
}

/// A netlink socket that wakes the loop on changes. Messages are drained and
/// not parsed: any message means "read sysfs again".
pub struct Watch {
    fd: RawFd,
}

impl Watch {
    /// Kernel uevents: battery, charger and hotplug changes.
    #[cfg(target_os = "linux")]
    pub fn uevents() -> std::io::Result<Watch> {
        Watch::open(libc::NETLINK_KOBJECT_UEVENT, 1)
    }

    /// Links going up or down, addresses coming and going.
    #[cfg(target_os = "linux")]
    pub fn links() -> std::io::Result<Watch> {
        const RTMGRP_LINK: u32 = 1;
        const RTMGRP_IPV4_IFADDR: u32 = 0x10;
        Watch::open(libc::NETLINK_ROUTE, RTMGRP_LINK | RTMGRP_IPV4_IFADDR)
    }

    /// Files added, removed or rewritten in `dirs`: the Store installing an
    /// app. Missing folders are skipped.
    #[cfg(target_os = "linux")]
    pub fn dirs(dirs: &[&Path]) -> std::io::Result<Watch> {
        use std::os::unix::ffi::OsStrExt;
        // SAFETY: inotify calls with NUL-terminated paths built here.
        unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mask = libc::IN_CREATE
                | libc::IN_DELETE
                | libc::IN_CLOSE_WRITE
                | libc::IN_MOVED_FROM
                | libc::IN_MOVED_TO;
            for d in dirs {
                let mut c = d.as_os_str().as_bytes().to_vec();
                c.push(0);
                libc::inotify_add_watch(fd, c.as_ptr() as *const libc::c_char, mask);
            }
            Ok(Watch { fd })
        }
    }

    /// Watches one more folder, on a watch made by `dirs`.
    #[cfg(target_os = "linux")]
    pub fn add(&self, dir: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let mut c = dir.as_os_str().as_bytes().to_vec();
        c.push(0);
        let mask = libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_FROM
            | libc::IN_MOVED_TO;
        // SAFETY: inotify_add_watch with a NUL-terminated path; adding a
        // folder already watched changes nothing.
        unsafe { libc::inotify_add_watch(self.fd, c.as_ptr() as *const libc::c_char, mask) };
    }

    #[cfg(target_os = "linux")]
    fn open(proto: libc::c_int, groups: u32) -> std::io::Result<Watch> {
        // SAFETY: plain socket and bind calls with a zeroed sockaddr_nl.
        unsafe {
            let fd = libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                proto,
            );
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut sa: libc::sockaddr_nl = std::mem::zeroed();
            sa.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            sa.nl_groups = groups;
            let len = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
            if libc::bind(fd, &sa as *const _ as *const libc::sockaddr, len) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            Ok(Watch { fd })
        }
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Empties the socket. True if anything arrived.
    pub fn drain(&self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        loop {
            // SAFETY: reading into a local buffer of its stated size.
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                return any;
            }
            any = true;
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: closing the descriptor this struct owns.
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIRELESS: &str = "Inter-| sta-|   Quality        |   Discarded packets\n face | tus | link level noise |  nwid  crypt\n wlan0: 0000   52.  -58.  -256        0      0\n";

    #[test]
    fn link_quality_becomes_bars() {
        assert_eq!(wifi_bars(WIRELESS), 3);
        assert_eq!(wifi_bars(""), 0);
    }

    #[test]
    fn a_cable_with_link_beats_wifi() {
        let root = std::env::temp_dir().join(format!("gh0st-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let iface = |name: &str, state: &str, wireless: bool| {
            let d = root.join(name);
            std::fs::create_dir_all(d.join("device")).unwrap();
            if wireless {
                std::fs::create_dir_all(d.join("wireless")).unwrap();
            }
            std::fs::write(d.join("operstate"), format!("{state}\n")).unwrap();
        };
        iface("wlan0", "up", true);
        iface("eth0", "down", false);
        let w = root.join("wireless");
        std::fs::write(&w, WIRELESS).unwrap();
        assert_eq!(network(&root, &w), Net::Wifi(3));
        iface("eth0", "up", false);
        assert_eq!(network(&root, &w), Net::Ethernet);
        let ap = |n: &str| n == "wlan0";
        assert_eq!(network_with(&root, &w, ap), Net::Hotspot, "over the cable");
        iface("eth0", "down", false);
        assert_eq!(network_with(&root, &w, ap), Net::Hotspot, "not Wi-Fi bars");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
