//! System files the root helper writes and Settings reads: the Pi's boot
//! files, the CPU governor, the hostname, the zram swap size and
//! NetworkManager's privacy settings. Pure text
//! in and out, so both sides use the same parser and the transforms test
//! on any host.

use std::path::{Path, PathBuf};

pub const CONFIG: &str = "config.txt";
pub const CMDLINE: &str = "cmdline.txt";
/// 16 boots the cut-down firmware (no camera or video); past 128 under KMS
/// only takes RAM from Linux.
pub const GPU_MEMS: [u32; 7] = [16, 32, 64, 96, 128, 192, 256];
pub const COUNTRIES: &str = "/usr/share/zoneinfo/iso3166.tab";
const REGDOM: &str = "cfg80211.ieee80211_regdom=";

/// The boot partition. `GH0ST_BOOT_DIR` points a host run at a copy.
pub fn boot_dir() -> PathBuf {
    std::env::var_os("GH0ST_BOOT_DIR")
        .map_or_else(|| PathBuf::from("/boot/firmware"), PathBuf::from)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuMem {
    Set(u32),
    /// No line: the firmware default.
    #[default]
    Absent,
    /// Set in a way we won't edit: a `gpu_mem_512` line, a filtered
    /// section, two lines, or a value that isn't a number.
    Unsupported,
}

/// Lines with their endings, and whether each applies to every board.
/// `[all]` returns to unconditional; any other filter makes what follows
/// conditional. `include` is not followed.
fn scoped(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut all = true;
    text.split_inclusive('\n').map(move |line| {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            all = t.eq_ignore_ascii_case("[all]");
        }
        (line, all)
    })
}

/// The key and value of an active `key=value` line.
fn entry(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    if t.starts_with('#') {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    Some((k.trim(), v.trim()))
}

pub fn gpu_mem(config: &str) -> GpuMem {
    let mut found = GpuMem::Absent;
    for (line, all) in scoped(config) {
        let Some((k, v)) = entry(line) else { continue };
        if k.starts_with("gpu_mem_") {
            return GpuMem::Unsupported;
        }
        if k != "gpu_mem" {
            continue;
        }
        match (found, all, v.parse::<u32>()) {
            (GpuMem::Absent, true, Ok(n)) if v.bytes().all(|b| b.is_ascii_digit()) => {
                found = GpuMem::Set(n)
            }
            _ => return GpuMem::Unsupported,
        }
    }
    found
}

/// Changes only the digits of the one `gpu_mem` line, or adds the line at
/// the top. Every other byte stays.
pub fn set_gpu_mem(config: &str, mb: u32) -> Result<String, &'static str> {
    if !GPU_MEMS.contains(&mb) {
        return Err("unsupported gpu_mem");
    }
    match gpu_mem(config) {
        GpuMem::Unsupported => Err("gpu_mem is set by hand in config.txt"),
        GpuMem::Absent => {
            let eol = if config.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            Ok(format!("gpu_mem={mb}{eol}{config}"))
        }
        GpuMem::Set(_) => Ok(config
            .split_inclusive('\n')
            .map(|line| match entry(line) {
                Some(("gpu_mem", v)) => {
                    let eq = line.find('=').expect("an entry has '='") + 1;
                    let at = eq + line[eq..].find(v).expect("value is in its line");
                    format!("{}{mb}{}", &line[..at], &line[at + v.len()..])
                }
                _ => line.to_string(),
            })
            .collect()),
    }
}

fn valid_country(cc: &str) -> bool {
    cc.len() == 2 && cc.bytes().all(|b| b.is_ascii_uppercase())
}

/// The Wi-Fi regulatory domain on a kernel command line. Works for
/// `cmdline.txt` and `/proc/cmdline` alike.
pub fn regdom(cmdline: &str) -> Option<String> {
    cmdline
        .split_whitespace()
        .filter_map(|t| t.strip_prefix(REGDOM))
        .next_back()
        .map(String::from)
}

/// Replaces the one regdom value in place, or appends the token.
pub fn set_regdom(cmdline: &str, cc: &str) -> Result<String, &'static str> {
    if !valid_country(cc) {
        return Err("not a country code");
    }
    let body = cmdline.trim_end_matches(['\n', '\r']);
    if body.contains('\n') || !body.contains("root=") {
        return Err("cmdline.txt is not one kernel command line");
    }
    let end = &cmdline[body.len()..];
    let mut at = body
        .match_indices(REGDOM)
        .filter(|(i, _)| *i == 0 || body.as_bytes()[i - 1].is_ascii_whitespace());
    match (at.next(), at.next()) {
        (Some(_), Some(_)) => Err("cmdline.txt sets the Wi-Fi country twice"),
        (Some((i, _)), None) => {
            let start = i + REGDOM.len();
            let stop = body[start..]
                .find(char::is_whitespace)
                .map_or(body.len(), |n| start + n);
            Ok(format!("{}{cc}{}{end}", &body[..start], &body[stop..]))
        }
        (None, _) => Ok(format!("{body} {REGDOM}{cc}{end}")),
    }
}

