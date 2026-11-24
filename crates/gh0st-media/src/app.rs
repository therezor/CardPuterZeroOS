//! Media: one page in five views.
//!
//! - **Start**: the places, Camera to Home.
//! - **Grid**: a folder as tiles, subfolders first.
//! - **Viewer**: a picture on the whole screen, fitted or filling it,
//!   zoomed to its own pixels, panned a third at a time.
//! - **Player**: a video or a moving picture on the whole screen.
//! - **Music**: a song with its cover, and the folder as its queue.
//!
//! The helper decodes everything; this page asks for exactly what the
//! screen shows, one ask at a time. docs/media.md has the keys.

use crate::files::{self, Entry, Look, Place, Sort};
use crate::helper::{Helper, Source};
use crate::proto::{Cmd, Event, Fail, Flags, Info, Kind, PlayState, Tags};
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::theme::{CARDPUTER, Metrics, ON_PICTURE, PANEL_OFF};
use gh0st_app::gfx::{Canvas, Rect, Theme, widgets};
use gh0st_app::media::{AskQueue, Picture, PictureView, SeekBar, plate, plate_text};
use gh0st_app::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod key {
    pub const SORT: &str = "gh0st.media.sort";
    pub const SHUFFLE: &str = "gh0st.media.shuffle";
    pub const REPEAT: &str = "gh0st.media.repeat";
}

const OPEN: u32 = 1;
const LEFT: u32 = 2;
const RIGHT: u32 = 3;
const UP: u32 = 4;
const DOWN: u32 = 5;
const SORT: u32 = 6;
const ZOOM_IN: u32 = 7;
const ZOOM_OUT: u32 = 8;
const FILL: u32 = 9;
const INFO: u32 = 10;
const PLAY: u32 = 11;
const BACK: u32 = 12;
const AHEAD: u32 = 13;
const SPEED: u32 = 14;
const AUDIO: u32 = 15;
const SUBS: u32 = 16;
const MARK_A: u32 = 17;
const MARK_B: u32 = 18;
const NO_REPEAT: u32 = 19;
const STEP: u32 = 20;
const PREV: u32 = 21;
const NEXT: u32 = 22;
const SHUFFLE: u32 = 23;
const REPEAT: u32 = 24;
const QUEUE: u32 = 25;
const SLIDES: u32 = 26;

/// How far the arrows and 5 and 7 move a video.
const SEEK_MS: u64 = 10_000;
/// A slideshow's picture stays this long.
const SLIDE_MS: u64 = 4000;
/// The grid: columns, and rows on screen.
const GRID_COLS: usize = 5;
const GRID_ROWS: usize = 3;
const SPEEDS: [u16; 6] = [50, 75, 100, 125, 150, 200];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Start,
    Grid,
    Viewer,
    Player,
    Music,
}

/// What an ask is for.
#[derive(Clone, Debug, PartialEq)]
enum For {
    /// A tile: the picture of `path` (a folder's face for a folder).
    Tile(PathBuf),
    Probe(PathBuf),
    Tags(PathBuf),
    /// The picture on screen, as asked.
    Full(PathBuf),
    /// A song's cover.
    Cover(PathBuf),
}

#[derive(Clone, Debug)]
struct Ask {
    id: u32,
    what: For,
    cmd: Cmd,
}

/// What plays.
#[derive(Clone, Debug)]
struct Playback {
    id: u32,
    path: PathBuf,
    bar: SeekBar,
    state: PlayState,
    speed: usize,
    /// A set, waiting for B.
    mark: Option<u64>,
    audio: Option<u32>,
    subs: Option<u32>,
    slow: bool,
}

pub struct Media {
    source: Source,
    helper: Option<Helper>,
    watch: Option<Watch>,
    missing: bool,
    places: Vec<Place>,
    start: List,
    view: View,
    dir: PathBuf,
    dir_title: String,
    /// Folders gone into, with the tile that was selected in each.
    trail: Vec<(PathBuf, usize)>,
    entries: Vec<Entry>,
    tiles: gh0st_app::Tiles,
    sort: Sort,
    context: String,
    /// Tile pictures by the file they show; an empty one could not be
    /// drawn.
    faces: HashMap<PathBuf, Picture>,
    /// A folder's face file, once looked for.
    folder_faces: HashMap<PathBuf, Option<PathBuf>>,
    infos: HashMap<PathBuf, Info>,
    fails: HashMap<PathBuf, Fail>,
    asks: AskQueue<Ask>,
    next_id: u32,
    full: Picture,
    pv: PictureView,
    show_info: bool,
    slides: Option<Timer>,
    play: Option<Playback>,
    tags: Option<Tags>,
    cover: Picture,
    /// Music: entries in play order, and which plays.
    queue: Vec<usize>,
    qpos: usize,
    /// The id the next track will play under, once asked ahead.
    next_track: Option<(u32, usize)>,
    shuffle: bool,
    repeat_all: bool,
    showing_queue: bool,
    queue_list: List,
    opened: Option<PathBuf>,
    quitting: bool,
}

