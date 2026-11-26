//! The sandbox an app runs in: bubblewrap with its own user, mount and PID
//! namespaces (`docs/isolation.md`). The app keeps its uid, groups and
//! devices, so a stock app runs as it did. What changes:
//!
//! - It sees only its own processes, so it can't trace or signal another
//!   app, or read another's memory.
//! - Home stays writable, as stock apps expect, but secrets are hidden and
//!   what an unsandboxed program later runs or loads is read-only: shell
//!   start files, `~/bin`, `~/.local/bin`, user units, D-Bus and PipeWire
//!   config, and our other apps' data. `prepare` makes the ones that are
//!   missing, so an app can't make them first.
//! - The runtime folder holds only the bus, PipeWire and pulse sockets.
//! - Network, camera and mic go when the user turns them off. Without the
//!   mic an app hears neither it nor what the device plays.
//! - An input method's bridge (`bridges`) loads only into an app the user
//!   lets it type into.
//!
//! No `--die-with-parent`: the shell's SIGTERM would kill bwrap at once
//! and bwrap the app, before the app's own handler could save.

use std::ffi::OsString;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

pub const BWRAP: &str = "/usr/bin/bwrap";

/// What a sandboxed app may reach beyond its files. All but `input` by
/// default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grants {
    pub network: bool,
    pub camera: bool,
    pub mic: bool,
    /// Input methods' bridges load into it: they read its keys and type
    /// into it. Off until the user turns it on.
    pub input: bool,
}

impl Default for Grants {
    fn default() -> Grants {
        Grants {
            network: true,
            camera: true,
            mic: true,
            input: false,
        }
    }
}

/// Where the stock launcher's unit takes drop-ins, under home. A Store
/// input method puts its `LD_PRELOAD` there for the launcher to pass on.
pub const LAUNCHER_DROPINS: &str = ".config/systemd/user/APPLaunch.service.d";

/// An input method's bridge: a library loaded into apps, so a Store app
/// (voice typing, pinyin) reads their keys and types into them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bridge {
    pub lib: PathBuf,
    /// Its folder in the runtime dir, named as the folder its library is
    /// in: `/usr/lib/voice-input/x.so` talks through `$XDG_RUNTIME_DIR/voice-input`.
    pub dir: String,
}

/// The bridges the launcher drop-ins under `home` name. Only a library in
/// a folder of its own under `/usr/lib`, both owned by root and writable
/// by no one else, counts: an app can't name its own file.
pub fn bridges(home: &Path) -> Vec<Bridge> {
    bridges_in(
        &home.join(LAUNCHER_DROPINS),
        Path::new("/usr/lib"),
        root_owned,
    )
}

/// `LD_PRELOAD`'s value for these bridges.
pub fn preload(bridges: &[Bridge]) -> String {
    let libs: Vec<String> = bridges
        .iter()
        .map(|b| b.lib.to_string_lossy().into_owned())
        .collect();
    libs.join(":")
}

fn root_owned(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(p)
        .is_ok_and(|m| (m.is_file() || m.is_dir()) && m.uid() == 0 && m.mode() & 0o022 == 0)
}

fn bridges_in(dropins: &Path, lib_root: &Path, trusted: impl Fn(&Path) -> bool) -> Vec<Bridge> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dropins)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "conf"))
        .collect();
    files.sort();
    let mut out: Vec<Bridge> = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for lib in preload_paths(&text) {
            let lib = PathBuf::from(lib);
            let Ok(rest) = lib.strip_prefix(lib_root) else {
                continue;
            };
            let parts: Vec<&str> = rest.iter().filter_map(|c| c.to_str()).collect();
            let [dir, file] = parts[..] else {
                continue;
            };
            let named = !dir.is_empty()
                && dir.as_bytes()[0].is_ascii_alphanumeric()
                && dir
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+-.".contains(&b))
                && file.ends_with(".so");
            if named
                && trusted(&lib_root.join(dir))
                && trusted(&lib)
                && !out.iter().any(|b| b.lib == lib)
            {
                out.push(Bridge {
                    dir: dir.to_string(),
                    lib,
                });
            }
        }
    }
    out
}

/// The paths a unit file's `Environment=` lines put in `LD_PRELOAD`.
fn preload_paths(unit: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in unit.lines() {
        let Some(rest) = line.trim().strip_prefix("Environment=") else {
            continue;
        };
        let rest = rest.replace('"', " ");
        for tok in rest.split_whitespace() {
            if let Some(v) = tok.strip_prefix("LD_PRELOAD=") {
                out.extend(v.split(':').filter(|p| !p.is_empty()).map(String::from));
            }
        }
    }
    out
}

