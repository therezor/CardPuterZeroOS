//! The Files page: two sides of one pane, Places at each side's root, and
//! the views a side opens (a text file, a file's info, a server's form, the
//! transfers).

use crate::engine::{Ask, Engine, Event, Id, Request};
use crate::model::{self, Entry, Kind, Loc, Sort, Type};
use crate::open_cache;
use crate::pick::{Mode, Spec};
use crate::servers::{self, Proto, Server};
use crate::words;
use gh0st_app::Handler;
use gh0st_app::prelude::*;
use gh0st_app::secret::{Keyring, Lookup};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const VIEW_MAX: u64 = 256 * 1024;

// Commands.
const CMD_MARK: u32 = 1;
const CMD_ALL: u32 = 2;
const CMD_FIND: u32 = 3;
const CMD_SIDE: u32 = 4;
const CMD_RENAME: u32 = 5;
const CMD_MKDIR: u32 = 6;
const CMD_DELETE: u32 = 7;
const CMD_INFO: u32 = 8;
const CMD_EXTRACT: u32 = 9;
const CMD_COMPRESS: u32 = 10;
const CMD_SORT: u32 = 11;
const CMD_HIDDEN: u32 = 12;
const CMD_SAME: u32 = 13;
const CMD_TRANSFERS: u32 = 14;
const CMD_EMPTY: u32 = 15;
const CMD_ADD: u32 = 16;
const CMD_COPY: u32 = 17;
const CMD_CUT: u32 = 18;
const CMD_PGUP: u32 = 19;
const CMD_PGDN: u32 = 20;
const CMD_KEY: u32 = 21;
const CMD_PASTE: u32 = 22;
const CMD_CLEAR: u32 = 23;
const CMD_OPEN_WITH: u32 = 24;
const CMD_TYPE: u32 = 25;

// Dialogs.
const ASK_DELETE: u32 = 3;
const ASK_RENAME: u32 = 4;
const ASK_MKDIR: u32 = 5;
const ASK_PASSWORD: u32 = 6;
const ASK_HOSTKEY: u32 = 7;
const ASK_EMPTY: u32 = 9;
const ASK_ADDRESS: u32 = 10;
const ASK_USER: u32 = 11;
const ASK_SECRET: u32 = 12;
const ASK_FOLDER: u32 = 13;
const ASK_NAME: u32 = 14;
const ASK_COMPRESS: u32 = 15;
const ASK_SAVE: u32 = 16;
const ASK_REPLACE: u32 = 17;

const KEY_SORT: &str = "files.sort";
const KEY_HIDDEN: &str = "files.hidden";

/// What a page needs from outside: tests and mockups pass their own.
pub struct Options {
    pub home: PathBuf,
    /// The app's data folder; None takes `cx.data_dir()`.
    pub data: Option<PathBuf>,
    pub engine: Box<dyn Engine>,
    pub keyring: Keyring,
    /// Files runs as another app's file chooser.
    pub pick: Option<Picker>,
}

/// The file chooser's question and where its answer goes: the paths
/// picked, or None when the user gave up, and the file type in force.
pub struct Picker {
    pub spec: Spec,
    pub done: Done,
}

/// Where a chooser's answer goes.
pub type Done = Box<dyn FnMut(Option<Vec<PathBuf>>, Option<usize>)>;

impl Options {
    pub fn system() -> Options {
        Options {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/".into()),
            data: None,
            engine: Box::new(crate::child::Child::new()),
            keyring: Keyring::connect(),
            pick: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Loading(Id),
    Failed(String),
}

/// A row of Places.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Place {
    Home,
    Card,
    Downloads,
    /// Files opened lately, by any app.
    Recent,
    Trash,
    Server(usize),
    Add,
}

struct Side {
    loc: Loc,
    /// Where the side entered from Places: Esc there goes back to Places.
    base: Loc,
    /// What shows: sorted, hidden ones gone unless asked for.
    entries: Vec<Entry>,
    /// The folder had more than the listing keeps.
    more: bool,
    state: State,
    list: List,
    marks: BTreeSet<String>,
    search: Search,
    /// An archive's whole table of contents while inside one.
    archive: Option<Vec<Entry>>,
    /// The folder as listed, before sorting and hiding: a new sort or
    /// Hidden files reshape it without listing again.
    all: Vec<Entry>,
    /// The row to put the cursor on once a listing under way arrives.
    want: Option<String>,
    /// As a file chooser: which files show.
    spec: Option<Spec>,
}

impl Side {
    fn new() -> Side {
        Side {
            loc: Loc::Places,
            base: Loc::Places,
            entries: Vec::new(),
            more: false,
            state: State::Ready,
            list: List::stopping(),
            marks: BTreeSet::new(),
            search: Search::new(),
            archive: None,
            all: Vec::new(),
            want: None,
            spec: None,
        }
    }

    /// The rows on screen, as indexes into `entries`: all, or the search's.
    fn shown(&self) -> Vec<usize> {
        if self.search.active() && !self.search.query().is_empty() {
            self.search.rank(self.entries.len(), |i| {
                self.search.score(&[&self.entries[i].name])
            })
        } else {
            (0..self.entries.len()).collect()
        }
    }

    /// A ".." row heads a folder, while no search narrows it.
    fn up_row(&self) -> bool {
        self.loc != Loc::Places && self.state == State::Ready && !self.search.active()
    }

    /// Rows above the entries: ".." or none.
    fn offset(&self) -> usize {
        usize::from(self.up_row())
    }

    /// The cursor is on "..".
    fn on_up(&self) -> bool {
        self.up_row() && self.list.selected() == 0
    }

    /// The cursor to the first entry, past "..", as a folder opens.
    fn top(&mut self) {
        let first = if self.entries.is_empty() {
            0
        } else {
            self.offset()
        };
        self.list.select(first);
    }

    /// A listing arrived: the cursor where it was asked to be, or the top.
    fn settle(&mut self) {
        match self.want.take() {
            Some(w) if self.entries.iter().any(|e| e.name == w) => self.select_name(&w),
            _ => self.top(),
        }
    }

    /// New entries: sorted and filtered from `all`, marks kept for names
    /// still there.
    fn reshape(&mut self, sort: Sort, hidden: bool) {
        self.entries = shape(self.all.clone(), sort, hidden);
        if let Some(spec) = &self.spec {
            self.entries
                .retain(|e| spec.shows(&e.name, e.kind.is_dir()));
        }
        let names: HashSet<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
        self.marks.retain(|m| names.contains(m.as_str()));
    }

    fn current(&self) -> Option<&Entry> {
        let i = self.list.selected().checked_sub(self.offset())?;
        self.shown().get(i).and_then(|&i| self.entries.get(i))
    }

    /// The marked names, or the one under the cursor.
    fn picked(&self) -> Vec<String> {
        if !self.marks.is_empty() {
            return self
                .entries
                .iter()
                .filter(|e| self.marks.contains(&e.name))
                .map(|e| e.name.clone())
                .collect();
        }
        self.current()
            .map(|e| vec![e.name.clone()])
            .unwrap_or_default()
    }

    /// As `picked`, files only: what a chooser hands back.
    fn picked_files(&self) -> Vec<String> {
        let names = self.picked();
        self.entries
            .iter()
            .filter(|e| !e.kind.is_dir() && names.contains(&e.name))
            .map(|e| e.name.clone())
            .collect()
    }

    fn select_name(&mut self, name: &str) {
        if self.state != State::Ready {
            self.want = Some(name.into());
            return;
        }
        if let Some(i) = self
            .shown()
            .iter()
            .position(|&i| self.entries[i].name == name)
        {
            self.list.select(i + self.offset());
        }
    }
}

/// What Copy or Cut took, waiting for Paste in another folder.
#[derive(Clone, Debug)]
struct Clip {
    /// The folder the names are in.
    from: Loc,
    names: Vec<String>,
    cut: bool,
}

#[derive(Clone, Debug)]
struct Transfer {
    id: Id,
    verb: &'static str,
    what: String,
    to: String,
    done: u64,
    total: u64,
    name: String,
}

/// Recent files the Recent place lists at most.
const RECENT_SHOWN: usize = 50;

/// What a request was for, so its events land in the right place.
#[derive(Clone, Debug)]
enum Pending {
    List(usize),
    Contents(usize),
    /// A change to folders: what to say, and the folders to list again.
    Op(&'static str, Vec<Loc>),
    /// A copy or move, and the folders it changes.
    Transfer(Vec<Loc>),
    /// Making the SSH key.
    Key,
    /// A copy of a file on a server or in an archive, to open once it is
    /// here, and the source's time.
    Fetch(PathBuf, i64),
}

struct ServerForm {
    s: Server,
    /// Typed here; handed to the first password question, kept in the
    /// keyring only once it worked.
    password: Option<String>,
    /// Which saved server this edits.
    editing: Option<usize>,
    list: List,
}

enum View {
    Browse,
    Viewer {
        name: String,
        text: String,
        scroll: Scroll,
    },
    Info {
        rows: Vec<(&'static str, String)>,
        list: List,
    },
    Server(ServerForm),
    Transfers,
    /// The app's SSH key: its public half, once made.
    Key(Option<String>),
    Conflict {
        id: Id,
        name: String,
        all: bool,
        list: List,
    },
    /// The apps that open a file, the default first.
    OpenWith {
        path: PathBuf,
        handlers: Vec<Handler>,
        list: List,
    },
    /// The chooser's file types.
    Filters {
        list: List,
    },
    /// Files opened lately, newest first: Enter opens one, or picks it.
    Recent {
        files: Vec<PathBuf>,
        list: List,
    },
}

/// A question the worker is waiting on, and where its answer comes from.
struct Asking {
    id: Id,
    ask: Ask,
    /// A keyring prompt under way for it.
    prompt: Option<String>,
}

pub struct Files {
    home: PathBuf,
    data: Option<PathBuf>,
    sides: [Side; 2],
    cur: usize,
    servers: Vec<Server>,
    /// Servers the worker has been told of.
    told: HashSet<String>,
    view: View,
    sort: Sort,
    hidden: bool,
    engine: Box<dyn Engine>,
    engine_watch: Option<(Watch, std::os::fd::RawFd)>,
    kr: Keyring,
    kr_watch: Option<Watch>,
    next: Id,
    pending: HashMap<Id, Pending>,
    transfers: Vec<Transfer>,
    asking: Option<Asking>,
    /// A password the server form or a dialog gave, by server: stored once
    /// its listing works, if the server remembers.
    typed: HashMap<String, String>,
    /// Requests whose password question has asked the keyring to unlock
    /// once: a second time types it instead.
    tried_unlock: HashSet<Id>,
    /// Requests answered with a typed password, and its server: kept only
    /// if that request's listing works.
    typed_for: HashMap<Id, String>,
    /// Worker questions waiting behind the one showing.
    queued: VecDeque<(Id, Ask)>,
    /// Forget pressed once on a server row.
    armed: bool,
    title: String,
    status: String,
    /// Read when Places shows, not on every draw: whether Downloads is
    /// there, and how much the Trash holds.
    has_downloads: bool,
    trash_n: usize,
    clip: Option<Clip>,
    status_tone: Tone,
    /// What an open Delete, Rename or Compress dialog is about: the side,
    /// its folder, the names.
    target: Option<(usize, Loc, Vec<String>)>,
    pick: Option<Picker>,
    /// The file a save would replace, once asked.
    replacing: Option<PathBuf>,
}

impl Files {
    pub fn new() -> Files {
        Files::with(Options::system())
    }

    pub fn with(o: Options) -> Files {
        let mut f = Files {
            home: o.home,
            data: o.data,
            sides: [Side::new(), Side::new()],
            cur: 0,
            servers: Vec::new(),
            told: HashSet::new(),
            view: View::Browse,
            sort: Sort::Name,
            hidden: false,
            engine: o.engine,
            engine_watch: None,
            kr: o.keyring,
            kr_watch: None,
            next: 1,
            pending: HashMap::new(),
            transfers: Vec::new(),
            asking: None,
            typed: HashMap::new(),
            tried_unlock: HashSet::new(),
            typed_for: HashMap::new(),
            queued: VecDeque::new(),
            armed: false,
            title: String::new(),
            status: String::new(),
            has_downloads: false,
            trash_n: 0,
            clip: None,
            status_tone: Tone::Info,
            target: None,
            pick: o.pick,
            replacing: None,
        };
        let spec = f.pick.as_ref().map(|p| p.spec.clone());
        for side in &mut f.sides {
            side.spec = spec.clone();
        }
        f.retitle();
        f
    }

