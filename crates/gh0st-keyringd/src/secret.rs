//! The freedesktop Secret Service, as a state machine with no I/O but the
//! vault file. Calls come in with who made them; replies, signals and
//! questions for the shell go out.
//!
//! A secret an app may not read looks locked to that app. `Unlock` then
//! asks the shell, and the answer (once, always, deny) decides.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Ty, Value};

use crate::db::{Approval, Collection, Db, Item};
use crate::secure::Locked;
use crate::vault::{self, Cost, Vault};

pub const NAME: &str = "org.freedesktop.secrets";
pub const ROOT: &str = "/org/freedesktop/secrets";
pub const SVC: &str = "org.freedesktop.Secret.Service";
pub const COLL: &str = "org.freedesktop.Secret.Collection";
pub const ITEM: &str = "org.freedesktop.Secret.Item";
pub const SESSION: &str = "org.freedesktop.Secret.Session";
pub const PROMPT: &str = "org.freedesktop.Secret.Prompt";

pub const IS_LOCKED: &str = "org.freedesktop.Secret.Error.IsLocked";
pub const NO_SESSION: &str = "org.freedesktop.Secret.Error.NoSession";
pub const NO_SUCH_OBJECT: &str = "org.freedesktop.Secret.Error.NoSuchObject";

/// NetworkManager's schema: its secrets stay readable to it while locked.
pub const NM_SCHEMA: &str = "org.freedesktop.NetworkManager.Connection";
/// Wrong passwords a prompt takes before it gives up.
const TRIES: u32 = 5;

/// Who is calling.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Caller {
    /// The connection: sessions, prompts and "allow once" belong to it.
    pub bus: String,
    /// The app, from its cgroup: approvals belong to it.
    pub app: String,
    /// The app's name, for prompts.
    pub label: String,
}

/// A question for the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Password {
        id: u64,
        reason: String,
        retry: bool,
    },
    Approve {
        id: u64,
        app: String,
        what: String,
    },
    Cancel {
        id: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Once,
    Always,
    Deny,
}

#[derive(Debug)]
pub enum Out {
    Signal(Message),
    Ask(Ask),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Collection(String),
    Item(String, u64),
}

#[derive(Clone, Debug)]
enum Action {
    Unlock(Vec<Target>),
    Delete(Target),
    CreateCollection { label: String, alias: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Password,
    Approve,
}

struct Prompt {
    caller: Caller,
    action: Action,
    step: Step,
    tries: u32,
    /// Items waiting for the approval answer.
    pending: Vec<u64>,
}

/// What an unlock from the shell found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlock {
    Opened,
    /// A new vault, made with this password.
    Created,
    Wrong,
    /// The file is damaged or unreadable.
    Broken,
}

pub struct Service {
    path: PathBuf,
    cost: Cost,
    /// Labels and attributes, kept from the first unlock on.
    db: Option<Db>,
    /// The data key: Some while unlocked.
    vault: Option<Vault>,
    sessions: HashMap<String, String>,
    next_session: u64,
    prompts: HashMap<u64, Prompt>,
    next_prompt: u64,
    once: HashSet<(String, u64)>,
    out: Vec<Out>,
    /// A password that opened the device but not the vault, kept until
    /// the old one arrives to rewrap.
    stale: Option<Locked>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn coll_path(id: &str) -> String {
    format!("{ROOT}/collection/{id}")
}

fn item_path(c: &str, id: u64) -> String {
    format!("{ROOT}/collection/{c}/{id}")
}

fn prompt_path(id: u64) -> String {
    format!("{ROOT}/prompt/p{id}")
}

/// An object path element from a label.
fn path_id(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if s.is_empty() { "collection".into() } else { s }
}

impl Service {
    pub fn new(path: PathBuf, cost: Cost) -> Service {
        Service {
            path,
            cost,
            db: None,
            vault: None,
            sessions: HashMap::new(),
            next_session: 0,
            prompts: HashMap::new(),
            next_prompt: 0,
            once: HashSet::new(),
            out: Vec::new(),
            stale: None,
        }
    }

    pub fn unlocked(&self) -> bool {
        self.vault.is_some()
    }

