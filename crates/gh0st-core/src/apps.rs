//! The app list: our entries and the stock `.desktop` folder, filtered and
//! ordered. Pins come first; everything else keeps one stable order, so an app
//! stays where the hand remembers it.

use crate::config::Config;
use crate::desktop::{self, DesktopEntry};
use std::path::{Path, PathBuf};

/// Stock caps the list at 128 desktop apps.
pub const MAX_APPS: usize = 128;

pub struct Dirs {
    /// Ours, scanned first: Settings, the Store entry.
    pub ours: PathBuf,
    /// Stock and third-party apps, installed by the Store.
    pub stock: PathBuf,
    /// Relative `Icon=` paths resolve here, as in stock.
    pub icon_root: PathBuf,
}

impl Default for Dirs {
    fn default() -> Dirs {
        Dirs {
            ours: PathBuf::from("/usr/share/gh0st/applications"),
            stock: PathBuf::from("/usr/share/APPLaunch/applications"),
            icon_root: PathBuf::from("/usr/share/APPLaunch"),
        }
    }
}

impl Dirs {
    /// The defaults, each overridable for tests and host runs:
    /// `GH0ST_APPS_OURS`, `GH0ST_APPS_STOCK`, `GH0ST_ICON_ROOT`.
    pub fn from_env() -> Dirs {
        let d = Dirs::default();
        let get = |k: &str, v: PathBuf| std::env::var_os(k).map(PathBuf::from).unwrap_or(v);
        Dirs {
            ours: get("GH0ST_APPS_OURS", d.ours),
            stock: get("GH0ST_APPS_STOCK", d.stock),
            icon_root: get("GH0ST_ICON_ROOT", d.icon_root),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconRef {
    /// A PNG on disk.
    Path(PathBuf),
    /// A symbol baked into gh0st-ui: `Icon=gh0st:settings`.
    Symbol(String),
    /// Nothing usable. The shell draws the first letter.
    None,
}

/// How home shows the apps: `gh0st.home_layout` in config.json.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Grid,
    List,
}

impl Layout {
    /// `gh0st.home_layout`, or the default.
    pub fn of(c: &crate::config::Config) -> Layout {
        c.get_str(crate::config::key::HOME_LAYOUT)
            .and_then(Layout::from_config)
            .unwrap_or_default()
    }

    pub fn from_config(s: &str) -> Option<Layout> {
        match s {
            "grid" => Some(Layout::Grid),
            "list" => Some(Layout::List),
            _ => None,
        }
    }

    pub fn config_name(self) -> &'static str {
        match self {
            Layout::Grid => "grid",
            Layout::List => "list",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Grid => "Grid",
            Layout::List => "List",
        }
    }

    pub fn toggled(self) -> Layout {
        match self {
            Layout::Grid => Layout::List,
            Layout::List => Layout::Grid,
        }
    }
}

/// Where the pinned apps live, one `.desktop` file name per line.
pub fn pins_path() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/".into())
        .join(".local/state/gh0st/pinned")
}

