//! The file portal from an app's side (`docs/portal.md`): asking for a
//! file with [`Cx::pick`](crate::Cx::pick), and Files answering as the
//! chooser.

use gh0st_dbus::{Conn, Kind, Message, Ty, Value};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const OURS: &str = "org.gh0st.Portal";
const OURS_PATH: &str = "/org/gh0st/Portal";
const CALL_MS: u64 = 3000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickMode {
    /// One file, or several with `multiple`.
    #[default]
    Open,
    /// A name to save to.
    Save,
    /// A folder.
    Folder,
}

/// What [`Cx::pick`](crate::Cx::pick) asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickSpec {
    pub mode: PickMode,
    pub multiple: bool,
    /// Named filters, each a list of globs (`*.txt`) or MIME types
    /// (`image/*`, anything with a `/`).
    pub filters: Vec<(String, Vec<String>)>,
    /// The name a save starts with.
    pub name: String,
    /// Where to start.
    pub folder: Option<PathBuf>,
    /// The main action's word, when Open, Save or Choose won't do.
    pub accept: Option<String>,
}

impl PickSpec {
    pub fn open() -> PickSpec {
        PickSpec::default()
    }

    pub fn save(name: &str) -> PickSpec {
        PickSpec {
            mode: PickMode::Save,
            name: name.into(),
            ..PickSpec::default()
        }
    }

    pub fn folder() -> PickSpec {
        PickSpec {
            mode: PickMode::Folder,
            ..PickSpec::default()
        }
    }

    pub fn filter(mut self, name: &str, patterns: &[&str]) -> PickSpec {
        self.filters.push((
            name.into(),
            patterns.iter().map(|p| p.to_string()).collect(),
        ));
        self
    }

    pub fn multiple(mut self) -> PickSpec {
        self.multiple = true;
        self
    }

    pub fn starting_in(mut self, folder: &Path) -> PickSpec {
        self.folder = Some(folder.to_path_buf());
        self
    }

    fn options(&self) -> Value {
        let mut o: Vec<(&str, Value)> = Vec::new();
        if self.multiple {
            o.push(("multiple", Value::Bool(true)));
        }
        if self.mode == PickMode::Folder {
            o.push(("directory", Value::Bool(true)));
        }
        if !self.filters.is_empty() {
            o.push((
                "filters",
                Value::Array(filter_ty(), self.filters.iter().map(filter_value).collect()),
            ));
        }
        if !self.name.is_empty() {
            o.push(("current_name", Value::str(self.name.clone())));
        }
        if let Some(f) = &self.folder {
            let mut b = f.as_os_str().as_bytes().to_vec();
            b.push(0);
            o.push(("current_folder", Value::Bytes(b)));
        }
        if let Some(a) = &self.accept {
            o.push(("accept_label", Value::str(a.clone())));
        }
        Value::props(o)
    }
}

fn filter_ty() -> Ty {
    Ty::Struct(vec![
        Ty::Str,
        Ty::Array(Box::new(Ty::Struct(vec![Ty::U32, Ty::Str]))),
    ])
}

fn filter_value(f: &(String, Vec<String>)) -> Value {
    let pats =
        f.1.iter()
            .map(|p| {
                let kind = if p.contains('/') { 1 } else { 0 };
                Value::Struct(vec![Value::U32(kind), Value::str(p.clone())])
            })
            .collect();
    Value::Struct(vec![
        Value::str(f.0.clone()),
        Value::Array(Ty::Struct(vec![Ty::U32, Ty::Str]), pats),
    ])
}

/// A pick the portal has taken and not answered yet.
struct Waiting {
    token: u64,
    owner: usize,
    /// The request's object path, which its Response comes from.
    path: String,
    /// The portal's unique name when it took the pick: if that leaves the
    /// bus, no answer will come.
    by: Option<String>,
}

/// The portal, from an app: one connection, the picks waiting on it.
pub(crate) struct Client {
    conn: Conn,
    waiting: Vec<Waiting>,
    /// The connection failed: a new one is made for the next pick.
    dead: bool,
}

