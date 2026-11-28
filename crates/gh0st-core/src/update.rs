//! What the update engine (`gh0st update`, root, from systemd) tells the
//! Updates app, Settings and the shell: two small files they watch with
//! inotify. `docs/updates.md` has the whole design.
//!
//! - `/var/lib/gh0st/update/state`: the last check and install, and what is
//!   new. `key=value` lines, then one tab-separated line per package.
//! - `/run/gh0st-update/progress`: one line while the engine runs.

use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = "/var/lib/gh0st/update";
pub const RUN_DIR: &str = "/run/gh0st-update";

/// The state folder, or `$GH0ST_UPDATE_STATE` for tests and the sim.
pub fn state_dir() -> PathBuf {
    std::env::var_os("GH0ST_UPDATE_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(STATE_DIR))
}

/// The edge suite's source. The channel is edge while it exists.
pub const EDGE_SOURCES: &str = "/etc/apt/sources.list.d/gh0st-edge.sources";

/// The channel as set now, from the edge source, or `$GH0ST_UPDATE_EDGE`
/// in tests and the sim. The last check's channel may be stale: a change
/// starts a check, which can wait on apt or fail.
pub fn channel_now() -> String {
    let path = std::env::var_os("GH0ST_UPDATE_EDGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(EDGE_SOURCES));
    if path.exists() { "edge" } else { "stable" }.into()
}

/// The run folder, or `$GH0ST_UPDATE_RUN`.
pub fn run_dir() -> PathBuf {
    std::env::var_os("GH0ST_UPDATE_RUN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(RUN_DIR))
}

/// A check is owed: the daily one found no network, or a metered one.
pub fn pending_path() -> PathBuf {
    state_dir().join("pending")
}

/// The packages that rewrite the boot partition. With any of them in an
/// install, the battery must be above [`KERNEL_BATTERY`].
pub fn is_kernel_set(name: &str) -> bool {
    matches!(
        name,
        "gh0st-kernel" | "gh0st-panel-modules" | "raspi-firmware"
    ) || name.starts_with("linux-image-")
}

/// An install with the kernel set needs more than this, charging or not.
pub const KERNEL_BATTERY: u8 = 50;
/// Any other install needs this much, or the charger.
pub const MIN_BATTERY: u8 = 30;

/// What a package needs once installed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Restart {
    #[default]
    None,
    /// Gh0st restarts: the shell and Settings run the old binary until then.
    Shell,
    /// The device restarts: the kernel, libc, systemd.
    Device,
}

impl Restart {
    pub fn of(name: &str) -> Restart {
        if is_kernel_set(name)
            || matches!(name, "libc6" | "systemd" | "udev" | "dbus" | "libsystemd0")
        {
            Restart::Device
        } else if name == "gh0st" {
            Restart::Shell
        } else {
            Restart::None
        }
    }

    fn word(self) -> &'static str {
        match self {
            Restart::None => "-",
            Restart::Shell => "shell",
            Restart::Device => "device",
        }
    }

    fn parse(s: &str) -> Restart {
        match s {
            "shell" => Restart::Shell,
            "device" => Restart::Device,
            _ => Restart::None,
        }
    }
}

/// Where a package comes from, as the app groups them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Our repo: gh0st, the kernel set, the apps we ship.
    Gh0st,
    /// Debian's security archive.
    Security,
    #[default]
    System,
}

impl Group {
    /// From apt's origin: `Gh0st`, `Debian-Security`, `Debian`.
    pub fn of_origin(origin: &str) -> Group {
        match origin {
            "Gh0st" => Group::Gh0st,
            "Debian-Security" => Group::Security,
            _ => Group::System,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Group::Gh0st => "gh0st",
            Group::Security => "security",
            Group::System => "system",
        }
    }