/// Country codes and names from `iso3166.tab`, sorted by name.
pub fn countries(tab: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = tab
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (cc, name) = l.split_once('\t')?;
            valid_country(cc).then(|| (cc.to_string(), name.trim().to_string()))
        })
        .collect();
    // By name as a person reads it: "US minor outlying islands" after
    // "Uganda", not before.
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out
}

/// One label of RFC 1123. Upper case is fine: stock names it `RZR-ZERO`.
pub fn valid_hostname(s: &str) -> bool {
    (1..=63).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Points every `127.0.1.1` line at `name`, or adds one, so `sudo` and
/// friends can still resolve the host.
pub fn hosts_with_name(hosts: &str, name: &str) -> String {
    let mut seen = false;
    let mut out: String = hosts
        .split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\n', '\r']);
            let rest = body.strip_prefix("127.0.1.1");
            match rest {
                Some(r) if r.starts_with([' ', '\t']) => {
                    seen = true;
                    let gap = r.len() - r.trim_start().len();
                    format!("127.0.1.1{}{name}{}", &r[..gap], &line[body.len()..])
                }
                _ => line.to_string(),
            }
        })
        .collect();
    if !seen {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("127.0.1.1\t{name}\n"));
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuMode {
    Balanced,
    PowerSaver,
    Performance,
}

impl CpuMode {
    pub const ALL: [CpuMode; 3] = [CpuMode::Balanced, CpuMode::PowerSaver, CpuMode::Performance];

    pub fn label(self) -> &'static str {
        match self {
            CpuMode::Balanced => "Balanced",
            CpuMode::PowerSaver => "Power Saver",
            CpuMode::Performance => "Performance",
        }
    }

    /// The helper's word for it.
    pub fn verb(self) -> &'static str {
        match self {
            CpuMode::Balanced => "balanced",
            CpuMode::PowerSaver => "power-saver",
            CpuMode::Performance => "performance",
        }
    }

    pub fn governor(self) -> &'static str {
        match self {
            CpuMode::Balanced => "schedutil",
            CpuMode::PowerSaver => "powersave",
            CpuMode::Performance => "performance",
        }
    }

    pub fn from_verb(s: &str) -> Option<CpuMode> {
        CpuMode::ALL.into_iter().find(|m| m.verb() == s)
    }

    pub fn from_governor(s: &str) -> Option<CpuMode> {
        CpuMode::ALL.into_iter().find(|m| m.governor() == s.trim())
    }
}

/// Our zram size for systemd-zram-generator. 90 sorts after the image's
/// `50-gh0st.conf` and stock's `20-rpi-swap-zram0-ctrl.conf`, so ours win.
pub const ZRAM_DROPIN: &str = "/etc/systemd/zram-generator.conf.d/90-gh0st-swap-size.conf";
/// The compression, on both systems: rpi-swap leaves it to the kernel.
pub const ZRAM_COMP_DROPIN: &str =
    "/etc/systemd/zram-generator.conf.d/90-gh0st-swap-compression.conf";
/// After the image's `50-gh0st.conf`, which sets 100.
pub const SWAPPINESS_DROPIN: &str = "/etc/sysctl.d/90-gh0st-swap.conf";
/// The same for stock's rpi-swap, which writes zram-generator's file itself
/// at boot from its own config.
pub const RPI_SWAP_DROPIN: &str = "/etc/rpi/swap.conf.d/90-gh0st-swap.conf";
const RPI_SWAP_GENERATOR: &str = "/usr/lib/systemd/system-generators/rpi-swap-generator";
/// The running zram swap, in bytes. World-readable.
pub const ZRAM_DISKSIZE: &str = "/sys/block/zram0/disksize";
/// What the kernel offers, the one in use in brackets: `lz4 [zstd]`.
pub const ZRAM_COMP: &str = "/sys/block/zram0/comp_algorithm";
pub const SWAPPINESS_NOW: &str = "/proc/sys/vm/swappiness";
/// zstd packs the most; lz4 and lzo-rle are faster and pack less.
pub const COMPRESSIONS: [&str; 3] = ["zstd", "lz4", "lzo-rle"];
/// The picks Settings offers. The kernel takes 0 to 200: past 100 it
/// prefers swapping app memory to dropping file cache.
pub const SWAPPINESS: [u32; 5] = [10, 60, 100, 150, 200];
pub const SWAPPINESS_MAX: u32 = 200;
const HEADER: &str = "# Settings > System > Advanced > Swap.\n";
/// Without a readable `MemTotal`.
const SWAP_MAX_FALLBACK: u32 = 1024;

