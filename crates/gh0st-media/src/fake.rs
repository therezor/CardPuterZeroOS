//! A stand-in for `gh0st-mediad`: the real protocol and ring, with painted
//! pictures. Tests, mockups and `--sim` runs use it, so a host needs no
//! decoders. What a file is comes from its name: `pano` is a panorama,
//! `tall` a strip, `broken` is damaged, `huge` too large, `movie` has two
//! sound tracks and subtitles.

use crate::helper::{ASK_SLOT, PLAY_SLOTS};
use crate::proto::{Cmd, Event, Fail, Flags, Info, Kind, PlayState, Tags};
use gh0st_app::frames::Ring;
use gh0st_app::rt;
use gh0st_app::sidecar::Link;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A picture every 40 ms while a video plays.
const FRAME_MS: u64 = 40;

/// The folder of the picture viewed last, each time the app said `forget`.
static FORGOTTEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// How often the app let go of a picture it viewed in `dir`: tests check
/// the helper is not left holding one.
pub fn forgotten(dir: &Path) -> usize {
    FORGOTTEN
        .lock()
        .map(|l| l.iter().filter(|d| d.as_path() == dir).count())
        .unwrap_or(0)
}

struct Play {
    id: u32,
    kind: Kind,
    seed: u32,
    w: u16,
    h: u16,
    pos_ms: u64,
    dur_ms: u64,
    /// The clock when it was at `pos_ms`, while playing.
    since: Option<u64>,
    speed: u16,
    told_s: u64,
    looped: bool,
}

struct Fake {
    link: Link,
    ring: Ring,
    seq: u32,
    slot: u32,
    next_frame: u64,
    play: Option<Play>,
    next: Option<(u32, String)>,
    viewed: Option<PathBuf>,
}

fn seed_of(path: &str) -> u32 {
    path.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

fn name_has(path: &str, word: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(word))
}

/// What the real helper's probe would say of a file with this name.
pub fn info_of(path: &str) -> Result<Info, Fail> {
    if name_has(path, "broken") {
        return Err(Fail::Corrupt);
    }
    if name_has(path, "huge") {
        return Err(Fail::TooBig);
    }
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut i = Info::default();
    match crate::files::look_of(Path::new(path)) {
        Some(crate::files::Look::Video) => {
            i.kind = Kind::Video;
            (i.w, i.h) = (1920, 1080);
            i.dur_ms = if name_has(path, "movie") {
                5_412_000
            } else {
                95_000
            };
            i.audio = if name_has(path, "movie") { 2 } else { 1 };
            i.subs = name_has(path, "movie") as u32;
            i.format = "matroska,webm".into();
            i.vcodec = "h264".into();
            i.acodec = "aac".into();
        }
        Some(crate::files::Look::Sound) => {
            i.kind = Kind::Audio;
            (i.w, i.h) = (600, 600);
            i.dur_ms = 180_000 + (seed_of(path) % 120) as u64 * 1000;
            i.audio = 1;
            i.format = ext.clone();
            i.acodec = ext.clone();
        }
        _ => {
            i.kind = if ext == "gif" {
                Kind::Anim
            } else {
                Kind::Image
            };
            (i.w, i.h) = if name_has(path, "pano") {
                (12_000, 1_600)
            } else if name_has(path, "tall") {
                (1_200, 5_000)
            } else if i.kind == Kind::Anim {
                (480, 270)
            } else {
                (4_032, 3_024)
            };
            i.format = ext.clone();
            i.vcodec = ext;
        }
    }
    Ok(i)
}

