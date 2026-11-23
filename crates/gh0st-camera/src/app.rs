//! The camera: one page in three views. The viewfinder keeps the action bar
//! (chips on 5 to 7, the menu on 8) and fills the rest: a 4:3 picture
//! between two labelled rails for photos, a 16:9 one for video. Review
//! shows what was shot, under the OS bars. docs/camera.md has the keys and
//! the design. Photos shows every shot as a grid of tiles; Enter opens
//! one on the whole screen, where it zooms to the photo's own pixels.

use crate::files::{self, Kind, Shot};
use crate::helper::{Helper, Source};
use crate::proto::{Cmd, Event, Fault, Mode, State, VideoSize};
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::theme::{CARDPUTER, Metrics, ON_PICTURE, PANEL_OFF, PICTURE_PLATE};
use gh0st_app::gfx::{Canvas, Color, Rect, Theme, widgets};
use gh0st_app::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

pub mod key {
    pub const MODE: &str = "gh0st.camera.mode";
    pub const VIDEO: &str = "gh0st.camera.video";
    pub const TIMER: &str = "gh0st.camera.timer";
    pub const GRID: &str = "gh0st.camera.grid";
    pub const QUALITY: &str = "gh0st.camera.quality";
    pub const RAW: &str = "gh0st.camera.raw";
    pub const MIC: &str = "gh0st.camera.mic";
    pub const FLICKER: &str = "gh0st.camera.flicker";
}

const SHOOT: u32 = 1;
const MODE: u32 = 2;
const ZOOM: u32 = 3;
const EV: u32 = 4;
const PHOTOS: u32 = 5;
const TIMER: u32 = 6;
const GRID: u32 = 7;
const QUALITY: u32 = 8;
const RAW: u32 = 9;
const VIDEO: u32 = 10;
const MIC: u32 = 11;
const FLICKER: u32 = 12;
const PREV: u32 = 13;
const NEXT: u32 = 14;
const FULL: u32 = 15;
const LIGHT: u32 = 16;
const LESS: u32 = 17;
const MORE: u32 = 18;
const UP: u32 = 19;
const DOWN: u32 = 20;
const ZOOM_OUT: u32 = 21;
const ZOOM_IN: u32 = 22;
const INFO: u32 = 23;

/// The gallery grid: columns, and rows on screen.
const GRID_COLS: usize = 5;
const GRID_ROWS: usize = 3;
/// Zoom steps in the full view, past fitting the screen. The last step is
/// always one photo pixel per screen pixel.
const FULL_ZOOMS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

const ZOOMS: [(f32, &str); 7] = [
    (1.0, "1\u{d7}"),
    (1.25, "1.25\u{d7}"),
    (1.5, "1.5\u{d7}"),
    (2.0, "2\u{d7}"),
    (2.5, "2.5\u{d7}"),
    (3.0, "3\u{d7}"),
    (4.0, "4\u{d7}"),
];
/// Exposure in thirds of a stop.
const EVS: [(f32, &str); 13] = [
    (-2.0, "\u{2212}2"),
    (-1.667, "\u{2212}1.7"),
    (-1.333, "\u{2212}1.3"),
    (-1.0, "\u{2212}1"),
    (-0.667, "\u{2212}0.7"),
    (-0.333, "\u{2212}0.3"),
    (0.0, "\u{b1}0"),
    (0.333, "+0.3"),
    (0.667, "+0.7"),
    (1.0, "+1"),
    (1.333, "+1.3"),
    (1.667, "+1.7"),
    (2.0, "+2"),
];
const EV_ZERO: usize = 6;
const TIMERS: [u8; 3] = [0, 3, 10];
/// JPEG quality: Fine, Normal.
const QUALITIES: [(u8, &str); 2] = [(95, "Fine"), (85, "Normal")];

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// The helper is starting; no frame yet.
    Starting,
    Live,
    /// The camera can't be used: why.
    Failed(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Finder,
    /// Every shot, a grid of tiles.
    Gallery,
    /// One shot on the whole screen, zoomable.
    Full,
}

/// What a picture asked of the helper is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum For {
    /// The last shot, in the viewfinder's rail.
    Last,
    Tile,
    Full,
}

/// One picture asked of the helper. It answers in order, so the one in
/// flight is always the next answer.
#[derive(Clone, Debug)]
struct Ask {
    what: For,
    path: PathBuf,
    cmd: Cmd,
}

struct Rec {
    ms: u64,
}

/// A picture the helper drew for us.
#[derive(Default)]
struct Pic {
    px: Vec<u16>,
    w: u16,
    h: u16,
}

impl Pic {
    fn draw(&self, c: &mut Canvas, r: Rect) {
        if self.w == 0 {
            return;
        }
        let x = r.x + (r.w - self.w as i32) / 2;
        let y = r.y + (r.h - self.h as i32) / 2;
        let old = c.set_clip(c.clip().intersect(&r));
        c.blit565(
            x,
            y,
            self.w as i32,
            self.h as i32,
            self.w as usize,
            &self.px,
        );
        c.set_clip(old);
    }
}

pub struct Camera {
    source: Source,
    dir: PathBuf,
    helper: Option<Helper>,
    watch: Option<Watch>,
    phase: Phase,
    state: State,
    view: View,
    video: bool,
    size: VideoSize,
    zoom: usize,
    ev: usize,
    timer: usize,
    grid: bool,
    quality: usize,
    raw: bool,
    mic: bool,
    flicker60: bool,
    /// The light control is open over the picture: 5 and 7 are − and +.
    light: bool,
    frame: Pic,
    /// The newest shot, small, for the rail.
    last: Option<PathBuf>,
    last_pic: Pic,
    /// Tiles drawn so far, by path. An empty one could not be drawn.
    tiles_pic: HashMap<PathBuf, Pic>,
    asks: VecDeque<Ask>,
    asking: Option<Ask>,
    tiles: gh0st_app::Tiles,
    full_pic: Pic,
    /// The zoom step, and where the view looks, 0 to 1 across the photo.
    zoom_at: usize,
    centre: (f32, f32),
    /// The photo's own size, for the closest zoom.
    full_px: Option<(u32, u32)>,
    info: bool,
    saving: bool,
    flash: bool,
    rec: Option<Rec>,
    count: Option<(u8, Timer)>,
    free: Option<u64>,
    shots: Vec<Shot>,
    context: String,
    quitting: bool,
}

