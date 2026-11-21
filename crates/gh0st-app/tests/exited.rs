//! A child a page adopts reports its exit. Alone in its own test binary:
//! the SDK reaps every child of the process.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::rc::Rc;

type Exit = Rc<RefCell<Option<(i32, Option<i32>)>>>;

struct Parent {
    exit: Exit,
    pid: i32,
}

impl Page for Parent {
    fn title(&self) -> &str {
        "parent"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, cx: &mut Cx) {
        // The SDK reaps it, so no wait() here.
        #[allow(clippy::zombie_processes)]
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        self.pid = child.id() as i32;
        cx.adopt(self.pid);
    }
    fn exited(&mut self, pid: i32, code: Option<i32>, _: &mut Cx) {
        *self.exit.borrow_mut() = Some((pid, code));
    }
}

#[test]
fn an_adopted_child_reports_its_exit() {
    let exit = Rc::new(RefCell::new(None));
    let mut d = Driver::new(Parent {
        exit: exit.clone(),
        pid: 0,
    });
    for _ in 0..100 {
        d.pump(10);
        if exit.borrow().is_some() {
            break;
        }
    }
    let (pid, code) = exit.borrow().unwrap();
    assert!(pid > 0);
    assert_eq!(code, Some(3));
}
