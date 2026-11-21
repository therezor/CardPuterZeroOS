//! A job's lines and exit code reach the page under the test driver. Alone
//! in its own test binary: the SDK reaps every child of the process.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<JobEvent>>>;

struct Runner(Log);

impl Page for Runner {
    fn title(&self) -> &str {
        "Runner"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, cx: &mut Cx) {
        cx.spawn(&["/bin/sh", "-c", "echo one; echo two; exit 4"]);
    }
    fn job(&mut self, _: Job, event: JobEvent, _: &mut Cx) {
        self.0.borrow_mut().push(event);
    }
}

#[test]
fn a_job_s_lines_then_its_code() {
    let log = Log::default();
    let mut d = Driver::new(Runner(log.clone()));
    d.jobs(5000);
    assert_eq!(
        *log.borrow(),
        [
            JobEvent::Line("one".into()),
            JobEvent::Line("two".into()),
            JobEvent::Done(Some(4)),
        ]
    );
}