pub fn load_pins() -> Vec<String> {
    std::fs::read_to_string(pins_path())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_pins(pins: &[String]) -> std::io::Result<()> {
    let path = pins_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::config::write_atomic(&path, (pins.join("\n") + "\n").as_bytes())
}

/// Pins `file`, or unpins it if pinned, in the file as it is now. Returns the
/// new pins and whether `file` is pinned.
pub fn toggle_pin(file: &str) -> std::io::Result<(Vec<String>, bool)> {
    let mut pins = load_pins();
    let on = match pins.iter().position(|p| p == file) {
        Some(i) => {
            pins.remove(i);
            false
        }
        None => {
            pins.push(file.to_string());
            true
        }
    };
    save_pins(&pins)?;
    Ok((pins, on))
}

/// Swaps `file` with the next pin in direction `d` that `visible` holds.
/// Pins nobody sees (an uninstalled app, Settings itself) keep their places.
pub fn move_pin(pins: &[String], visible: &[String], file: &str, d: i32) -> Option<Vec<String>> {
    let shown: Vec<&String> = pins.iter().filter(|p| visible.contains(p)).collect();
    let i = shown.iter().position(|p| *p == file)?;
    let other = shown.get(i.checked_add_signed(d as isize)?)?;
    let a = pins.iter().position(|p| p == file)?;
    let b = pins.iter().position(|p| p == *other)?;
    let mut out = pins.to_vec();
    out.swap(a, b);
    Some(out)
}

#[derive(Clone, Debug)]
pub struct App {
    /// File name of the `.desktop`, unique across both folders.
    pub file: String,
    pub entry: DesktopEntry,
    pub icon: IconRef,
    /// Ours run their argv directly; third-party ones go through `sh -c`.
    pub ours: bool,
    /// On home. Settings > Apps lists the hidden ones too.
    pub shown: bool,
    mtime: u64,
    seq: usize,
}

impl App {
    /// True when `name` means this app: its desktop file stem (`update`)
    /// or the program its `Exec` runs (`gh0st-update`). Notices and the
    /// shell link name apps either way.
    pub fn answers_to(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let stem = self.file.strip_suffix(".desktop").unwrap_or(&self.file);
        if stem == name {
            return true;
        }
        let program = self.entry.exec.split_whitespace().next().unwrap_or("");
        program.rsplit('/').next() == Some(name)
    }

    /// The desktop file stem: `text` for `text.desktop`.
    pub fn id(&self) -> &str {
        self.file.strip_suffix(".desktop").unwrap_or(&self.file)
    }

    /// Takes files: a field code in `Exec` and at least one `MimeType`.
    pub fn opens_files(&self) -> bool {
        !self.entry.mime_types.is_empty() && desktop::field_code(&self.entry.exec).is_some()
    }

    #[cfg(test)]
    pub(crate) fn for_test(entry: DesktopEntry, ours: bool) -> App {
        App {
            file: "test.desktop".into(),
            entry,
            icon: IconRef::None,
            ours,
            shown: true,
            mtime: 0,
            seq: 0,
        }
    }

    /// Stock's per-app switch in config.json, so Settings > Apps and stock
    /// agree on what is shown: `app_desktop_<name>_<fnv1a hex>`.
    pub fn stock_key(&self) -> String {
        stock_key(&self.file)
    }
}

pub fn stock_key(file: &str) -> String {
    let mut key = String::from("app_desktop_");
    key.extend(file.bytes().map(|b| {
        if b.is_ascii_alphanumeric() {
            b as char
        } else {
            '_'
        }
    }));
    let mut h: u32 = 2_166_136_261;
    for b in file.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    format!("{key}_{h:08x}")
}

/// Our per-app setting `leaf` for `file`: `gh0st.app.<file>.<leaf>`, with
/// the file name's dots and odd bytes made `_` so the key does not nest.
/// Whether the app runs on behind other screens: the user's choice in
/// Settings > Apps, else what the app declares (`X-Gh0st-Background`).
pub fn runs_behind(c: &crate::config::Config, file: &str, declared: bool) -> bool {
    c.get_bool(&background_key(file)).unwrap_or(declared)
}

pub fn background_key(file: &str) -> String {
    app_key(file, "background")
}

/// How an app runs: in the sandbox or not, and what it may reach there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Isolation {
    pub sandbox: bool,
    pub grants: crate::sandbox::Grants,
}

impl Default for Isolation {
    /// Sandboxed, with everything granted: an app nobody has set yet.
    fn default() -> Isolation {
        Isolation {
            sandbox: true,
            grants: crate::sandbox::Grants::default(),
        }
    }
}

/// Why an app runs outside the sandbox whatever the user picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exempt {
    /// Ours, and it needs the whole home or pkexec (`X-Gh0st-Sandbox`).
    BuiltIn,
    /// It runs as root: through sudo in its `Exec`, or by the Store's mark.
    Root,
}

pub fn exempt(app: &App, root: bool) -> Option<Exempt> {
    if app.ours && app.entry.no_sandbox {
        Some(Exempt::BuiltIn)
    } else if root || runs_as_root(&app.entry.exec) {
        Some(Exempt::Root)
    } else {
        None
    }
}

