//! What the app and the camera helper say to each other: one message per
//! packet, fields split by tabs, as on the shell link. docs/camera.md has
//! the full list.

/// The protocol version both sides send in `hello`.
pub const VERSION: &str = "1";

/// A video size. Each is the best the sensor gives at that size: no
/// upscaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSize {
    /// 1920x1080 from the sensor's 1080p crop: sharp, a narrower view.
    Sharp1080,
    /// 1280x720 from the whole sensor, binned: the full view.
    Wide720,
}

impl VideoSize {
    pub fn token(self) -> &'static str {
        match self {
            VideoSize::Sharp1080 => "1080",
            VideoSize::Wide720 => "720",
        }
    }

    pub fn from_token(s: &str) -> Option<VideoSize> {
        match s {
            "1080" => Some(VideoSize::Sharp1080),
            "720" => Some(VideoSize::Wide720),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Photo,
    Video(VideoSize),
}

/// App to helper.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Hello,
    /// What `start` streams. Sent while stopped.
    Mode(Mode),
    /// Opens the sensor and streams preview frames.
    Start,
    /// Closes the sensor: no frames, no CMA held.
    Stop,
    Quit,
    /// A control: `zoom` (1.0 to 4.0), `ev` (-2.0 to 2.0), `flicker` (50
    /// or 60), `quality` (JPEG, 50 to 100).
    Set(String, String),
    /// A still to `path`, with a DNG beside it when `raw`.
    Shoot {
        path: String,
        raw: bool,
    },
    RecStart {
        path: String,
        mic: bool,
    },
    RecStop,
    /// A picture of `path` (a JPEG or an MP4) in exactly w x h: fitted at
    /// zoom 1, or `zoom` times closer around (cx, cy), each 0 to 1 across
    /// the picture. Answered in order, by `thumb` or `nothumb`.
    Thumb {
        path: String,
        w: u16,
        h: u16,
        zoom: f32,
        cx: f32,
        cy: f32,
    },
}

/// The helper's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Preview,
    /// Taking a still: no frames until it is done.
    Busy,
    Rec,
}

/// Why the helper can't go on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// No sensor: a Lite, or the overlay is missing.
    NoCamera,
    /// Out of CMA for this mode.
    NoMemory,
    /// The sensor is open in another program.
    Busy,
    /// Writing a file failed.
    Io(String),
    Other(String),
}

/// Helper to app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Hello {
        sensor: String,
        w: u32,
        h: u32,
    },
    State(State),
    /// A preview frame waits in ring slot `slot`.
    Frame {
        slot: u32,
        seq: u32,
        w: u16,
        h: u16,
    },
    /// The still is saved, and how long it took from `shoot`.
    Shot {
        path: String,
        result: Result<u32, String>,
    },
    /// Once a second while recording: time and bytes so far.
    Rec {
        ms: u64,
        bytes: u64,
    },
    RecDone {
        path: String,
    },
    /// The picture asked for by `thumb` could not be made.
    NoThumb {
        path: String,
    },
    /// The picture asked for by `thumb`.
    Thumb {
        path: String,
        slot: u32,
        seq: u32,
        w: u16,
        h: u16,
    },
    Error(Fault),
}

fn num<T: std::str::FromStr>(s: Option<&String>) -> Option<T> {
    s?.parse().ok()
}

impl Cmd {
    pub fn fields(&self) -> Vec<String> {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
        match self {
            Cmd::Hello => v(&["hello", VERSION]),
            Cmd::Mode(Mode::Photo) => v(&["mode", "photo"]),
            Cmd::Mode(Mode::Video(s)) => v(&["mode", "video", s.token()]),
            Cmd::Start => v(&["start"]),
            Cmd::Stop => v(&["stop"]),
            Cmd::Quit => v(&["quit"]),
            Cmd::Set(k, val) => v(&["set", k, val]),
            Cmd::Shoot { path, raw } => {
                let mut f = v(&["shoot", path]);
                if *raw {
                    f.push("raw".into());
                }
                f
            }
            Cmd::RecStart { path, mic } => {
                v(&["rec", "start", path, if *mic { "mic" } else { "" }])
            }
            Cmd::RecStop => v(&["rec", "stop"]),
            Cmd::Thumb {
                path,
                w,
                h,
                zoom,
                cx,
                cy,
            } => v(&[
                "thumb",
                path,
                &w.to_string(),
                &h.to_string(),
                &zoom.to_string(),
                &cx.to_string(),
                &cy.to_string(),
            ]),
        }
    }

