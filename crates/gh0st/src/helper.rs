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
//!     gh0st-helper set-time EPOCH                password
//!     gh0st-helper ntp on|off                    password
//!     gh0st-helper dhcp-hostname on|off          password
//!     gh0st-helper ip6-privacy on|off            password
//!     gh0st-helper dns auto|SERVERS              password
//!     gh0st-helper rfkill bluetooth|wlan on|off
//!     gh0st-helper usb-power on|off
//!     gh0st-helper usb-hid hold                  until stdin closes
//!     gh0st-helper bt-classic on|off
//!     gh0st-helper wifi-powersave on|off
//!     gh0st-helper hotspot on|off
//!     gh0st-helper hotspot name NAME
//!     gh0st-helper hotspot password              the new password
//!     gh0st-helper hotspot security wpa3|mixed
//!     gh0st-helper power off|reboot
//!     gh0st-helper update-check
//!     gh0st-helper update-install                password
//!     gh0st-helper update-autoremove             password
//!     gh0st-helper root-place STEM PID
//!     gh0st-helper root-grant STEM on|off        password
//!     gh0st-helper update-stop
//!     gh0st-helper update-auto on|off            password
//!     gh0st-helper update-channel stable|edge    password
//!     gh0st-helper apt-clean
//!     gh0st-helper store-refresh
//!     gh0st-helper store-install [--mirror auto|github|china] PKG[=VER]...   password
//!     gh0st-helper store-remove PKG              password
//!     gh0st-helper store-upgrade [--mirror auto|github|china]          password
//!     gh0st-helper user-create NAME              setup only, the new password
//!     gh0st-helper setup-finish                  setup only
//!     gh0st-helper files-worker                  password, never in setup
//!
//! `usb-hid hold` builds the Keyboard app's USB gadget, prints `node NAME
//! PATH` for each function and `state WORD` as the host takes it, and
//! takes it down when stdin closes. The app keeps the pipe, so however it
//! ends, the host sees the keyboard unplugged and lets go of every key.
//!
//! `files-worker` is Files' root mode, and the one verb that is not a
//! fixed job: it hands the caller a root Files worker, as much power as
//! sudo behind the same password. Its stdin must be a socket. It reads one
//! line, the password or empty under grace, then execs `gh0st-files
//! --worker` with the socket on fd 3 and an environment built from nothing.
//! The worker serves local files only and acts as the user in the user's
//! own folders (docs/files.md).
//!
//! A password verb reads the user's password as one line on stdin (see
//! `auth`). `hotspot password` reads the hotspot's new password the same way. A setup verb runs only for root while setup's marker exists
//! (docs/setup.md); `user-create` reads the new user's password instead.
//! Exit codes: 0 done, 1 failed, 2 usage, 10 password required, 11 wrong
//! password, 12 locked (stderr says for how many seconds), 13 setup only.
//! Nothing it prints mentions polkit, so Settings can tell its errors from
//! pkexec's.

use crate::auth::{self, Auth};
use gh0st_core::hotspot::{self, Profile, Security};
use gh0st_core::rfkill::{self, Radio};
use gh0st_core::rootapps;
use gh0st_core::store::{self, Job, Mirror, Request};
use gh0st_core::sysconf::{self, CpuMode};
use std::ffi::CString;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, FileExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Sorts after stock's `persist.conf`, so its Storage= wins.
const LOGGING_CONF: &str = "/etc/systemd/journald.conf.d/zz-gh0st-logging.conf";
/// Runs after stock's `60-ondemand-governor.rules` on the same event.
const CPU_RULE: &str = "/etc/udev/rules.d/61-gh0st-cpu.rules";
const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";
/// A boot file edit changes one value; anything bigger is a bug.
const MAX_GROWTH: usize = 64;
/// A sanity bound for parsing; `set_swap` holds it to twice RAM.
const SWAP_PARSE_MAX: u32 = 4096;
/// Setup runs while this exists; `setup-finish` removes it.
const SETUP_MARKER: &str = "/var/lib/gh0st/setup";
const SETUP_STATE: &str = "/var/lib/gh0st/setup.state";
/// Setup's own home: the look chosen during setup is in its config.
const SETUP_HOME: &str = "/var/lib/gh0st/setup-home";
const LINGER_DIR: &str = "/var/lib/systemd/linger";
const USER_UID: u32 = 1000;
/// Our repo's stable source, from gh0st-base. Edge is the same with
/// another suite, in its own file, so the URL lives in one place.
const STABLE_SOURCES: &str = "/etc/apt/sources.list.d/gh0st.sources";
use gh0st_core::update::EDGE_SOURCES;
/// The device user's groups. One the system lacks is left out, so
/// `useradd` doesn't fail on it.
const USER_GROUPS: [&str; 11] = [
    "sudo", "netdev", "video", "input", "render", "audio", "gpio", "i2c", "spi", "dialout",
    "plugdev",
];
const PASSWORD_MIN: usize = 4;
/// The clock setup and Settings may set: 2024 to 2099.
const EPOCH_MIN: i64 = 1_704_067_200;
const EPOCH_MAX: i64 = 4_102_444_800;

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
    SetTime(i64),
    Ntp(bool),
    DhcpHostname(bool),
    Ip6Privacy(bool),
    /// Name servers for every connection, checked; None takes each network's.
    Dns(Option<String>),
    Rfkill(Radio, bool),
    /// The built-in Ethernet chip may suspend.
    UsbPower(bool),
    /// The Keyboard app's USB gadget, held while stdin is open.
    UsbHid,
    /// Files' root worker: exec'd, with the caller's socket on fd 3.
    FilesWorker,
    /// Classic Bluetooth (BR/EDR) on or off; LE stays on.
    BtClassic(bool),
    WifiPowersave(bool),
    Hotspot(bool),
    HotspotName(String),
    HotspotPassword,
    HotspotSecurity(Security),
    Power(bool),
    UserCreate(String),
    SetupFinish,
    /// The update engine (docs/updates.md) runs in its own units; these
    /// only start, stop and set them up.
    UpdateCheck,
    UpdateInstall,
    UpdateAutoremove,
    UpdateStop,
    UpdateAuto(bool),
    /// True for edge.
    UpdateChannel(bool),
    AptClean,
    /// The Store's engine (docs/store.md): the helper checks the job
    /// against the verified catalog, writes it, and starts the unit.
    StoreRefresh,
    StoreInstall(Mirror, Vec<String>),
    StoreRemove(String),
    StoreUpgrade(Mirror),
    /// The shell's trampoline for a granted app, into its root cgroup.
    RootPlace(String, u32),
    /// Root without a password for an app, or not.
    RootGrant(String, bool),
}

