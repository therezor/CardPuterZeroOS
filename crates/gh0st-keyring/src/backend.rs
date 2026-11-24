//! What the app asks the keyring, behind a trait so tests and mockups can
//! put the real service behind it with no bus.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

use gh0st_dbus::{Conn, Message, Value};

pub const NM_SCHEMA: &str = "org.freedesktop.NetworkManager.Connection";
/// What the app stores when the user adds a password by hand.
pub const OUR_SCHEMA: &str = "org.gh0st.Password";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRow {
    pub path: String,
    pub label: String,
    /// The app that stored it: a desktop stem or `exe:<path>`.
    pub app: String,
    pub schema: String,
}

impl ItemRow {
    pub fn wifi(&self) -> bool {
        self.schema == NM_SCHEMA
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRow {
    pub app: String,
    pub item: String,
    pub label: String,
}

/// What the keyring says of itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No keyring answers.
    Absent,
    Locked,
    Open,
}

pub trait Backend {
    fn state(&mut self) -> State;
    fn items(&mut self) -> Vec<ItemRow>;
    fn access(&mut self) -> Vec<AccessRow>;
    fn revoke(&mut self, app: &str, item: &str) -> bool;
    fn lock(&mut self);
    /// Each starts a prompt the shell shows, and returns its path; None
    /// when there was nothing to ask.
    fn unlock(&mut self) -> Result<Option<String>, String>;
    fn reveal(&mut self, item: &str) -> Result<Option<String>, String>;
    fn delete(&mut self, item: &str) -> Result<Option<String>, String>;
    /// The secret, once a reveal let this app read it.
    fn secret(&mut self, item: &str) -> Option<Vec<u8>>;
    fn store(&mut self, label: &str, attrs: &[(&str, &str)], secret: &[u8]) -> Result<(), String>;
    /// The fd to watch for prompt results and changes; None when they come
    /// at once.
    fn fd(&self) -> Option<RawFd>;
    /// Whether prompt `path` ended, and if so whether it was dismissed.
    fn ended(&mut self, path: &str) -> Option<bool>;
    /// The keyring changed since the last call: locked, unlocked, an item
    /// added or gone.
    fn changed(&mut self) -> bool;
}

pub type Shared = Rc<RefCell<Box<dyn Backend>>>;

/// The app's name for a caller id.
pub fn app_label(app: &str) -> String {
    match app.strip_prefix("exe:") {
        Some(path) => path.rsplit('/').next().unwrap_or(path).to_string(),
        None if app.starts_with("bus:") => "Unknown".into(),
        None => gh0st_core::ident::app_label(app),
    }
}

const NAME: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SVC: &str = "org.freedesktop.Secret.Service";
const OURS: &str = "org.gh0st.Keyring";
const TIMEOUT: u64 = 2000;

/// The real keyring, on the session bus. Calls that answer at once block
/// briefly; a prompt's end comes later, on the fd.
struct Dbus {
    conn: Option<Conn>,
    session: Option<String>,
    /// Prompts that ended and haven't been asked about.
    done: Vec<(String, bool)>,
    dirty: bool,
}

pub fn connect() -> Shared {
    let mut conn = Conn::session().ok();
    if let Some(c) = &mut conn {
        // The keyring's signals: prompts ending, locks, items coming and going.
        let rule = "type='signal',sender='org.freedesktop.secrets'";
        let _ = gh0st_dbus::bus::add_match(c, rule);
    }
    Rc::new(RefCell::new(Box::new(Dbus {
        conn,
        session: None,
        done: Vec::new(),
        dirty: false,
    })))
}

impl Dbus {
    /// Reads what the bus has: prompt ends kept by path, the rest a change.
    fn pump(&mut self) {
        let Some(conn) = &mut self.conn else {
            return;
        };
        let _ = conn.fill();
        while let Ok(Some(m)) = conn.recv() {
            if m.is("org.freedesktop.Secret.Prompt", "Completed") {
                let dismissed = m.body.first().and_then(Value::as_bool).unwrap_or(true);
                self.done
                    .push((m.path.clone().unwrap_or_default(), dismissed));
            } else if m.kind == gh0st_dbus::Kind::Signal {
                self.dirty = true;
            }
        }
    }