impl Client {
    /// Connects, and listens once for every answer this connection's
    /// picks will get and for the portal leaving the bus.
    pub(crate) fn connect() -> Option<Client> {
        let mut conn = Conn::session().ok()?;
        let me = conn.name.clone().unwrap_or_default();
        let answers = format!(
            "type='signal',interface='{REQUEST}',member='Response',path_namespace='{}'",
            requests(&me)
        );
        let gone = format!(
            "type='signal',sender='{}',member='NameOwnerChanged',arg0='{NAME}'",
            gh0st_dbus::bus::NAME
        );
        for rule in [answers, gone] {
            gh0st_dbus::bus::add_match(&mut conn, &rule).ok()?;
        }
        Some(Client {
            conn,
            waiting: Vec::new(),
            dead: false,
        })
    }

    pub(crate) fn fd(&self) -> std::os::fd::RawFd {
        self.conn.fd()
    }

    pub(crate) fn wants_write(&self) -> bool {
        self.conn.wants_write()
    }

    pub(crate) fn idle(&self) -> bool {
        self.waiting.is_empty()
    }

    pub(crate) fn dead(&self) -> bool {
        self.dead || self.conn.closed()
    }

    /// Asks for a file. The answer comes from [`ready`](Client::ready), or
    /// from [`drain`](Client::drain) right after: it may have come with the
    /// reply.
    pub(crate) fn pick(&mut self, token: u64, owner: usize, spec: &PickSpec) -> Result<(), String> {
        let member = match spec.mode {
            PickMode::Save => "SaveFile",
            PickMode::Open | PickMode::Folder => "OpenFile",
        };
        let handle = format!("gh0st{token}");
        let mut o = spec.options();
        if let Value::Array(_, items) = &mut o {
            items.push(Value::Entry(
                Box::new(Value::str("handle_token")),
                Box::new(Value::variant(Value::str(handle.clone()))),
            ));
        }
        let me = self.conn.name.clone().unwrap_or_default();
        let path = format!("{}/{handle}", requests(&me));
        let m = Message::call(NAME, PATH, CHOOSER, member)
            .arg(Value::str(""))
            .arg(Value::str(""))
            .arg(o);
        let r = self.conn.call(m, CALL_MS).map_err(|e| {
            if !matches!(e, gh0st_dbus::Error::Remote { .. }) {
                self.dead = true;
            }
            e.to_string()
        })?;
        let got = r
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or(&path)
            .to_string();
        self.waiting.push(Waiting {
            token,
            owner,
            path: got,
            by: r.sender.clone(),
        });
        Ok(())
    }

    /// Ends the picks of page `from` and every page above it: they closed.
    /// The chooser is told so it goes; no reply is waited for.
    pub(crate) fn forget(&mut self, from: usize) {
        for w in split_off(&mut self.waiting, |w| w.owner >= from) {
            let m = Message::call(NAME, &w.path, REQUEST, "Close").flags(gh0st_dbus::message::NO_REPLY);
            if self.conn.send(m).is_err() {
                self.dead = true;
            }
        }
    }

    /// The fd is ready: answers that arrived, as [`drain`](Client::drain).
    pub(crate) fn ready(&mut self) -> Vec<(u64, usize, Option<Vec<PathBuf>>)> {
        if self.conn.flush().and_then(|()| self.conn.fill()).is_err() || self.conn.closed() {
            // The bus went away: every pick waiting ends unanswered.
            self.dead = true;
            return self
                .waiting
                .drain(..)
                .map(|w| (w.token, w.owner, None))
                .collect();
        }
        self.drain()
    }

