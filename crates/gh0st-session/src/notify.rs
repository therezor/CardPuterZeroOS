//! `org.freedesktop.Notifications` and the portal's
//! `org.freedesktop.portal.Notification`: each notification is a notice
//! file in `~/.local/state/gh0st/notify` (`docs/apps.md`, Notifications),
//! so `notify-send`, libnotify and portal clients show as the shell's
//! toasts. Their actions are chips in the notifications panel; the shell
//! says which was picked, and the poster hears it as a signal.

use std::collections::HashMap;
use std::path::PathBuf;

use gh0st_core::ident::Who;
use gh0st_core::notify::{self as notice, Notice, Tone};
use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Message, Value};

pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const IFACE: &str = "org.freedesktop.Notifications";
/// The portal's interface, on the portal's object.
pub const PORTAL_IFACE: &str = "org.freedesktop.portal.Notification";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// Notices one app may keep at once; the oldest goes first.
const PER_APP: usize = 8;

struct Live {
    app: String,
    tag: String,
    owner: String,
    /// The poster's action names, by the ids the notice file uses.
    actions: Vec<(String, String)>,
    /// A portal notification: its id, and each action's target.
    portal: Option<(String, Targets)>,
}

/// A portal notification's actions, each with the target it goes back with.
type Targets = Vec<(String, Option<Value>)>;

