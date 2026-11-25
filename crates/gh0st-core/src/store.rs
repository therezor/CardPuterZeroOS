//! The Store's files, shared by the engine (`gh0st store`, root) and the
//! Store app: the signed catalog, its descriptions and icon pack, the state
//! of the last run, and what dpkg has installed. Formats are in
//! docs/store.md.

use crate::debver;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = "/var/lib/gh0st/store";
pub const RUN_DIR: &str = "/run/gh0st-store";
pub const CACHE_DIR: &str = "/var/cache/gh0st/store";
pub const DPKG_STATUS: &str = "/var/lib/dpkg/status";
/// The apt repo's key, which signs the catalog too.
pub const KEYRING: &str = "/usr/share/keyrings/gh0st-archive-keyring.pgp";
/// The first line of a catalog this build reads.
pub const MAGIC: &str = "gh0st-store 1";

/// The verified catalog and what came with it, or `$GH0ST_STORE_STATE`.
pub fn state_dir() -> PathBuf {
    dir("GH0ST_STORE_STATE", STATE_DIR)
}

/// Progress and the job the helper asked for, or `$GH0ST_STORE_RUN`.
pub fn run_dir() -> PathBuf {
    dir("GH0ST_STORE_RUN", RUN_DIR)
}

/// Debs while they download, or `$GH0ST_STORE_CACHE`.
pub fn cache_dir() -> PathBuf {
    dir("GH0ST_STORE_CACHE", CACHE_DIR)
}

/// dpkg's status file, or `$GH0ST_DPKG_STATUS`.
pub fn dpkg_status() -> PathBuf {
    dir("GH0ST_DPKG_STATUS", DPKG_STATUS)
}

/// Our apt source, from gh0st-base. The store site sits beside the repo.
pub const SOURCES: &str = "/etc/apt/sources.list.d/gh0st.sources";

/// The store site: `$GH0ST_STORE_URL`, or the apt source's URI with
/// `/store` in place of `/apt`. None without a Gh0st OS source.
pub fn site() -> Option<String> {
    if let Ok(u) = std::env::var("GH0ST_STORE_URL") {
        return Some(u.trim_end_matches('/').to_string());
    }
    site_of(&std::fs::read_to_string(SOURCES).ok()?)
}

fn site_of(sources: &str) -> Option<String> {
    let uri = sources.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("uris").then(|| v.trim())
    })?;
    let base = uri.split_whitespace().next()?.trim_end_matches('/');
    let base = base.strip_suffix("/apt")?;
    base.starts_with("https://")
        .then(|| format!("{base}/store"))
}

/// The key the catalog must be signed with, or `$GH0ST_STORE_KEYRING`.
pub fn keyring() -> PathBuf {
    dir("GH0ST_STORE_KEYRING", KEYRING)
}

fn dir(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

pub fn catalog_path() -> PathBuf {
    state_dir().join("catalog")
}

pub fn about_path() -> PathBuf {
    state_dir().join("about")
}

pub fn icons_path() -> PathBuf {
    state_dir().join("icons")
}

pub fn state_path() -> PathBuf {
    state_dir().join("state")
}

pub fn progress_path() -> PathBuf {
    run_dir().join("progress")
}

pub fn job_path() -> PathBuf {
    run_dir().join("job")
}

/// What an app's package does that the user should know before it runs as
/// root, and what it asks of the device. One bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(pub u16);

impl Flags {
    /// It has maintainer scripts: they run as root on install and remove.
    pub const SCRIPTS: Flags = Flags(1);
    /// It writes outside /usr/bin, its own folders and the app folder.
    pub const PATHS: Flags = Flags(1 << 1);
    /// It starts a system service.
    pub const SERVICE: Flags = Flags(1 << 2);
    pub const NETWORK: Flags = Flags(1 << 3);
    pub const CAMERA: Flags = Flags(1 << 4);
    pub const MICROPHONE: Flags = Flags(1 << 5);
    /// Something on the pins or the Grove port.
    pub const HARDWARE: Flags = Flags(1 << 6);
    pub const DISPLAY: Flags = Flags(1 << 7);
    pub const IMU: Flags = Flags(1 << 8);
    /// It asks for root as it runs (sudo, pkexec, a setuid file), which a
    /// sandbox can't give: it runs outside one.
    pub const ROOT: Flags = Flags(1 << 9);

    const NAMES: [(Flags, &'static str); 10] = [
        (Flags::SCRIPTS, "scripts"),
        (Flags::PATHS, "paths"),
        (Flags::SERVICE, "service"),
        (Flags::NETWORK, "network"),
        (Flags::CAMERA, "camera"),
        (Flags::MICROPHONE, "microphone"),
        (Flags::HARDWARE, "hardware"),
        (Flags::DISPLAY, "display"),
        (Flags::IMU, "imu"),
        (Flags::ROOT, "root"),
    ];

    pub fn has(self, f: Flags) -> bool {
        self.0 & f.0 == f.0
    }

    /// Flags that ask for a confirmation before install.
    pub fn risky(self) -> bool {
        self.has(Flags::SCRIPTS) || self.has(Flags::PATHS) || self.has(Flags::SERVICE)
    }

    /// `scripts,network`. Unknown words are skipped, so an older device
    /// reads a newer catalog.
    pub fn parse(s: &str) -> Flags {
        let mut f = 0;
        for w in s.split(',') {
            if let Some((bit, _)) = Flags::NAMES.iter().find(|(_, n)| *n == w) {
                f |= bit.0;
            }
        }
        Flags(f)
    }

    pub fn text(self) -> String {
        let words: Vec<&str> = Flags::NAMES
            .iter()
            .filter(|(b, _)| self.has(*b))
            .map(|(_, n)| *n)
            .collect();
        if words.is_empty() {
            "-".into()
        } else {
            words.join(",")
        }
    }
}

/// One app in the catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct App {
    pub package: String,
    pub version: String,
    /// The deb's size in bytes.
    pub size: u64,
    /// The deb's SHA-256, lower-case hex.
    pub sha256: String,
    pub category: String,
    pub flags: Flags,
    pub title: String,
    pub summary: String,
    pub author: String,
    pub licence: String,
    /// The deb on M5's GitHub release, and on their mirror in China.
    pub url: String,
    pub cn_url: String,
    /// M5's four-letter share code.
    pub share: String,
    /// The stem of the `.desktop` file it installs, to open it.
    pub desktop: String,
}

/// A file that comes with the catalog, pinned by it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pinned {
    pub sha256: String,
    pub size: u64,
}

