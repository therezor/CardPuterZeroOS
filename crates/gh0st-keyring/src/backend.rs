//! What the app asks the keyring, behind a trait so tests and mockups can
//! put the real service behind it with no bus.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

use gh0st_app::secret::Keyring;
use gh0st_dbus::Value;

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

const ROOT: &str = "/org/freedesktop/secrets";
const SVC: &str = "org.freedesktop.Secret.Service";
const OURS: &str = "org.gh0st.Keyring";

/// The real keyring, on the session bus, through the SDK's client plus
/// `org.gh0st.Keyring` for what the standard lacks.
struct Dbus(Keyring);

pub fn connect() -> Shared {
    Rc::new(RefCell::new(Box::new(Dbus(Keyring::connect()))))
}

impl Backend for Dbus {
    fn state(&mut self) -> State {
        match self.0.call(ROOT, OURS, "State", vec![]) {
            Ok(b) if b.first().and_then(Value::as_bool) == Some(true) => State::Open,
            Ok(_) => State::Locked,
            Err(_) => State::Absent,
        }
    }

    fn items(&mut self) -> Vec<ItemRow> {
        let Ok(b) = self.0.call(ROOT, OURS, "Items", vec![]) else {
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
        let Ok(b) = self.0.call(ROOT, OURS, "Access", vec![]) else {
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
        self.0
            .call(
                ROOT,
                OURS,
                "Revoke",
                vec![Value::str(app), Value::path(item)],
            )
            .is_ok()
    }

    fn lock(&mut self) {
        let _ = self.0.call(
            ROOT,
            SVC,
            "Lock",
            vec![Value::paths([format!("{ROOT}/aliases/default")])],
        );
    }

    fn unlock(&mut self) -> Result<Option<String>, String> {
        self.0.unlock(&[])
    }

    fn reveal(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.0.call(ROOT, OURS, "Reveal", vec![Value::path(item)])?;
        self.0.prompt(b.first())
    }

    fn delete(&mut self, item: &str) -> Result<Option<String>, String> {
        self.0.delete(item)
    }

    fn secret(&mut self, item: &str) -> Option<Vec<u8>> {
        self.0.item_secret(item).map(|s| s.as_bytes().to_vec())
    }

    fn store(&mut self, label: &str, attrs: &[(&str, &str)], secret: &[u8]) -> Result<(), String> {
        self.0.store(label, attrs, secret, false)
    }

    fn fd(&self) -> Option<RawFd> {
        self.0.fd()
    }

    fn ended(&mut self, path: &str) -> Option<bool> {
        self.0.ended(path)
    }

    fn changed(&mut self) -> bool {
        self.0.changed()
    }
}