    fn call(
        &mut self,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, String> {
        let conn = self.conn.as_mut().ok_or("No keyring")?;
        let mut m = Message::call(NAME, path, iface, member);
        m.body = args;
        conn.call(m, TIMEOUT)
            .map(|mut r| std::mem::take(&mut r.body))
            .map_err(|e| e.to_string())
    }

    fn session(&mut self) -> Result<String, String> {
        if let Some(s) = &self.session {
            return Ok(s.clone());
        }
        let b = self.call(
            ROOT,
            SVC,
            "OpenSession",
            vec![Value::str("plain"), Value::variant(Value::str(""))],
        )?;
        let s = b
            .get(1)
            .and_then(Value::as_str)
            .ok_or("No session")?
            .to_string();
        self.session = Some(s.clone());
        Ok(s)
    }

    /// Runs a prompt path from a reply: "/" means done already.
    fn prompt(&mut self, path: Option<&Value>) -> Result<Option<String>, String> {
        let p = path.and_then(Value::as_str).unwrap_or("/").to_string();
        if p == "/" {
            return Ok(None);
        }
        self.call(
            &p,
            "org.freedesktop.Secret.Prompt",
            "Prompt",
            vec![Value::str("")],
        )?;
        Ok(Some(p))
    }
}

impl Backend for Dbus {
    fn state(&mut self) -> State {
        match self.call(ROOT, OURS, "State", vec![]) {
            Ok(b) if b.first().and_then(Value::as_bool) == Some(true) => State::Open,
            Ok(_) => State::Locked,
            Err(_) => State::Absent,
        }
    }

    fn items(&mut self) -> Vec<ItemRow> {
        let Ok(b) = self.call(ROOT, OURS, "Items", vec![]) else {
            return Vec::new();
        };
        b.first()
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| {
                let f = r.as_struct()?;
                Some(ItemRow {
                    path: f.first()?.as_str()?.into(),
                    label: f.get(1)?.as_str()?.into(),
                    app: f.get(2)?.as_str()?.into(),
                    schema: f.get(3)?.as_str()?.into(),
                })
            })
            .collect()
    }

    fn access(&mut self) -> Vec<AccessRow> {
        let Ok(b) = self.call(ROOT, OURS, "Access", vec![]) else {
            return Vec::new();
        };
        b.first()
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| {
                let f = r.as_struct()?;
                Some(AccessRow {
                    app: f.first()?.as_str()?.into(),
                    item: f.get(1)?.as_str()?.into(),
                    label: f.get(2)?.as_str()?.into(),
                })
            })
            .collect()
    }

    fn revoke(&mut self, app: &str, item: &str) -> bool {
        self.call(
            ROOT,
            OURS,
            "Revoke",
            vec![Value::str(app), Value::path(item)],
        )
        .is_ok()
    }

    fn lock(&mut self) {
        let _ = self.call(
            ROOT,
            SVC,
            "Lock",
            vec![Value::paths([format!("{ROOT}/aliases/default")])],
        );
    }

    fn unlock(&mut self) -> Result<Option<String>, String> {
        let coll = format!("{ROOT}/collection/login");
        let b = self.call(ROOT, SVC, "Unlock", vec![Value::paths([coll])])?;
        self.prompt(b.get(1))
    }

    fn reveal(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.call(ROOT, OURS, "Reveal", vec![Value::path(item)])?;
        self.prompt(b.first())
    }

    fn delete(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.call(item, "org.freedesktop.Secret.Item", "Delete", vec![])?;
        self.prompt(b.first())
    }

    fn secret(&mut self, item: &str) -> Option<Vec<u8>> {
        let s = self.session().ok()?;
        let mut b = self
            .call(
                item,
                "org.freedesktop.Secret.Item",
                "GetSecret",
                vec![Value::path(s)],
            )
            .ok()?;
        let out = b.first()?.as_struct()?.get(2)?.as_bytes()?.to_vec();
        b.iter_mut().for_each(Value::wipe);
        Some(out)
    }

    fn store(&mut self, label: &str, attrs: &[(&str, &str)], secret: &[u8]) -> Result<(), String> {
        let s = self.session()?;
        let props = Value::props([
            ("org.freedesktop.Secret.Item.Label", Value::str(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::str_dict(attrs.iter().copied()),
            ),
        ]);
        let secret = Value::Struct(vec![
            Value::path(s),
            Value::Bytes(Vec::new()),
            Value::Bytes(secret.to_vec()),
            Value::str("text/plain"),
        ]);
        self.call(
            &format!("{ROOT}/aliases/default"),
            "org.freedesktop.Secret.Collection",
            "CreateItem",
            vec![props, secret, Value::Bool(false)],
        )
        .map(drop)
    }

    fn fd(&self) -> Option<RawFd> {
        self.conn.as_ref().map(Conn::fd)
    }

    fn ended(&mut self, path: &str) -> Option<bool> {
        self.pump();
        let i = self.done.iter().position(|(p, _)| p == path)?;
        Some(self.done.remove(i).1)
    }

    fn changed(&mut self) -> bool {
        self.pump();
        std::mem::take(&mut self.dirty)
    }
}