/// The catalog marks the app with `.desktop` stem `desktop` as one that
/// needs root as it runs. Read at each launch: a few milliseconds.
pub fn needs_root(desktop: &str) -> bool {
    Catalog::load()
        .and_then(|c| c.by_desktop(desktop).map(|a| a.flags.has(Flags::ROOT)))
        .unwrap_or(false)
}

/// The catalog: what the Store lists. Signed by our apt key; the engine
/// checks the signature before it lands in [`state_dir`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// When CI built it, Unix seconds. An older one never replaces a newer.
    pub date: u64,
    /// Past this the engine refuses it: a stale catalog can't hide updates
    /// for long.
    pub expires: u64,
    pub about: Pinned,
    pub icons: Pinned,
    pub apps: Vec<App>,
}

impl Catalog {
    /// Parses a catalog. Lines this build doesn't know are skipped; a bad
    /// app line is an error, so a broken catalog never half-loads.
    pub fn parse(text: &str) -> Result<Catalog, String> {
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err("not a catalog this version reads".into());
        }
        let mut c = Catalog::default();
        for (n, line) in lines.enumerate() {
            let (key, rest) = line.split_once(['\t', ' ']).unwrap_or((line, ""));
            match key {
                "date" => c.date = rest.trim().parse().map_err(|_| bad(n, "date"))?,
                "expires" => c.expires = rest.trim().parse().map_err(|_| bad(n, "expires"))?,
                "about" => c.about = pinned(rest).ok_or_else(|| bad(n, "about"))?,
                "icons" => c.icons = pinned(rest).ok_or_else(|| bad(n, "icons"))?,
                "app" => c.apps.push(app(rest).ok_or_else(|| bad(n, "app"))?),
                _ => {}
            }
        }
        if c.date == 0 || c.expires == 0 {
            return Err("no date".into());
        }
        Ok(c)
    }

    pub fn load() -> Option<Catalog> {
        Catalog::parse(&std::fs::read_to_string(catalog_path()).ok()?).ok()
    }

    /// The app that installs `<desktop>.desktop`.
    pub fn by_desktop(&self, desktop: &str) -> Option<&App> {
        self.apps
            .iter()
            .find(|a| !a.desktop.is_empty() && a.desktop == desktop)
    }

    /// Whether this catalog, already verified, replaces `old` at `now`.
    /// One no newer than `old` changes nothing: a replayed catalog can't
    /// bring back a version with a known hole, and a CDN edge may still
    /// serve the last one. An expired one is refused.
    pub fn fresh(&self, old: Option<&Catalog>, now: u64) -> Result<bool, Outcome> {
        if old.is_some_and(|o| self.date <= o.date) {
            return Ok(false);
        }
        if self.expires <= now {
            return Err(Outcome::Stale);
        }
        Ok(true)
    }

    pub fn find(&self, package: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.package == package)
    }

    /// The categories in the order they first appear.
    pub fn categories(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for a in &self.apps {
            if !out.contains(&a.category.as_str()) {
                out.push(&a.category);
            }
        }
        out
    }

    /// The catalog's text: what CI signs and [`parse`](Catalog::parse) reads.
    pub fn text(&self) -> String {
        let mut s = format!(
            "{MAGIC}\ndate {}\nexpires {}\nabout {} {}\nicons {} {}\n",
            self.date,
            self.expires,
            self.about.sha256,
            self.about.size,
            self.icons.sha256,
            self.icons.size
        );
        for a in &self.apps {
            let f = [
                a.package.as_str(),
                &a.version,
                &a.size.to_string(),
                &a.sha256,
                &a.category,
                &a.flags.text(),
                &a.title,
                &a.summary,
                &a.author,
                &a.licence,
                &a.url,
                &a.cn_url,
                &a.share,
                &a.desktop,
            ];
            let f: Vec<String> = f.iter().map(|x| clean(x)).collect();
            s.push_str("app\t");
            s.push_str(&f.join("\t"));
            s.push('\n');
        }
        s
    }
}

