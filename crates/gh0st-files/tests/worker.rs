//! The real worker binary, driven as the page drives it: local copies and
//! moves, conflicts, the Trash, archives through bsdtar, and SFTP through
//! the system's sftp-server with no network. Tests whose tool is missing
//! say so and pass.

use gh0st_files::child::Child;
use gh0st_files::engine::{Ask, Engine, Event, Id, Request};
use gh0st_files::model::{Kind, Loc};
use gh0st_files::servers::{Proto, Server};
use std::path::{Path, PathBuf};

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("gh0st-files-w-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

struct W {
    c: Child,
    next: Id,
}

impl W {
    fn new(t: &Tmp, extra: &[(&str, &str)]) -> W {
        // GH0ST_FILES_EXE runs these where cargo didn't build them: the device.
        let exe = std::env::var_os("GH0ST_FILES_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_gh0st-files")));
        let mut c = Child::with_exe(exe)
            .env("HOME", &t.0.join("home").to_string_lossy())
            .env("GH0ST_FILES_DATA", &t.0.join("data").to_string_lossy());
        for (k, v) in extra {
            c = c.env(k, v);
        }
        std::fs::create_dir_all(t.0.join("home")).unwrap();
        W { c, next: 0 }
    }

    fn id(&mut self) -> Id {
        self.next += 1;
        self.next
    }

    /// Sends `req` and collects its events until it ends, answering each
    /// question with `answer`.
    fn run(
        &mut self,
        req: Request,
        answer: &mut dyn FnMut(&Ask) -> Option<String>,
    ) -> (Vec<Event>, Result<(), String>) {
        let id = req.id().expect("a request with an id");
        self.c.send(req);
        let mut seen = Vec::new();
        let start = std::time::Instant::now();
        loop {
            assert!(start.elapsed().as_secs() < 30, "no end: {seen:?}");
            for e in self.c.events() {
                if e.id() != id {
                    continue;
                }
                match &e {
                    Event::Ask { ask, .. } => {
                        let text = answer(ask);
                        self.c.send(Request::Reply { id, text });
                    }
                    Event::Done { .. } | Event::Listed { .. } => {
                        seen.push(e);
                        return (seen, Ok(()));
                    }
                    Event::Error { text, .. } => {
                        let t = text.clone();
                        seen.push(e);
                        return (seen, Err(t));
                    }
                    _ => {}
                }
                seen.push(e);
            }
            if let Some(fd) = self.c.fd() {
                let mut p = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: polls one live pollfd.
                unsafe { libc::poll(&mut p, 1, 1000) };
            }
        }
    }

    fn ok(&mut self, req: Request) -> Vec<Event> {
        let (ev, r) = self.run(req, &mut |a| panic!("asked {a:?}"));
        r.unwrap_or_else(|e| panic!("{e}"));
        ev
    }
}

fn names(ev: &[Event]) -> Vec<String> {
    let mut v: Vec<String> = ev
        .iter()
        .flat_map(|e| match e {
            Event::Entries { entries, .. } => entries.iter().map(|x| x.name.clone()).collect(),
            _ => Vec::new(),
        })
        .collect();
    v.sort();
    v
}

