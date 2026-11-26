//! A helper that stays up: the first run, with no password, asks; a wrong
//! password asks again; the right one starts the program, which greets the
//! page on the socket and gets the page's next line, never the password.
//! A fake pkexec first on PATH answers as gh0st-helper files-worker does.
//! Alone in its own test binary: it changes PATH, and the SDK reaps every
//! child of the process.

use gh0st_app::Watch;
use gh0st_app::helper::{Held, HeldEvent};
use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

/// One line, then: 10 without a password, 11 for a wrong one, and for
/// "secret" a greeting on the socket, then the page's next line logged.
const PKEXEC: &str = r#"#!/bin/sh
echo "argv: $*" >> "$HELPER_LOG"
IFS= read -r pw
echo "line: $pw" >> "$HELPER_LOG"
case "$pw" in
  "") exit 10 ;;
  secret)
    printf 'ready\n' >&0
    IFS= read -r req
    echo "got: $req" >> "$HELPER_LOG"
    exit 0 ;;
  *) exit 11 ;;
esac
"#;

#[derive(Default)]
struct Seen {
    up: bool,
    ended: Option<Result<(), String>>,
}

struct Root {
    held: Held,
    link: Option<(OwnedFd, Watch)>,
    seen: Rc<RefCell<Seen>>,
}

impl Root {
    fn took(&mut self, e: HeldEvent, cx: &mut Cx) {
        match e {
            HeldEvent::Started(fd) => {
                let w = cx.watch(fd.as_raw_fd());
                self.link = Some((fd, w));
            }
            HeldEvent::Ended(r) => self.seen.borrow_mut().ended = Some(r),
        }
    }
}

impl Page for Root {
    fn title(&self) -> &str {
        "Root"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, cx: &mut Cx) {
        let e = self.held.start(cx);
        self.took(e, cx);
    }
    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if let Some(e) = self.held.answer(cx, id, answer) {
            self.took(e, cx);
        }
    }
    fn exited(&mut self, pid: i32, code: Option<i32>, cx: &mut Cx) {
        if let Some(e) = self.held.exited(cx, pid, code) {
            self.took(e, cx);
        }
    }
    fn ready(&mut self, watch: Watch, cx: &mut Cx) {
        if self.held.ready(watch) {
            return;
        }
        let Some((fd, w)) = &self.link else {
            return;
        };
        if *w != watch {
            return;
        }
        let mut f = std::fs::File::from(fd.try_clone().unwrap());
        let mut buf = [0u8; 64];
        let n = f.read(&mut buf).unwrap_or(0);
        if n == 0 {
            cx.unwatch(*w);
            self.link = None;
            return;
        }
        if buf[..n].starts_with(b"ready") && !self.seen.borrow().up {
            self.held.up();
            self.seen.borrow_mut().up = true;
            f.write_all(b"list /\n").unwrap();
        }
    }
}

#[test]
fn the_password_starts_the_program_and_never_reaches_it() {
    let dir = std::env::temp_dir().join(format!("gh0st-held-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("pkexec");
    std::fs::write(&fake, PKEXEC).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log_path = dir.join("log");
    // SAFETY: this test binary runs one test, so nothing reads the
    // environment meanwhile.
    unsafe {
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", dir.display()));
        std::env::set_var("HELPER_LOG", &log_path);
    }
    let seen = Rc::new(RefCell::new(Seen::default()));
    let page = Root {
        held: Held::new(900, &["files-worker"]),
        link: None,
        seen: seen.clone(),
    };
    let mut d = Driver::new(page);
    d.pump(1500);
    assert!(d.dialog().is_some(), "no password: the dialog asks");
    assert!(seen.borrow().ended.is_none());
    d.keys("type wrong\nenter");
    d.pump(1500);
    assert!(d.dialog().is_some(), "wrong: it asks again");
    d.keys("type secret\nenter");
    d.pump(2000);
    assert!(seen.borrow().up, "the program greeted the page");
    assert_eq!(seen.borrow().ended, Some(Ok(())), "and ended when done");
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains("argv: /usr/libexec/gh0st-helper files-worker"),
        "{log}"
    );
    assert!(
        log.contains("line: \n") || log.contains("line: \nline"),
        "first an empty line: {log}"
    );
    assert!(
        log.contains("got: list /"),
        "the page's line reached it: {log}"
    );
    assert!(
        !log.contains("got: secret"),
        "the password never did: {log}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
