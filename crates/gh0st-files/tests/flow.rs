//! Files driven by keys over a fixture home and the fake worker: what it
//! asks the worker, what it keeps in the keyring, and that it sleeps.

use gh0st_app::secret::{Keyring, Lookup, Wire};
use gh0st_app::testing::Driver;
use gh0st_dbus::Message;
use gh0st_files::app::{Files, Options};
use gh0st_files::engine::{Ask, Request};
use gh0st_files::fake::Fake;
use gh0st_files::model::{Entry, Loc};
use gh0st_files::servers::{self, Proto, Server};
use gh0st_keyringd::secret::{Answer, Ask as KrAsk, Caller, Out, Service};
use gh0st_keyringd::vault::Cost;
use std::cell::RefCell;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::rc::Rc;

struct Home {
    dir: PathBuf,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Home {
    fn new(name: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("gh0st-files-flow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let h = dir.join("home");
        std::fs::create_dir_all(h.join("Music")).unwrap();
        std::fs::write(h.join("Music/a.flac"), b"a").unwrap();
        std::fs::write(h.join("Music/b.flac"), b"b").unwrap();
        std::fs::write(h.join("Music/c.flac"), b"c").unwrap();
        let mut nas = Server::new(Proto::Sftp);
        nas.name = "nas".into();
        nas.host = "nas.local".into();
        nas.user = "me".into();
        nas.path = "/".into();
        servers::save(&dir.join("data/servers"), &[nas]).unwrap();
        Home { dir }
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }
}

fn fake() -> Fake {
    Fake::new().folder(
        "nas",
        "/",
        vec![Entry::dir("backup"), Entry::file("x.txt", 3)],
    )
}

fn drive(h: &Home, f: Fake, kr: Keyring) -> (Driver, Rc<RefCell<Vec<Request>>>) {
    let log = f.log.clone();
    let page = Files::with(Options {
        home: h.home(),
        data: Some(h.dir.join("data")),
        engine: Box::new(f),
        keyring: kr,
        pick: None,
    });
    let mut d = Driver::new(page);
    d.frame();
    (d, log)
}

/// Each key, then a frame, as the device draws after every event.
fn keys(d: &mut Driver, script: &str) {
    for line in script.lines() {
        d.keys(line);
        d.frame();
    }
}

// Places on side B: Home, Card, Recent, Trash, nas, Add server.
const B_TO_NAS: &str = "tab\nx\nx\nx\nx\nenter\ntab";

fn copies(log: &Rc<RefCell<Vec<Request>>>) -> Vec<(Vec<Loc>, Loc, bool)> {
    log.borrow()
        .iter()
        .filter_map(|r| match r {
            Request::Copy {
                items, to, remove, ..
            } => Some((items.clone(), to.clone(), *remove)),
            _ => None,
        })
        .collect()
}

#[test]
fn copy_then_paste_in_another_folder() {
    let h = Home::new("copy");
    let (mut d, log) = drive(&h, fake(), Keyring::over(None));
    // Paste waits, greyed, until something is copied.
    keys(&mut d, "6");
    assert!(copies(&log).is_empty());
    // Music, mark a (Space) and c (4), Copy on 5. Nothing moves yet.
    keys(&mut d, "enter\nspace\nx\n4\n5");
    assert_eq!(d.toast(), Some("Copied 2 items. Paste with 6"));
    assert!(copies(&log).is_empty());
    // Up, into the server on the same side, Paste on 6.
    keys(&mut d, "esc\nesc\nx\nx\nx\nx\nenter\n6");
    let music = Loc::Local(h.home().join("Music"));
    let nas = Loc::Remote {
        server: "nas".into(),
        path: "/".into(),
    };
    assert_eq!(
        copies(&log),
        [(
            vec![music.join("a.flac"), music.join("c.flac")],
            nas.clone(),
            false
        )]
    );
    assert_eq!(d.toast(), Some("Copied 2 items"));
    // A copy pastes again.
    keys(&mut d, "6");
    assert_eq!(copies(&log).len(), 2);
    assert!(d.lint().is_empty(), "{:?}", d.lint());
}

#[test]
fn cut_moves_once_and_not_into_its_own_folder() {
    let h = Home::new("cut");
    let (mut d, log) = drive(&h, fake(), Keyring::over(None));
    keys(&mut d, "enter\nx\n7\n6");
    assert_eq!(d.toast(), Some("It is in this folder already"));
    keys(&mut d, "esc\nesc\nx\nx\nx\nx\nenter\n6");
    let moved = copies(&log);
    assert_eq!(moved.len(), 1);
    assert!(moved[0].2, "a cut pastes as a move");
    keys(&mut d, "6");
    assert_eq!(copies(&log).len(), 1, "the clipboard is empty after");
}

#[test]
fn local_delete_goes_to_the_trash_and_remote_delete_is_held() {
    let h = Home::new("delete");
    let (mut d, log) = drive(&h, fake(), Keyring::over(None));
    keys(&mut d, "enter\ndelete\nenter");
    assert!(
        log.borrow()
            .iter()
            .any(|r| matches!(r, Request::Delete { trash: true, .. }))
    );
    keys(&mut d, B_TO_NAS);
    keys(&mut d, "tab\nx\ndelete");
    assert_eq!(d.dialog().map(|x| x.0), Some("Delete for good?"));
    keys(&mut d, "enter");
    assert!(d.dialog().is_some(), "Enter doesn't delete for good");
    d.keys("down 7").wait(800).keys("up 7");
    d.frame();
    assert!(
        log.borrow()
            .iter()
            .any(|r| matches!(r, Request::Delete { trash: false, .. }))
    );
}

#[test]
fn esc_climbs_to_places_and_tab_keeps_each_side() {
    let h = Home::new("nav");
    let (mut d, _) = drive(&h, fake(), Keyring::over(None));
    keys(&mut d, "enter");
    keys(&mut d, "esc");
    keys(&mut d, "esc");
    assert!(!d.quit());
    assert!(d.lint().is_empty());
    keys(&mut d, "tab\ntab\nenter");
    keys(&mut d, "x\nx\nx\nx\nx\nenter");
    assert_eq!(d.dialog(), None);
}

#[test]
fn an_idle_folder_never_wakes() {
    let h = Home::new("idle");
    let (mut d, _) = drive(&h, fake(), Keyring::over(None));
    keys(&mut d, "enter");
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "{wakes}: the clock's minute and nothing more");
}

// ---- the keyring ----

struct World {
    svc: Service,
    signals: Vec<Message>,
    serial: u32,
    dir: PathBuf,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Local {
    w: Rc<RefCell<World>>,
    me: Caller,
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
                    Out::Ask(KrAsk::Password { id, .. }) => w.svc.password(id, b"pw"),
                    Out::Ask(KrAsk::Approve { id, .. }) => w.svc.answer(id, Answer::Once),
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

fn files_kr(w: &Rc<RefCell<World>>, bus: &str) -> Keyring {
    let me = Caller {
        bus: bus.into(),
        app: "files".into(),
        label: "Files".into(),
    };
    Keyring::over(Some(Box::new(Local { w: w.clone(), me })))
}

fn attrs_of<'a>(a: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    a.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn keyring(name: &str) -> (Rc<RefCell<World>>, Keyring) {
    let dir = std::env::temp_dir().join(format!("gh0st-files-kr-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut svc = Service::new(dir.join("login.vault"), Cost::TEST);
    svc.unlock(b"pw", true);
    let w = Rc::new(RefCell::new(World {
        svc,
        signals: Vec::new(),
        serial: 0,
        dir,
    }));
    let me = Caller {
        bus: ":1.7".into(),
        app: "files".into(),
        label: "Files".into(),
    };
    let kr = Keyring::over(Some(Box::new(Local { w: w.clone(), me })));
    (w, kr)
}

fn nas_attrs() -> Vec<(&'static str, String)> {
    let mut nas = Server::new(Proto::Sftp);
    nas.host = "nas.local".into();
    nas.user = "me".into();
    nas.attrs().to_vec()
}

fn asks_password(f: &mut Fake) {
    f.first.insert(
        "nas".into(),
        vec![Ask::Password {
            server: "nas".into(),
            retry: false,
        }],
    );
}

fn replies(log: &Rc<RefCell<Vec<Request>>>) -> Vec<Option<String>> {
    log.borrow()
        .iter()
        .filter_map(|r| match r {
            Request::Reply { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_typed_password_is_kept_once_it_works() {
    let h = Home::new("typed");
    let (w, kr) = keyring("typed");
    let mut f = fake();
    asks_password(&mut f);
    let (mut d, log) = drive(&h, f, kr);
    keys(&mut d, "tab\nx\nx\nx\nx\nenter");
    assert_eq!(d.dialog().map(|x| x.0), Some("Password for me"));
    keys(&mut d, "type s3cret\nenter");
    assert_eq!(replies(&log), [Some("s3cret".to_string())]);
    // The listing worked: the password is in the keyring, not in a file.
    let (_, mut check) = (w.clone(), {
        let me = Caller {
            bus: ":1.8".into(),
            app: "files".into(),
            label: "Files".into(),
        };
        Keyring::over(Some(Box::new(Local { w: w.clone(), me })))
    });
    let a = nas_attrs();
    let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
    match check.lookup(&a) {
        Lookup::Found(s) => assert_eq!(s.as_bytes(), b"s3cret"),
        other => panic!("{other:?}"),
    }
    let file = std::fs::read_to_string(h.dir.join("data/servers")).unwrap();
    assert!(!file.contains("s3cret"));
}

#[test]
fn a_kept_password_connects_without_asking() {
    let h = Home::new("kept");
    let (_w, mut kr) = keyring("kept");
    let a = nas_attrs();
    let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
    kr.store("nas (SFTP)", &a, b"hunter2", true).unwrap();
    let mut f = fake();
    asks_password(&mut f);
    let (mut d, log) = drive(&h, f, kr);
    keys(&mut d, "tab\nx\nx\nx\nx\nenter");
    assert_eq!(d.dialog(), None, "no question");
    assert_eq!(replies(&log), [Some("hunter2".to_string())]);
}

#[test]
fn no_keyring_asks_every_time() {
    let h = Home::new("nokr");
    let mut f = fake();
    asks_password(&mut f);
    let (mut d, log) = drive(&h, f, Keyring::over(None));
    keys(&mut d, "tab\nx\nx\nx\nx\nenter");
    assert_eq!(d.dialog().map(|x| x.0), Some("Password for me"));
    keys(&mut d, "esc");
    assert_eq!(replies(&log), [None]);
}

#[test]
fn a_locked_keyring_opens_then_connects() {
    let h = Home::new("locked");
    let (w, mut kr) = keyring("locked");
    let a = nas_attrs();
    kr.store("nas (SFTP)", &attrs_of(&a), b"hunter2", true)
        .unwrap();
    w.borrow_mut().svc.lock();
    let mut f = fake();
    asks_password(&mut f);
    let (mut d, log) = drive(&h, f, files_kr(&w, ":1.9"));
    keys(&mut d, "tab\nx\nx\nx\nx\nenter");
    assert_eq!(
        d.dialog(),
        None,
        "the keyring's own prompt, then no question"
    );
    assert_eq!(replies(&log), [Some("hunter2".to_string())]);
}

#[test]
fn a_vault_not_opened_since_start_is_opened_not_skipped() {
    let h = Home::new("unopened");
    let (w, mut kr) = keyring("unopened");
    let a = nas_attrs();
    kr.store("nas (SFTP)", &attrs_of(&a), b"hunter2", true)
        .unwrap();
    let vault = w.borrow().dir.join("login.vault");
    w.borrow_mut().svc = Service::new(vault, Cost::TEST);
    let mut f = fake();
    asks_password(&mut f);
    let (mut d, log) = drive(&h, f, files_kr(&w, ":1.9"));
    keys(&mut d, "tab\nx\nx\nx\nx\nenter");
    assert_eq!(d.dialog(), None);
    assert_eq!(replies(&log), [Some("hunter2".to_string())]);
}

#[test]
fn only_the_server_that_listed_keeps_its_password() {
    let h = Home::new("two");
    let mut pi = Server::new(Proto::Sftp);
    pi.name = "pi".into();
    pi.host = "pi.local".into();
    pi.user = "pi".into();
    let mut nas = Server::new(Proto::Sftp);
    nas.name = "nas".into();
    nas.host = "nas.local".into();
    nas.user = "me".into();
    servers::save(&h.dir.join("data/servers"), &[nas, pi.clone()]).unwrap();
    let (w, kr) = keyring("two");
    let mut f = fake();
    f.first.insert(
        "pi".into(),
        vec![
            Ask::Password {
                server: "pi".into(),
                retry: false,
            },
            Ask::HostKey {
                server: "pi".into(),
                host: "pi.local".into(),
                fingerprint: "SHA256:abc".into(),
            },
        ],
    );
    let (mut d, _) = drive(&h, f, kr);
    // Places on B: Home, Card, Recent, Trash, nas, pi, Add server.
    keys(&mut d, "tab\nx\nx\nx\nx\nx\nenter\ntype pi-pw\nenter");
    assert_eq!(d.dialog().map(|x| x.0), Some("Trust pi.local?"));
    keys(&mut d, "esc\nesc\nf\nenter");
    let mut check = files_kr(&w, ":1.10");
    let a = pi.attrs();
    assert!(
        matches!(check.lookup(&attrs_of(&a)), Lookup::Missing),
        "pi never listed: its password isn't kept"
    );
}

/// The page with the real worker binary: Copy and Paste on the card.
#[test]
fn the_real_worker_copies_and_pastes() {
    let h = Home::new("real");
    std::fs::create_dir_all(h.home().join("Backup")).unwrap();
    let exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_gh0st-files"));
    let engine = gh0st_files::child::Child::with_exe(exe)
        .env("HOME", &h.home().to_string_lossy())
        .env("GH0ST_FILES_DATA", &h.dir.join("data").to_string_lossy());
    let page = Files::with(Options {
        home: h.home(),
        data: Some(h.dir.join("data")),
        engine: Box::new(engine),
        keyring: Keyring::over(None),
        pick: None,
    });
    let mut d = Driver::new(page);
    d.frame();
    // Music, b.flac, Copy; up, Backup, Paste: all on one side.
    keys(&mut d, "x\nenter\nx\n5\nesc\nf\nenter\n6");
    for _ in 0..50 {
        d.pump(100);
        if h.home().join("Backup/b.flac").exists() && d.toast().is_some() {
            break;
        }
    }
    assert_eq!(std::fs::read(h.home().join("Backup/b.flac")).unwrap(), b"b");
    assert_eq!(d.toast(), Some("Copied 1 item"));
    // Once done and the toast gone, nothing is left running.
    d.wait(3000);
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "{wakes}");
}

#[test]
fn dot_dot_heads_a_folder_and_goes_up() {
    let h = Home::new("dotdot");
    std::fs::create_dir_all(h.home().join("Music/empty")).unwrap();
    let (mut d, log) = drive(&h, fake(), Keyring::over(None));
    // Home opens on its first entry, Music; Up reaches "..".
    keys(&mut d, "enter\nf\nf");
    // In Music now, on "..": Enter goes back up, Music selected.
    keys(&mut d, "enter");
    keys(&mut d, "5");
    assert_eq!(
        d.toast(),
        Some("Copied 1 item. Paste with 6"),
        "Music is under the cursor again"
    );
    // An empty folder still has "..", and nothing to copy.
    keys(&mut d, "enter\nenter\nenter");
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    let _ = log;
}

// ---- a worker that answers late, as the real one does ----

/// Holds the fake's events until the test lets them go, and wakes the
/// page through a pipe as the real worker's socket would.
struct Late {
    inner: Fake,
    held: Vec<gh0st_files::engine::Event>,
    go: Rc<std::cell::Cell<bool>>,
    rx: std::os::fd::OwnedFd,
    tx: std::os::fd::OwnedFd,
}

impl Late {
    fn new(inner: Fake) -> (Late, Rc<std::cell::Cell<bool>>, std::os::fd::RawFd) {
        let mut fds = [0; 2];
        // SAFETY: pipe fills a two-int array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // SAFETY: both fds are fresh and owned from here.
        let (rx, tx) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        // SAFETY: fcntl on an fd we own.
        unsafe { libc::fcntl(rx.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        let go = Rc::new(std::cell::Cell::new(false));
        let w = tx.as_raw_fd();
        (
            Late {
                inner,
                held: Vec::new(),
                go: go.clone(),
                rx,
                tx,
            },
            go,
            w,
        )
    }
}

impl gh0st_files::engine::Engine for Late {
    fn send(&mut self, req: Request) {
        self.inner.send(req);
        self.held.extend(self.inner.events());
    }

    fn events(&mut self) -> Vec<gh0st_files::engine::Event> {
        use std::os::fd::AsRawFd;
        let mut b = [0u8; 64];
        // SAFETY: reads into a stack buffer of its own length.
        while unsafe { libc::read(self.rx.as_raw_fd(), b.as_mut_ptr().cast(), b.len()) } > 0 {}
        let _ = &self.tx;
        if self.go.get() {
            std::mem::take(&mut self.held)
        } else {
            Vec::new()
        }
    }

    fn fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        Some(self.rx.as_raw_fd())
    }
}

/// Lets the held events go, and wakes the page.
fn release(d: &mut Driver, go: &Rc<std::cell::Cell<bool>>, tx: std::os::fd::RawFd) {
    go.set(true);
    // SAFETY: writes one byte to our own pipe.
    unsafe { libc::write(tx, b"x".as_ptr().cast(), 1) };
    d.pump(50);
    d.frame();
    go.set(false);
}

#[test]
fn a_question_waits_for_the_users_own_dialog() {
    let h = Home::new("late-ask");
    let mut f = fake();
    f.first.insert(
        "nas".into(),
        vec![Ask::HostKey {
            server: "nas".into(),
            host: "nas.local".into(),
            fingerprint: "SHA256:abc".into(),
        }],
    );
    let log = f.log.clone();
    let (late, go, tx) = Late::new(f);
    let page = Files::with(Options {
        home: h.home(),
        data: Some(h.dir.join("data")),
        engine: Box::new(late),
        keyring: Keyring::over(None),
        pick: None,
    });
    let mut d = Driver::new(page);
    d.frame();
    // B starts connecting to the NAS; A opens a Trash dialog meanwhile.
    keys(&mut d, "tab\nx\nx\nx\nx\nenter\ntab\nenter\ndelete");
    assert_eq!(d.dialog().map(|x| x.0), Some("Move to the Trash?"));
    release(&mut d, &go, tx);
    assert_eq!(
        d.dialog().map(|x| x.0),
        Some("Move to the Trash?"),
        "the host key waits; Enter still means the Trash"
    );
    keys(&mut d, "esc");
    assert_eq!(d.dialog().map(|x| x.0), Some("Trust nas.local?"));
    assert!(replies(&log).is_empty());
}

#[test]
fn delete_acts_on_the_names_the_dialog_showed() {
    let h = Home::new("late-del");
    let f = fake();
    let log = f.log.clone();
    let (late, go, tx) = Late::new(f);
    let page = Files::with(Options {
        home: h.home(),
        data: Some(h.dir.join("data")),
        engine: Box::new(late),
        keyring: Keyring::over(None),
        pick: None,
    });
    let mut d = Driver::new(page);
    d.frame();
    // Music: new folder (its answer held), then b.flac and Delete.
    keys(&mut d, "enter\nk\ntype aaa\nenter\nx\ndelete");
    assert_eq!(d.dialog().map(|x| x.0), Some("Move to the Trash?"));
    // The folder is made and the list read again under the dialog.
    std::fs::create_dir_all(h.home().join("Music/aaa")).unwrap();
    release(&mut d, &go, tx);
    keys(&mut d, "enter");
    let deleted: Vec<Vec<Loc>> = log
        .borrow()
        .iter()
        .filter_map(|r| match r {
            Request::Delete { items, .. } => Some(items.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, [vec![Loc::Local(h.home().join("Music/b.flac"))]]);
}

#[test]
fn a_folder_opened_from_another_app_leaves_a_clash_waiting() {
    let h = Home::new("clash-open");
    let mut f = fake();
    f.copy.conflict = Some("a.flac".into());
    let (mut d, log) = drive(&h, f, Keyring::over(None));
    // Music: copy a.flac, paste it back into Music.
    keys(&mut d, "enter\n5\n6");
    assert_eq!(d.title(), "Already there");
    assert!(d.awake());
    // Another app opens a folder here meanwhile.
    d.open(&[h.home().as_path()]);
    d.frame();
    assert_eq!(d.title(), "Already there", "the question stays");
    keys(&mut d, "enter");
    assert_eq!(replies(&log), [Some("both".to_string())]);
    assert!(!d.awake(), "the copy ended");
}