/// More apps than this in one install is not a person asking.
const STORE_MAX_APPS: usize = 64;

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
            | Verb::Swappiness(_)
            | Verb::SetTime(_)
            | Verb::Ntp(_)
            | Verb::DhcpHostname(_)
            | Verb::Ip6Privacy(_)
            | Verb::Dns(_)
            | Verb::UpdateInstall
            | Verb::UpdateAutoremove
            | Verb::RootGrant(..)
            | Verb::UpdateAuto(_)
            | Verb::UpdateChannel(_)
            | Verb::StoreInstall(..)
            | Verb::StoreRemove(_)
            | Verb::StoreUpgrade(_)
            | Verb::FilesWorker => true,
            // The hotspot carries the trust NetworkManager's polkit grant
            // already gives the device user.
            Verb::Rfkill(..)
            | Verb::WifiPowersave(_)
            | Verb::Hotspot(_)
            | Verb::HotspotName(_)
            | Verb::HotspotPassword
            | Verb::HotspotSecurity(_)
            | Verb::UsbPower(_)
            // A keyboard that needs a cable and the side switch, typing
            // only what the app that holds it sends.
            | Verb::UsbHid
            // The radio the user already turned on, one half of it.
            | Verb::BtClassic(_)
            | Verb::Power(_)
            | Verb::UserCreate(_)
            | Verb::SetupFinish
            | Verb::UpdateCheck
            | Verb::UpdateStop
            | Verb::AptClean
            // What it places can only run the app's own Exec.
            | Verb::RootPlace(..)
            | Verb::StoreRefresh => false,
        }
    }

    /// Whether only setup may run it: root, while the marker exists.
    pub fn setup_only(&self) -> bool {
        matches!(self, Verb::UserCreate(_) | Verb::SetupFinish)
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
        ["set-time", n] => Verb::SetTime(epoch(n)?),
        ["ntp", s] => Verb::Ntp(on(s)?),
        ["dhcp-hostname", s] => Verb::DhcpHostname(on(s)?),
        ["ip6-privacy", s] => Verb::Ip6Privacy(on(s)?),
        ["dns", "auto"] => Verb::Dns(None),
        ["dns", list] => Verb::Dns(Some(sysconf::dns_servers(list)?)),
        ["user-create", n] if sysconf::valid_user_name(n) => Verb::UserCreate(n.to_string()),
        ["setup-finish"] => Verb::SetupFinish,
        ["rfkill", "bluetooth", s] => Verb::Rfkill(Radio::Bluetooth, on(s)?),
        ["rfkill", "wlan", s] => Verb::Rfkill(Radio::Wlan, on(s)?),
        ["usb-power", s] => Verb::UsbPower(on(s)?),
        ["usb-hid", "hold"] => Verb::UsbHid,
        ["files-worker"] => Verb::FilesWorker,
        ["bt-classic", s] => Verb::BtClassic(on(s)?),
        ["wifi-powersave", s] => Verb::WifiPowersave(on(s)?),
        ["hotspot", "name", n] if hotspot::valid_name(n) => Verb::HotspotName(n.to_string()),
        ["hotspot", "password"] => Verb::HotspotPassword,
        ["hotspot", "security", s] => Verb::HotspotSecurity(Security::from_verb(s)?),
        ["hotspot", s] => Verb::Hotspot(on(s)?),
        ["power", "off"] => Verb::Power(false),
        ["power", "reboot"] => Verb::Power(true),
        ["update-check"] => Verb::UpdateCheck,
        ["update-install"] => Verb::UpdateInstall,
        ["update-autoremove"] => Verb::UpdateAutoremove,
        ["root-place", s, pid] if rootapps::is_stem(s) => Verb::RootPlace(
            s.to_string(),
            number(pid, i32::MAX as u32).filter(|&p| p > 1)?,
        ),
        ["root-grant", s, o] if rootapps::is_stem(s) => Verb::RootGrant(s.to_string(), on(o)?),
        ["update-stop"] => Verb::UpdateStop,
        ["update-auto", s] => Verb::UpdateAuto(on(s)?),
        ["update-channel", "stable"] => Verb::UpdateChannel(false),
        ["update-channel", "edge"] => Verb::UpdateChannel(true),
        ["apt-clean"] => Verb::AptClean,
        ["store-refresh"] => Verb::StoreRefresh,
        ["store-remove", p] if store::is_package(p) => Verb::StoreRemove(p.to_string()),
        ["store-upgrade"] => Verb::StoreUpgrade(Mirror::Auto),
        ["store-upgrade", "--mirror", m] => Verb::StoreUpgrade(Mirror::parse(m)?),
        ["store-install", "--mirror", m, rest @ ..] => {
            Verb::StoreInstall(Mirror::parse(m)?, packages(rest)?)
        }
        ["store-install", rest @ ..] => Verb::StoreInstall(Mirror::Auto, packages(rest)?),
        _ => return None,
    })
}

/// Plain digits with no leading zero, up to `max`.
/// One or more Debian package names, each once, each maybe with the
/// version the user saw (`name=version`).
fn packages(names: &[&str]) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        if !store::is_wanted(n) {
            return None;
        }
        let name = store::split_seen(n).0;
        if !out.iter().any(|o| store::split_seen(o).0 == name) {
            out.push(n.to_string());
        }
    }
    (!out.is_empty() && out.len() <= STORE_MAX_APPS).then_some(out)
}

fn number(s: &str, max: u32) -> Option<u32> {
    let plain = s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    s.parse().ok().filter(|&n| plain && n <= max)
}