fn local(p: &Path) -> Loc {
    Loc::Local(p.to_path_buf())
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn copies_moves_and_conflicts_on_the_card() {
    let t = tmp("local");
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    write(&a.join("docs/x.txt"), "x1");
    write(&a.join("docs/deep/y.txt"), "y1");
    write(&a.join("n.txt"), "n1");
    std::fs::create_dir_all(&b).unwrap();
    let mut w = W::new(&t, &[]);

    let id = w.id();
    let ev = w.ok(Request::Copy {
        id,
        items: vec![local(&a.join("docs")), local(&a.join("n.txt"))],
        to: local(&b),
        remove: false,
    });
    assert_eq!(read(&b.join("docs/deep/y.txt")), "y1");
    assert_eq!(read(&b.join("n.txt")), "n1");
    let last = ev.iter().rev().find_map(|e| match e {
        Event::Progress { done, total, .. } => Some((*done, *total)),
        _ => None,
    });
    assert_eq!(last.map(|x| x.1), Some(6), "two-byte files, three of them");

    // Again: each name is there, so it asks. Keep both, then skip.
    write(&a.join("n.txt"), "n2");
    let id = w.id();
    let mut asked = Vec::new();
    let (_, r) = w.run(
        Request::Copy {
            id,
            items: vec![local(&a.join("n.txt"))],
            to: local(&b),
            remove: false,
        },
        &mut |q| {
            asked.push(q.clone());
            Some("both".into())
        },
    );
    r.unwrap();
    assert_eq!(
        asked,
        [Ask::Conflict {
            name: "n.txt".into()
        }]
    );
    assert_eq!(read(&b.join("n 2.txt")), "n2");
    assert_eq!(read(&b.join("n.txt")), "n1");
    let id = w.id();
    let (_, r) = w.run(
        Request::Copy {
            id,
            items: vec![local(&a.join("n.txt"))],
            to: local(&b),
            remove: false,
        },
        &mut |_| Some("replace".into()),
    );
    r.unwrap();
    assert_eq!(read(&b.join("n.txt")), "n2");

    // Pasted into its own folder, a copy gets a new name and no question.
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&b.join("n.txt"))],
        to: local(&b),
        remove: false,
    });
    assert_eq!(read(&b.join("n 3.txt")), "n2");

    // A move on one drive is a rename.
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&a.join("docs"))],
        to: local(&t.0),
        remove: true,
    });
    assert!(!a.join("docs").exists());
    assert_eq!(read(&t.0.join("docs/x.txt")), "x1");

    // A folder can't go into itself.
    let id = w.id();
    let (_, r) = w.run(
        Request::Copy {
            id,
            items: vec![local(&t.0.join("docs"))],
            to: local(&t.0.join("docs/deep")),
            remove: false,
        },
        &mut |_| None,
    );
    assert!(r.is_err());

    let id = w.id();
    w.ok(Request::Rename {
        id,
        item: local(&b.join("n 2.txt")),
        name: "renamed.txt".into(),
    });
    assert!(b.join("renamed.txt").exists());
    let id = w.id();
    let (_, r) = w.run(
        Request::Rename {
            id,
            item: local(&b.join("renamed.txt")),
            name: "n.txt".into(),
        },
        &mut |_| None,
    );
    assert_eq!(r, Err("\u{201c}n.txt\u{201d} is taken".into()));
    let id = w.id();
    w.ok(Request::Mkdir {
        id,
        at: local(&b),
        name: "new".into(),
    });
    assert!(b.join("new").is_dir());
}

#[test]
fn delete_goes_to_the_trash_or_for_good() {
    let t = tmp("delete");
    let home = t.0.join("home");
    write(&home.join("a/one.txt"), "1");
    write(&home.join("a/two.txt"), "2");
    let mut w = W::new(&t, &[]);
    let id = w.id();
    w.ok(Request::Delete {
        id,
        items: vec![local(&home.join("a/one.txt"))],
        trash: true,
    });
    let trash = home.join(".local/share/Trash");
    assert_eq!(read(&trash.join("files/one.txt")), "1");
    assert!(trash.join("info/one.txt.trashinfo").exists());
    // Out of the Trash for good: its info goes with it.
    let id = w.id();
    w.ok(Request::Delete {
        id,
        items: vec![local(&trash.join("files/one.txt"))],
        trash: false,
    });
    assert!(!trash.join("files/one.txt").exists());
    assert!(!trash.join("info/one.txt.trashinfo").exists());
    let id = w.id();
    w.ok(Request::Delete {
        id,
        items: vec![local(&home.join("a"))],
        trash: false,
    });
    assert!(!home.join("a").exists());
}