    /// Takes what the service wants sent.
    pub fn drain(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    pub fn db(&self) -> Option<&Db> {
        self.db.as_ref()
    }

    // ---- unlock, lock, rewrap: the shell's side ----

    /// Opens the vault, or makes one if there is none.
    pub fn unlock(&mut self, password: &[u8]) -> Unlock {
        if self.vault.is_some() {
            return Unlock::Opened;
        }
        let next = self.path.with_extension("next");
        let file = match std::fs::read(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !next.exists() => {
                return self.create(password);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Unlock::Broken,
        };
        let opened = match vault::Vault::open(&file, password) {
            Ok(v) => Ok(v),
            // A password change that passwd finished and we never heard of.
            Err(e) => match std::fs::read(&next)
                .ok()
                .map(|f| vault::Vault::open(&f, password))
            {
                Some(Ok(v)) => {
                    if std::fs::rename(&next, &self.path).is_err() {
                        return Unlock::Broken;
                    }
                    Ok(v)
                }
                _ => Err(e),
            },
        };
        match opened {
            Ok((v, db)) => {
                self.vault = Some(v);
                self.db = Some(db);
                self.unlocked_now();
                Unlock::Opened
            }
            Err(vault::Error::Wrong) => Unlock::Wrong,
            Err(vault::Error::Corrupt) => Unlock::Broken,
        }
    }

    fn create(&mut self, password: &[u8]) -> Unlock {
        let Ok(v) = Vault::create(password, self.cost) else {
            return Unlock::Broken;
        };
        let t = now();
        let mut db = Db {
            next_id: 1,
            ..Db::default()
        };
        db.collections.push(Collection {
            id: "login".into(),
            label: "Login".into(),
            created: t,
            modified: t,
            items: Vec::new(),
        });
        db.set_alias("default", Some("login"));
        self.vault = Some(v);
        self.db = Some(db);
        if !self.save() {
            self.vault = None;
            self.db = None;
            return Unlock::Broken;
        }
        self.unlocked_now();
        Unlock::Created
    }

    fn unlocked_now(&mut self) {
        let ids: Vec<String> = self.collection_ids();
        for id in ids {
            self.locked_changed(&id);
        }
        // Prompts that waited for the device to unlock go on.
        let waiting: Vec<u64> = self
            .prompts
            .iter()
            .filter(|(_, p)| p.step == Step::Password)
            .map(|(id, _)| *id)
            .collect();
        for id in waiting {
            self.out.push(Out::Ask(Ask::Cancel { id }));
            self.after_unlock(id);
        }
    }

    /// Drops the data key and every secret but NetworkManager's.
    pub fn lock(&mut self) {
        if self.vault.take().is_none() {
            return;
        }
        if let Some(db) = &mut self.db {
            db.forget_secrets(|i| i.attr("xdg:schema") == Some(NM_SCHEMA));
        }
        for id in self.collection_ids() {
            self.locked_changed(&id);
        }
    }

    /// The shell's lock screen took this password but the vault didn't:
    /// keep it until the old one comes.
    pub fn hold_stale(&mut self, password: &[u8]) {
        self.stale = Some(Locked::new(password));
    }

    /// The old password, to rewrap with the one held by `hold_stale`.
    pub fn rewrap_from_old(&mut self, old: &[u8]) -> Unlock {
        let Some(new) = self.stale.take() else {
            return Unlock::Broken;
        };
        match self.unlock(old) {
            Unlock::Opened => {}
            other => {
                self.stale = Some(new);
                return other;
            }
        }
        self.change_password(new.bytes())
    }

    /// Wraps the data key under `new` and saves. Unlocked only.
    pub fn change_password(&mut self, new: &[u8]) -> Unlock {
        let cost = self.cost;
        let Some(v) = &mut self.vault else {
            return Unlock::Wrong;
        };
        if v.wrap(new, cost).is_err() || !self.save() {
            return Unlock::Broken;
        }
        Unlock::Opened
    }

    /// Writes `vault.next` sealed with `new`, after checking `old`. Settings
    /// calls this before passwd, then [`commit`](Service::commit).
    pub fn prepare(&mut self, old: &[u8], new: &[u8]) -> bool {
        let Ok(file) = std::fs::read(&self.path) else {
            return false;
        };
        let Ok((mut v, db)) = Vault::open(&file, old) else {
            return false;
        };
        if v.wrap(new, self.cost).is_err() {
            return false;
        }
        vault::write_atomic(&self.path.with_extension("next"), &v.seal(&db)).is_ok()
    }

    pub fn commit(&mut self, ok: bool) -> bool {
        let next = self.path.with_extension("next");
        if !ok {
            return std::fs::remove_file(next).is_ok();
        }
        if std::fs::rename(&next, &self.path).is_err() {
            return false;
        }
        // The open vault holds the old wrap: lock, so the next unlock
        // reads the new file.
        self.lock();
        true
    }

    /// Writes the vault. Unlocked only.
    fn save(&mut self) -> bool {
        let (Some(v), Some(db)) = (&self.vault, &self.db) else {
            return false;
        };
        let file = v.seal(db);
        match vault::write_atomic(&self.path, &file) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("gh0st keyring: can't save the vault: {e}");
                false
            }
        }
    }

    // ---- what a caller may see ----

    fn may_read(&self, c: &Caller, item: &Item) -> bool {
        item.creator == c.app
            || self.once.contains(&(c.bus.clone(), item.id))
            || self
                .db
                .as_ref()
                .is_some_and(|db| db.approved(&c.app, item.id))
    }

    /// Locked to this caller: the vault is, or the item is someone else's.
    fn locked_for(&self, c: &Caller, item: &Item) -> bool {
        !self.unlocked() || !self.may_read(c, item)
    }

    fn collection_ids(&self) -> Vec<String> {
        self.db
            .as_ref()
            .map(|d| d.collections.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default()
    }

    fn locked_changed(&mut self, coll: &str) {
        let locked = !self.unlocked();
        self.out.push(Out::Signal(bus::changed(
            &coll_path(coll),
            COLL,
            vec![("Locked", Value::Bool(locked))],
        )));
        self.out.push(Out::Signal(
            Message::signal(ROOT, SVC, "CollectionChanged").arg(Value::path(coll_path(coll))),
        ));
    }

    /// A collection id from a collection or alias path.
    fn resolve(&self, path: &str) -> Option<String> {
        let db = self.db.as_ref()?;
        if let Some(a) = path.strip_prefix(&format!("{ROOT}/aliases/")) {
            return db.alias(a).map(String::from);
        }
        let id = path.strip_prefix(&format!("{ROOT}/collection/"))?;
        db.collection(id).map(|c| c.id.clone())
    }

    fn resolve_item(&self, path: &str) -> Option<(String, u64)> {
        let rest = path.strip_prefix(&format!("{ROOT}/collection/"))?;
        let (c, n) = rest.split_once('/')?;
        let n: u64 = n.parse().ok()?;
        let db = self.db.as_ref()?;
        db.collection(c)?.items.iter().find(|i| i.id == n)?;
        Some((c.into(), n))
    }

    fn item(&self, c: &str, n: u64) -> Option<&Item> {
        self.db
            .as_ref()?
            .collection(c)?
            .items
            .iter()
            .find(|i| i.id == n)
    }

    fn item_mut(&mut self, c: &str, n: u64) -> Option<&mut Item> {
        self.db
            .as_mut()?
            .collection_mut(c)?
            .items
            .iter_mut()
            .find(|i| i.id == n)
    }

    /// Every item matching `attrs`, as (collection, item).
    fn search(&self, attrs: &[(String, String)], only: Option<&str>) -> Vec<(String, u64)> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        db.collections
            .iter()
            .filter(|c| only.is_none_or(|o| o == c.id))
            .flat_map(|c| {
                c.items
                    .iter()
                    .filter(|i| i.matches(attrs))
                    .map(|i| (c.id.clone(), i.id))
            })
            .collect()
    }

