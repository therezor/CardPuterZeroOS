//! A tree of objects a client exports for a service to call back, as
//! BlueZ calls a GATT application, an advertisement and an agent. The tree
//! answers what every object must: ObjectManager on its root, Properties,
//! Introspectable and Peer. Methods of the app's own interfaces are left
//! to the app.

use crate::bus::{self, PROPERTIES, err};
use crate::message::{Kind, Message};
use crate::sig::Ty;
use crate::value::Value;

pub const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

struct Iface {
    name: String,
    props: Vec<(String, Value)>,
}

struct Object {
    path: String,
    ifaces: Vec<Iface>,
}

/// Objects under one root path, in the order added.
pub struct Tree {
    root: String,
    objects: Vec<Object>,
}

impl Tree {
    pub fn new(root: &str) -> Tree {
        Tree {
            root: root.to_string(),
            objects: Vec::new(),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// Adds an interface with its properties to the object at `path`,
    /// making the object if it is new.
    pub fn add(&mut self, path: &str, iface: &str, props: Vec<(&str, Value)>) {
        let props = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let iface = Iface {
            name: iface.to_string(),
            props,
        };
        match self.objects.iter_mut().find(|o| o.path == path) {
            Some(o) => {
                o.ifaces.retain(|i| i.name != iface.name);
                o.ifaces.push(iface);
            }
            None => self.objects.push(Object {
                path: path.to_string(),
                ifaces: vec![iface],
            }),
        }
    }

    /// Takes the object at `path` and every object under it.
    pub fn remove(&mut self, path: &str) {
        let under = format!("{path}/");
        self.objects
            .retain(|o| o.path != path && !o.path.starts_with(&under));
    }

    pub fn has(&self, path: &str) -> bool {
        self.objects.iter().any(|o| o.path == path)
    }

    pub fn get(&self, path: &str, iface: &str, prop: &str) -> Option<&Value> {
        self.iface(path, iface)?
            .props
            .iter()
            .find(|(k, _)| k == prop)
            .map(|(_, v)| v)
    }

    /// Changes a property and returns the PropertiesChanged signal to send,
    /// or None when it was already so or isn't there.
    pub fn set(&mut self, path: &str, iface: &str, prop: &str, value: Value) -> Option<Message> {
        let o = self.objects.iter_mut().find(|o| o.path == path)?;
        let i = o.ifaces.iter_mut().find(|i| i.name == iface)?;
        let slot = i.props.iter_mut().find(|(k, _)| k == prop)?;
        if slot.1 == value {
            return None;
        }
        slot.1 = value.clone();
        Some(bus::changed(path, iface, vec![(prop, value)]))
    }

    fn iface(&self, path: &str, iface: &str) -> Option<&Iface> {
        self.objects
            .iter()
            .find(|o| o.path == path)?
            .ifaces
            .iter()
            .find(|i| i.name == iface)
    }

    /// `a{oa{sa{sv}}}`: every object under the root with its interfaces and
    /// their properties.
    pub fn managed(&self) -> Value {
        let under = format!("{}/", self.root.trim_end_matches('/'));
        let objects = self
            .objects
            .iter()
            .filter(|o| o.path.starts_with(&under))
            .map(|o| {
                let ifaces = o.ifaces.iter().map(|i| {
                    let props = i.props.iter().map(|(k, v)| (k.as_str(), v.clone()));
                    (Value::str(&i.name), Value::props(props))
                });
                (
                    Value::path(&o.path),
                    Value::dict(Ty::Str, props_ty(), ifaces),
                )
            });
        Value::dict(Ty::Path, ifaces_ty(), objects)
    }

    /// The reply to `call` if the tree answers it: the standard
    /// interfaces, and an error for a path it doesn't have. None means the
    /// call is for the app's own methods.
    pub fn answer(&self, call: &Message) -> Option<Message> {
        if call.kind != Kind::Call {
            return None;
        }
        if let Some(reply) = bus::peer(call) {
            return Some(reply);
        }
        let path = call.path.as_deref().unwrap_or("");
        let iface = call.interface.as_deref().unwrap_or("");
        let member = call.member.as_deref().unwrap_or("");
        let arg = |i: usize| call.body.get(i).and_then(Value::as_str).unwrap_or("");
        match (iface, member) {
            (OBJECT_MANAGER, "GetManagedObjects") if path == self.root => {
                return Some(Message::reply(call).arg(self.managed()));
            }
            (bus::INTROSPECTABLE, "Introspect") => {
                return Some(Message::reply(call).arg(Value::str(self.introspect(path))));
            }
            _ => {}
        }
        if !self.has(path) {
            return Some(Message::error(call, err::UNKNOWN_OBJECT, "no such object"));
        }
        if iface != PROPERTIES {
            return None;
        }
        let Some(i) = self.iface(path, arg(0)) else {
            return Some(Message::error(
                call,
                err::UNKNOWN_INTERFACE,
                "no such interface",
            ));
        };
        Some(match member {
            "Get" => match i.props.iter().find(|(k, _)| k == arg(1)) {
                Some((_, v)) => Message::reply(call).arg(Value::variant(v.clone())),
                None => Message::error(call, err::UNKNOWN_PROPERTY, "no such property"),
            },
            "GetAll" => {
                let props = i.props.iter().map(|(k, v)| (k.as_str(), v.clone()));
                Message::reply(call).arg(Value::props(props))
            }
            "Set" => Message::error(call, err::READ_ONLY, "read only"),
            _ => Message::error(call, err::UNKNOWN_METHOD, "no such method"),
        })
    }

    fn introspect(&self, path: &str) -> String {
        let mut ifaces = Vec::new();
        if path == self.root {
            ifaces.push(format!(
                " <interface name=\"{OBJECT_MANAGER}\">\n  <method name=\"GetManagedObjects\">\
                 <arg type=\"a{{oa{{sa{{sv}}}}}}\" direction=\"out\"/></method>\n </interface>\n"
            ));
        }
        if let Some(o) = self.objects.iter().find(|o| o.path == path) {
            ifaces.push(bus::PROPERTIES_XML.to_string());
            for i in &o.ifaces {
                let mut x = format!(" <interface name=\"{}\">\n", i.name);
                for (k, v) in &i.props {
                    x.push_str(&format!(
                        "  <property name=\"{k}\" type=\"{}\" access=\"read\"/>\n",
                        v.ty().sig()
                    ));
                }
                x.push_str(" </interface>\n");
                ifaces.push(x);
            }
        }
        let prefix = format!("{}/", path.trim_end_matches('/'));
        let mut children: Vec<&str> = self
            .objects
            .iter()
            .filter_map(|o| o.path.strip_prefix(&prefix))
            .filter_map(|rest| rest.split('/').next())
            .collect();
        children.dedup();
        let refs: Vec<&str> = ifaces.iter().map(String::as_str).collect();
        bus::introspect(&refs, &children)
    }
}

fn props_ty() -> Ty {
    Ty::Array(Box::new(Ty::Entry(
        Box::new(Ty::Str),
        Box::new(Ty::Variant),
    )))
}

fn ifaces_ty() -> Ty {
    Ty::Array(Box::new(Ty::Entry(Box::new(Ty::Str), Box::new(props_ty()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/org/gh0st/test";

    fn tree() -> Tree {
        let mut t = Tree::new(ROOT);
        t.add(
            "/org/gh0st/test/service0",
            "org.bluez.GattService1",
            vec![
                ("UUID", Value::str("00001812-0000-1000-8000-00805f9b34fb")),
                ("Primary", Value::Bool(true)),
            ],
        );
        t.add(
            "/org/gh0st/test/service0/char0",
            "org.bluez.GattCharacteristic1",
            vec![("Flags", Value::strs(["read"]))],
        );
        t
    }

    fn call(path: &str, iface: &str, member: &str) -> Message {
        let mut m = Message::call(":1.9", path, iface, member);
        m.serial = 7;
        m
    }

    #[test]
    fn managed_objects_list_every_object_under_the_root() {
        let t = tree();
        let reply = t
            .answer(&call(ROOT, OBJECT_MANAGER, "GetManagedObjects"))
            .unwrap();
        assert_eq!(reply.signature(), "a{oa{sa{sv}}}");
        let objects = reply.body[0].as_array().unwrap();
        assert_eq!(objects.len(), 2);
        let (path, ifaces) = reply.body[0].entries().next().unwrap();
        assert_eq!(path.as_str(), Some("/org/gh0st/test/service0"));
        let props = ifaces.get("org.bluez.GattService1").unwrap();
        assert_eq!(
            props.get("Primary").map(Value::inner),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn properties_answer_and_refuse_to_change() {
        let t = tree();
        let get = call("/org/gh0st/test/service0", PROPERTIES, "Get")
            .arg(Value::str("org.bluez.GattService1"))
            .arg(Value::str("Primary"));
        let reply = t.answer(&get).unwrap();
        assert_eq!(reply.body, [Value::variant(Value::Bool(true))]);
        let all = call("/org/gh0st/test/service0", PROPERTIES, "GetAll")
            .arg(Value::str("org.bluez.GattService1"));
        assert_eq!(t.answer(&all).unwrap().signature(), "a{sv}");
        let set = call("/org/gh0st/test/service0", PROPERTIES, "Set")
            .arg(Value::str("org.bluez.GattService1"))
            .arg(Value::str("Primary"))
            .arg(Value::variant(Value::Bool(false)));
        assert_eq!(
            t.answer(&set).unwrap().error_name.as_deref(),
            Some(err::READ_ONLY)
        );
        let nope = call("/org/gh0st/test/service0", PROPERTIES, "Get")
            .arg(Value::str("x.y"))
            .arg(Value::str("Primary"));
        assert_eq!(
            t.answer(&nope).unwrap().error_name.as_deref(),
            Some(err::UNKNOWN_INTERFACE)
        );
    }

    #[test]
    fn app_methods_are_left_to_the_app() {
        let t = tree();
        let read = call(
            "/org/gh0st/test/service0/char0",
            "org.bluez.GattCharacteristic1",
            "ReadValue",
        );
        assert!(t.answer(&read).is_none());
        let lost = call(
            "/org/gh0st/test/nothing",
            "org.bluez.GattCharacteristic1",
            "ReadValue",
        );
        assert_eq!(
            t.answer(&lost).unwrap().error_name.as_deref(),
            Some(err::UNKNOWN_OBJECT)
        );
    }

    #[test]
    fn set_signals_only_a_change() {
        let mut t = tree();
        let p = "/org/gh0st/test/service0/char0";
        let i = "org.bluez.GattCharacteristic1";
        t.add(p, i, vec![("Value", Value::Bytes(vec![0]))]);
        assert!(t.set(p, i, "Value", Value::Bytes(vec![0])).is_none());
        let sig = t.set(p, i, "Value", Value::Bytes(vec![1])).unwrap();
        assert_eq!(sig.member.as_deref(), Some("PropertiesChanged"));
        assert_eq!(t.get(p, i, "Value"), Some(&Value::Bytes(vec![1])));
    }

    #[test]
    fn introspection_names_children_once() {
        let t = tree();
        let reply = t
            .answer(&call(ROOT, bus::INTROSPECTABLE, "Introspect"))
            .unwrap();
        let xml = reply.body[0].as_str().unwrap();
        assert_eq!(xml.matches("<node name=\"service0\"/>").count(), 1);
        assert!(xml.contains(OBJECT_MANAGER));
    }

    #[test]
    fn remove_takes_the_subtree() {
        let mut t = tree();
        t.remove("/org/gh0st/test/service0");
        assert!(!t.has("/org/gh0st/test/service0/char0"));
        let reply = t
            .answer(&call(ROOT, OBJECT_MANAGER, "GetManagedObjects"))
            .unwrap();
        assert!(reply.body[0].as_array().unwrap().is_empty());
    }
}