    fn spec(&self) -> Option<&Spec> {
        self.pick.as_ref().map(|p| &p.spec)
    }

    /// The chooser's answer: hand it over and close.
    fn finish(&mut self, paths: Option<Vec<PathBuf>>, cx: &mut Cx) {
        if let Some(p) = &mut self.pick {
            let filter = p.spec.filter;
            (p.done)(paths, filter);
        }
        cx.quit();
    }

    /// The local folder the side shows.
    fn here(&self) -> Option<PathBuf> {
        match &self.side().loc {
            Loc::Local(p) => Some(p.clone()),
            _ => None,
        }
    }

    /// As a chooser, Enter on a file.
    fn pick_file(&mut self, e: &Entry, cx: &mut Cx) {
        let Some(dir) = self.here() else {
            return;
        };
        let Some(spec) = self.spec().cloned() else {
            return;
        };
        match spec.mode {
            Mode::Open => {
                let names = if spec.multiple {
                    self.side().picked_files()
                } else {
                    vec![e.name.clone()]
                };
                if names.is_empty() {
                    return;
                }
                self.finish(Some(names.iter().map(|n| dir.join(n)).collect()), cx);
            }
            Mode::Save => self.ask_replace(dir.join(&e.name), cx),
            Mode::Folder => {}
        }
    }

    fn ask_replace(&mut self, path: PathBuf, cx: &mut Cx) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.replacing = Some(path);
        cx.ask(
            ASK_REPLACE,
            Dialog::confirm(
                &format!("Replace {}?", name.unwrap_or_default()),
                "The file there now is lost.",
            )
            .yes("replace")
            .danger(),
        );
    }

    fn ask_save(&mut self, name: &str, cx: &mut Cx) {
        let d = Dialog::text("Save as", "name").value(name).yes("save");
        cx.ask(ASK_SAVE, d);
    }

    /// The chooser's chip `n`.
    fn pick_soft(&mut self, n: u8, cx: &mut Cx) {
        let Some(spec) = self.spec().cloned() else {
            return;
        };
        let filters = spec.filters.len() > 1;
        let (type_key, main_key) = match spec.mode {
            Mode::Open => (6, 0),
            Mode::Save if filters => (5, 7),
            Mode::Save | Mode::Folder => (0, 6),
        };
        if n == 4 && spec.multiple && spec.mode == Mode::Open {
            self.mark();
        } else if n == type_key && filters && self.in_folder() {
            self.show_filters();
        } else if n == main_key && self.in_folder() {
            let Some(dir) = self.here() else {
                return;
            };
            match spec.mode {
                Mode::Save => self.ask_save(&spec.name, cx),
                Mode::Folder => self.finish(Some(vec![dir]), cx),
                Mode::Open => {}
            }
        }
    }

    fn show_filters(&mut self) {
        let mut list = List::stopping();
        list.select(self.spec().and_then(|s| s.filter).unwrap_or(0));
        self.view = View::Filters { list };
    }

    fn set_filter(&mut self, i: usize) {
        if let Some(p) = &mut self.pick {
            p.spec.filter = Some(i);
        }
        let spec = self.spec().cloned();
        let (sort, hidden) = (self.sort, self.hidden);
        for side in &mut self.sides {
            side.spec = spec.clone();
            side.reshape(sort, hidden);
            side.settle();
        }
    }

    fn pick_bars(&self) -> Bars {
        let Some(spec) = self.spec() else {
            return Bars::top();
        };
        let s = self.side();
        if s.search.active() {
            return Bars::top().search(&s.search);
        }
        if !self.in_folder() {
            return Bars::top();
        }
        let filters = spec.filters.len() > 1;
        let ty = Chip::new("file", "Type");
        let main = Chip::new("check", spec.verb()).main();
        match spec.mode {
            Mode::Open => {
                let mut b = Bars::top();
                if spec.multiple {
                    let marked = s.current().is_some_and(|e| s.marks.contains(&e.name));
                    b = b.chip(4, Chip::new("check", "Mark").on(marked));
                }
                if filters { b.chip(6, ty) } else { b }
            }
            Mode::Save if filters => Bars::top().chip(5, ty).chip(7, main),
            Mode::Save | Mode::Folder => Bars::top().chip(6, main),
        }
    }

    fn pick_commands(&self, spec: &Spec, browse: bool, c: &mut Commands) {
        let s = self.side();
        let folder = browse && self.in_folder();
        let free = folder && !s.search.active();
        c.group("Files");
        if spec.mode == Mode::Open && spec.multiple {
            c.add(
                Command::action(CMD_MARK, "Mark")
                    .keys("space")
                    .enabled(folder)
                    .mode("Folder", free),
            );
            c.add(
                Command::action(CMD_ALL, "Mark all")
                    .keys("a")
                    .enabled(folder)
                    .mode("Folder", free),
            );
        }
        c.add(
            Command::action(CMD_FIND, "Find in folder")
                .keys("n")
                .enabled(folder)
                .mode("Folder", free),
        );
        if spec.filters.len() > 1 {
            c.add(
                Command::action(CMD_TYPE, "File type")
                    .opens()
                    .enabled(folder),
            );
        }
        if spec.mode != Mode::Open {
            c.add(
                Command::action(CMD_MKDIR, "New folder")
                    .keys("k")
                    .enabled(folder)
                    .mode("Folder", free),
            );
        }
        c.group("View");
        c.add(Command::choice(CMD_SORT, "Sort by", self.sort.word()));
        c.add(Command::toggle(CMD_HIDDEN, "Hidden files", self.hidden));
    }

    /// The Open-with list for the file under the cursor, local ones only.
    fn open_with(&mut self, cx: &mut Cx) {
        let Some(e) = self.side().current().cloned() else {
            return;
        };
        let Loc::Local(dir) = &self.side().loc else {
            cx.toast("Copy it to the card to open it");
            return;
        };
        if e.kind.is_dir() {
            return;
        }
        let path = dir.join(&e.name);
        let handlers = cx.handlers(&path);
        if handlers.is_empty() {
            cx.toast("No app opens this file");
            return;
        }
        self.view = View::OpenWith {
            path,
            handlers,
            list: List::stopping(),
        };
    }

    /// The recent list, what is still there and, in the chooser, what its
    /// file types take.
    fn open_recent(&mut self, cx: &mut Cx) {
        let spec = self.spec().cloned();
        let files: Vec<PathBuf> = cx
            .recent_files(RECENT_SHOWN)
            .into_iter()
            .filter(|p| p.is_file())
            .filter(|p| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                spec.as_ref().is_none_or(|s| s.shows(&name, false))
            })
            .collect();
        if files.is_empty() {
            cx.toast("Nothing opened lately");
            return;
        }
        self.view = View::Recent {
            files,
            list: List::stopping(),
        };
    }

    fn side(&self) -> &Side {
        &self.sides[self.cur]
    }

    fn side_mut(&mut self) -> &mut Side {
        &mut self.sides[self.cur]
    }

    fn servers_path(&self) -> Option<PathBuf> {
        self.data.as_ref().map(|d| d.join("servers"))
    }

    fn trash(&self) -> PathBuf {
        self.home.join(".local/share/Trash/files")
    }

    fn downloads(&self) -> PathBuf {
        self.home.join("Downloads")
    }

    fn id(&mut self) -> Id {
        self.next += 1;
        self.next
    }

    fn letter(i: usize) -> char {
        if i == 0 { 'A' } else { 'B' }
    }

    /// "nas:/backup", for a transfer naming where things go.
    /// A folder as the user reads it: `~` for home.
    fn short(&self, d: &Path) -> String {
        match d.strip_prefix(&self.home) {
            Ok(r) if r.as_os_str().is_empty() => "~".into(),
            Ok(r) => format!("~/{}", r.display()),
            Err(_) => d.display().to_string(),
        }
    }

    fn where_(&self, i: usize) -> String {
        self.sides[i].loc.show(&self.home)
    }

    fn retitle(&mut self) {
        let shown = self.side().loc.show(&self.home);
        // The end of a path says more than its start.
        // A plain-text server says so first: it matters more than progress.
        (self.status, self.status_tone) = match (self.transfers.first(), &self.clip) {
            _ if self.plain_here() => ("Not private".into(), Tone::Warning),
            (Some(t), _) if t.total > 0 => (format!("{}%", t.done * 100 / t.total), Tone::Info),
            (Some(_), _) => ("\u{2026}".into(), Tone::Info),
            (None, Some(c)) => {
                let verb = if c.cut { "cut" } else { "copied" };
                (format!("{} {verb}", c.names.len()), Tone::Info)
            }
            (None, None) => (String::new(), Tone::Info),
        };
        // As a chooser, the asking app's name says who it is for.
        if let Some(spec) = self.pick.as_ref().map(|p| &p.spec) {
            if self.status.is_empty() {
                self.status = spec.app.clone();
            }
        }
        // Room left beside the clock and the status.
        let room = if self.status.is_empty() {
            22
        } else {
            22 - self.status.chars().count().min(12)
        };
        let n = shown.chars().count();
        let shown = if n > room {
            let tail: String = shown.chars().skip(n - (room - 1)).collect();
            format!("\u{2026}{tail}")
        } else {
            shown
        };
        self.title = match self.pick.as_ref().map(|p| &p.spec) {
            Some(spec) if self.side().loc == Loc::Places => spec.verb().to_string(),
            Some(spec) => format!("{} \u{00b7} {shown}", spec.verb()),
            None => format!("{} \u{00b7} {shown}", Files::letter(self.cur)),
        };
    }

    fn server(&self, name: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.name == name)
    }

    fn server_of(&self, loc: &Loc) -> Option<&Server> {
        match loc {
            Loc::Remote { server, .. } => self.server(server),
            Loc::Archive { file, .. } => self.server_of(file),
            _ => None,
        }
    }

    fn plain_here(&self) -> bool {
        match &self.view {
            View::Server(f) => f.s.proto.plain(),
            _ => self
                .server_of(&self.side().loc)
                .is_some_and(|s| s.proto.plain()),
        }
    }

    // ---- places ----

    fn places(&self) -> Vec<Place> {
        let mut v = vec![Place::Home, Place::Card];
        if self.has_downloads {
            v.push(Place::Downloads);
        }
        if self.spec().is_none_or(|s| s.mode == Mode::Open) {
            v.push(Place::Recent);
        }
        // A chooser picks on the card only.
        if self.pick.is_some() {
            return v;
        }
        v.push(Place::Trash);
        v.extend((0..self.servers.len()).map(Place::Server));
        v.push(Place::Add);
        v
    }

    fn place_here(&self) -> Option<Place> {
        if self.side().loc != Loc::Places {
            return None;
        }
        self.places().get(self.side().list.selected()).cloned()
    }

    fn open_place(&mut self, p: Place, cx: &mut Cx) {
        let loc = match p {
            Place::Recent => {
                self.open_recent(cx);
                return;
            }
            Place::Home => Loc::Local(self.home.clone()),
            Place::Card => Loc::Local("/".into()),
            Place::Downloads => Loc::Local(self.downloads()),
            Place::Trash => {
                let t = self.trash();
                let _ = std::fs::create_dir_all(&t);
                Loc::Local(t)
            }
            Place::Server(i) => {
                let Some(s) = self.servers.get(i) else {
                    return;
                };
                Loc::Remote {
                    server: s.name.clone(),
                    path: s.path.clone(),
                }
            }
            Place::Add => {
                self.open_form(Server::new(Proto::Sftp), None);
                return;
            }
        };
        self.side_mut().base = loc.clone();
        self.go(loc, cx);
    }

