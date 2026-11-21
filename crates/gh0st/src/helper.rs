//! `gh0st-helper`: the few things Settings needs root for. pkexec runs it as
//! root for the device user, allowed by our polkit rule, so it takes a fixed
//! set of verbs, checks every argument, and never runs a shell.
//!
//!     gh0st-helper logs off|ram|persistent       password
//!     gh0st-helper ssh on|off                    password
//!     gh0st-helper gpu-mem 16|32|64|96|128|192|256   password, restart
//!     gh0st-helper regdom XX                     password, restart
//!     gh0st-helper cpu balanced|power-saver|performance   password
//!     gh0st-helper hostname NAME                 password
//!     gh0st-helper swap MB                       password, restart
//!     gh0st-helper swap-comp zstd|lz4|lzo-rle    password, restart
//!     gh0st-helper swappiness 0..200             password
//!     gh0st-helper rfkill bluetooth|wlan on|off
//!     gh0st-helper power off|reboot
//!
//! A password verb reads the user's password as one line on stdin (see
//! `auth`). Exit codes: 0 done, 1 failed, 2 usage, 10 password required,
//! 11 wrong password, 12 locked (stderr says for how many seconds). Nothing
//! it prints mentions polkit, so Settings can tell its errors from pkexec's.

use crate::auth::{self, Auth};
use gh0st_core::rfkill::{self, Radio};
use gh0st_core::sysconf::{self, CpuMode};
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::process::Command;