    fn parse(s: &str) -> Group {
        match s {
            "gh0st" => Group::Gh0st,
            "security" => Group::Security,
            _ => Group::System,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// The installed version; empty for a new package.
    pub from: String,
    pub to: String,
    /// Download size in bytes.
    pub size: u64,
    pub group: Group,
    pub restart: Restart,
    /// The description's first line.
    pub summary: String,
}

/// The progress item while the engine removes unused packages, so the
/// Updates app can say so even after it restarts.
pub const UNUSED_ITEM: &str = "unused packages";

/// A package nothing needs any more, which `apt-get autoremove` would
/// take: a library an upgrade replaced, or what a removed app brought.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unused {
    pub name: String,
    pub version: String,
    /// Bytes it takes on disk.
    pub size: u64,
    pub summary: String,
}

/// How the last run ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    /// Never checked.
    #[default]
    None,
    Ok,
    /// No network: the check waits for one.
    Offline,
    /// The daily check skipped a metered network.
    Metered,
    /// The clock is behind: apt would reject every signature.
    Clock,
    Space,
    Battery,
    /// The kernel set needs more than 50%.
    BatteryKernel,
    /// Another install holds dpkg's lock: the Store.
    Locked,
    /// Stopped by the user.
    Stopped,
    Error,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::None => "none",
            Outcome::Ok => "ok",
            Outcome::Offline => "offline",
            Outcome::Metered => "metered",
            Outcome::Clock => "clock",
            Outcome::Space => "space",
            Outcome::Battery => "battery",
            Outcome::BatteryKernel => "battery-kernel",
            Outcome::Locked => "locked",
            Outcome::Stopped => "stopped",
            Outcome::Error => "error",
        }
    }

    fn parse(s: &str) -> Outcome {
        match s {
            "ok" => Outcome::Ok,
            "offline" => Outcome::Offline,
            "metered" => Outcome::Metered,
            "clock" => Outcome::Clock,
            "space" => Outcome::Space,
            "battery" => Outcome::Battery,
            "battery-kernel" => Outcome::BatteryKernel,
            "locked" => Outcome::Locked,
            "stopped" => Outcome::Stopped,
            "error" => Outcome::Error,
            _ => Outcome::None,
        }
    }
}

/// The engine's last word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// When the last check ran, seconds since the epoch. 0: never.
    pub checked: u64,
    /// How the last run, check or install, ended.
    pub outcome: Outcome,
    /// What went wrong, in words for the user.
    pub why: String,
    /// `stable` or `edge`.
    pub channel: String,
    pub packages: Vec<Package>,
    /// Hosts of sources the last check went on without: added by someone
    /// else (an app, the user) and failing, unsigned or gone. Ours and
    /// Debian's failing fail the check instead.
    pub skipped: Vec<String>,
    /// What the next install removes, after the upgrade. Not updates: they
    /// count in no notice and wait for one.
    pub unused: Vec<Unused>,
    /// Upgrades apt holds back: half a set, or one that would remove
    /// something.
    pub held: u32,
    /// Bytes the install will take on disk, past what it frees.
    pub grow: u64,
    /// The last install: when, and how many packages.
    pub installed: u64,
    pub installed_count: u32,
    /// What the last install still needs.
    pub restart: Restart,
}

impl State {
    pub fn path() -> PathBuf {
        state_dir().join("state")
    }

    /// The last check's state, with the channel as set now.
    pub fn load() -> State {
        let mut s = State::read(&State::path());
        s.channel = channel_now();
        s
    }

    pub fn read(path: &Path) -> State {
        std::fs::read_to_string(path)
            .map(|t| State::parse(&t))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> State {
        let mut s = State::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("pkg\t") {
                let f: Vec<&str> = rest.split('\t').collect();
                if f.len() >= 7 {
                    s.packages.push(Package {
                        name: f[0].into(),
                        from: f[1].into(),
                        to: f[2].into(),
                        size: f[3].parse().unwrap_or(0),
                        group: Group::parse(f[4]),
                        restart: Restart::parse(f[5]),
                        summary: f[6..].join(" "),
                    });
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("unused\t") {
                let f: Vec<&str> = rest.split('\t').collect();
                if f.len() >= 4 {
                    s.unused.push(Unused {
                        name: f[0].into(),
                        version: f[1].into(),
                        size: f[2].parse().unwrap_or(0),
                        summary: f[3..].join(" "),
                    });
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let num = || v.trim().parse().unwrap_or(0);
            match k {
                "checked" => s.checked = num(),
                "outcome" => s.outcome = Outcome::parse(v.trim()),
                "why" => s.why = v.trim().into(),
                "channel" => s.channel = v.trim().into(),
                "held" => s.held = num() as u32,
                "grow" => s.grow = num(),
                "installed" => s.installed = num(),
                "installed_count" => s.installed_count = num() as u32,
                "restart" => s.restart = Restart::parse(v.trim()),
                "skipped" => s.skipped = v.split_whitespace().map(String::from).collect(),
                _ => {}
            }
        }
        s
    }

    pub fn text(&self) -> String {
        let clean = |s: &str| s.replace(['\n', '\r', '\t'], " ");
        let mut t = format!(
            "checked={}\noutcome={}\nwhy={}\nchannel={}\nheld={}\ngrow={}\ninstalled={}\ninstalled_count={}\nrestart={}\n",
            self.checked,
            self.outcome.word(),
            clean(&self.why),
            clean(&self.channel),
            self.held,
            self.grow,
            self.installed,
            self.installed_count,
            self.restart.word(),
        );
        if !self.skipped.is_empty() {
            t.push_str(&format!("skipped={}\n", clean(&self.skipped.join(" "))));
        }
        for p in &self.packages {
            t.push_str(&format!(
                "pkg\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                clean(&p.name),
                clean(&p.from),
                clean(&p.to),
                p.size,
                p.group.word(),
                p.restart.word(),
                clean(&p.summary),
            ));
        }
        for u in &self.unused {
            t.push_str(&format!(
                "unused\t{}\t{}\t{}\t{}\n",
                clean(&u.name),
                clean(&u.version),
                u.size,
                clean(&u.summary),
            ));
        }
        t
    }

    /// Bytes the unused packages free.
    pub fn frees(&self) -> u64 {
        self.unused.iter().map(|u| u.size).sum()
    }

    /// Bytes to download.
    pub fn download(&self) -> u64 {
        self.packages.iter().map(|p| p.size).sum()
    }

    /// The kernel set is in the list.
    pub fn kernel(&self) -> bool {
        self.packages.iter().any(|p| is_kernel_set(&p.name))
    }

    /// What installing the list will need.
    pub fn needs(&self) -> Restart {
        self.packages
            .iter()
            .map(|p| p.restart)
            .max()
            .unwrap_or_default()
    }

    pub fn count(&self, g: Group) -> usize {
        self.packages.iter().filter(|p| p.group == g).count()
    }
}

/// What the engine is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Lists,
    Download,
    Install,
    Clean,
}

impl Phase {
    fn word(self) -> &'static str {
        match self {
            Phase::Lists => "lists",
            Phase::Download => "download",
            Phase::Install => "install",
            Phase::Clean => "clean",
        }
    }

    fn parse(s: &str) -> Option<Phase> {
        Some(match s {
            "lists" => Phase::Lists,
            "download" => Phase::Download,
            "install" => Phase::Install,
            "clean" => Phase::Clean,
            _ => return None,
        })
    }
}

/// One line of progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// An install, not a check.
    pub install: bool,
    pub phase: Phase,
    /// 0 to 100; None when apt can't say.
    pub percent: Option<u8>,
    /// Bytes downloaded so far, and in all.
    pub done: u64,
    pub total: u64,
    /// Item n of m: a file in a check, a package in an install.
    pub n: u32,
    pub m: u32,
    /// What it works on: a package, or an index's host.
    pub item: String,
}

impl Progress {
    pub fn path() -> PathBuf {
        run_dir().join("progress")
    }

