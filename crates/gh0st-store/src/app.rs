//! The Store's one page, in three views: the app list, an app's details,
//! and its screenshots full screen. It reads the verified catalog the
//! engine keeps, watches the engine's files with inotify, and asks
//! gh0st-helper to refresh, install or remove. Nothing touches the network
//! until the user asks.

use crate::pics::Pics;
use crate::{shots, words};
use gh0st_app::cycle;
use gh0st_app::gfx::Rect;
use gh0st_app::gfx::image::Image;
use gh0st_app::gfx::widgets::{self, AppItem, Visual};
use gh0st_app::helper::{Finished, Helper};
use gh0st_app::prelude::*;
use gh0st_core::config::key;
use gh0st_core::store::{self, About, App, Catalog, Icons, Mirror, Outcome, State};
use gh0st_core::update::{self, Phase, Progress};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

const APP_ICON: &str = "shopping-bag";
/// Dialog ids.
const ASK_PASSWORD: u32 = 900;
const ASK_INSTALL: u32 = 1;
/// Helper calls.
const REFRESH: u32 = 1;
const INSTALL: u32 = 2;
const REMOVE: u32 = 3;
const UPGRADE: u32 = 4;
/// Menu commands.
const CMD_REFRESH: u32 = 10;
const CMD_UPDATE_ALL: u32 = 11;
const CMD_MIRROR: u32 = 12;
const CMD_FIND: u32 = 13;
const CMD_LAYOUT: u32 = 14;
/// A started unit has this long to show progress before the page stops
/// saying "Starting".
const START_MS: u64 = 15_000;
const SEGMENT_MS: u64 = 60;
/// Icon sizes: a list row's badge, and the detail page's.
const ROW_ICON: u32 = 20;
const DETAIL_ICON: u32 = 30;

/// Which apps the list shows. Left and Right step through them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Updates,
    Installed,
    Category(String),
}

impl Filter {
    fn name(&self) -> &str {
        match self {
            Filter::All => "All apps",
            Filter::Updates => "Updates",
            Filter::Installed => "Installed",
            // The short name: the top bar drops a context that doesn't fit
            // beside the battery.
            Filter::Category(c) => words::category(c).1,
        }
    }
}

/// How the Store's home shows its categories, as home shows apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Grid,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum View {
    /// Home: Updates, Installed, All apps, then the categories.
    Browse,
    /// The apps of one filter, or a search through all of them.
    List,
    Detail(String),
    /// An app's screenshots, full screen, and which one.
    Shots(String, usize),
}

/// What the engine runs or was just asked to run.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Work {
    Refresh,
    /// Installing, updating or removing these packages.
    Apps(Vec<String>),
}

/// A screenshot on its way: the curl job, where it lands, its hash.
struct Fetch {
    job: Job,
    tmp: PathBuf,
    sha256: String,
    /// Downloaded; `sha256sum` now runs, and this is what it said.
    hashing: Option<String>,
}

/// Removing M5's old cache, in two child processes: `du`, then `rm`.
struct Clean {
    job: Job,
    kb: u64,
    removing: bool,
}