#[test]
fn a_cancel_before_it_starts_stops_it() {
    let t = tmp("cancel");
    write(&t.0.join("a/x.txt"), "x");
    std::fs::create_dir_all(t.0.join("b")).unwrap();
    let mut w = W::new(&t, &[]);
    // Something slow-free first, so both lines arrive while it is queued.
    let first = w.id();
    let id = w.id();
    w.c.send(Request::Mkdir {
        id: first,
        at: local(&t.0),
        name: "c".into(),
    });
    w.c.send(Request::Copy {
        id,
        items: vec![local(&t.0.join("a/x.txt"))],
        to: local(&t.0.join("b")),
        remove: false,
    });
    w.c.send(Request::Cancel { id });
    let start = std::time::Instant::now();
    let mut end = None;
    while end.is_none() && start.elapsed().as_secs() < 10 {
        for e in w.c.events() {
            if e.id() == id && matches!(e, Event::Done { .. } | Event::Error { .. }) {
                end = Some(e);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    match end {
        // The copy may have run before the cancel arrived; either way it ends.
        Some(Event::Error { text, .. }) => assert_eq!(text, "Stopped"),
        Some(Event::Done { .. }) => {}
        other => panic!("{other:?}"),
    }
}

fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn archives_open_extract_and_refuse_to_escape() {
    if !have("bsdtar") {
        eprintln!("no bsdtar: skipped");
        return;
    }
    let t = tmp("arc");
    let src = t.0.join("src");
    write(&src.join("d/a.txt"), "a");
    write(&src.join("d/sub/b.txt"), "b");
    let zip = t.0.join("x.zip");
    let ok = std::process::Command::new("bsdtar")
        .arg("-a")
        .arg("-cf")
        .arg(&zip)
        .arg("-C")
        .arg(&src)
        .arg("d")
        .status()
        .unwrap();
    assert!(ok.success());
    let mut w = W::new(&t, &[]);
    let id = w.id();
    let ev = w.ok(Request::Contents {
        id,
        archive: local(&zip),
    });
    assert_eq!(names(&ev), ["d", "d/a.txt", "d/sub", "d/sub/b.txt"]);
    let kinds: Vec<Kind> = ev
        .iter()
        .flat_map(|e| match e {
            Event::Entries { entries, .. } => entries.iter().map(|x| x.kind).collect(),
            _ => Vec::new(),
        })
        .collect();
    assert!(kinds.contains(&Kind::Dir));

    // From inside "d": "sub" lands as "sub", not "d/sub".
    let out = t.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let id = w.id();
    w.ok(Request::Extract {
        id,
        archive: local(&zip),
        entries: vec!["d/sub".into()],
        to: local(&out),
    });
    assert_eq!(read(&out.join("sub/b.txt")), "b");
    assert!(!out.join("d").exists());
    // The whole archive, into a new folder.
    let id = w.id();
    w.ok(Request::Extract {
        id,
        archive: local(&zip),
        entries: Vec::new(),
        to: local(&t.0.join("x")),
    });
    assert_eq!(read(&t.0.join("x/d/a.txt")), "a");
    let leftovers: Vec<_> = std::fs::read_dir(&t.0)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".gh0st-extract")
        })
        .collect();
    assert!(leftovers.is_empty());

    // An entry that climbs out is refused, and nothing lands outside.
    write(&t.0.join("e/f"), "evil");
    let evil = t.0.join("evil.tar");
    let ok = std::process::Command::new("bsdtar")
        .arg("-cf")
        .arg(&evil)
        .arg("-C")
        .arg(t.0.join("e"))
        .args(["-s", ",^f,../../escaped,", "f"])
        .status()
        .unwrap();
    assert!(ok.success());
    let inner = t.0.join("in/deeper");
    std::fs::create_dir_all(&inner).unwrap();
    let id = w.id();
    let (_, r) = w.run(
        Request::Extract {
            id,
            archive: local(&evil),
            entries: Vec::new(),
            to: local(&inner),
        },
        &mut |_| None,
    );
    assert!(r.is_err(), "{r:?}");
    assert!(!t.0.join("escaped").exists() && !t.0.join("in/escaped").exists());

    let id = w.id();
    w.ok(Request::Compress {
        id,
        items: vec![local(&src.join("d"))],
        to: local(&t.0.join("made.zip")),
    });
    let id = w.id();
    let ev = w.ok(Request::Contents {
        id,
        archive: local(&t.0.join("made.zip")),
    });
    assert!(names(&ev).contains(&"d/sub/b.txt".to_string()));
}

fn sftp_server() -> Option<&'static str> {
    [
        "/usr/lib/openssh/sftp-server",
        "/usr/libexec/sftp-server",
        "/usr/libexec/openssh/sftp-server",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
}