    // ---- moving around ----

    /// Shows `loc` on this side, from the top.
    fn go(&mut self, loc: Loc, cx: &mut Cx) {
        self.leave(self.cur, cx);
        let s = self.side_mut();
        s.loc = loc;
        s.marks.clear();
        s.search.cancel();
        s.list.select(0);
        s.want = None;
        if !s.loc.is_archive() {
            s.archive = None;
        }
        self.armed = false;
        self.load(self.cur, cx);
        if self.side().state == State::Ready {
            self.side_mut().settle();
        }
        self.retitle();
    }

    /// A side moves on: a listing it waited for is no longer wanted.
    fn leave(&mut self, i: usize, cx: &mut Cx) {
        if let State::Loading(old) = self.sides[i].state {
            self.pending.remove(&old);
            self.drop_asks(old, cx);
            self.send(Request::Cancel { id: old }, cx);
            self.sides[i].state = State::Ready;
        }
    }

    fn up(&mut self, cx: &mut Cx) {
        let s = self.side();
        let from = s.loc.leaf();
        let to = s.loc.up(&s.base).unwrap_or(Loc::Places);
        let leaving_archive = s.loc.is_archive() && !to.is_archive();
        if to == Loc::Places {
            let i = self.places_index_of(&s.base);
            self.go(Loc::Places, cx);
            self.side_mut().list.select(i);
            return;
        }
        self.go(to, cx);
        if leaving_archive {
            self.side_mut().archive = None;
        }
        if let Some(name) = from {
            self.side_mut().select_name(&name);
        }
    }

    fn places_index_of(&self, base: &Loc) -> usize {
        let places = self.places();
        let want = match base {
            Loc::Local(p) if *p == self.home => Some(Place::Home),
            Loc::Local(p) if p == Path::new("/") => Some(Place::Card),
            Loc::Local(p) if *p == self.downloads() => Some(Place::Downloads),
            Loc::Local(p) if *p == self.trash() => Some(Place::Trash),
            Loc::Remote { server, .. } => self
                .servers
                .iter()
                .position(|s| &s.name == server)
                .map(Place::Server),
            _ => None,
        };
        want.and_then(|w| places.iter().position(|p| *p == w))
            .unwrap_or(0)
    }

    fn open(&mut self, cx: &mut Cx) {
        if self.side().loc == Loc::Places {
            if let Some(p) = self.place_here() {
                self.open_place(p, cx);
            }
            return;
        }
        if self.side().state != State::Ready {
            return;
        }
        if self.side().on_up() {
            self.up(cx);
            return;
        }
        let Some(e) = self.side().current().cloned() else {
            return;
        };
        let loc = self.side().loc.join(&e.name);
        if e.kind.is_dir() {
            self.go(loc, cx);
            return;
        }
        if self.pick.is_some() {
            self.pick_file(&e, cx);
            return;
        }
        // A local file opens in its app; without one, as before.
        let local = match &loc {
            Loc::Local(p) if e.sort() != Type::Archive => Some(p.clone()),
            _ => None,
        };
        if let Some(path) = local {
            if e.kind == Kind::File && !cx.handlers(&path).is_empty() {
                cx.open_file(&path);
                return;
            }
        }
        // On a server or in an archive: a copy in the cache opens instead.
        let elsewhere = matches!(loc, Loc::Remote { .. } | Loc::Archive { .. });
        if elsewhere && e.kind == Kind::File && e.sort() != Type::Archive {
            let copy = open_cache::slot(&self.home, &format!("{loc:?}")).join(&e.name);
            if !cx.handlers(&copy).is_empty() {
                self.fetch_open(&e, &loc, cx);
                return;
            }
        }
        match e.sort() {
            Type::Archive if !self.side().loc.is_archive() => {
                let inside = Loc::Archive {
                    file: Box::new(loc.clone()),
                    inner: String::new(),
                };
                let s = self.side_mut();
                s.loc = inside;
                s.list.select(0);
                s.marks.clear();
                s.search.cancel();
                s.entries.clear();
                s.all.clear();
                s.archive = None;
                s.want = None;
                let id = self.id();
                self.side_mut().state = State::Loading(id);
                self.pending.insert(id, Pending::Contents(self.cur));
                self.send(Request::Contents { id, archive: loc }, cx);
                self.retitle();
            }
            t if t.readable() && e.kind != Kind::Other => self.view_text(&e, &loc, cx),
            _ => self.info(),
        }
    }

    /// Copies `e` at `loc` into the open cache, then opens it in its app.
    /// A copy already there, the same size, opens at once.
    fn fetch_open(&mut self, e: &Entry, loc: &Loc, cx: &mut Cx) {
        let dir = open_cache::slot(&self.home, &format!("{loc:?}"));
        let file = dir.join(&e.name);
        // Under way already: a second start would pull the folder from
        // under the first.
        let fetching = self
            .pending
            .values()
            .any(|p| matches!(p, Pending::Fetch(f, _) if f.parent() == Some(dir.as_path())));
        if fetching {
            cx.toast("Getting it already");
            return;
        }
        if open_cache::fresh(&file, e.size, e.mtime) {
            open_cache::touch(&dir);
            cx.open_file(&file);
            return;
        }
        open_cache::remove(&dir);
        if let Err(err) = std::fs::create_dir_all(&dir) {
            self.say_failed(&err.to_string(), cx);
            return;
        }
        let id = self.id();
        let to = Loc::Local(dir);
        let request = match loc {
            Loc::Archive {
                file: archive,
                inner,
            } => Request::Extract {
                id,
                archive: (**archive).clone(),
                entries: vec![inner.clone()],
                to,
            },
            _ => Request::Copy {
                id,
                items: vec![loc.clone()],
                to,
                remove: false,
            },
        };
        self.transfers.push(Transfer {
            id,
            verb: "Getting",
            what: format!("\u{201c}{}\u{201d}", e.name),
            to: "this device".into(),
            done: 0,
            total: e.size,
            name: e.name.clone(),
        });
        self.pending.insert(id, Pending::Fetch(file, e.mtime));
        cx.keep_awake(true);
        self.send(request, cx);
    }