/// The user's choices in Settings > Apps, else everything on. `root` is
/// the Store's mark for an app that asks for root as it runs.
pub fn isolation(c: &crate::config::Config, app: &App, root: bool) -> Isolation {
    let get = |leaf: &str| c.get_bool(&app_key(&app.file, leaf)).unwrap_or(true);
    Isolation {
        sandbox: get("isolate") && exempt(app, root).is_none(),
        grants: crate::sandbox::Grants {
            network: get("network"),
            camera: get("camera"),
            mic: get("mic"),
        },
    }
}

/// An `Exec` line that asks for root. A sandbox can't give it: setuid
/// programs don't raise anything there.
pub fn runs_as_root(exec: &str) -> bool {
    exec.split(|c: char| c.is_whitespace() || ";&|()`'\"".contains(c))
        .filter_map(|t| t.rsplit('/').next())
        .any(|t| matches!(t, "sudo" | "pkexec" | "doas" | "su" | "run0" | "gksudo"))
}

pub fn app_key(file: &str, leaf: &str) -> String {
    let id: String = file
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                b as char
            } else {
                '_'
            }
        })
        .collect();
    format!("gh0st.app.{id}.{leaf}")
}

/// Shown unless switched on in Settings > Apps.
const HIDDEN_BY_DEFAULT: &[&str] = &["zclaw.desktop"];

/// Stock's built-in order, by lower-case alphanumeric label, with the
/// terminal app where stock had its CLI page.
const ORDER: &[&str] = &[
    "settings",
    "store",
    "terminal",
    "files",
    "camera",
    "rec",
    "recorder",
    "music",
    "piano",
    "compass",
    "irremote",
    "irchat",
    "keyboardguide",
    "factorytest",
    "gps",
    "nfc",
];

fn order_of(label: &str) -> Option<usize> {
    let norm: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    ORDER.iter().position(|&o| o == norm)
}

/// Reads both folders and returns the apps to show, in order, with Settings
/// built in.
pub fn scan(dirs: &Dirs, config: &Config, pins: &[String]) -> Vec<App> {
    let mut apps = scan_all(dirs, config, pins, desktop::is_executable);
    apps.retain(|a| a.shown && !a.entry.no_display);
    apps
}

pub fn scan_with(
    dirs: &Dirs,
    config: &Config,
    pins: &[String],
    executable: impl Fn(&Path) -> bool + Copy,
) -> Vec<App> {
    let mut apps = scan_all(dirs, config, pins, executable);
    apps.retain(|a| a.shown && !a.entry.no_display);
    apps
}

/// The apps that open files, hidden ones and `NoDisplay` ones included.
pub fn openers(dirs: &Dirs, config: &Config) -> Vec<App> {
    let mut apps = scan_all(dirs, config, &[], desktop::is_executable);
    apps.retain(App::opens_files);
    apps
}

/// Every app, hidden ones included (`shown` false), for Settings > Apps.
/// `NoDisplay` entries are in it too: Settings leaves them out.
pub fn scan_all(
    dirs: &Dirs,
    config: &Config,
    pins: &[String],
    executable: impl Fn(&Path) -> bool + Copy,
) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    let mut seq = 0;
    for (dir, ours) in [(&dirs.ours, true), (&dirs.stock, false)] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<(String, PathBuf)> = rd
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().into_string().ok()?;
                desktop::filename_valid(&name).then(|| (name, e.path()))
            })
            .collect();
        files.sort();
        for (file, path) in files {
            seq += 1;
            if apps.iter().any(|a| a.file == file) {
                continue;
            }
            let Some(entry) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| desktop::parse(&s))
            else {
                continue;
            };
            if desktop::exec_allowed(&entry.exec, executable).is_err() {
                continue;
            }
            if !entry.try_exec.is_empty() && !executable(Path::new(&entry.try_exec)) {
                continue;
            }
            if apps.iter().any(|a| a.entry.exec == entry.exec) {
                continue;
            }
            let shown = match config.get_int(&stock_key(&file)) {
                Some(v) => v != 0,
                None => !HIDDEN_BY_DEFAULT.contains(&file.as_str()),
            };
            let icon = resolve_icon(&entry.icon, &dirs.icon_root);
            let mtime = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX);
            apps.push(App {
                file,
                entry,
                icon,
                ours,
                shown,
                mtime,
                seq,
            });
            if apps.len() == MAX_APPS {
                break;
            }
        }
    }
    sort(&mut apps, pins);
    apps
}

