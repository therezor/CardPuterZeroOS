//! A helper call that needs the password: the dialog opens, and the call
//! runs again with the password on stdin, never in argv. A fake pkexec
//! first on PATH answers as gh0st-helper does. Alone in its own test
//! binary: it changes PATH, and the SDK reaps every child of the process.

use gh0st_app::helper::{Finished, Helper};
use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::rc::Rc;

/// Exit 10 without a password, 11 for a wrong one, 0 for "secret".
const PKEXEC: &str = r#"#!/bin/sh
echo "argv: $*" >> "$HELPER_LOG"
read -r pw
echo "stdin: $pw" >> "$HELPER_LOG"
case "$pw" in
  "") exit 10 ;;
  secret) echo "gh0st-helper: done"; exit 0 ;;
  *) exit 11 ;;
esac
"#;

type Log = Rc<RefCell<Vec<Finished>>>;

struct Installer {
    helper: Helper,
    log: Log,
}

impl Page for Installer {
    fn title(&self) -> &str {
        "Installer"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, cx: &mut Cx) {
        self.helper.run(cx, 7, &["update-install"]);
    }
    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        if let Some(f) = self.helper.job(cx, job, event) {
            self.log.borrow_mut().push(f);
        }
    }
    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if let Some(f) = self.helper.answer(cx, id, answer) {
            self.log.borrow_mut().push(f);
        }
    }
}

#[test]
fn a_wrong_password_asks_again_and_the_right_one_runs_the_call() {
    let dir = std::env::temp_dir().join(format!("gh0st-helper-test-{}", std::process::id()));
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

    let log = Log::default();
    let page = Installer {
        helper: Helper::new(900),
        log: log.clone(),
    };
    let mut d = Driver::new(page);
    d.jobs(5000);
    assert!(
        log.borrow().is_empty(),
        "the first run asks, it doesn't end"
    );
    d.keys("type wrong\nenter");
    d.jobs(5000);
    assert!(log.borrow().is_empty(), "a wrong one asks again");
    d.keys("type secret\nenter");
    d.jobs(5000);
    assert_eq!(
        *log.borrow(),
        [Finished {
            tag: 7,
            result: Ok("gh0st-helper: done\n".into()),
        }]
    );
    let calls = std::fs::read_to_string(&log_path).unwrap();
    assert_eq!(
        calls,
        "argv: /usr/libexec/gh0st-helper update-install\nstdin: \n\
         argv: /usr/libexec/gh0st-helper update-install\nstdin: wrong\n\
         argv: /usr/libexec/gh0st-helper update-install\nstdin: secret\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
