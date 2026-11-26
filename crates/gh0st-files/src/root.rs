//! Root mode as the page sees it: a second worker, started by
//! `gh0st-helper files-worker` after the user's password. Requests wait
//! until the worker says it is up, which is how the page learns the
//! password was right. Tests and mockups give it an engine of their own.

use crate::engine::{Engine, Event, Id, Request};
use crate::proto::{self, Conn};
use gh0st_app::helper::{Held, HeldEvent};
use gh0st_app::{Answer, Cx, Watch};
use std::collections::BTreeSet;
use std::os::fd::RawFd;

pub struct Root {
    held: Option<Held>,
    conn: Option<Conn>,
    test: Option<Box<dyn Engine>>,
    up: bool,
    queue: Vec<Request>,
    /// Requests sent and not yet done: a worker that ends fails them.
    open: BTreeSet<Id>,
    out: Vec<Event>,
}

impl Root {
    /// The real one, through the helper. `ask` is its password dialog's id.
    pub fn helper(ask: u32) -> Root {
        Root {
            held: Some(Held::new(ask, &["files-worker"])),
            conn: None,
            test: None,
            up: false,
            queue: Vec::new(),
            open: BTreeSet::new(),
            out: Vec::new(),
        }
    }

    /// Over `engine`, up at once: for tests and mockups.
    pub fn open(engine: Box<dyn Engine>) -> Root {
        Root {
            held: None,
            conn: None,
            test: Some(engine),
            up: true,
            queue: Vec::new(),
            open: BTreeSet::new(),
            out: Vec::new(),
        }
    }

    /// Starts the worker. An error when it can't even begin.
    pub fn start(&mut self, cx: &mut Cx) -> Result<(), String> {
        let Some(h) = self.held.as_mut() else {
            return Ok(());
        };
        let e = h.start(cx);
        self.took(e).unwrap_or(Ok(()))
    }

    /// Up: the password, if one was asked, was right.
    pub fn up(&self) -> bool {
        self.up
    }

    /// A child ended. `Some` when root mode is over, with why.
    pub fn exited(
        &mut self,
        cx: &mut Cx,
        pid: i32,
        code: Option<i32>,
    ) -> Option<Result<(), String>> {
        let e = self.held.as_mut()?.exited(cx, pid, code)?;
        self.took(e)
    }

    /// A dialog's answer. `Some` when root mode is over, with why.
    pub fn answer(&mut self, cx: &mut Cx, id: u32, answer: Answer) -> Option<Result<(), String>> {
        let e = self.held.as_mut()?.answer(cx, id, answer)?;
        self.took(e)
    }

    /// The helper's stderr is readable. True if `watch` was its.
    pub fn ready(&mut self, watch: Watch) -> bool {
        self.held.as_mut().is_some_and(|h| h.ready(watch))
    }

    /// Ends root mode: the worker reads the link closing and goes.
    pub fn stop(&mut self, cx: &mut Cx) {
        if let Some(h) = self.held.as_mut() {
            h.stop(cx);
        }
        self.conn = None;
        self.fail_open("Root mode ended");
    }

    fn took(&mut self, e: HeldEvent) -> Option<Result<(), String>> {
        match e {
            HeldEvent::Started(fd) => {
                self.conn = Some(Conn::new(fd));
                self.up = false;
                None
            }
            HeldEvent::Ended(r) => {
                self.conn = None;
                self.up = false;
                self.fail_open("Root mode ended");
                let queued = std::mem::take(&mut self.queue);
                for id in queued.iter().filter_map(Request::id) {
                    self.out.push(Event::Error {
                        id,
                        text: "Not as root".into(),
                    });
                }
                Some(r)
            }
        }
    }

    fn fail_open(&mut self, why: &str) {
        for id in std::mem::take(&mut self.open) {
            self.out.push(Event::Error {
                id,
                text: why.into(),
            });
        }
    }

    fn send_now(&mut self, req: Request) {
        if let Some(id) = req.id() {
            self.open.insert(id);
        }
        let sent = self
            .conn
            .as_mut()
            .is_some_and(|c| c.send(&proto::request(&req)).is_ok());
        if !sent {
            self.conn = None;
            self.fail_open("Root mode ended");
        }
    }
}

impl Engine for Root {
    fn send(&mut self, req: Request) {
        if let Some(t) = self.test.as_mut() {
            return t.send(req);
        }
        if self.up {
            self.send_now(req);
        } else {
            self.queue.push(req);
        }
    }

    fn events(&mut self) -> Vec<Event> {
        if let Some(t) = self.test.as_mut() {
            return t.events();
        }
        let Some(c) = self.conn.as_mut() else {
            return std::mem::take(&mut self.out);
        };
        let lines = c.recv();
        let eof = c.eof();
        let mut greeted = false;
        for f in lines {
            let Some(e) = proto::parse_event(&f) else {
                continue;
            };
            if e == Event::Ready {
                greeted = true;
                continue;
            }
            if let Event::Done { id }
            | Event::Error { id, .. }
            | Event::Listed { id }
            | Event::Text { id, .. } = e
            {
                self.open.remove(&id);
            }
            match (self.out.last_mut(), e) {
                (
                    Some(Event::Entries { id: a, entries }),
                    Event::Entries {
                        id: b,
                        entries: more,
                    },
                ) if *a == b => entries.extend(more),
                (_, e) => self.out.push(e),
            }
        }
        if greeted && !self.up {
            self.up = true;
            if let Some(h) = self.held.as_mut() {
                h.up();
            }
            for req in std::mem::take(&mut self.queue) {
                self.send_now(req);
            }
        }
        if eof {
            self.conn = None;
            self.fail_open("Root mode ended");
        }
        std::mem::take(&mut self.out)
    }

    fn fd(&self) -> Option<RawFd> {
        match &self.test {
            Some(t) => t.fd(),
            None => self.conn.as_ref().map(Conn::fd),
        }
    }
}