/// Lowercase letters and digits of `s`: a tag from a portal id.
fn tag_of(prefix: &str, s: &str) -> String {
    let mut h: u32 = 2_166_136_261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    format!("{prefix}{h:08x}")
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
                // [key, label, key, label...]: "default" is Enter's, the
                // rest are chips.
                let pairs: Vec<&str> = m
                    .body
                    .get(5)
                    .and_then(Value::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                let mut actions = Vec::new();
                let mut chips = Vec::new();
                let mut default = String::new();
                for kv in pairs.chunks(2) {
                    let (key, label) = (kv[0], kv.get(1).copied().unwrap_or(""));
                    if key == "default" {
                        default = "default".into();
                        actions.push(("default".to_string(), key.to_string()));
                    } else if chips.len() < notice::MAX_ACTIONS && !label.is_empty() {
                        let id = format!("a{}", chips.len());
                        chips.push((id.clone(), plain(label)));
                        actions.push((id, key.to_string()));
                    }
                }
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
                    actions: chips,
                    default,
                    via_bus: true,
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
                        actions,
                        portal: None,
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
            "GetCapabilities" => {
                Message::reply(m).arg(Value::strs(["body", "actions", "persistence"]))
            }
            "GetServerInformation" => Message::reply(m)
                .arg(Value::str("gh0st"))
                .arg(Value::str("Gh0st OS"))
                .arg(Value::str(env!("CARGO_PKG_VERSION")))
                .arg(Value::str("1.2")),
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// `org.freedesktop.portal.Notification` from `who`, on the bus as
    /// `bus_name`: AddNotification and RemoveNotification.
    pub fn portal_call(&mut self, m: &Message, who: &Who, bus_name: &str) -> Message {
        let app = who.short();
        let pid = m
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if pid.is_empty() || pid.len() > 256 {
            return Message::error(m, err::INVALID_ARGS, "a notification needs an id");
        }
        let tag = tag_of("p", &pid);
        match m.member.as_deref().unwrap_or("") {
            "AddNotification" => {
                let o = m.body.get(1);
                let get = |k: &str| o.and_then(|o| o.get(k)).map(Value::inner);
                let text = |k: &str| get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let body = match get("markup-body").and_then(Value::as_str) {
                    Some(b) => plain(b),
                    None => text("body"),
                };
                let tone = match text("priority").as_str() {
                    "urgent" => Tone::Critical,
                    "high" => Tone::Warning,
                    _ => Tone::Info,
                };
                // Each button: (id in the file, the app's action, target).
                let mut chips = Vec::new();
                let mut targets = Vec::new();
                for b in get("buttons").and_then(Value::as_array).unwrap_or(&[]) {
                    let label = b.get("label").map(Value::inner).and_then(Value::as_str);
                    let action = b.get("action").map(Value::inner).and_then(Value::as_str);
                    if let (Some(l), Some(a)) = (label, action) {
                        if chips.len() < notice::MAX_ACTIONS {
                            let id = format!("a{}", chips.len());
                            chips.push((id.clone(), plain(l)));
                            targets.push((a.to_string(), b.get("target").cloned()));
                        }
                    }
                }
                let mut default = String::new();
                if let Some(a) = get("default-action").and_then(Value::as_str) {
                    default = "default".into();
                    targets.push((
                        a.to_string(),
                        o.and_then(|o| o.get("default-action-target")).cloned(),
                    ));
                }
                let n = Notice {
                    app: app.clone(),
                    tag: tag.clone(),
                    title: text("title"),
                    body,
                    tone,
                    time: notice::now_secs(),
                    actions: chips,
                    default,
                    via_bus: true,
                    ..Notice::default()
                };
                if n.title.is_empty() {
                    return Message::error(m, err::INVALID_ARGS, "a notification needs a title");
                }
                if let Err(e) = notice::post(&self.dir, &n) {
                    return Message::error(m, err::FAILED, &e.to_string());
                }
                // The same id again replaces it.
                let old: Vec<u32> = self
                    .live
                    .iter()
                    .filter(|(_, l)| l.app == app && l.tag == tag)
                    .map(|(i, _)| *i)
                    .collect();
                for i in old {
                    self.live.remove(&i);
                }
                self.next = self.next.wrapping_add(1).max(1);
                // The file's ids: a0.. for buttons, "default" for Enter.
                let ids = (0..targets.len()).map(|i| {
                    if i < n.actions.len() {
                        format!("a{i}")
                    } else {
                        "default".into()
                    }
                });
                let actions: Vec<(String, String)> =
                    ids.zip(targets.iter().map(|(a, _)| a.clone())).collect();
                self.live.insert(
                    self.next,
                    Live {
                        app: app.clone(),
                        tag,
                        owner: bus_name.into(),
                        actions,
                        portal: Some((pid, targets)),
                    },
                );
                self.trim(&app);
                Message::reply(m)
            }
            "RemoveNotification" => {
                let id = self
                    .live
                    .iter()
                    .find(|(_, l)| l.app == app && l.tag == tag)
                    .map(|(i, _)| *i);
                if let Some(id) = id {
                    self.close(id, 3);
                }
                Message::reply(m)
            }
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// The shell: `action <app.tag> <id>` was picked, or `closed
    /// <app.tag>` when the user cleared it.
    pub fn shell_said(&mut self, fields: &[String]) {
        let (Some(verb), Some(nid)) = (fields.first(), fields.get(1)) else {
            return;
        };
        let Some((app, tag)) = nid.split_once('.') else {
            return;
        };
        let Some((&id, live)) = self.live.iter().find(|(_, l)| l.app == app && l.tag == tag) else {
            return;
        };
        match verb.as_str() {
            "action" => {
                let Some(which) = fields.get(2) else {
                    return;
                };
                let Some(at) = live.actions.iter().position(|(i, _)| i == which) else {
                    return;
                };
                if live.owner.is_empty() {
                    return;
                }
                let key = live.actions[at].1.clone();
                let mut s = match &live.portal {
                    Some((pid, targets)) => {
                        let target = targets.get(at).and_then(|(_, t)| t.clone());
                        Message::signal(PORTAL_PATH, PORTAL_IFACE, "ActionInvoked")
                            .arg(Value::str(pid.clone()))
                            .arg(Value::str(key))
                            .arg(Value::Array(
                                gh0st_dbus::Ty::Variant,
                                target.into_iter().map(Value::variant).collect(),
                            ))
                    }
                    None => Message::signal(PATH, IFACE, "ActionInvoked")
                        .arg(Value::U32(id))
                        .arg(Value::str(key)),
                };
                s.destination = Some(live.owner.clone());
                self.out.push(s);
            }
            "closed" => self.close(id, 2),
            _ => {}
        }
    }

    fn close(&mut self, id: u32, reason: u32) {
        if let Some(l) = self.live.remove(&id) {
            let _ = notice::remove(&self.dir, &l.app, &l.tag);
            // The portal has no closed signal.
            if l.portal.is_some() {
                return;
            }
            let mut s = Message::signal(PATH, IFACE, "NotificationClosed")
                .arg(Value::U32(id))
                .arg(Value::U32(reason));
            s.destination = Some(l.owner);
            self.out.push(s);
        }
    }

    /// The notice last posted with a chip, and that chip: what `--test`
    /// presses in place of the shell.
    pub fn last_chip(&self) -> Option<(String, String)> {
        let (_, l) = self.live.iter().max_by_key(|(id, _)| **id)?;
        let chip = l.actions.iter().find(|(i, _)| i != "default")?;
        Some((format!("{}.{}", l.app, l.tag), chip.0.clone()))
    }

    /// A caller left the bus: its portal notifications stay, but nothing
    /// can hear their actions.
    pub fn gone(&mut self, bus_name: &str) {
        for l in self.live.values_mut() {
            if l.owner == bus_name {
                l.owner.clear();
            }
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

    #[test]
    fn actions_become_chips_and_come_back_as_signals() {
        let dir = std::env::temp_dir().join(format!("gh0st-notify-act-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut n = Notifications::new(dir.clone());
        let who = Who {
            app: "exe:/usr/bin/mail".into(),
            label: "mail".into(),
        };
        let mut m = Message::call(NAME, PATH, IFACE, "Notify")
            .arg(Value::str("mail"))
            .arg(Value::U32(0))
            .arg(Value::str(""))
            .arg(Value::str("New mail"))
            .arg(Value::str("From Ana"))
            .arg(Value::strs([
                "default", "Open", "reply-7", "Reply", "spam", "Spam",
            ]))
            .arg(Value::props::<&str>([]))
            .arg(Value::I32(-1));
        m.serial = 1;
        let id = n.call(&m, &who, ":1.4").body[0].as_u32().unwrap();
        let file = std::fs::read_to_string(dir.join(format!("mail.n{id}"))).unwrap();
        assert!(
            file.contains("action.a0=Reply") && file.contains("action.a1=Spam"),
            "{file}"
        );
        assert!(file.contains("default=default") && file.contains("via=bus"));
        n.shell_said(&["action".into(), format!("mail.n{id}"), "a0".into()]);
        let s = n.out.pop().unwrap();
        assert!(s.is(IFACE, "ActionInvoked"));
        assert_eq!(s.body[1].as_str(), Some("reply-7"));
        assert_eq!(s.destination.as_deref(), Some(":1.4"));
        n.shell_said(&["closed".into(), format!("mail.n{id}")]);
        let s = n.out.pop().unwrap();
        assert!(s.is(IFACE, "NotificationClosed"));
        assert_eq!(s.body[1].as_u32(), Some(2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_portal_notification_answers_with_its_action_and_target() {
        let dir = std::env::temp_dir().join(format!("gh0st-notify-portal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut n = Notifications::new(dir.clone());
        let who = Who {
            app: "exe:/usr/bin/player".into(),
            label: "player".into(),
        };
        let button = Value::props([
            ("label", Value::str("Next")),
            ("action", Value::str("app.next")),
            ("target", Value::U32(7)),
        ]);
        let buttons = Value::Array(button.ty(), vec![button]);
        let mut m = Message::call(NAME, PORTAL_PATH, PORTAL_IFACE, "AddNotification")
            .arg(Value::str("now-playing"))
            .arg(Value::props([
                ("title", Value::str("Night Drive")),
                ("priority", Value::str("high")),
                ("default-action", Value::str("app.show")),
                ("buttons", buttons),
            ]));
        m.serial = 1;
        assert!(n.portal_call(&m, &who, ":1.9").error_name.is_none());
        let tag = tag_of("p", "now-playing");
        let file = std::fs::read_to_string(dir.join(format!("player.{tag}"))).unwrap();
        assert!(
            file.contains("action.a0=Next") && file.contains("tone=warning"),
            "{file}"
        );
        n.shell_said(&["action".into(), format!("player.{tag}"), "a0".into()]);
        let s = n.out.pop().unwrap();
        assert!(s.is(PORTAL_IFACE, "ActionInvoked"));
        assert_eq!(s.body[0].as_str(), Some("now-playing"));
        assert_eq!(s.body[1].as_str(), Some("app.next"));
        assert_eq!(s.body[2].as_array().map(|a| a.len()), Some(1));
        n.shell_said(&["action".into(), format!("player.{tag}"), "default".into()]);
        assert_eq!(n.out.pop().unwrap().body[1].as_str(), Some("app.show"));
        let mut rm = Message::call(NAME, PORTAL_PATH, PORTAL_IFACE, "RemoveNotification")
            .arg(Value::str("now-playing"));
        rm.serial = 2;
        n.portal_call(&rm, &who, ":1.9");
        assert!(!dir.join(format!("player.{tag}")).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
