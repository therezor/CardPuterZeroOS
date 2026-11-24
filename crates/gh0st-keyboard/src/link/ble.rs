//! Bluetooth LE through BlueZ: the keyboard as a HID-over-GATT device.
//!
//! All of it is D-Bus on the system bus, as the user, in the app's poll
//! loop. Nothing waits on a reply: BlueZ calls our objects back before it
//! answers a registration, so calls go out with [`Pending`] and replies
//! are matched as they come.
//!
//! - The GATT tree (`gatt.rs`) is registered once. BlueZ serves it to
//!   every host that connects.
//! - The advert runs only when asked: discoverable with the name while
//!   pairing, nameless for a bonded host to come back. It has a timeout.
//! - The agent shows the pairing code and refuses pairing at any other
//!   time.
//! - A report is a PropertiesChanged on its characteristic's Value: BlueZ
//!   notifies the host that listens.

use std::os::fd::RawFd;

use gh0st_dbus::bus::{self, Pending};
use gh0st_dbus::message::NO_REPLY;
use gh0st_dbus::objects::Tree;
use gh0st_dbus::{Conn, Kind, Message, Value};

use super::gatt::{self, AGENT, CHAR, DESC};
use super::{Ask, Device, Event};
use crate::hid::report::Report;

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const GATT_MANAGER: &str = "org.bluez.GattManager1";
const ADV_MANAGER: &str = "org.bluez.LEAdvertisingManager1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const REJECTED: &str = "org.bluez.Error.Rejected";

/// Seconds an advert runs: long enough to pick the keyboard in a list, or
/// to wake a sleeping computer.
pub const PAIR_SECS: u16 = 180;
pub const RECONNECT_SECS: u16 = 120;

/// What the advert is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advert {
    Off,
    /// Discoverable, with the name: a new host can find and pair.
    Pair,
    /// No name: a bonded host comes back.
    Reconnect,
}

#[derive(Debug)]
enum Call {
    Quiet,
    Objects,
    App,
    Advertise,
    Power,
}

pub struct Ble {
    conn: Conn,
    tree: Tree,
    pending: Pending<Call>,
    /// BlueZ's unique name: only it may call our objects.
    bluez: Option<String>,
    adapter: Option<String>,
    name: String,
    powered: bool,
    registered: bool,
    advert: Advert,
    want: Advert,
    pairing: bool,
    /// The agent call that waits for the user.
    asking: Option<(Message, Ask)>,
    devices: Vec<Device>,
    /// The paths of the app's hosts. Other devices (headphones paired in
    /// Settings) come and go without touching the keyboard.
    hosts: Vec<String>,
    listening: bool,
    /// The host asked for the boot protocol.
    boot: bool,
    dead: bool,
}

impl Ble {
    /// Joins the system bus and asks BlueZ what it has.
    pub fn system() -> Result<Ble, String> {
        let conn = Conn::system().map_err(|e| format!("No system bus: {e}"))?;
        Ok(Ble::new(conn))
    }

    /// Over `conn`, which tests connect to a fake BlueZ.
    pub fn new(conn: Conn) -> Ble {
        let mut b = Ble {
            conn,
            tree: gatt::tree(),
            pending: Pending::new(),
            bluez: None,
            adapter: None,
            name: String::new(),
            powered: false,
            registered: false,
            advert: Advert::Off,
            want: Advert::Off,
            pairing: false,
            asking: None,
            devices: Vec::new(),
            hosts: Vec::new(),
            listening: false,
            boot: false,
            dead: false,
        };
        for rule in [
            "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
            "type='signal',sender='org.bluez',interface='org.freedesktop.DBus.ObjectManager'",
            "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.bluez'",
        ] {
            let m =
                Message::call(bus::NAME, bus::PATH, bus::NAME, "AddMatch").arg(Value::str(rule));
            b.call(m, Call::Quiet);
        }
        b.call(
            Message::call(BLUEZ, "/", OBJECT_MANAGER, "GetManagedObjects"),
            Call::Objects,
        );
        b.flush();
        b
    }