    // ---- calls ----

    /// Handles one call. The reply is None for calls that want none.
    pub fn call(&mut self, m: &Message, c: &Caller) -> Option<Message> {
        let reply = self.dispatch(m, c);
        if m.no_reply() { None } else { Some(reply) }
    }

    fn dispatch(&mut self, m: &Message, c: &Caller) -> Message {
        let path = m.path.as_deref().unwrap_or("/");
        let iface = m.interface.as_deref().unwrap_or("");
        let member = m.member.as_deref().unwrap_or("");
        if let Some(r) = bus::peer(m) {
            return r;
        }
        if iface == bus::INTROSPECTABLE && member == "Introspect" {
            return Message::reply(m).arg(Value::str(self.introspect(path)));
        }
        if iface == bus::PROPERTIES {
            return self.properties(m, c, path, member);
        }
        let r = if path == ROOT {
            self.service_call(m, c, member)
        } else if let Some(id) = path.strip_prefix(&format!("{ROOT}/prompt/p")) {
            id.parse().ok().map(|id| self.prompt_call(m, c, id, member))
        } else if path.starts_with(&format!("{ROOT}/session/")) {
            (member == "Close").then(|| {
                if self.sessions.get(path) == Some(&c.bus) {
                    self.sessions.remove(path);
                }
                Message::reply(m)
            })
        } else if let Some((coll, n)) = self.resolve_item(path) {
            self.item_call(m, c, &coll, n, member)
        } else if let Some(coll) = self.resolve(path) {
            self.collection_call(m, c, &coll, member)
        } else {
            return Message::error(m, NO_SUCH_OBJECT, "no such object");
        };
        r.unwrap_or_else(|| Message::error(m, err::UNKNOWN_METHOD, "no such method"))
    }

    fn service_call(&mut self, m: &Message, c: &Caller, member: &str) -> Option<Message> {
        Some(match member {
            "OpenSession" => {
                if m.body.first().and_then(Value::as_str) != Some("plain") {
                    return Some(Message::error(m, err::NOT_SUPPORTED, "only plain sessions"));
                }
                self.next_session += 1;
                let p = format!("{ROOT}/session/s{}", self.next_session);
                self.sessions.insert(p.clone(), c.bus.clone());
                Message::reply(m)
                    .arg(Value::variant(Value::str("")))
                    .arg(Value::path(p))
            }
            "SearchItems" => {
                let attrs = str_dict(m.body.first());
                let (mut open, mut shut) = (Vec::new(), Vec::new());
                for (coll, n) in self.search(&attrs, None) {
                    let Some(item) = self.item(&coll, n) else {
                        continue;
                    };
                    if self.locked_for(c, item) {
                        shut.push(item_path(&coll, n));
                    } else {
                        open.push(item_path(&coll, n));
                    }
                }
                Message::reply(m)
                    .arg(Value::paths(open))
                    .arg(Value::paths(shut))
            }
            "Unlock" => {
                let mut open = Vec::new();
                let mut need = Vec::new();
                for p in paths(m.body.first()) {
                    let target = if let Some((coll, n)) = self.resolve_item(&p) {
                        Target::Item(coll, n)
                    } else if let Some(coll) = self.resolve(&p) {
                        Target::Collection(coll)
                    } else {
                        continue;
                    };
                    if self.open_for(c, &target) {
                        open.push(p);
                    } else {
                        need.push(target);
                    }
                }
                let prompt = if need.is_empty() {
                    "/".to_string()
                } else {
                    self.new_prompt(c, Action::Unlock(need))
                };
                Message::reply(m)
                    .arg(Value::paths(open))
                    .arg(Value::path(prompt))
            }
            "Lock" => {
                self.lock();
                Message::reply(m)
                    .arg(Value::Array(
                        Ty::Path,
                        m.body
                            .first()
                            .and_then(|v| v.as_array())
                            .unwrap_or(&[])
                            .to_vec(),
                    ))
                    .arg(Value::path("/"))
            }
            "GetSecrets" => {
                let Some(session) = self.session_of(m.body.get(1), c) else {
                    return Some(Message::error(m, NO_SESSION, "no such session"));
                };
                let mut out = Vec::new();
                for p in paths(m.body.first()) {
                    let Some((coll, n)) = self.resolve_item(&p) else {
                        continue;
                    };
                    if let Some(s) = self.secret_for(c, &coll, n, &session) {
                        out.push((Value::path(p), s));
                    }
                }
                let reply = Message::reply(m).arg(Value::dict(Ty::Path, secret_ty(), out));
                return Some(reply);
            }
            "ReadAlias" => {
                let name = m.body.first().and_then(Value::as_str).unwrap_or("");
                let p = self
                    .db
                    .as_ref()
                    .and_then(|d| d.alias(name))
                    .map(coll_path)
                    .unwrap_or_else(|| "/".into());
                Message::reply(m).arg(Value::path(p))
            }
            "SetAlias" => {
                if !self.unlocked() {
                    return Some(Message::error(m, IS_LOCKED, "the keyring is locked"));
                }
                let name = m
                    .body
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let target = m
                    .body
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .to_string();
                let coll = if target == "/" {
                    None
                } else {
                    self.resolve(&target)
                };
                if target != "/" && coll.is_none() {
                    return Some(Message::error(m, NO_SUCH_OBJECT, "no such collection"));
                }
                if let Some(db) = &mut self.db {
                    db.set_alias(&name, coll.as_deref());
                }
                self.save();
                Message::reply(m)
            }
            "CreateCollection" => {
                let props = m.body.first();
                let label = props
                    .and_then(|p| p.get(&format!("{COLL}.Label")))
                    .map(Value::inner)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let alias = m
                    .body
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !alias.is_empty() {
                    if let Some(id) = self.db.as_ref().and_then(|d| d.alias(&alias)) {
                        return Some(
                            Message::reply(m)
                                .arg(Value::path(coll_path(id)))
                                .arg(Value::path("/")),
                        );
                    }
                }
                if !self.unlocked() {
                    let p = self.new_prompt(c, Action::CreateCollection { label, alias });
                    return Some(Message::reply(m).arg(Value::path("/")).arg(Value::path(p)));
                }
                let id = self.create_collection(&label, &alias);
                Message::reply(m)
                    .arg(Value::path(coll_path(&id)))
                    .arg(Value::path("/"))
            }
            _ => return None,
        })
    }

