//! The sandbox an app runs in: bubblewrap with its own user, mount and PID
//! namespaces (`docs/isolation.md`). The app keeps its uid, groups and
//! devices, so a stock app runs as it did. What changes:
//!
//! - It sees only its own processes, so it can't trace or signal another
//!   app, or read another's memory.
//! - Secrets are hidden, and what an unsandboxed program later runs or
//!   loads is read-only: shell start files, `~/.local/bin`, user units,
//!   D-Bus and PipeWire config.
//! - Home is read-only at the top. Every entry in it is bound back
//!   writable unless locked, so an app can't make a new start file.
//! - The runtime folder holds only the bus, PipeWire and pulse sockets.
//! - Network, camera and mic go when the user turns them off. Without the
//!   mic an app hears neither it nor what the device plays.
//!
//! No `--die-with-parent`: the shell's SIGTERM would kill bwrap at once
//! and bwrap the app, before the app's own handler could save.

use std::ffi::OsString;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

pub const BWRAP: &str = "/usr/bin/bwrap";

/// What a sandboxed app may reach beyond its files. All by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grants {
    pub network: bool,
    pub camera: bool,
    pub mic: bool,
}

impl Default for Grants {
    fn default() -> Grants {
        Grants {
            network: true,
            camera: true,
            mic: true,
        }
    }
}

/// Home entries an app never sees.
const HIDDEN: [&str; 2] = [".ssh", ".gnupg"];

/// Top-level home files an app may read but not write. bash reads the
/// missing ones too, so they must not be made either.
const LOCKED_TOP: [&str; 14] = [
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".bash_logout",
    ".bash_history",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".inputrc",
    ".pam_environment",
    "bin",
    ".xsessionrc",
];

/// Folders an unsandboxed program runs or loads from. `prepare` makes them,
/// so they exist to be locked; a missing one could be made by an app.
pub const LOCKED_DIRS: [&str; 15] = [
    ".config/systemd",
    ".config/environment.d",
    ".config/autostart",
    ".config/pipewire",
    ".config/wireplumber",
    ".config/pulse",
    ".config/fish",
    ".config/gh0st",
    ".local/bin",
    ".local/lib",
    ".local/share/systemd",
    ".local/share/dbus-1",
    ".local/share/wireplumber",
    ".local/share/pipewire",
    ".local/state/gh0st",
];

/// The keyring's vaults, outside home.
const VAULTS: &str = "/var/lib/gh0st/keyring";

/// The no-mic pulse socket (`gh0st-base`): its clients see no source.
pub const PULSE_NOMIC: &str = "pulse/native-nomic";

/// What the host has, read at each launch: a few `readdir`s.
#[derive(Clone, Debug, Default)]
pub struct Host {
    pub home: PathBuf,
    pub runtime: PathBuf,
    /// Names of home's entries, links left out: they resolve through the
    /// read-only home to wherever they point.
    pub home_entries: Vec<String>,
    /// The camera's nodes: the sensor, unicam, the ISP and their media
    /// devices. The codec's nodes, which Media plays through, are not.
    pub camera_nodes: Vec<PathBuf>,
    /// ALSA capture nodes.
    pub capture_nodes: Vec<PathBuf>,
    /// Every other node in `/dev/snd`: kept when the capture ones go.
    pub sound_nodes: Vec<PathBuf>,
    /// Runtime sockets that exist, relative to `runtime`.
    pub sockets: Vec<String>,
    /// The keyring's vault folder exists, so it can be hidden.
    pub vaults: bool,
}

/// Runtime sockets an app may get.
const SOCKETS: [&str; 4] = ["bus", "pipewire-0", "pulse/native", PULSE_NOMIC];

/// Drivers whose V4L2 and media nodes are the camera.
const CAMERA_DRIVERS: [&str; 5] = ["unicam", "bcm2835-isp", "rp1-cfe", "pisp_be", "uvcvideo"];