/// Who sets up zram swap at boot. Both resize only then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapBackend {
    /// Stock Raspberry Pi OS.
    RpiSwap,
    /// Gh0st OS: zram-generator alone.
    ZramGenerator,
}

pub fn swap_backend() -> SwapBackend {
    if std::path::Path::new(RPI_SWAP_GENERATOR).exists() {
        SwapBackend::RpiSwap
    } else {
        SwapBackend::ZramGenerator
    }
}

impl SwapBackend {
    pub fn dropin(self) -> &'static str {
        match self {
            SwapBackend::RpiSwap => RPI_SWAP_DROPIN,
            SwapBackend::ZramGenerator => ZRAM_DROPIN,
        }
    }

    /// The whole drop-in for `mb` of swap; 0 is none. zram-generator drops
    /// a device of size 0. rpi-swap needs `Mechanism=none`, or its swap unit
    /// fails on an empty device.
    pub fn conf(self, mb: u32) -> String {
        let body = match (self, mb) {
            (SwapBackend::RpiSwap, 0) => "[Main]\nMechanism=none\n".to_string(),
            (SwapBackend::RpiSwap, n) => format!("[Zram]\nFixedSizeMiB={n}\n"),
            (SwapBackend::ZramGenerator, n) => format!("[zram0]\nzram-size = {n}\n"),
        };
        format!("{HEADER}{body}")
    }
}

pub fn comp_conf(alg: &str) -> String {
    format!("{HEADER}[zram0]\ncompression-algorithm = {alg}\n")
}

/// The compression a drop-in sets.
pub fn comp_from_conf(text: &str) -> Option<String> {
    text.lines()
        .filter_map(entry)
        .filter(|(k, _)| *k == "compression-algorithm")
        .map(|(_, v)| v.to_string())
        .next_back()
}

/// The algorithms in `comp_algorithm`, and the one in use.
pub fn comp_algorithms(text: &str) -> (Vec<String>, Option<String>) {
    let mut now = None;
    let all = text
        .split_whitespace()
        .map(
            |w| match w.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
                Some(cur) => {
                    now = Some(cur.to_string());
                    cur.to_string()
                }
                None => w.to_string(),
            },
        )
        .collect();
    (all, now)
}

pub fn swappiness_conf(n: u32) -> String {
    format!("{HEADER}vm.swappiness = {n}\n")
}

/// The swap size in MB a drop-in of either kind sets, 0 for none.
pub fn swap_conf(text: &str) -> Option<u32> {
    let mut found = None;
    for line in text.lines() {
        match entry(line) {
            Some(("Mechanism", "none")) => return Some(0),
            Some(("zram-size" | "FixedSizeMiB", v)) if v.bytes().all(|b| b.is_ascii_digit()) => {
                found = v.parse().ok()
            }
            _ => {}
        }
    }
    found
}

/// `/sys/block/zram0/disksize` in MB.
pub fn swap_running(disksize: &str) -> u32 {
    (disksize.trim().parse::<u64>().unwrap_or(0) >> 20) as u32
}

/// The largest swap allowed: twice RAM, past which rpi-swap warns that poor
/// compression could fill memory.
pub fn swap_max(meminfo: &str) -> u32 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u32>().ok())
        .map_or(SWAP_MAX_FALLBACK, |kb| kb / 1024 * 2)
}

// NetworkManager's privacy settings. The image ships its defaults in
// /usr/lib/NetworkManager/conf.d/50-gh0st.conf; Settings and setup write
// one whole file per setting in /etc, which NetworkManager reads after it.

/// The built-in USB Ethernet chip on the board's hub. Left awake, it keeps
/// the hub and the USB host up at about 8,100 IRQ/s: 93 mA, measured.
pub const ETH_USB_ID: (&str, &str) = ("0fe6", "9900");
/// Settings' USB power saving, after the image's `60-gh0st.rules`.
pub const USB_POWER_RULE: &str = "/etc/udev/rules.d/61-gh0st-usb-power.rules";

/// The USB devices with this vendor and product id under `devices`
/// (`/sys/bus/usb/devices`).
pub fn usb_devices(devices: &Path, id: (&str, &str)) -> Vec<PathBuf> {
    let read = |p: PathBuf| std::fs::read_to_string(p).map(|s| s.trim().to_string());
    let mut out: Vec<PathBuf> = std::fs::read_dir(devices)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|d| {
            read(d.join("idVendor")).is_ok_and(|v| v == id.0)
                && read(d.join("idProduct")).is_ok_and(|v| v == id.1)
        })
        .collect();
    out.sort();
    out
}