    fn open_for(&self, c: &Caller, t: &Target) -> bool {
        match t {
            Target::Collection(_) => self.unlocked(),
            Target::Item(coll, n) => self.item(coll, *n).is_some_and(|i| !self.locked_for(c, i)),
        }
    }

    fn create_collection(&mut self, label: &str, alias: &str) -> String {
        let Some(db) = &mut self.db else {
            return String::new();
        };
        let base = path_id(label);
        let mut id = base.clone();
        let mut k = 1;
        while db.collection(&id).is_some() {
            k += 1;
            id = format!("{base}{k}");
        }
        let t = now();
        db.collections.push(Collection {
            id: id.clone(),
            label: label.into(),
            created: t,
            modified: t,
            items: Vec::new(),
        });
        if !alias.is_empty() {
            db.set_alias(alias, Some(&id));
        }
        self.save();
        self.out.push(Out::Signal(
            Message::signal(ROOT, SVC, "CollectionCreated").arg(Value::path(coll_path(&id))),
        ));
        id
    }

    fn session_of(&self, v: Option<&Value>, c: &Caller) -> Option<String> {
        let p = v?.as_str()?;
        (self.sessions.get(p) == Some(&c.bus)).then(|| p.to_string())
    }

    /// The (oayays) for an item, if this caller may have it.
    fn secret_for(&self, c: &Caller, coll: &str, n: u64, session: &str) -> Option<Value> {
        let item = self.item(coll, n)?;
        if self.locked_for(c, item) {
            return None;
        }
        Some(Value::Struct(vec![
            Value::path(session),
            Value::Bytes(Vec::new()),
            Value::Bytes(item.secret.as_ref()?.bytes().to_vec()),
            Value::str(if item.content_type.is_empty() {
                "text/plain"
            } else {
                &item.content_type
            }),
        ]))
    }

    fn collection_call(
        &mut self,
        m: &Message,
        c: &Caller,
        coll: &str,
        member: &str,
    ) -> Option<Message> {
        Some(match member {
            "SearchItems" => {
                let attrs = str_dict(m.body.first());
                let found = self
                    .search(&attrs, Some(coll))
                    .into_iter()
                    .map(|(c, n)| item_path(&c, n));
                Message::reply(m).arg(Value::paths(found))
            }
            "CreateItem" => {
                if !self.unlocked() {
                    return Some(Message::error(m, IS_LOCKED, "the keyring is locked"));
                }
                let props = m.body.first();
                let label = props
                    .and_then(|p| p.get(&format!("{ITEM}.Label")))
                    .map(Value::inner)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let attrs = str_dict(
                    props
                        .and_then(|p| p.get(&format!("{ITEM}.Attributes")))
                        .map(Value::inner),
                );
                let Some(secret) = m.body.get(1).and_then(Value::as_struct) else {
                    return Some(Message::error(m, err::INVALID_ARGS, "no secret"));
                };
                if self.session_of(secret.first(), c).is_none() {
                    return Some(Message::error(m, NO_SESSION, "no such session"));
                }
                let value = secret.get(2).and_then(Value::as_bytes).unwrap_or_default();
                let ctype = secret
                    .get(3)
                    .and_then(Value::as_str)
                    .unwrap_or("text/plain")
                    .to_string();
                let replace = m.body.get(2).and_then(Value::as_bool).unwrap_or(false);
                let t = now();
                let existing = if replace {
                    self.search(&attrs, Some(coll)).into_iter().find(|(cc, n)| {
                        self.item(cc, *n)
                            .is_some_and(|i| i.attrs.len() == attrs.len())
                    })
                } else {
                    None
                };
                let n = if let Some((_, n)) = existing {
                    let allowed = self.item(coll, n).is_some_and(|i| self.may_read(c, i));
                    if !allowed {
                        return Some(Message::error(
                            m,
                            err::ACCESS_DENIED,
                            "another app's secret",
                        ));
                    }
                    if let Some(i) = self.item_mut(coll, n) {
                        i.label = label;
                        i.secret = Some(Locked::new(value));
                        i.content_type = ctype;
                        i.modified = t;
                    }
                    self.out.push(Out::Signal(
                        Message::signal(&coll_path(coll), COLL, "ItemChanged")
                            .arg(Value::path(item_path(coll, n))),
                    ));
                    n
                } else {
                    let Some(db) = &mut self.db else {
                        return Some(Message::error(m, IS_LOCKED, "the keyring is locked"));
                    };
                    let n = db.next_id.max(1);
                    db.next_id = n + 1;
                    let Some(col) = db.collection_mut(coll) else {
                        return Some(Message::error(m, NO_SUCH_OBJECT, "no such collection"));
                    };
                    col.items.push(Item {
                        id: n,
                        label,
                        attrs,
                        secret: Some(Locked::new(value)),
                        content_type: ctype,
                        created: t,
                        modified: t,
                        creator: c.app.clone(),
                    });
                    col.modified = t;
                    self.out.push(Out::Signal(
                        Message::signal(&coll_path(coll), COLL, "ItemCreated")
                            .arg(Value::path(item_path(coll, n))),
                    ));
                    n
                };
                if !self.save() {
                    return Some(Message::error(m, err::FAILED, "can't save"));
                }
                Message::reply(m)
                    .arg(Value::path(item_path(coll, n)))
                    .arg(Value::path("/"))
            }
            "Delete" => {
                let p = self.new_prompt(c, Action::Delete(Target::Collection(coll.into())));
                Message::reply(m).arg(Value::path(p))
            }
            _ => return None,
        })
    }