impl Host {
    pub fn read(home: &Path, runtime: &Path) -> Host {
        let home_entries = std::fs::read_dir(home)
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| !t.is_symlink()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        let sockets = SOCKETS
            .iter()
            .filter(|s| runtime.join(s).exists())
            .map(|s| s.to_string())
            .collect();
        Host {
            home: home.to_path_buf(),
            runtime: runtime.to_path_buf(),
            home_entries,
            camera_nodes: camera_nodes(Path::new("/sys")),
            capture_nodes: capture_nodes(Path::new("/dev/snd")),
            sound_nodes: sound_nodes(Path::new("/dev/snd")),
            sockets,
            vaults: Path::new(VAULTS).is_dir(),
        }
    }
}

/// Makes the locked folders, so a sandbox can lock them. The shell runs it
/// at start.
pub fn prepare(home: &Path, notify: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    for d in LOCKED_DIRS
        .iter()
        .map(|d| home.join(d))
        .chain([notify.to_path_buf()])
    {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(d);
    }
}

/// The camera's device nodes, by driver, from sysfs under `sys`.
pub fn camera_nodes(sys: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for class in ["class/video4linux", "bus/media/devices"] {
        let Ok(dir) = std::fs::read_dir(sys.join(class)) else {
            continue;
        };
        for e in dir.flatten() {
            let p = e.path();
            let driver = std::fs::read_link(p.join("device/driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
            let model = std::fs::read_to_string(p.join("model")).unwrap_or_default();
            let is_camera = |n: &str| CAMERA_DRIVERS.iter().any(|d| n.trim() == *d);
            let sensor = e.file_name().to_string_lossy().starts_with("v4l-subdev");
            if sensor || driver.as_deref().is_some_and(is_camera) || is_camera(&model) {
                out.push(Path::new("/dev").join(e.file_name()));
            }
        }
    }
    out.sort();
    out
}

/// ALSA capture nodes in `snd`: `pcmC<card>D<dev>c`.
pub fn capture_nodes(snd: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(snd)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("pcmC") && n.ends_with('c'))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The nodes in `snd` that aren't for capture: playback, controls, the
/// sequencer and the timer. Folders and links are left out.
pub fn sound_nodes(snd: &Path) -> Vec<PathBuf> {
    let capture = capture_nodes(snd);
    let mut out: Vec<PathBuf> = std::fs::read_dir(snd)
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_char_device()))
                .map(|e| e.path())
                .filter(|p| !capture.contains(p))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Where a sandboxed app's notices go on the host: its own folder, bound
/// over the shared one, so it can't post as another app.
pub fn notice_dir(notify: &Path, stem: &str) -> PathBuf {
    notify.join(stem)
}

/// The bwrap command that runs `inner` for app `stem` with `grants`.
/// `notify` is the shared notice folder; the app's own one is bound there.
pub fn argv(
    host: &Host,
    grants: Grants,
    stem: &str,
    notify: &Path,
    inner: &[OsString],
) -> Vec<OsString> {
    let mut a = Args(Vec::new());
    a.raw(BWRAP);
    a.raw("--unshare-user");
    a.raw("--unshare-pid");
    a.raw("--unshare-ipc");
    a.raw("--disable-userns");
    a.pair("--dev-bind", "/", "/");
    a.one("--proc", "/proc");
    // An app that can't name its cgroup can't leave it.
    a.one("--tmpfs", "/sys/fs/cgroup");
    a.one("--remount-ro", "/sys/fs/cgroup");
    if host.vaults {
        a.one("--tmpfs", VAULTS);
        a.one("--remount-ro", VAULTS);
    }
    runtime(&mut a, host, grants);
    home(&mut a, host, stem, notify);
    if !grants.network {
        a.raw("--unshare-net");
        // No system bus either: NetworkManager and BlueZ reach out too.
        a.one("--tmpfs", "/run/dbus");
    }
    if !grants.camera {
        for n in &host.camera_nodes {
            a.pair("--dev-bind", "/dev/null", n);
        }
    }
    if !grants.mic && !host.capture_nodes.is_empty() {
        // Gone, not stood in for: ALSA then sees a card with no capture,
        // as on a device without a mic.
        a.one("--tmpfs", "/dev/snd");
        for n in &host.sound_nodes {
            a.pair("--dev-bind", n, n);
        }
    }
    a.raw("--");
    a.0.extend(inner.iter().cloned());
    a.0
}

fn runtime(a: &mut Args, host: &Host, grants: Grants) {
    let r = &host.runtime;
    if r.as_os_str().is_empty() {
        return;
    }
    a.one("--tmpfs", r);
    let has = |s: &str| host.sockets.iter().any(|x| x == s);
    if has("bus") {
        a.pair("--bind", r.join("bus"), r.join("bus"));
    }
    // Off, the mic takes the pulse socket that hides sources, and no
    // PipeWire socket: there every client could record.
    if grants.mic && has("pipewire-0") {
        a.pair("--bind", r.join("pipewire-0"), r.join("pipewire-0"));
    }
    let pulse = if grants.mic {
        "pulse/native"
    } else {
        PULSE_NOMIC
    };
    if has(pulse) {
        a.one("--dir", r.join("pulse"));
        a.pair("--bind", r.join(pulse), r.join("pulse/native"));
    }
}

fn home(a: &mut Args, host: &Host, stem: &str, notify: &Path) {
    let h = &host.home;
    if h.as_os_str().is_empty() {
        return;
    }
    a.pair("--ro-bind", h, h);
    for name in &host.home_entries {
        if HIDDEN.contains(&name.as_str()) || LOCKED_TOP.contains(&name.as_str()) {
            continue;
        }
        a.pair("--bind", h.join(name), h.join(name));
    }
    for d in LOCKED_DIRS {
        let p = h.join(d);
        a.pair("--ro-bind-try", &p, &p);
    }
    // The app's own notice folder, writable, where it posts.
    if notify.starts_with(h) {
        a.pair("--bind-try", notice_dir(notify, stem), notify);
    }
    for name in HIDDEN {
        if host.home_entries.iter().any(|e| e == name) {
            a.one("--tmpfs", h.join(name));
        }
    }
    if host.home_entries.iter().any(|e| e == ".bash_history") {
        a.pair("--ro-bind", "/dev/null", h.join(".bash_history"));
    }
}

/// The shell's cgroup, when its unit delegates it (`Delegate=yes`): each
/// app gets `app-<stem>` right under it, which names the app to the
/// keyring and the session (`ident`).
pub struct Cgroups {
    unit: PathBuf,
}

impl Cgroups {
    /// The shell's own cgroup, if it may make cgroups there.
    pub fn own() -> Option<Cgroups> {
        let body = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        let rel = body.lines().find_map(|l| l.strip_prefix("0::"))?;
        let unit = Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
        let probe = unit.join("app-gh0st-probe");
        let ok = std::fs::create_dir(&probe).is_ok() || probe.is_dir();
        let _ = std::fs::remove_dir(&probe);
        ok.then_some(Cgroups { unit })
    }

    /// App `stem`'s cgroup, made if missing. None for a name a cgroup
    /// can't take, or when the kernel refuses.
    pub fn make(&self, stem: &str) -> Option<PathBuf> {
        let ok = !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !ok {
            return None;
        }
        let d = self.unit.join(format!("app-{stem}"));
        (std::fs::create_dir(&d).is_ok() || d.is_dir()).then_some(d)
    }

    /// Drops app `stem`'s cgroup once empty. The kernel refuses while a
    /// process is left in it.
    pub fn remove(&self, stem: &str) {
        let _ = std::fs::remove_dir(self.unit.join(format!("app-{stem}")));
    }
}

struct Args(Vec<OsString>);

impl Args {
    fn raw(&mut self, s: impl Into<OsString>) {
        self.0.push(s.into());
    }

    fn one(&mut self, flag: &str, p: impl AsRef<Path>) {
        self.raw(flag);
        self.raw(p.as_ref().as_os_str());
    }

    fn pair(&mut self, flag: &str, from: impl AsRef<Path>, to: impl AsRef<Path>) {
        self.raw(flag);
        self.raw(from.as_ref().as_os_str());
        self.raw(to.as_ref().as_os_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            home: "/home/u".into(),
            runtime: "/run/user/1000".into(),
            home_entries: [
                ".bashrc",
                ".ssh",
                ".config",
                ".local",
                "Documents",
                ".bash_history",
                "bin",
            ]
            .map(String::from)
            .to_vec(),
            camera_nodes: vec!["/dev/video0".into(), "/dev/media1".into()],
            capture_nodes: vec!["/dev/snd/pcmC1D0c".into()],
            sound_nodes: vec!["/dev/snd/controlC1".into(), "/dev/snd/pcmC1D0p".into()],
            sockets: ["bus", "pipewire-0", "pulse/native", PULSE_NOMIC]
                .map(String::from)
                .to_vec(),
            vaults: true,
        }
    }

    fn run(g: Grants) -> Vec<String> {
        argv(
            &host(),
            g,
            "zsynth",
            Path::new("/home/u/.local/state/gh0st/notify"),
            &["/bin/sh".into(), "-c".into(), "exec zsynth".into()],
        )
        .into_iter()
        .map(|s| s.into_string().unwrap())
        .collect()
    }

    /// The flag and its arguments, `n` of them, wherever it appears.
    fn has(v: &[String], want: &[&str]) -> bool {
        v.windows(want.len()).any(|w| w == want)
    }

    #[test]
    fn the_app_runs_last_after_the_separator() {
        let v = run(Grants::default());
        let sep = v.iter().position(|s| s == "--").unwrap();
        assert_eq!(v[sep + 1..], ["/bin/sh", "-c", "exec zsynth"]);
        assert_eq!(v[0], BWRAP);
        assert!(!v.iter().any(|s| s == "--die-with-parent"));
    }

    #[test]
    fn home_is_read_only_with_entries_bound_back_but_not_locked_ones() {
        let v = run(Grants::default());
        assert!(has(&v, &["--ro-bind", "/home/u", "/home/u"]));
        assert!(has(
            &v,
            &["--bind", "/home/u/Documents", "/home/u/Documents"]
        ));
        assert!(has(&v, &["--bind", "/home/u/.config", "/home/u/.config"]));
        assert!(!has(&v, &["--bind", "/home/u/.bashrc", "/home/u/.bashrc"]));
        assert!(!has(&v, &["--bind", "/home/u/bin", "/home/u/bin"]));
        assert!(!has(&v, &["--bind", "/home/u/.ssh", "/home/u/.ssh"]));
        assert!(has(&v, &["--tmpfs", "/home/u/.ssh"]));
        assert!(has(
            &v,
            &["--ro-bind", "/dev/null", "/home/u/.bash_history"]
        ));
        assert!(has(
            &v,
            &[
                "--ro-bind-try",
                "/home/u/.config/systemd",
                "/home/u/.config/systemd"
            ]
        ));
        assert!(has(
            &v,
            &["--ro-bind-try", "/home/u/.local/bin", "/home/u/.local/bin"]
        ));
    }

    #[test]
    fn locked_folders_come_after_the_entries_that_hold_them() {
        let v = run(Grants::default());
        let config = v.iter().position(|s| s == "/home/u/.config").unwrap();
        let systemd = v
            .iter()
            .position(|s| s == "/home/u/.config/systemd")
            .unwrap();
        assert!(config < systemd, "a later bind would cover the lock");
    }

    #[test]
    fn notices_go_to_the_apps_own_folder() {
        let v = run(Grants::default());
        let n = "/home/u/.local/state/gh0st/notify";
        assert!(has(&v, &["--bind-try", &format!("{n}/zsynth"), n]));
        let lock = v
            .iter()
            .position(|s| s == "/home/u/.local/state/gh0st")
            .unwrap();
        let own = v.iter().position(|s| s == &format!("{n}/zsynth")).unwrap();
        assert!(lock < own);
    }

    #[test]
    fn everything_granted_keeps_network_camera_and_mic() {
        let v = run(Grants::default());
        assert!(!v.iter().any(|s| s == "--unshare-net"));
        assert!(!v.iter().any(|s| s == "/dev/video0"));
        assert!(has(
            &v,
            &[
                "--bind",
                "/run/user/1000/pulse/native",
                "/run/user/1000/pulse/native"
            ]
        ));
        assert!(has(
            &v,
            &[
                "--bind",
                "/run/user/1000/pipewire-0",
                "/run/user/1000/pipewire-0"
            ]
        ));
        assert!(has(&v, &["--tmpfs", "/sys/fs/cgroup"]));
        assert!(has(&v, &["--tmpfs", VAULTS]));
    }

    #[test]
    fn network_off_takes_the_network_and_the_system_bus() {
        let v = run(Grants {
            network: false,
            ..Grants::default()
        });
        assert!(v.iter().any(|s| s == "--unshare-net"));
        assert!(has(&v, &["--tmpfs", "/run/dbus"]));
    }

    #[test]
    fn camera_off_covers_the_camera_nodes_only() {
        let v = run(Grants {
            camera: false,
            ..Grants::default()
        });
        assert!(has(&v, &["--dev-bind", "/dev/null", "/dev/video0"]));
        assert!(has(&v, &["--dev-bind", "/dev/null", "/dev/media1"]));
        assert!(!v.iter().any(|s| s == "/dev/snd"));
    }

    #[test]
    fn mic_off_swaps_the_pulse_socket_and_drops_pipewire() {
        let v = run(Grants {
            mic: false,
            ..Grants::default()
        });
        assert!(has(&v, &["--tmpfs", "/dev/snd"]));
        assert!(has(
            &v,
            &["--dev-bind", "/dev/snd/pcmC1D0p", "/dev/snd/pcmC1D0p"]
        ));
        assert!(!v.iter().any(|s| s == "/dev/snd/pcmC1D0c"));
        assert!(has(
            &v,
            &[
                "--bind",
                "/run/user/1000/pulse/native-nomic",
                "/run/user/1000/pulse/native"
            ]
        ));
        assert!(!v.iter().any(|s| s == "/run/user/1000/pipewire-0"));
    }

    #[test]
    fn camera_nodes_are_found_by_driver_and_the_codec_is_left() {
        let sys = std::env::temp_dir().join(format!("gh0st-sb-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sys);
        let drivers = sys.join("drivers");
        let node = |class: &str, name: &str, driver: &str| {
            let d = sys.join(class).join(name);
            std::fs::create_dir_all(d.join("device")).unwrap();
            std::fs::create_dir_all(drivers.join(driver)).unwrap();
            std::os::unix::fs::symlink(drivers.join(driver), d.join("device/driver")).unwrap();
        };
        node("class/video4linux", "video0", "unicam");
        node("class/video4linux", "video10", "bcm2835-codec");
        node("class/video4linux", "video13", "bcm2835-isp");
        node("class/video4linux", "v4l-subdev0", "imx219");
        let m = sys.join("bus/media/devices/media3");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("model"), "bcm2835-codec\n").unwrap();
        let m = sys.join("bus/media/devices/media1");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("model"), "unicam\n").unwrap();
        let got = camera_nodes(&sys);
        let names: Vec<_> = got.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "/dev/media1",
                "/dev/v4l-subdev0",
                "/dev/video0",
                "/dev/video13"
            ]
        );
        std::fs::remove_dir_all(&sys).unwrap();
    }
}