/// Whether the built-in Ethernet chip may suspend: `power/control` reads
/// `auto`. None without the chip.
pub fn usb_power(devices: &Path) -> Option<bool> {
    let d = usb_devices(devices, ETH_USB_ID).into_iter().next()?;
    let c = std::fs::read_to_string(d.join("power/control")).ok()?;
    Some(c.trim() == "auto")
}

/// The udev rule that keeps the choice across a restart.
pub fn usb_power_rule(save: bool) -> String {
    format!(
        "# Settings > System > Advanced > USB power saving.\n\
         ACTION==\"add\", SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"{}\", \
         ATTR{{idProduct}}==\"{}\", TEST==\"power/control\", \
         ATTR{{power/control}}=\"{}\"\n",
        ETH_USB_ID.0,
        ETH_USB_ID.1,
        if save { "auto" } else { "on" }
    )
}

pub const NM_HOSTNAME_DROPIN: &str = "/etc/NetworkManager/conf.d/90-gh0st-dhcp-hostname.conf";
pub const NM_IP6_PRIVACY_DROPIN: &str = "/etc/NetworkManager/conf.d/90-gh0st-ip6-privacy.conf";
pub const NM_POWERSAVE_DROPIN: &str = "/etc/NetworkManager/conf.d/90-gh0st-wifi-powersave.conf";
pub const NM_DNS_DROPIN: &str = "/etc/NetworkManager/conf.d/90-gh0st-dns.conf";
const NM_HEADER: &str = "# Settings > Privacy.\n";

/// Whether DHCP tells the network this device's name.
pub fn dhcp_hostname_conf(send: bool) -> String {
    format!(
        "{NM_HEADER}[connection]\nipv4.dhcp-send-hostname={send}\nipv6.dhcp-send-hostname={send}\n"
    )
}

/// IPv6 temporary addresses, preferred over the stable one.
pub fn ip6_privacy_conf(on: bool) -> String {
    let v = if on { 2 } else { 0 };
    format!("{NM_HEADER}[connection]\nipv6.ip6-privacy={v}\n")
}

/// Name servers for every connection, picked by name in Settings: (verb,
/// label, servers). Neither logs who asks, by their own policies.
pub const DNS_PRESETS: [(&str, &str, &str); 2] = [
    (
        "cloudflare",
        "Cloudflare",
        "1.1.1.1,1.0.0.1,2606:4700:4700::1111",
    ),
    ("quad9", "Quad9", "9.9.9.9,149.112.112.112,2620:fe::fe"),
];

/// glibc reads three name servers (`MAXNS`).
pub const DNS_MAX: usize = 3;

/// A list of name servers, split on commas and spaces, each an IPv4 or
/// IPv6 address, written back the way the address prints. None if any
/// is not an address, or there are none or too many.
pub fn dns_servers(list: &str) -> Option<String> {
    let mut out = Vec::new();
    for s in list.split([',', ' ']).filter(|s| !s.is_empty()) {
        out.push(s.parse::<std::net::IpAddr>().ok()?.to_string());
    }
    (!out.is_empty() && out.len() <= DNS_MAX).then(|| out.join(","))
}

/// Name servers for every connection. NetworkManager's global DNS takes
/// the place of what each network offers, and of a server a profile names.
pub fn dns_conf(servers: &str) -> String {
    format!("# Settings > Network > DNS.\n[global-dns-domain-*]\nservers={servers}\n")
}

/// The preset these servers are, by label.
pub fn dns_preset(servers: &str) -> Option<&'static str> {
    DNS_PRESETS
        .iter()
        .find(|(_, _, s)| *s == servers)
        .map(|(_, label, _)| *label)
}

/// Wi-Fi power saving for every connection: 3 on, 2 off. It applies from
/// the next connection; the helper sets the running one with iw.
pub fn wifi_powersave_conf(on: bool) -> String {
    let v = if on { 3 } else { 2 };
    format!("# Settings > Wi-Fi.\n[connection]\nwifi.powersave={v}\n")
}

/// NetworkManager's config files in the order it reads them; a later value
/// wins.
pub const NM_CONF_DIRS: [&str; 3] = [
    "/usr/lib/NetworkManager/conf.d",
    "/run/NetworkManager/conf.d",
    "/etc/NetworkManager/conf.d",
];
pub const NM_MAIN_CONF: &str = "/etc/NetworkManager/NetworkManager.conf";