impl Media {
    /// The app, opening `file` when one is given.
    pub fn new(file: Option<PathBuf>) -> Media {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into()));
        Media::with(Source::from_env(), home, file)
    }

    /// The app over `source`, its places under `home`: for tests and
    /// mockups.
    pub fn with(source: Source, home: PathBuf, file: Option<PathBuf>) -> Media {
        let m = &CARDPUTER;
        Media {
            source,
            helper: None,
            watch: None,
            missing: false,
            places: files::places(&home),
            start: List::new(),
            view: View::Start,
            dir: PathBuf::new(),
            dir_title: String::new(),
            trail: Vec::new(),
            entries: Vec::new(),
            tiles: gh0st_app::Tiles::new(GRID_COLS, GRID_ROWS),
            sort: Sort::Newest,
            context: String::new(),
            faces: HashMap::new(),
            folder_faces: HashMap::new(),
            infos: HashMap::new(),
            fails: HashMap::new(),
            asks: AskQueue::new(),
            next_id: 1,
            full: Picture::default(),
            pv: PictureView::new(m.width, m.height),
            show_info: false,
            slides: None,
            play: None,
            tags: None,
            cover: Picture::default(),
            queue: Vec::new(),
            qpos: 0,
            next_track: None,
            shuffle: false,
            repeat_all: false,
            showing_queue: false,
            queue_list: List::stopping(),
            opened: file,
            quitting: false,
        }
    }

    fn send(&self, cmd: &Cmd) {
        if let Some(h) = &self.helper {
            h.send(cmd);
        }
    }

    fn id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }

    fn launch(&mut self, cx: &mut Cx) {
        match Helper::start(&self.source) {
            Ok(h) => {
                self.watch = Some(cx.watch(h.fd()));
                if let Some(pid) = h.pid() {
                    cx.adopt(pid);
                }
                h.send(&Cmd::Hello);
                self.helper = Some(h);
                self.missing = false;
            }
            Err(e) => {
                eprintln!("gh0st-media: helper: {e}");
                self.missing = true;
            }
        }
    }

    fn ask(&mut self, what: For) {
        let p = |p: &PathBuf| p.to_string_lossy().into_owned();
        match &what {
            For::Tile(path) => {
                let same = |a: &Ask| a.what == what;
                if self.faces.contains_key(path) || self.asks.has(same) {
                    return;
                }
            }
            For::Full(_) => self.asks.retain(|a| !matches!(a.what, For::Full(_))),
            For::Cover(_) => self.asks.retain(|a| !matches!(a.what, For::Cover(_))),
            For::Probe(path) | For::Tags(path) => {
                if self.infos.contains_key(path) && matches!(what, For::Probe(_)) {
                    return;
                }
            }
        }
        let id = self.id();
        let m = &CARDPUTER;
        let cmd = match &what {
            For::Tile(path) => {
                let (w, h) = tile_size(m);
                Cmd::Thumb {
                    id,
                    w: w as u16,
                    h: h as u16,
                    path: p(path),
                }
            }
            For::Probe(path) => Cmd::Probe { id, path: p(path) },
            For::Tags(path) => Cmd::Tags { id, path: p(path) },
            For::Full(path) => Cmd::View {
                id,
                w: m.width as u16,
                h: m.height as u16,
                zoom: self.pv.zoom(),
                cx: self.pv.centre().0,
                cy: self.pv.centre().1,
                path: p(path),
            },
            For::Cover(path) => {
                let s = cover_side(m) as u16;
                Cmd::View {
                    id,
                    w: s,
                    h: s,
                    zoom: 1.0,
                    cx: 0.5,
                    cy: 0.5,
                    path: p(path),
                }
            }
        };
        let urgent = matches!(what, For::Full(_) | For::Probe(_) | For::Cover(_));
        let a = Ask { id, what, cmd };
        if urgent {
            self.asks.push_front(a);
        } else {
            self.asks.push(a);
        }
        self.next_ask();
    }

    fn next_ask(&mut self) {
        if let Some(cmd) = self.asks.take_next().map(|a| a.cmd.clone()) {
            self.send(&cmd);
        }
    }

    // ---- folders

    fn open_dir(&mut self, dir: PathBuf, select: usize) {
        self.stop_play();
        self.dir = dir;
        self.dir_title = self.title_of_dir();
        self.entries = files::list(&self.dir, self.sort);
        // A folder of songs (an album, with its cover) plays in its own
        // order.
        let songs = self
            .entries
            .iter()
            .filter(|e| e.look == Look::Sound)
            .count();
        let shows = self
            .entries
            .iter()
            .filter(|e| matches!(e.look, Look::Picture | Look::Video))
            .count();
        if songs > shows && self.sort != Sort::Name {
            self.entries = files::list(&self.dir, Sort::Name);
        }
        self.tiles = gh0st_app::Tiles::new(GRID_COLS, GRID_ROWS);
        self.tiles.set_len(self.entries.len());
        self.tiles.select(select);
        self.view = View::Grid;
        self.asks.retain(|a| !matches!(a.what, For::Tile(_)));
        self.want_tiles();
    }

    fn title_of_dir(&self) -> String {
        if let Some(p) = self.places.iter().find(|p| p.path == self.dir) {
            return p.name.to_string();
        }
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into())
    }

    /// The file a tile shows: the file itself, or a folder's face.
    fn face_of(&mut self, e: &Entry) -> Option<PathBuf> {
        if e.look != Look::Folder {
            return Some(e.path.clone());
        }
        self.folder_faces
            .entry(e.path.clone())
            .or_insert_with(|| files::folder_face(&e.path))
            .clone()
    }

    /// The tiles on screen and the row past it.
    fn want_tiles(&mut self) {
        let (first, n) = self.tiles.window();
        let end = (first + n + GRID_COLS).min(self.entries.len());
        for i in first..end {
            let e = self.entries[i].clone();
            if let Some(f) = self.face_of(&e) {
                self.ask(For::Tile(f));
            }
        }
        self.count_context();
    }

    /// The grid names the selected tile in the top bar; a whole-screen
    /// picture says where it is in the folder.
    fn count_context(&mut self) {
        self.context = match (self.view, self.entry()) {
            (View::Grid, Some(e)) => e.name.clone(),
            (_, Some(_)) => format!("{} of {}", self.tiles.at() + 1, self.entries.len()),
            _ => String::new(),
        };
    }

    // ---- one file

    fn entry(&self) -> Option<&Entry> {
        self.entries.get(self.tiles.at())
    }

    /// Opens the selected entry: a folder, a picture, a video or a song.
    fn open(&mut self, cx: &mut Cx) {
        let Some(e) = self.entry().cloned() else {
            return;
        };
        match e.look {
            Look::Folder => {
                self.trail.push((self.dir.clone(), self.tiles.at()));
                self.open_dir(e.path, 0);
            }
            Look::Picture | Look::Video => self.show(cx),
            Look::Sound => self.music(cx),
        }
    }

    /// The selected picture or video, on the whole screen.
    fn show(&mut self, cx: &mut Cx) {
        let Some(e) = self.entry().cloned() else {
            return;
        };
        self.stop_play();
        self.full = Picture::default();
        self.show_info = false;
        self.pv.set_fill(false);
        self.pv.show(self.infos.get(&e.path).map(|i| (i.w, i.h)));
        self.view = if e.look == Look::Video {
            View::Player
        } else {
            View::Viewer
        };
        self.count_context();
        self.ask(For::Probe(e.path.clone()));
        if self.infos.contains_key(&e.path) {
            self.ask(For::Full(e.path.clone()));
        }
        // A moving picture plays as soon as it shows.
        if self
            .infos
            .get(&e.path)
            .is_some_and(|i| i.kind == Kind::Anim)
        {
            self.start_play(cx, e.path, Flags::default());
        }
    }

    fn start_play(&mut self, cx: &mut Cx, path: PathBuf, flags: Flags) {
        let id = self.id();
        let m = &CARDPUTER;
        let audio = flags.audio;
        let subs = flags.subs;
        self.send(&Cmd::PlayOpen {
            id,
            w: m.width as u16,
            h: m.height as u16,
            flags,
            path: path.to_string_lossy().into_owned(),
        });
        self.play = Some(Playback {
            id,
            path,
            bar: SeekBar::default(),
            state: PlayState::Playing,
            speed: 2,
            mark: None,
            audio,
            subs,
            slow: false,
        });
        cx.keep_awake(self.view == View::Player);
    }

    fn stop_play(&mut self) {
        if self.play.take().is_some() {
            self.send(&Cmd::Stop);
        }
        self.next_track = None;
    }

    fn play_pause(&mut self, cx: &mut Cx) {
        let Some(e) = self.current() else {
            return;
        };
        match self.play.as_ref().map(|p| p.state) {
            None => self.start_play(cx, e, Flags::default()),
            Some(PlayState::Playing) => {
                self.send(&Cmd::Pause);
                if let Some(p) = &mut self.play {
                    p.state = PlayState::Paused;
                }
                cx.keep_awake(false);
            }
            Some(_) => {
                self.send(&Cmd::Resume);
                if let Some(p) = &mut self.play {
                    p.state = PlayState::Playing;
                }
                cx.keep_awake(self.view == View::Player);
            }
        }
    }

    fn seek(&mut self, ahead: bool) {
        let Some(p) = &mut self.play else {
            return;
        };
        let b = &mut p.bar;
        let to = if ahead {
            b.pos_ms + SEEK_MS
        } else {
            b.pos_ms.saturating_sub(SEEK_MS)
        };
        b.pos_ms = if b.dur_ms > 0 { to.min(b.dur_ms) } else { to };
        let at = b.pos_ms;
        self.send(&Cmd::Seek(at));
    }

    /// The file on screen or playing.
    fn current(&self) -> Option<PathBuf> {
        match self.view {
            View::Music => self
                .queue
                .get(self.qpos)
                .and_then(|&i| self.entries.get(i))
                .map(|e| e.path.clone()),
            _ => self.entry().map(|e| e.path.clone()),
        }
    }

    /// The next or previous picture or video in the folder, folders and
    /// songs passed over.
    fn step(&mut self, by: isize, cx: &mut Cx) {
        let n = self.entries.len() as isize;
        let mut i = self.tiles.at() as isize;
        loop {
            i += by;
            if i < 0 || i >= n {
                return;
            }
            if matches!(self.entries[i as usize].look, Look::Picture | Look::Video) {
                break;
            }
        }
        self.tiles.select(i as usize);
        self.show(cx);
    }

    // ---- music

    /// Plays the selected song, the folder's songs as the queue.
    fn music(&mut self, cx: &mut Cx) {
        self.stop_play();
        let at = self.tiles.at();
        let mut q: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.entries[i].look == Look::Sound)
            .collect();
        if self.shuffle {
            shuffle_from(&mut q, at, self.entries.len() as u64);
        }
        self.qpos = q.iter().position(|&i| i == at).unwrap_or(0);
        self.queue = q;
        self.view = View::Music;
        self.track(cx);
    }

    /// Starts the queue's track at qpos.
    fn track(&mut self, cx: &mut Cx) {
        let Some(path) = self.current() else {
            return;
        };
        self.tags = None;
        self.cover = Picture::default();
        self.ask(For::Tags(path.clone()));
        self.ask(For::Cover(path.clone()));
        self.start_play(cx, path, Flags::default());
        self.ahead();
        cx.keep_awake(false);
    }

    /// Asks the helper to open the following track, so it starts the
    /// moment this one ends.
    fn ahead(&mut self) {
        let next = self.qpos + 1;
        let i = if next < self.queue.len() {
            next
        } else if self.repeat_all && !self.queue.is_empty() {
            0
        } else {
            self.next_track = None;
            return;
        };
        let Some(path) = self
            .queue
            .get(i)
            .and_then(|&e| self.entries.get(e))
            .map(|e| e.path.to_string_lossy().into_owned())
        else {
            return;
        };
        let id = self.id();
        self.send(&Cmd::PlayNext { id, path });
        self.next_track = Some((id, i));
    }

    fn skip(&mut self, by: isize, cx: &mut Cx) {
        let n = self.queue.len() as isize;
        if n == 0 {
            return;
        }
        // Back from past a few seconds in: the start of this one.
        if by < 0 && self.play.as_ref().is_some_and(|p| p.bar.pos_ms > 3000) {
            self.send(&Cmd::Seek(0));
            return;
        }
        let i = self.qpos as isize + by;
        let i = if self.repeat_all {
            i.rem_euclid(n)
        } else {
            i.clamp(0, n - 1)
        };
        self.stop_play();
        self.qpos = i as usize;
        self.track(cx);
    }

    // ---- the helper's answers

    fn event(&mut self, e: Event, cx: &mut Cx) {
        match e {
            Event::Hello(_) => {}
            Event::Info { id, info } => {
                if let Some(a) = self.answered(id) {
                    if let For::Probe(path) = a.what {
                        self.probed(path, info, cx);
                    }
                }
            }
            Event::Tags { id, tags } => {
                if let Some(a) = self.answered(id) {
                    if matches!(a.what, For::Tags(ref p) if Some(p) == self.current().as_ref()) {
                        self.tags = Some(tags);
                    }
                }
            }
            Event::Thumb { id, slot, seq, .. } | Event::View { id, slot, seq, .. } => {
                let pic = self
                    .helper
                    .as_ref()
                    .map(|h| Picture::read(h.ring(), slot, seq))
                    .unwrap_or_default();
                if let Some(a) = self.answered(id) {
                    self.picture(a, pic);
                }
            }
            Event::Fail { id, why } => {
                if let Some(a) = self.answered(id) {
                    match a.what {
                        For::Tile(p) => {
                            self.faces.insert(p, Picture::default());
                        }
                        For::Probe(p) | For::Full(p) => {
                            self.fails.insert(p, why);
                        }
                        For::Cover(_) | For::Tags(_) => {}
                    }
                }
            }
            Event::Frame { slot, seq, .. } => {
                if let Some(h) = &self.helper {
                    if self.play.is_some() && self.view == View::Player {
                        self.full.read_into(h.ring(), slot, seq);
                    }
                }
            }
            Event::Play {
                id,
                pos_ms,
                dur_ms,
                state,
                ..
            } => self.played(id, pos_ms, dur_ms, state, cx),
            Event::Slow { id, .. } => {
                if let Some(p) = &mut self.play {
                    if p.id == id && !p.slow {
                        p.slow = true;
                        cx.toast_tone(Tone::Warning, "Too heavy to play smoothly here");
                    }
                }
            }
            Event::PlayErr { id, why } => {
                if self.play.as_ref().is_some_and(|p| p.id == id) {
                    eprintln!("gh0st-media: playing: {why}");
                    self.play = None;
                    cx.toast_tone(Tone::Critical, "Can't play this file");
                    cx.keep_awake(false);
                }
            }
        }
    }

    /// The ask an answer is for. Stale ones are dropped.
    fn answered(&mut self, id: u32) -> Option<Ask> {
        let a = self.asks.answered();
        self.next_ask();
        a.filter(|a| a.id == id)
    }

    fn probed(&mut self, path: PathBuf, info: Info, cx: &mut Cx) {
        let here = self.current().as_ref() == Some(&path);
        let anim = info.kind == Kind::Anim;
        let size = (info.w, info.h);
        self.infos.insert(path.clone(), info);
        if !here || !matches!(self.view, View::Viewer | View::Player) {
            return;
        }
        self.pv.show(Some(size));
        self.ask(For::Full(path.clone()));
        if anim && self.play.is_none() {
            self.start_play(cx, path, Flags::default());
        }
    }

    fn picture(&mut self, a: Ask, pic: Picture) {
        match a.what {
            For::Tile(p) => {
                self.faces.insert(p, pic);
            }
            For::Full(p) => {
                // Only if it is still what the screen wants.
                let want = Cmd::View {
                    id: a.id,
                    w: CARDPUTER.width as u16,
                    h: CARDPUTER.height as u16,
                    zoom: self.pv.zoom(),
                    cx: self.pv.centre().0,
                    cy: self.pv.centre().1,
                    path: p.to_string_lossy().into_owned(),
                };
                if self.current().as_ref() == Some(&p) && a.cmd == want && self.frame_is_still() {
                    self.full = pic;
                }
            }
            For::Cover(p) => {
                if self.current().as_ref() == Some(&p) {
                    self.cover = pic;
                }
            }
            For::Probe(_) | For::Tags(_) => {}
        }
    }

    /// True while no video plays over the picture asked.
    fn frame_is_still(&self) -> bool {
        self.play.is_none() || self.view == View::Viewer
    }

    fn played(&mut self, id: u32, pos_ms: u64, dur_ms: u64, state: PlayState, cx: &mut Cx) {
        // The next track has begun.
        if let Some((nid, i)) = self.next_track {
            if nid == id {
                self.qpos = i;
                if let Some(p) = &mut self.play {
                    p.id = id;
                    p.mark = None;
                }
                self.tags = None;
                self.cover = Picture::default();
                if let Some(path) = self.current() {
                    if let Some(p) = &mut self.play {
                        p.path = path.clone();
                    }
                    self.ask(For::Tags(path.clone()));
                    self.ask(For::Cover(path));
                }
                self.ahead();
            }
        }
        let Some(p) = &mut self.play else {
            return;
        };
        if p.id != id {
            return;
        }
        p.bar.pos_ms = pos_ms;
        p.bar.dur_ms = dur_ms;
        p.state = state;
        if state == PlayState::Done {
            cx.keep_awake(false);
        }
    }

    fn gone(&mut self, cx: &mut Cx) {
        if let Some(w) = self.watch.take() {
            cx.unwatch(w);
        }
        self.helper = None;
        self.asks.clear();
        self.play = None;
        if self.quitting {
            return;
        }
        // The file it was reading may be what killed it: marked, then the
        // helper starts again.
        if let Some(p) = self.current() {
            self.fails.entry(p).or_insert(Fail::Corrupt);
        }
        self.launch(cx);
        match self.view {
            View::Grid => self.want_tiles(),
            View::Music => self.track(cx),
            _ => {}
        }
    }

    fn save(&self, cx: &mut Cx) {
        let s = cx.settings();
        s.set_str(
            key::SORT,
            if self.sort == Sort::Name {
                "name"
            } else {
                "newest"
            },
        );
        s.set_bool(key::SHUFFLE, self.shuffle);
        s.set_bool(key::REPEAT, self.repeat_all);
    }

    fn load(&mut self, cx: &mut Cx) {
        let s = cx.settings();
        self.sort = if s.get_str(key::SORT) == Some("name") {
            Sort::Name
        } else {
            Sort::Newest
        };
        self.shuffle = s.get_bool(key::SHUFFLE).unwrap_or(false);
        self.repeat_all = s.get_bool(key::REPEAT).unwrap_or(false);
    }

    /// Opens the file the app was started for, in its folder.
    fn open_given(&mut self, path: PathBuf, cx: &mut Cx) {
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        self.open_dir(dir, 0);
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            self.tiles.select(i);
            self.open(cx);
        } else {
            cx.toast("That file isn't here any more");
        }
    }
}

