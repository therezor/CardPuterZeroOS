//! Which app opens a file. Apps say what they open with `MimeType=` and a
//! field code in `Exec`; the user's choices live in the XDG
//! `~/.config/mimeapps.list`, so `gio` and `xdg-mime` agree with us.
//!
//! For each type in a file's ancestry, nearest first: the user's default for
//! it, then apps added for it, then the apps that declare it, ours before the
//! rest. The first hit is the default; all of them, once each, are the
//! Open-with list.

use crate::apps::App;
use crate::mime;
use std::path::{Path, PathBuf};

const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const REMOVED: &str = "Removed Associations";

/// `~/.config/mimeapps.list`, or under `$XDG_CONFIG_HOME`.
pub fn list_path() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/".into())
                .join(".config")
        });
    config.join("mimeapps.list")
}

/// `mimeapps.list` as lines, so writing it back keeps what we don't read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MimeApps {
    lines: Vec<String>,
}

impl MimeApps {
    pub fn load(path: &Path) -> MimeApps {
        // A list past 256 KB is not one a person keeps: ignore it.
        let text = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= 256 * 1024)
            .and_then(|_| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        MimeApps::parse(&text)
    }

    pub fn parse(text: &str) -> MimeApps {
        MimeApps {
            lines: text.lines().map(String::from).collect(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::config::write_atomic(path, self.to_string().as_bytes())
    }

    /// The desktop ids a group lists for `mime`.
    pub fn get(&self, group: &str, mime: &str) -> Vec<&str> {
        let mut inside = false;
        for line in &self.lines {
            let t = line.trim();
            if t.starts_with('[') {
                inside = t == format!("[{group}]");
                continue;
            }
            if !inside {
                continue;
            }
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == mime {
                    return v
                        .split(';')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// Sets `mime=ids` in `group`, replacing the line or adding it.
    pub fn set(&mut self, group: &str, mime: &str, ids: &[&str]) {
        let header = format!("[{group}]");
        let value = format!("{mime}={};", ids.join(";"));
        let mut start = None;
        let mut end = self.lines.len();
        for (i, line) in self.lines.iter().enumerate() {
            let t = line.trim();
            if t.starts_with('[') {
                if start.is_some() {
                    end = i;
                    break;
                }
                if t == header {
                    start = Some(i);
                }
            }
        }
        let Some(start) = start else {
            if !ids.is_empty() {
                if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    self.lines.push(String::new());
                }
                self.lines.push(header);
                self.lines.push(value);
            }
            return;
        };
        let found = (start + 1..end).find(|&i| {
            self.lines[i]
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == mime)
        });
        match (found, ids.is_empty()) {
            (Some(i), true) => {
                self.lines.remove(i);
            }
            (Some(i), false) => self.lines[i] = value,
            (None, false) => {
                // After the group's last entry, before any blank lines.
                let mut at = end;
                while at > start + 1 && self.lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                self.lines.insert(at, value);
            }
            (None, true) => {}
        }
    }
}

impl std::fmt::Display for MimeApps {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for l in &self.lines {
            writeln!(f, "{l}")?;
        }
        Ok(())
    }
}

/// An app named in mimeapps.list: `text.desktop`, or the
/// `gh0st-text.desktop` name our entries also have under
/// `/usr/share/applications`.
fn named<'a>(apps: &'a [App], id: &str) -> Option<&'a App> {
    apps.iter().find(|a| a.file == id).or_else(|| {
        let bare = id.strip_prefix("gh0st-")?;
        apps.iter().find(|a| a.ours && a.file == bare)
    })
}

fn declares(app: &App, mime: &str) -> bool {
    app.entry.mime_types.iter().any(|p| mime::matches(p, mime))
}

/// The apps that open `mime`, best first, each once. `apps` are the
/// openers ([`crate::apps::openers`]).
pub fn all_for<'a>(apps: &'a [App], list: &MimeApps, mime: &str) -> Vec<&'a App> {
    let mut out: Vec<&App> = Vec::new();
    let mut push = |a: &'a App| {
        if !out.iter().any(|o| std::ptr::eq(*o, a)) {
            out.push(a);
        }
    };
    for m in mime::ancestry(mime) {
        let removed = list.get(REMOVED, &m);
        let ok = |a: &&App| a.opens_files() && !removed.contains(&a.file.as_str());
        for id in list
            .get(DEFAULTS, &m)
            .into_iter()
            .chain(list.get(ADDED, &m))
        {
            if let Some(a) = named(apps, id).filter(ok) {
                push(a);
            }
        }
        let declared = apps.iter().filter(|a| declares(a, &m)).filter(ok);
        let (ours, rest): (Vec<&App>, Vec<&App>) = declared.partition(|a| a.ours);
        for a in ours.into_iter().chain(rest) {
            push(a);
        }
    }
    out
}

