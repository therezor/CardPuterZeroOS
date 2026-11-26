//! The page and its worker speak lines over a stream socket pair: one
//! message a line, fields split by tabs, each field escaped so a name with a
//! tab or a newline stays one field (`servers::esc`).

use crate::engine::{Ask, Event, Id, Request};
use crate::model::{Entry, Kind, Loc};
use crate::servers::{Proto, Server, esc, unesc};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

/// The longest line either side takes: a copy of many items, each a path.
const MAX_LINE: usize = 4 << 20;

/// One end of the pair: lines out, lines in, never blocking on reads.
pub struct Conn {
    fd: OwnedFd,
    buf: Vec<u8>,
    eof: bool,
}

/// A stream socket pair, both ends closing on exec.
pub fn pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: socketpair fills a two-int array.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds are fresh and owned from here.
    let (a, b) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [&a, &b] {
        // SAFETY: fcntl on an fd we own.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok((a, b))
}

impl Conn {
    pub fn new(fd: OwnedFd) -> Conn {
        // SAFETY: fcntl on an fd we own.
        unsafe {
            let fl = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        Conn {
            fd,
            buf: Vec::new(),
            eof: false,
        }
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// The other end is gone.
    pub fn eof(&self) -> bool {
        self.eof
    }

    /// Writes one line whole, waiting for room when the other side is slow.
    pub fn send(&mut self, fields: &[String]) -> io::Result<()> {
        let mut line = fields.iter().map(|f| esc(f)).collect::<Vec<_>>().join("\t");
        line.push('\n');
        let mut rest = line.as_bytes();
        while !rest.is_empty() {
            #[cfg(target_os = "linux")]
            let flags = libc::MSG_NOSIGNAL;
            #[cfg(not(target_os = "linux"))]
            let flags = 0;
            // SAFETY: sends from a live buffer.
            let n = unsafe { libc::send(self.fd(), rest.as_ptr().cast(), rest.len(), flags) };
            if n >= 0 {
                rest = &rest[n as usize..];
                continue;
            }
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => {}
                Some(libc::EAGAIN) => {
                    let mut p = libc::pollfd {
                        fd: self.fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    // SAFETY: polls one live pollfd.
                    if unsafe { libc::poll(&mut p, 1, 5000) } == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the other side stopped reading",
                        ));
                    }
                }
                _ => {
                    self.eof = true;
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Every whole line waiting, split into fields.
    pub fn recv(&mut self) -> Vec<Vec<String>> {
        let mut chunk = [0u8; 16384];
        loop {
            // SAFETY: reads into a stack buffer of its own length.
            let n = unsafe { libc::read(self.fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
            if n > 0 {
                self.buf.extend_from_slice(&chunk[..n as usize]);
                if self.buf.len() > MAX_LINE && !self.buf.contains(&b'\n') {
                    // A line this long is not ours: drop the link.
                    self.eof = true;
                    self.buf.clear();
                    break;
                }
                continue;
            }
            if n == 0 {
                self.eof = true;
                break;
            }
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::EAGAIN) => break,
                _ => {
                    self.eof = true;
                    break;
                }
            }
        }
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            out.push(text.split('\t').map(unesc).collect());
        }
        out
    }
}

// ---- locations ----

/// "L/path" for the card, "Rserver:/path" for a server. Archives never
/// travel: the worker gets the archive's own place.
pub fn loc(l: &Loc) -> String {
    match l {
        Loc::Local(p) => format!("L{}", p.display()),
        Loc::Remote { server, path } => format!("R{server}:{path}"),
        Loc::Places => "P".into(),
        Loc::Archive { file, .. } => loc(file),
    }
}

pub fn parse_loc(s: &str) -> Option<Loc> {
    let (k, rest) = s.split_at_checked(1)?;
    match k {
        "L" => Some(Loc::Local(PathBuf::from(rest))),
        "R" => {
            let (server, path) = rest.split_once(':')?;
            Some(Loc::Remote {
                server: server.into(),
                path: path.into(),
            })
        }
        "P" => Some(Loc::Places),
        _ => None,
    }
}

fn kind_code(k: Kind) -> &'static str {
    match k {
        Kind::Dir => "d",
        Kind::File => "f",
        Kind::LinkDir => "D",
        Kind::Link => "l",
        Kind::Other => "o",
    }
}

fn parse_kind(s: &str) -> Kind {
    match s {
        "d" => Kind::Dir,
        "f" => Kind::File,
        "D" => Kind::LinkDir,
        "l" => Kind::Link,
        _ => Kind::Other,
    }
}

fn b(v: bool) -> String {
    if v { "1" } else { "0" }.into()
}

// ---- requests ----

pub fn request(r: &Request) -> Vec<String> {
    let s = |x: &str| x.to_string();
    let mut f = match r {
        Request::Server(sv) => vec![
            s("server"),
            sv.name.clone(),
            s(sv.proto.id()),
            sv.host.clone(),
            sv.port.to_string(),
            sv.user.clone(),
            sv.path.clone(),
            b(sv.remember),
        ],
        Request::List { id, at } => vec![s("list"), id.to_string(), loc(at)],
        Request::Contents { id, archive } => vec![s("contents"), id.to_string(), loc(archive)],
        Request::Copy {
            id,
            items,
            to,
            remove,
        } => {
            let mut v = vec![s("copy"), id.to_string(), b(*remove), loc(to)];
            v.extend(items.iter().map(loc));
            v
        }
        Request::Delete { id, items, trash } => {
            let mut v = vec![s("delete"), id.to_string(), b(*trash)];
            v.extend(items.iter().map(loc));
            v
        }
        Request::Rename { id, item, name } => {
            vec![s("rename"), id.to_string(), loc(item), name.clone()]
        }
        Request::Mkdir { id, at, name } => vec![s("mkdir"), id.to_string(), loc(at), name.clone()],
        Request::Extract {
            id,
            archive,
            entries,
            to,
        } => {
            let mut v = vec![s("extract"), id.to_string(), loc(archive), loc(to)];
            v.extend(entries.iter().cloned());
            v
        }
        Request::Compress { id, items, to } => {
            let mut v = vec![s("compress"), id.to_string(), loc(to)];
            v.extend(items.iter().map(loc));
            v
        }
        Request::Cancel { id } => vec![s("cancel"), id.to_string()],
        Request::Reply { id, text } => vec![
            s("reply"),
            id.to_string(),
            b(text.is_some()),
            text.clone().unwrap_or_default(),
        ],
        Request::MakeKey { id, passphrase } => {
            vec![s("makekey"), id.to_string(), passphrase.clone()]
        }
        Request::Read { id, file } => vec![s("read"), id.to_string(), loc(file)],
    };
    f.shrink_to_fit();
    f
}

pub fn parse_request(f: &[String]) -> Option<Request> {
    let id = || f.get(1)?.parse::<Id>().ok();
    let locs =
        |from: usize| -> Option<Vec<Loc>> { f.get(from..)?.iter().map(|x| parse_loc(x)).collect() };
    Some(match f.first()?.as_str() {
        "server" => {
            let [_, name, proto, host, port, user, path, remember] = f else {
                return None;
            };
            Request::Server(Server {
                name: name.clone(),
                proto: Proto::parse(proto)?,
                host: host.clone(),
                port: port.parse().ok()?,
                user: user.clone(),
                path: path.clone(),
                remember: remember == "1",
            })
        }
        "list" => Request::List {
            id: id()?,
            at: parse_loc(f.get(2)?)?,
        },
        "contents" => Request::Contents {
            id: id()?,
            archive: parse_loc(f.get(2)?)?,
        },
        "copy" => Request::Copy {
            id: id()?,
            remove: f.get(2)? == "1",
            to: parse_loc(f.get(3)?)?,
            items: locs(4)?,
        },
        "delete" => Request::Delete {
            id: id()?,
            trash: f.get(2)? == "1",
            items: locs(3)?,
        },
        "rename" => Request::Rename {
            id: id()?,
            item: parse_loc(f.get(2)?)?,
            name: f.get(3)?.clone(),
        },
        "mkdir" => Request::Mkdir {
            id: id()?,
            at: parse_loc(f.get(2)?)?,
            name: f.get(3)?.clone(),
        },
        "extract" => Request::Extract {
            id: id()?,
            archive: parse_loc(f.get(2)?)?,
            to: parse_loc(f.get(3)?)?,
            entries: f.get(4..)?.to_vec(),
        },
        "compress" => Request::Compress {
            id: id()?,
            to: parse_loc(f.get(2)?)?,
            items: locs(3)?,
        },
        "cancel" => Request::Cancel { id: id()? },
        "reply" => Request::Reply {
            id: id()?,
            text: (f.get(2)? == "1").then(|| f.get(3).cloned().unwrap_or_default()),
        },
        "makekey" => Request::MakeKey {
            id: id()?,
            passphrase: f.get(2)?.clone(),
        },
        "read" => Request::Read {
            id: id()?,
            file: parse_loc(f.get(2)?)?,
        },
        _ => return None,
    })
}

// ---- events ----

pub fn entry(id: Id, e: &Entry) -> Vec<String> {
    vec![
        "entry".into(),
        id.to_string(),
        kind_code(e.kind).into(),
        e.size.to_string(),
        e.mtime.to_string(),
        e.mode.to_string(),
        e.name.clone(),
        e.link.clone().unwrap_or_default(),
    ]
}

/// An event's lines: a listing is one line an entry.
pub fn event(e: &Event) -> Vec<Vec<String>> {
    let s = |x: &str| x.to_string();
    match e {
        Event::Entries { id, entries } => entries.iter().map(|x| entry(*id, x)).collect(),
        Event::Listed { id } => vec![vec![s("listed"), id.to_string()]],
        Event::Resolved { id, path } => vec![vec![s("resolved"), id.to_string(), path.clone()]],
        Event::Progress {
            id,
            done,
            total,
            name,
        } => vec![vec![
            s("progress"),
            id.to_string(),
            done.to_string(),
            total.to_string(),
            name.clone(),
        ]],
        Event::Ask { id, ask } => {
            let mut v = vec![s("ask"), id.to_string()];
            match ask {
                Ask::Password { server, retry } => {
                    v.extend([s("password"), server.clone(), b(*retry)])
                }
                Ask::HostKey {
                    server,
                    host,
                    fingerprint,
                } => v.extend([
                    s("hostkey"),
                    server.clone(),
                    host.clone(),
                    fingerprint.clone(),
                ]),
                Ask::Passphrase => v.push(s("passphrase")),
                Ask::Conflict { name } => v.extend([s("conflict"), name.clone()]),
            }
            vec![v]
        }
        Event::Done { id } => vec![vec![s("done"), id.to_string()]],
        Event::Error { id, text } => vec![vec![s("error"), id.to_string(), text.clone()]],
        Event::Text { id, text } => vec![vec![s("text"), id.to_string(), text.clone()]],
        Event::Ready => vec![vec![s("ready"), s("0")]],
    }
}

pub fn parse_event(f: &[String]) -> Option<Event> {
    let id = f.get(1)?.parse::<Id>().ok()?;
    let get = |i: usize| f.get(i).cloned();
    Some(match f.first()?.as_str() {
        "entry" => Event::Entries {
            id,
            entries: vec![Entry {
                kind: parse_kind(f.get(2)?),
                size: f.get(3)?.parse().ok()?,
                mtime: f.get(4)?.parse().ok()?,
                mode: f.get(5)?.parse().ok()?,
                name: get(6)?,
                link: get(7).filter(|l| !l.is_empty()),
            }],
        },
        "listed" => Event::Listed { id },
        "resolved" => Event::Resolved { id, path: get(2)? },
        "progress" => Event::Progress {
            id,
            done: f.get(2)?.parse().ok()?,
            total: f.get(3)?.parse().ok()?,
            name: get(4).unwrap_or_default(),
        },
        "ask" => Event::Ask {
            id,
            ask: match f.get(2)?.as_str() {
                "password" => Ask::Password {
                    server: get(3)?,
                    retry: f.get(4).is_some_and(|r| r == "1"),
                },
                "hostkey" => Ask::HostKey {
                    server: get(3)?,
                    host: get(4)?,
                    fingerprint: get(5)?,
                },
                "passphrase" => Ask::Passphrase,
                "conflict" => Ask::Conflict { name: get(3)? },
                _ => return None,
            },
        },
        "done" => Event::Done { id },
        "error" => Event::Error {
            id,
            text: get(2).unwrap_or_default(),
        },
        "text" => Event::Text {
            id,
            text: get(2).unwrap_or_default(),
        },
        "ready" => Event::Ready,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_modes_messages_round_trip() {
        let r = Request::Read {
            id: 7,
            file: Loc::Local("/etc/os release".into()),
        };
        assert_eq!(parse_request(&request(&r)), Some(r));
        for e in [
            Event::Text {
                id: 7,
                text: "line\n\ttab \u{e9}".into(),
            },
            Event::Ready,
        ] {
            let lines = event(&e);
            assert_eq!(parse_event(&lines[0]), Some(e));
        }
    }

    #[test]
    fn requests_round_trip_with_odd_names() {
        let mut srv = Server::new(Proto::Davs);
        srv.name = "cloud".into();
        srv.host = "dav.example".into();
        let odd = Loc::Local("/home/me/a\tb\nc\\d".into());
        let reqs = [
            Request::Server(srv),
            Request::List {
                id: 3,
                at: Loc::Remote {
                    server: "nas".into(),
                    path: "/x:y".into(),
                },
            },
            Request::Copy {
                id: 4,
                items: vec![odd.clone(), Loc::Local("/b".into())],
                to: Loc::Local("/t".into()),
                remove: true,
            },
            Request::Delete {
                id: 5,
                items: vec![odd.clone()],
                trash: true,
            },
            Request::Extract {
                id: 6,
                archive: odd.clone(),
                entries: vec!["docs/a b.txt".into()],
                to: Loc::Local("/t".into()),
            },
            Request::Reply {
                id: 7,
                text: Some(String::new()),
            },
            Request::Reply { id: 8, text: None },
            Request::MakeKey {
                id: 9,
                passphrase: "p\tq".into(),
            },
        ];
        for r in reqs {
            let line = request(&r);
            // Through escaping, as the wire has it.
            let wire: Vec<String> = line.iter().map(|f| unesc(&esc(f))).collect();
            assert_eq!(parse_request(&wire), Some(r));
        }
    }

    #[test]
    fn events_round_trip() {
        let e = Entry {
            name: "a\tb".into(),
            kind: Kind::Link,
            size: 5,
            mtime: 1_700_000_000,
            mode: 0o777,
            link: Some("target".into()),
        };
        let evs = [
            Event::Entries {
                id: 1,
                entries: vec![e],
            },
            Event::Resolved {
                id: 1,
                path: "/home/me".into(),
            },
            Event::Progress {
                id: 2,
                done: 10,
                total: 20,
                name: "x".into(),
            },
            Event::Ask {
                id: 3,
                ask: Ask::HostKey {
                    server: "nas".into(),
                    host: "nas.local".into(),
                    fingerprint: "SHA256:x".into(),
                },
            },
            Event::Ask {
                id: 3,
                ask: Ask::Password {
                    server: "nas".into(),
                    retry: true,
                },
            },
            Event::Error {
                id: 4,
                text: "no".into(),
            },
        ];
        for ev in evs {
            let lines = event(&ev);
            assert_eq!(lines.len(), 1);
            assert_eq!(parse_event(&lines[0]), Some(ev));
        }
    }

    #[test]
    fn lines_cross_a_socket_pair_whole() {
        let (a, b) = pair().unwrap();
        let (mut a, mut b) = (Conn::new(a), Conn::new(b));
        let big = "x".repeat(6_000);
        a.send(&["one".into(), "t\tab".into()]).unwrap();
        a.send(std::slice::from_ref(&big)).unwrap();
        let mut got = Vec::new();
        for _ in 0..100 {
            got.extend(b.recv());
            if got.len() == 2 {
                break;
            }
        }
        assert_eq!(got[0], ["one", "t\tab"]);
        assert_eq!(got[1], [big]);
        drop(a);
        assert!(b.recv().is_empty());
        assert!(b.eof());
    }
}