impl Default for Media {
    fn default() -> Media {
        Media::new(None)
    }
}

/// Shuffles `q`, keeping `first` at the front: a small LCG seeded by `seed`.
fn shuffle_from(q: &mut [usize], first: usize, seed: u64) {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        | 1;
    for i in (1..q.len()).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (s >> 33) as usize % (i + 1);
        q.swap(i, j);
    }
    if let Some(p) = q.iter().position(|&i| i == first) {
        q.swap(0, p);
    }
}

/// A grid tile: 4:3, three rows between the bars.
fn tile_size(m: &Metrics) -> (i32, i32) {
    let h = (m.height - m.bar_h - m.hint_h - 3 - 2 * m.tile_gap) / GRID_ROWS as i32 - 1;
    (h * 4 / 3, h)
}

/// A song's cover on its page: square, as tall as the page lets it be.
fn cover_side(m: &Metrics) -> i32 {
    m.height - m.bar_h - m.hint_h - 2 * m.margin_v
}

fn speed_word(p: u16) -> String {
    match p {
        100 => "Normal".into(),
        p if p % 100 == 0 => format!("{}\u{d7}", p / 100),
        p => format!("{}\u{d7}", p as f32 / 100.0),
    }
}

impl Media {
    fn draw_start(&self, ui: &mut Ui) {
        let counts: Vec<String> = self
            .places
            .iter()
            .map(|p| match std::fs::read_dir(&p.path) {
                Ok(rd) => rd.flatten().count().to_string(),
                Err(_) => String::new(),
            })
            .collect();
        let rows: Vec<Row> = self
            .places
            .iter()
            .zip(&counts)
            .map(|(p, n)| {
                let r = Row::link(p.name).icon(p.icon);
                if n.is_empty() {
                    r.disabled("not there")
                } else {
                    r.value(n)
                }
            })
            .collect();
        ui.list(&self.start, &rows);
    }

