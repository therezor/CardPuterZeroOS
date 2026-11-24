//! `org.freedesktop.ScreenSaver` Inhibit and UnInhibit: the app's `awake`
//! on the shell link, as if it had said it itself.

use std::collections::HashMap;

use gh0st_core::ident::Who;
use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Value};

pub const NAME: &str = "org.freedesktop.ScreenSaver";
pub const PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
pub const IFACE: &str = "org.freedesktop.ScreenSaver";
/// Inhibitors one connection may hold.
const PER_CONN: usize = 16;

struct Hold {
    /// The app's desktop stem; empty for a process outside apps.
    app: String,
    owner: String,
}

#[derive(Default)]
pub struct Inhibit {
    next: u32,
    holds: HashMap<u32, Hold>,
    /// (app, on) for the shell.
    pub out: Vec<(String, bool)>,
}

impl Inhibit {
    fn count(&self, app: &str) -> usize {
        self.holds.values().filter(|h| h.app == app).count()
    }

    pub fn call(&mut self, m: &Message, who: &Who, bus_name: &str) -> Message {
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
                // Only an app the shell runs can be kept in front.
                let app = if who.app.starts_with("exe:") {
                    String::new()
                } else {
                    who.app.clone()
                };
                if !app.is_empty() && self.count(&app) == 0 {
                    self.out.push((app.clone(), true));
                }
                self.next = self.next.wrapping_add(1).max(1);
                self.holds.insert(
                    self.next,
                    Hold {
                        app,
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
            if !h.app.is_empty() && self.count(&h.app) == 0 {
                self.out.push((h.app, false));
            }
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

    fn call(i: &mut Inhibit, member: &str, arg: Option<u32>, who: &Who, bus: &str) -> Message {
        let mut m = Message::call(NAME, PATHS[0], IFACE, member);
        if let Some(c) = arg {
            m = m.arg(Value::U32(c));
        } else {
            m = m.arg(Value::str("mpv")).arg(Value::str("playing"));
        }
        m.serial = 1;
        i.call(&m, who, bus)
    }

    #[test]
    fn inhibit_keeps_the_app_awake_until_the_last_one_goes() {
        let mut i = Inhibit::default();
        let mpv = Who {
            app: "mpv".into(),
            label: "mpv".into(),
        };
        let a = call(&mut i, "Inhibit", None, &mpv, ":1.1").body[0]
            .as_u32()
            .unwrap();
        let b = call(&mut i, "Inhibit", None, &mpv, ":1.1").body[0]
            .as_u32()
            .unwrap();
        assert_eq!(i.out, [("mpv".to_string(), true)]);
        call(&mut i, "UnInhibit", Some(a), &mpv, ":1.2");
        assert_eq!(i.out.len(), 1, "another connection can't release it");
        call(&mut i, "UnInhibit", Some(a), &mpv, ":1.1");
        assert_eq!(i.out.len(), 1);
        call(&mut i, "UnInhibit", Some(b), &mpv, ":1.1");
        assert_eq!(i.out.last(), Some(&("mpv".to_string(), false)));
        call(&mut i, "Inhibit", None, &mpv, ":1.1");
        i.gone(":1.1");
        assert_eq!(i.out.last(), Some(&("mpv".to_string(), false)));
    }

    #[test]
    fn a_process_outside_apps_gets_a_cookie_and_nothing_else() {
        let mut i = Inhibit::default();
        let w = Who {
            app: "exe:/usr/bin/x".into(),
            label: "x".into(),
        };
        assert!(
            call(&mut i, "Inhibit", None, &w, ":1.1").body[0]
                .as_u32()
                .is_some()
        );
        assert!(i.out.is_empty());
    }
}