pub struct Store {
    catalog: Option<Catalog>,
    icons: Option<Icons>,
    pics: Pics,
    installed: HashMap<String, String>,
    state: State,
    progress: Option<Progress>,
    /// Asked for, and the engine hasn't shown progress yet.
    starting: Option<Work>,
    start_timer: Option<Timer>,
    /// The engine was running at the last look: its end is news.
    was_busy: bool,
    /// The user asked for this refresh, so its end gets a toast.
    refresh_asked: bool,
    /// Installs waiting for the engine, as `package=version`.
    queue: VecDeque<String>,
    helper: Helper,
    filter: Filter,
    search: Search,
    list: List,
    layout: Layout,
    browse_grid: Grid,
    browse_list: List,
    view: View,
    about: Option<About>,
    /// The open app's extras that aren't installed: what installing it adds.
    adds: Vec<String>,
    scroll: Scroll,
    shot: Option<Rc<Image>>,
    fetch: Option<Fetch>,
    clean: Option<Clean>,
    /// The screenshot couldn't be fetched or didn't match.
    shot_failed: bool,
    /// The app the install dialog asks about, as `package=version`: the
    /// version the dialog described is the one that installs.
    asking: Option<String>,
    mirror: Mirror,
    phase: u8,
    segment: Option<Timer>,
    /// The screen stays on while a run shows its progress.
    awake: bool,
    /// The package this app asked to remove, until the run ends: the
    /// engine's progress says install for both.
    removing: Option<String>,
    watch: Option<(gh0st_core::status::Watch, Watch)>,
    zone: String,
}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Store {
            catalog: None,
            icons: None,
            pics: Pics::default(),
            installed: HashMap::new(),
            state: State::default(),
            progress: None,
            starting: None,
            start_timer: None,
            was_busy: false,
            refresh_asked: false,
            queue: VecDeque::new(),
            helper: Helper::new(ASK_PASSWORD),
            filter: Filter::All,
            search: Search::new(),
            list: List::new(),
            layout: Layout::Grid,
            browse_grid: Grid::new(),
            browse_list: List::new(),
            view: View::Browse,
            about: None,
            adds: Vec::new(),
            scroll: Scroll::new(),
            shot: None,
            fetch: None,
            clean: None,
            shot_failed: false,
            asking: None,
            mirror: Mirror::Auto,
            phase: 0,
            segment: None,
            awake: false,
            removing: None,
            watch: None,
            zone: String::new(),
        }
    }

    /// Reads the catalog, the icons and what is installed. Done at start
    /// and when the engine has finished, never per frame.
    fn load_catalog(&mut self) {
        self.catalog = Catalog::load().map(readable);
        self.icons = Icons::load();
        self.pics.clear();
        self.installed = self
            .catalog
            .as_ref()
            .map(store::installed_apps)
            .unwrap_or_default();
        if !self.filters().contains(&self.filter) {
            self.filter = Filter::All;
        }
        if let View::Detail(p) = &self.view {
            self.adds = self.adds_for(&p.clone());
        }
    }

    /// What installing or updating `package` would add besides it: its
    /// extras dpkg lacks. None for an installed app with no update, since
    /// nothing would run.
    fn adds_for(&self, package: &str) -> Vec<String> {
        let Some(a) = self.app(package) else {
            return Vec::new();
        };
        if a.extras.is_empty() || (self.installed.contains_key(package) && !self.has_update(a)) {
            return Vec::new();
        }
        let status = std::fs::read_to_string(store::dpkg_status()).unwrap_or_default();
        let have = store::installed(&status, |n| a.extras.iter().any(|e| e == n));
        a.extras
            .iter()
            .filter(|e| !have.contains_key(*e))
            .cloned()
            .collect()
    }

    /// Reads the engine's state and progress. Returns true when a run just
    /// ended.
    fn load_engine(&mut self) -> bool {
        let before = self.state.clone();
        self.state = State::load();
        let running = update::engine_running();
        self.progress = if running {
            std::fs::read_to_string(store::progress_path())
                .ok()
                .and_then(|t| Progress::parse(&t))
        } else {
            None
        };
        // Every run writes the state once, as it ends: that, not the
        // progress file going, says it is over.
        let changed = self.state != before;
        if self.progress.is_some() || changed {
            self.starting = None;
        }
        let between = self.was_busy && running && !changed;
        let busy = self.engine_busy() || between;
        let ended = self.was_busy && !busy;
        self.was_busy = busy;
        ended
    }

    fn engine_busy(&self) -> bool {
        self.progress.is_some() || self.starting.is_some()
    }

    fn work(&self) -> Option<Work> {
        if let Some(p) = &self.progress {
            return Some(if p.install {
                Work::Apps(vec![p.item.clone()])
            } else {
                Work::Refresh
            });
        }
        self.starting.clone()
    }

    /// The package the engine works on now.
    fn busy_with(&self, package: &str) -> bool {
        matches!(self.work(), Some(Work::Apps(p)) if p.iter().any(|x| x == package))
    }

    fn app(&self, package: &str) -> Option<&App> {
        self.catalog.as_ref()?.find(package)
    }

    fn updates(&self) -> Vec<&App> {
        self.catalog
            .as_ref()
            .map(|c| store::updates(c, &self.installed))
            .unwrap_or_default()
    }

    fn has_update(&self, a: &App) -> bool {
        self.installed
            .get(&a.package)
            .is_some_and(|v| gh0st_core::debver::newer(&a.version, v))
    }

    fn filters(&self) -> Vec<Filter> {
        let mut f = vec![Filter::All];
        if !self.updates().is_empty() {
            f.push(Filter::Updates);
        }
        if !self.installed.is_empty() {
            f.push(Filter::Installed);
        }
        if let Some(c) = &self.catalog {
            f.extend(
                c.categories()
                    .into_iter()
                    .map(|s| Filter::Category(s.to_string())),
            );
        }
        f
    }

    /// Home's entries: Updates and Installed when they have apps, All
    /// apps, then each category.
    fn browse(&self) -> Vec<Filter> {
        let mut f = Vec::new();
        if !self.updates().is_empty() {
            f.push(Filter::Updates);
        }
        if !self.installed.is_empty() {
            f.push(Filter::Installed);
        }
        f.push(Filter::All);
        if let Some(c) = &self.catalog {
            f.extend(
                c.categories()
                    .into_iter()
                    .map(|s| Filter::Category(s.to_string())),
            );
        }
        f
    }

    fn browse_selected(&self) -> usize {
        match self.layout {
            Layout::Grid => self.browse_grid.selected(),
            Layout::List => self.browse_list.selected(),
        }
    }

    fn open_filter(&mut self, f: Filter) {
        self.filter = f;
        self.list.select(0);
        self.view = View::List;
    }

    /// Find starts from any list: it searches every app.
    fn find(&mut self) {
        self.filter = Filter::All;
        self.view = View::List;
        self.search.start();
        self.list.select(0);
    }

    /// The catalog indexes the list shows: a search looks through every
    /// app, otherwise the filter picks.
    fn shown(&self) -> Vec<usize> {
        let Some(c) = &self.catalog else {
            return Vec::new();
        };
        if self.search.active() {
            return self.search.rank(c.apps.len(), |i| {
                let a = &c.apps[i];
                self.search
                    .score(&[&a.title, &a.share, &a.summary, &a.author])
            });
        }
        (0..c.apps.len())
            .filter(|&i| {
                let a = &c.apps[i];
                match &self.filter {
                    Filter::All => true,
                    Filter::Updates => self.has_update(a),
                    Filter::Installed => self.installed.contains_key(&a.package),
                    Filter::Category(name) => a.category == *name,
                }
            })
            .collect()
    }

    fn selected(&self) -> Option<String> {
        let c = self.catalog.as_ref()?;
        let shown = self.shown();
        let i = *shown.get(self.list.selected().min(shown.len().checked_sub(1)?))?;
        Some(c.apps[i].package.clone())
    }

    fn step_filter(&mut self, d: i32) {
        let f = self.filters();
        let i = f.iter().position(|x| *x == self.filter).unwrap_or(0);
        self.filter = f[cycle(i, d, f.len())].clone();
        self.list.select(0);
    }

    // Actions.

    fn refresh(&mut self, cx: &mut Cx) {
        if self.helper.busy(REFRESH) || self.engine_busy() {
            return;
        }
        self.refresh_asked = true;
        self.starting_for(Work::Refresh);
        self.helper.run(cx, REFRESH, &["store-refresh"]);
    }

    /// Enter on an app: install, update or open it.
    fn main_action(&mut self, cx: &mut Cx, package: &str) {
        let Some(a) = self.app(package).cloned() else {
            return;
        };
        if self.busy_with(package) || self.queued(&a.package) {
            return;
        }
        let want = format!("{}={}", a.package, a.version);
        if self.installed.contains_key(package) && !self.has_update(&a) {
            let name = if a.desktop.is_empty() {
                &a.package
            } else {
                &a.desktop
            };
            cx.open(name);
            return;
        }
        if a.flags.risky() && !self.installed.contains_key(package) {
            let line = words::risks(a.flags);
            self.asking = Some(want);
            cx.ask(
                ASK_INSTALL,
                Dialog::confirm(&format!("Install {}?", a.title), &line).yes("install"),
            );
            return;
        }
        self.install(cx, &want);
    }

    fn queued(&self, package: &str) -> bool {
        self.queue.iter().any(|q| store::split_seen(q).0 == package)
    }

    /// Installs `package=version` now, or after what the engine is doing.
    /// The engine won't install another version than the one shown.
    fn install(&mut self, cx: &mut Cx, want: &str) {
        let package = store::split_seen(want).0;
        if self.engine_busy() || self.helper.busy(INSTALL) || self.helper.busy(REMOVE) {
            if !self.queued(package) {
                self.queue.push_back(want.to_string());
            }
            return;
        }
        let m = self.mirror.word();
        self.starting_for(Work::Apps(vec![package.to_string()]));
        self.helper
            .run(cx, INSTALL, &["store-install", "--mirror", m, want]);
    }

    fn remove(&mut self, cx: &mut Cx, package: &str) {
        if self.engine_busy() {
            cx.toast("Wait for the install to finish");
            return;
        }
        self.starting_for(Work::Apps(vec![package.to_string()]));
        self.removing = Some(package.to_string());
        self.helper.run(cx, REMOVE, &["store-remove", package]);
    }

    fn update_all(&mut self, cx: &mut Cx) {
        let all: Vec<String> = self.updates().iter().map(|a| a.package.clone()).collect();
        if all.is_empty() || self.engine_busy() {
            return;
        }
        let m = self.mirror.word();
        self.starting_for(Work::Apps(all));
        self.helper
            .run(cx, UPGRADE, &["store-upgrade", "--mirror", m]);
    }

    /// Shows the work as started until the helper says otherwise.
    fn starting_for(&mut self, w: Work) {
        self.starting = Some(w);
        self.was_busy = true;
    }

    fn finished(&mut self, cx: &mut Cx, f: Finished) {
        if let Err(why) = f.result {
            self.starting = None;
            self.was_busy = false;
            self.removing = None;
            if why != "Cancelled" {
                cx.toast_tone(Tone::Critical, &why);
            }
            self.next(cx);
        } else if self.progress.is_none() && self.starting.is_some() {
            // The unit started. The clock runs from here, not from before
            // the password.
            if let Some(t) = self.start_timer.take() {
                cx.cancel(t);
            }
            self.start_timer = Some(cx.after(START_MS));
        }
        self.sync_segment(cx);
    }

    /// The engine finished: what happened, then the next in the queue.
    fn ended(&mut self, cx: &mut Cx) {
        self.removing = None;
        self.load_catalog();
        let st = &self.state;
        let title = |p: &str| {
            self.app(p)
                .map_or_else(|| p.to_string(), |a| a.title.clone())
        };
        let first = st.packages.first().map(String::as_str).unwrap_or("");
        match (st.outcome, st.op.as_str()) {
            (Outcome::Ok, "refresh") if self.refresh_asked => {
                let n = self.updates().len();
                let msg = if n > 0 {
                    words::updates(n)
                } else {
                    "App list is up to date".into()
                };
                cx.toast_tone(Tone::Ok, &msg);
            }
            (Outcome::Ok, "refresh") => {}
            (Outcome::Ok, "remove") => {
                cx.toast_tone(Tone::Ok, &format!("{} removed", title(first)))
            }
            (Outcome::Ok, _) if st.packages.len() > 1 => cx.toast_tone(
                Tone::Ok,
                &format!("{} installed", words::apps(st.packages.len())),
            ),
            (Outcome::Ok, _) => cx.toast_tone(Tone::Ok, &format!("{} installed", title(first))),
            (o, op) => {
                let failed = if st.failed.is_empty() {
                    first
                } else {
                    &st.failed
                };
                let msg = words::outcome(o, op, &title(failed));
                cx.toast_tone(Tone::Critical, &msg);
            }
        }
        if st.op == "refresh" {
            self.refresh_asked = false;
        }
        self.next(cx);
    }

    fn next(&mut self, cx: &mut Cx) {
        if self.engine_busy() {
            return;
        }
        if let Some(p) = self.queue.pop_front() {
            self.install(cx, &p);
        }
    }

    // Screenshots.

    fn open_shots(&mut self, cx: &mut Cx, package: String) {
        if self.about.as_ref().is_none_or(|a| a.shots.is_empty()) {
            return;
        }
        self.view = View::Shots(package, 0);
        self.load_shot(cx);
    }

    fn load_shot(&mut self, cx: &mut Cx) {
        let View::Shots(_, i) = self.view else {
            return;
        };
        self.shot = None;
        self.shot_failed = false;
        let Some((sha, url)) = self.about.as_ref().and_then(|a| a.shots.get(i)).cloned() else {
            return;
        };
        if let Some(p) = shots::cached(&sha) {
            match Image::load_png(&p, 320) {
                Ok(img) => self.shot = Some(Rc::new(img)),
                Err(_) => self.shot_failed = true,
            }
            return;
        }
        if self.fetch.is_some() {
            return;
        }
        if !url.starts_with("https://") {
            self.shot_failed = true;
            return;
        }
        let _ = std::fs::create_dir_all(shots::dir());
        let tmp = shots::dir().join(format!("{sha}.part"));
        let argv = shots::fetch(&url, &tmp);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let job = cx.spawn(&argv);
        self.fetch = Some(Fetch {
            job,
            tmp,
            sha256: sha,
            hashing: None,
        });
    }

    /// The download, then its hash, each a child process.
    fn fetch_event(&mut self, cx: &mut Cx, event: JobEvent) {
        let Some(f) = self.fetch.as_mut() else {
            return;
        };
        match (event, &mut f.hashing) {
            (JobEvent::Line(l), Some(h)) if h.is_empty() => {
                *h = l.split_whitespace().next().unwrap_or("").to_string();
            }
            (JobEvent::Done(Some(0)), None) => {
                let tmp = f.tmp.to_string_lossy().into_owned();
                f.job = cx.spawn(&["sha256sum", "--", &tmp]);
                f.hashing = Some(String::new());
            }
            (JobEvent::Done(_), hashing) => {
                let ok = hashing.as_deref() == Some(f.sha256.as_str());
                self.fetched(cx, ok);
            }
            _ => {}
        }
    }

    fn fetched(&mut self, cx: &mut Cx, ok: bool) {
        let Some(f) = self.fetch.take() else {
            return;
        };
        let ok = ok && shots::keep(&f.tmp, &f.sha256);
        if !ok {
            let _ = std::fs::remove_file(&f.tmp);
        }
        let want = match (&self.view, &self.about) {
            (View::Shots(_, i), Some(a)) => a.shots.get(*i).map(|s| s.0.clone()),
            _ => None,
        };
        if want.as_deref() == Some(&f.sha256) {
            if ok {
                self.load_shot(cx);
            } else {
                self.shot_failed = true;
            }
        } else {
            // The user flipped on while it came: fetch the one shown now.
            self.load_shot(cx);
        }
        cx.redraw();
    }

    /// The moving segment runs only while a bar without a percent shows.
    fn sync_segment(&mut self, cx: &mut Cx) {
        // An install is a minute the user watches without typing.
        let busy = self.engine_busy();
        if busy != self.awake {
            self.awake = busy;
            cx.keep_awake(busy);
        }
        let bar = matches!(&self.view, View::Detail(p) if self.busy_with(p))
            && self.progress.as_ref().is_none_or(|p| p.percent.is_none());
        let want = bar && cx.in_front();
        match (want, self.segment) {
            (true, None) => self.segment = Some(cx.every(SEGMENT_MS)),
            (false, Some(t)) => {
                cx.cancel(t);
                self.segment = None;
            }
            _ => {}
        }
    }

    /// M5's Store kept every deb it downloaded. `du` then `rm` as child
    /// processes, so the first frame doesn't wait on the SD card.
    fn clear_old_store(&mut self, cx: &mut Cx) {
        let old = shots::old_store_cache();
        if !old.is_dir() {
            return;
        }
        let p = old.to_string_lossy().into_owned();
        self.clean = Some(Clean {
            job: cx.spawn(&["du", "-sk", "--", &p]),
            kb: 0,
            removing: false,
        });
    }

    fn clean_event(&mut self, cx: &mut Cx, event: JobEvent) {
        let Some(c) = self.clean.as_mut() else {
            return;
        };
        match event {
            JobEvent::Line(l) if !c.removing => {
                c.kb = l
                    .split_whitespace()
                    .next()
                    .and_then(|k| k.parse().ok())
                    .unwrap_or(0);
            }
            JobEvent::Done(_) if !c.removing => {
                let p = shots::old_store_cache().to_string_lossy().into_owned();
                c.job = cx.spawn(&["rm", "-rf", "--", &p]);
                c.removing = true;
            }
            JobEvent::Done(code) => {
                let bytes = c.kb * 1024;
                self.clean = None;
                if code == Some(0) && bytes >= 1_000_000 {
                    cx.toast(&format!(
                        "Freed {} left by the old Store",
                        update::size_text(bytes)
                    ));
                }
            }
            JobEvent::Line(_) => {}
        }
    }

    // Drawing.

    fn tag_of(&self, a: &App) -> Option<(String, Tone)> {
        if self.busy_with(&a.package) {
            let pct = self
                .progress
                .as_ref()
                .filter(|p| p.item == a.package)
                .and_then(|p| p.percent);
            return Some((
                pct.map_or("\u{2026}".into(), |p| format!("{p}%")),
                Tone::Info,
            ));
        }
        if self.queued(&a.package) {
            return Some(("Queued".into(), Tone::Info));
        }
        if self.has_update(a) {
            return Some(("Update".into(), Tone::Info));
        }
        self.installed
            .contains_key(&a.package)
            .then(|| ("Installed".into(), Tone::Ok))
    }

    fn draw_list(&self, ui: &mut Ui) {
        let Some(c) = &self.catalog else {
            return self.draw_empty(ui);
        };
        let shown = self.shown();
        if shown.is_empty() {
            let text = format!("No app matches \u{201c}{}\u{201d}", self.search.query());
            return ui.notice("search", None, &text, "Try fewer letters", &[]);
        }
        let t = *ui.theme();
        let sel = self.list.selected().min(shown.len() - 1);
        // Icons only for rows that can show: three each side of the selection.
        let near = |k: usize| k + 3 >= sel && k <= sel + 3;
        let pics: Vec<Option<Rc<Image>>> = shown
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                near(k)
                    .then(|| {
                        self.pics
                            .get(self.icons.as_ref(), &c.apps[i].package, ROW_ICON)
                    })
                    .flatten()
            })
            .collect();
        let tags: Vec<Option<(String, Tone)>> =
            shown.iter().map(|&i| self.tag_of(&c.apps[i])).collect();
        let items: Vec<AppItem> = shown
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let a = &c.apps[i];
                let visual = match &pics[k] {
                    Some(img) => Visual::Image(img),
                    None => Visual::Letter {
                        ch: a.title.chars().next().unwrap_or('?'),
                        tint: t.c.tints[a.package.len() % t.c.tints.len()],
                    },
                };
                AppItem {
                    label: &a.title,
                    note: &a.summary,
                    visual,
                    pinned: false,
                    count: 0,
                    tag: tags[k].as_ref().map(|(w, tone)| (w.as_str(), *tone)),
                }
            })
            .collect();
        ui.apps(&self.list, &items);
    }

    fn draw_browse(&self, ui: &mut Ui) {
        let Some(c) = &self.catalog else {
            return self.draw_empty(ui);
        };
        let t = *ui.theme();
        let n_updates = self.updates().len();
        let count = |f: &Filter| {
            c.apps
                .iter()
                .filter(|a| match f {
                    Filter::All => true,
                    Filter::Updates => self.has_update(a),
                    Filter::Installed => self.installed.contains_key(&a.package),
                    Filter::Category(name) => a.category == *name,
                })
                .count()
        };
        let entries = self.browse();
        let notes: Vec<String> = entries
            .iter()
            .map(|f| match f {
                Filter::Updates => words::updates(n_updates),
                f => words::apps(count(f)),
            })
            .collect();
        let grid = self.layout == Layout::Grid;
        let items: Vec<AppItem> = entries
            .iter()
            .enumerate()
            .map(|(k, f)| {
                let (visual, label) = match f {
                    Filter::Updates => (Visual::System { icon: "download" }, "Updates"),
                    Filter::Installed => (
                        Visual::System {
                            icon: "circle-check",
                        },
                        "Installed",
                    ),
                    Filter::All => (
                        Visual::System {
                            icon: "layout-grid",
                        },
                        "All apps",
                    ),
                    Filter::Category(name) => {
                        let (icon, short) = words::category(name);
                        let tint = t.c.tints[k % t.c.tints.len()];
                        (
                            Visual::Badge { icon, tint },
                            if grid { short } else { name.as_str() },
                        )
                    }
                };
                AppItem {
                    label,
                    note: &notes[k],
                    visual,
                    pinned: false,
                    count: if *f == Filter::Updates {
                        n_updates as u32
                    } else {
                        0
                    },
                    tag: None,
                }
            })
            .collect();
        match self.layout {
            Layout::Grid => ui.tiles(&self.browse_grid, &items),
            Layout::List => ui.apps(&self.browse_list, &items),
        }
    }

    fn draw_empty(&self, ui: &mut Ui) {
        if matches!(self.work(), Some(Work::Refresh)) {
            return ui.notice("download", None, "Getting the app list\u{2026}", "", &[]);
        }
        if self.state.op == "refresh" && self.state.outcome != Outcome::Ok {
            let why = words::outcome(self.state.outcome, "refresh", "");
            return ui.notice(
                "wifi-off",
                Some(Tone::Critical),
                "No app list yet",
                &why,
                &[("Enter", "try again")],
            );
        }
        ui.notice(
            APP_ICON,
            None,
            "No app list yet",
            "Gh0st's server lists the apps, signed.\nNo account, no device ID.",
            &[("Enter", "get the list")],
        );
    }

    fn draw_detail(&self, ui: &mut Ui, package: &str) {
        let Some(a) = self.app(package) else {
            return ui.notice("circle-x", None, "Not in the list any more", "", &[]);
        };
        let t = *ui.theme();
        let area = ui.area();
        let x = area.x + t.m.margin + 3;
        let w = area.w - 2 * (t.m.margin + 3);
        let top = area.y + 5;
        let status = if self.has_update(a) {
            Some(("Update", Tone::Info))
        } else if self.installed.contains_key(package) {
            Some(("Installed", Tone::Ok))
        } else {
            None
        };
        let pic = self.pics.get(self.icons.as_ref(), package, DETAIL_ICON);
        let c = ui.canvas();
        let b = t.m.badge;
        let br = Rect::new(x, top, b, b);
        let visual = match &pic {
            Some(img) => Visual::Image(img),
            None => Visual::Letter {
                ch: a.title.chars().next().unwrap_or('?'),
                tint: t.c.tints[a.package.len() % t.c.tints.len()],
            },
        };
        widgets::badge(c, &t, br, &visual, false);
        let tx = br.right() + 9;
        let mut right = x + w;
        if let Some((word, tone)) = status {
            let row = Rect::new(x, top, w, t.f.body_bold.cap() + 6);
            right = widgets::tag(c, &t, x + w, row, word, tone) - 8;
        }
        t.f.body_bold.draw_elided(
            c,
            tx,
            top + t.f.body_bold.cap() + 2,
            &a.title,
            right - tx,
            t.c.fg,
        );
        let line = words::line(&a.author, &a.version, a.size);
        t.f.small
            .draw_elided(c, tx, top + b - 3, &line, x + w - tx, t.c.muted);
        let mut y = br.bottom() + 6;
        if self.busy_with(package) {
            let (label, pct) = self.busy_label(a);
            widgets::progress(c, &t, Rect::new(x, y, w, 24), &label, pct, self.phase);
            y += 28;
        } else {
            // Pills in one row: the risks first. What doesn't fit is in Help.
            let row = Rect::new(x, y, w, 16);
            let mut px = x;
            let mut any = false;
            for (word, tone) in words::pills(a.flags) {
                let pw = widgets::tag_w(&t, word);
                if px + pw > x + w {
                    break;
                }
                widgets::tag(c, &t, px + pw, row, word, tone);
                px += pw + 5;
                any = true;
            }
            if any {
                y += 20;
            }
        }
        ui.space(y - top - 5);
        let text = match &self.about {
            Some(ab) if !ab.description.is_empty() => ab.description.as_str(),
            _ => a.summary.as_str(),
        };
        if self.adds.is_empty() {
            ui.scroll_text(&self.scroll, text);
        } else {
            let text = format!("{text}\n\n{}", words::adds(&self.adds));
            ui.scroll_text(&self.scroll, &text);
        }
    }

    fn busy_label(&self, a: &App) -> (String, Option<u8>) {
        let Some(p) = self.progress.as_ref().filter(|p| p.item == a.package) else {
            return ("Starting\u{2026}".into(), None);
        };
        let removing = self.removing.as_deref() == Some(a.package.as_str());
        let label = match p.phase {
            Phase::Lists => "Checking what it needs".to_string(),
            Phase::Download if p.total > 0 => format!(
                "Downloading \u{b7} {} of {}",
                update::size_text(p.done),
                update::size_text(p.total)
            ),
            Phase::Download => "Downloading".into(),
            Phase::Install if removing => "Removing".into(),
            Phase::Install => "Installing".into(),
            Phase::Clean => "Cleaning up".into(),
        };
        (label, p.percent)
    }

    fn draw_shot(&self, ui: &mut Ui, i: usize) {
        let n = self.about.as_ref().map_or(0, |a| a.shots.len());
        match &self.shot {
            Some(img) => {
                let t = *ui.theme();
                let a = ui.area();
                let c = ui.canvas();
                c.image(a.x + (a.w - img.w) / 2, a.y + (a.h - img.h) / 2, img);
                widgets::page_dots(c, &t, a.right() - 6, a.bottom() - 6, i, n);
            }
            None if self.shot_failed => ui.notice(
                "images",
                Some(Tone::Warning),
                "Couldn't load the screenshot",
                "Check the network, then try again",
                &[("Enter", "try again")],
            ),
            None => ui.message("images", "Loading screenshot\u{2026}"),
        }
    }
}