    pub fn parse(f: &[String]) -> Option<Cmd> {
        let s = |i: usize| f.get(i).map(String::as_str);
        Some(match (s(0)?, s(1)) {
            ("hello", _) => Cmd::Hello,
            ("mode", Some("photo")) => Cmd::Mode(Mode::Photo),
            ("mode", Some("video")) => Cmd::Mode(Mode::Video(VideoSize::from_token(s(2)?)?)),
            ("start", _) => Cmd::Start,
            ("stop", _) => Cmd::Stop,
            ("quit", _) => Cmd::Quit,
            ("set", Some(k)) => Cmd::Set(k.into(), s(2)?.into()),
            ("shoot", Some(p)) => Cmd::Shoot {
                path: p.into(),
                raw: s(2) == Some("raw"),
            },
            ("rec", Some("start")) => Cmd::RecStart {
                path: s(2)?.into(),
                mic: s(3) == Some("mic"),
            },
            ("rec", Some("stop")) => Cmd::RecStop,
            ("thumb", Some(p)) => Cmd::Thumb {
                path: p.into(),
                w: num(f.get(2))?,
                h: num(f.get(3))?,
                zoom: num(f.get(4)).unwrap_or(1.0),
                cx: num(f.get(5)).unwrap_or(0.5),
                cy: num(f.get(6)).unwrap_or(0.5),
            },
            _ => return None,
        })
    }
}

impl Event {
    pub fn fields(&self) -> Vec<String> {
        let mut f: Vec<String> = Vec::new();
        let mut put = |a: &[&dyn ToString]| f.extend(a.iter().map(|x| x.to_string()));
        match self {
            Event::Hello { sensor, w, h } => put(&[&"hello", &VERSION, sensor, w, h]),
            Event::State(s) => put(&[
                &"state",
                &match s {
                    State::Off => "off",
                    State::Preview => "preview",
                    State::Busy => "busy",
                    State::Rec => "rec",
                },
            ]),
            Event::Frame { slot, seq, w, h } => put(&[&"frame", slot, seq, w, h]),
            Event::Shot {
                path,
                result: Ok(ms),
            } => put(&[&"shot", &"ok", path, ms]),
            Event::Shot {
                path,
                result: Err(e),
            } => put(&[&"shot", &"err", path, e]),
            Event::Rec { ms, bytes } => put(&[&"rec", ms, bytes]),
            Event::RecDone { path } => put(&[&"recdone", path]),
            Event::NoThumb { path } => put(&[&"nothumb", path]),
            Event::Thumb {
                path,
                slot,
                seq,
                w,
                h,
            } => put(&[&"thumb", path, slot, seq, w, h]),
            Event::Error(e) => match e {
                Fault::NoCamera => put(&[&"error", &"nocamera"]),
                Fault::NoMemory => put(&[&"error", &"nomem"]),
                Fault::Busy => put(&[&"error", &"busy"]),
                Fault::Io(t) => put(&[&"error", &"io", t]),
                Fault::Other(t) => put(&[&"error", &"other", t]),
            },
        }
        f
    }