fn bad(n: usize, what: &str) -> String {
    format!("line {}: bad {what}", n + 2)
}

fn pinned(rest: &str) -> Option<Pinned> {
    let mut f = rest.split_whitespace();
    let sha256 = f.next().filter(|h| is_sha256(h))?.to_string();
    let size = f.next()?.parse().ok()?;
    Some(Pinned { sha256, size })
}

fn app(rest: &str) -> Option<App> {
    let f: Vec<&str> = rest.split('\t').collect();
    if f.len() < 13 || !is_package(f[0]) || !is_sha256(f[3]) {
        return None;
    }
    let s = |i: usize| f[i].to_string();
    Some(App {
        package: s(0),
        version: s(1),
        size: f[2].parse().ok()?,
        sha256: s(3),
        category: s(4),
        flags: Flags::parse(f[5]),
        title: s(6),
        summary: s(7),
        author: s(8),
        licence: s(9),
        url: s(10),
        cn_url: s(11),
        share: s(12),
        desktop: f.get(13).map_or(String::new(), |d| d.to_string()),
    })
}

/// One field of a tab-separated line: no tab, no newline.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Debian's package name grammar: lower case, digits, `+ - .`, two or more
/// characters, starting with a letter or digit.
/// An install request: `package`, or `package=version` for the version the
/// user saw, which the engine then insists on.
pub fn split_seen(s: &str) -> (&str, Option<&str>) {
    match s.split_once('=') {
        Some((p, v)) => (p, Some(v)),
        None => (s, None),
    }
}

/// A `package` or `package=version` token.
pub fn is_wanted(s: &str) -> bool {
    match split_seen(s) {
        (p, None) => is_package(p),
        (p, Some(v)) => is_package(p) && is_version(v),
    }
}

/// A Debian version's characters, starting with a digit.
pub fn is_version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'~' | b':'))
}

pub fn is_package(s: &str) -> bool {
    s.len() >= 2
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'+' | b'-' | b'.'))
}

/// A detail page's text: the description and the screenshots, read only
/// when the page opens. One app a line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    pub description: String,
    /// SHA-256 and URL of each 320x170 screenshot.
    pub shots: Vec<(String, String)>,
}

impl About {
    /// Finds `package` in an about file: `package \t text \t sha url...`,
    /// with `\n` in the text for a new paragraph.
    pub fn find(text: &str, package: &str) -> Option<About> {
        let line = text
            .lines()
            .find(|l| l.split('\t').next() == Some(package))?;
        let mut f = line.split('\t').skip(1);
        let description = f.next().unwrap_or("").replace("\\n", "\n");
        let shots = f
            .filter_map(|s| {
                let (h, u) = s.split_once(' ')?;
                is_sha256(h).then(|| (h.to_string(), u.to_string()))
            })
            .collect();
        Some(About { description, shots })
    }

    pub fn load(package: &str) -> Option<About> {
        About::find(&std::fs::read_to_string(about_path()).ok()?, package)
    }

    /// The line [`find`](About::find) reads.
    pub fn line(&self, package: &str) -> String {
        let mut s = format!(
            "{package}\t{}",
            clean(&self.description.replace('\n', "\\n"))
        );
        for (h, u) in &self.shots {
            s.push_str(&format!("\t{h} {}", clean(u)));
        }
        s.push('\n');
        s
    }
}