impl Page for Store {
    fn title(&self) -> &str {
        "Store"
    }

    fn icon(&self) -> &str {
        APP_ICON
    }

    fn context(&self) -> Option<&str> {
        match &self.view {
            View::Browse => None,
            View::List if !self.search.active() => Some(self.filter.name()),
            View::List => None,
            View::Detail(p) | View::Shots(p, _) => self.app(p).map(|a| a.title.as_str()),
        }
    }

    fn about(&self) -> &str {
        words::ABOUT
    }

    fn start(&mut self, cx: &mut Cx) {
        self.mirror = cx
            .settings()
            .get_str(key::STORE_MIRROR)
            .and_then(Mirror::parse)
            .unwrap_or_default();
        self.zone = store::local_zone();
        self.layout = match cx.settings().get_str(key::STORE_LAYOUT) {
            Some("list") => Layout::List,
            _ => Layout::Grid,
        };
        self.load_catalog();
        self.load_engine();
        if let Some(w) = watch_dirs() {
            let id = cx.watch(w.fd());
            self.watch = Some((w, id));
        }
        self.clear_old_store(cx);
        self.sync_segment(cx);
    }

    fn stop(&mut self, cx: &mut Cx) {
        if let Some((_, id)) = self.watch.take() {
            cx.unwatch(id);
        }
    }