/// Seconds since 1970, plain digits, in the range a clock may be set to.
fn epoch(s: &str) -> Option<i64> {
    let plain = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0');
    s.parse()
        .ok()
        .filter(|n| plain && (EPOCH_MIN..EPOCH_MAX).contains(n))
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
        let last = n.len()
            - o.iter()
                .rev()
                .zip(n.iter().rev())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(io)?;
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
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("policy"))
        })
        .collect();
    if policies.is_empty() {
        return Err("no cpufreq policy".into());
    }
    for p in &policies {
        let have =
            std::fs::read_to_string(p.join("scaling_available_governors")).unwrap_or_default();
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

/// The rule for later boots and replugs, then the chip now.
/// Writes a line for the app, never failing: a closed stdout must not stop
/// the helper before it takes the gadget down.
fn say(line: &str) {
    // SAFETY: a write from a buffer of its own length.
    unsafe { libc::write(1, line.as_ptr().cast(), line.len()) };
}

/// Classic Bluetooth on or off. Turning it off needs the radio off, so a
/// powered radio restarts; turning it on doesn't. While a phone pairs with the
/// Keyboard app classic is off: Android, seeing a classic side, looks for a
/// classic keyboard there and never uses the LE one.
fn bt_classic(on: bool) -> Result<(), String> {
    use gh0st_core::btmgmt::Mgmt;
    let e = |e: std::io::Error| format!("bluetooth: {e}");
    let m = Mgmt::open().map_err(e)?;
    // Classic comes back on a powered radio, so nothing connected drops;
    // it goes off only on one that is off.
    let powered = m.powered(0).map_err(e)?;
    if on || !powered {
        return m.set_bredr(0, on).map_err(e);
    }
    m.set_powered(0, false).map_err(e)?;
    let set = m.set_bredr(0, false).map_err(e);
    m.set_powered(0, true).map_err(e)?;
    set
}

/// Builds the USB keyboard, binds it, and holds it until stdin closes.
fn usb_hid_hold() -> Result<(), String> {
    use gh0st_core::hidgadget::{self, Real};
    let _ = run(&["modprobe", "libcomposite"]);
    if !Path::new(hidgadget::CONFIGFS).exists() {
        run(&["mount", "-t", "configfs", "none", "/sys/kernel/config"])?;
    }
    let root = hidgadget::root();
    let mut fs = Real;
    // One left by a helper that was killed.
    hidgadget::teardown(&mut fs, &root);
    let udc = hidgadget::udc(Path::new(hidgadget::UDC_CLASS)).ok_or("no USB device controller")?;
    let up = hidgadget::build(&mut fs, &root).and_then(|_| hidgadget::bind(&mut fs, &root, &udc));
    if let Err(e) = up {
        hidgadget::teardown(&mut fs, &root);
        return Err(format!("usb gadget: {e}"));
    }
    // The nodes are the user's alone, not every member of a group's.
    let uid: Option<u32> = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|u| u.parse().ok());
    for (name, node) in hidgadget::nodes(&root, Path::new(hidgadget::HIDG_CLASS)) {
        if let (Some(uid), Ok(c)) = (uid, CString::new(node.as_os_str().as_encoded_bytes())) {
            // SAFETY: chown and chmod on a path we own as root.
            unsafe {
                libc::chown(c.as_ptr(), uid, u32::MAX);
                libc::chmod(c.as_ptr(), 0o600);
            }
        }
        say(&format!("node {name} {}\n", node.display()));
    }
    let state = std::fs::File::open(Path::new(hidgadget::UDC_CLASS).join(&udc).join("state"))
        .map_err(|e| format!("udc state: {e}"))?;
    let mut last = String::new();
    loop {
        let mut buf = [0u8; 64];
        let n = state.read_at(&mut buf, 0).unwrap_or(0);
        let now = String::from_utf8_lossy(&buf[..n]).trim().to_string();
        if now != last {
            say(&format!("state {now}\n"));
            last = now;
        }
        // sysfs wakes a poller with POLLPRI when the state changes.
        use std::os::fd::AsRawFd;
        let mut fds = [
            libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: state.as_raw_fd(),
                events: libc::POLLPRI | libc::POLLERR,
                revents: 0,
            },
        ];
        // SAFETY: two pollfds on our stack.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if r < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fds[0].revents != 0 {
            let mut sink = [0u8; 64];
            // SAFETY: a read into a buffer of its own length.
            let got = unsafe { libc::read(0, sink.as_mut_ptr().cast(), sink.len()) };
            if got <= 0 {
                break;
            }
        }
    }
    hidgadget::teardown(&mut fs, &root);
    Ok(())
}

fn usb_power(save: bool) -> Result<(), String> {
    write_conf(sysconf::USB_POWER_RULE, &sysconf::usb_power_rule(save))?;
    let value = if save { "auto" } else { "on" };
    for d in sysconf::usb_devices(Path::new("/sys/bus/usb/devices"), sysconf::ETH_USB_ID) {
        let f = d.join("power/control");
        std::fs::write(&f, value).map_err(|e| format!("{}: {e}", f.display()))?;
    }
    Ok(())
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

/// nmcli, its `Error:` line kept for the toast.
fn nmcli(args: &[&str]) -> Result<(), String> {
    let out = Command::new("nmcli")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stderr);
    Err(text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Error: "))
        .unwrap_or("nmcli failed")
        .trim_end_matches('.')
        .to_string())
}

/// The keyfile, 0600 from the start: it holds the password. NetworkManager
/// reads it on `load`; it takes effect the next time the hotspot starts.
fn save_hotspot(p: &Profile) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = Path::new(hotspot::KEYFILE);
    let io = |e: std::io::Error| format!("{}: {e}", path.display());
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    let mut text = p.render();
    let written = f.write_all(text.as_bytes()).and_then(|()| f.sync_all());
    // SAFETY: zeroes keep the string valid UTF-8.
    gh0st_core::password::wipe(unsafe { text.as_bytes_mut() });
    written.map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)?;
    sync_dir(path);
    nmcli(&["connection", "load", hotspot::KEYFILE])
}

/// Ours, or a new one with a neutral name and a random password.
fn load_hotspot() -> Result<Profile, String> {
    match std::fs::read_to_string(hotspot::KEYFILE) {
        Ok(mut text) => {
            let p = Profile::parse(&text);
            // SAFETY: zeroes keep the string valid UTF-8.
            gh0st_core::password::wipe(unsafe { text.as_bytes_mut() });
            p.ok_or_else(|| format!("{}: not a hotspot profile", hotspot::KEYFILE))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let p = Profile::new().map_err(|e| format!("random: {e}"))?;
            save_hotspot(&p)?;
            Ok(p)
        }
        Err(e) => Err(format!("{}: {e}", hotspot::KEYFILE)),
    }
}