#[test]
fn sftp_lists_copies_both_ways_and_changes_files() {
    let Some(srv) = sftp_server() else {
        eprintln!("no sftp-server: skipped");
        return;
    };
    let t = tmp("sftp");
    let remote = t.0.join("remote");
    write(&remote.join("r.txt"), "remote");
    std::fs::create_dir_all(remote.join("dir")).unwrap();
    std::os::unix::fs::symlink("dir", remote.join("to-dir")).unwrap();
    write(&t.0.join("card/c.txt"), "card");
    let mut w = W::new(&t, &[("GH0ST_FILES_SFTP", srv)]);
    let mut s = Server::new(Proto::Sftp);
    s.name = "t".into();
    s.host = "test".into();
    w.c.send(Request::Server(s));
    let at = |p: &Path| Loc::Remote {
        server: "t".into(),
        path: p.to_string_lossy().into_owned(),
    };

    let id = w.id();
    let ev = w.ok(Request::List {
        id,
        at: at(&remote),
    });
    assert_eq!(names(&ev), ["dir", "r.txt", "to-dir"]);
    let link = ev.iter().find_map(|e| match e {
        Event::Entries { entries, .. } => entries.iter().find(|x| x.name == "to-dir").cloned(),
        _ => None,
    });
    assert_eq!(
        link.map(|l| (l.kind, l.link)),
        Some((Kind::LinkDir, Some("dir".into())))
    );

    let id = w.id();
    let ev = w.ok(Request::List {
        id,
        at: Loc::Remote {
            server: "t".into(),
            path: "~".into(),
        },
    });
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Resolved { path, .. } if path.starts_with('/')))
    );

    // Card to server, server to card.
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&t.0.join("card/c.txt"))],
        to: at(&remote.join("dir")),
        remove: false,
    });
    assert_eq!(read(&remote.join("dir/c.txt")), "card");
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![at(&remote.join("r.txt")), at(&remote.join("dir"))],
        to: local(&t.0.join("card")),
        remove: false,
    });
    assert_eq!(read(&t.0.join("card/r.txt")), "remote");
    assert_eq!(read(&t.0.join("card/dir/c.txt")), "card");

    // A 300 KB file: many reads and writes, the bytes kept.
    let big: String = (0..300_000)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    write(&t.0.join("card/big.txt"), &big);
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&t.0.join("card/big.txt"))],
        to: at(&remote),
        remove: true,
    });
    assert_eq!(read(&remote.join("big.txt")), big);
    assert!(
        !t.0.join("card/big.txt").exists(),
        "a move removes the source"
    );

    let id = w.id();
    w.ok(Request::Rename {
        id,
        item: at(&remote.join("r.txt")),
        name: "s.txt".into(),
    });
    assert!(remote.join("s.txt").exists());
    let id = w.id();
    w.ok(Request::Mkdir {
        id,
        at: at(&remote),
        name: "made".into(),
    });
    assert!(remote.join("made").is_dir());
    let id = w.id();
    w.ok(Request::Delete {
        id,
        items: vec![at(&remote.join("dir")), at(&remote.join("s.txt"))],
        trash: false,
    });
    assert!(!remote.join("dir").exists() && !remote.join("s.txt").exists());
}

/// Real ssh against a real sshd: the new-host question, then the password,
/// a wrong one asked again, and the key kept for next time. Runs only with
/// `GH0ST_FILES_SSH=user:password@host`, as the container check does.
#[test]
fn ssh_asks_for_the_host_key_and_the_password() {
    let Ok(spec) = std::env::var("GH0ST_FILES_SSH") else {
        eprintln!("GH0ST_FILES_SSH unset: skipped");
        return;
    };
    let (cred, host) = spec.rsplit_once('@').unwrap();
    let (user, pass) = cred.split_once(':').unwrap();
    let t = tmp("ssh");
    let mut s = Server::new(Proto::Sftp);
    s.name = "real".into();
    s.host = host.into();
    s.user = user.into();
    let home = Loc::Remote {
        server: "real".into(),
        path: "~".into(),
    };
    for round in 0..2 {
        let mut w = W::new(&t, &[]);
        w.c.send(Request::Server(s.clone()));
        let id = w.id();
        let mut asked = Vec::new();
        let (ev, r) = w.run(
            Request::List {
                id,
                at: home.clone(),
            },
            &mut |a| {
                asked.push(a.clone());
                match a {
                    Ask::HostKey { .. } => Some("yes".into()),
                    Ask::Password { retry: false, .. } => Some("wrong".into()),
                    Ask::Password { retry: true, .. } => Some(pass.into()),
                    _ => None,
                }
            },
        );
        r.unwrap();
        assert!(ev.iter().any(|e| matches!(e, Event::Resolved { .. })));
        let kinds: Vec<&str> = asked
            .iter()
            .map(|a| match a {
                Ask::HostKey { fingerprint, .. } => {
                    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
                    "hostkey"
                }
                Ask::Password { retry: false, .. } => "password",
                Ask::Password { retry: true, .. } => "again",
                _ => "other",
            })
            .collect();
        if round == 0 {
            assert_eq!(kinds, ["hostkey", "password", "again"]);
        } else {
            assert_eq!(kinds, ["password", "again"], "the key is known now");
        }
    }
}

