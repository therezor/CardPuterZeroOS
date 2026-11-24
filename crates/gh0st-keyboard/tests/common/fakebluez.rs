//! A fake BlueZ on the other end of a socket pair: the bus daemon's
//! AddMatch, BlueZ's managers and adapter, and calls into the app's
//! objects as bluetoothd makes them. Single-threaded: a test pumps both
//! ends in turn.

#![allow(dead_code)]

use gh0st_dbus::{Conn, Kind, Message, Value};
use gh0st_keyboard::link::Event;
use gh0st_keyboard::link::ble::Ble;

pub const ADAPTER: &str = "/org/bluez/hci0";
pub const PHONE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";
const ME: &str = ":1.3";

enum Waiting {
    /// GetManagedObjects to the app, then this RegisterApplication's reply.
    App(Message),
    /// GetAll of the advert, then this RegisterAdvertisement's reply.
    Advert(Message),
    /// A call a test made into the app.
    Test,
}

pub struct FakeBluez {
    conn: Conn,
    serial: u32,
    waiting: Vec<(u32, Waiting)>,
    /// Every call the app made, as "Member path".
    pub log: Vec<String>,
    /// What GetManagedObjects of the app returned.
    pub app: Option<Value>,
    /// The advert's properties when it was registered.
    pub advert: Option<Value>,
    /// Signals the app sent.
    pub signals: Vec<Message>,
    /// Replies to the test's calls, by serial.
    pub replies: Vec<Message>,
    pub powered: bool,
    pub devices: Vec<(String, bool)>,
}

pub fn start(powered: bool) -> (Ble, FakeBluez) {
    let (a, b) = gh0st_dbus::conn::pair().unwrap();
    let ble = Ble::new(Conn::raw(a).unwrap());
    let fake = FakeBluez {
        conn: Conn::raw(b).unwrap(),
        serial: 1000,
        waiting: Vec::new(),
        log: Vec::new(),
        app: None,
        advert: None,
        signals: Vec::new(),
        replies: Vec::new(),
        powered,
        devices: vec![(PHONE.into(), true)],
    };
    (ble, fake)
}

/// Both ends until neither has anything to say. Returns the app's events.
pub fn pump(ble: &mut Ble, fake: &mut FakeBluez) -> Vec<Event> {
    let mut out = Vec::new();
    for _ in 0..20 {
        fake.step();
        ble.ready(&mut out);
    }
    out
}

impl FakeBluez {
    fn send(&mut self, mut m: Message) -> u32 {
        m.sender = Some(ME.into());
        self.conn.send(m).unwrap()
    }

    fn reply(&mut self, call: &Message) {
        self.send(Message::reply(call));
    }

    /// Calls into the app from `sender` (BlueZ unless a test says not).
    pub fn call_as(
        &mut self,
        sender: &str,
        path: &str,
        iface: &str,
        member: &str,
        args: Vec<Value>,
    ) -> u32 {
        let mut m = Message::call(":1.99", path, iface, member);
        m.body = args;
        m.sender = Some(sender.into());
        let serial = self.conn.send(m).unwrap();
        self.conn.flush().unwrap();
        self.waiting.push((serial, Waiting::Test));
        serial
    }

    pub fn call(&mut self, path: &str, iface: &str, member: &str, args: Vec<Value>) -> u32 {
        self.call_as(ME, path, iface, member, args)
    }

    pub fn reply_to(&self, serial: u32) -> Option<&Message> {
        self.replies.iter().find(|r| r.reply_serial == Some(serial))
    }

    /// A device's property changes, as BlueZ signals them.
    pub fn device(&mut self, path: &str, props: Vec<(&str, Value)>) {
        let m = Message::signal(path, "org.freedesktop.DBus.Properties", "PropertiesChanged")
            .arg(Value::str("org.bluez.Device1"))
            .arg(Value::props(props))
            .arg(Value::strs::<&str>([]));
        self.send(m);
        self.conn.flush().unwrap();
    }

    pub fn emit(&mut self, m: Message) {
        self.send(m);
        self.conn.flush().unwrap();
    }

