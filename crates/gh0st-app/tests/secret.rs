//! `secret` against the real Secret Service, with no bus: calls go straight
//! to it, and the shell's questions are answered as the test says.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

use gh0st_app::secret::{Keyring, Lookup, Wire};
use gh0st_dbus::Message;
use gh0st_keyringd::secret::{Answer, Ask, Caller, Out, Service};
use gh0st_keyringd::vault::Cost;

struct World {
    svc: Service,
    signals: Vec<Message>,
    answer: Answer,
    serial: u32,
    dir: std::path::PathBuf,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

type Shared = Rc<RefCell<World>>;

fn world(name: &str) -> Shared {
    let dir = std::env::temp_dir().join(format!("gh0st-sdk-kr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut svc = Service::new(dir.join("login.vault"), Cost::TEST);
    svc.unlock(b"pw", true);
    Rc::new(RefCell::new(World {
        svc,
        signals: Vec::new(),
        answer: Answer::Once,
        serial: 0,
        dir,
    }))
}

struct Local {
    w: Shared,
    me: Caller,
}

fn app(w: &Shared, bus: &str, name: &str) -> Keyring {
    let me = Caller {
        bus: bus.into(),
        app: name.into(),
        label: name.into(),
    };
    Keyring::over(Some(Box::new(Local { w: w.clone(), me })))
}

impl Wire for Local {
    fn call(&mut self, mut m: Message) -> Result<Message, String> {
        let mut w = self.w.borrow_mut();
        w.serial += 1;
        m.serial = w.serial;
        m.sender = Some(self.me.bus.clone());
        let r = w.svc.call(&m, &self.me).ok_or("no reply")?;
        for _ in 0..8 {
            let outs = w.svc.drain();
            if outs.is_empty() {
                break;
            }
            for o in outs {
                match o {
                    Out::Ask(Ask::Password { id, .. }) => w.svc.password(id, b"pw"),
                    Out::Ask(Ask::Approve { id, .. }) => {
                        let a = w.answer;
                        w.svc.answer(id, a)
                    }
                    Out::Signal(s) => w.signals.push(s),
                    _ => {}
                }
            }
        }
        match r.error_name {
            Some(e) => Err(e),
            None => Ok(r),
        }
    }

    fn incoming(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.w.borrow_mut().signals)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }
}

const NAS: [(&str, &str); 2] = [("server", "nas"), ("user", "me")];

fn found(l: Lookup) -> Vec<u8> {
    match l {
        Lookup::Found(s) => s.as_bytes().to_vec(),
        other => panic!("not found: {other:?}"),
    }
}

#[test]
fn stores_and_finds_its_own_secret() {
    let w = world("own");
    let mut kr = app(&w, ":1.1", "files");
    assert!(matches!(kr.lookup(&NAS), Lookup::Missing));
    kr.store("nas", &NAS, b"one", true).unwrap();
    assert_eq!(found(kr.lookup(&NAS)), b"one");
    assert_eq!(
        found(kr.lookup(&[("server", "nas")])),
        b"one",
        "a subset matches"
    );
}

#[test]
fn replace_keeps_one_item() {
    let w = world("replace");
    let mut kr = app(&w, ":1.1", "files");
    kr.store("nas", &NAS, b"one", true).unwrap();
    kr.store("nas", &NAS, b"two", true).unwrap();
    let (open, shut) = kr.search(&NAS).unwrap();
    assert_eq!((open.len(), shut.len()), (1, 0));
    assert_eq!(found(kr.lookup(&NAS)), b"two");
}

#[test]
fn a_locked_keyring_opens_through_a_prompt() {
    let w = world("locked");
    let mut kr = app(&w, ":1.1", "files");
    kr.store("nas", &NAS, b"one", true).unwrap();
    w.borrow_mut().svc.lock();
    let Lookup::Locked(items) = kr.lookup(&NAS) else {
        panic!("locked")
    };
    let p = kr.unlock(&items).unwrap().expect("a prompt");
    assert_eq!(kr.ended(&p), Some(false), "the password opened it");
    assert_eq!(found(kr.lookup(&NAS)), b"one");
}

#[test]
fn a_vault_not_yet_opened_is_locked_not_missing() {
    let w = world("unopened");
    let mut kr = app(&w, ":1.1", "files");
    kr.store("nas", &NAS, b"one", true).unwrap();
    // The device starts again: the vault is on disk, not open.
    let vault = w.borrow().dir.join("login.vault");
    w.borrow_mut().svc = Service::new(vault, Cost::TEST);
    let mut kr = app(&w, ":1.2", "files");
    let Lookup::Locked(items) = kr.lookup(&NAS) else {
        panic!("locked, not missing")
    };
    assert!(items.is_empty());
    let p = kr.unlock(&items).unwrap().expect("a prompt");
    assert_eq!(kr.ended(&p), Some(false));
    assert_eq!(found(kr.lookup(&NAS)), b"one");
}

#[test]
fn another_apps_secret_takes_the_users_yes() {
    let w = world("other");
    let mut mail = app(&w, ":1.1", "mail");
    mail.store("nas", &NAS, b"theirs", false).unwrap();
    let mut kr = app(&w, ":1.2", "files");
    let Lookup::Locked(items) = kr.lookup(&NAS) else {
        panic!("locked to files")
    };
    w.borrow_mut().answer = Answer::Deny;
    let p = kr.unlock(&items).unwrap().expect("a prompt");
    assert_eq!(kr.ended(&p), Some(true), "dismissed");
    assert!(matches!(kr.lookup(&NAS), Lookup::Locked(_)));
    w.borrow_mut().answer = Answer::Once;
    let p = kr.unlock(&items).unwrap().expect("a prompt");
    assert_eq!(kr.ended(&p), Some(false));
    assert_eq!(found(kr.lookup(&NAS)), b"theirs");
}

#[test]
fn delete_asks_then_the_item_is_gone() {
    let w = world("delete");
    let mut kr = app(&w, ":1.1", "files");
    kr.store("nas", &NAS, b"one", true).unwrap();
    let (open, _) = kr.search(&NAS).unwrap();
    if let Some(p) = kr.delete(&open[0]).unwrap() {
        assert_eq!(kr.ended(&p), Some(false));
    }
    assert!(matches!(kr.lookup(&NAS), Lookup::Missing));
}

#[test]
fn no_keyring_is_absent() {
    let mut kr = Keyring::over(None);
    assert!(matches!(kr.lookup(&NAS), Lookup::Absent));
    assert!(kr.store("x", &NAS, b"x", true).is_err());
}