    fn view_text(&mut self, e: &Entry, loc: &Loc, cx: &mut Cx) {
        let Loc::Local(path) = loc else {
            cx.toast("Copy it to the card to read it");
            return;
        };
        // A pipe or a device named .txt would block the page.
        if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
            cx.toast("Only a plain file opens here");
            return;
        }
        use std::io::Read;
        let mut buf = Vec::new();
        let read = std::fs::File::open(path).and_then(|f| f.take(VIEW_MAX).read_to_end(&mut buf));
        if let Err(err) = read {
            cx.toast_tone(Tone::Critical, &io_words(&err));
            return;
        }
        let mut text = String::from_utf8(buf)
            .unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
        if e.size > VIEW_MAX {
            text.push_str("\n\n(The rest is not shown.)");
        }
        self.view = View::Viewer {
            name: e.name.clone(),
            text: text.replace('\t', "    "),
            scroll: Scroll::new(),
        };
    }

    fn info(&mut self) {
        let Some(e) = self.side().current().cloned() else {
            return;
        };
        let kind = match e.kind {
            Kind::Dir => "Folder",
            Kind::LinkDir => "Link to a folder",
            Kind::Link => "Link",
            Kind::Other => "Device or pipe",
            Kind::File => "File",
        };
        let mut rows = vec![("Name", e.name.clone()), ("Kind", kind.to_string())];
        if !e.kind.is_dir() {
            rows.push(("Size", model::size_text(e.size)));
        }
        rows.push(("Changed", model::time_text(e.mtime)));
        if e.mode != 0 {
            rows.push(("Access", model::mode_text(e.mode)));
        }
        if let Some(l) = &e.link {
            rows.push(("Points to", l.clone()));
        }
        rows.push(("In", self.side().loc.show(&self.home)));
        self.view = View::Info {
            rows,
            list: List::stopping(),
        };
    }

    /// Lists the side's folder: a local one here and now, the rest through
    /// the worker.
    fn load(&mut self, i: usize, cx: &mut Cx) {
        let loc = self.sides[i].loc.clone();
        match &loc {
            Loc::Places => {
                self.look_at_places();
                let s = &mut self.sides[i];
                s.entries.clear();
                s.state = State::Ready;
            }
            Loc::Local(p) => {
                let r = model::list_local(p, model::MAX_ENTRIES);
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                match r {
                    Ok((all, more)) => {
                        s.all = all;
                        s.reshape(sort, hidden);
                        s.more = more;
                        s.state = State::Ready;
                    }
                    Err(err) => {
                        s.entries.clear();
                        s.all.clear();
                        s.state = State::Failed(io_words(&err));
                    }
                }
            }
            Loc::Remote { server, .. } => {
                if let Some(srv) = self.server(server).cloned() {
                    if self.told.insert(srv.name.clone()) {
                        self.send(Request::Server(srv), cx);
                    }
                }
                let id = self.id();
                self.sides[i].state = State::Loading(id);
                self.sides[i].entries.clear();
                self.sides[i].all.clear();
                self.pending.insert(id, Pending::List(i));
                self.send(Request::List { id, at: loc }, cx);
            }
            Loc::Archive { inner, .. } => {
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                if let Some(all) = &s.archive {
                    s.all = inside(all, inner);
                    s.reshape(sort, hidden);
                    s.state = State::Ready;
                }
            }
        }
    }

    fn look_at_places(&mut self) {
        self.has_downloads = self.downloads().is_dir();
        self.trash_n = std::fs::read_dir(self.trash())
            .map(|r| r.count())
            .unwrap_or(0);
    }

    /// Lists again the folders a change touched, and every folder on the
    /// card (that costs nothing). The cursor stays on its name, and marks
    /// on names still there.
    fn reload(&mut self, touched: &[Loc], cx: &mut Cx) {
        for i in 0..2 {
            let loc = self.sides[i].loc.clone();
            let cheap = matches!(loc, Loc::Local(_) | Loc::Places);
            if !cheap && !touched.contains(&loc) {
                continue;
            }
            if matches!(self.sides[i].state, State::Loading(_)) {
                continue;
            }
            let keep = self.sides[i].current().map(|e| e.name.clone());
            let marks = self.sides[i].marks.clone();
            self.load(i, cx);
            let s = &mut self.sides[i];
            s.marks = marks;
            let names: HashSet<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
            s.marks.retain(|m| names.contains(m.as_str()));
            match keep {
                Some(n) => s.select_name(&n),
                None if s.state == State::Ready => {}
                None => s.want = None,
            }
        }
    }

    fn resort(&mut self) {
        let (sort, hidden) = (self.sort, self.hidden);
        for s in &mut self.sides {
            if s.state != State::Ready {
                continue;
            }
            let keep = s.current().map(|e| e.name.clone());
            s.reshape(sort, hidden);
            if let Some(n) = keep {
                s.select_name(&n);
            }
        }
    }

    // ---- the worker ----

    fn send(&mut self, req: Request, cx: &mut Cx) {
        self.engine.send(req);
        self.rewatch(cx);
        self.drain(cx);
    }

    /// Watches the worker's fd, a new one after a restart, none once gone.
    fn rewatch(&mut self, cx: &mut Cx) {
        let fd = self.engine.fd();
        if self.engine_watch.map(|w| w.1) == fd {
            return;
        }
        if let Some((w, _)) = self.engine_watch.take() {
            cx.unwatch(w);
            // A new worker knows no server yet.
            self.told.clear();
        }
        if let Some(fd) = fd {
            self.engine_watch = Some((cx.watch(fd), fd));
        }
    }

    fn drain(&mut self, cx: &mut Cx) {
        loop {
            let events = self.engine.events();
            if events.is_empty() {
                break;
            }
            for e in events {
                self.event(e, cx);
            }
        }
        self.rewatch(cx);
        self.next_ask(cx);
        self.retitle();
    }

    fn event(&mut self, e: Event, cx: &mut Cx) {
        let id = e.id();
        let Some(p) = self.pending.get(&id).cloned() else {
            return;
        };
        match (p, e) {
            (Pending::List(i), Event::Entries { entries, .. }) => {
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i].all.extend(entries);
                }
            }
            (Pending::List(i), Event::Listed { .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                self.listed(id);
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                if s.state == State::Loading(id) {
                    s.state = State::Ready;
                    s.reshape(sort, hidden);
                    s.settle();
                }
            }
            (Pending::Contents(i), Event::Entries { entries, .. }) => {
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i]
                        .archive
                        .get_or_insert_with(Vec::new)
                        .extend(entries);
                }
            }
            (Pending::Contents(i), Event::Listed { .. }) => {
                self.pending.remove(&id);
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i].state = State::Ready;
                    self.sides[i].archive.get_or_insert_with(Vec::new);
                    self.load(i, cx);
                    self.sides[i].settle();
                }
            }
            (Pending::List(i) | Pending::Contents(i), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                self.failed_password(id);
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i].state = State::Failed(text);
                }
            }
            (_, Event::Ask { ask, .. }) => self.ask(id, ask, cx),
            (Pending::List(i), Event::Resolved { path, .. }) => {
                // "~" was the server's home: the side and its base go there.
                if self.sides[i].state == State::Loading(id) {
                    let s = &mut self.sides[i];
                    for l in [&mut s.loc, &mut s.base] {
                        if let Loc::Remote { path: p, .. } = l {
                            if p == "~" {
                                *p = path.clone();
                            }
                        }
                    }
                    self.retitle();
                }
            }
            (
                Pending::Transfer(_) | Pending::Fetch(..),
                Event::Progress {
                    done, total, name, ..
                },
            ) => {
                if let Some(t) = self.transfers.iter_mut().find(|t| t.id == id) {
                    t.done = done;
                    t.total = total;
                    t.name = name;
                }
            }
            (Pending::Transfer(touched), Event::Done { .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                if let Some(i) = self.transfers.iter().position(|t| t.id == id) {
                    let t = self.transfers.remove(i);
                    let past = match t.verb {
                        "Moving" => "Moved",
                        "Extracting" => "Extracted",
                        "Compressing" => "Made",
                        _ => "Copied",
                    };
                    let say = format!("{past} {}", t.what);
                    if cx.in_front() {
                        cx.toast_tone(Tone::Ok, &say);
                    } else {
                        cx.post("transfer", &say, &format!("To {}", t.to));
                    }
                }
                cx.keep_awake(!self.transfers.is_empty());
                self.reload(&touched, cx);
            }
            (Pending::Transfer(touched), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                let shown = self.transfers.iter().any(|t| t.id == id);
                self.transfers.retain(|t| t.id != id);
                cx.keep_awake(!self.transfers.is_empty());
                // Stop already said so.
                if shown || text != "Stopped" {
                    self.say_failed(&text, cx);
                }
                self.reload(&touched, cx);
            }
            (Pending::Fetch(file, mtime), Event::Done { .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                self.transfers.retain(|t| t.id != id);
                cx.keep_awake(!self.transfers.is_empty());
                open_cache::seal(&file, mtime);
                cx.open_file(&file);
            }
            (Pending::Fetch(file, _), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                let shown = self.transfers.iter().any(|t| t.id == id);
                self.transfers.retain(|t| t.id != id);
                cx.keep_awake(!self.transfers.is_empty());
                if let Some(dir) = file.parent() {
                    open_cache::remove(dir);
                }
                if shown || text != "Stopped" {
                    self.say_failed(&text, cx);
                }
            }
            (Pending::Key, Event::Done { .. }) => {
                self.pending.remove(&id);
                if let View::Key(k) = &mut self.view {
                    *k = self.data.as_ref().and_then(|d| read_public_key(d));
                }
                cx.toast_tone(Tone::Ok, "Key made");
            }
            (Pending::Key, Event::Error { text, .. }) => {
                self.pending.remove(&id);
                self.say_failed(&text, cx);
            }
            (Pending::Op(done, touched), Event::Done { .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                cx.toast_tone(Tone::Ok, done);
                self.reload(&touched, cx);
            }
            (Pending::Op(_, touched), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                self.drop_asks(id, cx);
                self.say_failed(&text, cx);
                self.reload(&touched, cx);
            }
            _ => {}
        }
    }

    /// A failure: a toast, or a notice when the app is behind.
    fn say_failed(&mut self, text: &str, cx: &mut Cx) {
        if cx.in_front() {
            cx.toast_tone(Tone::Critical, text);
        } else {
            cx.post("failed", "Files couldn't finish", text);
        }
    }

    /// A request ended: its questions, showing or waiting, go with it.
    fn drop_asks(&mut self, id: Id, cx: &mut Cx) {
        if self.asking.as_ref().is_some_and(|a| a.id == id) {
            self.asking = None;
        }
        self.queued.retain(|(q, _)| *q != id);
        if matches!(self.view, View::Conflict { id: c, .. } if c == id) {
            self.view = View::Browse;
        }
        self.next_ask(cx);
    }

    /// A listing worked. If this very request was answered with a typed
    /// password, that password is known good now, and kept if the server
    /// remembers.
    fn listed(&mut self, id: Id) {
        self.tried_unlock.remove(&id);
        let Some(server) = self.typed_for.remove(&id) else {
            return;
        };
        let Some(srv) = self.server(&server).cloned() else {
            return;
        };
        let Some(pw) = self.typed.remove(&srv.name) else {
            return;
        };
        if srv.remember {
            let attrs = srv.attrs();
            let a: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            if self
                .kr
                .store(&srv.label(), &a, pw.as_bytes(), true)
                .is_err()
            {
                eprintln!(
                    "files: the keyring didn't keep the password for {}",
                    srv.name
                );
            }
        }
        pw.into_bytes().fill(0);
    }

    /// A listing failed: a password typed for it was not good; forget it.
    fn failed_password(&mut self, id: Id) {
        self.tried_unlock.remove(&id);
        if let Some(server) = self.typed_for.remove(&id) {
            if let Some(pw) = self.typed.remove(&server) {
                pw.into_bytes().fill(0);
            }
        }
    }

    // ---- questions ----

    /// A question from the worker. One shows at a time, and never over a
    /// dialog of the user's own or a half filled form: the rest wait.
    fn ask(&mut self, id: Id, ask: Ask, cx: &mut Cx) {
        self.queued.push_back((id, ask));
        self.next_ask(cx);
    }

    /// The next waiting question, when nothing else is being asked.
    fn next_ask(&mut self, cx: &mut Cx) {
        if self.asking.is_some() || cx.asking() {
            return;
        }
        let quiet = matches!(self.view, View::Browse | View::Transfers);
        let Some(i) = self
            .queued
            .iter()
            .position(|(_, a)| quiet || !matches!(a, Ask::Conflict { .. }))
        else {
            return;
        };
        if let Some((id, ask)) = self.queued.remove(i) {
            if !cx.in_front() {
                cx.post("ask", "Files needs an answer", "Open Files to go on");
            }
            self.ask_now(id, ask, cx);
            // Answered at once, from what is known: on to the next.
            if self.asking.is_none() && !cx.asking() {
                self.next_ask(cx);
            }
        }
    }

    /// Answers a question from the keyring item `a`. False when it has no
    /// answer; true when answered, or waiting on the keyring's prompt.
    fn ask_keyring(&mut self, id: Id, ask: &Ask, a: &[(&str, &str)], cx: &mut Cx) -> bool {
        // Twice at most: once, and again after an unlock.
        for _ in 0..2 {
            match self.kr.lookup(a) {
                Lookup::Found(pw) => {
                    let text = pw.as_str().unwrap_or_default().to_string();
                    self.reply(id, Some(text), cx);
                    return true;
                }
                Lookup::Locked(items) if self.tried_unlock.insert(id) => {
                    match self.kr.unlock(&items) {
                        Ok(None) => continue,
                        Ok(Some(p)) => match self.kr.ended(&p) {
                            Some(false) => continue,
                            Some(true) => return false,
                            None => {
                                self.watch_keyring(cx);
                                self.asking = Some(Asking {
                                    id,
                                    ask: ask.clone(),
                                    prompt: Some(p),
                                });
                                return true;
                            }
                        },
                        Err(_) => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn ask_now(&mut self, id: Id, ask: Ask, cx: &mut Cx) {
        match &ask {
            Ask::Password { server, retry } => {
                let server = server.clone();
                if !retry {
                    if let Some(pw) = self.typed.get(&server).cloned() {
                        self.typed_for.insert(id, server.clone());
                        self.reply(id, Some(pw), cx);
                        return;
                    }
                }
                if *retry {
                    // What was tried is wrong: neither it nor the kept one again.
                    self.typed.remove(&server);
                }
                let srv = self.server(&server).cloned();
                if let Some(srv) = srv.as_ref().filter(|s| s.remember && !retry) {
                    let attrs = srv.attrs();
                    let a: Vec<(&str, &str)> =
                        attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    if self.ask_keyring(id, &ask, &a, cx) {
                        return;
                    }
                }
                let (title, host) = match &srv {
                    Some(s) if !s.user.is_empty() => {
                        (format!("Password for {}", s.user), s.address())
                    }
                    Some(s) => ("Password".to_string(), s.address()),
                    None => ("Password".to_string(), server.clone()),
                };
                let mut d = Dialog::password(&title).line(&host).yes("connect");
                if *retry {
                    d = d.error("Wrong password");
                }
                cx.ask(ASK_PASSWORD, d);
            }
            Ask::HostKey {
                host, fingerprint, ..
            } => {
                // Every character of the key shows: it is what the user checks.
                let line = format!("New key: {}", words::grouped(fingerprint));
                cx.ask(
                    ASK_HOSTKEY,
                    Dialog::confirm(&format!("Trust {host}?"), &line).yes("trust"),
                );
            }
            Ask::Passphrase => {
                // The key's passphrase is the keyring's alone: unlocked if
                // need be, never typed.
                let a = [("xdg:schema", servers::SCHEMA), ("key", "ssh")];
                if !self.ask_keyring(id, &ask, &a, cx) {
                    self.reply(id, None, cx);
                }
                return;
            }
            Ask::Conflict { name } => {
                // The cursor starts on Keep both: Enter loses nothing.
                let mut list = List::new();
                list.select(1);
                self.view = View::Conflict {
                    id,
                    name: name.clone(),
                    all: false,
                    list,
                };
            }
        }
        self.asking = Some(Asking {
            id,
            ask,
            prompt: None,
        });
    }

    fn reply(&mut self, id: Id, text: Option<String>, cx: &mut Cx) {
        self.send(Request::Reply { id, text }, cx);
    }

    fn watch_keyring(&mut self, cx: &mut Cx) {
        if self.kr_watch.is_none() {
            if let Some(fd) = self.kr.fd() {
                self.kr_watch = Some(cx.watch(fd));
            }
        }
    }

    // ---- actions ----

    fn in_folder(&self) -> bool {
        self.side().loc != Loc::Places && self.side().state == State::Ready
    }

    fn mark(&mut self) {
        if !self.in_folder() {
            return;
        }
        let picking = self.pick.is_some();
        let s = self.side_mut();
        // A chooser picks files: a folder is for going into.
        let Some(name) = s
            .current()
            .filter(|e| !(picking && e.kind.is_dir()))
            .map(|e| e.name.clone())
        else {
            return;
        };
        if !s.marks.remove(&name) {
            s.marks.insert(name);
        }
        let n = s.shown().len() + s.offset();
        let i = s.list.selected();
        s.list.select((i + 1).min(n.saturating_sub(1)));
    }

    /// Marks every row shown (a search's matches only), or clears. A
    /// chooser marks files only.
    fn mark_all(&mut self) {
        let picking = self.pick.is_some();
        let s = self.side_mut();
        let shown: BTreeSet<String> = s
            .shown()
            .into_iter()
            .map(|i| &s.entries[i])
            .filter(|e| !(picking && e.kind.is_dir()))
            .map(|e| e.name.clone())
            .collect();
        if shown.is_subset(&s.marks) {
            s.marks.retain(|m| !shown.contains(m));
        } else {
            s.marks.extend(shown);
        }
    }

    /// Copy (or Cut): the marked names, or the one under the cursor, wait
    /// for Paste in any folder.
    fn take(&mut self, cut: bool, cx: &mut Cx) {
        if !self.in_folder() {
            return;
        }
        let names = self.side().picked();
        if names.is_empty() {
            return;
        }
        if cut && self.side().loc.is_archive() {
            cx.toast("Nothing leaves an archive: Copy takes from it");
            return;
        }
        let n = names.len();
        self.clip = Some(Clip {
            from: self.side().loc.clone(),
            names,
            cut,
        });
        self.side_mut().marks.clear();
        let verb = if cut { "Cut" } else { "Copied" };
        cx.toast(&format!("{verb} {}. Paste with 6", words::items(n)));
    }

    /// Paste: what Copy or Cut took, into this folder.
    fn paste(&mut self, cx: &mut Cx) {
        let Some(clip) = self.clip.clone() else {
            cx.toast("Copy or Cut something first");
            return;
        };
        let to = self.side().loc.clone();
        if !self.in_folder() || to.is_archive() {
            cx.toast("Open a folder to paste into");
            return;
        }
        if clip.cut && clip.from == to {
            cx.toast("It is in this folder already");
            return;
        }
        let id = self.id();
        let (req, verb) = match &clip.from {
            Loc::Archive { file, inner } => {
                let entries = clip
                    .names
                    .iter()
                    .map(|n| {
                        if inner.is_empty() {
                            n.clone()
                        } else {
                            format!("{inner}/{n}")
                        }
                    })
                    .collect();
                let r = Request::Extract {
                    id,
                    archive: (**file).clone(),
                    entries,
                    to: to.clone(),
                };
                (r, "Extracting")
            }
            from => {
                let r = Request::Copy {
                    id,
                    items: clip.names.iter().map(|n| from.join(n)).collect(),
                    to: to.clone(),
                    remove: clip.cut,
                };
                (r, if clip.cut { "Moving" } else { "Copying" })
            }
        };
        self.transfers.push(Transfer {
            id,
            verb,
            what: words::items(clip.names.len()),
            to: self.where_(self.cur),
            done: 0,
            total: 0,
            name: String::new(),
        });
        let mut touched = vec![to.clone()];
        if !clip.from.is_archive() {
            touched.push(clip.from.clone());
        }
        // Cut moves once; a copy pastes again.
        if clip.cut {
            self.clip = None;
        }
        self.pending.insert(id, Pending::Transfer(touched));
        cx.keep_awake(true);
        self.send(req, cx);
    }

    fn delete(&mut self, cx: &mut Cx) {
        if !self.in_folder() || self.side().loc.is_archive() {
            return;
        }
        let names = self.side().picked();
        if names.is_empty() {
            return;
        }
        let what = words::what(&names);
        let for_good = self.side().loc.is_remote() || self.in_trash();
        // The question in the title, the names in the line: a long name
        // can't push the question out.
        let d = if for_good {
            Dialog::confirm(
                "Delete for good?",
                &format!("{what}. This can't be undone."),
            )
            .yes("delete")
            .danger()
            .hold()
        } else {
            Dialog::confirm(
                "Move to the Trash?",
                &format!("{what}. Empty the Trash to free the space."),
            )
            .yes("delete")
        };
        self.target = Some((self.cur, self.side().loc.clone(), names));
        cx.ask(ASK_DELETE, d);
    }

    fn in_trash(&self) -> bool {
        matches!(&self.side().loc, Loc::Local(p) if p.starts_with(self.trash()))
    }

    fn op(
        &mut self,
        req_of: impl FnOnce(Id) -> Request,
        done: &'static str,
        touched: Vec<Loc>,
        cx: &mut Cx,
    ) {
        let id = self.id();
        self.pending.insert(id, Pending::Op(done, touched));
        self.send(req_of(id), cx);
    }

    /// What a dialog was opened about, if the side is still there.
    fn take_target(&mut self) -> Option<(Loc, Vec<String>)> {
        let (side, loc, names) = self.target.take()?;
        (self.sides[side].loc == loc).then_some((loc, names))
    }

    fn rename(&mut self, cx: &mut Cx) {
        if !self.in_folder() || self.side().loc.is_archive() {
            return;
        }
        let Some(e) = self.side().current() else {
            return;
        };
        let d = Dialog::text("Rename", "name").value(&e.name).yes("rename");
        self.target = Some((self.cur, self.side().loc.clone(), vec![e.name.clone()]));
        cx.ask(ASK_RENAME, d);
    }

    fn mkdir(&mut self, cx: &mut Cx) {
        if self.side().loc == Loc::Places || self.side().loc.is_archive() {
            return;
        }
        cx.ask(ASK_MKDIR, Dialog::text("New folder", "name").yes("create"));
    }

    fn extract_here(&mut self, cx: &mut Cx) {
        let Some(e) = self.side().current().cloned() else {
            return;
        };
        if e.sort() != Type::Archive || self.side().loc.is_archive() {
            cx.toast("Pick an archive to extract");
            return;
        }
        let stem = archive_stem(&e.name);
        let names: HashSet<&str> = self
            .side()
            .entries
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        let mut folder = stem.clone();
        let mut n = 2;
        while names.contains(folder.as_str()) {
            folder = format!("{stem} {n}");
            n += 1;
        }
        let here = self.side().loc.clone();
        let id = self.id();
        self.transfers.push(Transfer {
            id,
            verb: "Extracting",
            what: format!("\u{201c}{}\u{201d}", e.name),
            to: self.where_(self.cur),
            done: 0,
            total: 0,
            name: String::new(),
        });
        self.pending
            .insert(id, Pending::Transfer(vec![here.clone()]));
        cx.keep_awake(true);
        self.send(
            Request::Extract {
                id,
                archive: here.join(&e.name),
                entries: Vec::new(),
                to: here.join(&folder),
            },
            cx,
        );
    }

    fn compress(&mut self, cx: &mut Cx) {
        if !self.in_folder() || self.side().loc.is_archive() {
            return;
        }
        let names = self.side().picked();
        let Some(first) = names.first() else {
            return;
        };
        let stem = if names.len() == 1 {
            first.clone()
        } else {
            "Archive".into()
        };
        self.target = Some((self.cur, self.side().loc.clone(), names.clone()));
        cx.ask(
            ASK_COMPRESS,
            Dialog::text("Compress to zip", "name")
                .value(&format!("{stem}.zip"))
                .yes("compress"),
        );
    }

    // ---- servers ----

    fn open_form(&mut self, s: Server, editing: Option<usize>) {
        self.armed = false;
        self.view = View::Server(ServerForm {
            s,
            password: None,
            editing,
            list: List::stopping(),
        });
        self.retitle();
    }

    fn form_key(&mut self, key: Key, cx: &mut Cx) {
        let View::Server(f) = &mut self.view else {
            return;
        };
        if key == Key::Soft(6) {
            self.connect(cx);
            return;
        }
        let row = match f.list.key(key) {
            Some(Action::Open(i)) => (i, 1),
            Some(Action::Change(i, d)) => (i, d),
            None => return,
        };
        match row {
            (0, d) => {
                let all = Proto::ALL;
                let i = all.iter().position(|p| *p == f.s.proto).unwrap_or(0);
                let default_port = f.s.port == f.s.proto.port();
                let default_path = f.s.path == f.s.proto.start();
                f.s.proto = all[gh0st_app::cycle(i, d, all.len())];
                if default_port {
                    f.s.port = f.s.proto.port();
                }
                if default_path {
                    f.s.path = f.s.proto.start().into();
                }
            }
            (1, _) => cx.ask(
                ASK_ADDRESS,
                Dialog::text("Address", "nas.local or 192.168.1.5")
                    .value(&f.s.address())
                    .line("Add :port for another port."),
            ),
            (2, _) => cx.ask(ASK_USER, Dialog::text("User", "name").value(&f.s.user)),
            (3, _) => cx.ask(ASK_SECRET, Dialog::password("Password").yes("set")),
            (4, _) => f.s.remember = !f.s.remember,
            (5, _) => cx.ask(
                ASK_FOLDER,
                Dialog::text("Folder to open", "/")
                    .value(&f.s.path)
                    .line("~ is your home folder there."),
            ),
            (6, _) => {
                let name = if f.s.name.is_empty() {
                    f.s.host.clone()
                } else {
                    f.s.name.clone()
                };
                cx.ask(
                    ASK_NAME,
                    Dialog::text("Name in Places", "name").value(&name),
                );
            }
            _ => {}
        }
        self.retitle();
    }

    fn connect(&mut self, cx: &mut Cx) {
        let View::Server(f) = &mut self.view else {
            return;
        };
        if f.s.host.is_empty() {
            cx.toast_tone(Tone::Warning, "Type the server's address first");
            return;
        }
        if f.s.name.is_empty() {
            f.s.name = f.s.host.clone();
        }
        let others: Vec<Server> = self
            .servers
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != f.editing)
            .map(|(_, s)| s.clone())
            .collect();
        if let Err(why) = servers::name_ok(&f.s.name, &others) {
            cx.toast_tone(Tone::Warning, why);
            return;
        }
        let s = f.s.clone();
        let editing = f.editing;
        if let Some(pw) = f.password.take() {
            self.typed.insert(s.name.clone(), pw);
        }
        let i = match editing {
            Some(i) => {
                if let Some(old) = self.servers.get(i) {
                    self.told.remove(&old.name);
                }
                self.servers[i] = s.clone();
                i
            }
            None => {
                self.servers.push(s.clone());
                self.servers.len() - 1
            }
        };
        self.save_servers(cx);
        self.view = View::Browse;
        self.side_mut().loc = Loc::Places;
        self.open_place(Place::Server(i), cx);
    }

    fn public_key(&self) -> Option<String> {
        self.data.as_ref().and_then(|d| read_public_key(d))
    }

    /// Makes the SSH key: a random passphrase goes to the keyring first,
    /// then the worker runs ssh-keygen with it.
    fn make_key(&mut self, cx: &mut Cx) {
        if self.pending.values().any(|p| matches!(p, Pending::Key)) {
            return;
        }
        let Some(pass) = random_passphrase() else {
            cx.toast_tone(Tone::Critical, "No randomness to make a key with");
            return;
        };
        let attrs = [("xdg:schema", servers::SCHEMA), ("key", "ssh")];
        if self
            .kr
            .store("Files SSH key", &attrs, pass.as_bytes(), true)
            .is_err()
        {
            cx.toast_tone(Tone::Warning, "The key needs the keyring: unlock it first");
            return;
        }
        let id = self.id();
        self.pending.insert(id, Pending::Key);
        self.send(
            Request::MakeKey {
                id,
                passphrase: pass,
            },
            cx,
        );
    }

    fn save_servers(&mut self, cx: &mut Cx) {
        if let Some(p) = self.servers_path() {
            if servers::save(&p, &self.servers).is_err() {
                cx.toast_tone(Tone::Critical, "Couldn't save the server list");
            }
        }
    }

    fn forget(&mut self, i: usize, cx: &mut Cx) {
        if !self.armed {
            self.armed = true;
            return;
        }
        self.armed = false;
        let s = self.servers.remove(i);
        self.told.remove(&s.name);
        self.save_servers(cx);
        let attrs = s.attrs();
        let a: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // Locked or not: the keyring asks the user before each delete.
        if let Ok((open, shut)) = self.kr.search(&a) {
            for item in open.into_iter().chain(shut) {
                let _ = self.kr.delete(&item);
            }
        }
        cx.toast(&format!("Forgot {}", s.name));
    }

    fn browse_key(&mut self, key: Key, cx: &mut Cx) {
        if self.side().search.active() {
            match self.side_mut().search.key(key) {
                Some(Edit::Submit) => {
                    self.open(cx);
                    return;
                }
                Some(Edit::Changed) => {
                    self.side_mut().list.select(0);
                    return;
                }
                None => {}
            }
        }
        if let Key::Soft(n) = key {
            self.soft(n, cx);
            return;
        }
        if key == Key::Tab {
            self.flip();
            return;
        }
        let before = self.side().list.selected();
        match self.side_mut().list.key(key) {
            Some(Action::Open(_) | Action::Change(_, 1)) => self.open(cx),
            Some(Action::Change(_, _)) if self.side().loc != Loc::Places => self.up(cx),
            _ => {}
        }
        if self.side().list.selected() != before {
            self.armed = false;
        }
    }

    fn soft(&mut self, n: u8, cx: &mut Cx) {
        if let Some(Place::Server(i)) = self.place_here() {
            match n {
                5 => {
                    let s = self.servers[i].clone();
                    self.open_form(s, Some(i));
                }
                7 => self.forget(i, cx),
                _ => {}
            }
            return;
        }
        let archive = self.side().loc.is_archive();
        match n {
            4 => self.mark(),
            5 => self.take(false, cx),
            6 | 7 if archive => {}
            6 => self.paste(cx),
            7 => self.take(true, cx),
            _ => {}
        }
    }

    fn flip(&mut self) {
        self.cur = 1 - self.cur;
        self.armed = false;
        self.retitle();
    }
}

impl Default for Files {
    fn default() -> Files {
        Files::new()
    }
}

fn read_public_key(data: &Path) -> Option<String> {
    std::fs::read_to_string(data.join("ssh/id_ed25519.pub"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 32 random bytes as hex.
fn random_passphrase() -> Option<String> {
    use std::io::Read;
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Sorted, and hidden names gone unless asked for.
fn shape(mut v: Vec<Entry>, sort: Sort, hidden: bool) -> Vec<Entry> {
    if !hidden {
        v.retain(|e| !e.name.starts_with('.'));
    }
    model::sort(&mut v, sort);
    v
}

/// One folder of an archive's table of contents. Folders only implied by a
/// deeper path show too.
fn inside(all: &[Entry], inner: &str) -> Vec<Entry> {
    let prefix = if inner.is_empty() {
        String::new()
    } else {
        format!("{}/", inner.trim_end_matches('/'))
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut seen = HashSet::new();
    for e in all {
        let path = e.name.trim_end_matches('/');
        let Some(rest) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            None => {
                if seen.insert(rest.to_string()) {
                    out.push(Entry {
                        name: rest.into(),
                        ..e.clone()
                    });
                }
            }
            Some((dir, _)) => {
                if seen.insert(dir.to_string()) {
                    out.push(Entry::dir(dir));
                }
            }
        }
    }
    out
}

/// "photos.tar.gz" gives "photos".
fn archive_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for ext in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".zip", ".tar",
        ".7z", ".rar", ".iso", ".gz", ".xz", ".bz2", ".cpio",
    ] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    name.to_string()
}

fn io_words(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "No permission to open this".into(),
        std::io::ErrorKind::NotFound => "It isn't there any more".into(),
        _ => {
            let s = e.to_string();
            match s.split_once(" (os error") {
                Some((head, _)) => head.to_string(),
                None => s,
            }
        }
    }
}

impl Page for Files {
    fn title(&self) -> &str {
        match &self.view {
            View::Viewer { name, .. } => name,
            View::Info { .. } => "Info",
            View::Server(f) if f.editing.is_some() => "Edit server",
            View::Server(_) => "Add server",
            View::Transfers => "Transfers",
            View::Key(_) => "SSH key",
            View::Conflict { .. } => "Already there",
            View::OpenWith { .. } => "Open with",
            View::Recent { .. } => "Recent",
            View::Filters { .. } => "File type",
            View::Browse => &self.title,
        }
    }

    fn icon(&self) -> &str {
        "folder"
    }

    fn about(&self) -> &str {
        words::ABOUT
    }

    fn status(&self) -> Option<(&str, Tone)> {
        (!self.status.is_empty()).then_some((self.status.as_str(), self.status_tone))
    }

    fn typing(&self) -> bool {
        matches!(self.view, View::Browse) && self.side().search.active()
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.data.is_none() {
            self.data = Some(cx.data_dir());
        }
        // A chooser comes up over another app, which may have a copy open.
        if self.pick.is_none() {
            open_cache::trim(&self.home, open_cache::CAP);
        }
        if let Some(p) = self.servers_path() {
            self.servers = servers::load(&p);
        }
        self.look_at_places();
        let s = cx.settings();
        self.sort = Sort::parse(s.get_str(KEY_SORT).unwrap_or("name"));
        self.hidden = s.get_bool(KEY_HIDDEN).unwrap_or(false);
        // Side A opens at home, or where the chooser was asked to start;
        // B waits in Places.
        let first = self
            .spec()
            .and_then(|s| s.folder.clone())
            .filter(|f| f.is_dir())
            .unwrap_or_else(|| self.home.clone());
        self.sides[0].base = Loc::Local(first.clone());
        self.go(Loc::Local(first), cx);
    }

    /// A folder to show, or a file to show in its folder: another app
    /// opened it here (OpenDirectory, `gh0st open`).
    fn opened(&mut self, paths: &[PathBuf], cx: &mut Cx) {
        let Some(p) = paths.first() else {
            return;
        };
        let (dir, name) = if p.is_dir() {
            (p.clone(), None)
        } else {
            match (p.parent(), p.file_name()) {
                (Some(d), Some(n)) => (d.to_path_buf(), Some(n.to_string_lossy().into_owned())),
                _ => return,
            }
        };
        // A question the worker waits on, and a half filled form, stay:
        // the folder opens under them.
        if !matches!(self.view, View::Conflict { .. } | View::Server(_)) {
            self.view = View::Browse;
            self.armed = false;
        }
        let base = if dir.starts_with(&self.home) {
            self.home.clone()
        } else {
            PathBuf::from("/")
        };
        self.side_mut().base = Loc::Local(base);
        self.go(Loc::Local(dir), cx);
        if let Some(n) = name {
            self.side_mut().select_name(&n);
        }
        // A question that waited for the viewer.
        self.next_ask(cx);
        self.retitle();
    }

    fn bars(&self) -> Bars {
        match &self.view {
            View::Browse if self.pick.is_some() => self.pick_bars(),
            View::OpenWith { .. } => Bars::top().chip(6, Chip::new("pin", "Always")),
            View::Browse => {
                let s = self.side();
                if s.search.active() {
                    return Bars::top().search(&s.search);
                }
                if let Some(Place::Server(_)) = self.place_here() {
                    return Bars::top()
                        .chip(5, Chip::new("pencil", "Edit"))
                        .chip(7, Chip::new("trash-2", "Forget").danger().armed(self.armed));
                }
                if !self.in_folder() {
                    return Bars::top();
                }
                let copy = Chip::new("copy", "Copy");
                // Mark on 4: Esc itself still goes up a folder.
                let marked = s.current().is_some_and(|e| s.marks.contains(&e.name));
                let mark = Chip::new("check", "Mark").on(marked);
                if s.loc.is_archive() {
                    // The same keys as in a folder; nothing pastes into an
                    // archive or leaves it by Cut.
                    return if s.entries.is_empty() {
                        Bars::top()
                    } else {
                        Bars::top()
                            .chip(4, mark)
                            .chip(5, copy)
                            .chip(6, Chip::new("corner-down-left", "Paste").disabled(true))
                            .chip(7, Chip::new("x", "Cut").disabled(true))
                    };
                }
                let paste = Chip::new("corner-down-left", "Paste");
                let paste = if self.clip.is_some() {
                    paste.main()
                } else {
                    paste.disabled(true)
                };
                if s.entries.is_empty() {
                    // Nothing to take here; somewhere to paste.
                    return if self.clip.is_some() {
                        Bars::top().chip(6, paste)
                    } else {
                        Bars::top()
                    };
                }
                Bars::top()
                    .chip(4, mark)
                    .chip(5, copy)
                    .chip(6, paste)
                    .chip(7, Chip::new("x", "Cut"))
            }
            View::Server(_) => Bars::top().chip(6, Chip::new("plug-zap", "Connect").main()),
            View::Transfers if !self.transfers.is_empty() => {
                Bars::top().chip(6, Chip::new("x", "Stop").danger())
            }
            View::Key(None) => Bars::top().chip(
                6,
                Chip::new("key-round", "Make")
                    .main()
                    .busy(self.pending.values().any(|p| matches!(p, Pending::Key))),
            ),
            _ => Bars::top(),
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match &mut self.view {
            View::Browse if self.pick.is_some() && matches!(key, Key::Soft(_) | Key::Tab) => {
                if let Key::Soft(n) = key {
                    self.pick_soft(n, cx);
                }
            }
            View::Browse => self.browse_key(key, cx),
            View::OpenWith {
                path,
                handlers,
                list,
            } => {
                let always = key == Key::Soft(6);
                let at = match list.key(key) {
                    Some(Action::Open(i)) => Some(i),
                    _ if always => Some(list.selected()),
                    _ => None,
                };
                if let Some(h) = at.and_then(|i| handlers.get(i)).cloned() {
                    let path = path.clone();
                    if always {
                        if let Err(e) = cx.always_open_with(&path, &h.id) {
                            cx.toast_tone(Tone::Critical, &io_words(&e));
                        }
                    }
                    self.view = View::Browse;
                    cx.open_with(&h.id, &path);
                }
            }
            View::Recent { files, list } => {
                if let Some(Action::Open(i)) = list.key(key) {
                    let Some(path) = files.get(i).cloned() else {
                        return;
                    };
                    self.view = View::Browse;
                    if self.pick.is_some() {
                        self.finish(Some(vec![path]), cx);
                    } else {
                        cx.open_file(&path);
                    }
                }
            }
            View::Filters { list } => {
                if let Some(Action::Open(i)) = list.key(key) {
                    self.view = View::Browse;
                    self.set_filter(i);
                }
            }
            View::Key(None) if key == Key::Soft(6) => self.make_key(cx),
            View::Key(_) => {}
            View::Viewer { scroll, .. } => {
                if !scroll.key(key) {
                    let page = 6;
                    match key {
                        Key::Other(gh0st_app::codes::PAGEDOWN) => {
                            (0..page).for_each(|_| {
                                scroll.key(Key::Down);
                            });
                        }
                        Key::Other(gh0st_app::codes::PAGEUP) => {
                            (0..page).for_each(|_| {
                                scroll.key(Key::Up);
                            });
                        }
                        _ => {}
                    }
                }
            }
            View::Info { list, .. } => {
                list.key(key);
            }
            View::Server(_) => self.form_key(key, cx),
            View::Transfers => {
                if key == Key::Soft(6) {
                    let ids: Vec<Id> = self.transfers.iter().map(|t| t.id).collect();
                    for id in ids {
                        self.drop_asks(id, cx);
                        self.send(Request::Cancel { id }, cx);
                    }
                    self.transfers.clear();
                    cx.keep_awake(false);
                    cx.toast("Stopped");
                    self.view = View::Browse;
                }
            }
            View::Conflict { id, all, list, .. } => {
                let id = *id;
                let pick = match list.key(key) {
                    Some(Action::Open(3) | Action::Change(3, _)) => {
                        *all = !*all;
                        None
                    }
                    Some(Action::Open(i)) => ["replace", "both", "skip"].get(i).copied(),
                    _ => None,
                };
                if let Some(word) = pick {
                    let text = if *all {
                        format!("{word} all")
                    } else {
                        word.into()
                    };
                    self.view = View::Browse;
                    self.asking = None;
                    self.reply(id, Some(text), cx);
                    self.next_ask(cx);
                }
            }
        }
        self.retitle();
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        match &self.view {
            View::Browse => {}
            View::Conflict { id, .. } => {
                let id = *id;
                self.view = View::Browse;
                self.asking = None;
                self.reply(id, Some("skip".into()), cx);
                self.next_ask(cx);
                return false;
            }
            _ => {
                self.view = View::Browse;
                self.armed = false;
                self.retitle();
                // A question that waited for the form or the viewer.
                self.next_ask(cx);
                return false;
            }
        }
        if self.side().search.active() {
            // Back to the whole folder, the cursor on the same name.
            let keep = self.side().current().map(|e| e.name.clone());
            self.side_mut().search.cancel();
            if let Some(n) = keep {
                self.side_mut().select_name(&n);
            }
            return false;
        }
        if !self.side().marks.is_empty() {
            self.side_mut().marks.clear();
            return false;
        }
        if self.side().loc != Loc::Places {
            self.up(cx);
            return false;
        }
        true
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        self.answer_one(id, answer, cx);
        // A worker question held while this dialog was open.
        self.next_ask(cx);
        self.retitle();
    }

    /// Back in front: another app may have changed the folder (Text saved
    /// a file opened from here). Local folders read again; servers wait.
    fn front(&mut self, front: bool, cx: &mut Cx) {
        if front && self.pick.is_none() && matches!(self.view, View::Browse) {
            self.reload(&[], cx);
            cx.redraw();
        }
    }

    fn ready(&mut self, watch: Watch, cx: &mut Cx) {
        if self.engine_watch.is_some_and(|w| w.0 == watch) {
            self.drain(cx);
        } else if Some(watch) == self.kr_watch {
            let Some(prompt) = self.asking.as_ref().and_then(|a| a.prompt.clone()) else {
                self.kr.changed();
                return;
            };
            if self.kr.ended(&prompt).is_some() {
                // Opened, the keyring answers; dismissed, the user types it.
                let Some(a) = self.asking.take() else {
                    return;
                };
                self.ask_now(a.id, a.ask, cx);
            }
        }
        cx.redraw();
    }

    fn commands(&self, c: &mut Commands) {
        let browse = matches!(self.view, View::Browse);
        let folder = browse && self.in_folder();
        let s = self.side();
        let writable = folder && !s.loc.is_archive();
        let archive_row = s.current().is_some_and(|e| e.sort() == Type::Archive);
        if let Some(spec) = self.spec() {
            self.pick_commands(spec, browse, c);
            return;
        }
        let file_row = folder
            && matches!(s.loc, Loc::Local(_))
            && s.current().is_some_and(|e| e.kind == Kind::File);
        c.group("Files");
        c.add(
            Command::action(CMD_OPEN_WITH, "Open with")
                .keys("o")
                .opens()
                .enabled(file_row)
                .mode("Folder", file_row && !s.search.active()),
        );
        c.add(
            Command::action(CMD_COPY, "Copy")
                .keys("Ctrl+c")
                .enabled(folder)
                .mode("Folder", folder && !s.search.active()),
        );
        c.add(
            Command::action(CMD_CUT, "Cut")
                .keys("Ctrl+x")
                .enabled(writable)
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(
            Command::action(CMD_PASTE, "Paste here")
                .keys("Ctrl+v")
                .enabled(writable && self.clip.is_some())
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(
            Command::action(CMD_MARK, "Mark")
                .keys("space")
                .enabled(folder)
                .mode("Folder", folder && !s.search.active()),
        );
        c.add(
            Command::action(CMD_ALL, "Mark all")
                .keys("a")
                .enabled(folder)
                .mode("Folder", folder && !s.search.active()),
        );
        c.add(
            Command::action(CMD_FIND, "Find in folder")
                .keys("n")
                .enabled(folder)
                .mode("Folder", folder && !s.search.active()),
        );
        c.add(
            Command::action(CMD_SIDE, "Other side")
                .keys("tab")
                .enabled(browse)
                .mode("Folder", browse && !s.search.active()),
        );
        c.add(
            Command::action(CMD_PGUP, "A screen up")
                .keys("pgup")
                .keys_only()
                .repeats()
                .mode("Folder", browse && !s.search.active()),
        );
        c.add(
            Command::action(CMD_PGDN, "A screen down")
                .keys("pgdn")
                .keys_only()
                .repeats()
                .mode("Folder", browse && !s.search.active()),
        );
        c.add(
            Command::action(CMD_RENAME, "Rename")
                .keys("r")
                .enabled(writable)
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(
            Command::action(CMD_MKDIR, "New folder")
                .keys("k")
                .enabled(writable)
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(
            Command::action(CMD_DELETE, "Delete")
                .keys("del")
                .enabled(writable)
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(
            Command::action(CMD_INFO, "Info")
                .keys("i")
                .enabled(folder)
                .mode("Folder", folder && !s.search.active()),
        );
        c.add(
            Command::action(CMD_EXTRACT, "Extract here")
                .keys("e")
                .enabled(writable && archive_row)
                .mode("Folder", writable && !s.search.active()),
        );
        c.add(Command::action(CMD_COMPRESS, "Compress to zip").enabled(writable));
        c.add(Command::action(CMD_CLEAR, "Clear the clipboard").enabled(self.clip.is_some()));
        c.add(Command::action(CMD_SAME, "Same folder on both sides").enabled(browse));
        c.group("View");
        c.add(Command::choice(CMD_SORT, "Sort by", self.sort.word()));
        c.add(Command::toggle(CMD_HIDDEN, "Hidden files", self.hidden));
        // Not while a clash waits for its answer: that page must stay.
        let free = !matches!(self.view, View::Conflict { .. });
        c.add(
            Command::action(CMD_TRANSFERS, "Transfers")
                .keys("t")
                .opens()
                .enabled(free)
                .mode("Folder", browse && !s.search.active()),
        );
        c.group("Places");
        c.add(Command::action(CMD_ADD, "Add server").opens().enabled(free));
        c.add(Command::action(CMD_KEY, "SSH key").opens().enabled(free));
        c.add(Command::action(CMD_EMPTY, "Empty the Trash"));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        self.armed = false;
        match id {
            CMD_OPEN_WITH => self.open_with(cx),
            CMD_TYPE => self.show_filters(),
            CMD_MARK => self.mark(),
            CMD_ALL => self.mark_all(),
            CMD_FIND => {
                self.side_mut().search.start();
                self.side_mut().list.select(0);
            }
            CMD_SIDE => self.flip(),
            CMD_PGUP | CMD_PGDN => {
                let code = if id == CMD_PGUP {
                    gh0st_app::codes::PAGEUP
                } else {
                    gh0st_app::codes::PAGEDOWN
                };
                self.side_mut().list.key(Key::Other(code));
            }
            CMD_COPY => self.take(false, cx),
            CMD_CUT => self.take(true, cx),
            CMD_PASTE => self.paste(cx),
            CMD_CLEAR => self.clip = None,
            CMD_RENAME => self.rename(cx),
            CMD_MKDIR => self.mkdir(cx),
            CMD_DELETE => self.delete(cx),
            CMD_INFO => self.info(),
            CMD_EXTRACT => self.extract_here(cx),
            CMD_COMPRESS => self.compress(cx),
            CMD_SAME => {
                let (loc, base, archive) = {
                    let s = self.side();
                    (s.loc.clone(), s.base.clone(), s.archive.clone())
                };
                let o = 1 - self.cur;
                self.sides[o].loc = loc;
                self.sides[o].base = base;
                self.sides[o].archive = archive;
                self.sides[o].marks.clear();
                self.load(o, cx);
            }
            CMD_SORT => {
                let all = Sort::ALL;
                let i = all.iter().position(|s| *s == self.sort).unwrap_or(0);
                let d = if step == Step::Prev { -1 } else { 1 };
                self.sort = all[gh0st_app::cycle(i, d, all.len())];
                cx.settings()
                    .set_str(KEY_SORT, &self.sort.word().to_lowercase());
                self.resort();
            }
            CMD_HIDDEN => {
                self.hidden = !self.hidden;
                cx.settings().set_bool(KEY_HIDDEN, self.hidden);
                self.resort();
            }
            CMD_TRANSFERS => self.view = View::Transfers,
            CMD_ADD => self.open_form(Server::new(Proto::Sftp), None),
            CMD_KEY => self.view = View::Key(self.public_key()),
            CMD_EMPTY => {
                let n = std::fs::read_dir(self.trash())
                    .map(|r| r.count())
                    .unwrap_or(0);
                if n == 0 {
                    cx.toast("The Trash is empty");
                    return;
                }
                cx.ask(
                    ASK_EMPTY,
                    Dialog::confirm(
                        &format!("Empty the Trash, {}?", words::items(n)),
                        "This can't be undone.",
                    )
                    .yes("empty")
                    .danger()
                    .hold(),
                );
            }
            _ => {}
        }
        self.retitle();
    }

    fn draw(&self, ui: &mut Ui) {
        match &self.view {
            View::Browse => self.draw_browse(ui),
            View::OpenWith {
                path,
                handlers,
                list,
            } => {
                let name = path.file_name().map(|n| n.to_string_lossy());
                ui.note(&name.unwrap_or_default());
                let rows: Vec<Row> = handlers
                    .iter()
                    .map(|h| {
                        let r = if h.default {
                            Row::info(&h.name, "default")
                        } else {
                            Row::text(&h.name)
                        };
                        r.icon(h.icon)
                    })
                    .collect();
                ui.list(list, &rows);
            }
            View::Recent { files, list } => {
                let names: Vec<(String, String)> = files
                    .iter()
                    .map(|p| {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                        let dir = p.parent().map(|d| self.short(d)).unwrap_or_default();
                        (name.unwrap_or_default(), dir)
                    })
                    .collect();
                let rows: Vec<Row> = names
                    .iter()
                    .map(|(n, d)| Row::info(n, d).icon("file"))
                    .collect();
                ui.list(list, &rows);
            }
            View::Filters { list } => {
                let spec = self.spec();
                let at = spec.and_then(|s| s.filter);
                let rows: Vec<Row> = spec
                    .map(|s| s.filters.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .enumerate()
                    .map(|(i, f)| Row::pick(&f.name, at == Some(i)))
                    .collect();
                ui.list(list, &rows);
            }
            View::Viewer { text, scroll, .. } => ui.scroll_text(scroll, text),
            View::Info { rows, list } => {
                // The name whole, wrapped, above the rest.
                if let Some((_, name)) = rows.first() {
                    ui.note(name);
                }
                let r: Vec<Row> = rows.iter().skip(1).map(|(k, v)| Row::info(k, v)).collect();
                ui.list(list, &r);
            }
            View::Server(f) => {
                let address = f.s.address();
                let pw = match (&f.password, f.s.remember) {
                    (Some(_), _) => "set",
                    (None, true) => "ask once",
                    (None, false) => "ask",
                };
                let name = if f.s.name.is_empty() {
                    f.s.host.as_str()
                } else {
                    f.s.name.as_str()
                };
                let none = |s: &str| if s.is_empty() { "none" } else { "" };
                let rows = [
                    Row::choice("Type", f.s.proto.word()).icon("server"),
                    Row::link("Address")
                        .value(if address.is_empty() { "none" } else { &address })
                        .icon("globe"),
                    Row::link("User")
                        .value(if f.s.user.is_empty() {
                            none(&f.s.user)
                        } else {
                            &f.s.user
                        })
                        .icon("user"),
                    Row::link("Password").value(pw).icon("key-round"),
                    Row::toggle("Remember it", f.s.remember).icon("lock"),
                    Row::link("Folder")
                        .value(if f.s.path == "~" { "home" } else { &f.s.path })
                        .icon("folder"),
                    Row::link("Name")
                        .value(if name.is_empty() { "none" } else { name })
                        .icon("tag"),
                ];
                ui.list(&f.list, &rows);
            }
            View::Key(Some(k)) => {
                ui.qr(k, &[("Key", "ed25519"), ("Add it to", "authorized_keys")]);
            }
            View::Key(None) => ui.notice(
                "key-round",
                None,
                "No SSH key yet",
                "With one, servers that know it let Files in with no password.",
                &[],
            ),
            View::Transfers => {
                if self.transfers.is_empty() {
                    ui.message("check", "Nothing under way");
                    return;
                }
                // One shows whole; the rest wait their turn.
                let waiting = self.transfers.len() - 1;
                if waiting > 0 {
                    ui.note(&format!("{} more waiting", waiting));
                }
                for t in self.transfers.iter().take(1) {
                    let pct = (t.total > 0).then(|| (t.done * 100 / t.total) as u8);
                    ui.text(&format!("{} {}", t.verb, t.what));
                    ui.note(&format!("To {}", t.to));
                    let label = if t.name.is_empty() {
                        "Starting".to_string()
                    } else {
                        t.name.clone()
                    };
                    ui.progress(&label, pct, 0);
                    if t.total > 0 {
                        ui.note(&format!(
                            "{} of {}",
                            model::size_text(t.done),
                            model::size_text(t.total)
                        ));
                    }
                }
            }
            View::Conflict {
                name, all, list, ..
            } => {
                let n = name.chars().count();
                let short = if n > 22 {
                    let head: String = name.chars().take(21).collect();
                    format!("{head}\u{2026}")
                } else {
                    name.clone()
                };
                ui.note(&format!("\u{201c}{short}\u{201d} is there."));
                let rows = [
                    Row::text("Replace it").icon("copy"),
                    Row::text("Keep both").icon("copy-x"),
                    Row::text("Skip it").icon("skip-forward"),
                    Row::toggle("Same for the rest", *all).icon("check-check"),
                ];
                ui.list(list, &rows);
            }
        }
    }
}

impl Files {
    fn answer_one(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        let yes = match answer {
            Answer::Yes(t) => Some(t),
            Answer::No | Answer::Also => None,
        };
        if matches!(id, ASK_DELETE | ASK_RENAME | ASK_COMPRESS) && yes.is_none() {
            self.target = None;
        }
        match (id, yes) {
            (ASK_SAVE, Some(name)) => {
                if let Err(why) = model::name_ok(&name) {
                    let d = Dialog::text("Save as", "name").value(&name).error(why);
                    cx.ask(ASK_SAVE, d.yes("save"));
                    return;
                }
                let Some(dir) = self.here() else {
                    return;
                };
                let path = dir.join(&name);
                if path.is_dir() {
                    let d = Dialog::text("Save as", "name")
                        .value(&name)
                        .error("A folder has that name");
                    cx.ask(ASK_SAVE, d.yes("save"));
                } else if std::fs::symlink_metadata(&path).is_ok() {
                    self.ask_replace(path, cx);
                } else {
                    self.finish(Some(vec![path]), cx);
                }
            }
            (ASK_REPLACE, Some(_)) => {
                if let Some(path) = self.replacing.take() {
                    self.finish(Some(vec![path]), cx);
                }
            }
            (ASK_REPLACE, None) => self.replacing = None,
            (ASK_DELETE, Some(_)) => {
                // The names the dialog said, not what the cursor is on now.
                let Some((loc, names)) = self.take_target() else {
                    cx.toast("The folder changed: nothing deleted");
                    return;
                };
                let items: Vec<Loc> = names.iter().map(|n| loc.join(n)).collect();
                let trash = !(loc.is_remote() || self.in_trash());
                self.side_mut().marks.clear();
                let done = if trash {
                    "Moved to the Trash"
                } else {
                    "Deleted"
                };
                self.op(
                    |id| Request::Delete { id, items, trash },
                    done,
                    vec![loc],
                    cx,
                );
            }
            (ASK_RENAME, Some(name)) => {
                if let Err(why) = model::name_ok(&name) {
                    cx.ask(
                        ASK_RENAME,
                        Dialog::text("Rename", "name").value(&name).error(why),
                    );
                    return;
                }
                let Some((loc, names)) = self.take_target() else {
                    cx.toast("The folder changed: nothing renamed");
                    return;
                };
                let Some(old) = names.into_iter().next() else {
                    return;
                };
                if old == name {
                    return;
                }
                let item = loc.join(&old);
                self.op(
                    |id| Request::Rename { id, item, name },
                    "Renamed",
                    vec![loc],
                    cx,
                );
            }
            (ASK_MKDIR, Some(name)) => {
                if let Err(why) = model::name_ok(&name) {
                    cx.ask(
                        ASK_MKDIR,
                        Dialog::text("New folder", "name").value(&name).error(why),
                    );
                    return;
                }
                let at = self.side().loc.clone();
                let touched = vec![at.clone()];
                self.op(
                    |id| Request::Mkdir { id, at, name },
                    "Folder made",
                    touched,
                    cx,
                );
            }
            (ASK_COMPRESS, Some(name)) => {
                if let Err(why) = model::name_ok(&name) {
                    cx.ask(
                        ASK_COMPRESS,
                        Dialog::text("Compress to zip", "name")
                            .value(&name)
                            .error(why),
                    );
                    return;
                }
                let Some((loc, names)) = self.take_target() else {
                    cx.toast("The folder changed: nothing compressed");
                    return;
                };
                let items: Vec<Loc> = names.iter().map(|n| loc.join(n)).collect();
                let to = loc.join(&name);
                let id = self.id();
                self.transfers.push(Transfer {
                    id,
                    verb: "Compressing",
                    what: format!("\u{201c}{name}\u{201d}"),
                    to: self.where_(self.cur),
                    done: 0,
                    total: 0,
                    name: String::new(),
                });
                self.pending.insert(id, Pending::Transfer(vec![loc]));
                self.side_mut().marks.clear();
                cx.keep_awake(true);
                self.send(Request::Compress { id, items, to }, cx);
            }
            (ASK_EMPTY, Some(_)) => {
                let trash = self.trash();
                let items = std::fs::read_dir(&trash)
                    .map(|rd| {
                        rd.filter_map(Result::ok)
                            .map(|d| Loc::Local(d.path()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                self.op(
                    |id| Request::Delete {
                        id,
                        items,
                        trash: false,
                    },
                    "The Trash is empty",
                    vec![Loc::Local(trash)],
                    cx,
                );
            }
            (ASK_PASSWORD | ASK_HOSTKEY, text) => {
                let Some(a) = self.asking.take() else {
                    return;
                };
                if id == ASK_PASSWORD {
                    if let (Ask::Password { server, .. }, Some(pw)) = (&a.ask, &text) {
                        self.typed.insert(server.clone(), pw.clone());
                        self.typed_for.insert(a.id, server.clone());
                    }
                }
                let text = match id {
                    ASK_HOSTKEY => text.map(|_| "yes".to_string()),
                    _ => text,
                };
                self.reply(a.id, text, cx);
                self.next_ask(cx);
            }
            (ASK_ADDRESS..=ASK_NAME, Some(text)) => {
                let View::Server(f) = &mut self.view else {
                    return;
                };
                match id {
                    ASK_ADDRESS => {
                        if let Err(why) = f.s.set_address(&text) {
                            cx.ask(
                                ASK_ADDRESS,
                                Dialog::text("Address", "nas.local").value(&text).error(why),
                            );
                        }
                    }
                    ASK_USER => f.s.user = text.trim().to_string(),
                    ASK_SECRET => f.password = (!text.is_empty()).then_some(text),
                    ASK_FOLDER => {
                        let t = text.trim();
                        f.s.path = if t.starts_with('/') || t == "~" {
                            t.to_string()
                        } else if t.is_empty() {
                            f.s.proto.start().to_string()
                        } else {
                            format!("/{t}")
                        };
                    }
                    _ => f.s.name = text.trim().to_string(),
                }
            }
            _ => {}
        }
    }
}

impl Files {
    fn draw_browse(&self, ui: &mut Ui) {
        let s = self.side();
        if s.loc == Loc::Places {
            let places = self.places();
            let labels: Vec<(String, String, &str)> = places
                .iter()
                .map(|p| match p {
                    Place::Home => ("Home".into(), String::new(), "house"),
                    Place::Card => ("Card".into(), String::new(), "hard-drive"),
                    Place::Downloads => ("Downloads".into(), String::new(), "download"),
                    Place::Recent => ("Recent".into(), String::new(), "clock"),
                    Place::Trash => {
                        let n = self.trash_n;
                        let v = if n == 0 {
                            "empty".into()
                        } else {
                            n.to_string()
                        };
                        ("Trash".into(), v, "trash-2")
                    }
                    Place::Server(i) => {
                        let srv = &self.servers[*i];
                        let icon = match srv.proto {
                            Proto::Dav | Proto::Davs => "cloud",
                            _ => "server",
                        };
                        (srv.name.clone(), srv.proto.word().into(), icon)
                    }
                    Place::Add => ("Add server".into(), String::new(), "plus"),
                })
                .collect();
            let rows: Vec<Row> = labels
                .iter()
                .zip(&places)
                .map(|((label, value, icon), p)| {
                    let row = if value.is_empty() {
                        Row::link(label)
                    } else {
                        Row::link(label).value(value)
                    };
                    match p {
                        Place::Server(_) if self.armed && self.place_here() == Some(p.clone()) => {
                            Row::tag(label, "Press again", Tone::Warning)
                                .icon(icon)
                                .plain()
                        }
                        Place::Server(i) if self.servers[*i].proto.plain() => {
                            Row::tag(label, self.servers[*i].proto.word(), Tone::Warning)
                                .icon(icon)
                                .plain()
                        }
                        _ => row.icon(icon).plain(),
                    }
                })
                .collect();
            ui.list(&s.list, &rows);
            return;
        }
        match &s.state {
            State::Loading(_) => {
                let what = match &s.loc {
                    Loc::Archive { .. } => "Opening the archive".to_string(),
                    Loc::Remote { server, .. } => format!("Connecting to {server}"),
                    _ => "Reading".to_string(),
                };
                ui.message("refresh-cw", &what);
                return;
            }
            State::Failed(why) => {
                let what = if s.loc.is_remote() {
                    "Can't reach the server"
                } else {
                    "Can't open this folder"
                };
                ui.notice(
                    "triangle-alert",
                    Some(Tone::Critical),
                    what,
                    why,
                    &[("Esc", "back")],
                );
                return;
            }
            State::Ready => {}
        }
        let shown = s.shown();
        let up = Row::text("..").icon("folder-up").plain();
        if s.entries.is_empty() {
            ui.list(&s.list, &[up]);
            ui.message("folder-open", "Nothing here");
            return;
        }
        if shown.is_empty() {
            ui.message("search", "No name matches");
            return;
        }
        let sizes: Vec<String> = shown
            .iter()
            .map(|&i| {
                let e = &s.entries[i];
                if e.kind.is_dir() {
                    String::new()
                } else {
                    model::size_text(e.size)
                }
            })
            .collect();
        let mut rows: Vec<Row> = Vec::with_capacity(shown.len() + 2);
        if s.up_row() {
            rows.push(up);
        }
        rows.extend(shown.iter().zip(&sizes).map(|(&i, size)| {
            let e = &s.entries[i];
            let marked = s.marks.contains(&e.name);
            let icon = match e.kind {
                Kind::Link => "link",
                _ => e.sort().icon(),
            };
            let row = if e.kind.is_dir() {
                Row::link(&e.name)
            } else {
                Row::info(&e.name, size)
            };
            // Cut, waiting for Paste: grey until it moves.
            let cut = self
                .clip
                .as_ref()
                .is_some_and(|c| c.cut && c.from == s.loc && c.names.contains(&e.name));
            let row = row.icon(icon).marked(marked).plain();
            if cut { row.dim() } else { row }
        }));
        if s.more && !s.search.active() {
            rows.push(
                Row::text("The rest is not shown")
                    .icon("info")
                    .dim()
                    .plain(),
            );
        }
        ui.list(&s.list, &rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_lists_one_folder_at_a_time() {
        let all = vec![
            Entry::file("docs/a.txt", 3),
            Entry::file("docs/deep/b.txt", 4),
            Entry::dir("docs/"),
            Entry::file("top.md", 1),
            Entry::file("implied/c.txt", 1),
        ];
        let names = |v: Vec<Entry>| v.into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(inside(&all, "")), ["docs", "top.md", "implied"]);
        assert_eq!(names(inside(&all, "docs")), ["a.txt", "deep"]);
        assert_eq!(inside(&all, "implied")[0].size, 1);
    }

    #[test]
    fn archive_stems_drop_the_whole_extension() {
        assert_eq!(archive_stem("photos.tar.gz"), "photos");
        assert_eq!(archive_stem("Backup.ZIP"), "Backup");
        assert_eq!(archive_stem("notes"), "notes");
    }
}