/// The icon pack: one small PNG per app. A text index, a blank line, then
/// the PNGs back to back:
///
/// ```text
/// gh0st-icons 1
/// acidzero 0 1834
/// snake 1834 2210
///
/// <PNG bytes>
/// ```
pub struct Icons {
    data: Vec<u8>,
    /// Where the PNGs start, then each package's offset and length.
    base: usize,
    index: HashMap<String, (usize, usize)>,
}

impl Icons {
    pub fn parse(data: Vec<u8>) -> Option<Icons> {
        let end = data.windows(2).position(|w| w == b"\n\n")?;
        let head = std::str::from_utf8(&data[..end]).ok()?;
        let mut lines = head.lines();
        if lines.next()? != "gh0st-icons 1" {
            return None;
        }
        let base = end + 2;
        let mut index = HashMap::new();
        for l in lines {
            let mut f = l.split(' ');
            let (Some(p), Some(o), Some(n)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(o), Ok(n)) = (o.parse::<usize>(), n.parse::<usize>()) else {
                continue;
            };
            if base + o + n <= data.len() {
                index.insert(p.to_string(), (o, n));
            }
        }
        Some(Icons { data, base, index })
    }

    pub fn load() -> Option<Icons> {
        Icons::parse(std::fs::read(icons_path()).ok()?)
    }

    /// The PNG bytes of `package`'s icon.
    pub fn png(&self, package: &str) -> Option<&[u8]> {
        let &(o, n) = self.index.get(package)?;
        self.data.get(self.base + o..self.base + o + n)
    }

    /// Builds a pack, for CI and tests.
    pub fn build(icons: &[(&str, &[u8])]) -> Vec<u8> {
        let mut head = String::from("gh0st-icons 1\n");
        let mut body = Vec::new();
        for (p, png) in icons {
            head.push_str(&format!("{p} {} {}\n", body.len(), png.len()));
            body.extend_from_slice(png);
        }
        head.push('\n');
        let mut out = head.into_bytes();
        out.extend(body);
        out
    }
}

/// How the last engine run ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    None,
    Ok,
    Offline,
    /// The catalog's signature didn't check out.
    BadSignature,
    /// The catalog is past its expiry date.
    Stale,
    /// The device's clock is behind the catalog's signature.
    Clock,
    /// A deb didn't match the catalog's SHA-256.
    Checksum,
    /// apt couldn't find a package the app needs.
    Depends,
    /// Another install holds dpkg's lock.
    Locked,
    Space,
    Battery,
    /// apt or dpkg failed some other way.
    Failed,
}

impl Outcome {
    const WORDS: [(Outcome, &'static str); 12] = [
        (Outcome::None, "none"),
        (Outcome::Ok, "ok"),
        (Outcome::Offline, "offline"),
        (Outcome::BadSignature, "signature"),
        (Outcome::Stale, "stale"),
        (Outcome::Clock, "clock"),
        (Outcome::Checksum, "checksum"),
        (Outcome::Depends, "depends"),
        (Outcome::Locked, "locked"),
        (Outcome::Space, "space"),
        (Outcome::Battery, "battery"),
        (Outcome::Failed, "failed"),
    ];

    pub fn word(self) -> &'static str {
        Outcome::WORDS
            .iter()
            .find(|(o, _)| *o == self)
            .map_or("none", |(_, w)| w)
    }

    pub fn parse(s: &str) -> Outcome {
        Outcome::WORDS
            .iter()
            .find(|(_, w)| *w == s)
            .map_or(Outcome::Failed, |(o, _)| *o)
    }
}

/// What the engine last did: `key=value` lines in [`state_path`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// When the catalog was last fetched and checked, Unix seconds.
    pub refreshed: u64,
    /// When the last run ended, so two runs that end alike still differ.
    pub ended: u64,
    /// The last run: `refresh`, `install` or `remove`, and how it ended.
    pub op: String,
    pub outcome: Outcome,
    /// The packages it worked on, and the one that failed.
    pub packages: Vec<String>,
    pub failed: String,
    /// apt's own words, for the details line.
    pub detail: String,
}

impl State {
    pub fn parse(text: &str) -> State {
        let mut s = State::default();
        for l in text.lines() {
            let Some((k, v)) = l.split_once('=') else {
                continue;
            };
            match k {
                "refreshed" => s.refreshed = v.parse().unwrap_or(0),
                "ended" => s.ended = v.parse().unwrap_or(0),
                "op" => s.op = v.into(),
                "outcome" => s.outcome = Outcome::parse(v),
                "packages" => s.packages = v.split_whitespace().map(String::from).collect(),
                "failed" => s.failed = v.into(),
                "detail" => s.detail = v.into(),
                _ => {}
            }
        }
        s
    }

    pub fn load() -> State {
        std::fs::read_to_string(state_path())
            .map(|t| State::parse(&t))
            .unwrap_or_default()
    }