    fn draw_grid(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        if self.entries.is_empty() {
            ui.message("images", "Nothing to show in this folder");
            return;
        }
        let c = ui.canvas();
        let (tw, th) = tile_size(&t.m);
        let gap = t.m.tile_gap;
        let inner = Rect::new(area.x, area.y + 3, area.w, area.h - 3);
        let mut last = Rect::new(0, 0, 0, 0);
        for (i, r) in self.tiles.layout(inner, (tw, th), gap) {
            let Some(e) = self.entries.get(i) else {
                continue;
            };
            c.fill(r, t.c.surface);
            let face = if e.look == Look::Folder {
                self.folder_faces.get(&e.path).cloned().flatten()
            } else {
                Some(e.path.clone())
            };
            match face.as_ref().and_then(|f| self.faces.get(f)) {
                Some(p) if !p.is_empty() => p.draw(c, r),
                None if e.look == Look::Folder && face.is_none() => {
                    let sz = IconSize::Medium as i32;
                    icon::draw(
                        c,
                        "folder",
                        IconSize::Medium,
                        r.x + (r.w - sz) / 2,
                        r.y + (r.h - sz) / 2,
                        t.c.muted,
                    );
                }
                Some(_) if e.look != Look::Folder => {
                    let sz = IconSize::Small as i32;
                    icon::draw(
                        c,
                        "triangle-alert",
                        IconSize::Small,
                        r.x + (r.w - sz) / 2,
                        r.y + (r.h - sz) / 2,
                        t.c.muted,
                    );
                }
                _ => {}
            }
            let badge = match e.look {
                Look::Folder => Some("folder"),
                Look::Video => Some("video"),
                Look::Sound => Some("music"),
                Look::Picture => None,
            };
            if let Some(name) = badge {
                let sz = IconSize::Small as i32;
                let b = Rect::new(r.x + 2, r.bottom() - sz - 4, sz + 4, sz + 2);
                plate(c, b);
                icon::draw(c, name, IconSize::Small, b.x + 2, b.y + 1, ON_PICTURE);
            }
            if i == self.tiles.at() {
                widgets::tile_focus(c, &t, r);
            }
            last = last.union(&r);
        }
        let (first, n) = self.tiles.window();
        widgets::scrollbar(c, &t, last, first, n, self.tiles.len());
    }

