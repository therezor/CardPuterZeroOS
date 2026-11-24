//! The Files page: two sides of one pane, Places at each side's root, and
//! the views a side opens (a text file, a file's info, a server's form, the
//! transfers).

use crate::engine::{Ask, Engine, Event, Id, Request};
use crate::model::{self, Entry, Kind, Loc, Sort, Type};
use crate::servers::{self, Proto, Server};
use crate::words;
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
const CMD_MOVE: u32 = 18;
const CMD_PGUP: u32 = 19;
const CMD_PGDN: u32 = 20;
const CMD_KEY: u32 = 21;

// Dialogs.
const ASK_COPY: u32 = 1;
const ASK_MOVE: u32 = 2;
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

const KEY_SORT: &str = "files.sort";
const KEY_HIDDEN: &str = "files.hidden";

/// What a page needs from outside: tests and mockups pass their own.
pub struct Options {
    pub home: PathBuf,
    /// The app's data folder; None takes `cx.data_dir()`.
    pub data: Option<PathBuf>,
    pub engine: Box<dyn Engine>,
    pub keyring: Keyring,
}

impl Options {
    pub fn system() -> Options {
        Options {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| "/".into()),
            data: None,
            engine: Box::new(crate::child::Child::new()),
            keyring: Keyring::connect(),
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

    fn current(&self) -> Option<&Entry> {
        let shown = self.shown();
        shown
            .get(self.list.selected())
            .and_then(|&i| self.entries.get(i))
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

    fn select_name(&mut self, name: &str) {
        if let Some(i) = self
            .shown()
            .iter()
            .position(|&i| self.entries[i].name == name)
        {
            self.list.select(i);
        }
    }
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

/// What a request was for, so its events land in the right place.
#[derive(Clone, Debug)]
enum Pending {
    List(usize),
    Contents(usize),
    /// A change on a side's folder: reload both when it ends.
    Op(&'static str),
    Transfer,
    /// Making the SSH key.
    Key,
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
            queued: VecDeque::new(),
            armed: false,
            title: String::new(),
            status: String::new(),
            has_downloads: false,
            trash_n: 0,
        };
        f.retitle();
        f
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

    /// "B: nas:/backup", for a dialog naming where things go.
    fn where_(&self, i: usize) -> String {
        format!(
            "{}: {}",
            Files::letter(i),
            self.sides[i].loc.show(&self.home)
        )
    }

    fn retitle(&mut self) {
        let shown = self.side().loc.show(&self.home);
        // The end of a path says more than its start.
        let n = shown.chars().count();
        let shown = if n > 22 {
            let tail: String = shown.chars().skip(n - 21).collect();
            format!("\u{2026}{tail}")
        } else {
            shown
        };
        self.title = format!("{} \u{00b7} {shown}", Files::letter(self.cur));
        self.status = match self.transfers.first() {
            Some(t) if t.total > 0 => format!("{}%", t.done * 100 / t.total),
            Some(_) => "\u{2026}".into(),
            None if self.plain_here() => "Not private".into(),
            None => String::new(),
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
        let s = self.side_mut();
        s.loc = loc;
        s.marks.clear();
        s.search.cancel();
        s.list.select(0);
        if !s.loc.is_archive() {
            s.archive = None;
        }
        self.armed = false;
        self.load(self.cur, cx);
        self.retitle();
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
        let Some(e) = self.side().current().cloned() else {
            return;
        };
        let loc = self.side().loc.join(&e.name);
        if e.kind.is_dir() {
            self.go(loc, cx);
            return;
        }
        match e.sort() {
            Type::Archive if !self.side().loc.is_archive() => {
                let inside = Loc::Archive {
                    file: Box::new(loc.clone()),
                    inner: String::new(),
                };
                self.side_mut().loc = inside;
                self.side_mut().list.select(0);
                self.side_mut().marks.clear();
                self.side_mut().search.cancel();
                let id = self.id();
                self.side_mut().state = State::Loading(id);
                self.pending.insert(id, Pending::Contents(self.cur));
                self.send(Request::Contents { id, archive: loc }, cx);
                self.retitle();
            }
            t if t.readable() => self.view_text(&e, &loc, cx),
            _ => self.info(),
        }
    }

    fn view_text(&mut self, e: &Entry, loc: &Loc, cx: &mut Cx) {
        let Loc::Local(path) = loc else {
            cx.toast("Copy it to the card to read it");
            return;
        };
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
                let r = model::list_local(p);
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                match r {
                    Ok((all, more)) => {
                        s.entries = shape(all, sort, hidden);
                        s.more = more;
                        s.state = State::Ready;
                    }
                    Err(err) => {
                        s.entries.clear();
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
                self.pending.insert(id, Pending::List(i));
                self.send(Request::List { id, at: loc }, cx);
            }
            Loc::Archive { inner, .. } => {
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                if let Some(all) = &s.archive {
                    s.entries = shape(inside(all, inner), sort, hidden);
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

    fn reload_all(&mut self, cx: &mut Cx) {
        for i in 0..2 {
            let keep = self.sides[i].current().map(|e| e.name.clone());
            self.sides[i].marks.clear();
            self.load(i, cx);
            if let Some(n) = keep {
                self.sides[i].select_name(&n);
            }
        }
    }

    fn resort(&mut self) {
        let (sort, hidden) = (self.sort, self.hidden);
        for s in &mut self.sides {
            if let Loc::Local(p) = &s.loc {
                if let Ok((all, more)) = model::list_local(p) {
                    s.entries = shape(all, sort, hidden);
                    s.more = more;
                }
            } else {
                let v = std::mem::take(&mut s.entries);
                s.entries = shape(v, sort, hidden);
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
                    let mut v = std::mem::take(&mut self.sides[i].entries);
                    v.extend(entries);
                    self.sides[i].entries = v;
                }
            }
            (Pending::List(i), Event::Listed { .. }) => {
                self.pending.remove(&id);
                self.listed(id, i);
                let (sort, hidden) = (self.sort, self.hidden);
                let s = &mut self.sides[i];
                if s.state == State::Loading(id) {
                    s.entries = shape(std::mem::take(&mut s.entries), sort, hidden);
                    s.state = State::Ready;
                }
            }
            (Pending::Contents(i), Event::Entries { entries, .. }) => {
                self.sides[i]
                    .archive
                    .get_or_insert_with(Vec::new)
                    .extend(entries);
            }
            (Pending::Contents(i), Event::Listed { .. }) => {
                self.pending.remove(&id);
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i].state = State::Ready;
                    self.load(i, cx);
                }
            }
            (Pending::List(i) | Pending::Contents(i), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                if self.sides[i].state == State::Loading(id) {
                    self.sides[i].state = State::Failed(text);
                }
            }
            (_, Event::Ask { ask, .. }) => self.ask(id, ask, cx),
            (Pending::List(i), Event::Resolved { path, .. }) => {
                // "~" was the server's home: the side and its base go there.
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
            (
                Pending::Transfer,
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
            (Pending::Transfer, Event::Done { .. }) => {
                self.pending.remove(&id);
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
                self.reload_all(cx);
            }
            (Pending::Transfer, Event::Error { text, .. }) => {
                self.pending.remove(&id);
                let shown = self.transfers.iter().any(|t| t.id == id);
                self.transfers.retain(|t| t.id != id);
                cx.keep_awake(!self.transfers.is_empty());
                // Stop already said so.
                if shown || text != "Stopped" {
                    cx.toast_tone(Tone::Critical, &text);
                }
                self.reload_all(cx);
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
                cx.toast_tone(Tone::Critical, &text);
            }
            (Pending::Op(done), Event::Done { .. }) => {
                self.pending.remove(&id);
                cx.toast_tone(Tone::Ok, done);
                self.reload_all(cx);
            }
            (Pending::Op(_), Event::Error { text, .. }) => {
                self.pending.remove(&id);
                cx.toast_tone(Tone::Critical, &text);
                self.reload_all(cx);
            }
            _ => {}
        }
    }

    /// A listing on side `i` worked: a password typed for its server is
    /// now known good. Others' wait for their own.
    fn listed(&mut self, id: Id, i: usize) {
        self.tried_unlock.remove(&id);
        let Some(srv) = self.server_of(&self.sides[i].loc).cloned() else {
            return;
        };
        let Some(pw) = self.typed.remove(&srv.name) else {
            return;
        };
        if srv.remember {
            let attrs = srv.attrs();
            let a: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let _ = self.kr.store(&srv.label(), &a, pw.as_bytes(), true);
        }
        pw.into_bytes().fill(0);
    }

    // ---- questions ----

    /// A question from the worker. One shows at a time; the rest wait.
    fn ask(&mut self, id: Id, ask: Ask, cx: &mut Cx) {
        if self.asking.is_some() {
            self.queued.push_back((id, ask));
            return;
        }
        self.ask_now(id, ask, cx);
    }

    /// The next waiting question, once the last is answered.
    fn next_ask(&mut self, cx: &mut Cx) {
        if self.asking.is_none() {
            if let Some((id, ask)) = self.queued.pop_front() {
                self.ask_now(id, ask, cx);
            }
        }
    }

    /// Answers a password question from the keyring. False when the user
    /// has to type it; true when answered, or waiting on the keyring's
    /// prompt.
    fn password_from_keyring(&mut self, id: Id, ask: &Ask, srv: &Server, cx: &mut Cx) -> bool {
        let attrs = srv.attrs();
        let a: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // Twice at most: once, and again after an unlock.
        for _ in 0..2 {
            match self.kr.lookup(&a) {
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
                    if self.password_from_keyring(id, &ask, srv, cx) {
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
                let a = [("xdg:schema", servers::SCHEMA), ("key", "ssh")];
                let pw = match self.kr.lookup(&a) {
                    Lookup::Found(s) => s.as_str().map(String::from),
                    _ => None,
                };
                self.reply(id, pw, cx);
                return;
            }
            Ask::Conflict { name } => {
                self.view = View::Conflict {
                    id,
                    name: name.clone(),
                    all: false,
                    list: List::new(),
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

    fn items(&self, names: &[String]) -> Vec<Loc> {
        names.iter().map(|n| self.side().loc.join(n)).collect()
    }

    fn in_folder(&self) -> bool {
        self.side().loc != Loc::Places && self.side().state == State::Ready
    }

    fn mark(&mut self) {
        if !self.in_folder() {
            return;
        }
        let s = self.side_mut();
        let Some(name) = s.current().map(|e| e.name.clone()) else {
            return;
        };
        if !s.marks.remove(&name) {
            s.marks.insert(name);
        }
        let n = s.shown().len();
        let i = s.list.selected();
        s.list.select((i + 1).min(n.saturating_sub(1)));
    }

    fn mark_all(&mut self) {
        let s = self.side_mut();
        if s.marks.len() == s.entries.len() {
            s.marks.clear();
        } else {
            s.marks = s.entries.iter().map(|e| e.name.clone()).collect();
        }
    }

    fn copy(&mut self, remove: bool, cx: &mut Cx) {
        if !self.in_folder() {
            return;
        }
        let names = self.side().picked();
        if names.is_empty() {
            return;
        }
        let other = 1 - self.cur;
        let to = &self.sides[other].loc;
        if *to == Loc::Places || to.is_archive() {
            cx.toast(&format!(
                "Open a folder on side {} first",
                Files::letter(other)
            ));
            return;
        }
        if *to == self.side().loc {
            cx.toast("Both sides show this folder");
            return;
        }
        if remove && self.side().loc.is_archive() {
            cx.toast("An archive can't be changed: Copy takes from it");
            return;
        }
        let verb = if remove { "Move" } else { "Copy" };
        let title = format!("{verb} {}?", words::what(&names));
        let line = format!("To {}", self.where_(other));
        let id = if remove { ASK_MOVE } else { ASK_COPY };
        cx.ask(id, Dialog::confirm(&title, &line).yes(&verb.to_lowercase()));
    }

    fn start_copy(&mut self, remove: bool, cx: &mut Cx) {
        let names = self.side().picked();
        let other = 1 - self.cur;
        let to = self.sides[other].loc.clone();
        let id = self.id();
        let what = words::items(names.len());
        let req = match &self.side().loc {
            Loc::Archive { file, inner } => {
                let entries = names
                    .iter()
                    .map(|n| {
                        if inner.is_empty() {
                            n.clone()
                        } else {
                            format!("{inner}/{n}")
                        }
                    })
                    .collect();
                Request::Extract {
                    id,
                    archive: (**file).clone(),
                    entries,
                    to: to.clone(),
                }
            }
            _ => Request::Copy {
                id,
                items: self.items(&names),
                to: to.clone(),
                remove,
            },
        };
        let verb = match (&req, remove) {
            (Request::Extract { .. }, _) => "Extracting",
            (_, true) => "Moving",
            _ => "Copying",
        };
        self.transfers.push(Transfer {
            id,
            verb,
            what,
            to: self.where_(other),
            done: 0,
            total: 0,
            name: String::new(),
        });
        self.pending.insert(id, Pending::Transfer);
        self.side_mut().marks.clear();
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
        let d = if for_good {
            Dialog::confirm(&format!("Delete {what}?"), "This can't be undone.")
                .yes("delete")
                .danger()
                .hold()
        } else {
            Dialog::confirm(
                &format!("Move {what} to the Trash?"),
                "Empty the Trash to free the space.",
            )
            .yes("delete")
        };
        cx.ask(ASK_DELETE, d);
    }

    fn in_trash(&self) -> bool {
        matches!(&self.side().loc, Loc::Local(p) if p.starts_with(self.trash()))
    }

    fn op(&mut self, req_of: impl FnOnce(Id) -> Request, done: &'static str, cx: &mut Cx) {
        let id = self.id();
        self.pending.insert(id, Pending::Op(done));
        self.send(req_of(id), cx);
    }

    fn rename(&mut self, cx: &mut Cx) {
        if !self.in_folder() || self.side().loc.is_archive() {
            return;
        }
        let Some(e) = self.side().current() else {
            return;
        };
        let d = Dialog::text("Rename", "name").value(&e.name).yes("rename");
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
        self.pending.insert(id, Pending::Transfer);
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
        cx.ask(
            ASK_COMPRESS,
            Dialog::text("Compress to zip", "name")
                .value(&format!("{stem}.zip"))
                .yes("compress"),
        );
    }

    // ---- servers ----

    fn open_form(&mut self, s: Server, editing: Option<usize>) {
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
        if let Ok((open, _)) = self.kr.search(&a) {
            for item in open {
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
            5 => self.mark(),
            6 if !archive => self.copy(false, cx),
            7 if archive => self.copy(false, cx),
            7 => self.copy(true, cx),
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
        if self.status.is_empty() {
            return None;
        }
        let tone = if self.transfers.is_empty() {
            Tone::Warning
        } else {
            Tone::Info
        };
        Some((&self.status, tone))
    }

    fn typing(&self) -> bool {
        matches!(self.view, View::Browse) && self.side().search.active()
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.data.is_none() {
            self.data = Some(cx.data_dir());
        }
        if let Some(p) = self.servers_path() {
            self.servers = servers::load(&p);
        }
        self.look_at_places();
        let s = cx.settings();
        self.sort = Sort::parse(s.get_str(KEY_SORT).unwrap_or("name"));
        self.hidden = s.get_bool(KEY_HIDDEN).unwrap_or(false);
        // Side A opens at home; B waits in Places.
        self.sides[0].base = Loc::Local(self.home.clone());
        self.go(Loc::Local(self.home.clone()), cx);
    }

    fn bars(&self) -> Bars {
        match &self.view {
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
                if !self.in_folder() || s.entries.is_empty() {
                    return Bars::top();
                }
                let marked = s.current().is_some_and(|e| s.marks.contains(&e.name));
                let mark = Chip::new("check", "Mark").on(marked);
                let copy = Chip::new("copy", "Copy");
                if s.loc.is_archive() {
                    // Nothing moves out of an archive: Copy takes 7.
                    Bars::top().chip(5, mark).chip(7, copy)
                } else {
                    Bars::top()
                        .chip(5, mark)
                        .chip(6, copy)
                        .chip(7, Chip::new("arrow-right", "Move"))
                }
            }
            View::Server(_) => Bars::top().chip(6, Chip::new("plug-zap", "Connect").main()),
            View::Transfers if !self.transfers.is_empty() => {
                Bars::top().chip(6, Chip::new("x", "Stop"))
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
            View::Browse => self.browse_key(key, cx),
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
                self.retitle();
                return false;
            }
        }
        if self.side_mut().search.cancel() {
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
        let yes = match answer {
            Answer::Yes(t) => Some(t),
            Answer::No => None,
        };
        match (id, yes) {
            (ASK_COPY, Some(_)) => self.start_copy(false, cx),
            (ASK_MOVE, Some(_)) => self.start_copy(true, cx),
            (ASK_DELETE, Some(_)) => {
                let names = self.side().picked();
                let items = self.items(&names);
                let trash = !(self.side().loc.is_remote() || self.in_trash());
                self.side_mut().marks.clear();
                let done = if trash {
                    "Moved to the Trash"
                } else {
                    "Deleted"
                };
                self.op(|id| Request::Delete { id, items, trash }, done, cx);
            }
            (ASK_RENAME, Some(name)) => {
                if let Err(why) = model::name_ok(&name) {
                    cx.ask(
                        ASK_RENAME,
                        Dialog::text("Rename", "name").value(&name).error(why),
                    );
                    return;
                }
                let Some(old) = self.side().current().map(|e| e.name.clone()) else {
                    return;
                };
                if old == name {
                    return;
                }
                let item = self.side().loc.join(&old);
                self.op(|id| Request::Rename { id, item, name }, "Renamed", cx);
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
                self.op(|id| Request::Mkdir { id, at, name }, "Folder made", cx);
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
                let names = self.side().picked();
                let items = self.items(&names);
                let to = self.side().loc.join(&name);
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
                self.pending.insert(id, Pending::Transfer);
                self.side_mut().marks.clear();
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
        self.retitle();
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
        c.group("Files");
        c.add(
            Command::action(CMD_MARK, "Mark")
                .keys("space")
                .keys_only()
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
        c.add(Command::action(CMD_COPY, "Copy to the other side").enabled(folder));
        c.add(Command::action(CMD_MOVE, "Move to the other side").enabled(writable));
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
        c.add(Command::action(CMD_SAME, "Same folder on both sides").enabled(browse));
        c.group("View");
        c.add(Command::choice(CMD_SORT, "Sort by", self.sort.word()));
        c.add(Command::toggle(CMD_HIDDEN, "Hidden files", self.hidden));
        c.add(
            Command::action(CMD_TRANSFERS, "Transfers")
                .keys("t")
                .opens()
                .mode("Folder", browse && !s.search.active()),
        );
        c.group("Places");
        c.add(Command::action(CMD_ADD, "Add server").opens());
        c.add(Command::action(CMD_KEY, "SSH key").opens());
        c.add(Command::action(CMD_EMPTY, "Empty the Trash"));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        match id {
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
            CMD_COPY => self.copy(false, cx),
            CMD_MOVE => self.copy(true, cx),
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
            View::Viewer { text, scroll, .. } => ui.scroll_text(scroll, text),
            View::Info { rows, list } => {
                let r: Vec<Row> = rows.iter().map(|(k, v)| Row::info(k, v)).collect();
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
                for t in self.transfers.iter().take(2) {
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
        if s.entries.is_empty() {
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
        let mut rows: Vec<Row> = shown
            .iter()
            .zip(&sizes)
            .map(|(&i, size)| {
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
                row.icon(icon).marked(marked).plain()
            })
            .collect();
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