    pub fn text(&self) -> String {
        format!(
            "refreshed={}\nended={}\nop={}\noutcome={}\npackages={}\nfailed={}\ndetail={}\n",
            self.refreshed,
            self.ended,
            self.op,
            self.outcome.word(),
            self.packages.join(" "),
            self.failed,
            self.detail.replace('\n', " "),
        )
    }
}

/// Where debs come from. The catalog names both; the SHA-256 pins the
/// file either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mirror {
    /// China's mirror in a Chinese time zone, else GitHub.
    #[default]
    Auto,
    GitHub,
    China,
}

impl Mirror {
    pub const ALL: [Mirror; 3] = [Mirror::Auto, Mirror::GitHub, Mirror::China];

    pub fn word(self) -> &'static str {
        match self {
            Mirror::Auto => "auto",
            Mirror::GitHub => "github",
            Mirror::China => "china",
        }
    }

    pub fn parse(s: &str) -> Option<Mirror> {
        Mirror::ALL.into_iter().find(|m| m.word() == s)
    }

    /// The deb's URL for `app` from this mirror, in `zone`.
    pub fn url<'a>(self, app: &'a App, zone: &str) -> &'a str {
        let china = match self {
            Mirror::Auto => china_zone(zone),
            Mirror::GitHub => false,
            Mirror::China => true,
        };
        if china && !app.cn_url.is_empty() {
            &app.cn_url
        } else {
            &app.url
        }
    }
}

/// A job and the mirror it downloads from: what the helper writes to
/// [`job_path`] for the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub job: Job,
    pub mirror: Mirror,
}

impl Request {
    pub fn parse(text: &str) -> Option<Request> {
        let mut lines = text.lines();
        let job = Job::parse(lines.next()?)?;
        let mut mirror = Mirror::Auto;
        for l in lines {
            match l.split_once(' ') {
                Some(("mirror", m)) => mirror = Mirror::parse(m.trim())?,
                _ if l.trim().is_empty() => {}
                _ => return None,
            }
        }
        Some(Request { job, mirror })
    }

    pub fn text(&self) -> String {
        format!("{}mirror {}\n", self.job.text(), self.mirror.word())
    }
}

/// What the helper asks the engine to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Refresh,
    Install(Vec<String>),
    Remove(String),
    /// Every installed app with a newer version in the catalog.
    Upgrade,
}

impl Job {
    pub fn parse(text: &str) -> Option<Job> {
        let mut w = text.split_whitespace();
        let job = match w.next()? {
            "refresh" => Job::Refresh,
            "upgrade" => Job::Upgrade,
            "install" => {
                let p: Vec<String> = w.by_ref().map(String::from).collect();
                if p.is_empty() || !p.iter().all(|p| is_wanted(p)) {
                    return None;
                }
                Job::Install(p)
            }
            "remove" => Job::Remove(w.next().filter(|p| is_package(p))?.to_string()),
            _ => return None,
        };
        w.next().is_none().then_some(job)
    }

    pub fn text(&self) -> String {
        match self {
            Job::Refresh => "refresh\n".into(),
            Job::Upgrade => "upgrade\n".into(),
            Job::Install(p) => format!("install {}\n", p.join(" ")),
            Job::Remove(p) => format!("remove {p}\n"),
        }
    }
}

/// The installed version of each package in `wanted`, from dpkg's status
/// file: one read, no `dpkg-query` per app.
pub fn installed(status: &str, wanted: impl Fn(&str) -> bool) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for record in status.split("\n\n") {
        let (mut name, mut version, mut ok) = ("", "", false);
        for l in record.lines() {
            if let Some(v) = l.strip_prefix("Package: ") {
                name = v.trim();
            } else if let Some(v) = l.strip_prefix("Version: ") {
                version = v.trim();
            } else if let Some(v) = l.strip_prefix("Status: ") {
                ok = v.trim().ends_with(" installed");
            }
        }
        if ok && !name.is_empty() && wanted(name) {
            out.insert(name.to_string(), version.to_string());
        }
    }
    out
}

/// Reads [`dpkg_status`] for the catalog's packages.
pub fn installed_apps(catalog: &Catalog) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(dpkg_status()) else {
        return HashMap::new();
    };
    installed(&text, |p| catalog.find(p).is_some())
}

/// The catalog apps that have a newer version than the one installed.
pub fn updates<'a>(catalog: &'a Catalog, installed: &HashMap<String, String>) -> Vec<&'a App> {
    catalog
        .apps
        .iter()
        .filter(|a| {
            installed
                .get(&a.package)
                .is_some_and(|v| debver::newer(&a.version, v))
        })
        .collect()
}