impl Camera {
    pub fn new() -> Camera {
        Camera::with(Source::from_env(), crate::helper::default_dir())
    }

    /// A camera over `source`, saving into `dir`: for tests and mockups.
    pub fn with(source: Source, dir: PathBuf) -> Camera {
        Camera {
            source,
            dir,
            helper: None,
            watch: None,
            phase: Phase::Starting,
            state: State::Off,
            view: View::Finder,
            video: false,
            size: VideoSize::Sharp1080,
            zoom: 0,
            ev: EV_ZERO,
            timer: 0,
            grid: false,
            quality: 0,
            raw: false,
            mic: true,
            flicker60: false,
            light: false,
            frame: Pic::default(),
            last: None,
            last_pic: Pic::default(),
            tiles_pic: HashMap::new(),
            asks: VecDeque::new(),
            asking: None,
            tiles: gh0st_app::Tiles::new(GRID_COLS, GRID_ROWS),
            full_pic: Pic::default(),
            zoom_at: 0,
            centre: (0.5, 0.5),
            full_px: None,
            info: false,
            saving: false,
            flash: false,
            rec: None,
            count: None,
            free: None,
            shots: Vec::new(),
            context: String::new(),
            quitting: false,
        }
    }

    fn mode(&self) -> Mode {
        if self.video {
            Mode::Video(self.size)
        } else {
            Mode::Photo
        }
    }

    fn send(&self, cmd: Cmd) {
        if let Some(h) = &self.helper {
            h.send(&cmd);
        }
    }

    fn load(&mut self, cx: &mut Cx) {
        let s = cx.settings();
        self.video = s.get_str(key::MODE) == Some("video");
        if s.get_str(key::VIDEO) == Some("720") {
            self.size = VideoSize::Wide720;
        }
        let t = s.get_int(key::TIMER).unwrap_or(0);
        self.timer = TIMERS.iter().position(|&v| v as i64 == t).unwrap_or(0);
        self.grid = s.get_bool(key::GRID).unwrap_or(false);
        self.quality = (s.get_str(key::QUALITY) == Some("normal")) as usize;
        self.raw = s.get_bool(key::RAW).unwrap_or(false);
        self.mic = s.get_bool(key::MIC).unwrap_or(true);
        self.flicker60 = s.get_int(key::FLICKER) == Some(60);
    }

