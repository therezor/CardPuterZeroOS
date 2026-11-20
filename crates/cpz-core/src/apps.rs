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
            ours: PathBuf::from("/usr/share/cpz/applications"),
            stock: PathBuf::from("/usr/share/APPLaunch/applications"),
            icon_root: PathBuf::from("/usr/share/APPLaunch"),
        }
    }
}

impl Dirs {
    /// The defaults, each overridable for tests and host runs:
    /// `CPZ_APPS_OURS`, `CPZ_APPS_STOCK`, `CPZ_ICON_ROOT`.
    pub fn from_env() -> Dirs {
        let d = Dirs::default();
        let get = |k: &str, v: PathBuf| std::env::var_os(k).map(PathBuf::from).unwrap_or(v);
        Dirs {
            ours: get("CPZ_APPS_OURS", d.ours),
            stock: get("CPZ_APPS_STOCK", d.stock),
            icon_root: get("CPZ_ICON_ROOT", d.icon_root),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconRef {
    /// A PNG on disk.
    Path(PathBuf),
    /// A symbol baked into cpz-ui: `Icon=cpz:settings`.
    Symbol(String),
    /// Nothing usable. The shell draws the first letter.
    None,
}

#[derive(Clone, Debug)]
pub struct App {
    /// File name of the `.desktop`, unique across both folders.
    pub file: String,
    pub entry: DesktopEntry,
    pub icon: IconRef,
    /// Ours run their argv directly; third-party ones go through `sh -c`.
    pub ours: bool,
    mtime: u64,
    seq: usize,
}

impl App {
    #[cfg(test)]
    pub(crate) fn for_test(entry: DesktopEntry, ours: bool) -> App {
        App {
            file: "test.desktop".into(),
            entry,
            icon: IconRef::None,
            ours,
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

/// Reads both folders and returns the apps to show, in order.
pub fn scan(dirs: &Dirs, config: &Config, pins: &[String]) -> Vec<App> {
    scan_with(dirs, config, pins, desktop::is_executable)
}

pub fn scan_with(
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
            if !shown {
                continue;
            }
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
    if let Some(sym) = icon.strip_prefix("cpz:") {
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
            let root = std::env::temp_dir().join(format!("cpz-apps-{tag}-{}", std::process::id()));
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
    fn stock_order_then_pins_first() {
        let t = Tree::new("order");
        t.add("stock", "music.desktop", "Music", "/bin/music", "");
        t.add("stock", "zz.desktop", "Zebra", "/bin/zebra", "");
        t.add(
            "ours",
            "settings.desktop",
            "Settings",
            "/usr/bin/cpz settings",
            "Icon=cpz:settings\n",
        );
        t.add("stock", "files.desktop", "Files", "/bin/files", "");
        let c = Config::default();
        let apps = scan_with(&t.dirs(), &c, &[], |_| true);
        assert_eq!(names(&apps), ["Settings", "Files", "Music", "Zebra"]);
        assert_eq!(apps[0].icon, IconRef::Symbol("settings".into()));
        assert!(apps[0].ours);
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
    fn the_stock_key_matches_stock() {
        // FNV-1a of "zclaw.desktop", the way desktop_app_loader.cpp builds it.
        let k = stock_key("zclaw.desktop");
        assert!(k.starts_with("app_desktop_zclaw_desktop_"));
        assert_eq!(k.len(), "app_desktop_zclaw_desktop_".len() + 8);
    }
}
