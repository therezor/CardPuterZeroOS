//! What the app and `gh0st-mediad` say to each other: one message per
//! packet, fields split by tabs. Every ask carries an id the answer names,
//! and a path is always the last field. docs/media.md has the list.

/// The protocol version both sides send in `hello`.
pub const VERSION: &str = "1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Image,
    /// A GIF, APNG or WebP that moves: it plays as a silent loop.
    Anim,
    Video,
    Audio,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Anim => "anim",
            Kind::Video => "video",
            Kind::Audio => "audio",
        }
    }

    fn from_word(s: &str) -> Option<Kind> {
        Some(match s {
            "image" => Kind::Image,
            "anim" => Kind::Anim,
            "video" => Kind::Video,
            "audio" => Kind::Audio,
            _ => return None,
        })
    }
}

/// Why a file gave no picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Unsupported,
    TooBig,
    Corrupt,
    Io,
    NoMem,
}

impl Fail {
    fn word(self) -> &'static str {
        match self {
            Fail::Unsupported => "unsupported",
            Fail::TooBig => "toobig",
            Fail::Corrupt => "corrupt",
            Fail::Io => "io",
            Fail::NoMem => "nomem",
        }
    }

    fn from_word(s: &str) -> Fail {
        match s {
            "unsupported" => Fail::Unsupported,
            "toobig" => Fail::TooBig,
            "io" => Fail::Io,
            "nomem" => Fail::NoMem,
            _ => Fail::Corrupt,
        }
    }

    /// What the screen says.
    pub fn says(self) -> &'static str {
        match self {
            Fail::Unsupported => "This kind of file can't be shown",
            Fail::TooBig => "Too large to show",
            Fail::Corrupt => "The file is damaged",
            Fail::Io => "Can't read the file",
            Fail::NoMem => "Not enough memory",
        }
    }
}

/// What a file is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub kind: Kind,
    /// As shown: after its rotation. A song's are its cover's.
    pub w: u32,
    pub h: u32,
    pub dur_ms: u64,
    pub audio: u32,
    pub subs: u32,
    pub format: String,
    pub vcodec: String,
    pub acodec: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track: String,
    pub date: String,
}

/// How a file opens to play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub looped: bool,
    pub silent: bool,
    pub paused: bool,
    pub audio: Option<u32>,
    pub subs: Option<u32>,
    pub at_ms: u64,
}

impl Flags {
    fn word(&self) -> String {
        let mut w: Vec<String> = Vec::new();
        if self.looped {
            w.push("loop".into());
        }
        if self.silent {
            w.push("silent".into());
        }
        if self.paused {
            w.push("paused".into());
        }
        if let Some(a) = self.audio {
            w.push(format!("audio={a}"));
        }
        if let Some(s) = self.subs {
            w.push(format!("subs={s}"));
        }
        if self.at_ms > 0 {
            w.push(format!("at={}", self.at_ms));
        }
        w.join(",")
    }

    fn from_word(s: &str) -> Flags {
        let mut f = Flags::default();
        for part in s.split(',') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            match k {
                "loop" => f.looped = true,
                "silent" => f.silent = true,
                "paused" => f.paused = true,
                "audio" => f.audio = v.parse().ok(),
                "subs" => f.subs = v.parse().ok(),
                "at" => f.at_ms = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        f
    }
}

/// App to helper.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Hello,
    Quit,
    /// Drops the picture kept for pan and zoom.
    Forget,
    Probe {
        id: u32,
        path: String,
    },
    Tags {
        id: u32,
        path: String,
    },
    /// The middle of the picture, cut to fill w x h.
    Thumb {
        id: u32,
        w: u16,
        h: u16,
        path: String,
    },
    /// Fitted in w x h at `zoom` (1: whole) around (cx, cy).
    View {
        id: u32,
        w: u16,
        h: u16,
        zoom: f32,
        cx: f32,
        cy: f32,
        path: String,
    },
    PlayOpen {
        id: u32,
        w: u16,
        h: u16,
        flags: Flags,
        path: String,
    },
    /// Opens the next track now, to start the moment this one ends.
    PlayNext {
        id: u32,
        path: String,
    },
    Pause,
    Resume,
    Stop,
    Seek(u64),
    Step,
    Speed(u16),
    /// A to B over and over, or off.
    Repeat(Option<(u64, u64)>),
    /// Opens it again where it is, with these tracks; -1 is none or the
    /// best.
    Tracks {
        audio: i32,
        subs: i32,
    },
}

