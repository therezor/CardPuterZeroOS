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
}

/// A cable with link wins over Wi-Fi, as on pt35.
pub fn network(sys_net: &Path, proc_wireless: &Path) -> Net {
    let Ok(rd) = std::fs::read_dir(sys_net) else {
        return Net::None;
    };
    let mut wifi_up = false;
    let mut cable = false;
    for e in rd.flatten() {
        let p = e.path();
        let up = std::fs::read_to_string(p.join("operstate"))
            .map(|s| s.trim() == "up")
            .unwrap_or(false);
        if !up || !p.join("device").exists() {
            continue;
        }
        if p.join("wireless").exists() {
            wifi_up = true;
        } else {
            cable = true;
        }
    }
    if cable {
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
        std::fs::remove_dir_all(&root).unwrap();
    }
}
