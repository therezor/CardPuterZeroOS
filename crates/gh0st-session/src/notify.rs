//! `org.freedesktop.Notifications`: each notification is a notice file in
//! `~/.local/state/gh0st/notify` (`docs/apps.md`, Notifications), so
//! `notify-send` and libnotify show as the shell's toasts.

use std::collections::HashMap;
use std::path::PathBuf;

use gh0st_core::ident::Who;
use gh0st_core::notify::{self as notice, Notice, Tone};
use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Value};

pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const IFACE: &str = "org.freedesktop.Notifications";
/// Notices one app may keep at once; the oldest goes first.
const PER_APP: usize = 8;

struct Live {
    app: String,
    tag: String,
    owner: String,
}

pub struct Notifications {
    dir: PathBuf,
    next: u32,
    live: HashMap<u32, Live>,
    pub out: Vec<Message>,
}

/// Body text without the markup some servers allow.
fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            c if !tag => out.push(if c == '\n' { ' ' } else { c }),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

impl Notifications {
    pub fn new(dir: PathBuf) -> Notifications {
        Notifications {
            dir,
            next: 0,
            live: HashMap::new(),
            out: Vec::new(),
        }
    }

    pub fn call(&mut self, m: &Message, who: &Who, bus_name: &str) -> Message {
        if let Some(r) = bus::peer(m) {
            return r;
        }
        if m.is(bus::INTROSPECTABLE, "Introspect") {
            return Message::reply(m).arg(Value::str(bus::introspect(&[XML], &[])));
        }
        if m.interface.as_deref().is_some_and(|i| i != IFACE) {
            return Message::error(m, err::UNKNOWN_INTERFACE, "no such interface");
        }
        match m.member.as_deref().unwrap_or("") {
            "Notify" => {
                let s = |i: usize| m.body.get(i).and_then(Value::as_str).unwrap_or("");
                let replaces = m.body.get(1).and_then(Value::as_u32).unwrap_or(0);
                let urgency = m
                    .body
                    .get(6)
                    .and_then(|h| h.get("urgency"))
                    .map(Value::inner);
                let tone = match urgency {
                    Some(Value::Byte(2)) => Tone::Critical,
                    _ => Tone::Info,
                };
                let title = if s(3).is_empty() { s(0) } else { s(3) };
                let app = who.short();
                let id = match self.live.get(&replaces) {
                    Some(l) if l.app == app => replaces,
                    _ => {
                        self.next = self.next.wrapping_add(1).max(1);
                        self.next
                    }
                };
                let n = Notice {
                    app: app.clone(),
                    tag: format!("n{id}"),
                    title: plain(title),
                    body: plain(s(4)),
                    tone,
                    time: notice::now_secs(),
                    ..Notice::default()
                };
                if let Err(e) = notice::post(&self.dir, &n) {
                    return Message::error(m, err::FAILED, &e.to_string());
                }
                self.live.insert(
                    id,
                    Live {
                        app: app.clone(),
                        tag: n.tag,
                        owner: bus_name.into(),
                    },
                );
                self.trim(&app);
                Message::reply(m).arg(Value::U32(id))
            }
            "CloseNotification" => {
                let id = m.body.first().and_then(Value::as_u32).unwrap_or(0);
                if self.live.get(&id).is_some_and(|l| l.app == who.short()) {
                    self.close(id, 3);
                }
                Message::reply(m)
            }
            "GetCapabilities" => Message::reply(m).arg(Value::strs(["body"])),
            "GetServerInformation" => Message::reply(m)
                .arg(Value::str("gh0st"))
                .arg(Value::str("Gh0st OS"))
                .arg(Value::str(env!("CARGO_PKG_VERSION")))
                .arg(Value::str("1.2")),
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    fn close(&mut self, id: u32, reason: u32) {
        if let Some(l) = self.live.remove(&id) {
            let _ = notice::remove(&self.dir, &l.app, &l.tag);
            let mut s = Message::signal(PATH, IFACE, "NotificationClosed")
                .arg(Value::U32(id))
                .arg(Value::U32(reason));
            s.destination = Some(l.owner);
            self.out.push(s);
        }
    }

    /// Past [`PER_APP`], the oldest of an app's notices go.
    fn trim(&mut self, app: &str) {
        let mut ids: Vec<u32> = self
            .live
            .iter()
            .filter(|(_, l)| l.app == app)
            .map(|(id, _)| *id)
            .collect();
        if ids.len() <= PER_APP {
            return;
        }
        ids.sort_unstable();
        for id in &ids[..ids.len() - PER_APP] {
            self.close(*id, 1);
        }
    }
}

const XML: &str = r#" <interface name="org.freedesktop.Notifications">
  <method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/><arg type="a{sv}" direction="in"/><arg type="i" direction="in"/><arg type="u" direction="out"/></method>
  <method name="CloseNotification"><arg type="u" direction="in"/></method>
  <method name="GetCapabilities"><arg type="as" direction="out"/></method>
  <method name="GetServerInformation"><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/><arg type="s" direction="out"/></method>
  <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
  <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
 </interface>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn notify(
        n: &mut Notifications,
        who: &Who,
        replaces: u32,
        summary: &str,
        body: &str,
    ) -> Message {
        let mut m = Message::call(NAME, PATH, IFACE, "Notify")
            .arg(Value::str("app"))
            .arg(Value::U32(replaces))
            .arg(Value::str(""))
            .arg(Value::str(summary))
            .arg(Value::str(body))
            .arg(Value::strs::<&str>([]))
            .arg(Value::props([("urgency", Value::Byte(2))]))
            .arg(Value::I32(-1));
        m.serial = 1;
        m.sender = Some(":1.4".into());
        n.call(&m, who, ":1.4")
    }

    #[test]
    fn a_notification_is_a_notice_file() {
        let dir = std::env::temp_dir().join(format!("gh0st-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut n = Notifications::new(dir.clone());
        let who = Who {
            app: "exe:/usr/bin/notify-send".into(),
            label: "notify-send".into(),
        };
        let r = notify(&mut n, &who, 0, "Build done", "<b>all</b> &amp; green");
        let id = r.body[0].as_u32().unwrap();
        let text = std::fs::read_to_string(dir.join(format!("notify-send.n{id}"))).unwrap();
        assert!(text.contains("title=Build done") && text.contains("body=all & green"));
        assert!(text.contains("tone=critical"));
        // Replacing keeps the id and the file.
        let r = notify(&mut n, &who, id, "Build failed", "");
        assert_eq!(r.body[0].as_u32(), Some(id));
        let text = std::fs::read_to_string(dir.join(format!("notify-send.n{id}"))).unwrap();
        assert!(text.contains("title=Build failed"));
        // Another app can't close it.
        let other = Who {
            app: "chromium".into(),
            label: "Chromium".into(),
        };
        let mut close = Message::call(NAME, PATH, IFACE, "CloseNotification").arg(Value::U32(id));
        close.serial = 2;
        n.call(&close, &other, ":1.5");
        assert!(dir.join(format!("notify-send.n{id}")).exists());
        n.call(&close, &who, ":1.4");
        assert!(!dir.join(format!("notify-send.n{id}")).exists());
        assert!(n.out.iter().any(|s| s.is(IFACE, "NotificationClosed")));
        // A flood keeps only the newest few.
        for i in 0..20 {
            notify(&mut n, &who, 0, &format!("spam {i}"), "");
        }
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count, PER_APP);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