    /// Answers already read: token, owner, the paths or None. A pick whose
    /// portal left the bus ends with None.
    pub(crate) fn drain(&mut self) -> Vec<(u64, usize, Option<Vec<PathBuf>>)> {
        let mut out = Vec::new();
        while let Ok(Some(m)) = self.conn.recv() {
            if m.kind != Kind::Signal {
                continue;
            }
            if m.is(gh0st_dbus::bus::NAME, "NameOwnerChanged") {
                let arg = |i: usize| m.body.get(i).and_then(Value::as_str);
                let bus = m.sender.as_deref() == Some(gh0st_dbus::bus::NAME);
                if let (true, Some(NAME), Some(old)) = (bus, arg(0), arg(1)) {
                    let gone = split_off(&mut self.waiting, |w| w.by.as_deref() == Some(old));
                    out.extend(gone.into_iter().map(|w| (w.token, w.owner, None)));
                }
                continue;
            }
            if !m.is(REQUEST, "Response") {
                continue;
            }
            let path = m.path.as_deref().unwrap_or("");
            let Some(at) = self.waiting.iter().position(|w| w.path == path) else {
                continue;
            };
            let w = self.waiting.remove(at);
            let ok = m.body.first().and_then(Value::as_u32) == Some(0);
            let paths = ok.then(|| {
                m.body
                    .get(1)
                    .and_then(|r| r.get("uris"))
                    .map(Value::inner)
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(gh0st_core::uri::to_path)
                    .collect()
            });
            out.push((w.token, w.owner, paths));
        }
        out
    }
}

/// Takes the picks `f` says out of `waiting`.
fn split_off(waiting: &mut Vec<Waiting>, f: impl Fn(&Waiting) -> bool) -> Vec<Waiting> {
    let (gone, kept) = std::mem::take(waiting).into_iter().partition(f);
    *waiting = kept;
    gone
}

/// Where the portal puts this connection's requests (`unique` is its
/// name on the bus).
fn requests(unique: &str) -> String {
    format!(
        "{PATH}/request/{}",
        unique.trim_start_matches(':').replace('.', "_")
    )
}

/// What the shell's chooser is asked, from `gh0st session`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Question {
    pub mode: PickMode,
    pub multiple: bool,
    /// The app asking.
    pub app: String,
    pub accept: String,
    /// Named filters: each pattern a glob (kind 0) or a MIME type (1).
    pub filters: Vec<(String, Vec<(u32, String)>)>,
    pub filter: Option<usize>,
    pub name: String,
    pub folder: Option<PathBuf>,
}

/// The chooser's side: the question for request `id`, which only the
/// process the shell started may take.
pub fn take(id: u64) -> Result<(Conn, Question), String> {
    let mut conn = Conn::session().map_err(|e| e.to_string())?;
    let m = Message::call(NAME, OURS_PATH, OURS, "Take").arg(Value::U64(id));
    let r = conn.call(m, CALL_MS).map_err(|e| e.to_string())?;
    let q = r.body.first().ok_or("no question")?;
    let s = |k: &str| {
        q.get(k)
            .map(Value::inner)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let filters = q
        .get("filters")
        .map(Value::inner)
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| {
            let f = f.as_struct()?;
            let pats = f
                .get(1)?
                .as_array()?
                .iter()
                .filter_map(|p| {
                    let p = p.as_struct()?;
                    Some((p.first()?.as_u32()?, p.get(1)?.as_str()?.to_string()))
                })
                .collect();
            Some((f.first()?.as_str()?.to_string(), pats))
        })
        .collect();
    let question = Question {
        mode: match s("mode").as_str() {
            "save" => PickMode::Save,
            "folder" => PickMode::Folder,
            _ => PickMode::Open,
        },
        multiple: q
            .get("multiple")
            .map(Value::inner)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        app: s("app"),
        accept: s("accept"),
        filters,
        filter: q
            .get("filter")
            .map(Value::inner)
            .and_then(Value::as_i32)
            .and_then(|f| usize::try_from(f).ok()),
        name: s("name"),
        folder: q
            .get("folder")
            .map(Value::inner)
            .and_then(Value::as_bytes)
            .map(|b| PathBuf::from(std::ffi::OsStr::from_bytes(b))),
    };
    Ok((conn, question))
}