/// Home entries an app never sees.
const HIDDEN: [&str; 2] = [".ssh", ".gnupg"];

/// Top-level home entries an app may read but not write. Some shells
/// read them only if there: `prepare` makes the ones ours read, so they
/// are there to lock. The others are locked once someone makes them.
const LOCKED_TOP: [&str; 13] = [
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".bash_logout",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".inputrc",
    ".pam_environment",
    "bin",
    ".xsessionrc",
];

/// What `prepare` writes where nothing is: each reads the same as no file.
const STARTERS: [(&str, &str); 5] = [
    (".profile", ""),
    (".bashrc", ""),
    (".bash_logout", ""),
    // bash reads this instead of .profile once it exists, so it reads that.
    (
        ".bash_profile",
        "[ -f \"$HOME/.profile\" ] && . \"$HOME/.profile\"\n",
    ),
    (".inputrc", "$include /etc/inputrc\n"),
];

/// Our apps' ids, which name their data and cache folders. In a sandbox
/// only the app's own are writable.
const OUR_APPS: [&str; 10] = [
    "gh0st-settings",
    "gh0st-files",
    "gh0st-text",
    "gh0st-keyboard",
    "gh0st-keyring",
    "gh0st-store",
    "gh0st-update",
    "gh0st-term",
    "gh0st-camera",
    "gh0st-media",
];

/// Where our programs keep data, under home.
const DATA_ROOTS: [&str; 4] = [".local/share", ".cache", ".config", ".local/state"];

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
    /// Names of home's entries.
    pub home_entries: Vec<String>,
    /// Our programs' data folders, `gh0st` and `gh0st-*` under
    /// `DATA_ROOTS`, relative to home.
    pub ours: Vec<PathBuf>,
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
    /// Input methods' bridges, for an app granted `input`.
    pub bridges: Vec<Bridge>,
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
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        let mut ours = Vec::new();
        for root in DATA_ROOTS {
            for e in std::fs::read_dir(home.join(root))
                .into_iter()
                .flatten()
                .flatten()
            {
                let n = e.file_name().to_string_lossy().into_owned();
                if (n == "gh0st" || n.starts_with("gh0st-"))
                    && e.file_type().is_ok_and(|t| t.is_dir())
                {
                    ours.push(Path::new(root).join(n));
                }
            }
        }
        ours.sort();
        let sockets = SOCKETS
            .iter()
            .filter(|s| runtime.join(s).exists())
            .map(|s| s.to_string())
            .collect();
        Host {
            home: home.to_path_buf(),
            runtime: runtime.to_path_buf(),
            home_entries,
            ours,
            camera_nodes: camera_nodes(Path::new("/sys")),
            capture_nodes: capture_nodes(Path::new("/dev/snd")),
            sound_nodes: sound_nodes(Path::new("/dev/snd")),
            sockets,
            vaults: Path::new(VAULTS).is_dir(),
            bridges: bridges(home),
        }
    }
}