    pub fn fd(&self) -> RawFd {
        self.conn.fd()
    }

    /// The adapter's name, as hosts see it while pairing.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The host connected now, if any: the first of the app's hosts that
    /// BlueZ says is connected.
    pub fn connected(&self) -> Option<&str> {
        self.devices
            .iter()
            .find(|d| d.connected && self.hosts.contains(&d.path))
            .map(|d| d.path.as_str())
    }

    /// BlueZ has said which adapter there is and whether it is on.
    pub fn known(&self) -> bool {
        self.adapter.is_some()
    }

    /// The app's hosts, by BlueZ path.
    pub fn set_hosts(&mut self, paths: Vec<String>) {
        self.hosts = paths;
        self.sync();
        self.flush();
    }

    /// The connected host has asked for key reports: typing reaches it.
    pub fn listening(&self) -> bool {
        self.listening
    }

    /// The advert asked for: it runs once the adapter is on and no host
    /// is connected.
    pub fn wanted(&self) -> Advert {
        self.want
    }

    pub fn asking(&self) -> Option<Ask> {
        self.asking.as_ref().map(|(_, a)| *a)
    }

    fn call(&mut self, m: Message, tag: Call) {
        if self.pending.send(&mut self.conn, m, tag).is_err() {
            self.dead = true;
        }
    }

    fn send_msg(&mut self, m: Message) {
        if self.conn.send(m).is_err() {
            self.dead = true;
        }
    }

    pub fn flush(&mut self) {
        let _ = self.conn.flush();
    }

    /// Reads and answers what came. Call when the fd is readable.
    pub fn ready(&mut self, out: &mut Vec<Event>) {
        if self.dead {
            return;
        }
        if self.conn.fill().is_err() || self.conn.closed() {
            return self.fail(out, "Bluetooth stopped");
        }
        loop {
            match self.conn.recv() {
                Ok(Some(m)) => self.handle(m, out),
                Ok(None) => break,
                Err(_) => return self.fail(out, "Bluetooth sent something broken"),
            }
        }
        self.flush();
        if self.dead {
            self.fail(out, "Bluetooth stopped");
        }
    }

    fn fail(&mut self, out: &mut Vec<Event>, why: &str) {
        if !out.iter().any(|e| matches!(e, Event::Failed(_))) {
            out.push(Event::Failed(why.into()));
        }
        self.dead = true;
        self.listening = false;
    }

    fn handle(&mut self, m: Message, out: &mut Vec<Event>) {
        match m.kind {
            Kind::Return | Kind::Error => {
                if let Some(tag) = self.pending.take(&m) {
                    self.replied(tag, &m, out);
                }
            }
            Kind::Signal => self.signal(&m, out),
            Kind::Call => self.called(m, out),
        }
    }

    fn replied(&mut self, tag: Call, m: &Message, out: &mut Vec<Event>) {
        let failed = m.kind == Kind::Error;
        match tag {
            Call::Quiet => {}
            Call::Objects if failed => self.fail(out, "Bluetooth is not running"),
            Call::Objects => {
                self.bluez = m.sender.clone();
                if let Some(objects) = m.body.first() {
                    self.objects(objects);
                }
                let Some(adapter) = self.adapter.clone() else {
                    return self.fail(out, "No Bluetooth here");
                };
                self.register(&adapter);
                out.push(Event::Power(self.powered));
                out.push(Event::Devices);
            }
            Call::App if failed => {
                let why = format!("Bluetooth refused the keyboard: {}", m.error_text());
                self.fail(out, &why);
            }
            Call::App => {
                self.registered = true;
                out.push(Event::Ready);
                self.sync();
            }
            Call::Advertise if failed => {
                self.advert = Advert::Off;
                self.want = Advert::Off;
                out.push(Event::AdvertEnded);
            }
            Call::Advertise => {}
            Call::Power if failed => {
                out.push(Event::Power(false));
            }
            Call::Power => {}
        }
    }

