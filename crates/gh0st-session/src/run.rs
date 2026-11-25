//! The poll loop: the session bus and the shell link. No threads, no
//! timers.

use std::collections::HashMap;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

use gh0st_core::ident::{self, Who};
use gh0st_core::link::Link;
use gh0st_dbus::{Conn, Kind, Message, Value, bus};

use crate::clipboard::{self, Clipboard};
use crate::inhibit::{self, Inhibit};
use crate::memory::{self, Memory};
use crate::monitor::{self, Network, Power};
use crate::notify::{self, Notifications};
use crate::portal::{self, Portal};
use crate::settings::{self, Settings};

/// Where the session daemon finds its end of the shell's socket pair.
pub const ENV: &str = "GH0ST_SESSION_FD";

/// A caller as the bus first named it.
struct Caller {
    who: Who,
    pid: Option<i32>,
    /// Says whether `pid` is still that process.
    pidfd: Option<OwnedFd>,
    /// What its inhibitors keep awake.
    awake: Option<String>,
}

struct Session {
    bus: Conn,
    link: Option<Link>,
    /// Each caller once.
    who: HashMap<String, Caller>,
    /// The shell: our parent, and every app's.
    shell: i32,
    /// What the shell was told is awake.
    told: inhibit::Told,
    notes: Notifications,
    inhibit: Inhibit,
    portal: Portal,
    settings: Settings,
    clipboard: Clipboard,
    memory: Memory,
    network: Network,
    power: Power,
    /// The CPU mode's rule folder, for Power Saver.
    #[cfg(target_os = "linux")]
    rules_watch: Option<gh0st_core::status::Watch>,
    /// config.json's folder, for SettingChanged.
    #[cfg(target_os = "linux")]
    config_watch: Option<gh0st_core::status::Watch>,
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
    for name in [notify::NAME, inhibit::NAME, portal::NAME, clipboard::NAME] {
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
    let config = gh0st_core::config::default_path();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut portal = Portal::default();
    portal.remember = home
        .as_ref()
        .map(|h| h.join(".local/state/gh0st/portal-always"));
    portal.config = config.clone();
    portal.scratch = home
        .map(|h| h.join(".cache/gh0st"))
        .unwrap_or_else(std::env::temp_dir);
    portal.settings_bin = std::env::var_os("GH0ST_SETTINGS")
        .unwrap_or_else(|| "/usr/bin/gh0st-settings".into())
        .into();
    portal.open = test;
    portal.seed_ids();
    let mut s = Session {
        bus: conn,
        link,
        who: HashMap::new(),
        // SAFETY: getppid has no preconditions.
        shell: unsafe { libc::getppid() },
        told: inhibit::Told::default(),
        notes: Notifications::new(gh0st_core::notify::user_dir()),
        inhibit: Inhibit::default(),
        portal,
        settings: Settings::new(&config),
        clipboard: Clipboard::new(test),
        memory: Memory::open(),
        network: Network::default(),
        power: Power::new(),
        #[cfg(target_os = "linux")]
        rules_watch: gh0st_core::status::Watch::dirs(&[std::path::Path::new(monitor::RULES)]).ok(),
        #[cfg(target_os = "linux")]
        config_watch: config
            .parent()
            .and_then(|d| gh0st_core::status::Watch::dirs(&[d]).ok()),
    };
    s.say(&["ready"]);
    s.run()
}

impl Session {
    /// A line for the shell; false if it didn't go.
    fn say(&self, fields: &[&str]) -> bool {
        match &self.link {
            Some(l) => l.send(fields),
            None => {
                println!("{}", fields.join(" "));
                true
            }
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
            let net_at = self.network.fd().map(|fd| {
                fds.push(gh0st_core::rt::pollfd_rw(
                    fd,
                    true,
                    self.network.wants_write(),
                ));
                fds.len() - 1
            });
            #[cfg(target_os = "linux")]
            let rules_at = self.rules_watch.as_ref().map(|w| {
                fds.push(gh0st_core::rt::pollfd(w.fd()));
                fds.len() - 1
            });
            let pressure_at = fds.len();
            fds.extend(self.memory.pollfds());
            let job_at = self.portal.job_fd().map(|fd| {
                fds.push(gh0st_core::rt::pollfd(fd));
                fds.len() - 1
            });
            #[cfg(target_os = "linux")]
            let watch_at = self.config_watch.as_ref().map(|w| {
                fds.push(gh0st_core::rt::pollfd(w.fd()));
                fds.len() - 1
            });
            if gh0st_core::rt::wait(&mut fds, None).is_err() {
                continue;
            }
            let fired: Vec<bool> = fds[pressure_at..pressure_at + self.memory.len()]
                .iter()
                .map(|p| p.revents & libc::POLLPRI != 0)
                .collect();
            if let Some(level) = self.memory.level(&fired) {
                self.low_memory(level);
            }
            if net_at.is_some_and(|i| fds[i].revents != 0) {
                self.network.ready();
            }
            #[cfg(target_os = "linux")]
            if let (Some(i), Some(w)) = (rules_at, self.rules_watch.as_ref()) {
                if fds[i].revents != 0 && w.drain() {
                    self.power.changed();
                }
            }
            if job_at.is_some_and(|i| fds[i].revents != 0) {
                self.portal.job_done();
            }
            #[cfg(target_os = "linux")]
            if let (Some(i), Some(w)) = (watch_at, self.config_watch.as_mut()) {
                if fds[i].revents != 0 && w.drain() {
                    self.settings.changed();
                }
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
            // The shell first: a call is judged against the app in front now.
            if !self.shell_lines() {
                return 0;
            }
            if let Err(e) = self.messages() {
                eprintln!("gh0st session: bus: {e}");
                return 1;
            }
            if let Err(e) = self.flush_out() {
                eprintln!("gh0st session: bus: {e}");
                return 1;
            }
        }
    }

    fn messages(&mut self) -> gh0st_dbus::Result<()> {
        while let Some(m) = self.bus.recv()? {
            match m.kind {
                Kind::Call => self.call(m)?,
                Kind::Signal
                    if m.is(bus::NAME, "NameOwnerChanged")
                        && m.sender.as_deref() == Some(bus::NAME) =>
                {
                    let name = m.body.first().and_then(Value::as_str).unwrap_or("");
                    let new = m.body.get(2).and_then(Value::as_str).unwrap_or("x");
                    if new.is_empty() && name.starts_with(':') {
                        self.who.remove(name);
                        self.inhibit.gone(name);
                        self.portal.gone(name);
                        self.notes.gone(name);
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
        let (who, pid, awake) = self.caller(&sender);
        let path = m.path.as_deref().unwrap_or("");
        let reply = if path == portal::OURS_PATH {
            // The shell names the chooser it started before the chooser can
            // ask: read what it said first.
            self.shell_lines();
            self.portal.chooser_call(&m, pid)
        } else if path == portal::PATH && m.interface.as_deref() == Some(notify::PORTAL_IFACE) {
            let r = self.notes.portal_call(&m, &who, &sender);
            self.test_press();
            r
        } else if path == portal::PATH && m.interface.as_deref() == Some(monitor::NETWORK) {
            self.network.call(&m)
        } else if path == portal::PATH
            && (m.is(bus::PROPERTIES, "Get") || m.is(bus::PROPERTIES, "GetAll"))
            && m.body.first().and_then(Value::as_str) == Some(monitor::POWER)
        {
            self.power.properties(&m)
        } else if path == portal::PATH && m.interface.as_deref() == Some(settings::IFACE) {
            self.settings.call(&m)
        } else if path.starts_with(portal::PATH) {
            self.portal.call(&m, &sender, pid, &who, awake.as_deref())
        } else if path == clipboard::PATH {
            self.clipboard.call(&m, pid)
        } else if path == notify::PATH {
            let r = self.notes.call(&m, &who, &sender);
            self.test_press();
            r
        } else if inhibit::PATHS.contains(&path) {
            self.inhibit.call(&m, &sender, awake.as_deref())
        } else if m.is(bus::INTROSPECTABLE, "Introspect") {
            let child = match path {
                "/" => vec!["org", "ScreenSaver"],
                "/org" => vec!["freedesktop", "gh0st"],
                "/org/gh0st" => vec!["Clipboard"],
                "/org/freedesktop" => vec!["Notifications", "ScreenSaver", "portal"],
                "/org/freedesktop/portal" => vec!["desktop"],
                _ => vec![],
            };
            Message::reply(&m).arg(Value::str(bus::introspect(&[], &child)))
        } else {
            Message::error(&m, bus::err::UNKNOWN_OBJECT, "no such object")
        };
        if !m.no_reply() {
            match self.bus.send(reply) {
                // A reply we couldn't build is that call's loss alone.
                Err(gh0st_dbus::Error::Invalid(why)) => {
                    eprintln!("gh0st session: dropped a reply: {why}")
                }
                r => {
                    r?;
                }
            }
        }
        Ok(())
    }

    /// Who `sender` is, its pid while that is still the process the bus
    /// named, and what its inhibitors keep awake. A lookup that failed is
    /// not kept: the next call asks again.
    fn caller(&mut self, sender: &str) -> (Who, Option<i32>, Option<String>) {
        if let Some(c) = self.who.get(sender) {
            let live = c.pidfd.as_ref().is_none_or(ident::alive);
            return (c.who.clone(), c.pid.filter(|_| live), c.awake.clone());
        }
        let unknown = Who {
            app: "exe:unknown".into(),
            label: "An unknown program".into(),
            installed: false,
        };
        let Ok(creds) = bus::credentials(&mut self.bus, sender) else {
            return (unknown, None, None);
        };
        let Some(p) = creds.pid else {
            return (unknown, None, None);
        };
        let pidfd = creds.pidfd.or_else(|| ident::pidfd_open(p));
        let Some(who) = ident::of_pid(p, pidfd.as_ref()) else {
            return (unknown, None, None);
        };
        let pid = i32::try_from(p).ok();
        let awake = inhibit::key(&who, pid, self.shell);
        let c = Caller {
            who: who.clone(),
            pid,
            pidfd,
            awake: awake.clone(),
        };
        self.who.insert(sender.to_string(), c);
        (who, pid, awake)
    }

    /// Memory is short: every listener on the bus hears it, and the shell
    /// tells its apps.
    fn low_memory(&mut self, level: u8) {
        if let Err(e) = self.bus.send(memory::warning(level)) {
            eprintln!("gh0st session: bus: {e}");
        }
        self.say(&["lowmem", &level.to_string()]);
    }

    /// Under `--test` there is no shell: `GH0ST_NOTIFY_PRESS` presses the
    /// first chip of each new notice, as a user in the panel would.
    fn test_press(&mut self) {
        if self.link.is_some() || std::env::var_os("GH0ST_NOTIFY_PRESS").is_none() {
            return;
        }
        if let Some((nid, chip)) = self.notes.last_chip() {
            self.notes.shell_said(&["action".into(), nid, chip]);
        }
    }

    /// Under `--test` there is no shell: `GH0ST_PORTAL_ANSWER` names the
    /// file every chooser picks, and every open succeeds.
    fn test_answer(&mut self, f: &[String]) {
        if self.link.is_some() {
            return;
        }
        // Or the real chooser, run as the shell would, under --sim with
        // keys from GH0ST_PORTAL_KEYS.
        if let (Some(exe), Some(keys), Some(["pick", id, _])) = (
            std::env::var_os("GH0ST_PORTAL_CHOOSER"),
            std::env::var_os("GH0ST_PORTAL_KEYS"),
            f.iter().map(String::as_str).collect::<Vec<_>>().get(..3),
        ) {
            let frames = std::env::temp_dir().join(format!("gh0st-chooser-{id}"));
            let _ = std::fs::create_dir_all(&frames);
            let child = std::fs::File::open(&keys).and_then(|k| {
                std::process::Command::new(&exe)
                    .args(["--pick", id, "--sim"])
                    .arg(&frames)
                    .stdin(k)
                    .spawn()
            });
            match child {
                Ok(c) => {
                    self.portal
                        .shell_said(&["picker".into(), id.to_string(), c.id().to_string()])
                }
                Err(e) => eprintln!("gh0st session: chooser: {e}"),
            }
            return;
        }
        let Some(answer) = std::env::var_os("GH0ST_PORTAL_ANSWER") else {
            return;
        };
        // GH0ST_PORTAL_APPROVE answers the user's yes or no questions.
        // A screenshot is the GH0ST_PORTAL_ANSWER file; a colour is orange.
        if let (Some(verb @ ("account" | "wallpaper" | "shot" | "color")), Some(id), Some(a)) = (
            f.first().map(String::as_str),
            f.get(1),
            std::env::var_os("GH0ST_PORTAL_APPROVE"),
        ) {
            let a = a.to_string_lossy().into_owned();
            let mut said = vec![verb.to_string(), id.clone()];
            match verb {
                "color" => said.extend(["ok", "255", "128", "0"].map(String::from)),
                "shot" => {
                    let file = std::env::var_os("GH0ST_PORTAL_ANSWER").unwrap_or_default();
                    said.push(a);
                    said.push(gh0st_core::link::path_field(std::path::Path::new(&file)));
                }
                _ => said.push(a),
            }
            self.portal.shell_said(&said);
            return;
        }
        if let (Some(verb), Some(id)) = (f.first(), f.get(1).and_then(|i| i.parse().ok())) {
            if verb == "pick" || verb == "open" {
                self.portal
                    .answer_for_test(id, std::path::Path::new(&answer));
            }
        }
    }

    /// What the shell said: the portal's answers. False once it hung up.
    fn shell_lines(&mut self) -> bool {
        let Some(l) = &self.link else {
            return true;
        };
        let mut lines = Vec::new();
        while let Some(m) = l.recv() {
            lines.push(m);
        }
        let open = !l.closed();
        for m in lines {
            match m.first().map(String::as_str) {
                Some("action" | "closed") => self.notes.shell_said(&m),
                Some("front") => {
                    self.clipboard.shell_said(&m);
                    self.portal.shell_said(&m);
                }
                Some("clipboard-clear") => self.clipboard.shell_said(&m),
                _ => self.portal.shell_said(&m),
            }
        }
        open
    }

    fn flush_out(&mut self) -> gh0st_dbus::Result<()> {
        for s in std::mem::take(&mut self.notes.out) {
            self.bus.send(s)?;
        }
        for s in std::mem::take(&mut self.settings.out) {
            self.bus.send(s)?;
        }
        for s in std::mem::take(&mut self.clipboard.out) {
            self.bus.send(s)?;
        }
        let monitors = std::mem::take(&mut self.network.out)
            .into_iter()
            .chain(std::mem::take(&mut self.power.out));
        for s in monitors {
            self.bus.send(s)?;
        }
        // Answers can make more answers (a test's), so drain until quiet.
        while !self.portal.out.is_empty() {
            for o in std::mem::take(&mut self.portal.out) {
                match o {
                    portal::Out::Bus(m) => {
                        self.bus.send(m)?;
                    }
                    portal::Out::Shell(f) => {
                        let fields: Vec<&str> = f.iter().map(String::as_str).collect();
                        if !self.say(&fields) {
                            self.portal.unsent(&f);
                        }
                        self.test_answer(&f);
                    }
                }
            }
        }
        let touched: Vec<String> = std::mem::take(&mut self.inhibit.out)
            .into_iter()
            .chain(std::mem::take(&mut self.portal.touched))
            .collect();
        for k in touched {
            let held = self.inhibit.holds(&k) + self.portal.holds(&k);
            if let Some(on) = self.told.update(&k, held) {
                self.say(&["awake", &k, if on { "1" } else { "0" }]);
            }
        }
        Ok(())
    }
}