/// Makes what a sandbox locks or hides where it is missing, so an app
/// can't make it first. The shell runs it at start on the device.
pub fn prepare(home: &Path, notify: &Path) {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = |p: PathBuf| {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(p);
    };
    for d in LOCKED_DIRS.iter().chain(&HIDDEN).chain(&["bin"]) {
        dir(home.join(d));
    }
    for app in OUR_APPS {
        dir(home.join(".local/share").join(app));
        dir(home.join(".cache").join(app));
    }
    dir(notify.to_path_buf());
    let new_file = |name: &str, mode: u32| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(home.join(name))
    };
    for (name, text) in STARTERS {
        if name == ".bash_profile" && home.join(".bash_login").exists() {
            continue;
        }
        if let Ok(mut f) = new_file(name, 0o644) {
            let _ = f.write_all(text.as_bytes());
        }
    }
    let _ = new_file(".bash_history", 0o600);
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
/// None without a home or a runtime folder: no sandbox is better than one
/// missing its rules, and the shell then won't start the app.
pub fn argv(
    host: &Host,
    grants: Grants,
    stem: &str,
    notify: &Path,
    inner: &[OsString],
) -> Option<Vec<OsString>> {
    if host.home.as_os_str().is_empty() || host.runtime.as_os_str().is_empty() {
        return None;
    }
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
    if grants.input && !host.bridges.is_empty() {
        a.raw("--setenv");
        a.raw("LD_PRELOAD");
        a.raw(preload(&host.bridges));
    } else {
        a.raw("--unsetenv");
        a.raw("LD_PRELOAD");
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
    Some(a.0)
}

fn runtime(a: &mut Args, host: &Host, grants: Grants) {
    let r = &host.runtime;
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
    // The bridge in the app talks to its input method here.
    if grants.input {
        for b in &host.bridges {
            a.pair("--bind-try", r.join(&b.dir), r.join(&b.dir));
        }
    }
}

fn home(a: &mut Args, host: &Host, stem: &str, notify: &Path) {
    let h = &host.home;
    for name in LOCKED_TOP {
        if host.home_entries.iter().any(|e| e == name) {
            a.pair("--ro-bind", h.join(name), h.join(name));
        }
    }
    for d in LOCKED_DIRS {
        let p = h.join(d);
        a.pair("--ro-bind-try", &p, &p);
    }
    // Our other apps' data: Text's unsaved edits, Keyboard's snippets.
    let own = format!("gh0st-{stem}");
    for d in &host.ours {
        if d.file_name() != Some(std::ffi::OsStr::new(&own)) {
            a.pair("--ro-bind-try", h.join(d), h.join(d));
        }
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
            ours: [
                ".local/share/gh0st-text",
                ".local/share/gh0st-media",
                ".cache/gh0st",
            ]
            .map(PathBuf::from)
            .to_vec(),
            camera_nodes: vec!["/dev/video0".into(), "/dev/media1".into()],
            capture_nodes: vec!["/dev/snd/pcmC1D0c".into()],
            sound_nodes: vec!["/dev/snd/controlC1".into(), "/dev/snd/pcmC1D0p".into()],
            sockets: ["bus", "pipewire-0", "pulse/native", PULSE_NOMIC]
                .map(String::from)
                .to_vec(),
            vaults: true,
            bridges: vec![Bridge {
                lib: "/usr/lib/voice-input/shared_bridge.so".into(),
                dir: "voice-input".into(),
            }],
        }
    }

    #[test]
    fn a_bridge_loads_only_into_an_app_granted_input() {
        let notify = Path::new("/home/u/.local/share/gh0st/notices");
        let inner = [OsString::from("/usr/bin/app")];
        let off = argv(&host(), Grants::default(), "zsynth", notify, &inner).unwrap();
        let off: Vec<String> = off
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(has(&off, &["--unsetenv", "LD_PRELOAD"]));
        assert!(!off.iter().any(|s| s.contains("voice-input")));
        let g = Grants {
            input: true,
            ..Grants::default()
        };
        let on = argv(&host(), g, "zsynth", notify, &inner).unwrap();
        let on: Vec<String> = on
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(has(
            &on,
            &[
                "--setenv",
                "LD_PRELOAD",
                "/usr/lib/voice-input/shared_bridge.so"
            ]
        ));
        let r = "/run/user/1000/voice-input";
        assert!(has(&on, &["--bind-try", r, r]));
        // Granted, with no bridge installed: nothing to load.
        let mut bare = host();
        bare.bridges.clear();
        let none = argv(&bare, g, "zsynth", notify, &inner).unwrap();
        let none: Vec<String> = none
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(has(&none, &["--unsetenv", "LD_PRELOAD"]));
    }

    #[test]
    fn bridges_come_from_the_launcher_drop_ins_and_must_be_trusted() {
        let d = std::env::temp_dir().join(format!("gh0st-bridges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dropins = d.join("dropins");
        std::fs::create_dir_all(&dropins).unwrap();
        let lib = d.join("lib");
        for (n, body) in [
            (
                "90-voice.conf",
                "[Service]\nEnvironment=LD_PRELOAD=LIB/voice-input/shared_bridge.so\n",
            ),
            (
                "99-pinyin.conf",
                "[Service]\nEnvironment=\"LD_PRELOAD=LIB/cardputer-pinyin/bridge.so\" A=1\n",
            ),
            // Not under the library root, not a .so, too deep, not a .conf.
            (
                "50-other.conf",
                "[Service]\nEnvironment=LD_PRELOAD=/home/u/evil.so:LIB/x/y.txt:LIB/a/b/c.so\n",
            ),
            ("readme", "Environment=LD_PRELOAD=LIB/z/z.so\n"),
        ] {
            let body = body.replace("LIB", &lib.to_string_lossy());
            std::fs::write(dropins.join(n), body).unwrap();
        }
        let all = |_: &Path| true;
        let b = bridges_in(&dropins, &lib, all);
        let dirs: Vec<&str> = b.iter().map(|b| b.dir.as_str()).collect();
        assert_eq!(dirs, ["voice-input", "cardputer-pinyin"]);
        assert_eq!(
            preload(&b),
            format!(
                "{0}/voice-input/shared_bridge.so:{0}/cardputer-pinyin/bridge.so",
                lib.display()
            )
        );
        // A library someone other than root could write is skipped.
        let only_pinyin = |p: &Path| !p.to_string_lossy().contains("voice-input");
        let b = bridges_in(&dropins, &lib, only_pinyin);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].dir, "cardputer-pinyin");
        assert!(bridges_in(&d.join("none"), &lib, all).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    fn run_as(stem: &str, g: Grants) -> Vec<String> {
        argv(
            &host(),
            g,
            stem,
            Path::new("/home/u/.local/state/gh0st/notify"),
            &["/bin/sh".into(), "-c".into(), "exec zsynth".into()],
        )
        .unwrap()
        .into_iter()
        .map(|s| s.into_string().unwrap())
        .collect()
    }

    fn run(g: Grants) -> Vec<String> {
        run_as("zsynth", g)
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
    fn home_stays_writable_but_start_files_and_secrets_do_not() {
        let v = run(Grants::default());
        assert!(!v.iter().any(|s| s == "/home/u"), "home itself is as it is");
        assert!(has(
            &v,
            &["--ro-bind", "/home/u/.bashrc", "/home/u/.bashrc"]
        ));
        assert!(has(&v, &["--ro-bind", "/home/u/bin", "/home/u/bin"]));
        assert!(!v.iter().any(|s| s == "/home/u/Documents"));
        assert!(
            !v.iter().any(|s| s == "/home/u/.zshrc"),
            "only what is there"
        );
        assert!(has(&v, &["--tmpfs", "/home/u/.ssh"]));
        assert!(has(
            &v,
            &["--ro-bind", "/dev/null", "/home/u/.bash_history"]
        ));
        let systemd = "/home/u/.config/systemd";
        assert!(has(&v, &["--ro-bind-try", systemd, systemd]));
        let bin = "/home/u/.local/bin";
        assert!(has(&v, &["--ro-bind-try", bin, bin]));
    }

    #[test]
    fn our_other_apps_data_is_read_only_but_the_apps_own_is_not() {
        let text = "/home/u/.local/share/gh0st-text";
        let media = "/home/u/.local/share/gh0st-media";
        let cache = "/home/u/.cache/gh0st";
        let v = run(Grants::default());
        assert!(has(&v, &["--ro-bind-try", text, text]));
        assert!(has(&v, &["--ro-bind-try", media, media]));
        assert!(has(&v, &["--ro-bind-try", cache, cache]));
        let v = run_as("media", Grants::default());
        assert!(has(&v, &["--ro-bind-try", text, text]));
        assert!(!v.iter().any(|s| s == media));
    }

    #[test]
    fn no_home_or_runtime_means_no_sandbox_rather_than_half_of_one() {
        let mut h = host();
        h.runtime = PathBuf::new();
        assert!(argv(&h, Grants::default(), "x", Path::new("/n"), &[]).is_none());
        let mut h = host();
        h.home = PathBuf::new();
        assert!(argv(&h, Grants::default(), "x", Path::new("/n"), &[]).is_none());
    }

    #[test]
    fn prepare_makes_what_a_sandbox_locks_and_keeps_what_is_there() {
        let home = std::env::temp_dir().join(format!("gh0st-sb-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".profile"), "PATH=mine\n").unwrap();
        prepare(&home, &home.join(".local/state/gh0st/notify"));
        for d in [
            ".ssh",
            "bin",
            ".local/bin",
            ".config/systemd",
            ".local/share/gh0st-text",
        ] {
            assert!(home.join(d).is_dir(), "{d}");
        }
        let profile = std::fs::read_to_string(home.join(".profile")).unwrap();
        assert_eq!(profile, "PATH=mine\n");
        let bp = std::fs::read_to_string(home.join(".bash_profile")).unwrap();
        assert!(bp.contains(". \"$HOME/.profile\""), "{bp}");
        assert!(home.join(".bash_history").is_file());
        assert!(home.join(".local/state/gh0st/notify").is_dir());
        std::fs::remove_dir_all(&home).unwrap();
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