/// A change while the hotspot runs restarts it, so clients join again with
/// the new name or password.
fn hotspot_edit(change: impl FnOnce(&mut Profile)) -> Result<(), String> {
    let mut p = load_hotspot()?;
    change(&mut p);
    save_hotspot(&p)?;
    if gh0st_core::status::hotspot() {
        nmcli(&["--wait", "30", "connection", "up", hotspot::CONN])?;
    }
    Ok(())
}

/// Taking wlan0 for the access point leaves the Wi-Fi network; turning it
/// off lets NetworkManager rejoin a saved one by itself.
fn hotspot_on(on: bool) -> Result<(), String> {
    if !on {
        return nmcli(&["--wait", "20", "connection", "down", hotspot::CONN]);
    }
    load_hotspot()?;
    rfkill::set(Radio::Wlan, false).map_err(|e| format!("/dev/rfkill: {e}"))?;
    nmcli(&["radio", "wifi", "on"])?;
    nmcli(&["--wait", "30", "connection", "up", hotspot::CONN])
}

/// A NetworkManager drop-in of ours, then a reload so the next connection
/// takes it.
fn write_nm(path: &str, text: &str) -> Result<(), String> {
    write_conf(path, text)?;
    run(&["nmcli", "general", "reload", "conf"])
}

/// The global DNS file, or none, then resolv.conf written again from it.
fn set_dns(servers: Option<&str>) -> Result<(), String> {
    let path = sysconf::NM_DNS_DROPIN;
    match servers {
        Some(s) => write_conf(path, &sysconf::dns_conf(s))?,
        None => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{path}: {e}"));
            }
            _ => {}
        },
    }
    run(&["nmcli", "general", "reload", "conf"])?;
    run(&["nmcli", "general", "reload", "dns-rc"])
}

/// The system clock now, and the RTC when there is one, so the time
/// survives a power cut before network time.
#[cfg(target_os = "linux")]
fn set_time(epoch: i64) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let ts = libc::timespec {
        tv_sec: epoch as _,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid timespec.
    if unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) } != 0 {
        return Err(format!("clock: {}", std::io::Error::last_os_error()));
    }
    let Ok(rtc) = std::fs::File::open("/dev/rtc0") else {
        return Ok(());
    };
    // SAFETY: a zeroed tm is valid, and gmtime_r fills it from t.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = ts.tv_sec;
    unsafe { libc::gmtime_r(&t, &mut tm) };
    /// `struct rtc_time` from linux/rtc.h.
    #[repr(C)]
    struct RtcTime([libc::c_int; 9]);
    let rt = RtcTime([
        tm.tm_sec, tm.tm_min, tm.tm_hour, tm.tm_mday, tm.tm_mon, tm.tm_year, tm.tm_wday,
        tm.tm_yday, 0,
    ]);
    /// `_IOW('p', 0x0a, struct rtc_time)`.
    const RTC_SET_TIME: u32 = 0x4024_700a;
    // SAFETY: rt is a valid rtc_time for the ioctl to read.
    if unsafe { libc::ioctl(rtc.as_raw_fd(), RTC_SET_TIME as _, &rt) } != 0 {
        return Err(format!("/dev/rtc0: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn set_time(_: i64) -> Result<(), String> {
    Err("setting the clock needs Linux".into())
}

/// Files' root worker. Refused in setup, for root itself and without a
/// socket on stdin; then one line is always read, so the worker never sees
/// the password or a stray line. On success it never returns: the helper
/// becomes the worker, keeping its pid, so the page that started pkexec
/// stays its parent.
fn files_worker() -> i32 {
    if in_setup() {
        eprintln!("gh0st-helper: not during setup");
        return 13;
    }
    let caller = match std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        Some(u) if u != 0 => u,
        _ => {
            eprintln!("gh0st-helper: files-worker is for the device user, through pkexec");
            return 1;
        }
    };
    if !stdin_is_socket() {
        eprintln!("gh0st-helper: files-worker wants a socket on stdin");
        return 2;
    }
    let line = auth::read_line(0);
    match auth::check_given(Some(line)) {
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
    let err = exec_files_worker(caller);
    eprintln!("gh0st-helper: files-worker: {err}");
    1
}

fn stdin_is_socket() -> bool {
    // SAFETY: fstat into a zeroed stat struct on fd 0.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(0, &mut st) == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFSOCK
    }
}

/// Puts the socket on fd 3, nothing on 0 and 1, and execs the worker with
/// a fixed environment. Its data folder is root's own: the worker empties
/// `stage` there at start, which in the user's folder would be a race.
fn exec_files_worker(caller: u32) -> String {
    use std::os::unix::process::CommandExt;
    let Some((uid, gid, _)) = auth::user_name(caller).and_then(|n| passwd(&n)) else {
        return format!("no user {caller}");
    };
    let data = "/run/gh0st-helper/files";
    if let Err(e) = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(data)
    {
        return format!("{data}: {e}");
    }
    // SAFETY: plain syscalls on fds this process owns; /dev/null is opened
    // read-write and closed after it is copied.
    unsafe {
        libc::umask(0o022);
        if libc::chdir(c"/".as_ptr()) != 0 || libc::dup2(0, 3) != 3 {
            return std::io::Error::last_os_error().to_string();
        }
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null < 0 {
            return std::io::Error::last_os_error().to_string();
        }
        libc::dup2(null, 0);
        libc::dup2(null, 1);
        if null > 3 {
            libc::close(null);
        }
    }
    let err = std::process::Command::new("/usr/bin/gh0st-files")
        .arg("--worker")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("HOME", "/root")
        .env("GH0ST_FILES_FD", "3")
        .env("GH0ST_FILES_DATA", data)
        .env("GH0ST_FILES_ROOT", format!("{uid}:{gid}"))
        .exec();
    err.to_string()
}

/// True for root while setup's marker exists. The helper checks this itself:
/// pkexec would let the device user run it too.
fn in_setup() -> bool {
    // SAFETY: plain syscalls with no arguments.
    let (uid, euid) = unsafe { (libc::getuid(), libc::geteuid()) };
    let root = match std::env::var("PKEXEC_UID").ok() {
        Some(v) => v == "0",
        None => uid == 0,
    };
    euid == 0 && root && Path::new(SETUP_MARKER).exists()
}

/// A user from the password database: uid, gid and home.
fn passwd(name: &str) -> Option<(u32, u32, PathBuf)> {
    let c = CString::new(name).ok()?;
    // SAFETY: getpwnam returns NULL or static storage, read at once; the
    // helper is single-threaded.
    unsafe {
        let pw = libc::getpwnam(c.as_ptr());
        if pw.is_null() {
            return None;
        }
        let home = std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy();
        Some(((*pw).pw_uid, (*pw).pw_gid, PathBuf::from(home.as_ref())))
    }
}

fn group_exists(name: &str) -> bool {
    let Ok(c) = CString::new(name) else {
        return false;
    };
    // SAFETY: as for getpwnam.
    unsafe { !libc::getgrnam(c.as_ptr()).is_null() }
}

/// Makes the device user, UID 1000, and sets its password. Run again after
/// a power cut it finishes the same user; any other clash refuses.
fn user_create(name: &str, password: &[u8]) -> Result<(), String> {
    if password.len() < PASSWORD_MIN {
        return Err(format!(
            "the password needs {PASSWORD_MIN} or more characters"
        ));
    }
    if password.contains(&0) {
        return Err("the password has a NUL in it".into());
    }
    let groups: Vec<&str> = USER_GROUPS
        .into_iter()
        .filter(|g| group_exists(g))
        .collect();
    let groups = groups.join(",");
    let taken = auth::user_name(USER_UID);
    match (passwd(name), taken) {
        (Some((uid, ..)), _) if uid != USER_UID => return Err(format!("{name} is taken")),
        (None, Some(other)) => return Err(format!("UID {USER_UID} is {other} already")),
        (None, None) if group_exists(name) => return Err(format!("{name} is a group already")),
        (None, None) => run(&[
            "useradd",
            "-m",
            "-u",
            "1000",
            "-U",
            "-s",
            "/bin/bash",
            "-G",
            &groups,
            name,
        ])?,
        (Some(_), _) => run(&["usermod", "-a", "-G", &groups, name])?,
    }
    chpasswd(name, password)?;
    let (uid, gid, home) = passwd(name).ok_or_else(|| format!("{name}: not made"))?;
    seed_home(&home, uid, gid)
}

fn chpasswd(name: &str, password: &[u8]) -> Result<(), String> {
    let mut child = Command::new("chpasswd")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("chpasswd: {e}"))?;
    let mut line = Vec::with_capacity(name.len() + password.len() + 2);
    line.extend_from_slice(name.as_bytes());
    line.push(b':');
    line.extend_from_slice(password);
    line.push(b'\n');
    let wrote = child.stdin.take().map(|mut i| i.write_all(&line));
    auth::wipe(&mut line);
    let ok = child.wait().is_ok_and(|s| s.success());
    match wrote {
        Some(Ok(())) if ok => Ok(()),
        _ => Err("chpasswd failed".into()),
    }
}