/// The conf.d files NetworkManager reads, in its order: by file name across
/// all of [`NM_CONF_DIRS`], a later directory's file shadowing one of the
/// same name (NetworkManager.conf(5)). `files` holds each directory's `.conf`
/// names, in `NM_CONF_DIRS` order.
pub fn nm_conf_order(files: &[Vec<String>]) -> Vec<std::path::PathBuf> {
    let mut by_name: std::collections::BTreeMap<&str, usize> = Default::default();
    for (dir, names) in files.iter().enumerate() {
        for n in names {
            by_name.insert(n, dir);
        }
    }
    by_name
        .into_iter()
        .filter_map(|(n, dir)| Some(std::path::Path::new(NM_CONF_DIRS.get(dir)?).join(n)))
        .collect()
}

/// The privacy settings NetworkManager runs with, from its config files'
/// text in read order. Unset ones take NetworkManager's own defaults: the
/// name sent, no IPv6 temporary addresses. The private MAC addresses are
/// the image's alone (`50-gh0st.conf`), so they are not read here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NmPrivacy {
    pub send_hostname: bool,
    pub ip6_privacy: bool,
    /// Not privacy, but read from the same files: Wi-Fi power saving, on
    /// unless a file says 2 (brcmfmac turns it on by itself).
    pub wifi_powersave: bool,
    /// The global DNS servers, comma-separated; empty takes each network's.
    pub dns: String,
}

pub fn nm_privacy(files: &[String]) -> NmPrivacy {
    let (mut v4_name, mut ip6) = (String::new(), String::new());
    let (mut powersave, mut dns) = (String::new(), String::new());
    for text in files {
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name;
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim().to_string();
            match (section, k.trim()) {
                ("connection", "ipv4.dhcp-send-hostname") => v4_name = v,
                ("connection", "ipv6.ip6-privacy") => ip6 = v,
                ("connection", "wifi.powersave") => powersave = v,
                ("global-dns-domain-*", "servers") => dns = v,
                _ => {}
            }
        }
    }
    let yes = |v: &str, unset: bool| match v {
        "" => unset,
        _ => matches!(v, "yes" | "true" | "1" | "on"),
    };
    NmPrivacy {
        send_hostname: yes(&v4_name, true),
        ip6_privacy: matches!(ip6.as_str(), "1" | "2"),
        wifi_powersave: powersave != "2",
        dns,
    }
}

/// The tz database's zones for each country.
pub const ZONE_TAB: &str = "/usr/share/zoneinfo/zone.tab";

/// The zones `zone.tab` lists for country `cc`, in its order.
pub fn zones_for(tab: &str, cc: &str) -> Vec<String> {
    tab.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split('\t');
            let code = f.next()?;
            let zone = f.nth(1)?;
            (code == cc).then(|| zone.to_string())
        })
        .collect()
}

/// A user or a group already has `name`, in `/etc/passwd` and `/etc/group`
/// text: `useradd -U` would fail on it.
pub fn name_taken(passwd: &str, group: &str, name: &str) -> bool {
    let has = |text: &str| text.lines().any(|l| l.split(':').next() == Some(name));
    has(passwd) || has(group)
}

/// A login name `useradd` takes and setup allows: a lowercase letter, then
/// up to 31 lowercase letters, digits, `-` and `_`.
pub fn valid_user_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

