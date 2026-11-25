//! MPRIS: music that plays on behind other apps answers `playerctl`, a
//! headset's buttons (through BlueZ's mpris-proxy) and any player widget.
//! The name is owned only while music plays. No thread and no timer: the
//! bus fd joins the page's loop, and calls are answered as they come.

use gh0st_app::dbus::{Conn, Kind, Message, Value, bus};
use std::path::Path;

pub const NAME: &str = "org.mpris.MediaPlayer2.gh0st_media";
pub const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// What a remote asks of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Forward or back, in ms.
    Seek(i64),
    /// To here in the track, in ms.
    SetPosition(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Playing => "Playing",
            Status::Paused => "Paused",
            Status::Stopped => "Stopped",
        }
    }
}

/// What the player says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub status: Status,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length_ms: u64,
    pub pos_ms: u64,
    /// Which track: a number that changes with each one.
    pub track: u32,
    pub can_next: bool,
    pub can_prev: bool,
    pub shuffle: bool,
    pub repeat: bool,
}

impl State {
    fn track_id(&self) -> String {
        if self.status == Status::Stopped {
            NO_TRACK.into()
        } else {
            format!("/org/gh0st/media/track/{}", self.track)
        }
    }

    fn metadata(&self) -> Value {
        let mut m = vec![("mpris:trackid", Value::path(self.track_id()))];
        if self.length_ms > 0 {
            m.push(("mpris:length", Value::I64(self.length_ms as i64 * 1000)));
        }
        if !self.title.is_empty() {
            m.push(("xesam:title", Value::str(self.title.clone())));
        }
        if !self.artist.is_empty() {
            m.push(("xesam:artist", Value::strs([self.artist.clone()])));
        }
        if !self.album.is_empty() {
            m.push(("xesam:album", Value::str(self.album.clone())));
        }
        Value::props(m)
    }

    fn player_props(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("PlaybackStatus", Value::str(self.status.word())),
            (
                "LoopStatus",
                Value::str(if self.repeat { "Playlist" } else { "None" }),
            ),
            ("Rate", Value::F64(1.0)),
            ("MinimumRate", Value::F64(1.0)),
            ("MaximumRate", Value::F64(1.0)),
            ("Shuffle", Value::Bool(self.shuffle)),
            ("Metadata", self.metadata()),
            ("Volume", Value::F64(1.0)),
            ("Position", Value::I64(self.pos_ms as i64 * 1000)),
            ("CanGoNext", Value::Bool(self.can_next)),
            ("CanGoPrevious", Value::Bool(self.can_prev)),
            ("CanPlay", Value::Bool(true)),
            ("CanPause", Value::Bool(true)),
            ("CanSeek", Value::Bool(self.length_ms > 0)),
            ("CanControl", Value::Bool(true)),
        ]
    }
}

fn root_props() -> Vec<(&'static str, Value)> {
    vec![
        ("CanQuit", Value::Bool(false)),
        ("CanRaise", Value::Bool(false)),
        ("HasTrackList", Value::Bool(false)),
        ("Identity", Value::str("Media")),
        ("DesktopEntry", Value::str("media")),
        ("SupportedUriSchemes", Value::strs(["file"])),
        (
            "SupportedMimeTypes",
            Value::strs(["audio/mpeg", "audio/flac", "audio/ogg"]),
        ),
    ]
}

const XML: &str = r#"<interface name="org.mpris.MediaPlayer2"><method name="Raise"/><method name="Quit"/><property name="CanQuit" type="b" access="read"/><property name="CanRaise" type="b" access="read"/><property name="HasTrackList" type="b" access="read"/><property name="Identity" type="s" access="read"/><property name="DesktopEntry" type="s" access="read"/><property name="SupportedUriSchemes" type="as" access="read"/><property name="SupportedMimeTypes" type="as" access="read"/></interface><interface name="org.mpris.MediaPlayer2.Player"><method name="Next"/><method name="Previous"/><method name="Pause"/><method name="PlayPause"/><method name="Stop"/><method name="Play"/><method name="Seek"><arg type="x" direction="in"/></method><method name="SetPosition"><arg type="o" direction="in"/><arg type="x" direction="in"/></method><method name="OpenUri"><arg type="s" direction="in"/></method><signal name="Seeked"><arg type="x"/></signal><property name="PlaybackStatus" type="s" access="read"/><property name="LoopStatus" type="s" access="read"/><property name="Rate" type="d" access="read"/><property name="Shuffle" type="b" access="read"/><property name="Metadata" type="a{sv}" access="read"/><property name="Volume" type="d" access="read"/><property name="Position" type="x" access="read"/><property name="MinimumRate" type="d" access="read"/><property name="MaximumRate" type="d" access="read"/><property name="CanGoNext" type="b" access="read"/><property name="CanGoPrevious" type="b" access="read"/><property name="CanPlay" type="b" access="read"/><property name="CanPause" type="b" access="read"/><property name="CanSeek" type="b" access="read"/><property name="CanControl" type="b" access="read"/></interface>"#;