    /// The adapter and the devices, from BlueZ's GetManagedObjects.
    fn objects(&mut self, objects: &Value) {
        for (path, ifaces) in objects.entries() {
            let Some(path) = path.as_str() else { continue };
            if let Some(a) = ifaces.get(ADAPTER)
                && self.adapter.is_none()
            {
                self.adapter = Some(path.to_string());
                self.powered = prop(a, "Powered").and_then(Value::as_bool).unwrap_or(false);
                self.name = prop_str(a, "Alias").unwrap_or_default();
            }
            if let Some(d) = ifaces.get(DEVICE) {
                let mut dev = Device {
                    path: path.to_string(),
                    ..Device::default()
                };
                update(&mut dev, d);
                self.devices.push(dev);
            }
        }
    }

    fn register(&mut self, adapter: &str) {
        let agent = Message::call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RegisterAgent")
            .arg(Value::path(AGENT))
            .arg(Value::str("KeyboardDisplay"));
        self.call(agent, Call::Quiet);
        let default = Message::call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RequestDefaultAgent")
            .arg(Value::path(AGENT));
        self.call(default, Call::Quiet);
        let app = Message::call(BLUEZ, adapter, GATT_MANAGER, "RegisterApplication")
            .arg(Value::path(gatt::ROOT))
            .arg(Value::props::<&str>([]));
        self.call(app, Call::App);
    }