/// The China mirror for a device whose zone is one of these, as M5's Store
/// picks it: by the clock's zone, with no network call.
pub fn china_zone(zone: &str) -> bool {
    matches!(
        zone,
        "Asia/Shanghai"
            | "Asia/Chongqing"
            | "Asia/Chungking"
            | "Asia/Harbin"
            | "Asia/Urumqi"
            | "Asia/Kashgar"
            | "Asia/Hong_Kong"
            | "Asia/Macau"
            | "PRC"
    )
}

/// The device's zone name from /etc/localtime's link, or `$TZ`.
pub fn local_zone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        return tz.trim_start_matches(':').to_string();
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().into_owned();
            s.split_once("zoneinfo/").map(|(_, z)| z.to_string())
        })
        .unwrap_or_default()
}

/// Whether a file's SHA-256 matches, by `sha256sum`: no hash crate.
pub fn file_sha256(path: &Path) -> Option<String> {
    let out = std::process::Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let h = s.split_whitespace().next()?.to_string();
    is_sha256(&h).then_some(h)
}

impl Pinned {
    /// Whether the file at `path` is the one pinned: size first, as that
    /// needs no read.
    pub fn matches(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.len() == self.size)
            && file_sha256(path).as_deref() == Some(self.sha256.as_str())
    }
}

/// Whether `sig` is a detached signature over `data` by [`keyring`], checked
/// by `sqv`, apt's own verifier. The image has no gpgv.
pub fn signed(data: &Path, sig: &Path) -> bool {
    signed_by(&keyring(), data, sig)
}