    fn launch(&mut self, cx: &mut Cx) {
        let h = match Helper::start(&self.source) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("gh0st-camera: helper: {e}");
                self.phase = Phase::Failed("The camera helper is missing");
                return;
            }
        };
        self.watch = Some(cx.watch(h.fd()));
        if let Some(pid) = h.pid {
            cx.adopt(pid);
        }
        self.helper = Some(h);
        self.send(Cmd::Hello);
        self.send(Cmd::Set(
            "quality".into(),
            QUALITIES[self.quality].0.to_string(),
        ));
        self.send(Cmd::Set(
            "flicker".into(),
            if self.flicker60 { "60" } else { "50" }.into(),
        ));
        self.restart();
    }

    /// Sends the mode and the controls, then starts the stream.
    fn restart(&mut self) {
        self.send(Cmd::Mode(self.mode()));
        self.send(Cmd::Set("zoom".into(), ZOOMS[self.zoom].0.to_string()));
        self.send(Cmd::Set("ev".into(), EVS[self.ev].0.to_string()));
        self.send(Cmd::Start);
    }

    fn refresh(&mut self) {
        self.free = files::free_mb(&self.dir);
        self.shots = files::list(&self.dir);
    }

    fn want_last_thumb(&mut self) {
        self.last = self.shots.first().map(|s| s.path.clone());
        if let Some(p) = self.last.clone() {
            let t = thumb_size(&CARDPUTER);
            self.ask(For::Last, p, t, 1.0, (0.5, 0.5));
        } else {
            self.last_pic = Pic::default();
        }
    }

    /// Asks for a picture. A newer full view replaces one still waiting; a
    /// tile already drawn or on its way isn't asked twice.
    fn ask(&mut self, what: For, path: PathBuf, size: (u16, u16), zoom: f32, centre: (f32, f32)) {
        match what {
            For::Full => self.asks.retain(|a| a.what != For::Full),
            For::Tile => {
                let same = |a: &Ask| a.what == For::Tile && a.path == path;
                if self.tiles_pic.contains_key(&path)
                    || self.asks.iter().any(same)
                    || self.asking.as_ref().is_some_and(same)
                {
                    return;
                }
            }
            For::Last => self.asks.retain(|a| a.what != For::Last),
        }
        let cmd = Cmd::Thumb {
            path: path.to_string_lossy().into_owned(),
            w: size.0,
            h: size.1,
            zoom,
            cx: centre.0,
            cy: centre.1,
        };
        self.asks.push_back(Ask { what, path, cmd });
        self.next_ask();
    }

    fn next_ask(&mut self) {
        if self.asking.is_some() {
            return;
        }
        if let Some(a) = self.asks.pop_front() {
            self.send(a.cmd.clone());
            self.asking = Some(a);
        }
    }

    /// The tiles on screen and the row past it, asked for if not drawn.
    fn want_tiles(&mut self) {
        let (first, n) = self.tiles.window();
        let size = tile_size(&CARDPUTER);
        let end = (first + n + GRID_COLS).min(self.shots.len());
        for i in first..end {
            let p = self.shots[i].path.clone();
            self.ask(
                For::Tile,
                p,
                (size.0 as u16, size.1 as u16),
                1.0,
                (0.5, 0.5),
            );
        }
        self.context = format!("{} of {}", self.tiles.at() + 1, self.shots.len());
    }

    /// The full view of the selected shot, at the zoom and place it is at.
    fn want_full(&mut self) {
        let Some(s) = self.shots.get(self.tiles.at()) else {
            return;
        };
        let p = s.path.clone();
        let m = &CARDPUTER;
        self.ask(
            For::Full,
            p,
            (m.width as u16, m.height as u16),
            self.zoom(),
            self.centre,
        );
        self.context = format!("{} of {}", self.tiles.at() + 1, self.shots.len());
    }

    /// The zoom steps this shot has: up to its own pixels.
    fn zooms(&self) -> Vec<f32> {
        let m = &CARDPUTER;
        let Some((w, h)) = self.full_px else {
            return vec![1.0];
        };
        let fit = (m.width as f32 / w as f32).min(m.height as f32 / h as f32);
        let most = 1.0 / fit;
        let mut z: Vec<f32> = FULL_ZOOMS
            .iter()
            .copied()
            .filter(|&z| z < most * 0.9)
            .collect();
        if most > 1.05 {
            z.push(most);
        }
        z
    }

    fn zoom(&self) -> f32 {
        let z = self.zooms();
        z[self.zoom_at.min(z.len() - 1)]
    }

    /// How much of the photo the screen shows at this zoom, 0 to 1 each way.
    fn view_frac(&self) -> (f32, f32) {
        let m = &CARDPUTER;
        let Some((w, h)) = self.full_px else {
            return (1.0, 1.0);
        };
        let (w, h) = (w as f32, h as f32);
        let scale = (m.width as f32 / w).min(m.height as f32 / h) * self.zoom();
        (
            (m.width as f32 / scale / w).min(1.0),
            (m.height as f32 / scale / h).min(1.0),
        )
    }

    /// Keeps the view inside the photo.
    fn clamp_centre(&mut self) {
        let (fw, fh) = self.view_frac();
        self.centre.0 = self.centre.0.clamp(fw / 2.0, 1.0 - fw / 2.0);
        self.centre.1 = self.centre.1.clamp(fh / 2.0, 1.0 - fh / 2.0);
    }

    /// Shows the selected shot whole, fitted.
    fn open_full(&mut self) {
        self.view = View::Full;
        self.zoom_at = 0;
        self.centre = (0.5, 0.5);
        self.full_pic = Pic::default();
        self.full_px = self
            .shots
            .get(self.tiles.at())
            .filter(|s| s.kind == Kind::Photo)
            .and_then(|s| files::jpeg_size(&s.path));
        self.want_full();
    }

    fn shoot(&mut self, cx: &mut Cx) {
        if self.phase != Phase::Live {
            return;
        }
        if self.video {
            if self.rec.is_some() {
                self.send(Cmd::RecStop);
                return;
            }
        } else if self.state != State::Preview || self.saving {
            return;
        }
        self.free = files::free_mb(&self.dir);
        if self.free.is_some_and(|f| f < files::MIN_FREE_MB) {
            cx.toast_tone(Tone::Warning, "The card is full");
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            eprintln!("gh0st-camera: {}: {e}", self.dir.display());
            cx.toast_tone(Tone::Critical, "Can't save to the card");
            return;
        }
        if self.video {
            let path = files::fresh(&self.dir, "VID", "mp4", files::now());
            self.send(Cmd::RecStart {
                path: path.to_string_lossy().into_owned(),
                mic: self.mic,
            });
            self.rec = Some(Rec { ms: 0 });
            cx.keep_awake(true);
            return;
        }
        if TIMERS[self.timer] > 0 && self.count.is_none() {
            let t = cx.every(1000);
            self.count = Some((TIMERS[self.timer], t));
            return;
        }
        self.take();
    }

    fn take(&mut self) {
        let path = files::fresh(&self.dir, "CAM", "jpg", files::now());
        self.send(Cmd::Shoot {
            path: path.to_string_lossy().into_owned(),
            raw: self.raw,
        });
        self.saving = true;
        self.flash = true;
    }

    fn cancel_count(&mut self, cx: &mut Cx) -> bool {
        match self.count.take() {
            Some((_, t)) => {
                cx.cancel(t);
                true
            }
            None => false,
        }
    }

    fn event(&mut self, e: Event, cx: &mut Cx) {
        match e {
            Event::Hello { .. } => {}
            Event::State(s) => {
                self.state = s;
                if s == State::Preview && self.phase == Phase::Starting {
                    self.phase = Phase::Live;
                }
            }
            Event::Frame { slot, seq, .. } => {
                if let Some(h) = &self.helper {
                    if let Some((w, hh)) = h.ring.read(slot, seq, &mut self.frame.px) {
                        (self.frame.w, self.frame.h) = (w, hh);
                        self.flash = false;
                        if self.phase == Phase::Starting {
                            self.phase = Phase::Live;
                        }
                    }
                }
            }
            Event::Shot { path, result } => {
                self.saving = false;
                match result {
                    Ok(_) => {
                        self.refresh();
                        self.last = Some(PathBuf::from(&path));
                        self.want_last_thumb();
                    }
                    Err(e) => {
                        eprintln!("gh0st-camera: {path}: {e}");
                        cx.toast_tone(Tone::Critical, "The photo wasn't saved");
                    }
                }
            }
            Event::Rec { ms, .. } => {
                if let Some(r) = &mut self.rec {
                    r.ms = ms;
                }
                self.free = files::free_mb(&self.dir);
                if self.free.is_some_and(|f| f < files::MIN_FREE_MB) {
                    self.send(Cmd::RecStop);
                    cx.toast_tone(Tone::Warning, "The card is full");
                }
            }
            Event::RecDone { .. } => {
                self.rec = None;
                cx.keep_awake(false);
                self.refresh();
                self.want_last_thumb();
            }
            Event::Thumb {
                path, slot, seq, ..
            } => {
                let mut pic = Pic::default();
                if let Some(h) = &self.helper {
                    if let Some((w, hh)) = h.ring.read(slot, seq, &mut pic.px) {
                        (pic.w, pic.h) = (w, hh);
                    }
                }
                self.answered(&path, pic);
            }
            Event::NoThumb { path } => self.answered(&path, Pic::default()),
            Event::Error(f) => {
                let why = match f {
                    Fault::NoCamera => "No camera on this device",
                    Fault::NoMemory => "Not enough memory for the camera",
                    Fault::Busy => "Another app is using the camera",
                    Fault::Io(e) => {
                        eprintln!("gh0st-camera: {e}");
                        self.rec = None;
                        self.saving = false;
                        cx.keep_awake(false);
                        cx.toast_tone(Tone::Critical, "Can't save to the card");
                        return;
                    }
                    Fault::Other(e) => {
                        eprintln!("gh0st-camera: {e}");
                        "The camera stopped"
                    }
                };
                self.phase = Phase::Failed(why);
            }
        }
    }

    /// The helper's answer to the picture in flight.
    fn answered(&mut self, path: &str, pic: Pic) {
        let Some(a) = self.asking.take() else {
            return;
        };
        if a.path.to_string_lossy() == path {
            match a.what {
                For::Last if self.last.as_ref() == Some(&a.path) => self.last_pic = pic,
                For::Tile => {
                    self.tiles_pic.insert(a.path, pic);
                }
                // Only if it is still what the screen wants: the same shot,
                // zoom and place.
                For::Full if self.view == View::Full => {
                    let now = self.shots.get(self.tiles.at()).map(|s| &s.path);
                    if now == Some(&a.path)
                        && a.cmd
                            == (Cmd::Thumb {
                                path: path.into(),
                                w: CARDPUTER.width as u16,
                                h: CARDPUTER.height as u16,
                                zoom: self.zoom(),
                                cx: self.centre.0,
                                cy: self.centre.1,
                            })
                    {
                        self.full_pic = pic;
                    }
                }
                _ => {}
            }
        }
        self.next_ask();
    }

    fn gone(&mut self, cx: &mut Cx) {
        if let Some(w) = self.watch.take() {
            cx.unwatch(w);
        }
        self.helper = None;
        self.asking = None;
        self.asks.clear();
        if self.rec.take().is_some() {
            cx.keep_awake(false);
        }
        if !self.quitting && !matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Failed("The camera stopped");
        }
    }

    fn open_review(&mut self, cx: &mut Cx) {
        self.refresh();
        if self.shots.is_empty() {
            cx.toast("Nothing shot yet");
            return;
        }
        self.tiles = gh0st_app::Tiles::new(GRID_COLS, GRID_ROWS);
        self.tiles.set_len(self.shots.len());
        self.view = View::Gallery;
        // The sensor rests while you look back.
        self.send(Cmd::Stop);
        self.want_tiles();
    }

    fn close_review(&mut self) {
        self.view = View::Finder;
        self.full_pic = Pic::default();
        self.asks.retain(|a| a.what == For::Last);
        self.refresh();
        self.want_last_thumb();
        self.restart();
    }

    fn delete(&mut self, cx: &mut Cx) {
        let Some(s) = self.shots.get(self.tiles.at()).cloned() else {
            return;
        };
        if let Err(e) = std::fs::remove_file(&s.path) {
            eprintln!("gh0st-camera: {}: {e}", s.path.display());
            cx.toast_tone(Tone::Critical, "Can't delete it");
            return;
        }
        let _ = std::fs::remove_file(s.path.with_extension("dng"));
        if let Some(poster) = files::poster(&s.path) {
            let _ = std::fs::remove_file(poster);
        }
        self.tiles_pic.remove(&s.path);
        self.refresh();
        if self.shots.is_empty() {
            self.close_review();
            return;
        }
        self.tiles.set_len(self.shots.len());
        if self.view == View::Full {
            self.open_full();
        } else {
            self.want_tiles();
        }
    }

    /// The next or the previous shot, in the full view.
    fn step(&mut self, by: isize) {
        if self.tiles.step(by, 0) {
            self.open_full();
        }
    }

    fn save(&self, cx: &mut Cx) {
        let s = cx.settings();
        s.set_str(key::MODE, if self.video { "video" } else { "photo" });
        s.set_str(key::VIDEO, self.size.token());
        s.set_int(key::TIMER, TIMERS[self.timer] as i64);
        s.set_bool(key::GRID, self.grid);
        s.set_str(
            key::QUALITY,
            if self.quality == 1 { "normal" } else { "fine" },
        );
        s.set_bool(key::RAW, self.raw);
        s.set_bool(key::MIC, self.mic);
        s.set_int(key::FLICKER, if self.flicker60 { 60 } else { 50 });
    }
}