/// Copies what setup gathered into the new home: the look in config.json.
fn seed_home(home: &Path, uid: u32, gid: u32) -> Result<(), String> {
    let config = Path::new(SETUP_HOME).join(".config/cardputerzero/config.json");
    if config.exists() {
        let dir = owned_dirs(home, &[".config", "cardputerzero"], 0o755, uid, gid)?;
        copy_owned(&config, &dir.join("config.json"), 0o644, uid, gid)?;
    }
    Ok(())
}

/// `base/a/b`, each part made if missing and given to the user.
fn owned_dirs(
    base: &Path,
    parts: &[&str],
    mode: u32,
    uid: u32,
    gid: u32,
) -> Result<PathBuf, String> {
    let mut dir = base.to_path_buf();
    for p in parts {
        dir.push(p);
        let io = |e: std::io::Error| format!("{}: {e}", dir.display());
        if !dir.is_dir() {
            std::fs::DirBuilder::new()
                .mode(mode)
                .create(&dir)
                .map_err(io)?;
        }
        std::os::unix::fs::chown(&dir, Some(uid), Some(gid)).map_err(io)?;
    }
    Ok(dir)
}

fn copy_owned(from: &Path, to: &Path, mode: u32, uid: u32, gid: u32) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", to.display());
    std::fs::copy(from, to).map_err(io)?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode)).map_err(io)?;
    std::os::unix::fs::chown(to, Some(uid), Some(gid)).map_err(io)
}