fn num(f: f32) -> String {
    // Short, and enough: thousandths of the picture.
    let s = format!("{f:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl Cmd {
    pub fn fields(&self) -> Vec<String> {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        match self {
            Cmd::Hello => s(&["hello", VERSION]),
            Cmd::Quit => s(&["quit"]),
            Cmd::Forget => s(&["forget"]),
            Cmd::Probe { id, path } => vec!["probe".into(), id.to_string(), path.clone()],
            Cmd::Tags { id, path } => vec!["tags".into(), id.to_string(), path.clone()],
            Cmd::Thumb { id, w, h, path } => vec![
                "thumb".into(),
                id.to_string(),
                w.to_string(),
                h.to_string(),
                path.clone(),
            ],
            Cmd::View {
                id,
                w,
                h,
                zoom,
                cx,
                cy,
                path,
            } => vec![
                "view".into(),
                id.to_string(),
                w.to_string(),
                h.to_string(),
                num(*zoom),
                num(*cx),
                num(*cy),
                path.clone(),
            ],
            Cmd::PlayOpen {
                id,
                w,
                h,
                flags,
                path,
            } => vec![
                "play".into(),
                "open".into(),
                id.to_string(),
                w.to_string(),
                h.to_string(),
                flags.word(),
                path.clone(),
            ],
            Cmd::PlayNext { id, path } => {
                vec!["play".into(), "next".into(), id.to_string(), path.clone()]
            }
            Cmd::Pause => s(&["play", "pause"]),
            Cmd::Resume => s(&["play", "resume"]),
            Cmd::Stop => s(&["play", "stop"]),
            Cmd::Seek(ms) => vec!["play".into(), "seek".into(), ms.to_string()],
            Cmd::Step => s(&["play", "step"]),
            Cmd::Speed(p) => vec!["play".into(), "speed".into(), p.to_string()],
            Cmd::Repeat(None) => s(&["play", "ab", "off"]),
            Cmd::Repeat(Some((a, b))) => {
                vec!["play".into(), "ab".into(), a.to_string(), b.to_string()]
            }
            Cmd::Tracks { audio, subs } => vec![
                "play".into(),
                "tracks".into(),
                audio.to_string(),
                subs.to_string(),
            ],
        }
    }

    /// For the fake helper: a message read back. Unknown words are None.
    pub fn parse(f: &[String]) -> Option<Cmd> {
        let s = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        let n = |i: usize| s(i).parse::<u32>().ok();
        let path = || f.last().cloned().unwrap_or_default();
        Some(match (s(0), s(1)) {
            ("hello", _) => Cmd::Hello,
            ("quit", _) => Cmd::Quit,
            ("forget", _) => Cmd::Forget,
            ("probe", _) if f.len() >= 3 => Cmd::Probe {
                id: n(1)?,
                path: path(),
            },
            ("tags", _) if f.len() >= 3 => Cmd::Tags {
                id: n(1)?,
                path: path(),
            },
            ("thumb", _) if f.len() >= 5 => Cmd::Thumb {
                id: n(1)?,
                w: s(2).parse().ok()?,
                h: s(3).parse().ok()?,
                path: path(),
            },
            ("view", _) if f.len() >= 8 => Cmd::View {
                id: n(1)?,
                w: s(2).parse().ok()?,
                h: s(3).parse().ok()?,
                zoom: s(4).parse().ok()?,
                cx: s(5).parse().ok()?,
                cy: s(6).parse().ok()?,
                path: path(),
            },
            ("play", "open") if f.len() >= 7 => Cmd::PlayOpen {
                id: n(2)?,
                w: s(3).parse().ok()?,
                h: s(4).parse().ok()?,
                flags: Flags::from_word(s(5)),
                path: path(),
            },
            ("play", "next") if f.len() >= 4 => Cmd::PlayNext {
                id: n(2)?,
                path: path(),
            },
            ("play", "pause") => Cmd::Pause,
            ("play", "resume") => Cmd::Resume,
            ("play", "stop") => Cmd::Stop,
            ("play", "seek") => Cmd::Seek(s(2).parse().ok()?),
            ("play", "step") => Cmd::Step,
            ("play", "speed") => Cmd::Speed(s(2).parse().ok()?),
            ("play", "ab") if s(2) == "off" => Cmd::Repeat(None),
            ("play", "ab") => Cmd::Repeat(Some((s(2).parse().ok()?, s(3).parse().ok()?))),
            ("play", "tracks") => Cmd::Tracks {
                audio: s(2).parse().ok()?,
                subs: s(3).parse().ok()?,
            },
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    /// At the end, paused there. Resume plays from the start.
    Done,
}

impl PlayState {
    fn word(self) -> &'static str {
        match self {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Done => "done",
        }
    }
}

/// Helper to app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Hello(String),
    Info {
        id: u32,
        info: Info,
    },
    Tags {
        id: u32,
        tags: Tags,
    },
    Thumb {
        id: u32,
        slot: u32,
        seq: u32,
        w: u16,
        h: u16,
    },
    View {
        id: u32,
        slot: u32,
        seq: u32,
        w: u16,
        h: u16,
    },
    Fail {
        id: u32,
        why: Fail,
    },
    /// A picture of what plays.
    Frame {
        slot: u32,
        seq: u32,
        w: u16,
        h: u16,
    },
    Play {
        id: u32,
        pos_ms: u64,
        dur_ms: u64,
        state: PlayState,
        dropped: u32,
        slow: u8,
    },
    /// Decoding skips more to keep up: step 1 to 4.
    Slow {
        id: u32,
        step: u8,
    },
    PlayErr {
        id: u32,
        why: String,
    },
}

impl Event {
    pub fn fields(&self) -> Vec<String> {
        let t = |v: Vec<String>| v;
        match self {
            Event::Hello(v) => t(vec!["hello".into(), v.clone()]),
            Event::Info { id, info } => t(vec![
                "info".into(),
                id.to_string(),
                info.kind.word().into(),
                info.w.to_string(),
                info.h.to_string(),
                info.dur_ms.to_string(),
                info.audio.to_string(),
                info.subs.to_string(),
                info.format.clone(),
                info.vcodec.clone(),
                info.acodec.clone(),
            ]),
            Event::Tags { id, tags } => t(vec![
                "tags".into(),
                id.to_string(),
                tags.title.clone(),
                tags.artist.clone(),
                tags.album.clone(),
                tags.track.clone(),
                tags.date.clone(),
            ]),
            Event::Thumb {
                id,
                slot,
                seq,
                w,
                h,
            }
            | Event::View {
                id,
                slot,
                seq,
                w,
                h,
            } => {
                let verb = if matches!(self, Event::Thumb { .. }) {
                    "thumb"
                } else {
                    "view"
                };
                t(vec![
                    verb.into(),
                    id.to_string(),
                    slot.to_string(),
                    seq.to_string(),
                    w.to_string(),
                    h.to_string(),
                ])
            }
            Event::Fail { id, why } => t(vec!["fail".into(), id.to_string(), why.word().into()]),
            Event::Frame { slot, seq, w, h } => t(vec![
                "frame".into(),
                slot.to_string(),
                seq.to_string(),
                w.to_string(),
                h.to_string(),
            ]),
            Event::Play {
                id,
                pos_ms,
                dur_ms,
                state,
                dropped,
                slow,
            } => t(vec![
                "play".into(),
                id.to_string(),
                pos_ms.to_string(),
                dur_ms.to_string(),
                state.word().into(),
                dropped.to_string(),
                slow.to_string(),
            ]),
            Event::Slow { id, step } => t(vec!["slow".into(), id.to_string(), step.to_string()]),
            Event::PlayErr { id, why } => t(vec!["playerr".into(), id.to_string(), why.clone()]),
        }
    }

    pub fn parse(f: &[String]) -> Option<Event> {
        let s = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        let n = |i: usize| s(i).parse::<u32>().ok();
        let id = n(1);
        Some(match s(0) {
            "hello" => Event::Hello(s(1).into()),
            "info" => Event::Info {
                id: id?,
                info: Info {
                    kind: Kind::from_word(s(2))?,
                    w: n(3)?,
                    h: n(4)?,
                    dur_ms: s(5).parse().ok()?,
                    audio: n(6)?,
                    subs: n(7)?,
                    format: s(8).into(),
                    vcodec: s(9).into(),
                    acodec: s(10).into(),
                },
            },
            "tags" => Event::Tags {
                id: id?,
                tags: Tags {
                    title: s(2).into(),
                    artist: s(3).into(),
                    album: s(4).into(),
                    track: s(5).into(),
                    date: s(6).into(),
                },
            },
            verb @ ("thumb" | "view") => {
                let (id, slot, seq) = (id?, n(2)?, n(3)?);
                let (w, h) = (s(4).parse().ok()?, s(5).parse().ok()?);
                if verb == "thumb" {
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
                }
            }
            "fail" => Event::Fail {
                id: id?,
                why: Fail::from_word(s(2)),
            },
            "frame" => Event::Frame {
                slot: n(1)?,
                seq: n(2)?,
                w: s(3).parse().ok()?,
                h: s(4).parse().ok()?,
            },
            "play" => Event::Play {
                id: id?,
                pos_ms: s(2).parse().ok()?,
                dur_ms: s(3).parse().ok()?,
                state: match s(4) {
                    "paused" => PlayState::Paused,
                    "done" => PlayState::Done,
                    _ => PlayState::Playing,
                },
                dropped: n(5).unwrap_or(0),
                slow: s(6).parse().unwrap_or(0),
            },
            "slow" => Event::Slow {
                id: id?,
                step: s(2).parse().ok()?,
            },
            "playerr" => Event::PlayErr {
                id: id?,
                why: s(2).into(),
            },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(f: Vec<String>) -> Vec<String> {
        f
    }

    #[test]
    fn every_ask_reads_back_as_itself() {
        let all = [
            Cmd::Hello,
            Cmd::Quit,
            Cmd::Forget,
            Cmd::Probe {
                id: 3,
                path: "/a/b c.jpg".into(),
            },
            Cmd::Tags {
                id: 4,
                path: "/m.flac".into(),
            },
            Cmd::Thumb {
                id: 5,
                w: 60,
                h: 45,
                path: "/p.png".into(),
            },
            Cmd::View {
                id: 6,
                w: 320,
                h: 170,
                zoom: 2.5,
                cx: 0.25,
                cy: 0.75,
                path: "/p.heic".into(),
            },
            Cmd::PlayOpen {
                id: 7,
                w: 320,
                h: 170,
                flags: Flags {
                    looped: true,
                    silent: false,
                    paused: true,
                    audio: Some(1),
                    subs: Some(0),
                    at_ms: 61_000,
                },
                path: "/v.mkv".into(),
            },
            Cmd::PlayNext {
                id: 8,
                path: "/n.mp3".into(),
            },
            Cmd::Pause,
            Cmd::Resume,
            Cmd::Stop,
            Cmd::Seek(5000),
            Cmd::Step,
            Cmd::Speed(150),
            Cmd::Repeat(Some((1000, 4000))),
            Cmd::Repeat(None),
            Cmd::Tracks { audio: 1, subs: -1 },
        ];
        for c in all {
            assert_eq!(Cmd::parse(&strs(c.fields())), Some(c.clone()), "{c:?}");
        }
    }

    #[test]
    fn every_answer_reads_back_as_itself() {
        let all = [
            Event::Hello("1".into()),
            Event::Info {
                id: 1,
                info: Info {
                    kind: Kind::Video,
                    w: 1920,
                    h: 1080,
                    dur_ms: 61_000,
                    audio: 2,
                    subs: 1,
                    format: "matroska,webm".into(),
                    vcodec: "h264".into(),
                    acodec: "aac".into(),
                },
            },
            Event::Tags {
                id: 2,
                tags: Tags {
                    title: "Song".into(),
                    artist: "Band".into(),
                    album: "Album".into(),
                    track: "3".into(),
                    date: "2024".into(),
                },
            },
            Event::Thumb {
                id: 3,
                slot: 3,
                seq: 8,
                w: 60,
                h: 45,
            },
            Event::View {
                id: 4,
                slot: 3,
                seq: 10,
                w: 320,
                h: 170,
            },
            Event::Fail {
                id: 5,
                why: Fail::TooBig,
            },
            Event::Frame {
                slot: 1,
                seq: 12,
                w: 302,
                h: 170,
            },
            Event::Play {
                id: 6,
                pos_ms: 1000,
                dur_ms: 2000,
                state: PlayState::Paused,
                dropped: 3,
                slow: 1,
            },
            Event::Slow { id: 6, step: 2 },
            Event::PlayErr {
                id: 6,
                why: "can't decode this video".into(),
            },
        ];
        for e in all {
            assert_eq!(Event::parse(&strs(e.fields())), Some(e.clone()), "{e:?}");
        }
    }

    #[test]
    fn the_helpers_own_words_parse() {
        let f = |s: &str| s.split('\t').map(String::from).collect::<Vec<_>>();
        assert_eq!(
            Event::parse(&f("info\t1\taudio\t300\t300\t3030\t1\t0\tmp3\t\tmp3")),
            Some(Event::Info {
                id: 1,
                info: Info {
                    kind: Kind::Audio,
                    w: 300,
                    h: 300,
                    dur_ms: 3030,
                    audio: 1,
                    subs: 0,
                    format: "mp3".into(),
                    vcodec: String::new(),
                    acodec: "mp3".into(),
                },
            })
        );
        assert_eq!(
            Event::parse(&f("play\t9\t1000\t10000\tplaying\t0\t0")),
            Some(Event::Play {
                id: 9,
                pos_ms: 1000,
                dur_ms: 10_000,
                state: PlayState::Playing,
                dropped: 0,
                slow: 0,
            })
        );
    }
}