fn resolve_icon(icon: &str, root: &Path) -> IconRef {
    if let Some(sym) = icon.strip_prefix("gh0st:") {
        return IconRef::Symbol(sym.to_string());
    }
    if icon.is_empty() {
        return IconRef::None;
    }
    let p = Path::new(icon);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    if p.is_file() {
        IconRef::Path(p)
    } else {
        IconRef::None
    }
}

/// Pins in pin order, then the stock order, then the rest oldest install
/// first (the Store rewrites a `.desktop` on install, so mtime is install time).
fn sort(apps: &mut [App], pins: &[String]) {
    apps.sort_by_key(|a| {
        let pin = pins.iter().position(|p| *p == a.file).unwrap_or(usize::MAX);
        let known = order_of(&a.entry.name).unwrap_or(usize::MAX);
        (pin, known, a.mtime, a.seq)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Tree {
            let root =
                std::env::temp_dir().join(format!("gh0st-apps-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for d in ["ours", "stock", "share/images"] {
                std::fs::create_dir_all(root.join(d)).unwrap();
            }
            Tree(root)
        }

        fn dirs(&self) -> Dirs {
            Dirs {
                ours: self.0.join("ours"),
                stock: self.0.join("stock"),
                icon_root: self.0.clone(),
            }
        }

        fn add(&self, dir: &str, file: &str, name: &str, exec: &str, extra: &str) {
            let s = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n{extra}");
            std::fs::write(self.0.join(dir).join(file), s).unwrap();
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|a| a.entry.name.as_str()).collect()
    }

    #[test]
    fn sudo_anywhere_in_exec_runs_as_root() {
        assert!(runs_as_root("sudo /usr/bin/thing"));
        assert!(runs_as_root("/usr/bin/sudo -E thing"));
        assert!(runs_as_root("sh -c 'cd x; pkexec y'"));
        assert!(!runs_as_root("bin/battleship-launch"));
        assert!(!runs_as_root("/opt/pseudo-app --sudo"));
    }

    #[test]
    fn isolation_is_on_with_everything_until_the_user_says_otherwise() {
        let path = std::env::temp_dir()
            .join(format!("gh0st-apps-iso-{}", std::process::id()))
            .join("config.json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut entry = DesktopEntry {
            name: "Synth".into(),
            exec: "/opt/zsynth/zsynth.py".into(),
            ..Default::default()
        };
        let mut app = App::for_test(entry.clone(), false);
        let c = crate::config::Config::load(&path);
        let i = isolation(&c, &app, false);
        assert!(i.sandbox);
        assert_eq!(i.grants, crate::sandbox::Grants::default());
        assert!(
            !isolation(&c, &app, true).sandbox,
            "the Store marks it root"
        );

        crate::config::Config::update(&path, |c| {
            c.set_bool(&app_key("test.desktop", "mic"), false)
        })
        .unwrap();
        let i = isolation(&crate::config::Config::load(&path), &app, false);
        assert!(i.sandbox && !i.grants.mic && i.grants.network);

        // A Store app can't opt out with our key; ours can.
        entry.no_sandbox = true;
        app.entry = entry;
        assert!(isolation(&c, &app, false).sandbox);
        app.ours = true;
        assert_eq!(exempt(&app, false), Some(Exempt::BuiltIn));
        assert!(!isolation(&c, &app, false).sandbox);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_pin_moves_past_its_visible_neighbour() {
        let v = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        let pins = v("a gone b c");
        let shown = v("a b c");
        assert_eq!(move_pin(&pins, &shown, "b", -1), Some(v("b gone a c")));
        assert_eq!(move_pin(&pins, &shown, "b", 1), Some(v("a gone c b")));
        assert_eq!(move_pin(&pins, &shown, "a", -1), None, "already first");
        assert_eq!(move_pin(&pins, &shown, "c", 1), None, "already last");
        assert_eq!(move_pin(&pins, &shown, "x", 1), None, "not pinned");
    }

    #[test]
    fn stock_order_then_pins_first() {
        let t = Tree::new("order");
        t.add("stock", "music.desktop", "Music", "/bin/music", "");
        t.add("stock", "zz.desktop", "Zebra", "/bin/zebra", "");
        t.add(
            "ours",
            "settings.desktop",
            "Settings",
            "/usr/bin/gh0st-settings",
            "Icon=gh0st:settings\n",
        );
        t.add("stock", "files.desktop", "Files", "/bin/files", "");
        let c = Config::default();
        let apps = scan_with(&t.dirs(), &c, &[], |_| true);
        assert_eq!(names(&apps), ["Settings", "Files", "Music", "Zebra"]);
        assert_eq!(apps[0].icon, IconRef::Symbol("settings".into()));
        let apps = scan_with(&t.dirs(), &c, &["zz.desktop".into()], |_| true);
        assert_eq!(names(&apps)[0], "Zebra");
    }

    #[test]
    fn zclaw_is_hidden_until_switched_on_and_any_app_can_be_hidden() {
        let t = Tree::new("hidden");
        t.add("stock", "zclaw.desktop", "ZClaw", "/bin/zclaw", "");
        t.add("stock", "music.desktop", "Music", "/bin/music", "");
        let mut c = Config::default();
        assert_eq!(names(&scan_with(&t.dirs(), &c, &[], |_| true)), ["Music"]);
        c.set_int(&stock_key("zclaw.desktop"), 1);
        c.set_int(&stock_key("music.desktop"), 0);
        assert_eq!(names(&scan_with(&t.dirs(), &c, &[], |_| true)), ["ZClaw"]);
        let all = scan_all(&t.dirs(), &c, &[], |_| true);
        assert_eq!(all.iter().filter(|a| !a.shown).count(), 1, "Music, hidden");
    }

    #[test]
    fn a_missing_try_exec_or_a_duplicate_exec_drops_the_entry() {
        let t = Tree::new("dupes");
        t.add(
            "ours",
            "store.desktop",
            "Store",
            "/opt/store",
            "TryExec=/opt/store\n",
        );
        t.add("stock", "a.desktop", "A", "/bin/same", "");
        t.add("stock", "b.desktop", "B", "/bin/same", "");
        let apps = scan_with(&t.dirs(), &Config::default(), &[], |p| {
            p != Path::new("/opt/store")
        });
        assert_eq!(names(&apps), ["A"]);
    }

    #[test]
    fn relative_icons_resolve_against_the_stock_root() {
        let t = Tree::new("icons");
        std::fs::write(t.0.join("share/images/m.png"), b"png").unwrap();
        t.add(
            "stock",
            "m.desktop",
            "Music",
            "/bin/m",
            "Icon=share/images/m.png\n",
        );
        t.add(
            "stock",
            "n.desktop",
            "Notes",
            "/bin/n",
            "Icon=share/images/missing.png\n",
        );
        let apps = scan_with(&t.dirs(), &Config::default(), &[], |_| true);
        assert_eq!(apps[0].icon, IconRef::Path(t.0.join("share/images/m.png")));
        assert_eq!(apps[1].icon, IconRef::None, "kept, drawn as a letter");
    }

    #[test]
    fn our_app_keys_do_not_nest_on_the_file_name() {
        assert_eq!(
            app_key("music.desktop", "icon"),
            "gh0st.app.music_desktop.icon"
        );
    }

    #[test]
    fn the_stock_key_matches_stock() {
        // FNV-1a of "zclaw.desktop", the way desktop_app_loader.cpp builds it.
        let k = stock_key("zclaw.desktop");
        assert!(k.starts_with("app_desktop_zclaw_desktop_"));
        assert_eq!(k.len(), "app_desktop_zclaw_desktop_".len() + 8);
    }
}