    fn objects(&self) -> Value {
        let adapter = Value::dict(
            gh0st_dbus::Ty::Str,
            props_ty(),
            [
                (
                    Value::str("org.bluez.Adapter1"),
                    Value::props([
                        ("Powered", Value::Bool(self.powered)),
                        ("Alias", Value::str("zero")),
                        ("Pairable", Value::Bool(false)),
                    ]),
                ),
                (
                    Value::str("org.bluez.GattManager1"),
                    Value::props::<&str>([]),
                ),
            ],
        );
        let mut objects = vec![(Value::path(ADAPTER), adapter)];
        for (path, paired) in &self.devices {
            let dev = Value::dict(
                gh0st_dbus::Ty::Str,
                props_ty(),
                [(
                    Value::str("org.bluez.Device1"),
                    Value::props([
                        ("Address", Value::str("AA:BB:CC:DD:EE:FF")),
                        ("Alias", Value::str("Pixel")),
                        ("Icon", Value::str("phone")),
                        ("Paired", Value::Bool(*paired)),
                        ("Connected", Value::Bool(false)),
                    ]),
                )],
            );
            objects.push((Value::path(path), dev));
        }
        Value::dict(gh0st_dbus::Ty::Path, ifaces_ty(), objects)
    }

    pub fn step(&mut self) {
        self.conn.fill().unwrap();
        while let Some(m) = self.conn.recv().unwrap() {
            self.handle(m);
        }
        self.conn.flush().unwrap();
    }

    fn handle(&mut self, m: Message) {
        match m.kind {
            Kind::Signal => self.signals.push(m),
            Kind::Return | Kind::Error => {
                let Some(i) = self
                    .waiting
                    .iter()
                    .position(|(s, _)| Some(*s) == m.reply_serial)
                else {
                    return;
                };
                match self.waiting.swap_remove(i).1 {
                    Waiting::App(register) => {
                        self.app = m.body.first().cloned();
                        self.reply(&register);
                    }
                    Waiting::Advert(register) => {
                        self.advert = m.body.first().cloned();
                        self.reply(&register);
                    }
                    Waiting::Test => self.replies.push(m),
                }
            }
            Kind::Call => self.called(m),
        }
    }

    fn called(&mut self, m: Message) {
        let path = m.path.clone().unwrap_or_default();
        let member = m.member.clone().unwrap_or_default();
        self.log.push(format!("{member} {path}"));
        match member.as_str() {
            "GetManagedObjects" => {
                let o = self.objects();
                self.send(Message::reply(&m).arg(o));
            }
            "RegisterApplication" => {
                let root = m.body[0].as_str().unwrap().to_string();
                let mut get = Message::call(
                    ":1.99",
                    &root,
                    "org.freedesktop.DBus.ObjectManager",
                    "GetManagedObjects",
                );
                get.sender = Some(ME.into());
                let serial = self.conn.send(get).unwrap();
                self.waiting.push((serial, Waiting::App(m)));
            }
            "RegisterAdvertisement" => {
                let adv = m.body[0].as_str().unwrap().to_string();
                let mut get =
                    Message::call(":1.99", &adv, "org.freedesktop.DBus.Properties", "GetAll")
                        .arg(Value::str("org.bluez.LEAdvertisement1"));
                get.sender = Some(ME.into());
                let serial = self.conn.send(get).unwrap();
                self.waiting.push((serial, Waiting::Advert(m)));
            }
            "Set" => {
                let prop = m.body[1].as_str().unwrap_or("").to_string();
                let v = m.body[2].inner().clone();
                self.reply(&m);
                if prop == "Powered" {
                    self.powered = v.as_bool().unwrap_or(false);
                    let s = Message::signal(
                        ADAPTER,
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                    )
                    .arg(Value::str("org.bluez.Adapter1"))
                    .arg(Value::props([("Powered", v)]))
                    .arg(Value::strs::<&str>([]));
                    self.send(s);
                }
            }
            "Disconnect" => {
                self.reply(&m);
                self.device(&path, vec![("Connected", Value::Bool(false))]);
            }
            _ if m.no_reply() => {}
            _ => self.reply(&m),
        }
    }

    /// The calls the app made with this member, in order.
    pub fn calls(&self, member: &str) -> Vec<&str> {
        self.log
            .iter()
            .filter_map(|l| l.strip_prefix(member).and_then(|r| r.strip_prefix(' ')))
            .collect()
    }
}

fn props_ty() -> gh0st_dbus::Ty {
    use gh0st_dbus::Ty;
    Ty::Array(Box::new(Ty::Entry(
        Box::new(Ty::Str),
        Box::new(Ty::Variant),
    )))
}

fn ifaces_ty() -> gh0st_dbus::Ty {
    use gh0st_dbus::Ty;
    Ty::Array(Box::new(Ty::Entry(Box::new(Ty::Str), Box::new(props_ty()))))
}
