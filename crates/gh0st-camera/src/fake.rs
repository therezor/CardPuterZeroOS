//! A stand-in for the camera helper: the real protocol and ring, with a
//! painted landscape for a sensor. Tests, mockups and `--sim` runs use it,
//! so nothing on a host needs libcamera. Files it "saves" hold a few bytes.

use crate::proto::{Cmd, Event, Mode, Playing, State};
use crate::ring::{PREVIEW_SLOTS, Ring, THUMB_SLOT};
use gh0st_core::link::Link;
use gh0st_core::rt;

/// A preview frame every 40 ms, as the real helper paces them.
const FRAME_MS: u64 = 40;
/// What a second of video "costs": 12 Mb/s and AAC.
const VIDEO_BYTES_PER_S: u64 = 1_560_000;
/// Every video it plays lasts this long.
const PLAY_MS: u64 = 6000;

/// A video playing: where it is, and since when it moves on its own.
struct Play {
    seed: u32,
    w: u16,
    h: u16,
    pos_ms: u64,
    /// The clock when it was at `pos_ms`, while playing.
    since: Option<u64>,
    told_s: u64,
}

struct Fake {
    link: Link,
    ring: Ring,
    mode: Mode,
    live: bool,
    zoom: f32,
    ev: f32,
    seq: u32,
    slot: u32,
    next_frame: u64,
    rec: Option<(String, u64)>,
    next_tick: u64,
    camera: bool,
    play: Option<Play>,
}

