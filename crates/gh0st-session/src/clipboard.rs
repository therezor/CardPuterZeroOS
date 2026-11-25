//! `org.gh0st.Clipboard`: one clipboard every app shares. What was copied,
//! by type, just under 1 MiB. Only the app in front, or a program it started,
//! may set it or read it: an app behind can't watch what the user copies.
//! The shell names the app in front; on lock it clears the clipboard
//! unless Settings keeps it.

use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Value};

pub const NAME: &str = "org.gh0st.Clipboard";
pub const PATH: &str = "/org/gh0st/Clipboard";
const IFACE: &str = "org.gh0st.Clipboard";
const EMPTY: &str = "org.gh0st.Clipboard.Error.Empty";
/// All of it, every type together. One message carries it each way, so it
/// leaves room for the header and the fields the bus adds.
const MAX: usize = gh0st_dbus::MAX_MESSAGE - (4 << 10);
const MAX_TYPE: usize = 255;

#[derive(Default)]
pub struct Clipboard {
    items: Vec<(String, Vec<u8>)>,
    /// The process group of the app in front; None at home or locked.
    front: Option<i32>,
    /// `gh0st session --test`: no shell, so anyone may.
    pub open: bool,
    /// Signals to send.
    pub out: Vec<Message>,
}

/// `text/plain` with or without its charset is the same text.
fn same_type(asked: &str, have: &str) -> bool {
    let base = |m: &str| {
        m.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    };
    asked == have || (base(asked) == "text/plain" && base(have) == "text/plain")
}

impl Clipboard {
    /// `open`: anyone may, for `gh0st session --test`.
    pub fn new(open: bool) -> Clipboard {
        Clipboard {
            open,
            ..Clipboard::default()
        }
    }

    fn allowed(&self, pid: Option<i32>) -> bool {
        if self.open {
            return true;
        }
        match (self.front, pid) {
            (Some(g), Some(p)) if p > 0 => gh0st_core::procfs::belongs_to(p, g),
            _ => false,
        }
    }

    fn types(&self) -> Value {
        Value::strs(self.items.iter().map(|(t, _)| t.clone()))
    }

    fn changed(&mut self) {
        let types = self.types();
        self.out
            .push(Message::signal(PATH, IFACE, "Changed").arg(types));
    }

    /// Everything copied goes, its bytes zeroed first.
    fn wipe(&mut self) {
        for (_, data) in &mut self.items {
            gh0st_dbus::wipe(data);
        }
        self.items.clear();
    }

    pub fn call(&mut self, m: &Message, pid: Option<i32>) -> Message {
        if m.is(bus::INTROSPECTABLE, "Introspect") {
            return Message::reply(m).arg(Value::str(bus::introspect(&[XML], &[])));
        }
        if !self.allowed(pid) {
            return Message::error(m, err::ACCESS_DENIED, "only the app in front may");
        }
        let s = |i: usize| m.body.get(i).and_then(Value::as_str).unwrap_or("");
        match m.member.as_deref().unwrap_or("") {
            "Set" => {
                let mime = s(0);
                let Some(data) = m.body.get(1).and_then(Value::as_bytes) else {
                    return Message::error(m, err::INVALID_ARGS, "no data");
                };
                if mime.is_empty() || mime.len() > MAX_TYPE || data.len() > MAX {
                    return Message::error(m, err::LIMITS, "too much to copy");
                }
                self.wipe();
                self.items.push((mime.to_string(), data.to_vec()));
                self.changed();
                Message::reply(m)
            }
            "Get" => match self.items.iter().find(|(t, _)| same_type(s(0), t)) {
                Some((_, data)) => Message::reply(m).arg(Value::Bytes(data.clone())),
                None => Message::error(m, EMPTY, "nothing of that type"),
            },
            "Types" => Message::reply(m).arg(self.types()),
            "Clear" => {
                self.wipe();
                self.changed();
                Message::reply(m)
            }
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// The shell: `front <pgid>` (0 for none), `clipboard-clear` at lock.
    pub fn shell_said(&mut self, fields: &[String]) {
        match fields.first().map(String::as_str) {
            Some("front") => {
                self.front = fields
                    .get(1)
                    .and_then(|g| g.parse::<i32>().ok())
                    .filter(|&g| g > 0);
            }
            Some("clipboard-clear") if !self.items.is_empty() => {
                self.wipe();
                self.changed();
            }
            _ => {}
        }
    }
}

const XML: &str = r#"<interface name="org.gh0st.Clipboard"><method name="Set"><arg name="mime" type="s" direction="in"/><arg name="data" type="ay" direction="in"/></method><method name="Get"><arg name="mime" type="s" direction="in"/><arg name="data" type="ay" direction="out"/></method><method name="Types"><arg type="as" direction="out"/></method><method name="Clear"/><signal name="Changed"><arg name="types" type="as"/></signal></interface>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn call(c: &mut Clipboard, member: &str, body: Vec<Value>, pid: i32) -> Message {
        let mut m = Message::call(NAME, PATH, IFACE, member);
        m.body = body;
        m.serial = 1;
        c.call(&m, Some(pid))
    }