#[cfg(test)]
mod tests {
    #[test]
    fn usb_power_reads_the_chip_and_writes_its_rule() {
        let root = std::env::temp_dir().join(format!("gh0st-usb-{}", std::process::id()));
        let dev = root.join("1-1.1");
        std::fs::create_dir_all(dev.join("power")).unwrap();
        assert_eq!(super::usb_power(&root), None, "no chip");
        std::fs::write(dev.join("idVendor"), "0fe6\n").unwrap();
        std::fs::write(dev.join("idProduct"), "9900\n").unwrap();
        std::fs::write(dev.join("power/control"), "on\n").unwrap();
        assert_eq!(super::usb_power(&root), Some(false));
        std::fs::write(dev.join("power/control"), "auto\n").unwrap();
        assert_eq!(super::usb_power(&root), Some(true));
        std::fs::remove_dir_all(&root).unwrap();
        let rule = super::usb_power_rule(true);
        assert!(rule.contains(r#"ATTR{idVendor}=="0fe6""#), "{rule}");
        assert!(rule.contains(r#"ATTR{power/control}="auto""#), "{rule}");
        assert!(super::usb_power_rule(false).contains(r#"ATTR{power/control}="on""#));
    }

    use super::*;

    #[test]
    fn swap_drop_ins_read_back() {
        for b in [SwapBackend::RpiSwap, SwapBackend::ZramGenerator] {
            for mb in [0, 256] {
                assert_eq!(swap_conf(&b.conf(mb)), Some(mb), "{b:?} {mb}");
            }
        }
        assert!(SwapBackend::RpiSwap.conf(0).contains("Mechanism=none"));
        assert!(SwapBackend::ZramGenerator.conf(0).contains("zram-size = 0"));
        assert_eq!(swap_conf("#zram-size=64\n"), None);
        assert_eq!(swap_conf("[zram0]\nzram-size = ram / 2\n"), None);
    }

    #[test]
    fn compression_reads_back_and_from_the_kernel() {
        for alg in COMPRESSIONS {
            assert_eq!(comp_from_conf(&comp_conf(alg)).as_deref(), Some(alg));
        }
        assert_eq!(comp_from_conf("[zram0]\nzram-size = 64\n"), None);
        let (all, now) = comp_algorithms("lzo-rle lzo lz4 [zstd] \n");
        assert_eq!(all, ["lzo-rle", "lzo", "lz4", "zstd"]);
        assert_eq!(now.as_deref(), Some("zstd"));
        assert_eq!(comp_algorithms("").1, None);
    }

    #[test]
    fn swappiness_is_a_sysctl_line() {
        assert!(swappiness_conf(150).ends_with("\nvm.swappiness = 150\n"));
        assert!(SWAPPINESS.iter().all(|&n| n <= SWAPPINESS_MAX));
    }

    #[test]
    fn stock_rpi_swap_output_parses() {
        // /run/systemd/zram-generator.conf.d/20-rpi-swap-zram0-ctrl.conf, 2026-11-21.
        let run = "[zram0]\nhost-memory-limit=none\nfs-type=swap\nzram-size=414\n\
                   writeback-device=/dev/disk/by-backingfile/var-swap\n";
        assert_eq!(swap_conf(run), Some(414));
    }

    #[test]
    fn swap_sizes_are_mb() {
        assert_eq!(swap_running("434110464\n"), 414);
        assert_eq!(swap_running(""), 0);
        let meminfo = "MemTotal:         424752 kB\nMemFree:           36000 kB\n";
        assert_eq!(swap_max(meminfo), 828);
        assert_eq!(swap_max(""), SWAP_MAX_FALLBACK);
    }

    /// The dev device's file, read 2026-11-21.
    const DEVICE: &str = include_str!("testdata/config.txt");

    #[test]
    fn the_device_file_round_trips_byte_for_byte() {
        assert_eq!(gpu_mem(DEVICE), GpuMem::Set(128));
        let low = set_gpu_mem(DEVICE, 64).unwrap();
        assert_eq!(gpu_mem(&low), GpuMem::Set(64));
        assert_eq!(low.len(), DEVICE.len() - 1);
        assert_eq!(low.replacen("gpu_mem=64", "gpu_mem=128", 1), DEVICE);
        assert_eq!(set_gpu_mem(&low, 128).unwrap(), DEVICE);
    }

    #[test]
    fn the_panel_overlay_keeps_its_place() {
        let low = set_gpu_mem(DEVICE, 64).unwrap();
        let lines: Vec<&str> = low.lines().collect();
        let v5 = lines
            .iter()
            .position(|l| *l == "dtoverlay=cardputerzero-v5-overlay");
        let te = lines.iter().position(|l| *l == "dtoverlay=gh0st-panel-te");
        assert_eq!(te, v5.map(|i| i + 1));
    }

    #[test]
    fn a_missing_line_goes_on_top() {
        let text = "arm_64bit=1\r\n[all]\r\n";
        let out = set_gpu_mem(text, 64).unwrap();
        assert_eq!(out, "gpu_mem=64\r\narm_64bit=1\r\n[all]\r\n");
        assert_eq!(set_gpu_mem("", 128).unwrap(), "gpu_mem=128\n");
    }

    #[test]
    fn hand_set_memory_is_left_alone() {
        for text in [
            "gpu_mem=128\ngpu_mem_512=96\n",
            "[cm4]\ngpu_mem=64\n",
            "gpu_mem=128\n[all]\ngpu_mem=64\n",
            "gpu_mem=lots\n",
        ] {
            assert_eq!(gpu_mem(text), GpuMem::Unsupported, "{text:?}");
            assert!(set_gpu_mem(text, 64).is_err());
        }
        assert_eq!(
            gpu_mem("#gpu_mem=16\n[pi4]\n[all]\ngpu_mem=96\n"),
            GpuMem::Set(96)
        );
        assert!(
            set_gpu_mem("gpu_mem=128\n", 48).is_err(),
            "only the listed sizes"
        );
    }

    #[test]
    fn crlf_endings_survive() {
        let out = set_gpu_mem("a=1\r\ngpu_mem=128\r\nb=2\r\n", 64).unwrap();
        assert_eq!(out, "a=1\r\ngpu_mem=64\r\nb=2\r\n");
    }

    const CMDLINE_DEV: &str = "console=tty1 root=PARTUUID=7b2299ea-02 rootfstype=ext4 fsck.repair=yes rootwait quiet fbcon=map:off cfg80211.ieee80211_regdom=AE\n";

    #[test]
    fn the_country_changes_in_place() {
        assert_eq!(regdom(CMDLINE_DEV).as_deref(), Some("AE"));
        let out = set_regdom(CMDLINE_DEV, "GB").unwrap();
        assert_eq!(out, CMDLINE_DEV.replace("=AE", "=GB"));
        assert_eq!(out.len(), CMDLINE_DEV.len());
    }

    #[test]
    fn a_missing_country_is_appended() {
        let out = set_regdom("root=/dev/sda2 quiet\n", "UA").unwrap();
        assert_eq!(out, "root=/dev/sda2 quiet cfg80211.ieee80211_regdom=UA\n");
        let mid = set_regdom("cfg80211.ieee80211_regdom=AE root=x", "US").unwrap();
        assert_eq!(mid, "cfg80211.ieee80211_regdom=US root=x");
    }

    #[test]
    fn odd_command_lines_are_refused() {
        assert!(set_regdom(CMDLINE_DEV, "gb").is_err());
        assert!(set_regdom(CMDLINE_DEV, "USA").is_err());
        assert!(set_regdom("quiet\n", "GB").is_err(), "no root=");
        assert!(set_regdom("root=x\nquiet\n", "GB").is_err(), "two lines");
        let twice = "root=x cfg80211.ieee80211_regdom=AE cfg80211.ieee80211_regdom=GB";
        assert!(set_regdom(twice, "US").is_err());
        assert_eq!(regdom(twice).as_deref(), Some("GB"), "the last one counts");
    }

    #[test]
    fn countries_sort_by_name() {
        let tab = "# comment\nGB\tBritain (UK)\nAE\tUnited Arab Emirates\nAD\tAndorra\nxx\tbad\n";
        let c = countries(tab);
        let names: Vec<&str> = c.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["Andorra", "Britain (UK)", "United Arab Emirates"]);
    }

    #[test]
    fn hostnames_are_one_label() {
        for ok in ["RZR-ZERO", "cardputer", "a", "x1-2"] {
            assert!(valid_hostname(ok), "{ok}");
        }
        let long = "a".repeat(64);
        for bad in ["", "-x", "x-", "a.b", "a b", "caf\u{e9}", long.as_str()] {
            assert!(!valid_hostname(bad), "{bad}");
        }
    }

    #[test]
    fn hosts_follow_the_name() {
        let device = "127.0.0.1\tlocalhost\n127.0.1.1\t\tcardputerzero\n127.0.1.1 RZR-ZERO\n";
        assert_eq!(
            hosts_with_name(device, "ghost"),
            "127.0.0.1\tlocalhost\n127.0.1.1\t\tghost\n127.0.1.1 ghost\n"
        );
        assert_eq!(
            hosts_with_name("127.0.0.1 localhost", "ghost"),
            "127.0.0.1 localhost\n127.0.1.1\tghost\n"
        );
    }

    #[test]
    fn cpu_modes_map_both_ways() {
        for m in CpuMode::ALL {
            assert_eq!(CpuMode::from_verb(m.verb()), Some(m));
            assert_eq!(CpuMode::from_governor(m.governor()), Some(m));
        }
        assert_eq!(CpuMode::from_governor("ondemand\n"), None);
    }
    #[test]
    fn the_privacy_files_say_what_they_set() {
        let h = dhcp_hostname_conf(false);
        assert!(h.contains("ipv4.dhcp-send-hostname=false\n"));
        assert!(h.contains("ipv6.dhcp-send-hostname=false\n"));
        assert!(ip6_privacy_conf(true).contains("ipv6.ip6-privacy=2\n"));
        assert!(ip6_privacy_conf(false).contains("ipv6.ip6-privacy=0\n"));
    }

    #[test]
    fn dns_servers_are_addresses_and_read_back() {
        assert_eq!(
            dns_servers(" 1.1.1.1, 2606:4700:4700::1111 ").as_deref(),
            Some("1.1.1.1,2606:4700:4700::1111")
        );
        for bad in [
            "",
            "1.1.1",
            "dns.example",
            "1.1.1.1;x",
            "1.1.1.1,2.2.2.2,3.3.3.3,4.4.4.4",
        ] {
            assert_eq!(dns_servers(bad), None, "{bad}");
        }
        for (_, label, servers) in DNS_PRESETS {
            assert_eq!(dns_servers(servers).as_deref(), Some(servers));
            assert_eq!(dns_preset(servers), Some(label));
        }
        let p = nm_privacy(&[dns_conf("9.9.9.9")]);
        assert_eq!(p.dns, "9.9.9.9");
        assert!(dns_conf("9.9.9.9").contains("[global-dns-domain-*]\nservers=9.9.9.9\n"));
    }

    #[test]
    fn user_names_useradd_takes() {
        for ok in ["alex", "a", "a-b_c9", &"a".repeat(32)] {
            assert!(valid_user_name(ok), "{ok}");
        }
        for bad in [
            "",
            "Alex",
            "9lives",
            "-x",
            "_x",
            "a.b",
            "a b",
            "al:ex",
            &"a".repeat(33),
        ] {
            assert!(!valid_user_name(bad), "{bad}");
        }
    }
    #[test]
    fn nm_conf_files_sort_by_name_and_etc_shadows() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let order = nm_conf_order(&[
            names(&["20-gh0st.conf", "90-mac.conf"]),
            names(&[]),
            names(&["10-user.conf", "90-mac.conf"]),
        ]);
        let shown: Vec<String> = order.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            shown,
            [
                "/etc/NetworkManager/conf.d/10-user.conf",
                "/usr/lib/NetworkManager/conf.d/20-gh0st.conf",
                "/etc/NetworkManager/conf.d/90-mac.conf",
            ]
        );
    }

    #[test]
    fn nm_privacy_reads_the_last_value_and_defaults_the_rest() {
        let image = "[device]\nwifi.scan-rand-mac-address=yes\n\n[connection]\n\
                     wifi.cloned-mac-address=stable-ssid\nipv4.dhcp-send-hostname=false\n\
                     ipv6.ip6-privacy=2\n";
        let p = nm_privacy(&[image.into()]);
        assert_eq!(
            p,
            NmPrivacy {
                send_hostname: false,
                ip6_privacy: true,
                wifi_powersave: true,
                dns: String::new(),
            }
        );
        let p = nm_privacy(&[image.into(), dhcp_hostname_conf(true)]);
        assert!(p.send_hostname && p.ip6_privacy);
        // Stock ships none of these: NetworkManager's defaults.
        let stock = nm_privacy(&["[main]\nplugins=ifupdown,keyfile\n".into()]);
        assert_eq!(
            stock,
            NmPrivacy {
                send_hostname: true,
                ip6_privacy: false,
                wifi_powersave: true,
                dns: String::new(),
            }
        );
        let off = nm_privacy(&[image.into(), wifi_powersave_conf(false)]);
        assert!(!off.wifi_powersave);
        let on = nm_privacy(&[wifi_powersave_conf(false), wifi_powersave_conf(true)]);
        assert!(on.wifi_powersave);
        // A key in the wrong section is not that setting.
        assert!(!nm_privacy(&["[device]\nipv6.ip6-privacy=2\n".into()]).ip6_privacy);
    }
    #[test]
    fn zones_come_from_the_country_s_lines() {
        let tab = "# comment\nUS\t+404251-0740023\tAmerica/New_York\tEastern\n\
                   GB\t+513030-0000731\tEurope/London\n\
                   US\t+415100-0873900\tAmerica/Chicago\tCentral\n";
        assert_eq!(
            zones_for(tab, "US"),
            ["America/New_York", "America/Chicago"]
        );
        assert_eq!(zones_for(tab, "GB"), ["Europe/London"]);
        assert!(zones_for(tab, "UA").is_empty());
    }

    #[test]
    fn a_name_is_taken_by_a_user_or_a_group() {
        let passwd = "root:x:0:0::/root:/bin/sh\nzero:x:1000:1000::/home/zero:/bin/bash\n";
        let group = "video:x:44:\nzero:x:1000:\n";
        assert!(name_taken(passwd, group, "root"));
        assert!(name_taken(passwd, group, "video"));
        assert!(!name_taken(passwd, group, "alex"));
        assert!(!name_taken(passwd, group, "ro"), "a prefix is not a name");
    }
    #[test]
    fn countries_sort_as_read_not_by_case() {
        let tab = "UM\tUS minor outlying islands\nUG\tUganda\nUS\tUnited States\n";
        let names: Vec<String> = countries(tab).into_iter().map(|(_, n)| n).collect();
        assert_eq!(
            names,
            ["Uganda", "United States", "US minor outlying islands"]
        );
    }
}