    pub fn parse(text: &str) -> Option<Progress> {
        let line = text.lines().next()?;
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            return None;
        }
        Some(Progress {
            install: f[0] == "install",
            phase: Phase::parse(f[1])?,
            percent: f[2].parse::<u8>().ok().map(|p| p.min(100)),
            done: f[3].parse().unwrap_or(0),
            total: f[4].parse().unwrap_or(0),
            n: f[5].parse().unwrap_or(0),
            m: f[6].parse().unwrap_or(0),
            item: f[7..].join(" "),
        })
    }

    pub fn text(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            if self.install { "install" } else { "check" },
            self.phase.word(),
            self.percent.map_or("-".into(), |p| p.to_string()),
            self.done,
            self.total,
            self.n,
            self.m,
            self.item.replace(['\n', '\t'], " "),
        )
    }

    /// The progress file while the engine runs, None when it doesn't: no
    /// file, or one left by an engine that died (its lock is free).
    pub fn load() -> Option<Progress> {
        if !engine_running() {
            return None;
        }
        Progress::parse(&std::fs::read_to_string(Progress::path()).ok()?)
    }
}

/// The restart the last install asked for has happened: the device booted,
/// or the shell started, after it. True when none is owed.
pub fn restart_done(st: &State) -> bool {
    match st.restart {
        Restart::None => true,
        Restart::Device => boot_time() > st.installed,
        Restart::Shell => boot_time() > st.installed || shell_started_after(st.installed),
    }
}

/// `btime` from /proc/stat: when the device booted, in epoch seconds.
pub fn boot_time() -> u64 {
    std::fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
        .unwrap_or(0)
}

/// A `gh0st shell` process started after `t`, epoch seconds.
pub fn shell_started_after(t: u64) -> bool {
    // SAFETY: sysconf has no failure mode for this name.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    let boot = boot_time();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.flatten().any(|e| {
        let p = e.path();
        let cmd = std::fs::read(p.join("cmdline")).unwrap_or_default();
        let mut words = cmd.split(|&b| b == 0);
        let shell = words.next() == Some(b"/usr/bin/gh0st") && words.next() == Some(b"shell");
        if !shell {
            return false;
        }
        let stat = std::fs::read_to_string(p.join("stat")).unwrap_or_default();
        // starttime is field 22; counted from after the command's ')'.
        let start = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(19)?.parse::<u64>().ok())
            .unwrap_or(0);
        boot + start / hz > t
    })
}

/// The engine holds an exclusive flock on this while it runs. Root's alone
/// (0600): a process that could open it could hold it and stall updates.
pub fn lock_path() -> PathBuf {
    run_dir().join("lock")
}

