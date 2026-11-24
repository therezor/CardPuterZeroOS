//! The Secret Service state machine, driven the way libsecret drives it.

use std::path::PathBuf;

use gh0st_dbus::{Message, Value};
use gh0st_keyringd::secret::*;
use gh0st_keyringd::vault::Cost;

struct T {
    svc: Service,
    dir: PathBuf,
    serial: u32,
}

impl Drop for T {
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

impl T {
    fn new(name: &str) -> T {
        let dir = std::env::temp_dir().join(format!("gh0st-kr-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let svc = Service::new(dir.join("login.vault"), Cost::TEST);
        T {
            svc,
            dir,
            serial: 0,
        }
    }

    fn call(
        &mut self,
        c: &Caller,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Message {
        self.serial += 1;
        let mut m = Message::call(NAME, path, iface, member);
        m.body = args;
        m.serial = self.serial;
        m.sender = Some(c.bus.clone());
        self.svc.call(&m, c).unwrap()
    }

    fn ok(
        &mut self,
        c: &Caller,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> Vec<Value> {
        let r = self.call(c, path, iface, member, args);
        assert_eq!(r.error_name, None, "{member}: {}", r.error_text());
        r.body
    }

    fn session(&mut self, c: &Caller) -> String {
        let b = self.ok(
            c,
            ROOT,
            SVC,
            "OpenSession",
            vec![Value::str("plain"), Value::variant(Value::str(""))],
        );
        b[1].as_str().unwrap().to_string()
    }

    fn store(&mut self, c: &Caller, label: &str, attrs: &[(&str, &str)], secret: &[u8]) -> String {
        let s = self.session(c);
        let coll = self.ok(c, ROOT, SVC, "ReadAlias", vec![Value::str("default")])[0]
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
        let secret = Value::Struct(vec![
            Value::path(s),
            Value::Bytes(vec![]),
            Value::Bytes(secret.to_vec()),
            Value::str("text/plain"),
        ]);
        let b = self.ok(
            c,
            &coll,
            COLL,
            "CreateItem",
            vec![props, secret, Value::Bool(true)],
        );
        b[0].as_str().unwrap().to_string()
    }

    fn search(&mut self, c: &Caller, attrs: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
        let b = self.ok(
            c,
            ROOT,
            SVC,
            "SearchItems",
            vec![Value::str_dict(attrs.iter().copied())],
        );
        let list = |v: &Value| {
            v.as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_string())
                .collect()
        };
        (list(&b[0]), list(&b[1]))
    }

    fn get(&mut self, c: &Caller, item: &str) -> Result<Vec<u8>, String> {
        let s = self.session(c);
        let r = self.call(c, item, ITEM, "GetSecret", vec![Value::path(s)]);
        match r.error_name {
            Some(e) => Err(e),
            None => Ok(r.body[0].as_struct().unwrap()[2]
                .as_bytes()
                .unwrap()
                .to_vec()),
        }
    }

    fn asks(&mut self) -> Vec<Ask> {
        self.svc
            .drain()
            .into_iter()
            .filter_map(|o| match o {
                Out::Ask(a) => Some(a),
                Out::Signal(_) | Out::Stale => None,
            })
            .collect()
    }

    fn completed(&mut self) -> Option<(bool, Value)> {
        self.svc.drain().into_iter().find_map(|o| match o {
            Out::Signal(s) if s.is(PROMPT, "Completed") => {
                Some((s.body[0].as_bool().unwrap(), s.body[1].inner().clone()))
            }
            _ => None,
        })
    }
}

#[test]
fn an_app_stores_and_reads_its_own_secret() {
    let mut t = T::new("own");
    assert_eq!(t.svc.unlock(b"pw", true), Unlock::Created);
    let git = caller(":1.1", "git");
    let item = t.store(&git, "github", &[("service", "github.com")], b"hunter2");
    assert_eq!(
        t.search(&git, &[("service", "github.com")]),
        (vec![item.clone()], vec![])
    );
    assert_eq!(t.get(&git, &item).unwrap(), b"hunter2");
}

#[test]
fn another_app_sees_it_locked_until_approved() {
    let mut t = T::new("approve");
    t.svc.unlock(b"pw", true);
    let git = caller(":1.1", "git");
    let item = t.store(&git, "github", &[("service", "github.com")], b"hunter2");
    let evil = caller(":1.2", "evil");
    assert_eq!(
        t.search(&evil, &[("service", "github.com")]),
        (vec![], vec![item.clone()])
    );
    assert_eq!(t.get(&evil, &item).unwrap_err(), IS_LOCKED);
    // libsecret's flow: Unlock gives a prompt, Prompt asks the shell.
    let b = t.ok(
        &evil,
        ROOT,
        SVC,
        "Unlock",
        vec![Value::paths([item.clone()])],
    );
    assert!(b[0].as_array().unwrap().is_empty());
    let prompt = b[1].as_str().unwrap().to_string();
    t.ok(&evil, &prompt, PROMPT, "Prompt", vec![Value::str("")]);
    let asks = t.asks();
    let Some(Ask::Approve { id, app, what }) = asks.first().cloned() else {
        panic!("{asks:?}")
    };
    assert_eq!(
        (app.as_str(), what.as_str()),
        ("evil", "read \u{201c}github\u{201d}")
    );
    t.svc.answer(id, Answer::Deny);
    assert!(t.completed().unwrap().0);
    assert_eq!(t.get(&evil, &item).unwrap_err(), IS_LOCKED);
    // Once: this connection only.
    let prompt = t.ok(
        &evil,
        ROOT,
        SVC,
        "Unlock",
        vec![Value::paths([item.clone()])],
    )[1]
    .as_str()
    .unwrap()
    .to_string();
    t.ok(&evil, &prompt, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Approve { id, .. }) = t.asks().first().cloned() else {
        panic!()
    };
    t.svc.answer(id, Answer::Once);
    let (dismissed, result) = t.completed().unwrap();
    assert!(!dismissed);
    assert_eq!(result.as_array().unwrap()[0].as_str(), Some(item.as_str()));
    assert_eq!(t.get(&evil, &item).unwrap(), b"hunter2");
    assert_eq!(
        t.get(&caller(":1.3", "evil"), &item).unwrap_err(),
        IS_LOCKED
    );
    t.svc.gone(":1.2");
    assert_eq!(
        t.get(&caller(":1.2", "evil"), &item).unwrap_err(),
        IS_LOCKED
    );
}

#[test]
fn always_is_kept_in_the_vault() {
    let mut t = T::new("always");
    t.svc.unlock(b"pw", true);
    let item = t.store(&caller(":1.1", "git"), "github", &[("k", "v")], b"x");
    let other = caller(":1.2", "ide");
    let prompt = t.ok(
        &other,
        ROOT,
        SVC,
        "Unlock",
        vec![Value::paths([item.clone()])],
    )[1]
    .as_str()
    .unwrap()
    .to_string();
    t.ok(&other, &prompt, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Approve { id, .. }) = t.asks().first().cloned() else {
        panic!()
    };
    t.svc.answer(id, Answer::Always);
    let mut again = Service::new(t.dir.join("login.vault"), Cost::TEST);
    assert_eq!(again.unlock(b"pw", true), Unlock::Opened);
    std::mem::swap(&mut t.svc, &mut again);
    assert_eq!(t.get(&caller(":1.9", "ide"), &item).unwrap(), b"x");
}

#[test]
fn locked_means_locked_and_unlock_asks_the_password() {
    let mut t = T::new("lock");
    t.svc.unlock(b"pw", true);
    let git = caller(":1.1", "git");
    let item = t.store(&git, "github", &[("service", "gh")], b"hunter2");
    t.svc.lock();
    assert!(!t.svc.unlocked());
    // Attributes stay in memory: the search still finds it, locked.
    assert_eq!(
        t.search(&git, &[("service", "gh")]),
        (vec![], vec![item.clone()])
    );
    assert_eq!(t.get(&git, &item).unwrap_err(), IS_LOCKED);
    let prompt = t.ok(
        &git,
        ROOT,
        SVC,
        "Unlock",
        vec![Value::paths([item.clone()])],
    )[1]
    .as_str()
    .unwrap()
    .to_string();
    t.ok(&git, &prompt, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Password {
        id, retry: false, ..
    }) = t.asks().first().cloned()
    else {
        panic!()
    };
    // A typo the shell let through would be refused there; here the
    // service sees it unchecked.
    t.svc.verified_prompts = false;
    t.svc.password(id, b"nope");
    assert!(matches!(
        t.asks().first(),
        Some(Ask::Password { retry: true, .. })
    ));
    t.svc.password(id, b"pw");
    let (dismissed, _) = t.completed().unwrap();
    assert!(!dismissed);
    assert_eq!(t.get(&git, &item).unwrap(), b"hunter2");
}

#[test]
fn a_device_unlock_moves_waiting_prompts_on() {
    let mut t = T::new("wait");
    t.svc.unlock(b"pw", true);
    let git = caller(":1.1", "git");
    let item = t.store(&git, "github", &[], b"x");
    t.svc.lock();
    let prompt = t.ok(&git, ROOT, SVC, "Unlock", vec![Value::paths([item])])[1]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(&git, &prompt, PROMPT, "Prompt", vec![Value::str("")]);
    t.asks();
    assert_eq!(t.svc.unlock(b"pw", true), Unlock::Opened);
    assert!(!t.completed().unwrap().0);
}

#[test]
fn wifi_secrets_survive_a_lock() {
    let mut t = T::new("wifi");
    t.svc.unlock(b"pw", true);
    let nm = caller(":1.1", "networkmanager");
    t.store(
        &nm,
        "Home",
        &[("xdg:schema", NM_SCHEMA), ("setting-key", "psk")],
        b"wifipass",
    );
    t.store(&nm, "Other", &[("service", "x")], b"other");
    t.svc.lock();
    let db = t.svc.db().unwrap();
    let items = &db.collections[0].items;
    assert!(
        items
            .iter()
            .any(|i| i.label == "Home" && i.secret.is_some())
    );
    assert!(
        items
            .iter()
            .any(|i| i.label == "Other" && i.secret.is_none())
    );
}

#[test]
fn replacing_another_apps_secret_is_refused() {
    let mut t = T::new("replace");
    t.svc.unlock(b"pw", true);
    t.store(
        &caller(":1.1", "git"),
        "github",
        &[("service", "gh")],
        b"real",
    );
    let evil = caller(":1.2", "evil");
    let s = t.session(&evil);
    let r = t.call(
        &evil,
        "/org/freedesktop/secrets/aliases/default",
        COLL,
        "CreateItem",
        vec![
            Value::props([(
                "org.freedesktop.Secret.Item.Attributes",
                Value::str_dict([("service", "gh")]),
            )]),
            Value::Struct(vec![
                Value::path(s),
                Value::Bytes(vec![]),
                Value::Bytes(b"fake".to_vec()),
                Value::str("text/plain"),
            ]),
            Value::Bool(true),
        ],
    );
    assert_eq!(
        r.error_name.as_deref(),
        Some("org.freedesktop.DBus.Error.AccessDenied")
    );
}

#[test]
fn only_plain_sessions_and_sessions_are_per_connection() {
    let mut t = T::new("session");
    t.svc.unlock(b"pw", true);
    let a = caller(":1.1", "a");
    let r = t.call(
        &a,
        ROOT,
        SVC,
        "OpenSession",
        vec![
            Value::str("dh-ietf1024-sha256-aes128-cbc-pkcs7"),
            Value::variant(Value::Bytes(vec![1])),
        ],
    );
    assert_eq!(
        r.error_name.as_deref(),
        Some("org.freedesktop.DBus.Error.NotSupported")
    );
    let item = t.store(&a, "x", &[], b"x");
    let s = t.session(&a);
    let r = t.call(
        &caller(":1.2", "a"),
        &item,
        ITEM,
        "GetSecret",
        vec![Value::path(s)],
    );
    assert_eq!(r.error_name.as_deref(), Some(NO_SESSION));
}

#[test]
fn delete_goes_through_a_prompt() {
    let mut t = T::new("delete");
    t.svc.unlock(b"pw", true);
    let a = caller(":1.1", "a");
    let item = t.store(&a, "x", &[("k", "v")], b"x");
    let p = t.ok(&a, &item, ITEM, "Delete", vec![])[0]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(&a, &p, PROMPT, "Prompt", vec![Value::str("")]);
    assert!(!t.completed().unwrap().0);
    assert_eq!(t.search(&a, &[("k", "v")]), (vec![], vec![]));
}

#[test]
fn properties_and_aliases() {
    let mut t = T::new("props");
    t.svc.unlock(b"pw", true);
    let a = caller(":1.1", "a");
    let item = t.store(&a, "label", &[("k", "v")], b"x");
    let all = t.ok(
        &a,
        &item,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        vec![Value::str(ITEM)],
    );
    assert_eq!(
        all[0]
            .get("Label")
            .map(Value::inner)
            .and_then(Value::as_str),
        Some("label")
    );
    assert_eq!(
        all[0]
            .get("Locked")
            .map(Value::inner)
            .and_then(Value::as_bool),
        Some(false)
    );
    t.ok(
        &a,
        &item,
        "org.freedesktop.DBus.Properties",
        "Set",
        vec![
            Value::str(ITEM),
            Value::str("Label"),
            Value::variant(Value::str("new")),
        ],
    );
    let got = t.ok(
        &a,
        &item,
        "org.freedesktop.DBus.Properties",
        "Get",
        vec![Value::str(ITEM), Value::str("Label")],
    );
    assert_eq!(got[0].inner().as_str(), Some("new"));
    let colls = t.ok(
        &a,
        ROOT,
        "org.freedesktop.DBus.Properties",
        "Get",
        vec![Value::str(SVC), Value::str("Collections")],
    );
    assert_eq!(colls[0].inner().as_array().unwrap().len(), 1);
    let xml = t.ok(
        &a,
        ROOT,
        "org.freedesktop.DBus.Introspectable",
        "Introspect",
        vec![],
    );
    assert!(
        xml[0]
            .as_str()
            .unwrap()
            .contains("org.freedesktop.Secret.Service")
    );
}

#[test]
fn only_a_verified_password_makes_a_vault() {
    let mut t = T::new("verified");
    assert_eq!(t.svc.unlock(b"typo", false), Unlock::Wrong);
    assert!(!t.dir.join("login.vault").exists());
    assert_eq!(t.svc.unlock(b"pw", true), Unlock::Created);
}

#[test]
fn a_checked_prompt_password_the_vault_refuses_is_stale() {
    let mut t = T::new("stale");
    t.svc.unlock(b"old", true);
    let a = caller(":1.1", "a");
    let item = t.store(&a, "x", &[], b"x");
    t.svc.lock();
    let p = t.ok(&a, ROOT, SVC, "Unlock", vec![Value::paths([item.clone()])])[1]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(&a, &p, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Password { id, .. }) = t.asks().first().cloned() else {
        panic!()
    };
    // passwd ran in a terminal: the account says "new", the vault "old".
    t.svc.password(id, b"new");
    assert!(t.svc.drain().iter().any(|o| matches!(o, Out::Stale)));
    assert_eq!(t.svc.rewrap_from_old(b"old"), Unlock::Opened);
    assert!(!t.completed().unwrap().0, "the waiting prompt went on");
    t.svc.lock();
    assert_eq!(t.svc.unlock(b"new", true), Unlock::Opened);
}

#[test]
fn password_changes() {
    let mut t = T::new("passwd");
    t.svc.unlock(b"old", true);
    t.store(&caller(":1.1", "a"), "x", &[], b"x");
    // Settings: prepare, passwd fails, abort.
    assert!(!t.svc.prepare(b"wrong", b"new"));
    assert!(t.svc.prepare(b"old", b"new"));
    assert!(t.svc.commit(false));
    t.svc.lock();
    assert_eq!(t.svc.unlock(b"new", true), Unlock::Wrong);
    assert_eq!(t.svc.unlock(b"old", true), Unlock::Opened);
    // Prepared, then the device died before commit: the next unlock with
    // the new password promotes vault.next.
    assert!(t.svc.prepare(b"old", b"new"));
    t.svc.lock();
    assert_eq!(t.svc.unlock(b"new", true), Unlock::Opened);
    t.svc.lock();
    assert_eq!(t.svc.unlock(b"old", true), Unlock::Wrong);
    // passwd in a terminal: the shell holds the new one, asks the old.
    t.svc.lock();
    let mut fresh = Service::new(t.dir.join("login.vault"), Cost::TEST);
    assert_eq!(fresh.unlock(b"newer", true), Unlock::Wrong);
    fresh.hold_stale(b"newer");
    assert_eq!(fresh.rewrap_from_old(b"wrong"), Unlock::Wrong);
    assert_eq!(fresh.rewrap_from_old(b"new"), Unlock::Opened);
    fresh.lock();
    assert_eq!(fresh.unlock(b"newer", true), Unlock::Opened);
    assert_eq!(fresh.db().unwrap().collections[0].items.len(), 1);
}

#[test]
fn the_keyring_app_lists_names_and_reveals_after_the_password() {
    let mut t = T::new("gh0st-iface");
    t.svc.unlock(b"pw", true);
    let git = caller(":1.1", "git");
    let item = t.store(
        &git,
        "GitHub",
        &[("xdg:schema", "org.git"), ("k", "v")],
        b"tok",
    );
    let app = caller(":1.2", "keyring");
    let rows = t.ok(&app, ROOT, GH0ST, "Items", vec![]);
    let row = rows[0].as_array().unwrap()[0].as_struct().unwrap().to_vec();
    assert_eq!(row[0].as_str(), Some(item.as_str()));
    assert_eq!(
        (row[1].as_str(), row[2].as_str(), row[3].as_str()),
        (Some("GitHub"), Some("git"), Some("org.git"))
    );
    // Reveal: the password, even with the vault open, then the secret.
    assert_eq!(t.get(&app, &item).unwrap_err(), IS_LOCKED);
    let p = t.ok(&app, ROOT, GH0ST, "Reveal", vec![Value::path(item.clone())])[0]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(&app, &p, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Password { id, reason, .. }) = t.asks().first().cloned() else {
        panic!()
    };
    assert_eq!(reason, "keyring wants to show \u{201c}GitHub\u{201d}");
    t.svc.password(id, b"pw");
    assert!(!t.completed().unwrap().0);
    assert_eq!(t.get(&app, &item).unwrap(), b"tok");
    // Access and revoke.
    let other = caller(":1.3", "ide");
    let p = t.ok(
        &other,
        ROOT,
        SVC,
        "Unlock",
        vec![Value::paths([item.clone()])],
    )[1]
    .as_str()
    .unwrap()
    .to_string();
    t.ok(&other, &p, PROMPT, "Prompt", vec![Value::str("")]);
    let Some(Ask::Approve { id, .. }) = t.asks().first().cloned() else {
        panic!()
    };
    t.svc.answer(id, Answer::Always);
    let access = t.ok(&app, ROOT, GH0ST, "Access", vec![]);
    let row = access[0].as_array().unwrap()[0]
        .as_struct()
        .unwrap()
        .to_vec();
    assert_eq!(
        (row[0].as_str(), row[2].as_str()),
        (Some("ide"), Some("GitHub"))
    );
    t.ok(
        &app,
        ROOT,
        GH0ST,
        "Revoke",
        vec![Value::str("ide"), Value::path(item.clone())],
    );
    assert!(
        t.ok(&app, ROOT, GH0ST, "Access", vec![])[0]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(t.get(&caller(":1.4", "ide"), &item).unwrap_err(), IS_LOCKED);
}