    fn signal(&mut self, m: &Message, out: &mut Vec<Event>) {
        let path = m.path.as_deref().unwrap_or("");
        match (m.interface.as_deref(), m.member.as_deref()) {
            (Some(bus::PROPERTIES), Some("PropertiesChanged")) => {
                let iface = m.body.first().and_then(Value::as_str).unwrap_or("");
                let Some(changed) = m.body.get(1) else { return };
                if iface == ADAPTER && Some(path) == self.adapter.as_deref() {
                    if let Some(on) = prop(changed, "Powered").and_then(Value::as_bool) {
                        self.powered = on;
                        out.push(Event::Power(on));
                        self.sync();
                    }
                    if let Some(name) = prop_str(changed, "Alias") {
                        self.name = name;
                    }
                } else if iface == DEVICE {
                    self.device_changed(path, changed, out);
                }
            }
            (Some(OBJECT_MANAGER), Some("InterfacesAdded")) => {
                let Some(ifaces) = m.body.get(1) else { return };
                let Some(path) = m.body.first().and_then(Value::as_str) else {
                    return;
                };
                if let Some(d) = ifaces.get(DEVICE) {
                    self.device_changed(path, d, out);
                }
            }
            (Some(OBJECT_MANAGER), Some("InterfacesRemoved")) => {
                let Some(path) = m.body.first().and_then(Value::as_str) else {
                    return;
                };
                let gone = |i: &str| {
                    m.body
                        .get(1)
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(i)))
                };
                if gone(DEVICE) {
                    let was = self.devices.iter().any(|d| d.path == path && d.connected);
                    self.devices.retain(|d| d.path != path);
                    if was {
                        self.went(path.to_string(), out);
                    }
                    out.push(Event::Devices);
                }
                if gone(ADAPTER) && Some(path) == self.adapter.as_deref() {
                    self.fail(out, "The Bluetooth adapter went away");
                }
            }
            (Some(bus::NAME), Some("NameOwnerChanged")) => {
                let new = m.body.get(2).and_then(Value::as_str).unwrap_or("");
                if new.is_empty() {
                    self.fail(out, "Bluetooth stopped");
                }
            }
            _ => {}
        }
    }

    fn device_changed(&mut self, path: &str, changed: &Value, out: &mut Vec<Event>) {
        let i = match self.devices.iter().position(|d| d.path == path) {
            Some(i) => i,
            None => {
                self.devices.push(Device {
                    path: path.to_string(),
                    ..Device::default()
                });
                self.devices.len() - 1
            }
        };
        let before = self.devices[i].clone();
        update(&mut self.devices[i], changed);
        let after = self.devices[i].clone();
        if after.paired && !before.paired && self.pairing {
            out.push(Event::Paired(path.to_string()));
        }
        if after.connected && !before.connected {
            out.push(Event::Connected(path.to_string()));
            self.sync();
        } else if !after.connected && before.connected {
            self.went(path.to_string(), out);
        }
        if before != after {
            out.push(Event::Devices);
        }
    }

    /// A device let go. Once no host is left, nobody listens.
    fn went(&mut self, path: String, out: &mut Vec<Event>) {
        if self.connected().is_none() && self.listening {
            self.listening = false;
            out.push(Event::Listening(false));
        }
        out.push(Event::Disconnected(path));
        self.sync();
    }

    /// A method call on one of our objects.
    fn called(&mut self, m: Message, out: &mut Vec<Event>) {
        let from_bluez = self.bluez.is_some() && m.sender == self.bluez;
        if !from_bluez {
            if !m.no_reply() {
                let e = Message::error(&m, bus::err::ACCESS_DENIED, "only BlueZ");
                self.send_msg(e);
            }
            return;
        }
        if let Some(reply) = self.tree.answer(&m) {
            return self.send_msg(reply);
        }
        let path = m.path.clone().unwrap_or_default();
        let iface = m.interface.clone().unwrap_or_default();
        let member = m.member.clone().unwrap_or_default();
        let reply = match (iface.as_str(), member.as_str()) {
            (CHAR | DESC, "ReadValue") => {
                let offset = m
                    .body
                    .first()
                    .and_then(|o| prop(o, "offset"))
                    .and_then(|v| match v {
                        Value::U16(n) => Some(*n as usize),
                        _ => None,
                    })
                    .unwrap_or(0);
                let value = self
                    .tree
                    .get(&path, &iface, "Value")
                    .and_then(Value::as_bytes)
                    .unwrap_or(&[]);
                let tail = value.get(offset..).unwrap_or(&[]).to_vec();
                Some(Message::reply(&m).arg(Value::Bytes(tail)))
            }
            (CHAR, "WriteValue") => {
                let value = m
                    .body
                    .first()
                    .and_then(Value::as_bytes)
                    .unwrap_or(&[])
                    .to_vec();
                self.wrote(&path, &value, out);
                (!m.no_reply()).then(|| Message::reply(&m))
            }
            (CHAR, "StartNotify") | (CHAR, "StopNotify") => {
                let on = member == "StartNotify";
                if (path == gatt::KEYBOARD || path == gatt::BOOT_IN) && self.listening != on {
                    self.listening = on;
                    out.push(Event::Listening(on));
                }
                Some(Message::reply(&m))
            }
            (gatt::ADV, "Release") => {
                self.advert = Advert::Off;
                if self.want != Advert::Off {
                    self.want = Advert::Off;
                    out.push(Event::AdvertEnded);
                }
                Some(Message::reply(&m))
            }
            (gatt::AGENT_IFACE, _) => self.agent(m, &member, out),
            _ => Some(Message::error(
                &m,
                bus::err::UNKNOWN_METHOD,
                "no such method",
            )),
        };
        if let Some(r) = reply {
            self.send_msg(r);
        }
    }

    fn wrote(&mut self, path: &str, value: &[u8], out: &mut Vec<Event>) {
        let Some(&first) = value.first() else { return };
        match path {
            gatt::LEDS | gatt::BOOT_OUT => out.push(Event::Leds(first)),
            gatt::MODE => self.boot = first == 0,
            _ => return,
        }
        self.tree
            .set(path, CHAR, "Value", Value::Bytes(value.to_vec()));
    }

    /// Agent1: BlueZ asks while a host pairs. Only the Pair page says yes;
    /// the questions that need the user wait for [`answer`](Ble::answer).
    fn agent(&mut self, m: Message, member: &str, out: &mut Vec<Event>) -> Option<Message> {
        let device = m
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let code = m.body.get(1).and_then(Value::as_u32).unwrap_or(0);
        let ask = match member {
            "Release" => return Some(Message::reply(&m)),
            "Cancel" => {
                self.asking = None;
                out.push(Event::PairCancelled);
                return Some(Message::reply(&m));
            }
            "AuthorizeService" => {
                let known = self.devices.iter().any(|d| d.path == device && d.paired);
                return Some(if self.pairing || known {
                    Message::reply(&m)
                } else {
                    Message::error(&m, REJECTED, "not pairing")
                });
            }
            "DisplayPasskey" | "DisplayPinCode" => {
                if self.pairing {
                    out.push(Event::Pairing {
                        device,
                        ask: Ask::Show(code),
                    });
                }
                return Some(Message::reply(&m));
            }
            "RequestConfirmation" => Ask::Confirm(code),
            "RequestPasskey" => Ask::Enter,
            "RequestAuthorization" => Ask::Allow,
            _ => return Some(Message::error(&m, REJECTED, "not supported")),
        };
        if !self.pairing {
            return Some(Message::error(&m, REJECTED, "not pairing"));
        }
        if let Some((old, _)) = self.asking.take() {
            self.send_msg(Message::error(&old, REJECTED, "superseded"));
        }
        out.push(Event::Pairing { device, ask });
        self.asking = Some((m, ask));
        None
    }

    /// The user's answer to the pairing question: yes or no.
    pub fn answer(&mut self, yes: bool) {
        let Some((m, ask)) = self.asking.take() else {
            return;
        };
        let r = if yes && ask != Ask::Enter {
            Message::reply(&m)
        } else {
            Message::error(&m, REJECTED, "the user said no")
        };
        self.send_msg(r);
        self.flush();
    }

    /// The code the host showed, typed here.
    pub fn passkey(&mut self, code: u32) {
        let Some((m, Ask::Enter)) = self.asking.take() else {
            return;
        };
        self.send_msg(Message::reply(&m).arg(Value::U32(code)));
        self.flush();
    }

    fn set_adapter(&mut self, prop: &str, v: Value, tag: Call) {
        let Some(adapter) = self.adapter.clone() else {
            return;
        };
        let m = Message::call(BLUEZ, &adapter, bus::PROPERTIES, "Set")
            .arg(Value::str(ADAPTER))
            .arg(Value::str(prop))
            .arg(Value::variant(v));
        self.call(m, tag);
    }

    pub fn power(&mut self, on: bool) {
        self.set_adapter("Powered", Value::Bool(on), Call::Power);
        self.flush();
    }

    /// Pairing on: the adapter takes new bonds and the advert shows the
    /// name. Off: neither, and an unanswered question is refused.
    pub fn pair(&mut self, on: bool) {
        self.pairing = on;
        self.set_adapter("Pairable", Value::Bool(on), Call::Quiet);
        if !on {
            self.answer(false);
        }
        self.want = match (on, self.want) {
            (true, _) => Advert::Pair,
            (false, Advert::Pair) => Advert::Off,
            (false, w) => w,
        };
        self.sync();
        self.flush();
    }

    /// Advertise for a bonded host to come back, or stop.
    pub fn reconnect(&mut self, on: bool) {
        if self.pairing {
            return;
        }
        self.want = if on { Advert::Reconnect } else { Advert::Off };
        self.sync();
        self.flush();
    }

    /// Brings the registered advert in line with what is wanted: none
    /// while a host is connected, unless pairing.
    fn sync(&mut self) {
        let Some(adapter) = self.adapter.clone() else {
            return;
        };
        let idle = !self.registered || !self.powered;
        let served = self.connected().is_some() && self.want == Advert::Reconnect;
        let want = if idle || served {
            Advert::Off
        } else {
            self.want
        };
        if want == self.advert {
            return;
        }
        if self.advert != Advert::Off {
            let m = Message::call(BLUEZ, &adapter, ADV_MANAGER, "UnregisterAdvertisement")
                .arg(Value::path(gatt::ADVERT));
            self.call(m, Call::Quiet);
            self.advert = Advert::Off;
        }
        if want == Advert::Off {
            return;
        }
        let secs = if want == Advert::Pair {
            PAIR_SECS
        } else {
            RECONNECT_SECS
        };
        let name = self.name.clone();
        gatt::advert(&mut self.tree, want == Advert::Pair, &name, secs);
        let m = Message::call(BLUEZ, &adapter, ADV_MANAGER, "RegisterAdvertisement")
            .arg(Value::path(gatt::ADVERT))
            .arg(Value::props::<&str>([]));
        self.call(m, Call::Advertise);
        self.advert = want;
    }

    pub fn disconnect(&mut self, device: &str) {
        let m = Message::call(BLUEZ, device, DEVICE, "Disconnect");
        self.call(m, Call::Quiet);
        self.flush();
    }

    /// Removes the bond: the host must pair again.
    pub fn forget(&mut self, device: &str) {
        let Some(adapter) = self.adapter.clone() else {
            return;
        };
        let m = Message::call(BLUEZ, &adapter, ADAPTER, "RemoveDevice").arg(Value::path(device));
        self.call(m, Call::Quiet);
        self.flush();
    }

    /// Sends a report to the connected host. False when none listens.
    pub fn send(&mut self, r: &Report) -> bool {
        if self.connected().is_none() || self.dead {
            return false;
        }
        let path = match r {
            Report::Keyboard(_) if self.boot => gatt::BOOT_IN,
            Report::Keyboard(_) => gatt::KEYBOARD,
            Report::Consumer(_) => gatt::CONSUMER,
            Report::Mouse(_) => gatt::MOUSE,
        };
        let mut buf = [0u8; 8];
        let n = r.bytes(&mut buf);
        let value = buf[..n].to_vec();
        self.tree
            .set(path, CHAR, "Value", Value::Bytes(value.clone()));
        // Always signal, even an equal value: two equal mouse moves are two
        // moves.
        // Sent to BlueZ alone, not broadcast: no other program on the bus
        // can listen to the keys.
        let mut sig = bus::changed(path, CHAR, vec![("Value", Value::Bytes(value))]);
        sig.destination = self.bluez.clone();
        self.send_msg(sig);
        self.flush();
        true
    }

    /// The battery level hosts show, 0 to 100.
    pub fn battery(&mut self, percent: u8) {
        let v = Value::Bytes(vec![percent.min(100)]);
        if let Some(mut sig) = self.tree.set(gatt::LEVEL, CHAR, "Value", v) {
            sig.destination = self.bluez.clone();
            self.send_msg(sig);
            self.flush();
        }
    }

    /// Lets go of BlueZ: no advert, no pairing, no services. BlueZ would
    /// clean up when the app's connection closes anyway.
    pub fn stop(&mut self) {
        let Some(adapter) = self.adapter.clone() else {
            return;
        };
        self.pair(false);
        self.want = Advert::Off;
        self.sync();
        let m = Message::call(BLUEZ, &adapter, GATT_MANAGER, "UnregisterApplication")
            .arg(Value::path(gatt::ROOT));
        self.call(m, Call::Quiet);
        let m = Message::call(BLUEZ, "/org/bluez", AGENT_MANAGER, "UnregisterAgent")
            .arg(Value::path(AGENT))
            .flags(NO_REPLY);
        self.send_msg(m);
        self.registered = false;
        self.flush();
    }
}

fn prop<'a>(props: &'a Value, key: &str) -> Option<&'a Value> {
    props.get(key).map(Value::inner)
}

fn prop_str(props: &Value, key: &str) -> Option<String> {
    prop(props, key).and_then(Value::as_str).map(String::from)
}

/// A device's fields from BlueZ's properties, changed or all.
fn update(d: &mut Device, props: &Value) {
    if let Some(a) = prop_str(props, "Address") {
        d.address = a;
    }
    if let Some(n) = prop_str(props, "Alias").or_else(|| prop_str(props, "Name")) {
        d.name = n;
    }
    if let Some(i) = prop_str(props, "Icon") {
        d.icon = i;
    }
    if let Some(m) = prop_str(props, "Modalias") {
        d.modalias = m;
    }
    if let Some(p) = prop(props, "Paired").and_then(Value::as_bool) {
        d.paired = p;
    }
    if let Some(c) = prop(props, "Connected").and_then(Value::as_bool) {
        d.connected = c;
    }
}