/// The app Enter opens `mime` with.
pub fn default_for<'a>(apps: &'a [App], list: &MimeApps, mime: &str) -> Option<&'a App> {
    all_for(apps, list, mime).into_iter().next()
}

/// "Always open this type with `app`": the user's default for `mime`.
pub fn set_default(path: &Path, mime: &str, app_file: &str) -> std::io::Result<()> {
    let mut list = MimeApps::load(path);
    list.set(DEFAULTS, mime, &[app_file]);
    list.save(path)
}

/// Forgets the user's default for `mime`.
pub fn clear_default(path: &Path, mime: &str) -> std::io::Result<()> {
    let mut list = MimeApps::load(path);
    list.set(DEFAULTS, mime, &[]);
    list.save(path)
}

/// Every type the user chose a default for, with the app's id.
pub fn defaults(list: &MimeApps) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in &list.lines {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == format!("[{DEFAULTS}]");
        } else if inside {
            if let Some((k, v)) = t.split_once('=') {
                if let Some(first) = v.split(';').map(str::trim).find(|s| !s.is_empty()) {
                    out.push((k.trim().to_string(), first.to_string()));
                }
            }
        }
    }
    out
}

/// A kind of file the user picks one app for, as Settings > Default apps
/// shows them.
pub struct Category {
    pub label: &'static str,
    pub icon: &'static str,
    /// The type whose default the row shows.
    pub sample: &'static str,
    /// The types a pick is written for.
    of: fn(&str) -> bool,
}

pub const CATEGORIES: [Category; 5] = [
    Category {
        label: "Text and code",
        icon: "file-code",
        sample: "text/plain",
        of: |m| m != mime::BINARY && mime::is_a(m, mime::TEXT),
    },
    Category {
        label: "Pictures",
        icon: "file-image",
        sample: "image/png",
        of: |m| m.starts_with("image/"),
    },
    Category {
        label: "Video",
        icon: "file-video",
        sample: "video/mp4",
        of: |m| m.starts_with("video/"),
    },
    Category {
        label: "Music",
        icon: "file-music",
        sample: "audio/mpeg",
        of: |m| m.starts_with("audio/"),
    },
    Category {
        label: "Folders",
        icon: "folder",
        sample: mime::DIR,
        of: |m| m == mime::DIR,
    },
];

impl Category {
    /// Every type a pick here is written for.
    pub fn types(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = mime::known().into_iter().filter(|m| (self.of)(m)).collect();
        if (self.of)(self.sample) && !v.contains(&self.sample) {
            v.push(self.sample);
        }
        v
    }