fn tags_of(path: &str) -> Tags {
    let p = Path::new(path);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (track, title) = match stem.split_once(' ') {
        Some((n, t)) if n.chars().all(|c| c.is_ascii_digit()) => (n.to_string(), t.to_string()),
        _ => (String::new(), stem.clone()),
    };
    Tags {
        title,
        artist: "Night Signals".into(),
        album: p
            .parent()
            .and_then(|d| d.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        track: track.trim_start_matches('0').to_string(),
        date: "2024".into(),
    }
}

/// The real helper's framing: the part of a W x H picture a w x h screen
/// shows at `zoom` around (cx, cy), and the size it is drawn at.
pub fn frame_for(
    big: (f64, f64),
    screen: (u16, u16),
    zoom: f32,
    centre: (f32, f32),
    fill: bool,
) -> (f64, f64, f64, f64, u16, u16) {
    let (bw, bh) = big;
    let (w, h) = (screen.0 as f64, screen.1 as f64);
    let mut scale = if fill {
        (w / bw).max(h / bh)
    } else {
        (w / bw).min(h / bh) * (zoom as f64).max(1.0)
    };
    if !fill {
        scale = scale.min(1.0);
    }
    let fw = bw.min(w / scale);
    let fh = bh.min(h / scale);
    let fx = (centre.0 as f64 * bw - fw / 2.0).clamp(0.0, bw - fw);
    let fy = (centre.1 as f64 * bh - fh / 2.0).clamp(0.0, bh - fh);
    let ow = ((fw * scale).round() as u16).clamp(1, screen.0);
    let oh = ((fh * scale).round() as u16).clamp(1, screen.1);
    (fx, fy, fw, fh, ow, oh)
}

fn rgb565(r: f32, g: f32, b: f32) -> u16 {
    let c = |v: f32, bits: u32| ((v.clamp(0.0, 1.0) * ((1 << bits) - 1) as f32).round()) as u16;
    c(r, 5) << 11 | c(g, 6) << 5 | c(b, 5)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// A landscape at (u, v), 0 to 1 across the whole picture. `aspect` keeps
/// the sun round on a panorama.
fn scene(u: f32, v: f32, seed: u32, aspect: f32, t: f32) -> [f32; 3] {
    let hue = (seed % 360) as f32 / 360.0;
    let sky_top = mix([0.20, 0.36, 0.62], [0.52, 0.28, 0.46], hue);
    let sky_low = mix([0.86, 0.80, 0.70], [0.95, 0.66, 0.46], hue);
    let mut c = mix(sky_top, sky_low, v / 0.6);
    let (sx, sy) = ((u - 0.7 - 0.02 * t) * aspect, v - 0.28);
    if (sx * sx + sy * sy).sqrt() < 0.07 {
        c = [1.0, 0.93, 0.72];
    }
    let far = 0.55 + 0.05 * (u * 9.0 + hue * 6.0).sin() + 0.03 * (u * 23.0).sin();
    if v > far {
        c = mix([0.36, 0.42, 0.56], [0.28, 0.34, 0.46], (v - far) / 0.2);
    }
    let near = 0.7 + 0.06 * (u * 5.0 + 1.3 + t * 0.1).sin() + 0.02 * (u * 17.0).sin();
    if v > near {
        c = mix([0.40, 0.60, 0.30], [0.16, 0.34, 0.14], (v - near) / 0.3);
    }
    c
}

/// A record sleeve: rings around a disc.
fn sleeve(u: f32, v: f32, seed: u32) -> [f32; 3] {
    let hue = (seed % 360) as f32 / 360.0;
    let (x, y) = (u - 0.5, v - 0.5);
    let r = (x * x + y * y).sqrt();
    let base = mix([0.12, 0.10, 0.22], [0.36, 0.12, 0.18], hue);
    let ring = mix([0.95, 0.55, 0.25], [0.30, 0.80, 0.85], hue);
    if r < 0.08 {
        [0.95, 0.95, 0.92]
    } else if r < 0.34 && ((r * 40.0) as i32) % 3 == 0 {
        ring
    } else {
        mix(base, [0.05, 0.05, 0.08], v)
    }
}

/// Paints the region (fx, fy, fw, fh) of a big x big picture into ow x oh.
#[allow(clippy::too_many_arguments)]
fn paint(
    px: &mut [u16],
    big: (f64, f64),
    region: (f64, f64, f64, f64),
    ow: u16,
    oh: u16,
    seed: u32,
    cover: bool,
    t: f32,
) {
    let (bw, bh) = big;
    let (fx, fy, fw, fh) = region;
    let aspect = (bw / bh) as f32;
    for y in 0..oh as usize {
        let v = ((fy + (y as f64 + 0.5) * fh / oh as f64) / bh) as f32;
        for x in 0..ow as usize {
            let u = ((fx + (x as f64 + 0.5) * fw / ow as f64) / bw) as f32;
            let c = if cover {
                sleeve(u, v, seed)
            } else {
                scene(u, v, seed, aspect, t)
            };
            px[y * ow as usize + x] = rgb565(c[0], c[1], c[2]);
        }
    }
}

/// Runs until `quit` or the app hangs up.
pub fn serve(link: Link, ring: Ring) {
    let mut f = Fake {
        link,
        ring,
        seq: 0,
        slot: 0,
        next_frame: 0,
        play: None,
        next: None,
        viewed: None,
    };
    loop {
        let now = rt::now_ms();
        f.tick(now);
        let playing = f.play.as_ref().is_some_and(|p| p.since.is_some());
        let deadline = if playing {
            Some(f.next_frame)
        } else {
            Some(now + 1000)
        };
        let mut fds = [rt::pollfd(f.link.fd())];
        let _ = rt::wait(&mut fds, deadline);
        while let Some(m) = f.link.recv() {
            let Some(cmd) = Cmd::parse(&m) else {
                continue;
            };
            if !f.run(cmd) {
                return;
            }
        }
        if f.link.closed() {
            return;
        }
    }
}

impl Fake {
    fn say(&self, e: Event) {
        let f = e.fields();
        let refs: Vec<&str> = f.iter().map(String::as_str).collect();
        self.link.send(&refs);
    }

    fn next_seq(&mut self) -> u32 {
        self.seq += 2;
        self.seq
    }

    fn picture(
        &mut self,
        id: u32,
        thumb: bool,
        path: &str,
        screen: (u16, u16),
        zoom: f32,
        centre: (f32, f32),
    ) {
        let info = match info_of(path) {
            Ok(i) => i,
            Err(why) => return self.say(Event::Fail { id, why }),
        };
        let cover = info.kind == Kind::Audio;
        // A song with no picture of its own, as the real helper says.
        if cover && name_has(path, "nocover") {
            return self.say(Event::Fail {
                id,
                why: Fail::Unsupported,
            });
        }
        let big = (info.w.max(1) as f64, info.h.max(1) as f64);
        let (fx, fy, fw, fh, ow, oh) = frame_for(big, screen, zoom, centre, thumb);
        let seq = self.next_seq();
        let seed = seed_of(path);
        self.ring.write(ASK_SLOT, seq, ow, oh, |px| {
            paint(px, big, (fx, fy, fw, fh), ow, oh, seed, cover, 0.0)
        });
        let (slot, w, h) = (ASK_SLOT, ow, oh);
        self.say(if thumb {
            Event::Thumb {
                id,
                slot,
                seq,
                w,
                h,
            }
        } else {
            Event::View {
                id,
                slot,
                seq,
                w,
                h,
            }
        });
    }

    fn open(&mut self, id: u32, size: (u16, u16), flags: &Flags, path: &str) {
        let (w, h) = size;
        let (paused, looped, at) = (flags.paused, flags.looped, flags.at_ms);
        let info = match info_of(path) {
            Ok(i) => i,
            Err(f) => {
                return self.say(Event::PlayErr {
                    id,
                    why: f.says().into(),
                });
            }
        };
        let (iw, ih) = (info.w.max(1) as f64, info.h.max(1) as f64);
        let k = (w as f64 / iw).min(h as f64 / ih);
        let now = rt::now_ms();
        self.play = Some(Play {
            id,
            kind: info.kind,
            seed: seed_of(path),
            w: ((iw * k) as u16 & !1).max(2),
            h: ((ih * k) as u16 & !1).max(2),
            pos_ms: at,
            dur_ms: if info.kind == Kind::Anim {
                960
            } else {
                info.dur_ms
            },
            since: (!paused).then_some(now),
            speed: 100,
            told_s: u64::MAX,
            looped: looped || info.kind == Kind::Anim,
        });
        self.next_frame = now;
        self.status();
        if info.kind != Kind::Audio {
            self.frame(now);
        }
    }

    fn pos(&self, now: u64) -> u64 {
        let Some(p) = &self.play else {
            return 0;
        };
        match p.since {
            Some(t) => p.pos_ms + (now - t) * p.speed as u64 / 100,
            None => p.pos_ms,
        }
    }

    fn status(&mut self) {
        let now = rt::now_ms();
        let pos = self.pos(now);
        let Some(p) = &self.play else {
            return;
        };
        let state = if p.since.is_some() {
            PlayState::Playing
        } else if p.dur_ms > 0 && p.pos_ms >= p.dur_ms {
            PlayState::Done
        } else {
            PlayState::Paused
        };
        self.say(Event::Play {
            id: p.id,
            pos_ms: pos.min(p.dur_ms),
            dur_ms: p.dur_ms,
            state,
            dropped: 0,
            slow: 0,
        });
    }

    fn frame(&mut self, now: u64) {
        let pos = self.pos(now);
        let Some(p) = &self.play else {
            return;
        };
        if p.kind == Kind::Audio {
            return;
        }
        let (w, h, seed) = (p.w, p.h, p.seed);
        let t = pos as f32 / 1000.0;
        self.slot = (self.slot + 1) % PLAY_SLOTS;
        let seq = self.next_seq();
        let slot = self.slot;
        let big = (w as f64, h as f64);
        self.ring.write(slot, seq, w, h, |px| {
            paint(px, big, (0.0, 0.0, big.0, big.1), w, h, seed, false, t)
        });
        self.say(Event::Frame { slot, seq, w, h });
    }

    fn tick(&mut self, now: u64) {
        let pos = self.pos(now);
        let Some(p) = &mut self.play else {
            return;
        };
        if p.since.is_none() {
            return;
        }
        if p.dur_ms > 0 && pos >= p.dur_ms {
            if p.looped {
                p.pos_ms = 0;
                p.since = Some(now);
            } else if let Some((id, path)) = self.next.take() {
                let (w, h) = (p.w, p.h);
                self.open(id, (w, h), &Flags::default(), &path);
                return;
            } else {
                p.pos_ms = p.dur_ms;
                p.since = None;
                self.status();
                return;
            }
        }
        if now >= self.next_frame {
            self.next_frame = now + FRAME_MS;
            self.frame(now);
        }
        let s = pos / 1000;
        let told = self.play.as_ref().map_or(0, |p| p.told_s);
        if s != told {
            if let Some(p) = &mut self.play {
                p.told_s = s;
            }
            self.status();
        }
    }

    fn set_paused(&mut self, paused: bool) {
        let now = rt::now_ms();
        let pos = self.pos(now);
        if let Some(p) = &mut self.play {
            if paused {
                p.pos_ms = pos.min(p.dur_ms);
                p.since = None;
            } else {
                if p.dur_ms > 0 && p.pos_ms >= p.dur_ms {
                    p.pos_ms = 0;
                }
                p.since = Some(now);
            }
        }
        self.status();
    }

    fn run(&mut self, cmd: Cmd) -> bool {
        // The fake reads names; the real helper reads bytes.
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        match cmd {
            Cmd::Hello => self.say(Event::Hello(crate::proto::VERSION.into())),
            Cmd::Quit => return false,
            Cmd::Forget => {
                let dir = self.viewed.take();
                if let (Some(d), Ok(mut l)) =
                    (dir.as_ref().and_then(|p| p.parent()), FORGOTTEN.lock())
                {
                    l.push(d.to_path_buf());
                }
            }
            Cmd::Probe { id, path } => match info_of(&s(&path)) {
                Ok(info) => self.say(Event::Info { id, info }),
                Err(why) => self.say(Event::Fail { id, why }),
            },
            Cmd::Tags { id, path } => self.say(Event::Tags {
                id,
                tags: tags_of(&s(&path)),
            }),
            Cmd::Thumb { id, w, h, path } => {
                self.picture(id, true, &s(&path), (w, h), 1.0, (0.5, 0.5))
            }
            Cmd::View {
                id,
                w,
                h,
                zoom,
                cx,
                cy,
                path,
            } => {
                self.picture(id, false, &s(&path), (w, h), zoom, (cx, cy));
                self.viewed = Some(path);
            }
            Cmd::PlayOpen {
                id,
                w,
                h,
                flags,
                path,
            } => self.open(id, (w, h), &flags, &s(&path)),
            Cmd::PlayNext { id, path } => self.next = Some((id, s(&path))),
            Cmd::Pause => self.set_paused(true),
            Cmd::Resume => self.set_paused(false),
            Cmd::Stop => {
                self.play = None;
                self.next = None;
            }
            Cmd::Seek(ms) => {
                let now = rt::now_ms();
                if let Some(p) = &mut self.play {
                    p.pos_ms = ms.min(p.dur_ms);
                    if p.since.is_some() {
                        p.since = Some(now);
                    }
                }
                self.status();
                self.frame(now);
            }
            Cmd::Step => {
                let now = rt::now_ms();
                if let Some(p) = &mut self.play {
                    p.pos_ms = (p.pos_ms + 33).min(p.dur_ms);
                }
                self.frame(now);
                self.status();
            }
            Cmd::Speed(s) => {
                let now = rt::now_ms();
                let pos = self.pos(now);
                if let Some(p) = &mut self.play {
                    p.pos_ms = pos;
                    if p.since.is_some() {
                        p.since = Some(now);
                    }
                    p.speed = s.clamp(50, 200);
                }
            }
            Cmd::Repeat(_) | Cmd::Tracks { .. } => {}
        }
        true
    }
}

/// `gh0st-media --fake-helper`: the fake as a process, for `--sim` runs.
pub fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with a signal number.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    match crate::helper::inherited() {
        Ok((link, ring)) => {
            serve(link, ring);
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gh0st-media --fake-helper: {e}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_frames_as_the_helper_does() {
        // A 4:3 photo fitted in the screen: as tall as it, 227 wide.
        let (_, _, _, _, w, h) = frame_for((4032.0, 3024.0), (320, 170), 1.0, (0.5, 0.5), false);
        assert_eq!((w, h), (227, 170));
        // A tile fills: cut to exactly its size.
        let (_, _, _, _, w, h) = frame_for((12000.0, 1600.0), (60, 45), 1.0, (0.5, 0.5), true);
        assert_eq!((w, h), (60, 45));
    }

    #[test]
    fn names_say_what_a_file_is() {
        assert_eq!(info_of("/a/pano.jpg").unwrap().w, 12_000);
        assert_eq!(info_of("/a/broken.png"), Err(Fail::Corrupt));
        assert_eq!(info_of("/a/x.flac").unwrap().kind, Kind::Audio);
        assert_eq!(info_of("/a/movie.mkv").unwrap().audio, 2);
        assert_eq!(tags_of("/m/Album/03 Night Drive.mp3").track, "3");
    }
}