#[test]
fn the_ssh_key_is_sealed_with_the_passphrase_given() {
    if !std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok()
    {
        eprintln!("no ssh-keygen: skipped");
        return;
    }
    let t = tmp("key");
    let mut w = W::new(&t, &[]);
    let id = w.id();
    w.ok(Request::MakeKey {
        id,
        passphrase: "correct horse".into(),
    });
    let key = t.0.join("data/ssh/id_ed25519");
    let open = |pass: &str| {
        std::process::Command::new("ssh-keygen")
            .args(["-y", "-P", pass, "-f"])
            .arg(&key)
            .output()
            .unwrap()
    };
    let good = open("correct horse");
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    assert!(String::from_utf8_lossy(&good.stdout).starts_with("ssh-ed25519 "));
    assert!(!open("").status.success(), "no key opens without it");
    assert!(t.0.join("data/ssh/id_ed25519.pub").exists());
    let id = w.id();
    let (_, r) = w.run(
        Request::MakeKey {
            id,
            passphrase: "x".into(),
        },
        &mut |_| None,
    );
    assert_eq!(r, Err("There is a key already".into()));
}

// ---- found in review: each lost data or escaped before ----

#[test]
fn a_move_keeps_what_was_skipped() {
    let t = tmp("skipmove");
    let mut w = W::new(&t, &[]);
    // Two servers stand in for two drives, so the move copies.
    let Some(srv) = sftp_server() else {
        eprintln!("no sftp-server: skipped");
        return;
    };
    let _ = srv;
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    write(&a.join("dir/keep.txt"), "mine");
    write(&a.join("dir/go.txt"), "go");
    write(&b.join("dir/keep.txt"), "theirs");
    let mut w2 = W::new(&t, &[("GH0ST_FILES_SFTP", sftp_server().unwrap())]);
    let mut s = Server::new(Proto::Sftp);
    s.name = "t".into();
    s.host = "x".into();
    w2.c.send(Request::Server(s));
    let id = w2.id();
    let (_, r) = w2.run(
        Request::Copy {
            id,
            items: vec![local(&a.join("dir"))],
            to: Loc::Remote {
                server: "t".into(),
                path: b.to_string_lossy().into_owned(),
            },
            remove: true,
        },
        &mut |q| match q {
            Ask::Conflict { name } if name == "dir" => Some("replace".into()),
            _ => Some("skip".into()),
        },
    );
    r.unwrap();
    assert_eq!(
        read(&b.join("dir/keep.txt")),
        "theirs",
        "skipped: theirs stays"
    );
    assert_eq!(read(&b.join("dir/go.txt")), "go");
    assert_eq!(
        read(&a.join("dir/keep.txt")),
        "mine",
        "skipped: the source stays"
    );
    assert!(!a.join("dir/go.txt").exists(), "what moved left");
    let _ = &mut w;
}

#[test]
fn replace_never_deletes_what_it_copies() {
    let t = tmp("ancestor");
    // A file named like the folder that holds it: Replace would delete it.
    write(&t.0.join("x/x"), "inner");
    write(&t.0.join("x/other"), "o");
    let mut w = W::new(&t, &[]);
    let id = w.id();
    let (_, r) = w.run(
        Request::Copy {
            id,
            items: vec![local(&t.0.join("x/x"))],
            to: local(&t.0),
            remove: false,
        },
        &mut |_| Some("replace".into()),
    );
    assert!(r.is_err(), "refused");
    assert_eq!(read(&t.0.join("x/x")), "inner");
    assert_eq!(read(&t.0.join("x/other")), "o");
    // A folder moved up onto its parent merges: nothing is lost.
    write(&t.0.join("proj/proj/code.txt"), "code");
    write(&t.0.join("proj/readme"), "r");
    let id = w.id();
    let (_, r) = w.run(
        Request::Copy {
            id,
            items: vec![local(&t.0.join("proj/proj"))],
            to: local(&t.0),
            remove: true,
        },
        &mut |_| Some("replace".into()),
    );
    r.unwrap();
    assert_eq!(read(&t.0.join("proj/code.txt")), "code");
    assert_eq!(read(&t.0.join("proj/readme")), "r");
}

