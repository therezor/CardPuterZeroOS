//! The poll loop: the session bus and the shell link. No threads, no
//! timers.

use std::collections::HashMap;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

use gh0st_core::ident::{self, Who};
use gh0st_core::link::Link;
use gh0st_dbus::{Conn, Kind, Message, Value, bus};

use crate::inhibit::{self, Inhibit};
use crate::notify::{self, Notifications};

/// Where the session daemon finds its end of the shell's socket pair.
pub const ENV: &str = "GH0ST_SESSION_FD";

struct Session {
    bus: Conn,
    link: Option<Link>,
    who: HashMap<String, Who>,
    notes: Notifications,
    inhibit: Inhibit,
}

fn take_link() -> Option<Link> {
    let n: RawFd = std::env::var(ENV).ok()?.parse().ok()?;
    // SAFETY: we have one thread and read nothing else from the env.
    unsafe { std::env::remove_var(ENV) };
    // SAFETY: fcntl only checks that the fd is open.
    if n < 3 || unsafe { libc::fcntl(n, libc::F_GETFD) } < 0 {
        return None;
    }
    // SAFETY: the shell handed us this fd and nothing else owns it.
    Link::from_fd(unsafe { OwnedFd::from_raw_fd(n) }).ok()
}

pub fn main(args: &[String]) -> i32 {
    let test = match args {
        [] => false,
        [t] if t == "--test" => true,
        _ => {
            eprintln!("usage: gh0st session [--test]");
            return 2;
        }
    };
    // SAFETY: ignoring SIGPIPE has no preconditions.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let link = if test { None } else { take_link() };
    if !test && link.is_none() {
        eprintln!("gh0st session: no shell link ({ENV}); run by the shell");
        return 2;
    }
    let mut conn = match Conn::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gh0st session: no session bus: {e}");
            return 1;
        }
    };
    for name in [notify::NAME, inhibit::NAME] {
        match bus::own(&mut conn, name) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("gh0st session: {name} is taken");
                return 1;
            }
            Err(e) => {
                eprintln!("gh0st session: can't own {name}: {e}");
                return 1;
            }
        }
    }
    let rule = "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged'";
    if let Err(e) = bus::add_match(&mut conn, rule) {
        eprintln!("gh0st session: {e}");
        return 1;
    }
    let mut s = Session {
        bus: conn,
        link,
        who: HashMap::new(),
        notes: Notifications::new(gh0st_core::notify::user_dir()),
        inhibit: Inhibit::default(),
    };
    s.say(&["ready"]);
    s.run()
}

impl Session {
    fn say(&self, fields: &[&str]) {
        match &self.link {
            Some(l) => {
                l.send(fields);
            }
            None => println!("{}", fields.join(" ")),
        }
    }

    fn run(&mut self) -> i32 {
        loop {
            let mut fds = vec![gh0st_core::rt::pollfd_rw(
                self.bus.fd(),
                true,
                self.bus.wants_write(),
            )];
            if let Some(l) = &self.link {
                fds.push(gh0st_core::rt::pollfd(l.fd()));
            }
            if gh0st_core::rt::wait(&mut fds, None).is_err() {
                continue;
            }
            if fds[0].revents != 0 {
                if let Err(e) = self.bus.flush().and_then(|()| self.bus.fill()) {
                    eprintln!("gh0st session: bus: {e}");
                    return 1;
                }
                if self.bus.closed() {
                    return 1;
                }
            }
            if let Err(e) = self.messages() {
                eprintln!("gh0st session: bus: {e}");
                return 1;
            }
            if let Some(l) = &self.link {
                // The shell says nothing to us yet; a hang-up ends us.
                while l.recv().is_some() {}
                if l.closed() {
                    return 0;
                }
            }
        }
    }

    fn messages(&mut self) -> gh0st_dbus::Result<()> {
        while let Some(m) = self.bus.recv()? {
            match m.kind {
                Kind::Call => self.call(m)?,
                Kind::Signal if m.is(bus::NAME, "NameOwnerChanged") => {
                    let name = m.body.first().and_then(Value::as_str).unwrap_or("");
                    let new = m.body.get(2).and_then(Value::as_str).unwrap_or("x");
                    if new.is_empty() && name.starts_with(':') {
                        self.who.remove(name);
                        self.inhibit.gone(name);
                    }
                }
                _ => {}
            }
            self.flush_out()?;
        }
        Ok(())
    }

    fn call(&mut self, m: Message) -> gh0st_dbus::Result<()> {
        let Some(sender) = m.sender.clone() else {
            return Ok(());
        };
        let who = match self.who.get(&sender) {
            Some(w) => w.clone(),
            None => {
                let creds = bus::credentials(&mut self.bus, &sender).unwrap_or_default();
                let w = creds
                    .pid
                    .and_then(|p| ident::of_pid(p, creds.pidfd.as_ref()))
                    .unwrap_or(Who {
                        app: "exe:unknown".into(),
                        label: "An unknown program".into(),
                    });
                self.who.insert(sender.clone(), w.clone());
                w
            }
        };
        let path = m.path.as_deref().unwrap_or("");
        let reply = if path == notify::PATH {
            self.notes.call(&m, &who, &sender)
        } else if inhibit::PATHS.contains(&path) {
            self.inhibit.call(&m, &who, &sender)
        } else if m.is(bus::INTROSPECTABLE, "Introspect") {
            let child = match path {
                "/" => vec!["org", "ScreenSaver"],
                "/org" => vec!["freedesktop"],
                "/org/freedesktop" => vec!["Notifications", "ScreenSaver"],
                _ => vec![],
            };
            Message::reply(&m).arg(Value::str(bus::introspect(&[], &child)))
        } else {
            Message::error(&m, bus::err::UNKNOWN_OBJECT, "no such object")
        };
        if !m.no_reply() {
            self.bus.send(reply)?;
        }
        Ok(())
    }

    fn flush_out(&mut self) -> gh0st_dbus::Result<()> {
        for s in std::mem::take(&mut self.notes.out) {
            self.bus.send(s)?;
        }
        for (app, on) in std::mem::take(&mut self.inhibit.out) {
            self.say(&["awake", &app, if on { "1" } else { "0" }]);
        }
        Ok(())
    }
}