    fn resume(&mut self, cx: &mut Cx) {
        self.load_catalog();
        self.load_engine();
        self.sync_segment(cx);
    }

    fn front(&mut self, front: bool, cx: &mut Cx) {
        if front {
            // Apps may have come or gone from the terminal meanwhile.
            self.load_catalog();
        }
        self.sync_segment(cx);
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        if let Some((w, _)) = &self.watch {
            w.drain();
        }
        if self.load_engine() {
            self.ended(cx);
        }
        cx.redraw();
        self.sync_segment(cx);
    }

    fn timer(&mut self, timer: Timer, cx: &mut Cx) {
        if Some(timer) == self.segment {
            self.phase = self.phase.wrapping_add(8);
            return;
        }
        if Some(timer) == self.start_timer {
            self.start_timer = None;
            if self.progress.is_none() && self.starting.take().is_some() {
                // No progress in time. The run may have ended unseen, as
                // off Linux with no inotify; else nothing ran.
                let before = self.state.clone();
                self.load_engine();
                self.was_busy = false;
                if self.state != before {
                    self.ended(cx);
                } else {
                    self.next(cx);
                }
            }
            self.sync_segment(cx);
        }
    }

    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        if self.fetch.as_ref().is_some_and(|f| f.job == job) {
            self.fetch_event(cx, event);
            return;
        }
        if self.clean.as_ref().is_some_and(|c| c.job == job) {
            self.clean_event(cx, event);
            return;
        }
        if let Some(f) = self.helper.job(cx, job, event) {
            self.finished(cx, f);
        }
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if id == ASK_INSTALL {
            if let (Some(p), true) = (self.asking.take(), answer != Answer::No) {
                self.install(cx, &p);
            }
            return;
        }
        if let Some(f) = self.helper.answer(cx, id, answer) {
            self.finished(cx, f);
        }
    }

    fn typing(&self) -> bool {
        self.view == View::List && self.search.active()
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        match std::mem::replace(&mut self.view, View::List) {
            View::Browse => {
                self.view = View::Browse;
                true
            }
            View::Shots(p, _) => {
                self.view = View::Detail(p);
                self.shot = None;
                false
            }
            View::Detail(_) => {
                self.about = None;
                self.sync_segment(cx);
                false
            }
            View::List => {
                if !self.search.cancel() {
                    self.view = View::Browse;
                }
                false
            }
        }
    }

    fn bars(&self) -> Bars {
        let update_all = Chip::new("download", "Update")
            .main()
            .busy(self.engine_busy());
        match &self.view {
            View::Browse if self.catalog.is_none() => Bars::top(),
            View::Browse if !self.updates().is_empty() => Bars::top()
                .chip(5, Chip::new("search", "Find"))
                .chip(7, update_all),
            View::Browse => Bars::top().chip(6, Chip::new("search", "Find")),
            View::List if self.filter == Filter::Updates && !self.search.active() => Bars::top()
                .chip(5, Chip::new("search", "Find"))
                .chip(7, update_all),
            View::List => Bars::top()
                .chip(6, Chip::new("search", "Find"))
                .search(&self.search),
            View::Detail(p) => self
                .detail_chips(p)
                .into_iter()
                .fold(Bars::top(), |b, (n, chip, _)| b.chip(n, chip)),
            View::Shots(..) => Bars::none(),
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.view.clone() {
            View::Browse => self.browse_key(key, cx),
            View::List => self.list_key(key, cx),
            View::Detail(p) => {
                let chip = match key {
                    Key::Soft(n) => self.detail_chips(&p).into_iter().find(|c| c.0 == n),
                    _ => None,
                };
                match (key, chip.map(|c| c.2)) {
                    (Key::Enter, _) | (_, Some(DetailDo::Main)) => self.main_action(cx, &p),
                    (_, Some(DetailDo::Shots)) => self.open_shots(cx, p),
                    (_, Some(DetailDo::Delete)) => self.remove(cx, &p),
                    (k, _) => {
                        self.scroll.key(k);
                    }
                }
            }
            View::Shots(p, i) => {
                let n = self.about.as_ref().map_or(0, |a| a.shots.len());
                let to = match key {
                    Key::Left | Key::Up => cycle(i, -1, n),
                    Key::Right | Key::Down => cycle(i, 1, n),
                    Key::Enter if self.shot_failed => i,
                    _ => return,
                };
                self.view = View::Shots(p, to);
                self.load_shot(cx);
            }
        }
        self.sync_segment(cx);
    }

    fn commands(&self, c: &mut Commands) {
        let list = matches!(self.view, View::List | View::Browse);
        let n = self.updates().len();
        c.group("Store");
        c.add(
            Command::action(CMD_FIND, "Find an app")
                .keys("s")
                .enabled(list && self.catalog.is_some())
                .mode("App list", list && !self.search.active()),
        );
        let all = if n > 0 {
            format!("Update all, {}", words::apps(n))
        } else {
            "Update all".to_string()
        };
        c.add(
            Command::action(CMD_UPDATE_ALL, all)
                .keys("u")
                .enabled(n > 0 && !self.engine_busy()),
        );
        c.add(
            Command::action(CMD_REFRESH, "Refresh the list")
                .keys("r")
                .enabled(!self.engine_busy()),
        );
        let m = match self.mirror {
            Mirror::Auto => "Auto",
            Mirror::GitHub => "GitHub",
            Mirror::China => "China",
        };
        c.add(Command::choice(CMD_MIRROR, "Download from", m));
        let l = match self.layout {
            Layout::Grid => "Grid",
            Layout::List => "List",
        };
        c.add(Command::choice(CMD_LAYOUT, "Categories as", l));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        match id {
            CMD_FIND => self.find(),
            CMD_LAYOUT => {
                self.layout = match self.layout {
                    Layout::Grid => Layout::List,
                    Layout::List => Layout::Grid,
                };
                let i = self.browse_selected();
                self.browse_grid.select(i);
                self.browse_list.select(i);
                let word = if self.layout == Layout::Grid {
                    "grid"
                } else {
                    "list"
                };
                cx.settings().set_str(key::STORE_LAYOUT, word);
            }
            CMD_UPDATE_ALL => self.update_all(cx),
            CMD_REFRESH => self.refresh(cx),
            CMD_MIRROR => {
                let i = Mirror::ALL
                    .iter()
                    .position(|m| *m == self.mirror)
                    .unwrap_or(0);
                let d = if step == Step::Prev { -1 } else { 1 };
                self.mirror = Mirror::ALL[cycle(i, d, Mirror::ALL.len())];
                cx.settings().set_str(key::STORE_MIRROR, self.mirror.word());
            }
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        match &self.view {
            View::Browse => self.draw_browse(ui),
            View::List => self.draw_list(ui),
            View::Detail(p) => self.draw_detail(ui, p),
            View::Shots(_, i) => self.draw_shot(ui, *i),
        }
    }
}

/// What a detail page's chip does.
#[derive(Clone, Copy)]
enum DetailDo {
    Shots,
    /// Install, Update or Open: what Enter does too.
    Main,
    Delete,
}

impl Store {
    /// Shots, the main action, Delete: one chip on 6, two on 5 and 7, three
    /// on 5, 6 and 7. The key runs what its chip shows.
    fn detail_chips(&self, package: &str) -> Vec<(u8, Chip, DetailDo)> {
        let mut c = Vec::new();
        if self.about.as_ref().is_some_and(|a| !a.shots.is_empty()) {
            c.push((Chip::new("images", "Shots"), DetailDo::Shots));
        }
        let installed = self.installed.contains_key(package);
        let update = self.app(package).is_some_and(|a| self.has_update(a));
        let busy = self.busy_with(package);
        let removing = self.removing.as_deref() == Some(package);
        // Busy: the percent when the engine says one. A busy chip adds "…",
        // which Install and Update have no room for.
        let pct = self
            .progress
            .as_ref()
            .filter(|p| p.item == package)
            .and_then(|p| p.percent)
            .map(|p| format!("{p}%"));
        let main = if busy {
            let word = match (&pct, removing) {
                (_, true) => "Wait".to_string(),
                (Some(p), _) => p.clone(),
                (None, _) => "Wait".to_string(),
            };
            Chip::new("download", &word).busy(true)
        } else if self.queued(package) {
            Chip::new("timer", "Queued").disabled(true)
        } else if update {
            Chip::new("download", "Update").main()
        } else if installed {
            Chip::new("square", "Open").main()
        } else {
            Chip::new("download", "Install").main()
        };
        c.push((main, DetailDo::Main));
        if installed && !busy {
            c.push((Chip::new("trash-2", "Delete").hold(), DetailDo::Delete));
        }
        let keys: &[u8] = match c.len() {
            1 => &[6],
            2 => &[5, 7],
            _ => &[5, 6, 7],
        };
        c.into_iter()
            .zip(keys)
            .map(|((chip, d), &n)| (n, chip, d))
            .collect()
    }

    fn browse_key(&mut self, key: Key, cx: &mut Cx) {
        if self.catalog.is_none() {
            if key == Key::Enter {
                self.refresh(cx);
            }
            return;
        }
        let updates = !self.updates().is_empty();
        match key {
            Key::Soft(5) if updates => return self.find(),
            Key::Soft(6) if !updates => return self.find(),
            Key::Soft(7) if updates => return self.update_all(cx),
            _ => {}
        }
        let entries = self.browse();
        let open = match self.layout {
            Layout::Grid => self.browse_grid.key(key),
            Layout::List => match self.browse_list.key(key) {
                Some(Action::Open(i)) => Some(i),
                _ => None,
            },
        };
        if let Some(f) = open.and_then(|i| entries.get(i).cloned()) {
            self.open_filter(f);
        }
    }

    fn list_key(&mut self, key: Key, cx: &mut Cx) {
        if self.catalog.is_none() {
            if key == Key::Enter {
                self.refresh(cx);
            }
            return;
        }
        if self.search.active() {
            match self.search.key(key) {
                Some(Edit::Changed) => return self.list.select(0),
                Some(Edit::Submit) => return self.open_selected(cx),
                None => {}
            }
        }
        let updates_page = self.filter == Filter::Updates && !self.search.active();
        let chip_find = if updates_page { 5 } else { 6 };
        match key {
            Key::Soft(n) if n == chip_find => self.find(),
            Key::Soft(7) if updates_page => self.update_all(cx),
            Key::Left if !self.search.active() => self.step_filter(-1),
            Key::Right if !self.search.active() => self.step_filter(1),
            Key::Enter => self.open_selected(cx),
            k => {
                self.list.key(k);
            }
        }
    }

    fn open_selected(&mut self, cx: &mut Cx) {
        let Some(p) = self.selected() else {
            return;
        };
        self.search.cancel();
        self.about = About::load(&p).map(|mut a| {
            if !text_font().drawable(&a.description) {
                a.description = words::NO_ENGLISH.into();
            }
            a
        });
        self.adds = self.adds_for(&p);
        self.scroll.reset();
        self.view = View::Detail(p);
        self.sync_segment(cx);
    }
}

/// The face the Store's text draws in.
fn text_font() -> gh0st_app::gfx::font::Font {
    gh0st_app::gfx::theme::Theme::default().f.small
}

/// Text the device can't draw would show as boxes: a title falls back to
/// the package name, a summary to the category.
fn readable(mut c: Catalog) -> Catalog {
    let f = text_font();
    for a in &mut c.apps {
        if !f.drawable(&a.title) {
            a.title = a.package.clone();
        }
        if !f.drawable(&a.summary) {
            a.summary = a.category.clone();
        }
    }
    c
}

#[cfg(target_os = "linux")]
fn watch_dirs() -> Option<gh0st_core::status::Watch> {
    let (state, run) = (store::state_dir(), store::run_dir());
    gh0st_core::status::Watch::dirs(&[state.as_path(), run.as_path()]).ok()
}

#[cfg(not(target_os = "linux"))]
fn watch_dirs() -> Option<gh0st_core::status::Watch> {
    None
}