/// The chooser's answer: the paths picked, or None for none.
pub fn done(
    conn: &mut Conn,
    id: u64,
    paths: Option<&[PathBuf]>,
    filter: Option<usize>,
) -> Result<(), String> {
    let code = if paths.is_some() { 0 } else { 1 };
    let list = paths
        .unwrap_or(&[])
        .iter()
        .map(|p| Value::Bytes(p.as_os_str().as_bytes().to_vec()))
        .collect();
    let m = Message::call(NAME, OURS_PATH, OURS, "Done")
        .arg(Value::U64(id))
        .arg(Value::U32(code))
        .arg(Value::Array(Ty::Array(Box::new(Ty::Byte)), list))
        .arg(Value::I32(filter.map(|f| f as i32).unwrap_or(-1)));
    conn.call(m, CALL_MS).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = ":1.42";
    const PORTAL: &str = ":1.7";

    fn client() -> (Client, Conn) {
        let (a, b) = gh0st_dbus::conn::pair().unwrap();
        let mut conn = Conn::raw(a).unwrap();
        conn.name = Some(ME.into());
        let client = Client {
            conn,
            waiting: Vec::new(),
            dead: false,
        };
        (client, Conn::raw(b).unwrap())
    }

    /// The portal's reply to the client's call number `serial`.
    fn reply(serial: u32, path: &str) -> Message {
        let mut call = Message::call(NAME, PATH, CHOOSER, "OpenFile");
        call.serial = serial;
        let mut r = Message::reply(&call).arg(Value::str(path));
        r.sender = Some(PORTAL.into());
        r
    }

    fn response(path: &str, uri: &str) -> Message {
        let uris = Value::Array(Ty::Str, vec![Value::str(uri)]);
        Message::signal(path, REQUEST, "Response")
            .arg(Value::U32(0))
            .arg(Value::props([("uris", Value::variant(uris))]))
    }

    #[test]
    fn an_answer_read_with_the_reply_is_handed_out_at_once() {
        let (mut c, mut portal) = client();
        let path = format!("{}/gh0st7", requests(ME));
        // Both are in the socket before the call reads anything.
        portal.send(response(&path, "file:///tmp/a.txt")).unwrap();
        portal.send(reply(1, &path)).unwrap();
        c.pick(7, 1, &PickSpec::open()).unwrap();
        assert_eq!(
            c.drain(),
            [(7, 1, Some(vec![PathBuf::from("/tmp/a.txt")]))]
        );
        assert!(c.idle());
    }

    #[test]
    fn a_closed_page_loses_its_pick_and_the_chooser_is_told() {
        let (mut c, mut portal) = client();
        let path = format!("{}/gh0st3", requests(ME));
        portal.send(reply(1, &path)).unwrap();
        c.pick(3, 2, &PickSpec::open()).unwrap();
        c.forget(2);
        assert!(c.idle());
        portal.fill().unwrap();
        let mut close = None;
        while let Some(m) = portal.recv().unwrap() {
            if m.is(REQUEST, "Close") {
                close = Some(m);
            }
        }
        let close = close.expect("Close sent");
        assert_eq!(close.path.as_deref(), Some(path.as_str()));
        assert!(close.no_reply());
        // A late answer for it goes nowhere.
        portal.send(response(&path, "file:///tmp/b.txt")).unwrap();
        assert!(c.ready().is_empty());
    }

    #[test]
    fn the_portal_leaving_ends_its_picks() {
        let (mut c, mut portal) = client();
        let path = format!("{}/gh0st5", requests(ME));
        portal.send(reply(1, &path)).unwrap();
        c.pick(5, 0, &PickSpec::open()).unwrap();
        let mut gone = Message::signal(gh0st_dbus::bus::PATH, gh0st_dbus::bus::NAME, "NameOwnerChanged")
            .arg(Value::str(NAME))
            .arg(Value::str(PORTAL))
            .arg(Value::str(""));
        // Only the bus says who left.
        gone.sender = Some(":1.99".into());
        portal.send(gone).unwrap();
        assert!(c.ready().is_empty());
        let mut gone = Message::signal(gh0st_dbus::bus::PATH, gh0st_dbus::bus::NAME, "NameOwnerChanged")
            .arg(Value::str(NAME))
            .arg(Value::str(PORTAL))
            .arg(Value::str(""));
        gone.sender = Some(gh0st_dbus::bus::NAME.into());
        portal.send(gone).unwrap();
        assert_eq!(c.ready(), [(5, 0, None)]);
    }
}
