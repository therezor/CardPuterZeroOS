//! System files the root helper writes and Settings reads: the Pi's boot
//! files, the CPU governor and the hostname. Pure text in and out, so both
//! sides use the same parser and the transforms test on any host.

use std::path::PathBuf;

pub const CONFIG: &str = "config.txt";
pub const CMDLINE: &str = "cmdline.txt";
/// 16 boots the cut-down firmware (no camera or video); past 128 under KMS
/// only takes RAM from Linux.
pub const GPU_MEMS: [u32; 7] = [16, 32, 64, 96, 128, 192, 256];
pub const COUNTRIES: &str = "/usr/share/zoneinfo/iso3166.tab";
const REGDOM: &str = "cfg80211.ieee80211_regdom=";

/// The boot partition. `GH0ST_BOOT_DIR` points a host run at a copy.
pub fn boot_dir() -> PathBuf {
    std::env::var_os("GH0ST_BOOT_DIR").map_or_else(|| PathBuf::from("/boot/firmware"), PathBuf::from)
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
            let eol = if config.contains("\r\n") { "\r\n" } else { "\n" };
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
    let mut at = body.match_indices(REGDOM).filter(|(i, _)| {
        *i == 0 || body.as_bytes()[i - 1].is_ascii_whitespace()
    });
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
    out.sort_by(|a, b| a.1.cmp(&b.1));
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
    Battery,
    Performance,
}

impl CpuMode {
    pub const ALL: [CpuMode; 3] = [CpuMode::Balanced, CpuMode::Battery, CpuMode::Performance];

    pub fn label(self) -> &'static str {
        match self {
            CpuMode::Balanced => "Balanced",
            CpuMode::Battery => "Battery",
            CpuMode::Performance => "Performance",
        }
    }

    /// The helper's word for it.
    pub fn verb(self) -> &'static str {
        match self {
            CpuMode::Balanced => "balanced",
            CpuMode::Battery => "battery",
            CpuMode::Performance => "performance",
        }
    }

    pub fn governor(self) -> &'static str {
        match self {
            CpuMode::Balanced => "schedutil",
            CpuMode::Battery => "powersave",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let v5 = lines.iter().position(|l| *l == "dtoverlay=cardputerzero-v5-overlay");
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
        assert_eq!(gpu_mem("#gpu_mem=16\n[pi4]\n[all]\ngpu_mem=96\n"), GpuMem::Set(96));
        assert!(set_gpu_mem("gpu_mem=128\n", 48).is_err(), "only the listed sizes");
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
}