/// What anyone may read to see the engine running: it holds an exclusive
/// flock on this too, as long as no reader holds one. A reader that does
/// can only make the engine look idle.
pub fn busy_path() -> PathBuf {
    run_dir().join("busy")
}

/// True while an engine holds the lock.
pub fn engine_running() -> bool {
    engine_running_at(&busy_path())
}

pub fn engine_running_at(busy: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(f) = std::fs::File::open(busy) else {
        return false;
    };
    // SAFETY: flock on a descriptor this function owns.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
    if r == 0 {
        // SAFETY: as above.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        false
    } else {
        true
    }
}

/// Writes `data` to `path` through a hidden file and a rename, so a reader
/// never sees half of it.
pub fn write_atomic(path: &Path, data: &str) -> std::io::Result<()> {
    crate::config::write_atomic(path, data.as_bytes())
}

/// The channel's name for the user.
pub fn channel_name(c: &str) -> &'static str {
    if c == "edge" { "Edge" } else { "Stable" }
}

/// "34 MB", "820 kB": a size for the user, in decimal units as apt shows.
pub fn size_text(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{} kB", bytes / 1_000),
        1_000_000..10_000_000 => {
            let tenths = bytes / 100_000;
            format!("{}.{} MB", tenths / 10, tenths % 10)
        }
        _ => format!("{} MB", bytes / 1_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> State {
        State {
            checked: 1_795_369_605,
            outcome: Outcome::Ok,
            why: String::new(),
            channel: "edge".into(),
            packages: vec![
                Package {
                    name: "gh0st".into(),
                    from: "0.4.1".into(),
                    to: "0.5.0".into(),
                    size: 744_972,
                    group: Group::Gh0st,
                    restart: Restart::Shell,
                    summary: "Gh0st OS, the CardputerZero shell".into(),
                },
                Package {
                    name: "gh0st-kernel".into(),
                    from: "6.12.47".into(),
                    to: "6.12.50".into(),
                    size: 21_000_000,
                    group: Group::Gh0st,
                    restart: Restart::Device,
                    summary: "the kernel\tset".into(),
                },
            ],
            skipped: vec!["www.kismetwireless.net".into()],
            unused: vec![Unused {
                name: "libavcodec61".into(),
                version: "7:7.1.1-1".into(),
                size: 12_300_000,
                summary: "FFmpeg's codecs".into(),
            }],
            held: 1,
            grow: 4096,
            installed: 0,
            installed_count: 0,
            restart: Restart::None,
        }
    }

    #[test]
    fn the_state_round_trips_and_tabs_in_text_stay_out() {
        let s = sample();
        let back = State::parse(&s.text());
        assert_eq!(back.packages[1].summary, "the kernel set");
        assert_eq!(back.unused[0].name, "libavcodec61");
        assert_eq!(back.skipped, ["www.kismetwireless.net"]);
        assert_eq!(back.frees(), 12_300_000);
        let mut s = s;
        s.packages[1].summary = "the kernel set".into();
        assert_eq!(back, s);
        assert_eq!(back.download(), 21_744_972);
        assert!(back.kernel());
        assert_eq!(back.needs(), Restart::Device);
        assert_eq!(back.count(Group::Gh0st), 2);
    }

    #[test]
    fn progress_round_trips_and_a_bad_line_is_none() {
        let p = Progress {
            install: true,
            phase: Phase::Install,
            percent: Some(48),
            done: 0,
            total: 34_000_000,
            n: 5,
            m: 12,
            item: "libc6".into(),
        };
        assert_eq!(Progress::parse(&p.text()), Some(p));
        let lists = Progress {
            percent: None,
            ..Progress::default()
        };
        assert_eq!(Progress::parse(&lists.text()), Some(lists));
        assert_eq!(Progress::parse("check\tnope\t1\t2"), None);
    }

    #[test]
    fn the_kernel_set_and_restarts() {
        assert!(is_kernel_set("linux-image-6.18.60+rpt-rpi-v8"));
        assert!(is_kernel_set("raspi-firmware"));
        assert!(!is_kernel_set("firmware-brcm80211"));
        assert_eq!(Restart::of("libc6"), Restart::Device);
        assert_eq!(Restart::of("gh0st"), Restart::Shell);
        assert_eq!(Restart::of("gh0st-term"), Restart::None);
        assert_eq!(
            Restart::of("gh0st-base"),
            Restart::None,
            "its postinst does it"
        );
    }

    #[test]
    fn sizes_read_as_apt_shows_them() {
        assert_eq!(size_text(820), "820 B");
        assert_eq!(size_text(744_972), "744 kB");
        assert_eq!(size_text(2_488_872), "2.4 MB");
        assert_eq!(size_text(34_200_000), "34 MB");
    }
}
