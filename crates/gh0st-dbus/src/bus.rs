//! The bus daemon's own calls, the standard errors, and the interfaces
//! every object answers (Peer, Introspectable).

use std::os::fd::OwnedFd;

use crate::{Conn, Message, Result, Value};

pub const NAME: &str = "org.freedesktop.DBus";
pub const PATH: &str = "/org/freedesktop/DBus";
pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
pub const INTROSPECTABLE: &str = "org.freedesktop.DBus.Introspectable";
pub const PEER: &str = "org.freedesktop.DBus.Peer";

pub mod err {
    pub const FAILED: &str = "org.freedesktop.DBus.Error.Failed";
    pub const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
    pub const UNKNOWN_OBJECT: &str = "org.freedesktop.DBus.Error.UnknownObject";
    pub const UNKNOWN_INTERFACE: &str = "org.freedesktop.DBus.Error.UnknownInterface";
    pub const UNKNOWN_PROPERTY: &str = "org.freedesktop.DBus.Error.UnknownProperty";
    pub const READ_ONLY: &str = "org.freedesktop.DBus.Error.PropertyReadOnly";
    pub const INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";
    pub const NOT_SUPPORTED: &str = "org.freedesktop.DBus.Error.NotSupported";
    pub const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
    pub const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
    pub const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";
    pub const LIMITS: &str = "org.freedesktop.DBus.Error.LimitsExceeded";
}

/// RequestName flags and answers.
pub const ALLOW_REPLACEMENT: u32 = 1;
pub const REPLACE_EXISTING: u32 = 2;
pub const DO_NOT_QUEUE: u32 = 4;
pub const PRIMARY_OWNER: u32 = 1;
pub const ALREADY_OWNER: u32 = 4;

fn daemon(member: &str) -> Message {
    Message::call(NAME, PATH, NAME, member)
}

/// Takes `name`, never queueing. True if we own it now.
pub fn own(c: &mut Conn, name: &str) -> Result<bool> {
    let r = c.call(
        daemon("RequestName")
            .arg(Value::str(name))
            .arg(Value::U32(DO_NOT_QUEUE)),
        5000,
    )?;
    Ok(matches!(
        r.body.first().and_then(Value::as_u32),
        Some(PRIMARY_OWNER | ALREADY_OWNER)
    ))
}

pub fn add_match(c: &mut Conn, rule: &str) -> Result<()> {
    c.call(daemon("AddMatch").arg(Value::str(rule)), 5000)
        .map(drop)
}

/// Who a connection is, from the bus.
#[derive(Debug, Default)]
pub struct Creds {
    pub uid: Option<u32>,
    pub pid: Option<u32>,
    /// A pidfd, where the bus gives one: the process can't be swapped for
    /// another under the same pid.
    pub pidfd: Option<OwnedFd>,
}

pub fn credentials(c: &mut Conn, name: &str) -> Result<Creds> {
    let mut r = c.call(
        daemon("GetConnectionCredentials").arg(Value::str(name)),
        5000,
    )?;
    let mut out = Creds::default();
    if let Some(d) = r.body.first() {
        out.uid = d
            .get("UnixUserID")
            .map(Value::inner)
            .and_then(Value::as_u32);
        out.pid = d.get("ProcessID").map(Value::inner).and_then(Value::as_u32);
        if let Some(Value::Fd(i)) = d.get("ProcessFD").map(Value::inner) {
            let i = *i as usize;
            if i < r.fds.len() {
                out.pidfd = Some(r.fds.swap_remove(i));
            }
        }
    }
    Ok(out)
}

/// The reply to a Peer call, which every object answers.
pub fn peer(call: &Message) -> Option<Message> {
    if call.interface.as_deref() != Some(PEER) {
        return None;
    }
    Some(match call.member.as_deref() {
        Some("Ping") => Message::reply(call),
        Some("GetMachineId") => {
            let id = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
            Message::reply(call).arg(Value::str(id.trim()))
        }
        _ => Message::error(call, err::UNKNOWN_METHOD, "no such method"),
    })
}

/// Introspection XML for one object: its interfaces' XML, then children.
pub fn introspect(ifaces: &[&str], children: &[&str]) -> String {
    let mut s = String::from(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n\
         \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n",
    );
    s.push_str(
        " <interface name=\"org.freedesktop.DBus.Introspectable\">\n  \
         <method name=\"Introspect\"><arg name=\"xml\" type=\"s\" direction=\"out\"/></method>\n \
         </interface>\n <interface name=\"org.freedesktop.DBus.Peer\">\n  \
         <method name=\"Ping\"/>\n  \
         <method name=\"GetMachineId\"><arg name=\"id\" type=\"s\" direction=\"out\"/></method>\n \
         </interface>\n",
    );
    for i in ifaces {
        s.push_str(i);
    }
    for c in children {
        s.push_str(&format!(" <node name=\"{c}\"/>\n"));
    }
    s.push_str("</node>\n");
    s
}

/// The Properties interface's XML, for objects that have properties.
pub const PROPERTIES_XML: &str = " <interface name=\"org.freedesktop.DBus.Properties\">\n  \
<method name=\"Get\"><arg type=\"s\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/><arg type=\"v\" direction=\"out\"/></method>\n  \
<method name=\"GetAll\"><arg type=\"s\" direction=\"in\"/><arg type=\"a{sv}\" direction=\"out\"/></method>\n  \
<method name=\"Set\"><arg type=\"s\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/><arg type=\"v\" direction=\"in\"/></method>\n  \
<signal name=\"PropertiesChanged\"><arg type=\"s\"/><arg type=\"a{sv}\"/><arg type=\"as\"/></signal>\n \
</interface>\n";

/// PropertiesChanged for `iface` on `path`.
pub fn changed(path: &str, iface: &str, props: Vec<(&str, Value)>) -> Message {
    Message::signal(path, PROPERTIES, "PropertiesChanged")
        .arg(Value::str(iface))
        .arg(Value::props(props))
        .arg(Value::strs::<&str>([]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_answers() {
        let mut call = Message::call(":1.2", "/a", PEER, "Ping");
        call.serial = 3;
        assert_eq!(peer(&call).unwrap().reply_serial, Some(3));
        assert!(peer(&Message::call(":1.2", "/a", "x.y", "Ping")).is_none());
    }

    #[test]
    fn introspection_lists_children() {
        let x = introspect(&[PROPERTIES_XML], &["collection"]);
        assert!(x.contains("<node name=\"collection\"/>") && x.contains("PropertiesChanged"));
    }
}
