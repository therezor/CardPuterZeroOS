//! Notifications: one small file per notice, in a folder the shell watches
//! with inotify, so posting needs no daemon and works from any language.
//!
//! - `/var/lib/gh0st/notify/` for system services (root),
//! - `~/.local/state/gh0st/notify/` for apps.
//!
//! The file name is `<app>.<tag>`. `app` is the poster's desktop file stem
//! or program name, `tag` names the notice: posting the same tag again
//! replaces it. Names are lowercase letters, digits, `-` and `_`. The body
//! is `key=value` lines; unknown keys are ignored:
//!
//! ```text
//! title=12 updates ready
//! body=Gh0st 0.5, 3 security fixes
//! tone=info
//! time=1795000000
//! open=update
//! count=12
//! ```

use std::path::{Path, PathBuf};

pub const SYSTEM_DIR: &str = "/var/lib/gh0st/notify";
/// A larger file is not a notice.
const MAX_FILE: u64 = 2048;
/// The shell reads at most this many per folder, newest first.
const MAX_NOTICES: usize = 32;
const TITLE_MAX: usize = 60;
const BODY_MAX: usize = 120;
const NAME_MAX: usize = 48;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Info,
    Ok,
    Warning,
    Critical,
}

impl Tone {
    fn parse(s: &str) -> Tone {
        match s {
            "ok" => Tone::Ok,
            "warning" => Tone::Warning,
            "critical" => Tone::Critical,
            _ => Tone::Info,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Tone::Info => "info",
            Tone::Ok => "ok",
            Tone::Warning => "warning",
            Tone::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notice {
    /// The poster: a desktop file stem or a program name.
    pub app: String,
    pub tag: String,
    pub title: String,
    pub body: String,
    pub tone: Tone,
    /// Seconds since the epoch.
    pub time: u64,
    /// What Enter opens, named as `app` is. Empty: the poster.
    pub open: String,
    /// A number for the app's tile, such as how many updates. 0: none.
    pub count: u32,
    /// Posted to the system folder.
    pub system: bool,
}

impl Notice {
    /// `<app>.<tag>`: the file name, and the notice's id.
    pub fn id(&self) -> String {
        format!("{}.{}", self.app, self.tag)
    }

    /// The app Enter opens.
    pub fn target(&self) -> &str {
        if self.open.is_empty() {
            &self.app
        } else {
            &self.open
        }
    }

    /// Reads a notice from its file name and text. None if either is not
    /// one: a bad name, or no title.
    pub fn parse(file: &str, text: &str, system: bool) -> Option<Notice> {
        let (app, tag) = file.split_once('.')?;
        if !valid_name(app) || !valid_name(tag) {
            return None;
        }
        let mut n = Notice {
            app: app.into(),
            tag: tag.into(),
            system,
            ..Notice::default()
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "title" => n.title = clip(v, TITLE_MAX),
                "body" => n.body = clip(v, BODY_MAX),
                "tone" => n.tone = Tone::parse(v),
                "time" => n.time = v.parse().unwrap_or(0),
                "open" if valid_name(v) => n.open = v.into(),
                "count" => n.count = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        (!n.title.is_empty()).then_some(n)
    }

    /// The file's text.
    pub fn text(&self) -> String {
        let mut s = format!(
            "title={}\nbody={}\ntone={}\ntime={}\n",
            one_line(&self.title),
            one_line(&self.body),
            self.tone.word(),
            self.time
        );
        if !self.open.is_empty() {
            s.push_str(&format!("open={}\n", self.open));
        }
        if self.count > 0 {
            s.push_str(&format!("count={}\n", self.count));
        }
        s
    }
}

/// Lowercase letters, digits, `-` and `_`, not empty, not too long.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= NAME_MAX
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn clip(s: &str, max: usize) -> String {
    let s = one_line(s);
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}\u{2026}", &s[..i]),
        None => s,
    }
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// The system folder, or `$GH0ST_NOTIFY_SYSTEM` for tests and the sim.
pub fn system_dir() -> PathBuf {
    std::env::var_os("GH0ST_NOTIFY_SYSTEM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SYSTEM_DIR))
}

fn state_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".local/state/gh0st")
}

/// The apps' folder, `~/.local/state/gh0st/notify`.
pub fn user_dir() -> PathBuf {
    state_dir().join("notify")
}

/// Where the shell keeps which system notices were dismissed.
pub fn dismissed_path() -> PathBuf {
    state_dir().join("notify-dismissed")
}

/// Every notice in `dir`, newest first, at most `MAX_NOTICES`.
pub fn read_dir(dir: &Path, system: bool) -> Vec<Notice> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Notice> = entries
        .flatten()
        .filter(|e| {
            e.metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE)
        })
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let text = std::fs::read_to_string(e.path()).ok()?;
            Notice::parse(&name, &text, system)
        })
        .collect();
    out.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.id().cmp(&b.id())));
    out.truncate(MAX_NOTICES);
    out
}

/// Posts `n` into `dir`, replacing any notice with its id. The write is
/// atomic, through a hidden file the readers skip.
pub fn post(dir: &Path, n: &Notice) -> std::io::Result<()> {
    if !valid_name(&n.app) || !valid_name(&n.tag) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "notice names are lowercase letters, digits, - and _",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let id = n.id();
    let tmp = dir.join(format!(".{id}.tmp"));
    std::fs::write(&tmp, n.text())?;
    std::fs::rename(&tmp, dir.join(id))
}