/// Runs until `quit` or the app hangs up. Without a `camera`, `start`
/// fails as on a Lite.
pub fn serve(link: Link, ring: Ring, camera: bool) {
    let mut f = Fake {
        link,
        ring,
        mode: Mode::Photo,
        live: false,
        zoom: 1.0,
        ev: 0.0,
        seq: 0,
        slot: 0,
        next_frame: 0,
        rec: None,
        next_tick: 0,
        camera,
        play: None,
    };
    // A process helper outlives a killed app on hosts without PDEATHSIG,
    // and a datagram pair never reports the hang-up: watch the parent.
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with a signal number.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    // SAFETY: getppid can't fail.
    let parent = unsafe { libc::getppid() };
    loop {
        // SAFETY: as above.
        if unsafe { libc::getppid() } != parent {
            f.finish();
            return;
        }
        let now = rt::now_ms();
        if f.live && now >= f.next_frame {
            f.frame();
            f.next_frame = now + FRAME_MS;
        }
        if let Some((_, start)) = &f.rec {
            if now >= f.next_tick {
                let ms = now - start;
                f.say(Event::Rec {
                    ms,
                    bytes: ms * VIDEO_BYTES_PER_S / 1000,
                });
                f.next_tick += 1000;
            }
        }
        if f.play.as_ref().is_some_and(|p| p.since.is_some()) && now >= f.next_frame {
            f.play_frame(now);
            f.next_frame = now + FRAME_MS;
        }
        let mut deadline = None;
        if f.live || f.play.as_ref().is_some_and(|p| p.since.is_some()) {
            deadline = Some(f.next_frame);
        }
        if f.rec.is_some() {
            deadline = Some(deadline.map_or(f.next_tick, |d: u64| d.min(f.next_tick)));
        }
        let check = rt::now_ms() + 1000;
        let deadline = Some(deadline.map_or(check, |d| d.min(check)));
        let mut fds = [rt::pollfd(f.link.fd())];
        let _ = rt::wait(&mut fds, deadline);
        while let Some(m) = f.link.recv() {
            let Some(cmd) = Cmd::parse(&m) else {
                continue;
            };
            if !f.run(cmd) {
                f.finish();
                return;
            }
        }
        if f.link.closed() {
            f.finish();
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

    /// False on `quit`.
    fn run(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Hello => self.say(Event::Hello {
                sensor: "fake".into(),
                w: 3280,
                h: 2464,
            }),
            Cmd::Mode(m) => self.mode = m,
            Cmd::Start if !self.camera => {
                self.say(Event::Error(crate::proto::Fault::NoCamera));
            }
            Cmd::Start => {
                self.live = true;
                self.next_frame = 0;
                self.say(Event::State(State::Preview));
                self.frame();
            }
            Cmd::Stop => {
                self.finish();
                self.play = None;
                self.live = false;
                self.say(Event::State(State::Off));
            }
            Cmd::Quit => return false,
            Cmd::Set(k, v) => {
                let v: f32 = v.parse().unwrap_or(0.0);
                match k.as_str() {
                    "zoom" => self.zoom = v.clamp(1.0, 4.0),
                    "ev" => self.ev = v.clamp(-2.0, 2.0),
                    _ => {}
                }
            }
            Cmd::Shoot { path, .. } => {
                self.say(Event::State(State::Busy));
                let result = std::fs::write(&path, b"\xff\xd8fake\xff\xd9")
                    .map(|_| 0)
                    .map_err(|e| e.to_string());
                self.say(Event::Shot { path, result });
                self.say(Event::State(State::Preview));
            }
            Cmd::RecStart { path, .. } => {
                if std::fs::write(&path, b"fake mp4").is_ok() {
                    let now = rt::now_ms();
                    self.rec = Some((path, now));
                    self.next_tick = now + 1000;
                    self.say(Event::State(State::Rec));
                } else {
                    self.say(Event::Error(crate::proto::Fault::Io(path)));
                }
            }
            Cmd::RecStop => {
                self.finish();
                self.say(Event::State(State::Preview));
            }
            Cmd::Mics => self.say(Event::Mics(vec![
                (
                    "alsa_input.platform-sound.stereo".into(),
                    "Built-in Audio".into(),
                ),
                ("alsa_input.usb-Mic.mono".into(), "USB Mic".into()),
            ])),
            Cmd::Play { path, w, h } => {
                self.finish();
                self.live = false;
                let now = rt::now_ms();
                // The picture fills the width at 16:9, as ours are.
                let ph = (w as u32 * 9 / 16).min(h as u32) as u16 & !1;
                self.play = Some(Play {
                    seed: seed_of(&path),
                    w,
                    h: ph,
                    pos_ms: 0,
                    since: Some(now),
                    told_s: u64::MAX,
                });
                self.next_frame = 0;
                self.play_frame(now);
            }
            Cmd::PlayPause => {
                let now = rt::now_ms();
                if let Some(p) = &mut self.play {
                    p.pos_ms = Self::pos(p, now);
                    p.since = None;
                }
                self.tell(Playing::Paused);
            }
            Cmd::PlayResume => {
                let now = rt::now_ms();
                if let Some(p) = &mut self.play {
                    if p.pos_ms >= PLAY_MS {
                        p.pos_ms = 0;
                    }
                    p.since = Some(now);
                }
                self.tell(Playing::Playing);
            }
            Cmd::PlaySeek(ms) => {
                let now = rt::now_ms();
                let mut state = Playing::Paused;
                if let Some(p) = &mut self.play {
                    p.pos_ms = ms.min(PLAY_MS);
                    if p.since.is_some() {
                        p.since = Some(now);
                        state = Playing::Playing;
                    }
                }
                self.play_frame(now);
                self.tell(state);
            }
            Cmd::PlayStop => self.play = None,
            Cmd::Thumb {
                path,
                w,
                h,
                zoom,
                cx,
                cy,
            } => {
                if !std::path::Path::new(&path).exists() {
                    self.say(Event::NoThumb { path });
                    return true;
                }
                let seq = self.next_seq();
                let seed = seed_of(&path);
                self.ring.write(THUMB_SLOT, seq, w, h, |px| {
                    paint(px, w, h, zoom.max(1.0), (cx, cy), 0.0, seed);
                });
                self.say(Event::Thumb {
                    path,
                    slot: THUMB_SLOT,
                    seq,
                    w,
                    h,
                });
            }
        }
        true
    }

    fn finish(&mut self) {
        if let Some((path, _)) = self.rec.take() {
            self.say(Event::RecDone { path });
        }
    }

    fn pos(p: &Play, now: u64) -> u64 {
        match p.since {
            Some(t) => (p.pos_ms + now.saturating_sub(t)).min(PLAY_MS),
            None => p.pos_ms,
        }
    }

    fn tell(&self, state: Playing) {
        if let Some(p) = &self.play {
            self.say(Event::Play {
                pos_ms: p.pos_ms,
                dur_ms: PLAY_MS,
                state,
            });
        }
    }

    /// The video's picture now: the landscape with its sun crossing the
    /// sky as it plays. At the end it stops there.
    fn play_frame(&mut self, now: u64) {
        let Some(p) = &mut self.play else {
            return;
        };
        let pos = Self::pos(p, now);
        let done = pos >= PLAY_MS && p.since.is_some();
        if p.since.is_some() {
            p.pos_ms = pos;
            p.since = (!done).then_some(now);
        }
        let (w, h, seed) = (p.w, p.h, p.seed.wrapping_add((pos / 400) as u32));
        let second = pos / 1000;
        let tell = second != p.told_s || done;
        p.told_s = second;
        let seq = self.next_seq();
        self.slot = (self.slot + 1) % PREVIEW_SLOTS;
        self.ring.write(self.slot, seq, w, h, |px| {
            paint(px, w, h, 1.0, (0.5, 0.5), 0.0, seed)
        });
        self.say(Event::Frame {
            slot: self.slot,
            seq,
            w,
            h,
        });
        if tell {
            self.tell(if done {
                Playing::Done
            } else if self.play.as_ref().is_some_and(|p| p.since.is_some()) {
                Playing::Playing
            } else {
                Playing::Paused
            });
        }
    }

    fn frame(&mut self) {
        let video = matches!(self.mode, Mode::Video(_));
        let (w, h) = crate::app::preview_size(&gh0st_app::gfx::theme::CARDPUTER, video);
        let (w, h) = (w as u16, h as u16);
        let seq = self.next_seq();
        self.slot = (self.slot + 1) % PREVIEW_SLOTS;
        let (zoom, ev) = (self.zoom, self.ev);
        self.ring.write(self.slot, seq, w, h, |px| {
            paint(px, w, h, zoom, (0.5, 0.5), ev, 0)
        });
        self.say(Event::Frame {
            slot: self.slot,
            seq,
            w,
            h,
        });
    }
}

fn seed_of(path: &str) -> u32 {
    path.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

fn rgb565(r: f32, g: f32, b: f32) -> u16 {
    let q = |v: f32, max: f32| (v.clamp(0.0, 1.0) * max + 0.5) as u16;
    (q(r, 31.0) << 11) | (q(g, 63.0) << 5) | q(b, 31.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// A landscape seen through a 4:3 sensor: sky, sun, hills and a tree. A
/// wider frame shows the middle band of it, as a 16:9 crop does. `seed`
/// moves the sun and tints the light, so thumbnails differ. `centre` is
/// where a zoomed view looks, 0 to 1 across.
pub fn paint(px: &mut [u16], w: u16, h: u16, zoom: f32, centre: (f32, f32), ev: f32, seed: u32) {
    let (w, h) = (w as usize, h as usize);
    let gain = 2f32.powf(ev * 0.5);
    let sun_x = 0.62 + (seed % 7) as f32 * 0.04;
    let warm = (seed % 5) as f32 * 0.04;
    // Sensor height a frame row spans: all of it at 4:3, less when wider.
    let span = (w as f32 / h as f32) / (4.0 / 3.0);
    for y in 0..h {
        for x in 0..w {
            let u = centre.0 + (x as f32 / w as f32 - 0.5) / zoom;
            let v = centre.1 + (y as f32 / h as f32 - 0.5) / (zoom * span);
            let mut c = mix([0.30, 0.50, 0.82], [0.86, 0.86 + warm, 0.92], v / 0.55);
            let (dx, dy) = ((u - sun_x) * 4.0 / 3.0, v - 0.24);
            let d = (dx * dx + dy * dy).sqrt();
            c = mix(c, [1.0, 0.95, 0.75], (0.16 - d) / 0.1);
            if d < 0.06 {
                c = [1.0, 0.97, 0.85];
            }
            let far = 0.53 + 0.05 * (6.0 * u + 1.0 + (seed % 628) as f32 * 0.01).sin();
            if v > far {
                c = mix([0.48, 0.60, 0.70], [0.40, 0.52, 0.60], (v - far) / 0.2);
            }
            let near = 0.66 + 0.06 * (4.0 * u + 1.3).sin() + 0.025 * (11.0 * u).sin();
            if v > near {
                c = mix([0.42, 0.66, 0.32], [0.18, 0.40, 0.16], (v - near) / 0.3);
            }
            let (tx, ty) = ((u - 0.26) * 4.0 / 3.0, v - 0.6);
            if (tx * tx + ty * ty).sqrt() < 0.075 {
                c = mix([0.16, 0.38, 0.18], [0.10, 0.26, 0.12], (ty + 0.075) / 0.15);
            } else if tx.abs() < 0.012 && v > 0.6 && v < 0.74 {
                c = [0.30, 0.22, 0.16];
            }
            px[y * w + x] = rgb565(c[0] * gain, c[1] * gain, c[2] * gain);
        }
    }
}

/// `gh0st-camera --fake-helper`: the fake as a process, for `--sim` runs.
pub fn main() -> std::process::ExitCode {
    match crate::helper::inherited() {
        Ok((link, ring)) => {
            serve(link, ring, true);
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gh0st-camera --fake-helper: {e}");
            std::process::ExitCode::from(2)
        }
    }
}
