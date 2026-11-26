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
    /// A remote folder's entries. A path of "~" is the server's home:
    /// [`Event::Resolved`] says where that is.
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
    /// Entries of `archive` (all when empty), from one folder of it, into
    /// the folder `to`, made if missing.
    Extract {
        id: Id,
        archive: Loc,
        entries: Vec<String>,
        to: Loc,
    },
    /// `items`, all in one folder, into a new zip `to`.
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
    /// Makes the app's SSH key, sealed with `passphrase`.
    MakeKey {
        id: Id,
        passphrase: String,
    },
    /// The start of a local file, for the viewer: the root worker reads
    /// what the page can't. [`Event::Text`] answers.
    Read {
        id: Id,
        file: Loc,
    },
}

impl Request {
    /// The request's own id; None for those that answer or name another.
    pub fn id(&self) -> Option<Id> {
        match self {
            Request::Server(_) | Request::Cancel { .. } | Request::Reply { .. } => None,
            Request::List { id, .. }
            | Request::Contents { id, .. }
            | Request::Copy { id, .. }
            | Request::Delete { id, .. }
            | Request::Rename { id, .. }
            | Request::Mkdir { id, .. }
            | Request::Extract { id, .. }
            | Request::Compress { id, .. }
            | Request::MakeKey { id, .. }
            | Request::Read { id, .. } => Some(*id),
        }
    }
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
    /// The app's SSH key passphrase: the page answers from the keyring.
    Passphrase,
    /// `name` is in the target already: "replace", "skip" or "both", and
    /// " all" after it to say the same for the rest.
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
    /// Where "~" turned out to be.
    Resolved {
        id: Id,
        path: String,
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
    /// What [`Request::Read`] read, ending it.
    Text {
        id: Id,
        text: String,
    },
    /// The worker's first line: it is up, and a root worker's password was
    /// right.
    Ready,
}

impl Event {
    pub fn id(&self) -> Id {
        match self {
            Event::Entries { id, .. }
            | Event::Listed { id }
            | Event::Resolved { id, .. }
            | Event::Progress { id, .. }
            | Event::Ask { id, .. }
            | Event::Done { id }
            | Event::Error { id, .. }
            | Event::Text { id, .. } => *id,
            Event::Ready => 0,
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
        if let Some(id) = req.id() {
            self.0.push(Event::Error {
                id,
                text: "This needs the worker".into(),
            });
        }
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }
}