/// Removes notice `app.tag` from `dir`. Gone already is fine.
pub fn remove(dir: &Path, app: &str, tag: &str) -> std::io::Result<()> {
    if !valid_name(app) || !valid_name(tag) {
        return Ok(());
    }
    match std::fs::remove_file(dir.join(format!("{app}.{tag}"))) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// System notices the user dismissed, by id and time: a newer post of the
/// same id shows again.
#[derive(Default)]
pub struct Dismissed {
    list: Vec<(String, u64)>,
}

impl Dismissed {
    pub fn load(path: &Path) -> Dismissed {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let list = text
            .lines()
            .filter_map(|l| {
                let (id, t) = l.split_once('\t')?;
                Some((id.to_string(), t.parse().ok()?))
            })
            .collect();
        Dismissed { list }
    }

    pub fn has(&self, n: &Notice) -> bool {
        self.list
            .iter()
            .any(|(id, t)| *id == n.id() && *t == n.time)
    }

    pub fn add(&mut self, n: &Notice) {
        let id = n.id();
        self.list.retain(|(i, _)| *i != id);
        self.list.push((id, n.time));
    }

    /// Forgets ids no longer posted, so the file never grows.
    pub fn keep_only(&mut self, live: &[Notice]) {
        self.list
            .retain(|(id, _)| live.iter().any(|n| n.system && n.id() == *id));
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let text: String = self
            .list
            .iter()
            .map(|(id, t)| format!("{id}\t{t}\n"))
            .collect();
        std::fs::write(path, text)
    }
}

/// Seconds since the epoch, now.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "now", "5 min", "3 h", "2 d": how long ago, for a row's right side.
pub fn age(then: u64, now: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{} min", s / 60),
        3600..86400 => format!("{} h", s / 3600),
        _ => format!("{} d", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gh0st-notify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_notice_round_trips_through_its_file() {
        let n = Notice {
            app: "gh0st-update".into(),
            tag: "updates".into(),
            title: "12 updates ready".into(),
            body: "Gh0st 0.5\nand more".into(),
            tone: Tone::Info,
            time: 1_795_000_000,
            open: "update".into(),
            count: 12,
            system: true,
        };
        let back = Notice::parse(&n.id(), &n.text(), true).unwrap();
        assert_eq!(back.body, "Gh0st 0.5 and more");
        assert_eq!(
            back,
            Notice {
                body: back.body.clone(),
                ..n
            }
        );
    }

    #[test]
    fn bad_names_and_empty_titles_are_not_notices() {
        assert!(Notice::parse("Updates.x", "title=a", false).is_none());
        assert!(Notice::parse("app", "title=a", false).is_none());
        assert!(Notice::parse(".app.tag.tmp", "title=a", false).is_none());
        assert!(Notice::parse("app.tag", "body=no title", false).is_none());
        assert!(
            Notice::parse("app.tag", "title=a\nopen=../etc", false)
                .unwrap()
                .open
                .is_empty()
        );
    }

    #[test]
    fn long_text_is_clipped_on_a_character() {
        let n = Notice::parse("a.b", &format!("title={}", "é".repeat(80)), false).unwrap();
        assert_eq!(n.title.chars().count(), TITLE_MAX + 1);
        assert!(n.title.ends_with('\u{2026}'));
    }

    #[test]
    fn posting_a_tag_again_replaces_it_and_reads_come_newest_first() {
        let d = temp("post");
        let mut n = Notice {
            app: "rec".into(),
            tag: "saved".into(),
            title: "Saved".into(),
            time: 10,
            ..Notice::default()
        };
        post(&d, &n).unwrap();
        n.title = "Saved again".into();
        post(&d, &n).unwrap();
        let other = Notice {
            tag: "older".into(),
            title: "Older".into(),
            time: 5,
            ..n.clone()
        };
        post(&d, &other).unwrap();
        std::fs::write(d.join(".rec.x.tmp"), "title=half written").unwrap();
        let all = read_dir(&d, false);
        assert_eq!(
            all.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(),
            ["Saved again", "Older"]
        );
        remove(&d, "rec", "saved").unwrap();
        remove(&d, "rec", "saved").unwrap();
        assert_eq!(read_dir(&d, false).len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_dismissed_notice_shows_again_when_posted_anew() {
        let d = temp("dismiss");
        let path = d.join("dismissed");
        let n = Notice {
            app: "gh0st-update".into(),
            tag: "updates".into(),
            title: "3 updates".into(),
            time: 100,
            system: true,
            ..Notice::default()
        };
        let mut seen = Dismissed::default();
        seen.add(&n);
        seen.save(&path).unwrap();
        let seen = Dismissed::load(&path);
        assert!(seen.has(&n));
        assert!(!seen.has(&Notice {
            time: 200,
            ..n.clone()
        }));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ages_read_short() {
        assert_eq!(age(1000, 1030), "now");
        assert_eq!(age(1000, 1000 + 5 * 60), "5 min");
        assert_eq!(age(0, 3 * 3600), "3 h");
        assert_eq!(age(0, 2 * 86400 + 5), "2 d");
        assert_eq!(age(10, 5), "now");
    }
}