impl Default for Camera {
    fn default() -> Camera {
        Camera::new()
    }
}

/// The viewfinder's picture height: the whole screen, under the buttons.
fn finder_h(m: &Metrics) -> i32 {
    m.height
}

/// Where marks on the picture and in the rails must stop: the buttons lie
/// over the last `hint_h` rows.
fn foot(m: &Metrics) -> i32 {
    m.height - m.hint_h
}

/// A rail beside the 4:3 picture.
fn rail_w(m: &Metrics) -> i32 {
    (m.width - finder_h(m) * 4 / 3) / 2
}

/// The preview the helper draws: the picture's own size.
pub fn preview_size(m: &Metrics, video: bool) -> (i32, i32) {
    let h = finder_h(m);
    let w = if video { h * 16 / 9 } else { h * 4 / 3 };
    (w & !1, h)
}

/// The rail's last-shot picture: 4:3 inside the rail's margins.
fn thumb_size(m: &Metrics) -> (u16, u16) {
    let w = rail_w(m) - 2 * m.margin - 2;
    (w as u16, (w * 3 / 4) as u16)
}

/// A gallery tile: 4:3, three rows between the bars.
fn tile_size(m: &Metrics) -> (i32, i32) {
    let h = (m.height - m.bar_h - m.hint_h - 3 - 2 * m.tile_gap) / GRID_ROWS as i32 - 1;
    (h * 4 / 3, h)
}

/// Where the preview goes: 4:3 between the rails, or 16:9 across, above
/// the action bar.
fn preview_rect(t: &Theme, video: bool) -> Rect {
    let (w, h) = preview_size(&t.m, video);
    Rect::new((t.m.width - w) / 2, 0, w, h)
}

/// A dark plate under words drawn over the picture.
fn plate(c: &mut Canvas, r: Rect) {
    c.fill_alpha(r, PICTURE_PLATE, 150);
}

