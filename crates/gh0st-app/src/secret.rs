//! The keyring, for apps: store a password, find it again, and wait for the
//! user when the keyring is locked. It speaks the freedesktop Secret Service
//! on the session bus, as libsecret does (`docs/keyring.md`).
//!
//! Calls answer at once. Only an unlock or a delete may need the user: it
//! returns a prompt path, the shell asks, and the end arrives on [`Keyring::fd`],
//! which the page watches. [`Keyring::ended`] then says how it went.
//!
//! ```no_run
//! use gh0st_app::secret::{Keyring, Lookup};
//!
//! let mut kr = Keyring::connect();
//! let who = [("server", "nas"), ("user", "me")];
//! kr.store("nas", &who, b"hunter2", true).ok();
//! match kr.lookup(&who) {
//!     Lookup::Found(pw) => { let _ = pw.as_bytes(); }
//!     Lookup::Locked(items) => { let _prompt = kr.unlock(&items); }
//!     Lookup::Missing | Lookup::Absent => {}
//! }
//! ```

use std::os::fd::RawFd;

use gh0st_dbus::{Conn, Kind, Message, Value};

const NAME: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SVC: &str = "org.freedesktop.Secret.Service";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const DEFAULT: &str = "/org/freedesktop/secrets/aliases/default";
const TIMEOUT: u64 = 2000;

/// How calls reach the keyring: the session bus, or in tests the service
/// itself.
pub trait Wire {
    /// Sends a call and returns its reply. An error reply is `Err` with its
    /// name.
    fn call(&mut self, m: Message) -> Result<Message, String>;
    /// What came unasked since the last read: signals.
    fn incoming(&mut self) -> Vec<Message>;
    /// The fd to watch for them; None when they come at once.
    fn fd(&self) -> Option<RawFd>;
}

struct Bus(Conn);

impl Wire for Bus {
    fn call(&mut self, m: Message) -> Result<Message, String> {
        self.0.call(m, TIMEOUT).map_err(|e| e.to_string())
    }

    fn incoming(&mut self) -> Vec<Message> {
        let _ = self.0.fill();
        let mut out = Vec::new();
        while let Ok(Some(m)) = self.0.recv() {
            out.push(m);
        }
        out
    }

    fn fd(&self) -> Option<RawFd> {
        Some(self.0.fd())
    }
}

