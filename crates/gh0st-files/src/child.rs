//! The worker as the page sees it: `gh0st-files --worker`, started on the
//! first request, on a stream socket pair whose fd the page watches.

use crate::engine::{Engine, Event, Id, Request};
use crate::proto::{self, Conn};
use std::collections::BTreeSet;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where the worker finds its end of the pair.
pub const FD_ENV: &str = "GH0ST_FILES_FD";

pub struct Child {
    exe: PathBuf,
    env: Vec<(String, String)>,
    conn: Option<Conn>,
    /// Requests sent and not yet done: a worker that dies fails them.
    open: BTreeSet<Id>,
    out: Vec<Event>,
}

impl Child {
    /// This program, run again as the worker.
    pub fn new() -> Child {
        let exe = std::env::current_exe().unwrap_or_else(|_| "/usr/bin/gh0st-files".into());
        Child::with_exe(exe)
    }

    pub fn with_exe(exe: PathBuf) -> Child {
        Child {
            exe,
            env: Vec::new(),
            conn: None,
            open: BTreeSet::new(),
            out: Vec::new(),
        }
    }

    /// Adds to the worker's environment, as tests do.
    pub fn env(mut self, k: &str, v: &str) -> Child {
        self.env.push((k.into(), v.into()));
        self
    }

    fn start(&mut self) -> io::Result<()> {
        let (mine, theirs) = proto::pair()?;
        let n = theirs.as_raw_fd();
        let mut cmd = Command::new(&self.exe);
        cmd.arg("--worker")
            .env(FD_ENV, n.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        // SAFETY: fcntl and prctl are async-signal-safe. The worker's end
        // stays open across exec, and the worker dies with the page.
        unsafe {
            cmd.pre_exec(move || {
                if libc::fcntl(n, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                #[cfg(target_os = "linux")]
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        cmd.spawn()?;
        drop::<OwnedFd>(theirs);
        self.conn = Some(Conn::new(mine));
        Ok(())
    }

    fn fail_open(&mut self, why: &str) {
        for id in std::mem::take(&mut self.open) {
            self.out.push(Event::Error {
                id,
                text: why.into(),
            });
        }
    }
}

impl Default for Child {
    fn default() -> Child {
        Child::new()
    }
}

impl Engine for Child {
    fn send(&mut self, req: Request) {
        if let Some(id) = req.id() {
            self.open.insert(id);
        }
        if self.conn.is_none() {
            if let Err(e) = self.start() {
                self.fail_open(&format!("Couldn't start the worker: {e}"));
                return;
            }
        }
        let sent = self
            .conn
            .as_mut()
            .map(|c| c.send(&proto::request(&req)).is_ok())
            .unwrap_or(false);
        if !sent {
            self.conn = None;
            self.fail_open("The worker stopped");
        }
    }

    fn events(&mut self) -> Vec<Event> {
        if let Some(c) = self.conn.as_mut() {
            let lines = c.recv();
            let eof = c.eof();
            for f in lines {
                let Some(e) = proto::parse_event(&f) else {
                    continue;
                };
                if let Event::Done { id } | Event::Error { id, .. } | Event::Listed { id } = e {
                    self.open.remove(&id);
                }
                // A listing's lines join into one event.
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
            if eof {
                self.conn = None;
                self.fail_open("The worker stopped");
            }
        }
        std::mem::take(&mut self.out)
    }

    fn fd(&self) -> Option<RawFd> {
        self.conn.as_ref().map(Conn::fd)
    }
}