fn signed_by(keyring: &Path, data: &Path, sig: &Path) -> bool {
    // The positional form: sqv 1.1 on CI's Ubuntu has no --signature-file.
    std::process::Command::new("sqv")
        .arg("--keyring")
        .arg(keyring)
        .arg("--")
        .arg(sig)
        .arg(data)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Checks a downloaded catalog and its signature: signed by our key,
/// readable and unexpired. `None` when it is no newer than `old`.
pub fn verify(
    data: &Path,
    sig: &Path,
    old: Option<&Catalog>,
    now: u64,
) -> Result<Option<Catalog>, Outcome> {
    let text = std::fs::read_to_string(data).map_err(|_| Outcome::Failed)?;
    if !signed(data, sig) {
        // sqv refuses a signature from the future. Say so, rather than
        // blame the signature, when the clock is what's wrong.
        return Err(match Catalog::parse(&text) {
            Ok(c) if c.date > now + CLOCK_SLACK => Outcome::Clock,
            _ => Outcome::BadSignature,
        });
    }
    // Signed but unreadable: a format this build doesn't know.
    let c = Catalog::parse(&text).map_err(|_| Outcome::Failed)?;
    Ok(c.fresh(old, now)?.then_some(c))
}

/// How far ahead of the clock a catalog's date may be before the clock is
/// the suspect. sqv allows half an hour.
const CLOCK_SLACK: u64 = 1800;

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Catalog {
        let h = |c: char| c.to_string().repeat(64);
        Catalog {
            date: 1_763_870_000,
            expires: 1_766_462_000,
            about: Pinned {
                sha256: h('a'),
                size: 10,
            },
            icons: Pinned {
                sha256: h('b'),
                size: 20,
            },
            apps: vec![
                App {
                    package: "acidzero".into(),
                    version: "0.2.0".into(),
                    size: 58_940,
                    sha256: h('c'),
                    category: "Creative & Office".into(),
                    flags: Flags::parse("hardware"),
                    title: "AcidZero".into(),
                    summary: "Acid bass synth\twith a sequencer".into(),
                    author: "kurogedelic".into(),
                    licence: "MIT".into(),
                    url: "https://github.com/x/acidzero_0.2.0_arm64.deb".into(),
                    cn_url: "https://oss.example/acidzero_0.2.0_arm64.deb".into(),
                    share: "ACID".into(),
                    desktop: "acidzero".into(),
                },
                App {
                    package: "zsnake".into(),
                    version: "1:1.0-2".into(),
                    size: 9_000,
                    sha256: h('d'),
                    category: "Games".into(),
                    flags: Flags::parse("scripts,network,future"),
                    title: "Snake".into(),
                    ..App::default()
                },
            ],
        }
    }

    #[test]
    fn a_catalog_reads_back_what_it_wrote() {
        let c = sample();
        let back = Catalog::parse(&c.text()).unwrap();
        assert_eq!(back.apps[0].summary, "Acid bass synth with a sequencer");
        assert_eq!(back.apps[1].flags.text(), "scripts,network");
        assert!(back.apps[1].flags.risky());
        assert!(!back.apps[0].flags.risky());
        assert_eq!(back.date, c.date);
        assert_eq!(back.icons, c.icons);
        assert_eq!(back.categories(), ["Creative & Office", "Games"]);
    }

    #[test]
    fn a_bad_line_fails_the_whole_catalog() {
        let mut t = sample().text();
        assert!(Catalog::parse("gh0st-store 2\ndate 1\n").is_err());
        t.push_str("app\tBad Name\t1\t1\t");
        assert!(Catalog::parse(&t).is_err());
        assert!(Catalog::parse(&format!("{MAGIC}\napp\n")).is_err());
        let unknown = format!("{}future thing\n", sample().text());
        assert!(Catalog::parse(&unknown).is_ok());
    }

    #[test]
    fn about_finds_one_app_and_its_shots() {
        let a = About {
            description: "One.\nTwo".into(),
            shots: vec![("e".repeat(64), "https://x/1.png".into())],
        };
        let text = format!("{}{}", About::default().line("other"), a.line("acidzero"));
        assert_eq!(About::find(&text, "acidzero"), Some(a));
        assert_eq!(About::find(&text, "nope"), None);
    }

    #[test]
    fn icons_index_their_pngs() {
        let pack = Icons::build(&[("a", b"PNG-A"), ("bb", b"PNG-BB")]);
        let i = Icons::parse(pack).unwrap();
        assert_eq!(i.png("bb"), Some(&b"PNG-BB"[..]));
        assert_eq!(i.png("a"), Some(&b"PNG-A"[..]));
        assert_eq!(i.png("c"), None);
    }

    #[test]
    fn installed_reads_only_installed_wanted_packages() {
        let status = "Package: acidzero\nStatus: install ok installed\nVersion: 0.1.0\n\n\
                      Package: zsnake\nStatus: deinstall ok config-files\nVersion: 1.0\n\n\
                      Package: libc6\nStatus: install ok installed\nVersion: 2.41\n";
        let c = sample();
        let got = installed(status, |p| c.find(p).is_some());
        assert_eq!(got.len(), 1);
        assert_eq!(got["acidzero"], "0.1.0");
        let up = updates(&c, &got);
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].package, "acidzero");
    }

    #[test]
    fn jobs_take_only_package_names() {
        assert_eq!(
            Job::parse("install a1 b-2"),
            Some(Job::Install(vec!["a1".into(), "b-2".into()]))
        );
        assert_eq!(Job::parse("install"), None);
        assert_eq!(Job::parse("install ../etc"), None);
        assert_eq!(Job::parse("remove x y"), None);
        assert_eq!(Job::parse("remove ab"), Some(Job::Remove("ab".into())));
        for j in [Job::Refresh, Job::Upgrade, Job::Remove("ab".into())] {
            assert_eq!(Job::parse(&j.text()), Some(j));
        }
    }

    #[test]
    fn a_request_names_its_mirror() {
        let r = Request {
            job: Job::Install(vec!["acidzero".into()]),
            mirror: Mirror::China,
        };
        assert_eq!(Request::parse(&r.text()), Some(r));
        assert_eq!(
            Request::parse("refresh\n").map(|r| r.mirror),
            Some(Mirror::Auto)
        );
        assert_eq!(Request::parse("refresh\nmirror moon\n"), None);
        let c = sample();
        let a = &c.apps[0];
        assert_eq!(Mirror::Auto.url(a, "Asia/Shanghai"), a.cn_url);
        assert_eq!(Mirror::Auto.url(a, "Europe/London"), a.url);
        assert_eq!(Mirror::GitHub.url(a, "Asia/Shanghai"), a.url);
        assert_eq!(Mirror::China.url(&c.apps[1], ""), "", "no China URL");
    }

    /// What os/store/mirror.py makes of its fixture registry.
    #[test]
    fn the_ci_mirror_writes_what_this_reads() {
        let c = Catalog::parse(include_str!("../../../os/store/testdata/catalog")).unwrap();
        let about = include_str!("../../../os/store/testdata/about");
        assert_eq!(c.about.size, about.len() as u64);
        let names: Vec<&str> = c.apps.iter().map(|a| a.package.as_str()).collect();
        assert_eq!(names, ["acidzero", "rootsetup"]);
        let acid = c.find("acidzero").unwrap();
        assert_eq!(acid.summary, "Acid bass synth with a sequencer");
        assert!(acid.cn_url.starts_with("https://") && acid.desktop == "acidzero");
        assert!(!acid.flags.risky() && acid.flags.has(Flags::HARDWARE));
        let root = c.find("rootsetup").unwrap();
        assert!(root.flags.has(Flags::SCRIPTS) && root.flags.has(Flags::SERVICE));
        assert!(root.cn_url.is_empty() && root.desktop.is_empty());
        let a = About::find(about, "acidzero").unwrap();
        assert_eq!(
            a.description,
            "AcidZero is a synth.\nKeys\n\u{2022} P: settings\n\u{2022} see the docs"
        );
        assert_eq!(a.shots.len(), 2);
    }

    #[test]
    fn an_old_catalog_changes_nothing_and_an_expired_one_is_refused() {
        let c = sample();
        let now = c.date + 60;
        assert_eq!(c.fresh(None, now), Ok(true));
        assert_eq!(c.fresh(Some(&c), now), Ok(false), "the same catalog again");
        let newer = Catalog {
            date: c.date + 1,
            ..c.clone()
        };
        assert_eq!(c.fresh(Some(&newer), now), Ok(false), "an older one");
        assert_eq!(newer.fresh(Some(&c), now), Ok(true));
        assert_eq!(c.fresh(None, c.expires), Err(Outcome::Stale));
    }

    #[test]
    fn the_store_site_sits_beside_the_apt_repo() {
        let src = include_str!("../../../os/repo/gh0st.sources");
        assert_eq!(
            site_of(src).as_deref(),
            Some("https://therezor.github.io/CardPuterZeroOS/store")
        );
        assert_eq!(site_of("URIs: http://x/apt\n"), None, "https only");
        assert_eq!(site_of("URIs: https://x/debian\n"), None);
    }

    #[test]
    fn a_pinned_file_matches_by_size_and_hash() {
        // Another test's waitpid(-1) would take this child's exit.
        let _one = crate::child::CHILD_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let p = std::env::temp_dir().join(format!("gh0st-pinned-{}", std::process::id()));
        std::fs::write(&p, "abc").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let pin = |sha256: &str, size| Pinned {
            sha256: sha256.into(),
            size,
        };
        assert!(pin(sha, 3).matches(&p));
        assert!(!pin(sha, 4).matches(&p));
        assert!(!pin(&"0".repeat(64), 3).matches(&p));
        std::fs::remove_file(&p).unwrap();
    }

    /// A real signature, made with a throwaway gpg key and checked by sqv.
    /// Skipped where either tool is missing; CI installs both.
    #[test]
    fn sqv_accepts_our_signature_and_nothing_else() {
        // Another test's waitpid(-1) would take this child's exit.
        let _one = crate::child::CHILD_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let have = |t: &str| {
            std::process::Command::new(t)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !have("sqv") || !have("gpg") {
            eprintln!("skipped: needs sqv and gpg");
            return;
        }
        let d = std::env::temp_dir().join(format!("g0sq{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let gpg = |home: &str, args: &[&str]| {
            let h = d.join(home);
            std::fs::create_dir_all(&h).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let ok = std::process::Command::new("gpg")
                .env("GNUPGHOME", &h)
                .args(["--batch", "--quiet", "--pinentry-mode", "loopback"])
                .args(["--passphrase", ""])
                .args(args)
                .current_dir(&d)
                .status()
                .unwrap()
                .success();
            assert!(ok, "gpg {args:?}");
        };
        for k in ["ours", "theirs"] {
            let uid = format!("{k} <{k}@example.invalid>");
            gpg(k, &["--quick-gen-key", &uid, "ed25519", "sign", "never"]);
            gpg(k, &["--output", &format!("{k}.pgp"), "--export", k]);
        }
        std::fs::write(d.join("catalog"), sample().text()).unwrap();
        gpg(
            "ours",
            &["--output", "ours.sig", "--detach-sign", "catalog"],
        );
        gpg(
            "theirs",
            &["--output", "theirs.sig", "--detach-sign", "catalog"],
        );
        let (cat, ours) = (d.join("catalog"), d.join("ours.pgp"));
        assert!(signed_by(&ours, &cat, &d.join("ours.sig")));
        assert!(!signed_by(&ours, &cat, &d.join("theirs.sig")));
        assert!(!signed_by(&ours, &cat, &d.join("missing.sig")));
        std::fs::write(&cat, sample().text().replace("0.2.0", "0.2.1")).unwrap();
        assert!(!signed_by(&ours, &cat, &d.join("ours.sig")));
        for k in ["ours", "theirs"] {
            let _ = std::process::Command::new("gpgconf")
                .env("GNUPGHOME", d.join(k))
                .args(["--kill", "gpg-agent"])
                .status();
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn state_round_trips() {
        let s = State {
            refreshed: 5,
            ended: 6,
            op: "install".into(),
            outcome: Outcome::Depends,
            packages: vec!["a1".into(), "b2".into()],
            failed: "b2".into(),
            detail: "libfoo is not available".into(),
        };
        assert_eq!(State::parse(&s.text()), s);
    }
}