/// Hands the device to the user. The order makes every power cut safe: the
/// linger file is on disk before the marker goes, and the next boot runs
/// setup while the marker is there, the shell once it's gone.
fn setup_finish() -> Result<(), String> {
    let name = auth::user_name(USER_UID).ok_or("no user made yet")?;
    std::fs::create_dir_all(LINGER_DIR).map_err(|e| format!("{LINGER_DIR}: {e}"))?;
    let linger = Path::new(LINGER_DIR).join(&name);
    let io = |e: std::io::Error| format!("{}: {e}", linger.display());
    std::fs::File::create(&linger)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    sync_dir(&linger);
    std::fs::remove_file(SETUP_MARKER).map_err(|e| format!("{SETUP_MARKER}: {e}"))?;
    sync_dir(Path::new(SETUP_MARKER));
    let _ = std::fs::remove_file(SETUP_STATE);
    let _ = std::fs::remove_dir_all(SETUP_HOME);
    run(&["systemctl", "start", "user@1000.service"])
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
            // The boot link without Debian's SysV sync (a Perl run, 2.6 s
            // on the CM0) or a manager reload (1.25 s). Boot reads the
            // links on disk; the running manager keeps the old one, which
            // matters only if multi-user.target is started again.
            let link = |verb: &str| {
                let ok = Command::new("systemctl")
                    .args(["--no-reload", verb, "ssh"])
                    .env("SYSTEMCTL_SKIP_SYSV", "1")
                    .status()
                    .is_ok_and(|s| s.success());
                if ok {
                    Ok(())
                } else {
                    Err(format!("systemctl {verb} ssh failed"))
                }
            };
            if on {
                // Started first, so a failed start leaves it off; and
                // stopped again if the link fails, so it never runs
                // without coming back at boot.
                run(&["systemctl", "start", "ssh"])?;
                link("enable").inspect_err(|_| {
                    let _ = run(&["systemctl", "stop", "ssh"]);
                })
            } else {
                // Both, so a failed stop still leaves it off after a boot.
                let unlinked = link("disable");
                run(&["systemctl", "stop", "ssh"])?;
                unlinked
            }
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
        Verb::SetTime(t) => set_time(t),
        Verb::Ntp(on) => run(&["timedatectl", "set-ntp", if on { "true" } else { "false" }]),
        Verb::DhcpHostname(send) => write_nm(
            sysconf::NM_HOSTNAME_DROPIN,
            &sysconf::dhcp_hostname_conf(send),
        ),
        Verb::Ip6Privacy(on) => write_nm(
            sysconf::NM_IP6_PRIVACY_DROPIN,
            &sysconf::ip6_privacy_conf(on),
        ),
        Verb::Dns(servers) => set_dns(servers.as_deref()),
        Verb::UserCreate(name) => {
            let mut password = auth::read_password();
            let r = user_create(&name, &password);
            auth::wipe(&mut password);
            r
        }
        Verb::SetupFinish => setup_finish(),
        Verb::Rfkill(radio, on) => rfkill::set(radio, !on).map_err(|e| format!("/dev/rfkill: {e}")),
        Verb::UsbPower(on) => usb_power(on),
        Verb::UsbHid => usb_hid_hold(),
        // main runs it before auth, as it reads its own line.
        Verb::FilesWorker => Err("files-worker runs from main".into()),
        Verb::BtClassic(on) => bt_classic(on),
        Verb::WifiPowersave(on) => {
            write_nm(
                sysconf::NM_POWERSAVE_DROPIN,
                &sysconf::wifi_powersave_conf(on),
            )?;
            // The link up now takes it at once. An access point has none.
            if !gh0st_core::status::is_ap(hotspot::IFACE) {
                let _ = run(&[
                    "iw",
                    "dev",
                    hotspot::IFACE,
                    "set",
                    "power_save",
                    if on { "on" } else { "off" },
                ]);
            }
            Ok(())
        }
        Verb::Hotspot(on) => hotspot_on(on),
        Verb::HotspotName(name) => hotspot_edit(|p| p.name = name),
        Verb::HotspotPassword => {
            let mut new = auth::read_password();
            let r = match String::from_utf8(std::mem::take(&mut new)) {
                Ok(psk) if hotspot::valid_password(&psk) => hotspot_edit(|p| p.psk = psk),
                Ok(mut psk) => {
                    // SAFETY: zeroes keep the string valid UTF-8.
                    gh0st_core::password::wipe(unsafe { psk.as_bytes_mut() });
                    Err("8 to 63 letters, digits or symbols, no spaces".into())
                }
                Err(e) => {
                    gh0st_core::password::wipe(&mut e.into_bytes());
                    Err("8 to 63 letters, digits or symbols, no spaces".into())
                }
            };
            auth::wipe(&mut new);
            r
        }
        Verb::HotspotSecurity(s) => hotspot_edit(|p| p.security = s),
        // -i: the user held a chip for it, or the battery is out; an
        // inhibitor must not stop either.
        Verb::Power(reboot) => run(&[
            "systemctl",
            if reboot { "reboot" } else { "poweroff" },
            "-i",
        ]),
        Verb::UpdateCheck => run(&[
            "systemctl",
            "start",
            "--no-block",
            "gh0st-update-check.service",
        ]),
        Verb::UpdateInstall => run(&[
            "systemctl",
            "start",
            "--no-block",
            "gh0st-update-install.service",
        ]),
        Verb::RootPlace(stem, pid) => {
            let caller = std::env::var("PKEXEC_UID")
                .ok()
                .and_then(|v| v.parse().ok())
                .ok_or("no caller")?;
            crate::root::place(&stem, pid, caller)
        }
        Verb::RootGrant(stem, on) => crate::root::grant(&stem, on),
        Verb::UpdateAutoremove => run(&[
            "systemctl",
            "start",
            "--no-block",
            "gh0st-update-autoremove.service",
        ]),
        Verb::UpdateStop => {
            // An install hears SIGTERM itself and stops only while it
            // downloads; once dpkg runs it finishes.
            let _ = run(&[
                "systemctl",
                "kill",
                "--kill-whom=main",
                "--signal=TERM",
                "gh0st-update-install.service",
            ]);
            run(&[
                "systemctl",
                "stop",
                "--no-block",
                "gh0st-update-check.service",
                "gh0st-update-daily.service",
            ])
        }
        Verb::UpdateAuto(on) => run(&[
            "systemctl",
            if on { "enable" } else { "disable" },
            "--now",
            "gh0st-update-daily.timer",
        ]),
        Verb::UpdateChannel(edge) => {
            set_channel(edge)?;
            run(&[
                "systemctl",
                "start",
                "--no-block",
                "gh0st-update-check.service",
            ])
        }
        Verb::AptClean => run(&["apt-get", "clean"]),
        Verb::StoreRefresh => store_job(Job::Refresh, Mirror::Auto),
        Verb::StoreInstall(m, pkgs) => {
            listed(&pkgs)?;
            store_job(Job::Install(pkgs), m)
        }
        Verb::StoreRemove(p) => {
            listed(std::slice::from_ref(&p))?;
            store_job(Job::Remove(p), Mirror::Auto)
        }
        Verb::StoreUpgrade(m) => store_job(Job::Upgrade, m),
    }
}

/// Every name must be an app in the verified catalog: the Store installs
/// only what it lists.
fn listed(pkgs: &[String]) -> Result<(), String> {
    let c = store::Catalog::load().ok_or("no app list yet")?;
    match pkgs
        .iter()
        .map(|p| store::split_seen(p).0)
        .find(|p| c.find(p).is_none())
    {
        Some(p) => Err(format!("{p} isn't in the app list")),
        None => Ok(()),
    }
}

/// Writes the job and starts the Store's unit. One run at a time: the app
/// queues the rest.
fn store_job(job: Job, mirror: Mirror) -> Result<(), String> {
    if gh0st_core::update::engine_running() {
        return Err("an install is running".into());
    }
    let dir = store::run_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = Request { job, mirror }.text();
    gh0st_core::update::write_atomic(&store::job_path(), &text)
        .map_err(|e| format!("{}: {e}", store::job_path().display()))?;
    run(&["systemctl", "start", "--no-block", "gh0st-store.service"])
}