    fn item_call(
        &mut self,
        m: &Message,
        c: &Caller,
        coll: &str,
        n: u64,
        member: &str,
    ) -> Option<Message> {
        Some(match member {
            "GetSecret" => {
                let Some(session) = self.session_of(m.body.first(), c) else {
                    return Some(Message::error(m, NO_SESSION, "no such session"));
                };
                match self.secret_for(c, coll, n, &session) {
                    Some(s) => Message::reply(m).arg(s),
                    None => Message::error(m, IS_LOCKED, "locked"),
                }
            }
            "SetSecret" => {
                let ok = self.item(coll, n).is_some_and(|i| !self.locked_for(c, i));
                if !ok {
                    return Some(Message::error(m, IS_LOCKED, "locked"));
                }
                let Some(secret) = m.body.first().and_then(Value::as_struct) else {
                    return Some(Message::error(m, err::INVALID_ARGS, "no secret"));
                };
                if self.session_of(secret.first(), c).is_none() {
                    return Some(Message::error(m, NO_SESSION, "no such session"));
                }
                let value = secret
                    .get(2)
                    .and_then(Value::as_bytes)
                    .unwrap_or_default()
                    .to_vec();
                let ctype = secret
                    .get(3)
                    .and_then(Value::as_str)
                    .unwrap_or("text/plain")
                    .to_string();
                if let Some(i) = self.item_mut(coll, n) {
                    i.secret = Some(Locked::new(&value));
                    i.content_type = ctype;
                    i.modified = now();
                }
                self.changed_item(coll, n);
                Message::reply(m)
            }
            "Delete" => {
                let p = self.new_prompt(c, Action::Delete(Target::Item(coll.into(), n)));
                Message::reply(m).arg(Value::path(p))
            }
            _ => return None,
        })
    }

    fn changed_item(&mut self, coll: &str, n: u64) {
        self.save();
        self.out.push(Out::Signal(
            Message::signal(&coll_path(coll), COLL, "ItemChanged")
                .arg(Value::path(item_path(coll, n))),
        ));
    }

    // ---- properties ----

    fn properties(&mut self, m: &Message, c: &Caller, path: &str, member: &str) -> Message {
        let iface = m
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let props = match self.props_of(c, path) {
            Some((name, props)) if iface.is_empty() || iface == name => props,
            Some(_) => return Message::error(m, err::UNKNOWN_INTERFACE, "no such interface"),
            None => return Message::error(m, NO_SUCH_OBJECT, "no such object"),
        };
        match member {
            "GetAll" => Message::reply(m).arg(Value::props(props)),
            "Get" => {
                let name = m.body.get(1).and_then(Value::as_str).unwrap_or("");
                match props.into_iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => Message::reply(m).arg(Value::variant(v)),
                    None => Message::error(m, err::UNKNOWN_PROPERTY, "no such property"),
                }
            }
            "Set" => {
                let name = m
                    .body
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let v = m.body.get(2).map(Value::inner).cloned();
                self.set_prop(m, c, path, &name, v)
            }
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    fn props_of(
        &self,
        c: &Caller,
        path: &str,
    ) -> Option<(&'static str, Vec<(&'static str, Value)>)> {
        if path == ROOT {
            let colls = self.collection_ids().into_iter().map(|id| coll_path(&id));
            return Some((SVC, vec![("Collections", Value::paths(colls))]));
        }
        if let Some((coll, n)) = self.resolve_item(path) {
            let i = self.item(&coll, n)?;
            let attrs = i.attrs.iter().map(|(k, v)| (k.as_str(), v.as_str()));
            return Some((
                ITEM,
                vec![
                    ("Locked", Value::Bool(self.locked_for(c, i))),
                    ("Attributes", Value::str_dict(attrs)),
                    ("Label", Value::str(&i.label)),
                    ("Type", Value::str(i.attr("xdg:schema").unwrap_or(""))),
                    ("Created", Value::U64(i.created)),
                    ("Modified", Value::U64(i.modified)),
                ],
            ));
        }
        let coll = self.resolve(path)?;
        let col = self.db.as_ref()?.collection(&coll)?;
        Some((
            COLL,
            vec![
                (
                    "Items",
                    Value::paths(col.items.iter().map(|i| item_path(&coll, i.id))),
                ),
                ("Label", Value::str(&col.label)),
                ("Locked", Value::Bool(!self.unlocked())),
                ("Created", Value::U64(col.created)),
                ("Modified", Value::U64(col.modified)),
            ],
        ))
    }