/// Words on a plate over the picture, starting at `x`. Returns the plate.
fn plate_text(c: &mut Canvas, t: &Theme, x: i32, y: i32, s: &str, ink: Color) -> Rect {
    let f = t.f.small_bold;
    let r = Rect::new(x, y, f.measure(s) + 2 * 5, t.m.pill_h + 4);
    plate(c, r);
    widgets::text_in(c, f, Rect::new(r.x + 5, r.y, r.w - 10, r.h), s, ink);
    r
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

impl Camera {
    fn draw_finder(&self, c: &mut Canvas, t: &Theme) {
        let p = preview_rect(t, self.video);
        c.fill(p, PANEL_OFF);
        self.frame.draw(c, p);
        if self.frame.w == 0 && self.phase == Phase::Starting {
            widgets::text_centre(c, t.f.small, p, "Starting\u{2026}", ON_PICTURE);
        }
        if self.grid {
            let ink = ON_PICTURE;
            for i in 1..3 {
                c.fill_alpha(Rect::new(p.x + p.w * i / 3, p.y, 1, p.h), ink, 90);
                c.fill_alpha(Rect::new(p.x, p.y + p.h * i / 3, p.w, 1), ink, 90);
            }
        }
        if self.flash {
            c.fill_alpha(p, ON_PICTURE, 200);
        }
        if let Some((n, _)) = self.count {
            let s = n.to_string();
            let f = t.f.big;
            let side = f.measure("10") + 24;
            let r = Rect::new(p.x + (p.w - side) / 2, p.y + (p.h - side) / 2, side, side);
            plate(c, r);
            widgets::text_centre(c, f, r, &s, ON_PICTURE);
        }
        if self.video {
            self.draw_video_marks(c, t, p);
        } else {
            self.draw_rails(c, t);
        }
        if self.light {
            self.draw_light(c, t, p);
        }
    }

    /// One control in the left rail: its icon, its value, and the keys
    /// that move it. Returns the next free row.
    #[allow(clippy::too_many_arguments)]
    fn rail_control(
        c: &mut Canvas,
        t: &Theme,
        x0: i32,
        y: i32,
        icon_name: &str,
        value: &str,
        set: bool,
        keys: &str,
    ) -> i32 {
        let rw = rail_w(&t.m);
        let line = t.m.line_h;
        let s = IconSize::Small as i32;
        icon::draw(
            c,
            icon_name,
            IconSize::Small,
            x0 + (rw - s) / 2,
            y,
            t.c.muted,
        );
        let ink = if set { t.c.accent } else { t.c.fg };
        widgets::text_centre(
            c,
            t.f.small_bold,
            Rect::new(x0, y + s + 1, rw, line),
            value,
            ink,
        );
        let pw = widgets::pills_w(t, keys);
        let py = y + s + 1 + line + 2;
        widgets::pills(c, t, x0 + (rw - pw) / 2, py, keys, widgets::Pill::Normal);
        py + t.m.pill_h
    }

    /// An icon and a few words, centred in a rail.
    fn rail_mark(
        c: &mut Canvas,
        t: &Theme,
        x0: i32,
        y: i32,
        icon_name: &str,
        word: &str,
        ink: Color,
    ) {
        let rw = rail_w(&t.m);
        let s = IconSize::Small as i32;
        let w = s + 3 + t.f.small.measure(word);
        let x = x0 + (rw - w) / 2;
        icon::draw(
            c,
            icon_name,
            IconSize::Small,
            x,
            y + (t.m.line_h - s) / 2,
            ink,
        );
        widgets::text_in(
            c,
            t.f.small,
            Rect::new(x + s + 3, y, w - s - 3, t.m.line_h),
            word,
            ink,
        );
    }

    fn draw_rails(&self, c: &mut Canvas, t: &Theme) {
        let rw = rail_w(&t.m);
        let h = finder_h(&t.m);
        let x0 = t.m.width - rw;
        c.fill(Rect::new(0, 0, rw, h), t.c.bg);
        c.fill(Rect::new(x0, 0, rw, h), t.c.bg);
        let line = t.m.line_h;
        let s = IconSize::Small as i32;
        // Left: zoom, and the last shot over its button on 4.
        let y = t.m.margin_v + 2;
        Self::rail_control(
            c,
            t,
            0,
            y,
            "zoom-in",
            ZOOMS[self.zoom].1,
            self.zoom != 0,
            "z c",
        );
        let (tw, th) = thumb_size(&t.m);
        let box_ = Rect::new(
            (rw - tw as i32) / 2,
            foot(&t.m) - th as i32 - t.m.margin_v,
            tw as i32,
            th as i32,
        );
        if self.last_pic.w > 0 {
            self.last_pic.draw(c, box_);
        } else {
            c.fill(box_, t.c.surface);
        }
        if self.saving {
            plate(c, box_);
            widgets::text_centre(c, t.f.small_bold, box_, "\u{2026}", ON_PICTURE);
        }
        // Right: the mode and the photos left, the light over its button
        // on 7, then the timer and grid when on.
        let mut y = t.m.margin_v + 2;
        icon::draw(
            c,
            "camera",
            IconSize::Small,
            x0 + (rw - s) / 2,
            y,
            t.c.accent,
        );
        y += s + 1;
        if let Some(f) = self.free {
            let ink = if f < files::MIN_FREE_MB + 4 * files::PHOTO_MB {
                t.c.warning
            } else {
                t.c.muted
            };
            widgets::text_centre(
                c,
                t.f.small,
                Rect::new(x0, y, rw, line),
                &files::left(f, Kind::Photo),
                ink,
            );
        }
        y += line + 6;
        let mut y = Self::rail_control(
            c,
            t,
            x0,
            y,
            "sun",
            EVS[self.ev].1,
            self.ev != EV_ZERO,
            "x f",
        ) + 4;
        if TIMERS[self.timer] > 0 {
            Self::rail_mark(
                c,
                t,
                x0,
                y,
                "timer",
                &format!("{}s", TIMERS[self.timer]),
                t.c.accent,
            );
            y += line;
        }
        if self.grid {
            Self::rail_mark(c, t, x0, y, "layout-grid", "", t.c.muted);
        }
    }

    /// The light control over the foot of the picture: − and + are on the
    /// chips below it, Esc closes it.
    fn draw_light(&self, c: &mut Canvas, t: &Theme, p: Rect) {
        let m = t.m.margin;
        let s = IconSize::Small as i32;
        let h = t.m.pill_h + 8;
        let r = Rect::new(p.x + m, foot(&t.m) - m - h, p.w - 2 * m, h);
        c.fill_alpha(r, PICTURE_PLATE, 190);
        let cy = r.y + h / 2;
        icon::draw(c, "sun", IconSize::Small, r.x + 5, cy - s / 2, ON_PICTURE);
        let value = EVS[self.ev].1;
        let vw = t.f.small_bold.measure("\u{2212}1.7") + 8;
        let ew = widgets::pills_w(t, "Esc");
        widgets::pills(
            c,
            t,
            r.right() - 5 - ew,
            cy - t.m.pill_h / 2,
            "Esc",
            widgets::Pill::Normal,
        );
        let vx = r.right() - 5 - ew - 6 - vw;
        widgets::text_centre(
            c,
            t.f.small_bold,
            Rect::new(vx, r.y, vw, h),
            value,
            ON_PICTURE,
        );
        // The track: −2 to +2, a tick at 0, the knob at the value.
        let x1 = r.x + 5 + s + 8;
        let x2 = vx - 8;
        c.fill_alpha(Rect::new(x1, cy, x2 - x1, 2), ON_PICTURE, 110);
        let mid = (x1 + x2) / 2;
        c.fill(Rect::new(mid, cy - 3, 1, 8), ON_PICTURE);
        let kx = x1 + (x2 - x1) * self.ev as i32 / (EVS.len() as i32 - 1);
        c.circle(kx as f32, cy as f32 + 1.0, 5.0, t.c.accent);
    }

    fn draw_video_marks(&self, c: &mut Canvas, t: &Theme, p: Rect) {
        let h = finder_h(&t.m);
        c.fill(Rect::new(0, 0, p.x, h), t.c.bg);
        c.fill(Rect::new(p.right(), 0, t.m.width - p.right(), h), t.c.bg);
        let m = t.m.margin;
        let y = t.m.margin_v;
        let size = match self.size {
            VideoSize::Sharp1080 => "1080p",
            VideoSize::Wide720 => "720p",
        };
        let r = plate_text(c, t, p.x + m, y, size, ON_PICTURE);
        if !self.mic {
            let s = IconSize::Small as i32;
            let mr = Rect::new(r.right() + 4, r.y, s + 10, r.h);
            plate(c, mr);
            icon::draw(
                c,
                "volume-x",
                IconSize::Small,
                mr.x + 5,
                mr.y + (mr.h - s) / 2,
                ON_PICTURE,
            );
        }
        if let Some(rec) = &self.rec {
            let s = clock(rec.ms);
            let f = t.f.small_bold;
            let w = 12 + f.measure(&s) + 10;
            let r = Rect::new(p.x + (p.w - w) / 2, y, w, t.m.pill_h + 4);
            plate(c, r);
            let cy = r.y as f32 + r.h as f32 / 2.0;
            c.circle(r.x as f32 + 8.0, cy, 4.0, t.c.critical);
            widgets::text_in(
                c,
                f,
                Rect::new(r.x + 15, r.y, r.w - 18, r.h),
                &s,
                ON_PICTURE,
            );
        } else if let Some(f) = self.free {
            let s = files::left(f, Kind::Video);
            let w = t.f.small_bold.measure(&s) + 10;
            plate_text(c, t, p.right() - m - w, y, &s, ON_PICTURE);
        }
        if self.light {
            return;
        }
        let mut words = Vec::new();
        if self.zoom != 0 {
            words.push(ZOOMS[self.zoom].1.to_string());
        }
        if self.ev != EV_ZERO {
            words.push(format!("{} EV", EVS[self.ev].1));
        }
        if !words.is_empty() {
            let foot = foot(&t.m) - t.m.margin_v - t.m.pill_h - 4;
            plate_text(c, t, p.x + m, foot, &words.join("  "), ON_PICTURE);
        }
    }

    fn draw_gallery(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        let c = ui.canvas();
        let (tw, th) = tile_size(&t.m);
        let gap = t.m.tile_gap;
        let inner = Rect::new(area.x, area.y + 3, area.w, area.h - 3);
        let mut last = Rect::new(0, 0, 0, 0);
        for (i, r) in self.tiles.layout(inner, (tw, th), gap) {
            let Some(s) = self.shots.get(i) else {
                continue;
            };
            c.fill(r, t.c.surface);
            match self.tiles_pic.get(&s.path) {
                Some(p) if p.w > 0 => p.draw(c, r),
                Some(_) => {
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
                None => {}
            }
            if s.kind == Kind::Video {
                let sz = IconSize::Small as i32;
                let b = Rect::new(r.x + 2, r.bottom() - sz - 4, sz + 4, sz + 2);
                plate(c, b);
                icon::draw(c, "video", IconSize::Small, b.x + 2, b.y + 1, ON_PICTURE);
            }
            if i == self.tiles.at() {
                widgets::tile_focus(c, &t, r);
            }
            last = last.union(&r);
        }
        let (first, n) = self.tiles.window();
        widgets::scrollbar(c, &t, last, first, n, self.tiles.len());
    }

    fn draw_full(&self, c: &mut Canvas, t: &Theme) {
        let screen = Rect::new(0, 0, t.m.width, t.m.height);
        c.fill(screen, PANEL_OFF);
        self.full_pic.draw(c, screen);
        let Some(s) = self.shots.get(self.tiles.at()) else {
            return;
        };
        if self.full_pic.w == 0 {
            widgets::text_centre(c, t.f.small, screen, "\u{2026}", ON_PICTURE);
        }
        let m = t.m.margin;
        let y = t.m.margin_v;
        let r = plate_text(c, t, m, y, &self.context, ON_PICTURE);
        if s.kind == Kind::Video {
            let sz = IconSize::Large as i32;
            let cr = Rect::new(
                (t.m.width - sz) / 2 - 6,
                (t.m.height - sz) / 2 - 6,
                sz + 12,
                sz + 12,
            );
            plate(c, cr);
            icon::draw(c, "video", IconSize::Large, cr.x + 6, cr.y + 6, ON_PICTURE);
        }
        if self.zoom_at > 0 {
            let z = self.zoom();
            let most = self.zooms().last().copied().unwrap_or(1.0);
            let label = if (z - most).abs() < 0.01 {
                "1:1".to_string()
            } else {
                format!("{}\u{d7}", z)
            };
            let w = t.f.small_bold.measure(&label) + 10;
            plate_text(c, t, t.m.width - m - w, y, &label, ON_PICTURE);
            // Where the view is in the photo: a small map.
            let (fw, fh) = self.view_frac();
            let map = Rect::new(t.m.width - m - 40, y + t.m.pill_h + 10, 40, 30);
            plate(c, map);
            let v = Rect::new(
                map.x + ((self.centre.0 - fw / 2.0) * map.w as f32) as i32,
                map.y + ((self.centre.1 - fh / 2.0) * map.h as f32) as i32,
                ((fw * map.w as f32) as i32).max(2),
                ((fh * map.h as f32) as i32).max(2),
            );
            c.fill_alpha(v, ON_PICTURE, 160);
        }
        if self.info {
            let l = gh0st_core::rt::local_at(s.when);
            let mut lines = vec![
                format!(
                    "{} {} {}, {:02}:{:02}",
                    l.day,
                    gh0st_core::rt::month_short(l.month),
                    l.year,
                    l.hour,
                    l.minute
                ),
                files::size(s.bytes),
            ];
            if let Some((w, h)) = self.full_px {
                lines.push(format!("{w}\u{d7}{h}"));
            }
            let mut y = r.bottom() + 4;
            for line in lines {
                let pr = plate_text(c, t, m, y, &line, ON_PICTURE);
                y = pr.bottom() + 2;
            }
        }
    }
}

impl Page for Camera {
    fn title(&self) -> &str {
        match self.view {
            View::Finder => "Camera",
            _ => "Photos",
        }
    }

    fn icon(&self) -> &str {
        "camera"
    }

    fn context(&self) -> Option<&str> {
        (self.view == View::Gallery).then_some(self.context.as_str())
    }

    fn status(&self) -> Option<(&str, Tone)> {
        self.rec.as_ref().map(|_| ("REC", Tone::Critical))
    }

    fn bars(&self) -> Bars {
        match self.view {
            View::Finder => {
                let rec = self.rec.is_some();
                let ready = self.phase == Phase::Live && !self.saving && self.count.is_none();
                // Icons only: the buttons under the screen, the shutter in
                // the middle.
                let shutter = match (self.video, rec) {
                    (false, _) => Chip::new("aperture", "").main(),
                    (true, false) => Chip::new("record", "").main(),
                    (true, true) => Chip::new("square", "").danger(),
                };
                let shutter = shutter.runs(SHOOT).disabled(!ready && !rec);
                // 4, far left as on a phone: the photos taken. Esc is Back.
                let photos = Chip::new("images", "").runs(PHOTOS).disabled(rec);
                if self.light {
                    Bars::actions()
                        .over()
                        .chip(4, photos)
                        .chip(5, Chip::new("minus", "").runs(LESS))
                        .chip(6, shutter)
                        .chip(7, Chip::new("plus", "").runs(MORE))
                } else {
                    let mode = if self.video { "camera" } else { "video" };
                    Bars::actions()
                        .over()
                        .chip(4, photos)
                        .chip(5, Chip::new(mode, "").runs(MODE).disabled(rec))
                        .chip(6, shutter)
                        .chip(7, Chip::new("sun", "").runs(LIGHT).on(self.ev != EV_ZERO))
                }
            }
            View::Gallery => Bars::top().chip(6, Chip::new("trash-2", "Delete").hold()),
            View::Full => {
                let photo = self.full_px.is_some();
                let most = self.zooms().len() - 1;
                let b = Bars::actions()
                    .over()
                    .chip(6, Chip::new("trash-2", "Delete").hold());
                if photo {
                    b.chip(
                        5,
                        Chip::new("zoom-out", "")
                            .runs(ZOOM_OUT)
                            .disabled(self.zoom_at == 0),
                    )
                    .chip(
                        7,
                        Chip::new("zoom-in", "")
                            .runs(ZOOM_IN)
                            .disabled(self.zoom_at >= most),
                    )
                } else {
                    b
                }
            }
        }
    }

    fn frame_ms(&self) -> u64 {
        40
    }

    fn start(&mut self, cx: &mut Cx) {
        self.load(cx);
        self.refresh();
        self.launch(cx);
        self.want_last_thumb();
    }

    fn stop(&mut self, cx: &mut Cx) {
        self.quitting = true;
        self.cancel_count(cx);
        // Quit finishes a recording before the helper exits.
        self.send(Cmd::Quit);
    }

    fn front(&mut self, front: bool, cx: &mut Cx) {
        if front {
            if self.view == View::Finder {
                self.restart();
            }
            cx.redraw();
            return;
        }
        self.cancel_count(cx);
        // The app is stopped right after this, so it can't wait for
        // `recdone`: the helper finishes the file on its own.
        if self.rec.take().is_some() {
            self.send(Cmd::RecStop);
            cx.keep_awake(false);
        }
        self.send(Cmd::Stop);
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
        if code != Some(0) {
            eprintln!("gh0st-camera: the helper exited: {code:?}");
        }
        self.gone(cx);
        cx.redraw();
    }

    fn timer(&mut self, _: Timer, cx: &mut Cx) {
        let Some((n, t)) = &mut self.count else {
            return;
        };
        *n -= 1;
        if *n == 0 {
            let t = *t;
            cx.cancel(t);
            self.count = None;
            self.take();
        }
    }

    fn back(&mut self, cx: &mut Cx) -> bool {
        match self.view {
            View::Full => {
                self.view = View::Gallery;
                self.info = false;
                self.full_pic = Pic::default();
                self.asks.retain(|a| a.what != For::Full);
                self.want_tiles();
                false
            }
            View::Gallery => {
                self.close_review();
                false
            }
            View::Finder => {
                if self.light {
                    self.light = false;
                    return false;
                }
                !self.cancel_count(cx)
            }
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Soft(6) && self.view != View::Finder {
            self.delete(cx);
        }
    }

    fn commands(&self, c: &mut Commands) {
        let rec = self.rec.is_some();
        let live = self.phase == Phase::Live;
        if self.view == View::Gallery {
            c.add(
                Command::action(PREV, "Left")
                    .keys("z")
                    .keys_only()
                    .repeats(),
            );
            c.add(
                Command::action(NEXT, "Right")
                    .keys("c")
                    .keys_only()
                    .repeats(),
            );
            c.add(Command::action(UP, "Up").keys("f").keys_only().repeats());
            c.add(
                Command::action(DOWN, "Down")
                    .keys("x")
                    .keys_only()
                    .repeats(),
            );
            c.add(Command::action(FULL, "Open").keys("Enter").keys_only());
            return;
        }
        if self.view == View::Full {
            let zoomed = self.zoom_at > 0;
            let (l, r) = if zoomed {
                ("Look left", "Look right")
            } else {
                ("Newer", "Older")
            };
            c.add(Command::action(PREV, l).keys("z").keys_only().repeats());
            c.add(Command::action(NEXT, r).keys("c").keys_only().repeats());
            if zoomed {
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
            }
            c.add(Command::action(ZOOM_OUT, "Zoom out").keys_only());
            c.add(Command::action(ZOOM_IN, "Zoom in").keys_only());
            c.add(Command::action(INFO, "Details").keys("Enter").keys_only());
            return;
        }
        c.group("Shoot");
        let shoot = match (self.video, rec) {
            (false, _) => "Take a photo",
            (true, false) => "Record",
            (true, true) => "Stop recording",
        };
        c.add(
            Command::action(SHOOT, shoot)
                .keys("Enter Space")
                .keys_only()
                .enabled(live),
        );
        let mode = if self.video { "Video" } else { "Photo" };
        c.add(
            Command::choice(MODE, "Mode", mode)
                .keys("Tab")
                .enabled(!rec),
        );
        c.add(Command::choice(ZOOM, "Zoom", ZOOMS[self.zoom].1).keys("z c"));
        c.add(Command::choice(EV, "Light", EVS[self.ev].1).keys("x f"));
        if self.light {
            c.add(Command::action(LESS, "Less light").keys_only());
            c.add(Command::action(MORE, "More light").keys_only());
        } else {
            c.add(Command::action(LIGHT, "Light").keys_only());
        }
        c.add(
            Command::action(PHOTOS, "Photos")
                .keys("p")
                .opens()
                .enabled(!rec),
        );
        c.group("Photo");
        let timer = match TIMERS[self.timer] {
            0 => "Off",
            3 => "3 s",
            _ => "10 s",
        };
        c.add(Command::choice(TIMER, "Timer", timer).keys("t"));
        c.add(Command::toggle(GRID, "Grid", self.grid).keys("g"));
        c.add(Command::choice(
            QUALITY,
            "Quality",
            QUALITIES[self.quality].1,
        ));
        c.add(Command::toggle(RAW, "Keep RAW (DNG)", self.raw));
        c.group("Video");
        let size = match self.size {
            VideoSize::Sharp1080 => "1080p sharp",
            VideoSize::Wide720 => "720p wide",
        };
        c.add(Command::choice(VIDEO, "Size", size).enabled(!rec));
        c.add(Command::toggle(MIC, "Sound", self.mic).enabled(!rec));
        c.group("Camera");
        let hz = if self.flicker60 { "60 Hz" } else { "50 Hz" };
        c.add(Command::choice(FLICKER, "Mains", hz));
    }

    fn command(&mut self, id: u32, step: Step, cx: &mut Cx) {
        let fwd = !matches!(step, Step::Prev);
        let stepped = |i: usize, n: usize| {
            if fwd {
                (i + 1).min(n - 1)
            } else {
                i.saturating_sub(1)
            }
        };
        match id {
            SHOOT => self.shoot(cx),
            MODE => {
                self.cancel_count(cx);
                self.video = !self.video;
                self.frame = Pic::default();
                self.send(Cmd::Stop);
                self.restart();
            }
            ZOOM => {
                self.zoom = stepped(self.zoom, ZOOMS.len());
                self.send(Cmd::Set("zoom".into(), ZOOMS[self.zoom].0.to_string()));
            }
            EV | LESS | MORE => {
                let up = match id {
                    LESS => false,
                    MORE => true,
                    _ => fwd,
                };
                self.ev = if up {
                    (self.ev + 1).min(EVS.len() - 1)
                } else {
                    self.ev.saturating_sub(1)
                };
                // The arrows show the control too, so you see what moved.
                self.light = true;
                self.send(Cmd::Set("ev".into(), EVS[self.ev].0.to_string()));
            }
            LIGHT => self.light = true,
            PHOTOS => self.open_review(cx),
            TIMER => self.timer = (self.timer + 1) % TIMERS.len(),
            GRID => self.grid = !self.grid,
            QUALITY => {
                self.quality = 1 - self.quality;
                self.send(Cmd::Set(
                    "quality".into(),
                    QUALITIES[self.quality].0.to_string(),
                ));
            }
            RAW => self.raw = !self.raw,
            VIDEO => {
                self.size = match self.size {
                    VideoSize::Sharp1080 => VideoSize::Wide720,
                    VideoSize::Wide720 => VideoSize::Sharp1080,
                };
                if self.video {
                    self.send(Cmd::Stop);
                    self.restart();
                }
            }
            MIC => self.mic = !self.mic,
            FLICKER => {
                self.flicker60 = !self.flicker60;
                let hz = if self.flicker60 { "60" } else { "50" };
                self.send(Cmd::Set("flicker".into(), hz.into()));
            }
            PREV | NEXT | UP | DOWN if self.view == View::Gallery => {
                let (dx, dy) = match id {
                    PREV => (-1, 0),
                    NEXT => (1, 0),
                    UP => (0, -1),
                    _ => (0, 1),
                };
                if self.tiles.step(dx, dy) {
                    self.want_tiles();
                }
            }
            PREV | NEXT | UP | DOWN if self.zoom_at > 0 => {
                // A third of the view at a time.
                let (fw, fh) = self.view_frac();
                match id {
                    PREV => self.centre.0 -= fw / 3.0,
                    NEXT => self.centre.0 += fw / 3.0,
                    UP => self.centre.1 -= fh / 3.0,
                    _ => self.centre.1 += fh / 3.0,
                }
                self.clamp_centre();
                self.want_full();
            }
            PREV => self.step(-1),
            NEXT => self.step(1),
            FULL => self.open_full(),
            ZOOM_IN | ZOOM_OUT => {
                let most = self.zooms().len() - 1;
                self.zoom_at = if id == ZOOM_IN {
                    (self.zoom_at + 1).min(most)
                } else {
                    self.zoom_at.saturating_sub(1)
                };
                self.clamp_centre();
                self.want_full();
            }
            INFO => self.info = !self.info,
            _ => {}
        }
        if !matches!(
            id,
            SHOOT
                | ZOOM
                | EV
                | LESS
                | MORE
                | LIGHT
                | PHOTOS
                | PREV
                | NEXT
                | UP
                | DOWN
                | FULL
                | ZOOM_IN
                | ZOOM_OUT
                | INFO
        ) {
            self.save(cx);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        if let Phase::Failed(why) = self.phase {
            if self.view == View::Finder {
                ui.message("camera", why);
                return;
            }
        }
        match self.view {
            View::Finder => self.draw_finder(ui.canvas(), &t),
            View::Gallery => self.draw_gallery(ui),
            View::Full => self.draw_full(ui.canvas(), &t),
        }
    }
}