    fn set(data: &[u8]) -> Vec<Value> {
        vec![
            Value::str("text/plain;charset=utf-8"),
            Value::Bytes(data.to_vec()),
        ]
    }

    #[test]
    fn copied_bytes_come_back_by_type_and_a_clear_takes_them() {
        let mut c = Clipboard::new(true);
        assert!(call(&mut c, "Set", set(b"hi \xff"), 1).error_name.is_none());
        assert_eq!(c.out.len(), 1, "Changed");
        let got = call(&mut c, "Get", vec![Value::str("text/plain")], 1);
        assert_eq!(got.body[0], Value::Bytes(b"hi \xff".to_vec()));
        let none = call(&mut c, "Get", vec![Value::str("text/uri-list")], 1);
        assert_eq!(none.error_name.as_deref(), Some(EMPTY));
        c.shell_said(&["clipboard-clear".into()]);
        let gone = call(&mut c, "Get", vec![Value::str("text/plain")], 1);
        assert_eq!(gone.error_name.as_deref(), Some(EMPTY));
        let big = vec![0u8; MAX + 1];
        assert!(
            call(&mut c, "Set", set(&big), 1).error_name.is_some(),
            "too much"
        );
    }

    #[test]
    fn the_most_it_takes_fits_one_message_each_way() {
        let mime = "x".repeat(MAX_TYPE);
        let peer = Some(":1.4294967295".to_string());
        let mut set = Message::call(NAME, PATH, IFACE, "Set")
            .arg(Value::str(mime.clone()))
            .arg(Value::Bytes(vec![7; MAX]));
        set.sender = peer.clone();
        assert!(set.encode(1).unwrap().len() <= gh0st_dbus::MAX_MESSAGE);
        let mut c = Clipboard::new(true);
        set.serial = 1;
        assert!(c.call(&set, Some(1)).error_name.is_none());
        let mut get = Message::call(NAME, PATH, IFACE, "Get").arg(Value::str(mime));
        get.serial = 2;
        get.sender = peer.clone();
        let mut reply = c.call(&get, Some(1));
        assert!(reply.error_name.is_none());
        // The bus adds ours as the sender.
        reply.sender = peer;
        assert!(reply.encode(3).unwrap().len() <= gh0st_dbus::MAX_MESSAGE);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_the_app_in_front_may() {
        let me = std::process::id() as i32;
        let mut c = Clipboard::new(false);
        assert!(
            call(&mut c, "Set", set(b"hi"), me).error_name.is_some(),
            "nothing in front"
        );
        // This process stands for the app in front: it leads the group.
        c.shell_said(&["front".into(), me.to_string()]);
        assert!(call(&mut c, "Set", set(b"hi"), me).error_name.is_none());
        c.shell_said(&["front".into(), "999999".into()]);
        let behind = call(&mut c, "Get", vec![Value::str("text/plain")], me);
        assert!(behind.error_name.is_some(), "an app behind can't read it");
    }
}