    pub fn parse(f: &[String]) -> Option<Event> {
        let s = |i: usize| f.get(i).map(String::as_str);
        let n = |i: usize| f.get(i);
        Some(match s(0)? {
            "hello" => Event::Hello {
                sensor: s(2).unwrap_or("").into(),
                w: num(n(3)).unwrap_or(0),
                h: num(n(4)).unwrap_or(0),
            },
            "state" => Event::State(match s(1)? {
                "off" => State::Off,
                "preview" => State::Preview,
                "busy" => State::Busy,
                "rec" => State::Rec,
                _ => return None,
            }),
            "frame" => Event::Frame {
                slot: num(n(1))?,
                seq: num(n(2))?,
                w: num(n(3))?,
                h: num(n(4))?,
            },
            "shot" => Event::Shot {
                path: s(2)?.into(),
                result: match s(1)? {
                    "ok" => Ok(num(n(3)).unwrap_or(0)),
                    _ => Err(s(3).unwrap_or("").into()),
                },
            },
            "rec" => Event::Rec {
                ms: num(n(1))?,
                bytes: num(n(2))?,
            },
            "recdone" => Event::RecDone { path: s(1)?.into() },
            "nothumb" => Event::NoThumb { path: s(1)?.into() },
            "thumb" => Event::Thumb {
                path: s(1)?.into(),
                slot: num(n(2))?,
                seq: num(n(3))?,
                w: num(n(4))?,
                h: num(n(5))?,
            },
            "error" => Event::Error(match s(1)? {
                "nocamera" => Fault::NoCamera,
                "nomem" => Fault::NoMemory,
                "busy" => Fault::Busy,
                "io" => Fault::Io(s(2).unwrap_or("").into()),
                _ => Fault::Other(s(2).unwrap_or("").into()),
            }),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_survives_the_wire() {
        let all = [
            Cmd::Hello,
            Cmd::Mode(Mode::Photo),
            Cmd::Mode(Mode::Video(VideoSize::Wide720)),
            Cmd::Start,
            Cmd::Stop,
            Cmd::Quit,
            Cmd::Set("zoom".into(), "2.0".into()),
            Cmd::Shoot {
                path: "/p/CAM_1.jpg".into(),
                raw: true,
            },
            Cmd::Shoot {
                path: "/p/CAM_2.jpg".into(),
                raw: false,
            },
            Cmd::RecStart {
                path: "/v.mp4".into(),
                mic: true,
            },
            Cmd::RecStart {
                path: "/v.mp4".into(),
                mic: false,
            },
            Cmd::RecStop,
            Cmd::Thumb {
                path: "/p/x.jpg".into(),
                w: 40,
                h: 30,
                zoom: 4.0,
                cx: 0.25,
                cy: 0.75,
            },
        ];
        for c in all {
            assert_eq!(Cmd::parse(&c.fields()), Some(c.clone()), "{c:?}");
        }
    }

    #[test]
    fn every_event_survives_the_wire() {
        let all = [
            Event::Hello {
                sensor: "imx219".into(),
                w: 3280,
                h: 2464,
            },
            Event::State(State::Rec),
            Event::Frame {
                slot: 2,
                seq: 41,
                w: 226,
                h: 170,
            },
            Event::Shot {
                path: "/a.jpg".into(),
                result: Ok(612),
            },
            Event::Shot {
                path: "/a.jpg".into(),
                result: Err("disk full".into()),
            },
            Event::Rec {
                ms: 12_000,
                bytes: 18_000_000,
            },
            Event::RecDone {
                path: "/v.mp4".into(),
            },
            Event::NoThumb {
                path: "/gone.jpg".into(),
            },
            Event::Thumb {
                path: "/a.jpg".into(),
                slot: 3,
                seq: 7,
                w: 40,
                h: 30,
            },
            Event::Error(Fault::NoCamera),
            Event::Error(Fault::Io("EROFS".into())),
        ];
        for e in all {
            assert_eq!(Event::parse(&e.fields()), Some(e.clone()), "{e:?}");
        }
    }

    #[test]
    fn unknown_words_are_ignored() {
        assert_eq!(Cmd::parse(&["dance".into()]), None);
        assert_eq!(Event::parse(&["state".into(), "dancing".into()]), None);
    }
}