/// Edge adds a second source: the stable one with `Suites: edge`.
fn set_channel(edge: bool) -> Result<(), String> {
    if !edge {
        return match std::fs::remove_file(EDGE_SOURCES) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{EDGE_SOURCES}: {e}"))
            }
            _ => Ok(()),
        };
    }
    let stable =
        std::fs::read_to_string(STABLE_SOURCES).map_err(|e| format!("{STABLE_SOURCES}: {e}"))?;
    let text = edge_sources(&stable).ok_or("the Gh0st OS source has no Suites line")?;
    write_conf(EDGE_SOURCES, &text)
}

/// The stable stanza with its suite swapped.
fn edge_sources(stable: &str) -> Option<String> {
    let mut found = false;
    let mut out = String::from("# Settings > System > Updates > Channel: every test build.\n");
    for line in stable.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        // deb822 field names ignore case.
        if line
            .get(..7)
            .is_some_and(|f| f.eq_ignore_ascii_case("suites:"))
        {
            found = true;
            out.push_str("Suites: edge\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    found.then_some(out)
}

pub fn main(args: &[String]) -> i32 {
    let Some(v) = parse(args) else {
        eprintln!(
            "usage: gh0st-helper logs off|ram|persistent | ssh on|off | gpu-mem 16..256 | \
             regdom XX | cpu balanced|power-saver|performance | hostname NAME | swap MB | \
             swap-comp zstd|lz4|lzo-rle | swappiness 0..200 | set-time EPOCH | ntp on|off | \
             dhcp-hostname on|off | \
             ip6-privacy on|off | dns auto|SERVERS | rfkill bluetooth|wlan on|off | usb-power on|off | usb-hid hold | bt-classic on|off | \
             wifi-powersave on|off | hotspot on|off | hotspot name NAME | hotspot password | \
             hotspot security wpa3|mixed | power off|reboot | update-check | update-install | update-autoremove | root-place STEM PID | root-grant STEM on|off | \
             update-stop | update-auto on|off | update-channel stable|edge | apt-clean | \
             store-refresh | store-install [--mirror auto|github|china] PKG... | \
             store-remove PKG | store-upgrade [--mirror auto|github|china] | \
             user-create NAME | setup-finish | files-worker"
        );
        return 2;
    };
    if v == Verb::FilesWorker {
        return files_worker();
    }
    if v.setup_only() && !in_setup() {
        eprintln!("gh0st-helper: only during setup");
        return 13;
    }
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
    fn bt_classic_takes_on_or_off() {
        assert_eq!(parse(&args("bt-classic off")), Some(Verb::BtClassic(false)));
        assert!(!Verb::BtClassic(true).gated());
        assert_eq!(parse(&args("bt-classic")), None);
    }

    #[test]
    fn usb_hid_holds_and_asks_no_password() {
        let v = parse(&args("usb-hid hold"));
        assert_eq!(v, Some(Verb::UsbHid));
        assert!(!Verb::UsbHid.gated());
        assert_eq!(parse(&args("usb-hid")), None);
        assert_eq!(parse(&args("usb-hid hold now")), None);
    }

    #[test]
    fn store_verbs_take_package_names_only() {
        assert_eq!(parse(&args("store-refresh")), Some(Verb::StoreRefresh));
        assert_eq!(
            parse(&args(
                "store-install --mirror china acidzero zsnake acidzero"
            )),
            Some(Verb::StoreInstall(
                Mirror::China,
                vec!["acidzero".into(), "zsnake".into()]
            ))
        );
        assert_eq!(
            parse(&args("store-install acidzero")),
            Some(Verb::StoreInstall(Mirror::Auto, vec!["acidzero".into()]))
        );
        assert_eq!(
            parse(&args("store-install acidzero=1:0.2.0~rc1+gh0st-1 acidzero")),
            Some(Verb::StoreInstall(
                Mirror::Auto,
                vec!["acidzero=1:0.2.0~rc1+gh0st-1".into()]
            ))
        );
        assert_eq!(
            parse(&args("store-upgrade --mirror github")),
            Some(Verb::StoreUpgrade(Mirror::GitHub))
        );
        assert_eq!(
            parse(&args("store-remove acidzero")),
            Some(Verb::StoreRemove("acidzero".into()))
        );
        for bad in [
            "store-install",
            "store-install --mirror mars acidzero",
            "store-install ../x",
            "store-install -y",
            "store-install Acid",
            "store-install acidzero=",
            "store-install acidzero=v1",
            "store-install acidzero=1;reboot",
            "store-remove a b",
            "store-upgrade now",
        ] {
            assert_eq!(parse(&args(bad)), None, "{bad}");
        }
        assert!(Verb::StoreInstall(Mirror::Auto, vec![]).gated());
        assert!(!Verb::StoreRefresh.gated());
    }

    #[test]
    fn the_files_worker_takes_no_arguments_and_needs_the_password() {
        assert_eq!(parse(&args("files-worker")), Some(Verb::FilesWorker));
        assert_eq!(parse(&args("files-worker /bin/sh")), None);
        assert_eq!(parse(&args("files-worker --root")), None);
        assert!(Verb::FilesWorker.gated());
        assert!(!Verb::FilesWorker.setup_only());
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
        assert_eq!(
            parse(&args("cpu power-saver")),
            Some(Verb::Cpu(CpuMode::PowerSaver))
        );
        assert_eq!(
            parse(&args("hostname RZR-ZERO")),
            Some(Verb::Hostname("RZR-ZERO".into()))
        );
        assert_eq!(parse(&args("swap 0")), Some(Verb::Swap(0)));
        assert_eq!(parse(&args("swap 256")), Some(Verb::Swap(256)));
        assert_eq!(parse(&args("swap-comp lz4")), Some(Verb::SwapComp("lz4")));
        assert_eq!(parse(&args("swappiness 0")), Some(Verb::Swappiness(0)));
        assert_eq!(parse(&args("swappiness 200")), Some(Verb::Swappiness(200)));
        assert_eq!(
            parse(&args("set-time 1795000000")),
            Some(Verb::SetTime(1_795_000_000))
        );
        assert_eq!(parse(&args("ntp off")), Some(Verb::Ntp(false)));
        assert_eq!(
            parse(&args("dhcp-hostname on")),
            Some(Verb::DhcpHostname(true))
        );
        assert_eq!(
            parse(&args("ip6-privacy off")),
            Some(Verb::Ip6Privacy(false))
        );
        assert_eq!(parse(&args("dns auto")), Some(Verb::Dns(None)));
        assert_eq!(
            parse(&args("dns 9.9.9.9,2620:fe::fe")),
            Some(Verb::Dns(Some("9.9.9.9,2620:fe::fe".into())))
        );
        assert_eq!(
            parse(&args("user-create alex")),
            Some(Verb::UserCreate("alex".into()))
        );
        assert_eq!(parse(&args("setup-finish")), Some(Verb::SetupFinish));
        assert_eq!(parse(&args("update-check")), Some(Verb::UpdateCheck));
        assert_eq!(
            parse(&args("update-auto off")),
            Some(Verb::UpdateAuto(false))
        );
        assert_eq!(
            parse(&args("update-channel edge")),
            Some(Verb::UpdateChannel(true))
        );
        assert_eq!(parse(&args("apt-clean")), Some(Verb::AptClean));
        assert_eq!(
            parse(&args("update-autoremove")),
            Some(Verb::UpdateAutoremove)
        );
        assert_eq!(
            parse(&args("wifi-powersave off")),
            Some(Verb::WifiPowersave(false))
        );
        assert_eq!(parse(&args("hotspot on")), Some(Verb::Hotspot(true)));
        assert_eq!(parse(&args("hotspot off")), Some(Verb::Hotspot(false)));
        assert_eq!(
            parse(&args("hotspot name zero-abcd")),
            Some(Verb::HotspotName("zero-abcd".into()))
        );
        assert_eq!(
            parse(&["hotspot".into(), "name".into(), "Галя WiFi".into()]),
            Some(Verb::HotspotName("Галя WiFi".into()))
        );
        assert_eq!(
            parse(&args("hotspot password")),
            Some(Verb::HotspotPassword)
        );
        assert_eq!(
            parse(&args("hotspot security mixed")),
            Some(Verb::HotspotSecurity(Security::Mixed))
        );
        for bad in [
            vec!["hotspot", "name", ""],
            vec!["hotspot", "name", &"a".repeat(33)],
            vec!["hotspot", "name", "new\nline"],
            vec!["hotspot", "name", " lead"],
        ] {
            let a: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            assert_eq!(parse(&a), None, "{bad:?}");
        }
        for bad in [
            "hotspot",
            "hotspot maybe",
            "hotspot on now",
            "hotspot name",
            "hotspot name a b",
            "hotspot password hunter22",
            "hotspot security wpa2",
            "hotspot security open",
            "hotspot security",
            "wifi-powersave",
            "wifi-powersave 2",
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
            "set-time",
            "set-time 0",
            "set-time 01795000000",
            "set-time -5",
            "set-time 1600000000",
            "set-time 9999999999",
            "set-time 1795000000.5",
            "ntp maybe",
            "mac random",
            "scan-mac random",
            "dns",
            "dns 9.9.9.9;reboot",
            "dns ../etc",
            "dns 1.1.1.1,1.0.0.1,9.9.9.9,8.8.8.8",
            "dhcp-hostname yes",
            "ip6-privacy 2",
            "user-create",
            "user-create Alex",
            "user-create root:x",
            "user-create ../x",
            "user-create alex extra",
            "setup-finish now",
            "update-check now",
            "update-install all",
            "update-auto daily",
            "update-channel testing",
            "update-channel",
            "apt-clean all",
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
            ("set-time 1795000000", true),
            ("ntp on", true),
            ("dhcp-hostname off", true),
            ("ip6-privacy on", true),
            ("dns auto", true),
            ("dns 9.9.9.9", true),
            ("rfkill wlan on", false),
            ("usb-power on", false),
            ("wifi-powersave on", false),
            ("hotspot on", false),
            ("hotspot off", false),
            ("hotspot name zero", false),
            ("hotspot password", false),
            ("hotspot security wpa3", false),
            ("power off", false),
            ("user-create alex", false),
            ("setup-finish", false),
            ("update-check", false),
            ("update-stop", false),
            ("apt-clean", false),
            ("update-install", true),
            ("update-autoremove", true),
            ("update-auto on", true),
            ("update-channel edge", true),
        ] {
            assert_eq!(parse(&args(verb)).unwrap().gated(), gated, "{verb}");
        }
    }

    #[test]
    fn edge_is_the_stable_source_with_its_suite_swapped() {
        let stable = "# Gh0st OS\nTypes: deb\nURIs: https://x/apt\nSuites: stable\nComponents: main\nSigned-By: /k.pgp\n";
        let edge = edge_sources(stable).unwrap();
        assert!(edge.contains("URIs: https://x/apt\nSuites: edge\nComponents: main\n"));
        assert!(!edge.contains("stable"));
        assert_eq!(edge_sources("Types: deb\n"), None);
        let lower = edge_sources("Types: deb\nsuites: stable\n").unwrap();
        assert!(lower.contains("Suites: edge\n") && !lower.contains("stable"));
    }

    #[test]
    fn only_making_the_user_and_finishing_are_setup_verbs() {
        for verb in ["user-create alex", "setup-finish"] {
            assert!(parse(&args(verb)).unwrap().setup_only(), "{verb}");
        }
        for verb in ["hostname zero", "ssh on", "power off", "ip6-privacy on"] {
            assert!(!parse(&args(verb)).unwrap().setup_only(), "{verb}");
        }
    }

    #[test]
    fn a_short_password_is_refused_before_anything_runs() {
        assert!(
            user_create("alex", b"abc")
                .unwrap_err()
                .contains("4 or more")
        );
        assert!(user_create("alex", b"ab\0cd").unwrap_err().contains("NUL"));
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
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            line.replace("AE", "GB")
        );
        assert_eq!(
            std::fs::read_to_string(p.with_extension("bak")).unwrap(),
            line
        );
        assert!(!p.with_extension("new").exists());
    }

    #[test]
    fn a_longer_edit_goes_through_a_rename() {
        let p = scratch("config.txt", "arm_64bit=1\n");
        write_boot(&p, |t| sysconf::set_gpu_mem(t, 64)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "gpu_mem=64\narm_64bit=1\n"
        );
        assert!(!p.with_extension("new").exists());
        write_boot(&p, |_| Ok("x".repeat(200))).unwrap_err();
        assert!(write_boot(&p, |_| Err("set by hand")).is_err());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "gpu_mem=64\narm_64bit=1\n"
        );
    }
}