/// A secret read from the keyring. Its bytes are zeroed when it drops.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Secret {
        Secret(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The secret as text, when it is UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// Zeroes `b` so the compiler can't skip it.
pub fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: `x` is a valid, aligned byte of `b`.
        unsafe { std::ptr::write_volatile(x, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// What [`Keyring::lookup`] found.
#[derive(Debug)]
pub enum Lookup {
    Found(Secret),
    /// Items match but the keyring is locked, or they are another app's:
    /// pass them to [`Keyring::unlock`] and look again once it ends.
    Locked(Vec<String>),
    Missing,
    /// No keyring answers.
    Absent,
}

/// The app's line to the keyring.
pub struct Keyring {
    wire: Option<Box<dyn Wire>>,
    session: Option<String>,
    /// Prompts that ended and haven't been asked about: path, dismissed.
    done: Vec<(String, bool)>,
    dirty: bool,
}

impl Keyring {
    /// The keyring on the session bus. Without a bus every call answers
    /// [`Lookup::Absent`] or an error.
    pub fn connect() -> Keyring {
        let wire = Conn::session().ok().map(|mut c| {
            // Prompts ending, locks, items coming and going.
            let _ = gh0st_dbus::bus::add_match(
                &mut c,
                "type='signal',sender='org.freedesktop.secrets'",
            );
            Box::new(Bus(c)) as Box<dyn Wire>
        });
        Keyring::over(wire)
    }

    /// A keyring over another wire, as tests have it.
    pub fn over(wire: Option<Box<dyn Wire>>) -> Keyring {
        Keyring {
            wire,
            session: None,
            done: Vec::new(),
            dirty: false,
        }
    }

    pub fn present(&self) -> bool {
        self.wire.is_some()
    }

    /// One call to the keyring; the reply's body.
    pub fn call(
        &mut self,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, String> {
        let wire = self.wire.as_mut().ok_or("No keyring")?;
        let mut m = Message::call(NAME, path, iface, member);
        m.body = args;
        let r = wire.call(m);
        self.pump();
        r.map(|mut r| std::mem::take(&mut r.body))
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

    /// Starts the prompt a reply names. None when there is nothing to ask
    /// ("/").
    pub fn prompt(&mut self, path: Option<&Value>) -> Result<Option<String>, String> {
        let p = path.and_then(Value::as_str).unwrap_or("/").to_string();
        if p == "/" {
            return Ok(None);
        }
        self.call(&p, PROMPT, "Prompt", vec![Value::str("")])?;
        Ok(Some(p))
    }

    /// The items whose attributes include all of `attrs`: those this app may
    /// read now, and those locked to it.
    pub fn search(&mut self, attrs: &[(&str, &str)]) -> Result<(Vec<String>, Vec<String>), String> {
        let b = self.call(
            ROOT,
            SVC,
            "SearchItems",
            vec![Value::str_dict(attrs.iter().copied())],
        )?;
        let list = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        };
        Ok((list(b.first()), list(b.get(1))))
    }

    /// The secret of the first item matching `attrs`.
    pub fn lookup(&mut self, attrs: &[(&str, &str)]) -> Lookup {
        if self.wire.is_none() {
            return Lookup::Absent;
        }
        let (open, shut) = match self.search(attrs) {
            Ok(r) => r,
            Err(e) if e.ends_with("IsLocked") => (Vec::new(), Vec::new()),
            Err(_) => return Lookup::Absent,
        };
        if let Some(item) = open.first() {
            return match self.item_secret(item) {
                Some(s) => Lookup::Found(s),
                None => Lookup::Locked(vec![item.clone()]),
            };
        }
        if shut.is_empty() {
            Lookup::Missing
        } else {
            Lookup::Locked(shut)
        }
    }

    /// One item's secret, when this app may read it now.
    pub fn item_secret(&mut self, item: &str) -> Option<Secret> {
        let s = self.session().ok()?;
        let mut b = self
            .call(item, ITEM, "GetSecret", vec![Value::path(s)])
            .ok()?;
        let out = b.first()?.as_struct()?.get(2)?.as_bytes()?.to_vec();
        b.iter_mut().for_each(Value::wipe);
        Some(Secret(out))
    }

    /// Stores `secret` under `label` and `attrs` in the default collection.
    /// With `replace`, an item of this app with the same attributes takes
    /// the new secret instead of a second item appearing.
    pub fn store(
        &mut self,
        label: &str,
        attrs: &[(&str, &str)],
        secret: &[u8],
        replace: bool,
    ) -> Result<(), String> {
        let s = self.session()?;
        let props = Value::props([
            (format!("{ITEM}.Label"), Value::str(label)),
            (
                format!("{ITEM}.Attributes"),
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
            DEFAULT,
            "org.freedesktop.Secret.Collection",
            "CreateItem",
            vec![props, secret, Value::Bool(replace)],
        )
        .map(drop)
    }

    /// Asks the user to open `items`, or the whole keyring when empty.
    /// Returns the prompt to wait for, None when nothing needed asking.
    pub fn unlock(&mut self, items: &[String]) -> Result<Option<String>, String> {
        let what = if items.is_empty() {
            vec![format!("{ROOT}/collection/login")]
        } else {
            items.to_vec()
        };
        let b = self.call(ROOT, SVC, "Unlock", vec![Value::paths(what)])?;
        self.prompt(b.get(1))
    }

    /// Deletes an item. The keyring asks the user first: wait for the
    /// prompt it returns.
    pub fn delete(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.call(item, ITEM, "Delete", vec![])?;
        self.prompt(b.first())
    }

    /// The fd to watch for prompts ending and changes.
    pub fn fd(&self) -> Option<RawFd> {
        self.wire.as_ref().and_then(|w| w.fd())
    }

    /// Whether prompt `path` ended, and if so whether it was dismissed.
    pub fn ended(&mut self, path: &str) -> Option<bool> {
        self.pump();
        let i = self.done.iter().position(|(p, _)| p == path)?;
        Some(self.done.remove(i).1)
    }

    /// The keyring changed since the last call: locked, unlocked, an item
    /// added or gone.
    pub fn changed(&mut self) -> bool {
        self.pump();
        std::mem::take(&mut self.dirty)
    }

    fn pump(&mut self) {
        let Some(w) = self.wire.as_mut() else {
            return;
        };
        for m in w.incoming() {
            if m.is(PROMPT, "Completed") {
                let dismissed = m.body.first().and_then(Value::as_bool).unwrap_or(true);
                self.done
                    .push((m.path.clone().unwrap_or_default(), dismissed));
            } else if m.kind == Kind::Signal {
                self.dirty = true;
            }
        }
    }
}