    /// Over a picture on the whole screen: where it is in the folder, its
    /// details, and a word when it failed.
    fn draw_marks(&self, c: &mut Canvas, t: &Theme) {
        let screen = Rect::new(0, 0, t.m.width, t.m.height);
        let m = t.m.margin;
        let y = t.m.margin_v;
        let Some(e) = self.entry() else {
            return;
        };
        if let Some(f) = self.fails.get(&e.path) {
            let s = IconSize::Large as i32;
            icon::draw(
                c,
                "triangle-alert",
                IconSize::Large,
                (t.m.width - s) / 2,
                t.m.height / 2 - s - 4,
                ON_PICTURE,
            );
            widgets::text_centre(
                c,
                t.f.small,
                Rect::new(0, t.m.height / 2 + 2, t.m.width, t.m.line_h),
                f.says(),
                ON_PICTURE,
            );
        } else if self.full.is_empty() {
            widgets::text_centre(c, t.f.small, screen, "\u{2026}", ON_PICTURE);
        }
        let r = plate_text(c, t, m, y, &self.context, ON_PICTURE);
        self.pv.draw_marks(c, t, y);
        if !self.show_info {
            return;
        }
        let l = gh0st_app::rt::local_at(e.when);
        let mut lines = vec![
            e.name.clone(),
            format!(
                "{} {} {}, {:02}:{:02}",
                l.day,
                gh0st_app::rt::month_short(l.month),
                l.year,
                l.hour,
                l.minute
            ),
            files::size(e.bytes),
        ];
        if let Some(i) = self.infos.get(&e.path) {
            if i.w > 0 {
                lines.push(format!("{}\u{d7}{}", i.w, i.h));
            }
            let codecs: Vec<&str> = [i.vcodec.as_str(), i.acodec.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            if !codecs.is_empty() {
                lines.push(codecs.join(", "));
            }
            if i.dur_ms > 0 {
                lines.push(gh0st_app::media::clock(i.dur_ms));
            }
        }
        let mut y = r.bottom() + 4;
        for line in lines {
            let pr = plate_text(c, t, m, y, &line, ON_PICTURE);
            y = pr.bottom() + 2;
        }
    }

    fn draw_full(&self, c: &mut Canvas, t: &Theme) {
        let screen = Rect::new(0, 0, t.m.width, t.m.height);
        c.fill(screen, PANEL_OFF);
        self.full.draw(c, screen);
        self.draw_marks(c, t);
        if self.view != View::Player {
            return;
        }
        let mark = match self.play.as_ref().map(|p| p.state) {
            Some(PlayState::Playing) => None,
            Some(PlayState::Paused) => Some("pause"),
            Some(PlayState::Done) => Some("rotate-cw"),
            None => Some("play"),
        };
        let anim = self
            .entry()
            .and_then(|e| self.infos.get(&e.path))
            .is_some_and(|i| i.kind == Kind::Anim);
        if let Some(name) = mark.filter(|_| !anim && !self.show_info) {
            let sz = IconSize::Large as i32;
            let pad = t.m.plate_pad;
            let cr = Rect::new(
                (t.m.width - sz) / 2 - pad,
                (t.m.height - sz) / 2 - pad,
                sz + 2 * pad,
                sz + 2 * pad,
            );
            plate(c, cr);
            icon::draw(c, name, IconSize::Large, cr.x + pad, cr.y + pad, ON_PICTURE);
        }
        if let Some(p) = self.play.as_ref().filter(|_| !anim) {
            p.bar.draw_over(c, t);
        }
    }

    fn draw_music(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        if self.showing_queue {
            let names: Vec<String> = self
                .queue
                .iter()
                .filter_map(|&i| self.entries.get(i))
                .map(|e| {
                    Path::new(&e.name)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect();
            let rows: Vec<Row> = names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let r = Row::text(n).icon(if i == self.qpos { "volume-2" } else { "music" });
                    if i == self.qpos { r } else { r.dim() }
                })
                .collect();
            ui.list(&self.queue_list, &rows);
            return;
        }
        let c = ui.canvas();
        let m = t.m.margin;
        let side = cover_side(&t.m);
        let cover = Rect::new(area.x + m, area.y + t.m.margin_v, side, side);
        c.fill(cover, t.c.surface);
        if self.cover.is_empty() {
            let s = IconSize::Large as i32;
            icon::draw(
                c,
                "music",
                IconSize::Large,
                cover.x + (side - s) / 2,
                cover.y + (side - s) / 2,
                t.c.muted,
            );
        } else {
            self.cover.draw(c, cover);
        }
        let x = cover.right() + m;
        let w = area.right() - m - x;
        let name = self.current().map(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let tags = self.tags.clone().unwrap_or_default();
        let title = if tags.title.is_empty() {
            name.unwrap_or_default()
        } else {
            tags.title.clone()
        };
        let line = t.m.line_h;
        let mut y = cover.y;
        widgets::text_in(
            c,
            t.f.body_bold,
            Rect::new(x, y, w, line + 4),
            &title,
            t.c.fg,
        );
        y += line + 6;
        if !tags.artist.is_empty() {
            widgets::text_in(
                c,
                t.f.body,
                Rect::new(x, y, w, line + 2),
                &tags.artist,
                t.c.fg,
            );
            y += line + 3;
        }
        if !tags.album.is_empty() {
            let soft = widgets::soft(&t);
            widgets::text_in(c, t.f.small, Rect::new(x, y, w, line), &tags.album, soft);
            y += line;
        }
        let mut marks = vec![format!("{} of {}", self.qpos + 1, self.queue.len())];
        if self.shuffle {
            marks.push("Shuffle".into());
        }
        if self.repeat_all {
            marks.push("Repeat".into());
        }
        if let Some(p) = &self.play {
            if SPEEDS[p.speed] != 100 {
                marks.push(speed_word(SPEEDS[p.speed]));
            }
        }
        let muted = t.c.muted;
        widgets::text_in(
            c,
            t.f.small,
            Rect::new(x, y, w, line),
            &marks.join("  "),
            muted,
        );
        if let Some(p) = &self.play {
            let bar = Rect::new(x, cover.bottom() - t.m.track_h - line - 2, w, line);
            p.bar.draw_in(c, &t, bar);
        }
    }
}

impl Page for Media {
    fn title(&self) -> &str {
        match self.view {
            View::Grid => &self.dir_title,
            View::Music => "Music",
            _ => "Media",
        }
    }

    fn icon(&self) -> &str {
        "images"
    }

    fn context(&self) -> Option<&str> {
        match self.view {
            View::Grid => Some(self.context.as_str()),
            _ => None,
        }
    }

    fn about(&self) -> &str {
        "Pictures, video and music from the card. A helper decodes every \
         file, so one that is damaged can't take the app down."
    }

    fn bars(&self) -> Bars {
        match self.view {
            View::Start => Bars::top(),
            View::Grid => Bars::top().chip(6, Chip::new("list", self.sort.word()).runs(SORT)),
            View::Viewer => {
                let fill = if self.pv.fill() {
                    "minimize"
                } else {
                    "maximize"
                };
                Bars::actions()
                    .over()
                    .chip(
                        5,
                        Chip::new("zoom-out", "")
                            .runs(ZOOM_OUT)
                            .disabled(!self.pv.can_zoom_out()),
                    )
                    .chip(6, Chip::new(fill, "").runs(FILL).on(self.pv.fill()))
                    .chip(
                        7,
                        Chip::new("zoom-in", "")
                            .runs(ZOOM_IN)
                            .disabled(!self.pv.can_zoom_in()),
                    )
            }
            View::Player => {
                let main = match self.play.as_ref().map(|p| p.state) {
                    Some(PlayState::Playing) => Chip::new("pause", ""),
                    Some(PlayState::Done) => Chip::new("rotate-cw", ""),
                    _ => Chip::new("play", ""),
                };
                let idle = self.play.is_none();
                Bars::actions()
                    .over()
                    .chip(5, Chip::new("rewind", "").runs(BACK).disabled(idle))
                    .chip(6, main.main().runs(PLAY))
                    .chip(7, Chip::new("fast-forward", "").runs(AHEAD).disabled(idle))
            }
            View::Music => {
                let playing = self
                    .play
                    .as_ref()
                    .is_some_and(|p| p.state == PlayState::Playing);
                Bars::top()
                    .chip(5, Chip::new("skip-back", "Prev").runs(PREV))
                    .chip(
                        6,
                        Chip::new(
                            if playing { "pause" } else { "play" },
                            if playing { "Pause" } else { "Play" },
                        )
                        .main()
                        .runs(PLAY),
                    )
                    .chip(7, Chip::new("skip-forward", "Next").runs(NEXT))
            }
        }
    }

    fn frame_ms(&self) -> u64 {
        40
    }

    fn start(&mut self, cx: &mut Cx) {
        self.load(cx);
        self.launch(cx);
        if let Some(p) = self.opened.take() {
            self.open_given(p, cx);
        }
    }

    fn stop(&mut self, cx: &mut Cx) {
        self.quitting = true;
        self.save(cx);
        self.send(&Cmd::Quit);
    }

    fn front(&mut self, front: bool, cx: &mut Cx) {
        if front {
            // The folder may have changed meanwhile.
            if self.view == View::Grid {
                self.open_dir(self.dir.clone(), self.tiles.at());
            }
            cx.redraw();
            return;
        }
        // Music plays on behind; a video and a slideshow stop.
        if self.view != View::Music && self.play.is_some() {
            self.send(&Cmd::Pause);
            if let Some(p) = &mut self.play {
                p.state = PlayState::Paused;
            }
        }
        if let Some(t) = self.slides.take() {
            cx.cancel(t);
        }
        cx.keep_awake(false);
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        loop {
            let Some(h) = &self.helper else {
                return;
            };
            match h.recv() {
                Some(e) => self.event(e, cx),
                None => {
                    if h.closed() {
                        self.gone(cx);
                    }
                    break;
                }
            }
        }
        cx.redraw();
    }

    fn exited(&mut self, _: i32, code: Option<i32>, cx: &mut Cx) {
        if code != Some(0) && !self.quitting {
            eprintln!("gh0st-media: the helper exited: {code:?}");
        }
        self.gone(cx);
        cx.redraw();
    }

    fn timer(&mut self, t: Timer, cx: &mut Cx) {
        if self.slides == Some(t) {
            let before = self.tiles.at();
            self.step(1, cx);
            if self.tiles.at() == before {
                // The end of the folder: the show is over.
                cx.cancel(t);
                self.slides = None;
                cx.keep_awake(false);
            }
        }
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        match self.view {
            View::Start => true,
            View::Grid => {
                if let Some((dir, at)) = self.trail.pop() {
                    self.open_dir(dir, at);
                } else {
                    self.view = View::Start;
                }
                false
            }
            View::Music if self.showing_queue => {
                self.showing_queue = false;
                false
            }
            View::Viewer | View::Player | View::Music => {
                // Music stops when its page closes; it plays on behind other
                // apps, not behind the grid.
                self.stop_play();
                if let Some(t) = self.slides.take() {
                    cx.cancel(t);
                }
                cx.keep_awake(false);
                self.full = Picture::default();
                self.asks.retain(|a| matches!(a.what, For::Tile(_)));
                self.send(&Cmd::Forget);
                self.view = View::Grid;
                self.want_tiles();
                false
            }
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.view == View::Start {
            if let Some(Action::Open(i)) = self.start.key(key) {
                if let Some(p) = self.places.get(i).cloned() {
                    self.trail.clear();
                    self.open_dir(p.path, 0);
                }
            }
            let _ = cx;
            return;
        }
        if self.view == View::Music && self.showing_queue {
            if let Some(Action::Open(i)) = self.queue_list.key(key) {
                self.showing_queue = false;
                self.stop_play();
                self.qpos = i;
                self.track(cx);
            }
        }
    }

    fn commands(&self, c: &mut Commands) {
        let arrows = |c: &mut Commands, l: &'static str, r: &'static str| {
            c.add(Command::action(LEFT, l).keys("z").keys_only().repeats());
            c.add(Command::action(RIGHT, r).keys("c").keys_only().repeats());
        };
        match self.view {
            View::Start => {}
            View::Grid => {
                arrows(c, "Left", "Right");
                c.add(Command::action(UP, "Up").keys("f").keys_only().repeats());
                c.add(
                    Command::action(DOWN, "Down")
                        .keys("x")
                        .keys_only()
                        .repeats(),
                );
                c.add(Command::action(OPEN, "Open").keys("Enter").keys_only());
                c.add(Command::choice(SORT, "Sort", self.sort.word()).keys("s"));
                c.add(Command::action(SLIDES, "Slideshow").keys("p"));
            }
            View::Viewer => {
                let moves = self.pv.moves();
                if moves {
                    arrows(c, "Look left", "Look right");
                    c.add(
                        Command::action(UP, "Look up")
                            .keys("f")
                            .keys_only()
                            .repeats(),
                    );
                    c.add(
                        Command::action(DOWN, "Look down")
                            .keys("x")
                            .keys_only()
                            .repeats(),
                    );
                } else {
                    arrows(c, "Previous", "Next");
                }
                c.add(Command::action(INFO, "Details").keys("Enter"));
                c.add(Command::toggle(FILL, "Fill the screen", self.pv.fill()).keys("Space"));
                c.add(Command::action(ZOOM_OUT, "Zoom out").keys_only());
                c.add(Command::action(ZOOM_IN, "Zoom in").keys_only());
                c.add(Command::toggle(SLIDES, "Slideshow", self.slides.is_some()).keys("p"));
            }
            View::Player => {
                let (l, r) = if self.play.is_some() {
                    ("Back 10 s", "Ahead 10 s")
                } else {
                    ("Previous", "Next")
                };
                arrows(c, l, r);
                let play = match self.play.as_ref().map(|p| p.state) {
                    Some(PlayState::Playing) => "Pause",
                    Some(PlayState::Done) => "Play again",
                    _ => "Play",
                };
                c.add(Command::action(PLAY, play).keys("Enter"));
                c.add(Command::action(BACK, "Back 10 s").keys_only());
                c.add(Command::action(AHEAD, "Ahead 10 s").keys_only());
                self.play_commands(c);
                c.add(Command::action(INFO, "Details").keys("i"));
            }
            View::Music => {
                c.add(Command::action(PLAY, "Play or pause").keys("Enter Space"));
                c.add(Command::action(PREV, "Previous").keys("z").repeats());
                c.add(Command::action(NEXT, "Next").keys("c").repeats());
                c.add(Command::action(BACK, "Back 10 s").keys("x").keys_only());
                c.add(Command::action(AHEAD, "Ahead 10 s").keys("f").keys_only());
                c.add(Command::action(QUEUE, "Queue").keys("q").opens());
                c.add(Command::toggle(SHUFFLE, "Shuffle", self.shuffle).keys("s"));
                c.add(Command::toggle(REPEAT, "Repeat", self.repeat_all).keys("r"));
                self.play_commands(c);
            }
        }
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let fwd = !matches!(step, Step::Prev);
        match (self.view, id) {
            (View::Grid, LEFT | RIGHT | UP | DOWN) => {
                let (dx, dy) = match id {
                    LEFT => (-1, 0),
                    RIGHT => (1, 0),
                    UP => (0, -1),
                    _ => (0, 1),
                };
                if self.tiles.step(dx, dy) {
                    self.want_tiles();
                }
            }
            (View::Grid, OPEN) => self.open(cx),
            (View::Grid, SORT) => {
                self.sort = if self.sort == Sort::Newest {
                    Sort::Name
                } else {
                    Sort::Newest
                };
                let at = self.entry().map(|e| e.path.clone());
                self.open_dir(self.dir.clone(), 0);
                if let Some(i) = at.and_then(|p| self.entries.iter().position(|e| e.path == p)) {
                    self.tiles.select(i);
                    self.want_tiles();
                }
                self.save(cx);
            }
            (View::Grid | View::Viewer, SLIDES) => {
                if let Some(t) = self.slides.take() {
                    cx.cancel(t);
                    cx.keep_awake(false);
                } else {
                    if self.view == View::Grid {
                        self.open(cx);
                    }
                    self.slides = Some(cx.every(SLIDE_MS));
                    cx.keep_awake(true);
                }
            }
            (View::Viewer, LEFT | RIGHT | UP | DOWN) if self.pv.moves() => {
                let (dx, dy) = match id {
                    LEFT => (-1, 0),
                    RIGHT => (1, 0),
                    UP => (0, -1),
                    _ => (0, 1),
                };
                if self.pv.pan(dx, dy) {
                    if let Some(p) = self.current() {
                        self.ask(For::Full(p));
                    }
                }
            }
            (View::Viewer | View::Player, LEFT | RIGHT) if self.play.is_none() => {
                self.step(if id == LEFT { -1 } else { 1 }, cx)
            }
            (View::Viewer, ZOOM_IN | ZOOM_OUT | FILL) => {
                let moved = match id {
                    ZOOM_IN => self.pv.zoom_in(),
                    ZOOM_OUT => self.pv.zoom_out(),
                    _ => {
                        self.pv.set_fill(!self.pv.fill());
                        true
                    }
                };
                if moved {
                    if let Some(p) = self.current() {
                        self.ask(For::Full(p));
                    }
                }
            }
            (_, INFO) => self.show_info = !self.show_info,
            (View::Player | View::Music, PLAY) => self.play_pause(cx),
            (View::Player, LEFT | BACK) | (View::Music, BACK) => self.seek(false),
            (View::Player, RIGHT | AHEAD) | (View::Music, AHEAD) => self.seek(true),
            (View::Music, PREV) => self.skip(-1, cx),
            (View::Music, NEXT) => self.skip(1, cx),
            (View::Music, QUEUE) => {
                self.showing_queue = true;
                self.queue_list.select(self.qpos);
            }
            (View::Music, SHUFFLE) => {
                self.shuffle = !self.shuffle;
                self.save(cx);
            }
            (View::Music, REPEAT) => {
                self.repeat_all = !self.repeat_all;
                self.ahead();
                self.save(cx);
            }
            (_, SPEED) => {
                if let Some(p) = &mut self.play {
                    p.speed = if fwd {
                        (p.speed + 1).min(SPEEDS.len() - 1)
                    } else {
                        p.speed.saturating_sub(1)
                    };
                    let s = SPEEDS[p.speed];
                    self.send(&Cmd::Speed(s));
                }
            }
            (_, AUDIO | SUBS) => self.tracks(id, fwd),
            (_, MARK_A) => {
                if let Some(p) = &mut self.play {
                    p.mark = Some(p.bar.pos_ms);
                    p.bar.repeat = None;
                }
            }
            (_, MARK_B) => {
                if let Some(p) = &mut self.play {
                    if let Some(a) = p.mark.take().filter(|&a| p.bar.pos_ms > a) {
                        p.bar.repeat = Some((a, p.bar.pos_ms));
                        let r = p.bar.repeat;
                        self.send(&Cmd::Repeat(r));
                    }
                }
            }
            (_, NO_REPEAT) => {
                if let Some(p) = &mut self.play {
                    p.bar.repeat = None;
                    p.mark = None;
                }
                self.send(&Cmd::Repeat(None));
            }
            (_, STEP) => self.send(&Cmd::Step),
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        if self.missing && self.view != View::Start {
            ui.message("triangle-alert", "The media helper is missing");
            return;
        }
        match self.view {
            View::Start => self.draw_start(ui),
            View::Grid => self.draw_grid(ui),
            View::Viewer | View::Player => self.draw_full(ui.canvas(), &t),
            View::Music => self.draw_music(ui),
        }
    }
}

impl Media {
    /// Speed, tracks and A to B, for a video or a song.
    fn play_commands(&self, c: &mut Commands) {
        let Some(p) = &self.play else {
            return;
        };
        let info = self.infos.get(&p.path);
        c.group("Playing");
        c.add(Command::choice(SPEED, "Speed", speed_word(SPEEDS[p.speed])));
        if let Some(i) = info.filter(|i| i.audio > 1) {
            let a = format!("{} of {}", p.audio.unwrap_or(0) + 1, i.audio);
            c.add(Command::choice(AUDIO, "Sound track", a));
        }
        if let Some(i) = info.filter(|i| i.subs > 0) {
            let s = match p.subs {
                None => "Off".to_string(),
                Some(n) => format!("{} of {}", n + 1, i.subs),
            };
            c.add(Command::choice(SUBS, "Subtitles", s));
        }
        if p.mark.is_some() {
            c.add(Command::action(MARK_B, "Repeat to here").keys("b"));
        } else {
            c.add(Command::action(MARK_A, "Repeat from here").keys("a"));
        }
        if p.bar.repeat.is_some() {
            c.add(Command::action(NO_REPEAT, "Stop repeating"));
        }
        if self.view == View::Player {
            c.add(
                Command::action(STEP, "Next picture")
                    .keys("n")
                    .enabled(p.state != PlayState::Playing),
            );
        }
    }

    /// Steps the sound or subtitle track; the helper opens it again there.
    fn tracks(&mut self, id: u32, fwd: bool) {
        let Some(p) = &mut self.play else {
            return;
        };
        let Some(info) = self.infos.get(&p.path) else {
            return;
        };
        if id == AUDIO && info.audio > 1 {
            let n = p.audio.unwrap_or(0);
            p.audio = Some(if fwd {
                (n + 1) % info.audio
            } else {
                (n + info.audio - 1) % info.audio
            });
        } else if id == SUBS && info.subs > 0 {
            // Off, then each track, round.
            let all = info.subs + 1;
            let at = p.subs.map_or(0, |s| s + 1);
            let next = if fwd {
                (at + 1) % all
            } else {
                (at + all - 1) % all
            };
            p.subs = next.checked_sub(1);
        }
        let cmd = Cmd::Tracks {
            audio: p.audio.map_or(-1, |a| a as i32),
            subs: p.subs.map_or(-1, |s| s as i32),
        };
        self.send(&cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_starts_with_the_song_picked() {
        let mut q: Vec<usize> = (0..9).collect();
        shuffle_from(&mut q, 4, 9);
        assert_eq!(q[0], 4);
        let mut sorted = q.clone();
        sorted.sort();
        assert_eq!(sorted, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn speeds_read_as_words() {
        assert_eq!(speed_word(100), "Normal");
        assert_eq!(speed_word(200), "2\u{d7}");
        assert_eq!(speed_word(150), "1.5\u{d7}");
    }
}