#[test]
fn links_inside_a_folder_are_not_followed() {
    let t = tmp("links");
    let a = t.0.join("a");
    write(&a.join("f.txt"), "f");
    std::os::unix::fs::symlink(".", a.join("l1")).unwrap();
    std::os::unix::fs::symlink(".", a.join("l2")).unwrap();
    std::os::unix::fs::symlink("gone", a.join("dead")).unwrap();
    std::fs::create_dir_all(t.0.join("b")).unwrap();
    let mut w = W::new(&t, &[]);
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&a)],
        to: local(&t.0.join("b")),
        remove: false,
    });
    let b = t.0.join("b/a");
    assert_eq!(read(&b.join("f.txt")), "f");
    assert_eq!(std::fs::read_link(b.join("l1")).unwrap(), Path::new("."));
    assert_eq!(
        std::fs::read_link(b.join("dead")).unwrap(),
        Path::new("gone")
    );
}

#[test]
fn a_big_folder_moves_whole() {
    let t = tmp("big");
    let a = t.0.join("a/many");
    std::fs::create_dir_all(&a).unwrap();
    for i in 0..20_050 {
        std::fs::write(a.join(format!("{i}")), b"").unwrap();
    }
    let Some(srv) = sftp_server() else {
        eprintln!("no sftp-server: skipped");
        return;
    };
    let mut w = W::new(&t, &[("GH0ST_FILES_SFTP", srv)]);
    let mut s = Server::new(Proto::Sftp);
    s.name = "t".into();
    s.host = "x".into();
    w.c.send(Request::Server(s));
    std::fs::create_dir_all(t.0.join("b")).unwrap();
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&a)],
        to: Loc::Remote {
            server: "t".into(),
            path: t.0.join("b").to_string_lossy().into_owned(),
        },
        remove: true,
    });
    assert_eq!(
        std::fs::read_dir(t.0.join("b/many")).unwrap().count(),
        20_050
    );
    assert!(!a.exists());
}

#[test]
fn archive_names_are_names_not_options_or_patterns() {
    if !have("bsdtar") {
        eprintln!("no bsdtar: skipped");
        return;
    }
    let t = tmp("arcnames");
    let src = t.0.join("src");
    write(&src.join("photo[1].jpg"), "bracket");
    write(&src.join("photo1.jpg"), "plain");
    write(&src.join("-P"), "dash");
    let zip = t.0.join("x.zip");
    let ok = std::process::Command::new("bsdtar")
        .arg("-a")
        .arg("-cf")
        .arg(&zip)
        .arg("-C")
        .arg(&src)
        .args(["photo[1].jpg", "photo1.jpg", "./-P"])
        .status()
        .unwrap();
    assert!(ok.success());
    let out = t.0.join("out");
    let mut w = W::new(&t, &[]);
    let id = w.id();
    w.ok(Request::Extract {
        id,
        archive: local(&zip),
        entries: vec!["photo[1].jpg".into(), "-P".into()],
        to: local(&out),
    });
    assert_eq!(read(&out.join("photo[1].jpg")), "bracket");
    assert!(!out.join("photo1.jpg").exists(), "only the one named");
    assert_eq!(read(&out.join("-P")), "dash");
}

/// Only as root, as on the board: `sudo` the test binary, with
/// GH0ST_FILES_EXE naming the worker. Nobody (65534) stands in for the
/// user. A move from root's folder into the user's copies the file as the
/// user, then removes the source as root.
#[test]
fn as_root_a_move_from_roots_folder_to_the_users_lands_theirs() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // SAFETY: geteuid has no arguments.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    // /run is root's and closed to others, unlike /tmp.
    let top = Tmp(PathBuf::from(format!(
        "/run/gh0st-files-w-root-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&top.0);
    let theirs = top.0.join("theirs");
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::set_permissions(&top.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::chown(&theirs, Some(65534), Some(65534)).unwrap();
    let src = top.0.join("root.conf");
    write(&src, "conf");
    let mut w = W::new(&top, &[("GH0ST_FILES_ROOT", "65534:65534")]);
    let id = w.id();
    w.ok(Request::Copy {
        id,
        items: vec![local(&src)],
        to: local(&theirs),
        remove: true,
    });
    assert!(!src.exists(), "the source is gone");
    let moved = theirs.join("root.conf");
    assert_eq!(read(&moved), "conf");
    assert_eq!(
        std::fs::metadata(&moved).unwrap().uid(),
        65534,
        "made as the user"
    );
}