    fn set_prop(
        &mut self,
        m: &Message,
        c: &Caller,
        path: &str,
        name: &str,
        v: Option<Value>,
    ) -> Message {
        if !self.unlocked() {
            return Message::error(m, IS_LOCKED, "the keyring is locked");
        }
        if let Some((coll, n)) = self.resolve_item(path) {
            if !self.item(&coll, n).is_some_and(|i| self.may_read(c, i)) {
                return Message::error(m, IS_LOCKED, "locked");
            }
            let Some(item) = self.item_mut(&coll, n) else {
                return Message::error(m, NO_SUCH_OBJECT, "no such object");
            };
            match (name, &v) {
                ("Label", Some(Value::Str(s))) => item.label = s.clone(),
                ("Attributes", Some(d)) => item.attrs = str_dict(Some(d)),
                _ => return Message::error(m, err::READ_ONLY, "read-only or wrong type"),
            }
            item.modified = now();
            self.changed_item(&coll, n);
            return Message::reply(m);
        }
        if let Some(coll) = self.resolve(path) {
            if let ("Label", Some(Value::Str(s))) = (name, &v) {
                if let Some(col) = self.db.as_mut().and_then(|d| d.collection_mut(&coll)) {
                    col.label = s.clone();
                    col.modified = now();
                }
                self.save();
                return Message::reply(m);
            }
            return Message::error(m, err::READ_ONLY, "read-only or wrong type");
        }
        Message::error(m, err::READ_ONLY, "read-only")
    }

    // ---- prompts ----

    fn new_prompt(&mut self, c: &Caller, action: Action) -> String {
        self.next_prompt += 1;
        let id = self.next_prompt;
        self.prompts.insert(
            id,
            Prompt {
                caller: c.clone(),
                action,
                step: Step::Idle,
                tries: 0,
                pending: Vec::new(),
            },
        );
        prompt_path(id)
    }

