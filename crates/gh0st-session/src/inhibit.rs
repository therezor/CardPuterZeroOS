//! `org.freedesktop.ScreenSaver` Inhibit and UnInhibit: the app's `awake`
//! on the shell link, as if it had said it itself.

use std::collections::{HashMap, HashSet};

use gh0st_core::ident::Who;
use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Value};

pub const NAME: &str = "org.freedesktop.ScreenSaver";
pub const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
pub const IFACE: &str = "org.freedesktop.ScreenSaver";
/// Inhibitors one connection may hold.
const PER_CONN: usize = 16;

struct Hold {
    /// What it keeps awake ([`key`]); None holds nothing.
    key: Option<String>,
    owner: String,
}

/// What a caller's inhibitors keep awake, as the shell names it: the app,
/// or for a program outside apps `pid:N`. N is the process under the shell
/// `pid` runs in, the app that started it, so all of a task's processes
/// hold one thing. The shell finds the task by it.
pub fn key(who: &Who, pid: Option<i32>, shell: i32) -> Option<String> {
    if !who.app.starts_with("exe:") {
        return Some(who.app.clone());
    }
    let p = pid.filter(|&p| p > 0)?;
    let top = gh0st_core::procfs::child_of(p, shell).unwrap_or(p);
    Some(format!("pid:{top}"))
}

/// What the shell was told is awake. It keeps one flag a task, so it hears
/// a key only as the first of its holds starts or the last ends, from
/// ScreenSaver and the portal together.
#[derive(Default)]
pub struct Told(HashSet<String>);

impl Told {
    /// `key` now has `held` holds: the word for the shell, if it changed.
    pub fn update(&mut self, key: &str, held: usize) -> Option<bool> {
        let on = held > 0;
        if on == self.0.contains(key) {
            return None;
        }
        if on {
            self.0.insert(key.to_string());
        } else {
            self.0.remove(key);
        }
        Some(on)
    }
}

#[derive(Default)]
pub struct Inhibit {
    next: u32,
    holds: HashMap<u32, Hold>,
    /// Keys whose holds changed, for the session to tell the shell.
    pub out: Vec<String>,
}

impl Inhibit {
    /// Holds that keep `key` awake.
    pub fn holds(&self, key: &str) -> usize {
        self.holds
            .values()
            .filter(|h| h.key.as_deref() == Some(key))
            .count()
    }

