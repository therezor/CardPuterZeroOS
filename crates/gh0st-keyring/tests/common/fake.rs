//! The real Secret Service behind the app, with no bus: calls go straight
//! to `Service`, and the shell's questions are answered as the test says.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::rc::Rc;

use gh0st_dbus::{Message, Value};
use gh0st_keyring::backend::{AccessRow, Backend, ItemRow, Shared, State};
use gh0st_keyringd::secret::{self, Answer, Ask, Caller, Out, Service};
use gh0st_keyringd::vault::Cost;

pub struct Fake {
    pub svc: Service,
    me: Caller,
    dir: PathBuf,
    serial: u32,
    done: Vec<(String, bool)>,
    /// How the shell answers approvals; a password prompt gets `password`.
    pub answer: Answer,
    pub password: Vec<u8>,
    session: Option<String>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn caller(bus: &str, app: &str) -> Caller {
    Caller {
        bus: bus.into(),
        app: app.into(),
        label: app.into(),
    }
}

impl Fake {
    pub fn new(name: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("gh0st-kr-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc = Service::new(dir.join("login.vault"), Cost::TEST);
        svc.unlock(b"pw", true);
        Fake {
            svc,
            me: caller(":1.9", "keyring"),
            dir,
            serial: 0,
            done: Vec::new(),
            answer: Answer::Once,
            password: b"pw".to_vec(),
            session: None,
        }
    }

    pub fn shared(self) -> Shared {
        Rc::new(RefCell::new(Box::new(self)))
    }

    fn call_as(
        &mut self,
        c: &Caller,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, String> {
        self.serial += 1;
        let mut m = Message::call(secret::NAME, path, iface, member);
        m.body = args;
        m.serial = self.serial;
        m.sender = Some(c.bus.clone());
        let r = self.svc.call(&m, c).ok_or("no reply")?;
        self.pump();
        match r.error_name {
            Some(e) => Err(e),
            None => Ok(r.body),
        }
    }

    fn call(
        &mut self,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, String> {
        let me = self.me.clone();
        self.call_as(&me, path, iface, member, args)
    }

    /// Answers what the service asks, as the shell would.
    fn pump(&mut self) {
        for _ in 0..8 {
            let outs = self.svc.drain();
            if outs.is_empty() {
                return;
            }
            for o in outs {
                match o {
                    Out::Ask(Ask::Password { id, .. }) => {
                        let pw = self.password.clone();
                        self.svc.password(id, &pw);
                    }
                    Out::Ask(Ask::Approve { id, .. }) => self.svc.answer(id, self.answer),
                    Out::Signal(s) if s.is(secret::PROMPT, "Completed") => {
                        let d = s.body[0].as_bool().unwrap_or(true);
                        self.done.push((s.path.clone().unwrap_or_default(), d));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Another app stores a secret.
    pub fn seed(
        &mut self,
        app: &str,
        label: &str,
        attrs: &[(&str, &str)],
        secret: &[u8],
    ) -> String {
        let c = caller(&format!(":2.{}", self.serial + 1), app);
        let s = self
            .call_as(
                &c,
                secret::ROOT,
                secret::SVC,
                "OpenSession",
                vec![Value::str("plain"), Value::variant(Value::str(""))],
            )
            .unwrap()[1]
            .as_str()
            .unwrap()
            .to_string();
        let props = Value::props([
            ("org.freedesktop.Secret.Item.Label", Value::str(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::str_dict(attrs.iter().copied()),
            ),
        ]);
        let sec = Value::Struct(vec![
            Value::path(s),
            Value::Bytes(vec![]),
            Value::Bytes(secret.to_vec()),
            Value::str("text/plain"),
        ]);
        let path = "/org/freedesktop/secrets/aliases/default";
        self.call_as(
            &c,
            path,
            secret::COLL,
            "CreateItem",
            vec![props, sec, Value::Bool(false)],
        )
        .unwrap()[0]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// `app` may read `item` for good.
    pub fn approve(&mut self, app: &str, item: &str) {
        let c = caller(":3.1", app);
        let p = self
            .call_as(
                &c,
                secret::ROOT,
                secret::SVC,
                "Unlock",
                vec![Value::paths([item.to_string()])],
            )
            .unwrap()[1]
            .as_str()
            .unwrap()
            .to_string();
        let keep = self.answer;
        self.answer = Answer::Always;
        self.call_as(&c, &p, secret::PROMPT, "Prompt", vec![Value::str("")])
            .unwrap();
        self.answer = keep;
    }

    fn prompt(&mut self, v: Option<&Value>) -> Result<Option<String>, String> {
        let p = v.and_then(Value::as_str).unwrap_or("/").to_string();
        if p == "/" {
            return Ok(None);
        }
        self.call(&p, secret::PROMPT, "Prompt", vec![Value::str("")])?;
        Ok(Some(p))
    }
}

impl Backend for Fake {
    fn state(&mut self) -> State {
        if self.svc.unlocked() {
            State::Open
        } else {
            State::Locked
        }
    }

    fn items(&mut self) -> Vec<ItemRow> {
        let b = self
            .call(secret::ROOT, secret::GH0ST, "Items", vec![])
            .unwrap_or_default();
        b.first()
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| {
                let f = r.as_struct()?;
                Some(ItemRow {
                    path: f[0].as_str()?.into(),
                    label: f[1].as_str()?.into(),
                    app: f[2].as_str()?.into(),
                    schema: f[3].as_str()?.into(),
                })
            })
            .collect()
    }

    fn access(&mut self) -> Vec<AccessRow> {
        let b = self
            .call(secret::ROOT, secret::GH0ST, "Access", vec![])
            .unwrap_or_default();
        b.first()
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| {
                let f = r.as_struct()?;
                Some(AccessRow {
                    app: f[0].as_str()?.into(),
                    item: f[1].as_str()?.into(),
                    label: f[2].as_str()?.into(),
                })
            })
            .collect()
    }

    fn revoke(&mut self, app: &str, item: &str) -> bool {
        self.call(
            secret::ROOT,
            secret::GH0ST,
            "Revoke",
            vec![Value::str(app), Value::path(item)],
        )
        .is_ok()
    }

    fn lock(&mut self) {
        self.svc.lock();
    }

    fn unlock(&mut self) -> Result<Option<String>, String> {
        let b = self.call(
            secret::ROOT,
            secret::SVC,
            "Unlock",
            vec![Value::paths(["/org/freedesktop/secrets/collection/login"])],
        )?;
        self.prompt(b.get(1))
    }

    fn reveal(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.call(
            secret::ROOT,
            secret::GH0ST,
            "Reveal",
            vec![Value::path(item)],
        )?;
        self.prompt(b.first())
    }

    fn delete(&mut self, item: &str) -> Result<Option<String>, String> {
        let b = self.call(item, secret::ITEM, "Delete", vec![])?;
        self.prompt(b.first())
    }

    fn secret(&mut self, item: &str) -> Option<Vec<u8>> {
        let s = match &self.session {
            Some(s) => s.clone(),
            None => {
                let b = self
                    .call(
                        secret::ROOT,
                        secret::SVC,
                        "OpenSession",
                        vec![Value::str("plain"), Value::variant(Value::str(""))],
                    )
                    .ok()?;
                let s = b[1].as_str()?.to_string();
                self.session = Some(s.clone());
                s
            }
        };
        let b = self
            .call(item, secret::ITEM, "GetSecret", vec![Value::path(s)])
            .ok()?;
        Some(b[0].as_struct()?[2].as_bytes()?.to_vec())
    }

    fn store(&mut self, label: &str, attrs: &[(&str, &str)], secret: &[u8]) -> Result<(), String> {
        let me = self.me.app.clone();
        let _ = me;
        let s = self.call(
            secret::ROOT,
            secret::SVC,
            "OpenSession",
            vec![Value::str("plain"), Value::variant(Value::str(""))],
        )?[1]
            .as_str()
            .unwrap_or("/")
            .to_string();
        let props = Value::props([
            ("org.freedesktop.Secret.Item.Label", Value::str(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::str_dict(attrs.iter().copied()),
            ),
        ]);
        let sec = Value::Struct(vec![
            Value::path(s),
            Value::Bytes(vec![]),
            Value::Bytes(secret.to_vec()),
            Value::str("text/plain"),
        ]);
        self.call(
            "/org/freedesktop/secrets/aliases/default",
            secret::COLL,
            "CreateItem",
            vec![props, sec, Value::Bool(false)],
        )
        .map(drop)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }

    fn ended(&mut self, path: &str) -> Option<bool> {
        let i = self.done.iter().position(|(p, _)| p == path)?;
        Some(self.done.remove(i).1)
    }

    fn changed(&mut self) -> bool {
        false
    }
}