    fn prompt_call(&mut self, m: &Message, c: &Caller, id: u64, member: &str) -> Message {
        let Some(p) = self.prompts.get(&id) else {
            return Message::error(m, NO_SUCH_OBJECT, "no such prompt");
        };
        if p.caller.bus != c.bus {
            return Message::error(m, err::ACCESS_DENIED, "not your prompt");
        }
        match member {
            "Prompt" => {
                if p.step == Step::Idle {
                    self.start(id);
                }
                Message::reply(m)
            }
            "Dismiss" => {
                self.out.push(Out::Ask(Ask::Cancel { id }));
                self.complete(id, true, Value::str(""));
                Message::reply(m)
            }
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    fn start(&mut self, id: u64) {
        if self.unlocked() {
            self.after_unlock(id);
            return;
        }
        let Some(p) = self.prompts.get_mut(&id) else {
            return;
        };
        p.step = Step::Password;
        let reason = format!("{} wants the keyring", p.caller.label);
        self.out.push(Out::Ask(Ask::Password {
            id,
            reason,
            retry: false,
        }));
    }

    /// The vault is open: ask about items the caller may not read, or
    /// finish.
    fn after_unlock(&mut self, id: u64) {
        let Some(p) = self.prompts.get(&id) else {
            return;
        };
        let c = p.caller.clone();
        let targets: Vec<(String, u64)> = match &p.action {
            Action::Unlock(ts) => ts
                .iter()
                .filter_map(|t| match t {
                    Target::Item(coll, n) => Some((coll.clone(), *n)),
                    Target::Collection(_) => None,
                })
                .collect(),
            Action::Delete(Target::Item(coll, n)) => vec![(coll.clone(), *n)],
            Action::Delete(Target::Collection(coll)) => self
                .db
                .as_ref()
                .and_then(|d| d.collection(coll))
                .map(|col| col.items.iter().map(|i| (coll.clone(), i.id)).collect())
                .unwrap_or_default(),
            Action::CreateCollection { .. } => Vec::new(),
        };
        let need: Vec<u64> = targets
            .iter()
            .filter(|(coll, n)| self.item(coll, *n).is_some_and(|i| !self.may_read(&c, i)))
            .map(|(_, n)| *n)
            .collect();
        if need.is_empty() {
            self.finish(id);
            return;
        }
        let labels: Vec<String> = targets
            .iter()
            .filter(|(_, n)| need.contains(n))
            .filter_map(|(coll, n)| self.item(coll, *n).map(|i| i.label.clone()))
            .collect();
        let what = match (&self.prompts[&id].action, labels.as_slice()) {
            (Action::Delete(_), _) => format!("delete {}", join_labels(&labels)),
            (_, _) => format!("read {}", join_labels(&labels)),
        };
        if let Some(p) = self.prompts.get_mut(&id) {
            p.step = Step::Approve;
            p.pending = need;
        }
        self.out.push(Out::Ask(Ask::Approve {
            id,
            app: c.label,
            what,
        }));
    }

    /// The shell's answer to a password prompt.
    pub fn password(&mut self, id: u64, pw: &[u8]) {
        let Some(p) = self.prompts.get(&id) else {
            return;
        };
        if p.step != Step::Password {
            return;
        }
        match self.unlock(pw) {
            Unlock::Opened | Unlock::Created => {
                // unlock() moved every waiting prompt on, this one too.
            }
            Unlock::Wrong => {
                let Some(p) = self.prompts.get_mut(&id) else {
                    return;
                };
                p.tries += 1;
                if p.tries >= TRIES {
                    self.out.push(Out::Ask(Ask::Cancel { id }));
                    self.complete(id, true, Value::str(""));
                    return;
                }
                let reason = format!("{} wants the keyring", p.caller.label);
                self.out.push(Out::Ask(Ask::Password {
                    id,
                    reason,
                    retry: true,
                }));
            }
            Unlock::Broken => {
                self.out.push(Out::Ask(Ask::Cancel { id }));
                self.complete(id, true, Value::str(""));
            }
        }
    }

    /// The shell's answer to an approval, or Esc on any prompt.
    pub fn answer(&mut self, id: u64, a: Answer) {
        let Some(p) = self.prompts.get(&id) else {
            return;
        };
        if a == Answer::Deny || p.step != Step::Approve {
            self.complete(id, true, Value::paths::<String>([]));
            return;
        }
        let (bus, app, pending) = (
            p.caller.bus.clone(),
            p.caller.app.clone(),
            p.pending.clone(),
        );
        for n in pending {
            match a {
                Answer::Once => {
                    self.once.insert((bus.clone(), n));
                }
                Answer::Always => {
                    if let Some(db) = &mut self.db {
                        if !db.approved(&app, n) {
                            db.approvals.push(Approval {
                                app: app.clone(),
                                item: n,
                            });
                        }
                    }
                }
                Answer::Deny => {}
            }
        }
        if a == Answer::Always {
            self.save();
        }
        self.finish(id);
    }

    fn finish(&mut self, id: u64) {
        let Some(p) = self.prompts.get(&id) else {
            return;
        };
        let c = p.caller.clone();
        let result = match p.action.clone() {
            Action::Unlock(ts) => {
                let open: Vec<String> = ts
                    .iter()
                    .filter(|t| self.open_for(&c, t))
                    .map(|t| match t {
                        Target::Collection(coll) => coll_path(coll),
                        Target::Item(coll, n) => item_path(coll, *n),
                    })
                    .collect();
                Value::paths(open)
            }
            Action::Delete(t) => {
                self.delete(&t);
                Value::str("")
            }
            Action::CreateCollection { label, alias } => {
                let existing = self
                    .db
                    .as_ref()
                    .and_then(|d| d.alias(&alias))
                    .map(String::from);
                let id = match existing {
                    Some(id) if !alias.is_empty() => id,
                    _ => self.create_collection(&label, &alias),
                };
                Value::path(coll_path(&id))
            }
        };
        self.complete(id, false, result);
    }

    fn delete(&mut self, t: &Target) {
        let Some(db) = &mut self.db else {
            return;
        };
        match t {
            Target::Item(coll, n) => {
                if let Some(col) = db.collection_mut(coll) {
                    col.items.retain(|i| i.id != *n);
                    col.modified = now();
                }
                db.approvals.retain(|a| a.item != *n);
                self.out.push(Out::Signal(
                    Message::signal(&coll_path(coll), COLL, "ItemDeleted")
                        .arg(Value::path(item_path(coll, *n))),
                ));
            }
            Target::Collection(coll) => {
                let gone: Vec<u64> = db
                    .collection(coll)
                    .map(|c| c.items.iter().map(|i| i.id).collect())
                    .unwrap_or_default();
                db.collections.retain(|c| &c.id != coll);
                db.aliases.retain(|(_, c)| c != coll);
                db.approvals.retain(|a| !gone.contains(&a.item));
                self.out.push(Out::Signal(
                    Message::signal(ROOT, SVC, "CollectionDeleted")
                        .arg(Value::path(coll_path(coll))),
                ));
            }
        }
        self.save();
    }

    fn complete(&mut self, id: u64, dismissed: bool, result: Value) {
        let Some(p) = self.prompts.remove(&id) else {
            return;
        };
        let mut s = Message::signal(&prompt_path(id), PROMPT, "Completed")
            .arg(Value::Bool(dismissed))
            .arg(Value::variant(result));
        s.destination = Some(p.caller.bus);
        self.out.push(Out::Signal(s));
    }

    /// A connection left: its sessions, prompts and "once" answers go.
    pub fn gone(&mut self, bus: &str) {
        self.sessions.retain(|_, owner| owner != bus);
        self.once.retain(|(b, _)| b != bus);
        let ids: Vec<u64> = self
            .prompts
            .iter()
            .filter(|(_, p)| p.caller.bus == bus)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.prompts.remove(&id);
            self.out.push(Out::Ask(Ask::Cancel { id }));
        }
    }

    // ---- introspection ----

    fn introspect(&self, path: &str) -> String {
        if path == "/" || path == "/org" || path == "/org/freedesktop" {
            let child = match path {
                "/" => "org",
                "/org" => "freedesktop",
                _ => "secrets",
            };
            return bus::introspect(&[], &[child]);
        }
        if path == ROOT {
            return bus::introspect(
                &[SERVICE_XML, bus::PROPERTIES_XML],
                &["collection", "aliases"],
            );
        }
        if path == format!("{ROOT}/collection") {
            let ids = self.collection_ids();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            return bus::introspect(&[], &refs);
        }
        if path == format!("{ROOT}/aliases") {
            let names: Vec<String> = self
                .db
                .as_ref()
                .map(|d| d.aliases.iter().map(|(a, _)| a.clone()).collect())
                .unwrap_or_default();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            return bus::introspect(&[], &refs);
        }
        if self.resolve_item(path).is_some() {
            return bus::introspect(&[ITEM_XML, bus::PROPERTIES_XML], &[]);
        }
        if let Some(coll) = self.resolve(path) {
            let ids: Vec<String> = self
                .db
                .as_ref()
                .and_then(|d| d.collection(&coll))
                .map(|c| c.items.iter().map(|i| i.id.to_string()).collect())
                .unwrap_or_default();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            return bus::introspect(&[COLLECTION_XML, bus::PROPERTIES_XML], &refs);
        }
        if path.starts_with(&format!("{ROOT}/session/")) {
            return bus::introspect(&[SESSION_XML], &[]);
        }
        if path.starts_with(&format!("{ROOT}/prompt/")) {
            return bus::introspect(&[PROMPT_XML], &[]);
        }
        bus::introspect(&[], &[])
    }
}

fn join_labels(labels: &[String]) -> String {
    match labels {
        [] => "a secret".into(),
        [one] => format!("\u{201c}{one}\u{201d}"),
        [one, rest @ ..] => format!("\u{201c}{one}\u{201d} and {} more", rest.len()),
    }
}

fn str_dict(v: Option<&Value>) -> Vec<(String, String)> {
    v.map(|d| {
        d.entries()
            .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
            .collect()
    })
    .unwrap_or_default()
}

fn paths(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p.as_str().map(String::from))
        .collect()
}

fn secret_ty() -> Ty {
    Ty::Struct(vec![
        Ty::Path,
        Ty::Array(Box::new(Ty::Byte)),
        Ty::Array(Box::new(Ty::Byte)),
        Ty::Str,
    ])
}

const SERVICE_XML: &str = r#" <interface name="org.freedesktop.Secret.Service">
  <method name="OpenSession"><arg name="algorithm" type="s" direction="in"/><arg name="input" type="v" direction="in"/><arg name="output" type="v" direction="out"/><arg name="result" type="o" direction="out"/></method>
  <method name="CreateCollection"><arg name="properties" type="a{sv}" direction="in"/><arg name="alias" type="s" direction="in"/><arg name="collection" type="o" direction="out"/><arg name="prompt" type="o" direction="out"/></method>
  <method name="SearchItems"><arg name="attributes" type="a{ss}" direction="in"/><arg name="unlocked" type="ao" direction="out"/><arg name="locked" type="ao" direction="out"/></method>
  <method name="Unlock"><arg name="objects" type="ao" direction="in"/><arg name="unlocked" type="ao" direction="out"/><arg name="prompt" type="o" direction="out"/></method>
  <method name="Lock"><arg name="objects" type="ao" direction="in"/><arg name="locked" type="ao" direction="out"/><arg name="Prompt" type="o" direction="out"/></method>
  <method name="GetSecrets"><arg name="items" type="ao" direction="in"/><arg name="session" type="o" direction="in"/><arg name="secrets" type="a{o(oayays)}" direction="out"/></method>
  <method name="ReadAlias"><arg name="name" type="s" direction="in"/><arg name="collection" type="o" direction="out"/></method>
  <method name="SetAlias"><arg name="name" type="s" direction="in"/><arg name="collection" type="o" direction="in"/></method>
  <property name="Collections" type="ao" access="read"/>
  <signal name="CollectionCreated"><arg name="collection" type="o"/></signal>
  <signal name="CollectionDeleted"><arg name="collection" type="o"/></signal>
  <signal name="CollectionChanged"><arg name="collection" type="o"/></signal>
 </interface>
"#;

const COLLECTION_XML: &str = r#" <interface name="org.freedesktop.Secret.Collection">
  <method name="Delete"><arg name="prompt" type="o" direction="out"/></method>
  <method name="SearchItems"><arg name="attributes" type="a{ss}" direction="in"/><arg name="results" type="ao" direction="out"/></method>
  <method name="CreateItem"><arg name="properties" type="a{sv}" direction="in"/><arg name="secret" type="(oayays)" direction="in"/><arg name="replace" type="b" direction="in"/><arg name="item" type="o" direction="out"/><arg name="prompt" type="o" direction="out"/></method>
  <property name="Items" type="ao" access="read"/>
  <property name="Label" type="s" access="readwrite"/>
  <property name="Locked" type="b" access="read"/>
  <property name="Created" type="t" access="read"/>
  <property name="Modified" type="t" access="read"/>
  <signal name="ItemCreated"><arg name="item" type="o"/></signal>
  <signal name="ItemDeleted"><arg name="item" type="o"/></signal>
  <signal name="ItemChanged"><arg name="item" type="o"/></signal>
 </interface>
"#;

const ITEM_XML: &str = r#" <interface name="org.freedesktop.Secret.Item">
  <method name="Delete"><arg name="Prompt" type="o" direction="out"/></method>
  <method name="GetSecret"><arg name="session" type="o" direction="in"/><arg name="secret" type="(oayays)" direction="out"/></method>
  <method name="SetSecret"><arg name="secret" type="(oayays)" direction="in"/></method>
  <property name="Locked" type="b" access="read"/>
  <property name="Attributes" type="a{ss}" access="readwrite"/>
  <property name="Label" type="s" access="readwrite"/>
  <property name="Type" type="s" access="read"/>
  <property name="Created" type="t" access="read"/>
  <property name="Modified" type="t" access="read"/>
 </interface>
"#;

const SESSION_XML: &str = r#" <interface name="org.freedesktop.Secret.Session">
  <method name="Close"/>
 </interface>
"#;

const PROMPT_XML: &str = r#" <interface name="org.freedesktop.Secret.Prompt">
  <method name="Prompt"><arg name="window-id" type="s" direction="in"/></method>
  <method name="Dismiss"/>
  <signal name="Completed"><arg name="dismissed" type="b"/><arg name="result" type="v"/></signal>
 </interface>
"#;