/// Answers one call against `state`: the reply, and what the player should
/// do. No I/O, so it tests without a bus.
pub fn call(m: &Message, state: &State) -> (Message, Option<Control>) {
    if let Some(r) = bus::peer(m) {
        return (r, None);
    }
    let path = m.path.as_deref().unwrap_or("");
    if path != PATH {
        let r = Message::error(m, bus::err::UNKNOWN_OBJECT, "no such object");
        return (r, None);
    }
    if m.is(bus::INTROSPECTABLE, "Introspect") {
        let xml = bus::introspect(&[XML, bus::PROPERTIES_XML], &[]);
        return (Message::reply(m).arg(Value::str(xml)), None);
    }
    if m.interface.as_deref() == Some(bus::PROPERTIES) {
        return (properties(m, state), None);
    }
    let control = match (m.interface.as_deref(), m.member.as_deref()) {
        (Some(PLAYER), Some("Play")) => Some(Control::Play),
        (Some(PLAYER), Some("Pause")) => Some(Control::Pause),
        (Some(PLAYER), Some("PlayPause")) => Some(Control::PlayPause),
        (Some(PLAYER), Some("Stop")) => Some(Control::Stop),
        (Some(PLAYER), Some("Next")) => Some(Control::Next),
        (Some(PLAYER), Some("Previous")) => Some(Control::Previous),
        (Some(PLAYER), Some("Seek")) => match m.body.first() {
            Some(Value::I64(us)) => Some(Control::Seek(us / 1000)),
            _ => return (bad(m), None),
        },
        (Some(PLAYER), Some("SetPosition")) => {
            let track = m.body.first().and_then(Value::as_str);
            match (track, m.body.get(1)) {
                // A position for another track is ignored, as the spec says.
                (Some(t), Some(Value::I64(us))) if t == state.track_id() && *us >= 0 => {
                    Some(Control::SetPosition(*us as u64 / 1000))
                }
                (Some(_), Some(Value::I64(_))) => None,
                _ => return (bad(m), None),
            }
        }
        (Some(PLAYER), Some("OpenUri")) => {
            let r = Message::error(m, bus::err::NOT_SUPPORTED, "open the file in Files");
            return (r, None);
        }
        // Neither raises nor quits: the shell owns the screen.
        (Some(ROOT), Some("Raise" | "Quit")) => None,
        _ => {
            let r = Message::error(m, bus::err::UNKNOWN_METHOD, "no such method");
            return (r, None);
        }
    };
    (Message::reply(m), control)
}

fn bad(m: &Message) -> Message {
    Message::error(m, bus::err::INVALID_ARGS, "bad arguments")
}

fn properties(m: &Message, state: &State) -> Message {
    let iface = m.body.first().and_then(Value::as_str).unwrap_or("");
    let all = match iface {
        ROOT => root_props(),
        PLAYER => state.player_props(),
        _ => return Message::error(m, bus::err::UNKNOWN_INTERFACE, "no such interface"),
    };
    match m.member.as_deref() {
        Some("GetAll") => Message::reply(m).arg(Value::props(all)),
        Some("Get") => {
            let name = m.body.get(1).and_then(Value::as_str).unwrap_or("");
            match all.into_iter().find(|(k, _)| *k == name) {
                Some((_, v)) => Message::reply(m).arg(Value::variant(v)),
                None => Message::error(m, bus::err::UNKNOWN_PROPERTY, "no such property"),
            }
        }
        Some("Set") => Message::error(m, bus::err::READ_ONLY, "read only"),
        _ => Message::error(m, bus::err::UNKNOWN_METHOD, "no such method"),
    }
}

/// What changed between two states, for PropertiesChanged. Position is
/// left out, as the spec asks: clients read it.
pub fn changes(old: &State, new: &State) -> Vec<(&'static str, Value)> {
    let a = old.player_props();
    new.player_props()
        .into_iter()
        .zip(a)
        .filter(|((k, v), (_, w))| *k != "Position" && v != w)
        .map(|(kv, _)| kv)
        .collect()
}

/// The player on the session bus.
pub struct Mpris {
    conn: Conn,
    /// What clients were last told.
    told: State,
    /// Reading or writing the bus failed.
    broken: bool,
}

impl Mpris {
    /// Connects and takes the name. None without a bus or when another
    /// player has it.
    pub fn start(state: &State) -> Option<Mpris> {
        let mut conn = Conn::session().ok()?;
        if !bus::own(&mut conn, NAME).ok()? {
            return None;
        }
        Some(Mpris {
            conn,
            told: state.clone(),
            broken: false,
        })
    }

    pub fn fd(&self) -> std::os::fd::RawFd {
        self.conn.fd()
    }

    /// Reads and answers what came. Returns what the player should do.
    pub fn ready(&mut self, state: &State) -> Vec<Control> {
        let mut out = Vec::new();
        if self.conn.flush().and_then(|()| self.conn.fill()).is_err() {
            self.broken = true;
            return out;
        }
        while let Ok(Some(m)) = self.conn.recv() {
            if m.kind != Kind::Call {
                continue;
            }
            let (reply, control) = call(&m, state);
            if !m.no_reply() {
                let _ = self.conn.send(reply);
            }
            out.extend(control);
        }
        let _ = self.conn.flush();
        out
    }