    /// A call from `bus_name`, whose inhibitors keep `awake` awake.
    pub fn call(&mut self, m: &Message, bus_name: &str, awake: Option<&str>) -> Message {
        if let Some(r) = bus::peer(m) {
            return r;
        }
        if m.is(bus::INTROSPECTABLE, "Introspect") {
            return Message::reply(m).arg(Value::str(bus::introspect(&[XML], &[])));
        }
        match m.member.as_deref().unwrap_or("") {
            "Inhibit" => {
                if self.holds.values().filter(|h| h.owner == bus_name).count() >= PER_CONN {
                    return Message::error(m, err::LIMITS, "too many inhibitors");
                }
                let key = awake.map(String::from);
                self.out.extend(key.clone());
                self.next = self.next.wrapping_add(1).max(1);
                self.holds.insert(
                    self.next,
                    Hold {
                        key,
                        owner: bus_name.into(),
                    },
                );
                Message::reply(m).arg(Value::U32(self.next))
            }
            "UnInhibit" => {
                let cookie = m.body.first().and_then(Value::as_u32).unwrap_or(0);
                if self.holds.get(&cookie).is_some_and(|h| h.owner == bus_name) {
                    self.release(cookie);
                }
                Message::reply(m)
            }
            "GetActive" => Message::reply(m).arg(Value::Bool(false)),
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    fn release(&mut self, cookie: u32) {
        if let Some(h) = self.holds.remove(&cookie) {
            self.out.extend(h.key);
        }
    }

    /// A connection left: its inhibitors go with it.
    pub fn gone(&mut self, bus_name: &str) {
        let mine: Vec<u32> = self
            .holds
            .iter()
            .filter(|(_, h)| h.owner == bus_name)
            .map(|(c, _)| *c)
            .collect();
        for c in mine {
            self.release(c);
        }
    }
}

const XML: &str = r#" <interface name="org.freedesktop.ScreenSaver">
  <method name="Inhibit"><arg name="application_name" type="s" direction="in"/><arg name="reason_for_inhibit" type="s" direction="in"/><arg name="cookie" type="u" direction="out"/></method>
  <method name="UnInhibit"><arg name="cookie" type="u" direction="in"/></method>
  <method name="GetActive"><arg type="b" direction="out"/></method>
 </interface>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn call(i: &mut Inhibit, member: &str, arg: Option<u32>, bus: &str) -> Message {
        let mut m = Message::call(NAME, PATHS[0], IFACE, member);
        if let Some(c) = arg {
            m = m.arg(Value::U32(c));
        } else {
            m = m.arg(Value::str("mpv")).arg(Value::str("playing"));
        }
        m.serial = 1;
        i.call(&m, bus, Some("mpv"))
    }

    #[test]
    fn inhibit_holds_until_its_owner_lets_go() {
        let mut i = Inhibit::default();
        let a = call(&mut i, "Inhibit", None, ":1.1").body[0]
            .as_u32()
            .unwrap();
        let b = call(&mut i, "Inhibit", None, ":1.1").body[0]
            .as_u32()
            .unwrap();
        assert_eq!(i.holds("mpv"), 2);
        call(&mut i, "UnInhibit", Some(a), ":1.2");
        assert_eq!(i.holds("mpv"), 2, "another connection can't release it");
        call(&mut i, "UnInhibit", Some(a), ":1.1");
        call(&mut i, "UnInhibit", Some(b), ":1.1");
        assert_eq!(i.holds("mpv"), 0);
        call(&mut i, "Inhibit", None, ":1.1");
        i.gone(":1.1");
        assert_eq!(i.holds("mpv"), 0);
        assert_eq!(i.out.len(), 6, "every hold and release is heard");
    }

    #[test]
    fn a_program_outside_apps_is_kept_awake_by_its_pid() {
        let w = Who {
            app: "exe:/usr/bin/x".into(),
            label: "x".into(),
            installed: true,
        };
        // No such parent: the program stands for itself.
        assert_eq!(key(&w, Some(77), -1).as_deref(), Some("pid:77"));
        assert_eq!(key(&w, None, -1), None);
        let app = Who {
            app: "mpv".into(),
            ..w
        };
        assert_eq!(key(&app, None, -1).as_deref(), Some("mpv"));
    }

    #[test]
    fn the_shell_hears_the_first_hold_and_the_last_release_of_both() {
        use crate::portal::{INHIBIT, Portal};
        let mut screensaver = Inhibit::default();
        let mut portal = Portal::default();
        let mut told = Told::default();
        let mut said = Vec::new();
        let mut tell = |s: &mut Inhibit, p: &mut Portal| {
            let keys: Vec<String> = s.out.drain(..).chain(p.touched.drain(..)).collect();
            for k in keys {
                if let Some(on) = told.update(&k, s.holds(&k) + p.holds(&k)) {
                    said.push((k, on));
                }
            }
        };
        let cookie = call(&mut screensaver, "Inhibit", None, ":1.1").body[0]
            .as_u32()
            .unwrap();
        let mut m = Message::call(crate::portal::NAME, crate::portal::PATH, INHIBIT, "Inhibit")
            .arg(Value::str(""))
            .arg(Value::U32(8))
            .arg(Value::props::<&str>([]));
        m.serial = 2;
        let w = Who {
            app: "mpv".into(),
            label: "mpv".into(),
            installed: false,
        };
        portal.call(&m, ":1.1", Some(5), &w, Some("mpv"));
        tell(&mut screensaver, &mut portal);
        call(&mut screensaver, "UnInhibit", Some(cookie), ":1.1");
        tell(&mut screensaver, &mut portal);
        portal.gone(":1.1");
        tell(&mut screensaver, &mut portal);
        assert_eq!(
            said,
            [("mpv".to_string(), true), ("mpv".to_string(), false)],
            "one release leaves the other hold"
        );
    }
}
