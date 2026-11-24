//! What the page asks of the worker, and what comes back. The worker is a
//! child, `gh0st-files --worker`, on a socket pair: anything that can be
//! slow (servers, archives, copies) runs there, so the page never blocks.
//! Each request has an id; its events name it.

use crate::model::{Entry, Loc};
use crate::servers::Server;
use std::os::fd::RawFd;

pub type Id = u32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// A server the next requests may name, sent before its first use.
    Server(Server),
    /// A remote folder's entries.
    List {
        id: Id,
        at: Loc,
    },
    /// An archive's whole table of contents, as paths inside it.
    Contents {
        id: Id,
        archive: Loc,
    },
    /// Copies `items` into the folder `to`; `remove` makes it a move.
    Copy {
        id: Id,
        items: Vec<Loc>,
        to: Loc,
        remove: bool,
    },
    /// `trash` moves local items to the Trash instead.
    Delete {
        id: Id,
        items: Vec<Loc>,
        trash: bool,
    },
    Rename {
        id: Id,
        item: Loc,
        name: String,
    },
    Mkdir {
        id: Id,
        at: Loc,
        name: String,
    },
    /// Entries of `archive` (all when empty) into a new folder `to`.
    Extract {
        id: Id,
        archive: Loc,
        entries: Vec<String>,
        to: Loc,
    },
    /// `items` into a new zip `to`.
    Compress {
        id: Id,
        items: Vec<Loc>,
        to: Loc,
    },
    Cancel {
        id: Id,
    },
    /// The answer to an [`Ask`]; None says no.
    Reply {
        id: Id,
        text: Option<String>,
    },
}

/// What the worker needs the user for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The server's password. `retry` after a wrong one.
    Password { server: String, retry: bool },
    /// A server not seen before: its key's fingerprint, to trust or not.
    HostKey {
        server: String,
        host: String,
        fingerprint: String,
    },
    /// A key changed since last time: refused, the user is told.
    HostKeyChanged { server: String },
    /// The app's SSH key passphrase: the page answers from the keyring.
    Passphrase,
    /// `name` is in the target already: "replace", "skip" or "both", and
    /// "all" after a space to say the same for the rest.
    Conflict { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Some of a listing; `Listed` ends it.
    Entries {
        id: Id,
        entries: Vec<Entry>,
    },
    Listed {
        id: Id,
    },
    /// Bytes done of all, and the file under way.
    Progress {
        id: Id,
        done: u64,
        total: u64,
        name: String,
    },
    Ask {
        id: Id,
        ask: Ask,
    },
    Done {
        id: Id,
    },
    Error {
        id: Id,
        text: String,
    },
}

impl Event {
    pub fn id(&self) -> Id {
        match self {
            Event::Entries { id, .. }
            | Event::Listed { id }
            | Event::Progress { id, .. }
            | Event::Ask { id, .. }
            | Event::Done { id }
            | Event::Error { id, .. } => *id,
        }
    }
}

/// The page's line to its worker.
pub trait Engine {
    fn send(&mut self, req: Request);
    /// Events since the last call.
    fn events(&mut self) -> Vec<Event>;
    /// The fd to watch for events; None when they come at once, after each
    /// send.
    fn fd(&self) -> Option<RawFd>;
}

/// No worker: every request fails, as on a host with nothing to run.
pub struct Nothing(Vec<Event>);

impl Nothing {
    pub fn new() -> Nothing {
        Nothing(Vec::new())
    }
}

impl Default for Nothing {
    fn default() -> Nothing {
        Nothing::new()
    }
}

impl Engine for Nothing {
    fn send(&mut self, req: Request) {
        let id = match req {
            Request::Server(_) | Request::Cancel { .. } | Request::Reply { .. } => return,
            Request::List { id, .. }
            | Request::Contents { id, .. }
            | Request::Copy { id, .. }
            | Request::Delete { id, .. }
            | Request::Rename { id, .. }
            | Request::Mkdir { id, .. }
            | Request::Extract { id, .. }
            | Request::Compress { id, .. } => id,
        };
        self.0.push(Event::Error {
            id,
            text: "The worker isn't built yet".into(),
        });
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }
}
