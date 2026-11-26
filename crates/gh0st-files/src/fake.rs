//! A worker for mockups and tests: servers and archives from fixtures,
//! answers at once, and what the page asked kept for the test to read.

use crate::engine::{Ask, Engine, Event, Id, Request};
use crate::model::{Entry, Kind, Loc};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::fd::RawFd;
use std::rc::Rc;

/// What a copy does before it would finish.
#[derive(Clone, Debug, Default)]
pub struct CopyScript {
    /// Stays under way here: bytes done, total, the file.
    pub progress: Option<(u64, u64, String)>,
    /// Asks about this name first.
    pub conflict: Option<String>,
}

#[derive(Default)]
pub struct Fake {
    /// Remote folders by server and path.
    pub folders: HashMap<(String, String), Vec<Entry>>,
    /// Archive contents by the archive's name: paths inside, and entries.
    pub archives: HashMap<String, Vec<(String, Entry)>>,
    /// Asked on a server's first listing, in order, before it answers.
    pub first: HashMap<String, Vec<Ask>>,
    /// Servers whose listing fails, with what they say.
    pub fail: HashMap<String, String>,
    pub copy: CopyScript,
    /// Every request, for tests.
    pub log: Rc<RefCell<Vec<Request>>>,
    out: Vec<Event>,
    /// Listings waiting on asks: id, what to list, asks left.
    waiting: Vec<(Id, Loc, Vec<Ask>)>,
    /// Copies waiting on a conflict's answer.
    conflicted: HashSet<Id>,
}

impl Fake {
    pub fn new() -> Fake {
        Fake::default()
    }

    pub fn folder(mut self, server: &str, path: &str, entries: Vec<Entry>) -> Fake {
        self.folders.insert((server.into(), path.into()), entries);
        self
    }

    fn answer_list(&mut self, id: Id, at: &Loc) {
        // A local folder lists under server "": the root worker's fixtures.
        let (server, path) = match at {
            Loc::Remote { server, path } => (server.clone(), path.clone()),
            Loc::Local(p) => (String::new(), p.to_string_lossy().into_owned()),
            _ => return,
        };
        let (server, path) = (&server, &path);
        if let Some(why) = self.fail.get(server) {
            self.out.push(Event::Error {
                id,
                text: why.clone(),
            });
            return;
        }
        let entries = self
            .folders
            .get(&(server.clone(), path.clone()))
            .cloned()
            .unwrap_or_default();
        self.out.push(Event::Entries { id, entries });
        self.out.push(Event::Listed { id });
    }

    fn contents(&mut self, id: Id, archive: &Loc) {
        let name = archive.leaf().unwrap_or_default();
        match self.archives.get(&name) {
            Some(v) => {
                let entries = v
                    .iter()
                    .map(|(p, e)| Entry {
                        name: p.clone(),
                        ..e.clone()
                    })
                    .collect();
                self.out.push(Event::Entries { id, entries });
                self.out.push(Event::Listed { id });
            }
            None => self.out.push(Event::Error {
                id,
                text: "Not an archive this can open".into(),
            }),
        }
    }
}

impl Engine for Fake {
    fn send(&mut self, req: Request) {
        self.log.borrow_mut().push(req.clone());
        match req {
            Request::List { id, at } => {
                let server = match &at {
                    Loc::Remote { server, .. } => server.clone(),
                    _ => String::new(),
                };
                match self.first.remove(&server) {
                    Some(mut asks) if !asks.is_empty() => {
                        let ask = asks.remove(0);
                        self.out.push(Event::Ask { id, ask });
                        self.waiting.push((id, at, asks));
                    }
                    _ => self.answer_list(id, &at),
                }
            }
            Request::Reply { id, text } => {
                if self.conflicted.remove(&id) {
                    self.out.push(Event::Done { id });
                    return;
                }
                let Some(i) = self.waiting.iter().position(|w| w.0 == id) else {
                    return;
                };
                if text.is_none() {
                    self.waiting.remove(i);
                    self.out.push(Event::Error {
                        id,
                        text: "Not connected".into(),
                    });
                    return;
                }
                if self.waiting[i].2.is_empty() {
                    let (id, at, _) = self.waiting.remove(i);
                    self.answer_list(id, &at);
                } else {
                    let ask = self.waiting[i].2.remove(0);
                    self.out.push(Event::Ask { id, ask });
                }
            }
            Request::Contents { id, archive } => self.contents(id, &archive),
            Request::Copy { id, .. } => {
                if let Some(name) = self.copy.conflict.clone() {
                    self.out.push(Event::Ask {
                        id,
                        ask: Ask::Conflict { name },
                    });
                    self.conflicted.insert(id);
                    return;
                }
                match self.copy.progress.clone() {
                    Some((done, total, name)) => self.out.push(Event::Progress {
                        id,
                        done,
                        total,
                        name,
                    }),
                    None => self.out.push(Event::Done { id }),
                }
            }
            Request::Delete { id, .. }
            | Request::Rename { id, .. }
            | Request::Mkdir { id, .. }
            | Request::Extract { id, .. }
            | Request::Compress { id, .. }
            | Request::MakeKey { id, .. } => self.out.push(Event::Done { id }),
            Request::Read { id, file } => self.out.push(Event::Text {
                id,
                text: format!("text of {}", crate::worker::loc_path(&file)),
            }),
            Request::Server(_) | Request::Cancel { .. } => {}
        }
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.out)
    }

    fn fd(&self) -> Option<RawFd> {
        None
    }
}

/// An entry for fixtures, with a time.
pub fn at(mut e: Entry, mtime: i64) -> Entry {
    e.mtime = mtime;
    e
}

/// A folder entry inside an archive.
pub fn arc_dir(path: &str) -> (String, Entry) {
    (path.into(), Entry::dir(path))
}

pub fn arc_file(path: &str, size: u64) -> (String, Entry) {
    (
        path.into(),
        Entry {
            kind: Kind::File,
            ..Entry::file(path, size)
        },
    )
}