    /// The bus hung up or failed: drop this player.
    pub fn closed(&self) -> bool {
        self.broken || self.conn.closed()
    }

    /// Tells clients what changed since they were last told.
    pub fn update(&mut self, state: &State) {
        let ch = changes(&self.told, state);
        let seeked = state.track == self.told.track
            && state.pos_ms.abs_diff(self.told.pos_ms) > 1500
            && state.status != Status::Stopped;
        if !ch.is_empty() {
            let _ = self.conn.send(bus::changed(PATH, PLAYER, ch));
        }
        if seeked {
            let s =
                Message::signal(PATH, PLAYER, "Seeked").arg(Value::I64(state.pos_ms as i64 * 1000));
            let _ = self.conn.send(s);
        }
        let _ = self.conn.flush();
        self.told = state.clone();
    }
}

impl Drop for Mpris {
    fn drop(&mut self) {
        // A bus that went has let the name go, and a call would wait.
        if self.closed() {
            return;
        }
        let m = Message::call(bus::NAME, "/org/freedesktop/DBus", bus::NAME, "ReleaseName")
            .arg(Value::str(NAME));
        let _ = self.conn.call(m, 500);
    }
}

/// A track's title for the remote: its tag, else its file name.
pub fn title_of(tag: &str, path: &Path) -> String {
    if tag.is_empty() {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        tag.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_of(iface: &str, member: &str, body: Vec<Value>) -> Message {
        let mut m = Message::call(NAME, PATH, iface, member);
        m.sender = Some(":1.5".into());
        m.serial = 3;
        m.body = body;
        m
    }

    fn playing() -> State {
        State {
            status: Status::Playing,
            title: "Test Tone".into(),
            artist: "gh0st".into(),
            length_ms: 20_000,
            pos_ms: 3_000,
            track: 4,
            can_next: true,
            ..State::default()
        }
    }

    #[test]
    fn the_buttons_become_controls() {
        let s = playing();
        for (member, want) in [
            ("Play", Control::Play),
            ("Pause", Control::Pause),
            ("PlayPause", Control::PlayPause),
            ("Stop", Control::Stop),
            ("Next", Control::Next),
            ("Previous", Control::Previous),
        ] {
            let (r, c) = call(&call_of(PLAYER, member, vec![]), &s);
            assert!(r.error_name.is_none(), "{member}");
            assert_eq!(c, Some(want));
        }
        let (_, c) = call(&call_of(PLAYER, "Seek", vec![Value::I64(-5_000_000)]), &s);
        assert_eq!(c, Some(Control::Seek(-5000)));
        let here = Value::path("/org/gh0st/media/track/4");
        let (_, c) = call(
            &call_of(PLAYER, "SetPosition", vec![here, Value::I64(9_000_000)]),
            &s,
        );
        assert_eq!(c, Some(Control::SetPosition(9000)));
        let other = Value::path("/org/gh0st/media/track/9");
        let (r, c) = call(
            &call_of(PLAYER, "SetPosition", vec![other, Value::I64(1)]),
            &s,
        );
        assert!(
            r.error_name.is_none() && c.is_none(),
            "another track's position is ignored"
        );
    }

    #[test]
    fn properties_say_what_plays() {
        let s = playing();
        let get = call_of(
            bus::PROPERTIES,
            "Get",
            vec![Value::str(PLAYER), Value::str("PlaybackStatus")],
        );
        assert_eq!(call(&get, &s).0.body[0].inner().as_str(), Some("Playing"));
        let all = call_of(bus::PROPERTIES, "GetAll", vec![Value::str(PLAYER)]);
        let r = call(&all, &s).0;
        let md = r.body[0]
            .get("Metadata")
            .map(Value::inner)
            .cloned()
            .unwrap();
        assert_eq!(
            md.get("xesam:title")
                .map(Value::inner)
                .and_then(Value::as_str),
            Some("Test Tone")
        );
        assert_eq!(
            md.get("mpris:length").map(Value::inner),
            Some(&Value::I64(20_000_000))
        );
        let set = call_of(
            bus::PROPERTIES,
            "Set",
            vec![
                Value::str(PLAYER),
                Value::str("Volume"),
                Value::variant(Value::F64(0.5)),
            ],
        );
        assert!(call(&set, &s).0.error_name.is_some());
    }

    #[test]
    fn only_what_changed_is_told_and_never_the_position() {
        let a = playing();
        let mut b = a.clone();
        b.pos_ms = 4_000;
        assert!(changes(&a, &b).is_empty(), "position moves on its own");
        b.status = Status::Paused;
        let ch = changes(&a, &b);
        assert_eq!(ch.len(), 1);
        assert_eq!(ch[0].0, "PlaybackStatus");
        b.track = 5;
        assert!(changes(&a, &b).iter().any(|(k, _)| *k == "Metadata"));
    }
}