    /// The app its files open with now.
    pub fn default<'a>(&self, apps: &'a [App], list: &MimeApps) -> Option<&'a App> {
        default_for(apps, list, self.sample)
    }

    /// The apps that open its files.
    pub fn choices<'a>(&self, apps: &'a [App], list: &MimeApps) -> Vec<&'a App> {
        all_for(apps, list, self.sample)
    }

    /// The user picked the app, not the built-in order.
    pub fn picked(&self, list: &MimeApps) -> bool {
        !list.get(DEFAULTS, self.sample).is_empty()
    }

    /// Makes `app_file` the default for every type here.
    pub fn set(&self, path: &Path, app_file: &str) -> std::io::Result<()> {
        let mut list = MimeApps::load(path);
        for m in self.types() {
            list.set(DEFAULTS, m, &[app_file]);
        }
        list.save(path)
    }

    /// Back to the built-in order for every type here.
    pub fn reset(&self, path: &Path) -> std::io::Result<()> {
        let mut list = MimeApps::load(path);
        for m in self.types() {
            list.set(DEFAULTS, m, &[]);
        }
        list.save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::DesktopEntry;

    fn app(file: &str, exec: &str, types: &[&str], ours: bool) -> App {
        let entry = DesktopEntry {
            name: file.into(),
            exec: exec.into(),
            mime_types: types.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let mut a = App::for_test(entry, ours);
        a.file = file.into();
        a
    }

    fn ids(v: Vec<&App>) -> Vec<&str> {
        v.into_iter().map(|a| a.file.as_str()).collect()
    }

    #[test]
    fn nearest_type_first_ours_before_the_rest() {
        let apps = [
            app("store-ed.desktop", "/opt/ed %f", &["text/plain"], false),
            app(
                "text.desktop",
                "/usr/bin/gh0st-text %f",
                &["text/plain"],
                true,
            ),
            app(
                "media.desktop",
                "/usr/bin/gh0st-media %f",
                &["image/*"],
                true,
            ),
            app("nofile.desktop", "/opt/x", &["text/plain"], false),
        ];
        let none = MimeApps::default();
        assert_eq!(
            ids(all_for(&apps, &none, "text/rust")),
            ["text.desktop", "store-ed.desktop"]
        );
        assert_eq!(
            ids(all_for(&apps, &none, "image/svg+xml")),
            ["media.desktop", "text.desktop", "store-ed.desktop"]
        );
        assert_eq!(
            default_for(&apps, &none, "image/png").map(|a| a.file.as_str()),
            Some("media.desktop")
        );
        assert!(default_for(&apps, &none, "application/pdf").is_none());
    }

    #[test]
    fn the_users_choice_wins_at_its_own_level_only() {
        let apps = [
            app("store-ed.desktop", "/opt/ed %f", &["text/plain"], false),
            app(
                "text.desktop",
                "/usr/bin/gh0st-text %f",
                &["text/plain"],
                true,
            ),
            app(
                "media.desktop",
                "/usr/bin/gh0st-media %f",
                &["image/*"],
                true,
            ),
        ];
        let list = MimeApps::parse("[Default Applications]\ntext/plain=store-ed.desktop;\n");
        assert_eq!(
            default_for(&apps, &list, "text/x-python").map(|a| a.file.as_str()),
            Some("store-ed.desktop")
        );
        assert_eq!(
            default_for(&apps, &list, "image/svg+xml").map(|a| a.file.as_str()),
            Some("media.desktop")
        );
        let gone = MimeApps::parse("[Removed Associations]\ntext/plain=text.desktop\n");
        assert_eq!(
            ids(all_for(&apps, &gone, "text/plain")),
            ["store-ed.desktop"]
        );
        let named = MimeApps::parse("[Default Applications]\nimage/png=gh0st-text.desktop\n");
        assert_eq!(
            default_for(&apps, &named, "image/png").map(|a| a.file.as_str()),
            Some("text.desktop")
        );
    }

    #[test]
    fn writing_keeps_what_it_does_not_read() {
        let mut l = MimeApps::parse(
            "# mine\n[Added Associations]\nx/y=a.desktop;\n\n[Default Applications]\ntext/plain=a.desktop;\n\n",
        );
        l.set(DEFAULTS, "text/plain", &["b.desktop"]);
        l.set(DEFAULTS, "image/png", &["m.desktop"]);
        l.set(ADDED, "x/z", &["c.desktop"]);
        assert_eq!(
            l.to_string(),
            "# mine\n[Added Associations]\nx/y=a.desktop;\nx/z=c.desktop;\n\n[Default Applications]\ntext/plain=b.desktop;\nimage/png=m.desktop;\n\n"
        );
        l.set(DEFAULTS, "text/plain", &[]);
        assert_eq!(
            defaults(&l),
            [("image/png".to_string(), "m.desktop".to_string())]
        );
        let mut fresh = MimeApps::default();
        fresh.set(DEFAULTS, "a/b", &["x.desktop"]);
        assert_eq!(
            fresh.to_string(),
            "[Default Applications]\na/b=x.desktop;\n"
        );
    }

    #[test]
    fn a_category_pick_covers_its_types_and_resets() {
        let d = std::env::temp_dir().join(format!("gh0st-cat-{}", std::process::id()));
        let p = d.join("mimeapps.list");
        let apps = [
            app(
                "text.desktop",
                "/usr/bin/gh0st-text %f",
                &["text/plain", "application/json"],
                true,
            ),
            app("ed.desktop", "/opt/ed %f", &["text/plain"], false),
        ];
        let text = &CATEGORIES[0];
        assert!(text.types().contains(&"text/rust") && text.types().contains(&"application/json"));
        assert!(!text.types().contains(&"image/png"));
        text.set(&p, "ed.desktop").unwrap();
        let list = MimeApps::load(&p);
        assert!(text.picked(&list));
        // Even JSON, which Text names itself, goes to the pick.
        assert_eq!(
            default_for(&apps, &list, "application/json").map(|a| a.file.as_str()),
            Some("ed.desktop")
        );
        text.reset(&p).unwrap();
        let list = MimeApps::load(&p);
        assert!(!text.picked(&list));
        assert_eq!(
            text.default(&apps, &list).map(|a| a.file.as_str()),
            Some("text.desktop")
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn set_default_writes_the_file() {
        let d = std::env::temp_dir().join(format!("gh0st-handlers-{}", std::process::id()));
        let p = d.join("mimeapps.list");
        set_default(&p, "text/plain", "text.desktop").unwrap();
        assert_eq!(
            MimeApps::load(&p).get(DEFAULTS, "text/plain"),
            ["text.desktop"]
        );
        clear_default(&p, "text/plain").unwrap();
        assert!(MimeApps::load(&p).get(DEFAULTS, "text/plain").is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