/// Sorts after stock's `persist.conf`, so its Storage= wins.
const LOGGING_CONF: &str = "/etc/systemd/journald.conf.d/zz-gh0st-logging.conf";
/// Runs after stock's `60-ondemand-governor.rules` on the same event.
const CPU_RULE: &str = "/etc/udev/rules.d/61-gh0st-cpu.rules";
const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";
/// A boot file edit changes one value; anything bigger is a bug.
const MAX_GROWTH: usize = 64;
/// A sanity bound for parsing; `set_swap` holds it to twice RAM.
const SWAP_PARSE_MAX: u32 = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum Verb {
    Logs(&'static str),
    Ssh(bool),
    GpuMem(u32),
    Regdom(String),
    Cpu(CpuMode),
    Hostname(String),
    Swap(u32),
    SwapComp(&'static str),
    Swappiness(u32),
    Rfkill(Radio, bool),
    Power(bool),
}

impl Verb {
    /// Whether the user's password is needed. No wildcard arm: a new verb
    /// doesn't build until someone decides.
    pub fn gated(&self) -> bool {
        match self {
            Verb::Logs(_)
            | Verb::Ssh(_)
            | Verb::GpuMem(_)
            | Verb::Regdom(_)
            | Verb::Cpu(_)
            | Verb::Hostname(_)
            | Verb::Swap(_)
            | Verb::SwapComp(_)
            | Verb::Swappiness(_) => true,
            Verb::Rfkill(..) | Verb::Power(_) => false,
        }
    }
}

pub fn parse(args: &[String]) -> Option<Verb> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let on = |s: &str| match s {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    Some(match a.as_slice() {
        ["logs", "off"] => Verb::Logs("off"),
        ["logs", "ram"] => Verb::Logs("ram"),
        ["logs", "persistent"] => Verb::Logs("persistent"),
        ["ssh", s] => Verb::Ssh(on(s)?),
        ["gpu-mem", n] => Verb::GpuMem(*sysconf::GPU_MEMS.iter().find(|m| m.to_string() == *n)?),
        ["regdom", cc] if cc.len() == 2 && cc.bytes().all(|b| b.is_ascii_uppercase()) => {
            Verb::Regdom(cc.to_string())
        }
        ["cpu", m] => Verb::Cpu(CpuMode::from_verb(m)?),
        ["hostname", n] if sysconf::valid_hostname(n) => Verb::Hostname(n.to_string()),
        ["swap", n] => Verb::Swap(number(n, SWAP_PARSE_MAX)?),
        ["swap-comp", a] => Verb::SwapComp(sysconf::COMPRESSIONS.into_iter().find(|c| c == a)?),
        ["swappiness", n] => Verb::Swappiness(number(n, sysconf::SWAPPINESS_MAX)?),
        ["rfkill", "bluetooth", s] => Verb::Rfkill(Radio::Bluetooth, on(s)?),
        ["rfkill", "wlan", s] => Verb::Rfkill(Radio::Wlan, on(s)?),
        ["power", "off"] => Verb::Power(false),
        ["power", "reboot"] => Verb::Power(true),
        _ => return None,
    })
}

/// Plain digits with no leading zero, up to `max`.
fn number(s: &str, max: u32) -> Option<u32> {
    let plain = s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    s.parse().ok().filter(|&n| plain && n <= max)
}

/// The journald drop-in for each logging level.
pub fn logging_conf(level: &str) -> &'static str {
    match level {
        "ram" => "[Journal]\nStorage=volatile\nRuntimeMaxUse=8M\n",
        "persistent" => "[Journal]\nStorage=persistent\nSystemMaxUse=16M\n",
        _ => "[Journal]\nStorage=none\n",
    }
}

fn cpu_rule(governor: &str) -> String {
    format!(
        "# Settings > System > Advanced > CPU mode.\n\
         KERNEL==\"cpu*\", SUBSYSTEM==\"cpu\", ATTR{{cpufreq/scaling_governor}}=\"{governor}\"\n"
    )
}

fn run(argv: &[&str]) -> Result<(), String> {
    let status = Command::new(argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed", argv.join(" ")))
    }
}

fn sync_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

/// Edits a file on the vfat boot partition, which has no journal. Keeps a
/// `.bak` of the old text. A same-length edit (the country) rewrites only
/// the changed bytes in place; anything else goes through a new file and a
/// rename. Then it reads the file back and restores the backup on a mismatch.
fn write_boot(
    path: &Path,
    edit: impl Fn(&str) -> Result<String, &'static str>,
) -> Result<(), String> {
    let name = path.display();
    let io = |e: std::io::Error| format!("{name}: {e}");
    let old = std::fs::read_to_string(path).map_err(io)?;
    let new = edit(&old).map_err(String::from)?;
    if new == old {
        return Ok(());
    }
    if new.is_empty() || new.len().abs_diff(old.len()) > MAX_GROWTH {
        return Err(format!("{name}: refusing an edit that big"));
    }
    let bak = path.with_extension("bak");
    let f = std::fs::File::create(&bak).map_err(io)?;
    f.write_all_at(old.as_bytes(), 0).map_err(io)?;
    f.sync_all().map_err(io)?;
    if new.len() == old.len() {
        let (o, n) = (old.as_bytes(), new.as_bytes());
        let first = o.iter().zip(n).position(|(a, b)| a != b).unwrap_or(0);
        let last = n.len() - o.iter().rev().zip(n.iter().rev()).position(|(a, b)| a != b).unwrap_or(0);
        let f = std::fs::OpenOptions::new().write(true).open(path).map_err(io)?;
        f.write_all_at(&n[first..last], first as u64).map_err(io)?;
        f.sync_all().map_err(io)?;
    } else {
        let tmp = path.with_extension("new");
        let f = std::fs::File::create(&tmp).map_err(io)?;
        f.write_all_at(new.as_bytes(), 0).map_err(io)?;
        f.sync_all().map_err(io)?;
        drop(f);
        std::fs::rename(&tmp, path).map_err(io)?;
        sync_dir(path);
    }
    if std::fs::read_to_string(path).ok().as_deref() != Some(new.as_str()) {
        std::fs::copy(&bak, path).map_err(io)?;
        sync_dir(path);
        return Err(format!("{name}: read back differs; restored"));
    }
    Ok(())
}

fn set_cpu(mode: CpuMode) -> Result<(), String> {
    let gov = mode.governor();
    let policies: Vec<_> = std::fs::read_dir(CPUFREQ)
        .map_err(|e| format!("{CPUFREQ}: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("policy")))
        .collect();
    if policies.is_empty() {
        return Err("no cpufreq policy".into());
    }
    for p in &policies {
        let have = std::fs::read_to_string(p.join("scaling_available_governors")).unwrap_or_default();
        if !have.split_whitespace().any(|g| g == gov) {
            return Err(format!("the kernel has no {gov} governor"));
        }
    }
    for p in &policies {
        let f = p.join("scaling_governor");
        std::fs::write(&f, gov).map_err(|e| format!("{}: {e}", f.display()))?;
    }
    gh0st_core::config::write_atomic(Path::new(CPU_RULE), cpu_rule(gov).as_bytes())
        .map_err(|e| format!("{CPU_RULE}: {e}"))
}

fn set_hostname(name: &str) -> Result<(), String> {
    run(&["hostnamectl", "set-hostname", name])?;
    let hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    let new = sysconf::hosts_with_name(&hosts, name);
    if new != hosts {
        gh0st_core::config::write_atomic(Path::new("/etc/hosts"), new.as_bytes())
            .map_err(|e| format!("/etc/hosts: {e}"))?;
    }
    Ok(())
}

/// Takes effect at the next boot: swapoff of a full zram can run this
/// device out of memory.
fn set_swap(mb: u32) -> Result<(), String> {
    let max = sysconf::swap_max(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
    if mb > max {
        return Err(format!("swap is at most {max} MB here"));
    }
    let backend = sysconf::swap_backend();
    write_conf(backend.dropin(), &backend.conf(mb))
}

/// Also at the next boot: zram takes a new algorithm only while empty.
fn set_swap_comp(alg: &str) -> Result<(), String> {
    // With swap off there is no device to ask; the list is ours then.
    if let Ok(text) = std::fs::read_to_string(sysconf::ZRAM_COMP) {
        if !sysconf::comp_algorithms(&text).0.iter().any(|a| a == alg) {
            return Err(format!("the kernel has no {alg}"));
        }
    }
    write_conf(sysconf::ZRAM_COMP_DROPIN, &sysconf::comp_conf(alg))
}

/// Now and at every boot.
fn set_swappiness(n: u32) -> Result<(), String> {
    write_conf(sysconf::SWAPPINESS_DROPIN, &sysconf::swappiness_conf(n))?;
    std::fs::write(sysconf::SWAPPINESS_NOW, n.to_string())
        .map_err(|e| format!("{}: {e}", sysconf::SWAPPINESS_NOW))
}

/// A whole drop-in of ours, its directory made if missing.
fn write_conf(path: &str, text: &str) -> Result<(), String> {
    let path = Path::new(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    gh0st_core::config::write_atomic(path, text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn act(v: Verb) -> Result<(), String> {
    match v {
        Verb::Logs(level) => {
            if level == "off" {
                // What is on disk goes too, not only what comes next.
                let _ = run(&["journalctl", "--rotate"]);
                let _ = run(&["journalctl", "--vacuum-time=1s"]);
            }
            std::fs::create_dir_all("/etc/systemd/journald.conf.d").map_err(|e| e.to_string())?;
            gh0st_core::config::write_atomic(
                Path::new(LOGGING_CONF),
                logging_conf(level).as_bytes(),
            )
            .map_err(|e| format!("{LOGGING_CONF}: {e}"))?;
            run(&["systemctl", "restart", "systemd-journald"])
        }
        Verb::Ssh(on) => {
            let verb = if on { "enable" } else { "disable" };
            run(&["systemctl", verb, "--now", "ssh"])
        }
        Verb::GpuMem(mb) => write_boot(&sysconf::boot_dir().join(sysconf::CONFIG), |t| {
            sysconf::set_gpu_mem(t, mb)
        }),
        Verb::Regdom(cc) => {
            let tab = std::fs::read_to_string(sysconf::COUNTRIES)
                .map_err(|e| format!("{}: {e}", sysconf::COUNTRIES))?;
            if !sysconf::countries(&tab).iter().any(|(c, _)| *c == cc) {
                return Err(format!("{cc} is not a country"));
            }
            write_boot(&sysconf::boot_dir().join(sysconf::CMDLINE), |t| {
                sysconf::set_regdom(t, &cc)
            })
        }
        Verb::Cpu(mode) => set_cpu(mode),
        Verb::Hostname(name) => set_hostname(&name),
        Verb::Swap(mb) => set_swap(mb),
        Verb::SwapComp(alg) => set_swap_comp(alg),
        Verb::Swappiness(n) => set_swappiness(n),
        Verb::Rfkill(radio, on) => rfkill::set(radio, !on).map_err(|e| format!("/dev/rfkill: {e}")),
        Verb::Power(reboot) => run(&["systemctl", if reboot { "reboot" } else { "poweroff" }]),
    }
}

pub fn main(args: &[String]) -> i32 {
    let Some(v) = parse(args) else {
        eprintln!(
            "usage: gh0st-helper logs off|ram|persistent | ssh on|off | gpu-mem 16..256 | \
             regdom XX | cpu balanced|power-saver|performance | hostname NAME | swap MB | \
             swap-comp zstd|lz4|lzo-rle | swappiness 0..200 | \
             rfkill bluetooth|wlan on|off | power off|reboot"
        );
        return 2;
    };
    if v.gated() {
        match auth::check() {
            Auth::Ok => {}
            Auth::Required => {
                eprintln!("gh0st-helper: password required");
                return 10;
            }
            Auth::Wrong => {
                eprintln!("gh0st-helper: wrong password");
                return 11;
            }
            Auth::Locked(secs) => {
                eprintln!("gh0st-helper: locked {secs}");
                return 12;
            }
            Auth::Failed(e) => {
                eprintln!("gh0st-helper: {e}");
                return 1;
            }
        }
    }
    match act(v) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gh0st-helper: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn only_the_fixed_verbs_get_through() {
        assert_eq!(parse(&args("logs ram")), Some(Verb::Logs("ram")));
        assert_eq!(parse(&args("ssh off")), Some(Verb::Ssh(false)));
        assert_eq!(
            parse(&args("rfkill bluetooth on")),
            Some(Verb::Rfkill(Radio::Bluetooth, true))
        );
        assert_eq!(parse(&args("power reboot")), Some(Verb::Power(true)));
        assert_eq!(parse(&args("gpu-mem 64")), Some(Verb::GpuMem(64)));
        assert_eq!(parse(&args("regdom GB")), Some(Verb::Regdom("GB".into())));
        assert_eq!(parse(&args("cpu power-saver")), Some(Verb::Cpu(CpuMode::PowerSaver)));
        assert_eq!(
            parse(&args("hostname RZR-ZERO")),
            Some(Verb::Hostname("RZR-ZERO".into()))
        );
        assert_eq!(parse(&args("swap 0")), Some(Verb::Swap(0)));
        assert_eq!(parse(&args("swap 256")), Some(Verb::Swap(256)));
        assert_eq!(parse(&args("swap-comp lz4")), Some(Verb::SwapComp("lz4")));
        assert_eq!(parse(&args("swappiness 0")), Some(Verb::Swappiness(0)));
        assert_eq!(parse(&args("swappiness 200")), Some(Verb::Swappiness(200)));
        for bad in [
            "",
            "logs",
            "logs /etc/passwd",
            "ssh maybe",
            "rfkill nfc on",
            "power",
            "ssh on extra",
            "password",
            "gpu-mem 48",
            "gpu-mem 512",
            "gpu-mem +64",
            "gpu-mem 064",
            "regdom ae",
            "regdom USA",
            "cpu turbo",
            "cpu schedutil",
            "hostname -x",
            "hostname a.b",
            "hostname",
            "swap",
            "swap 064",
            "swap -1",
            "swap +1",
            "swap 256MB",
            "swap 99999",
            "swap-comp",
            "swap-comp lzo",
            "swap-comp ZSTD",
            "swappiness 201",
            "swappiness 060",
            "swappiness -1",
        ] {
            assert_eq!(parse(&args(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn only_power_and_radios_skip_the_password() {
        for (verb, gated) in [
            ("logs off", true),
            ("ssh on", true),
            ("gpu-mem 128", true),
            ("regdom UA", true),
            ("cpu performance", true),
            ("hostname cardputer", true),
            ("swap 256", true),
            ("swap-comp zstd", true),
            ("swappiness 100", true),
            ("rfkill wlan on", false),
            ("power off", false),
        ] {
            assert_eq!(parse(&args(verb)).unwrap().gated(), gated, "{verb}");
        }
    }

    #[test]
    fn each_level_writes_its_storage() {
        assert!(logging_conf("off").contains("Storage=none"));
        assert!(logging_conf("ram").contains("Storage=volatile"));
        assert!(logging_conf("persistent").contains("Storage=persistent"));
    }

    #[test]
    fn the_cpu_rule_names_the_governor() {
        assert!(cpu_rule("schedutil").contains("ATTR{cpufreq/scaling_governor}=\"schedutil\""));
    }

    fn scratch(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gh0st-helper-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_same_length_edit_writes_in_place_with_a_backup() {
        let line = "root=x quiet cfg80211.ieee80211_regdom=AE\n";
        let p = scratch("cmdline.txt", line);
        write_boot(&p, |t| sysconf::set_regdom(t, "GB")).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), line.replace("AE", "GB"));
        assert_eq!(std::fs::read_to_string(p.with_extension("bak")).unwrap(), line);
        assert!(!p.with_extension("new").exists());
    }

    #[test]
    fn a_longer_edit_goes_through_a_rename() {
        let p = scratch("config.txt", "arm_64bit=1\n");
        write_boot(&p, |t| sysconf::set_gpu_mem(t, 64)).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "gpu_mem=64\narm_64bit=1\n");
        assert!(!p.with_extension("new").exists());
        write_boot(&p, |_| Ok("x".repeat(200))).unwrap_err();
        assert!(write_boot(&p, |_| Err("set by hand")).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "gpu_mem=64\narm_64bit=1\n");
    }
}
