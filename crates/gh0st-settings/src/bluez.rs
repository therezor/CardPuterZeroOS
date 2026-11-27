//! Bluetooth through BlueZ, as the user, in Settings' poll loop.
//!
//! One connection to the system bus. BlueZ's signals keep the adapter and
//! the devices current, so the page shows what BlueZ knows the moment it
//! changes. No call waits on its reply: replies are matched as they come.
//!
//! - Pairing makes the adapter pairable for that pairing only. Without
//!   that, BlueZ pairs without bonding: the device works until it
//!   disconnects, then is unpaired again.
//! - The agent answers BlueZ while Settings pairs a device: a code to
//!   compare goes to the user, Just Works and legacy PINs need no one.
//!   It refuses pairing that Settings did not start.

use std::os::fd::RawFd;

use gh0st_dbus::bus::{self, Pending};
use gh0st_dbus::{Conn, Kind, Message, Value};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const AGENT_IFACE: &str = "org.bluez.Agent1";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const REJECTED: &str = "org.bluez.Error.Rejected";
/// Our agent's object.
const AGENT: &str = "/org/gh0st/settings/agent";

/// A device BlueZ knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dev {
    /// BlueZ's object path, `/org/bluez/hci0/dev_..`.
    pub path: String,
    pub address: String,
    pub name: String,
    /// BlueZ's kind: `audio-headset`, `input-keyboard`...
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
    /// Seen by the scan running now or the last one.
    pub found: bool,
    /// BlueZ's signal strength: it holds one only for a device discovery
    /// saw lately.
    pub rssi: Option<i16>,
    /// When the scan found it: found devices list in this order, so a new
    /// one never moves the row under the cursor.
    pub seen: u32,
    /// BlueZ dropped it some 30 s after the scan, as it does with what it
    /// never paired. It stays listed; pairing it looks for it again.
    pub gone: bool,
}

/// What Settings asked of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Pair,
    Connect,
    Disconnect,
    Forget,
}

/// What the agent needs from the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Both sides show this code: pair if they match.
    Confirm(u32),
    /// Type this code on the device.
    Show(u32),
    /// The device shows a code: type it here.
    Enter,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// The adapter or a device changed: draw again.
    Changed,
    /// An operation ended. `err` is BlueZ's error name and text.
    Done {
        op: Op,
        name: String,
        err: Option<String>,
    },
    Ask {
        name: String,
        ask: Ask,
    },
    /// BlueZ gave up on the question it asked.
    AskEnded,
    /// The adapter's power could not change.
    PowerFailed(String),
    /// No BlueZ, or it stopped.
    Failed(String),
}

#[derive(Debug)]
enum Call {
    Quiet,
    Objects,
    Power,
    Pair(String),
    /// A connect, and whether a pairing started it.
    Connect(String, bool),
    Disconnect(String),
    Forget(String),
}

pub struct Bluez {
    conn: Conn,
    pending: Pending<Call>,
    /// BlueZ's unique name: only it may call the agent.
    bluez: Option<String>,
    adapter: Option<String>,
    powered: bool,
    discovering: bool,
    /// Discovery Settings asked for and has not stopped.
    scanning: bool,
    devices: Vec<Dev>,
    /// Counts devices as scans find them.
    seen: u32,
    /// The pairing in `busy` waits for its device to be found again.
    waiting: bool,
    /// The device an operation works on, which, and its name then: a
    /// forgotten device is gone by the time the reply says so.
    busy: Option<(String, Op, String)>,
    /// The agent call that waits for the user.
    asking: Option<Message>,
    dead: bool,
}

impl Bluez {
    /// Joins the system bus and asks BlueZ what it has.
    pub fn system() -> Result<Bluez, String> {
        let conn = Conn::system().map_err(|e| format!("No system bus: {e}"))?;
        Ok(Bluez::new(conn))
    }

    /// Over `conn`, which tests connect to a fake BlueZ.
    pub fn new(conn: Conn) -> Bluez {
        let mut b = Bluez {
            conn,
            pending: Pending::new(),
            bluez: None,
            adapter: None,
            powered: false,
            discovering: false,
            scanning: false,
            devices: Vec::new(),
            seen: 0,
            waiting: false,
            busy: None,
            asking: None,
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
        let agent = Message::call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RegisterAgent")
            .arg(Value::path(AGENT))
            .arg(Value::str("KeyboardDisplay"));
        b.call(agent, Call::Quiet);
        b.flush();
        b
    }

    pub fn fd(&self) -> RawFd {
        self.conn.fd()
    }

    /// BlueZ has said which adapter there is.
    pub fn known(&self) -> bool {
        self.adapter.is_some()
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    /// A scan Settings started runs.
    pub fn scanning(&self) -> bool {
        self.scanning
    }

    pub fn devices(&self) -> &[Dev] {
        &self.devices
    }

    pub fn device(&self, path: &str) -> Option<&Dev> {
        self.devices.iter().find(|d| d.path == path)
    }

    /// The device an operation is working on, and which.
    pub fn busy(&self) -> Option<(&str, Op)> {
        self.busy.as_ref().map(|(p, o, _)| (p.as_str(), *o))
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

    fn flush(&mut self) {
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
        self.adapter = None;
        self.powered = false;
        self.scanning = false;
        self.busy = None;
        self.devices.clear();
    }

    pub fn dead(&self) -> bool {
        self.dead
    }

    fn handle(&mut self, m: Message, out: &mut Vec<Event>) {
        match m.kind {
            Kind::Return | Kind::Error => {
                if let Some(tag) = self.pending.take(&m) {
                    self.replied(tag, &m, out);
                }
            }
            Kind::Signal => self.signal(&m, out),
            Kind::Call => {
                if let Some(r) = self.called(m, out) {
                    self.send_msg(r);
                }
            }
        }
    }

    fn replied(&mut self, tag: Call, m: &Message, out: &mut Vec<Event>) {
        let err = (m.kind == Kind::Error).then(|| {
            format!(
                "{}: {}",
                m.error_name.as_deref().unwrap_or(""),
                m.error_text()
            )
        });
        match tag {
            Call::Quiet => {}
            Call::Objects if err.is_some() => self.fail(out, "Bluetooth is not running"),
            Call::Objects => {
                self.bluez = m.sender.clone();
                if let Some(objects) = m.body.first() {
                    self.objects(objects);
                }
                // No adapter yet: a bluetoothd just started announces it
                // with InterfacesAdded.
                out.push(Event::Changed);
            }
            // Just unblocked: BlueZ powers the adapter itself (AutoEnable)
            // and says Busy meanwhile. PropertiesChanged tells the end.
            Call::Power => {
                if let Some(e) = err.filter(|e| !e.contains("Busy") && !e.contains("InProgress")) {
                    out.push(Event::PowerFailed(e));
                }
            }
            Call::Pair(path) => {
                // Paired before, by BlueZ's count: connect all the same.
                let paired = err.as_ref().is_none_or(|e| e.contains("AlreadyExists"));
                self.pairable(false);
                if paired {
                    self.trust(&path);
                    let c = Message::call(BLUEZ, &path, DEVICE, "Connect");
                    self.call(c, Call::Connect(path, true));
                } else {
                    self.done(Op::Pair, &path, err, out);
                }
            }
            Call::Connect(path, paired) => {
                let op = if paired { Op::Pair } else { Op::Connect };
                self.done(op, &path, err, out);
            }
            Call::Disconnect(path) => self.done(Op::Disconnect, &path, err, out),
            Call::Forget(path) => {
                if err.is_none() {
                    self.devices.retain(|d| d.path != path);
                }
                self.done(Op::Forget, &path, err, out);
            }
        }
    }

    fn done(&mut self, op: Op, path: &str, err: Option<String>, out: &mut Vec<Event>) {
        let name = match self.busy.take_if(|b| b.0 == path) {
            Some((_, _, name)) => name,
            None => self.name_of(path),
        };
        out.push(Event::Done { op, name, err });
        out.push(Event::Changed);
    }

    /// The name a device shows, or its address.
    pub fn name_of(&self, path: &str) -> String {
        match self.device(path) {
            Some(d) if !d.name.is_empty() => d.name.clone(),
            Some(d) => d.address.clone(),
            None => path.rsplit("dev_").next().unwrap_or(path).replace('_', ":"),
        }
    }

    /// The adapter and the devices, from BlueZ's GetManagedObjects.
    fn objects(&mut self, objects: &Value) {
        let mut stuck = false;
        for (path, ifaces) in objects.entries() {
            let Some(path) = path.as_str() else { continue };
            if let Some(a) = ifaces.get(ADAPTER)
                && self.adapter.is_none()
            {
                self.adapter = Some(path.to_string());
                self.powered = flag(a, "Powered").unwrap_or(false);
                self.discovering = flag(a, "Discovering").unwrap_or(false);
                stuck = flag(a, "Pairable") == Some(true);
            }
            if let Some(d) = ifaces.get(DEVICE) {
                let mut dev = Dev {
                    path: path.to_string(),
                    ..Dev::default()
                };
                update(&mut dev, d);
                self.devices.push(dev);
            }
        }
        // Left pairable by a pairing that never ended (Settings killed
        // halfway): nothing pairs here unasked.
        if stuck && self.busy.is_none() {
            self.pairable(false);
        }
    }

    fn signal(&mut self, m: &Message, out: &mut Vec<Event>) {
        let path = m.path.as_deref().unwrap_or("");
        match (m.interface.as_deref(), m.member.as_deref()) {
            (Some(bus::PROPERTIES), Some("PropertiesChanged")) => {
                let iface = m.body.first().and_then(Value::as_str).unwrap_or("");
                let Some(changed) = m.body.get(1) else { return };
                if iface == ADAPTER && Some(path) == self.adapter.as_deref() {
                    if let Some(on) = flag(changed, "Powered") {
                        self.powered = on;
                        if !on {
                            self.scanning = false;
                        }
                    }
                    if let Some(on) = flag(changed, "Discovering") {
                        self.discovering = on;
                    }
                    out.push(Event::Changed);
                } else if iface == DEVICE {
                    self.device_changed(path, changed);
                    let gone = m.body.get(2).and_then(Value::as_array).unwrap_or(&[]);
                    if gone.iter().any(|v| v.as_str() == Some("RSSI"))
                        && let Some(d) = self.devices.iter_mut().find(|d| d.path == path)
                    {
                        d.rssi = None;
                    }
                    out.push(Event::Changed);
                }
            }
            (Some(OBJECT_MANAGER), Some("InterfacesAdded")) => {
                let Some(path) = m.body.first().and_then(Value::as_str) else {
                    return;
                };
                let Some(ifaces) = m.body.get(1) else { return };
                if let Some(d) = ifaces.get(DEVICE) {
                    self.device_changed(path, d);
                    out.push(Event::Changed);
                }
                if let Some(a) = ifaces.get(ADAPTER)
                    && self.adapter.is_none()
                {
                    self.adapter = Some(path.to_string());
                    self.powered = flag(a, "Powered").unwrap_or(false);
                    out.push(Event::Changed);
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
                    match self.devices.iter_mut().find(|d| d.path == path) {
                        Some(d) if d.found && !d.paired => {
                            d.gone = true;
                            d.connected = false;
                            d.rssi = None;
                        }
                        _ => self.devices.retain(|d| d.path != path),
                    }
                    out.push(Event::Changed);
                }
                if gone(ADAPTER) && Some(path) == self.adapter.as_deref() {
                    self.adapter = None;
                    self.powered = false;
                    self.scanning = false;
                    self.devices.clear();
                    out.push(Event::Changed);
                }
            }
            // bluetoothd went or came back: what it knew went with it.
            (Some(bus::NAME), Some("NameOwnerChanged")) => {
                let new = m.body.get(2).and_then(Value::as_str).unwrap_or("");
                self.adapter = None;
                self.bluez = None;
                self.powered = false;
                self.scanning = false;
                self.busy = None;
                if let Some(old) = self.asking.take() {
                    self.send_msg(Message::error(&old, REJECTED, "BlueZ restarted"));
                }
                self.devices.clear();
                out.push(Event::Changed);
                if !new.is_empty() {
                    self.call(
                        Message::call(BLUEZ, "/", OBJECT_MANAGER, "GetManagedObjects"),
                        Call::Objects,
                    );
                    let agent = Message::call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RegisterAgent")
                        .arg(Value::path(AGENT))
                        .arg(Value::str("KeyboardDisplay"));
                    self.call(agent, Call::Quiet);
                }
            }
            _ => {}
        }
    }

    fn device_changed(&mut self, path: &str, changed: &Value) {
        let i = match self.devices.iter().position(|d| d.path == path) {
            Some(i) => i,
            None => {
                self.devices.push(Dev {
                    path: path.to_string(),
                    ..Dev::default()
                });
                self.devices.len() - 1
            }
        };
        update(&mut self.devices[i], changed);
        self.devices[i].gone = false;
        // Found again: the pairing that waited for it goes on.
        if self.waiting && self.busy.as_ref().is_some_and(|b| b.0 == path) {
            self.waiting = false;
            self.begin_pair(path);
        }
        // BlueZ sends RSSI only for what discovery sees now.
        if self.scanning && prop(changed, "RSSI").is_some() && !self.devices[i].found {
            self.seen += 1;
            self.devices[i].found = true;
            self.devices[i].seen = self.seen;
        }
    }

    /// Agent1 and Introspect. Only BlueZ may ask; the answer goes back at
    /// once or, for a code to compare, once the user says.
    fn called(&mut self, m: Message, out: &mut Vec<Event>) -> Option<Message> {
        if m.no_reply() {
            return None;
        }
        if m.path.as_deref() != Some(AGENT) || m.interface.as_deref() != Some(AGENT_IFACE) {
            return Some(Message::error(
                &m,
                bus::err::UNKNOWN_METHOD,
                "no such method",
            ));
        }
        if self.bluez.is_some() && m.sender != self.bluez {
            return Some(Message::error(&m, REJECTED, "not BlueZ"));
        }
        let member = m.member.clone().unwrap_or_default();
        let device = m
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let ours = self
            .busy
            .as_ref()
            .is_some_and(|(p, o, _)| *p == device && *o == Op::Pair);
        let code = m.body.get(1).and_then(Value::as_u32).unwrap_or(0);
        match member.as_str() {
            "Release" => Some(Message::reply(&m)),
            "Cancel" => {
                if let Some(old) = self.asking.take() {
                    self.send_msg(Message::error(&old, REJECTED, "cancelled"));
                }
                out.push(Event::AskEnded);
                Some(Message::reply(&m))
            }
            // A service of a paired device, or of the one being paired.
            "AuthorizeService" => {
                let known = self.device(&device).is_some_and(|d| d.paired);
                Some(if ours || known {
                    Message::reply(&m)
                } else {
                    Message::error(&m, REJECTED, "not paired")
                })
            }
            _ if !ours => Some(Message::error(&m, REJECTED, "Settings is not pairing it")),
            // Just Works, started here: nothing to compare.
            "RequestAuthorization" => Some(Message::reply(&m)),
            // An old device's fixed PIN.
            "RequestPinCode" => Some(Message::reply(&m).arg(Value::str("0000"))),
            "DisplayPasskey" | "DisplayPinCode" => {
                let code = match member.as_str() {
                    "DisplayPinCode" => m
                        .body
                        .get(1)
                        .and_then(Value::as_str)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    _ => code,
                };
                out.push(Event::Ask {
                    name: self.name_of(&device),
                    ask: Ask::Show(code),
                });
                Some(Message::reply(&m))
            }
            "RequestConfirmation" | "RequestPasskey" => {
                if let Some(old) = self.asking.take() {
                    self.send_msg(Message::error(&old, REJECTED, "superseded"));
                }
                let ask = if member == "RequestPasskey" {
                    Ask::Enter
                } else {
                    Ask::Confirm(code)
                };
                out.push(Event::Ask {
                    name: self.name_of(&device),
                    ask,
                });
                self.asking = Some(m);
                None
            }
            _ => Some(Message::error(&m, REJECTED, "not supported")),
        }
    }

    /// The user's answer to the code to compare.
    pub fn answer(&mut self, yes: bool) {
        let Some(m) = self.asking.take() else {
            return;
        };
        let r = if yes {
            Message::reply(&m)
        } else {
            Message::error(&m, REJECTED, "the codes did not match")
        };
        self.send_msg(r);
        self.flush();
    }

    /// The code the device showed, typed here.
    pub fn passkey(&mut self, code: u32) {
        let Some(m) = self.asking.take() else {
            return;
        };
        self.send_msg(Message::reply(&m).arg(Value::U32(code)));
        self.flush();
    }

    fn set(&mut self, path: &str, iface: &str, prop: &str, v: Value, tag: Call) {
        let m = Message::call(BLUEZ, path, bus::PROPERTIES, "Set")
            .arg(Value::str(iface))
            .arg(Value::str(prop))
            .arg(Value::variant(v));
        self.call(m, tag);
    }

    fn pairable(&mut self, on: bool) {
        if let Some(a) = self.adapter.clone() {
            self.set(&a, ADAPTER, "Pairable", Value::Bool(on), Call::Quiet);
        }
    }

    /// Trusted: BlueZ lets it back in without asking the agent.
    fn trust(&mut self, path: &str) {
        self.set(path, DEVICE, "Trusted", Value::Bool(true), Call::Quiet);
    }

    pub fn power(&mut self, on: bool) {
        if let Some(a) = self.adapter.clone() {
            self.set(&a, ADAPTER, "Powered", Value::Bool(on), Call::Power);
            self.flush();
        }
    }

    /// Starts or stops looking for devices. What a new scan finds replaces
    /// what the last one did.
    pub fn scan(&mut self, on: bool) {
        let Some(a) = self.adapter.clone() else {
            return;
        };
        if on == self.scanning {
            return;
        }
        self.scanning = on;
        if on {
            // A new scan finds again what is still there.
            let wait = self
                .busy
                .as_ref()
                .filter(|_| self.waiting)
                .map(|b| b.0.clone());
            self.devices
                .retain(|d| !d.gone || Some(&d.path) == wait.as_ref());
            // What BlueZ saw lately counts: it may not report it again until
            // its signal moves.
            for d in &mut self.devices {
                d.found = d.rssi.is_some();
                if d.found {
                    self.seen += 1;
                    d.seen = self.seen;
                }
            }
        }
        let member = if on {
            "StartDiscovery"
        } else {
            "StopDiscovery"
        };
        self.call(Message::call(BLUEZ, &a, ADAPTER, member), Call::Quiet);
        self.flush();
    }

    /// One operation at a time: false while another runs.
    fn start(&mut self, path: &str, op: Op) -> bool {
        if self.busy.is_some() || self.device(path).is_none() {
            return false;
        }
        self.busy = Some((path.to_string(), op, self.name_of(path)));
        true
    }

    /// Pairs, trusts and connects.
    pub fn pair(&mut self, path: &str) -> bool {
        if !self.start(path, Op::Pair) {
            return false;
        }
        if self.device(path).is_some_and(|d| d.gone) {
            self.waiting = true;
            self.scan(true);
            return true;
        }
        self.begin_pair(path);
        true
    }

    fn begin_pair(&mut self, path: &str) {
        // Discovery slows a pairing's page scan down.
        self.scan(false);
        self.pairable(true);
        let m = Message::call(BLUEZ, path, DEVICE, "Pair");
        self.call(m, Call::Pair(path.to_string()));
        self.flush();
    }

    /// Ends the scan Settings started. A pairing still waiting for its
    /// device gives up: its device's name.
    pub fn end_scan(&mut self) -> Option<String> {
        self.scan(false);
        if !std::mem::take(&mut self.waiting) {
            return None;
        }
        self.busy.take().map(|(_, _, name)| name)
    }

    pub fn connect(&mut self, path: &str) -> bool {
        if !self.start(path, Op::Connect) {
            return false;
        }
        self.scan(false);
        let m = Message::call(BLUEZ, path, DEVICE, "Connect");
        self.call(m, Call::Connect(path.to_string(), false));
        self.flush();
        true
    }

    pub fn disconnect(&mut self, path: &str) -> bool {
        if !self.start(path, Op::Disconnect) {
            return false;
        }
        let m = Message::call(BLUEZ, path, DEVICE, "Disconnect");
        self.call(m, Call::Disconnect(path.to_string()));
        self.flush();
        true
    }

    /// Unpairs and drops it: BlueZ forgets the keys and the device.
    pub fn forget(&mut self, path: &str) -> bool {
        let Some(a) = self.adapter.clone() else {
            return false;
        };
        if !self.start(path, Op::Forget) {
            return false;
        }
        let m = Message::call(BLUEZ, &a, ADAPTER, "RemoveDevice").arg(Value::path(path));
        self.call(m, Call::Forget(path.to_string()));
        self.flush();
        true
    }
}

/// What the Bluetooth page lists: paired devices by name, connected first,
/// then what the last scan found that has a name, in the order it found
/// them. A device BlueZ only remembers from long ago, or one that
/// reconnects after a forget, stays off the list.
pub fn listed(devices: &[Dev]) -> Vec<&Dev> {
    let mut v: Vec<&Dev> = devices
        .iter()
        .filter(|d| d.paired || (d.found && !d.name.is_empty()))
        .collect();
    v.sort_by_key(|d| {
        let name = if d.paired {
            d.name.to_lowercase()
        } else {
            String::new()
        };
        (!d.connected, !d.paired, name, d.seen)
    });
    v
}

fn prop<'a>(props: &'a Value, key: &str) -> Option<&'a Value> {
    props.get(key).map(Value::inner)
}

fn flag(props: &Value, key: &str) -> Option<bool> {
    prop(props, key).and_then(Value::as_bool)
}

fn prop_str(props: &Value, key: &str) -> Option<String> {
    prop(props, key).and_then(Value::as_str).map(String::from)
}

/// A device's fields from BlueZ's properties, changed or all.
fn update(d: &mut Dev, props: &Value) {
    if let Some(a) = prop_str(props, "Address") {
        d.address = a;
    }
    // A device with no name shows its address as Alias, with dashes.
    // Some devices pad their names with spaces.
    if let Some(n) = prop_str(props, "Name") {
        d.name = n.trim().to_string();
    } else if let Some(n) = prop_str(props, "Alias")
        && n.replace('-', ":") != d.address
    {
        d.name = n.trim().to_string();
    }
    if let Some(i) = prop_str(props, "Icon") {
        d.icon = i;
    }
    if let Some(p) = flag(props, "Paired") {
        d.paired = p;
    }
    if let Some(c) = flag(props, "Connected") {
        d.connected = c;
    }
    if let Some(Value::I16(r)) = prop(props, "RSSI") {
        d.rssi = Some(*r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, paired: bool, connected: bool, found: bool) -> Dev {
        Dev {
            path: format!("/org/bluez/hci0/dev_{name}"),
            name: name.into(),
            paired,
            connected,
            found,
            ..Dev::default()
        }
    }

    #[test]
    fn the_list_is_paired_then_found_with_a_name() {
        let mut all = [
            dev("Speaker", false, false, true),
            dev("Old", false, false, false),
            dev("", false, false, true),
            dev("Buds", true, false, false),
            dev("Keys", true, true, false),
            dev("Amp", false, false, true),
        ];
        all[0].seen = 1;
        all[5].seen = 2;
        let names: Vec<&str> = listed(&all).iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            ["Keys", "Buds", "Speaker", "Amp"],
            "found ones keep their order"
        );
    }

    /// A call from BlueZ to the agent, and the reply it gets.
    fn agent_call(b: &mut Bluez, peer: &mut Conn, member: &str, device: &str) -> Message {
        let m = Message::call(":1.1", AGENT, AGENT_IFACE, member)
            .arg(Value::path(device))
            .arg(Value::U32(123456));
        peer.send(m).unwrap();
        peer.flush().unwrap();
        let mut out = Vec::new();
        b.ready(&mut out);
        loop {
            peer.fill().unwrap();
            if let Some(r) = peer.recv().unwrap()
                && matches!(r.kind, Kind::Return | Kind::Error)
            {
                return r;
            }
        }
    }

    #[test]
    fn the_agent_answers_only_for_a_pairing_settings_started() {
        let (a, p) = gh0st_dbus::conn::pair().unwrap();
        let mut b = Bluez::new(Conn::raw(a).unwrap());
        let mut peer = Conn::raw(p).unwrap();
        let path = "/org/bluez/hci0/dev_X";
        b.devices.push(dev("X", false, false, true));
        b.devices[0].path = path.into();

        let r = agent_call(&mut b, &mut peer, "RequestAuthorization", path);
        assert_eq!(r.error_name.as_deref(), Some(REJECTED), "not pairing");

        b.busy = Some((path.into(), Op::Pair, "X".into()));
        let r = agent_call(&mut b, &mut peer, "RequestAuthorization", path);
        assert_eq!(r.kind, Kind::Return, "Just Works, started here");
        let r = agent_call(
            &mut b,
            &mut peer,
            "RequestAuthorization",
            "/org/bluez/hci0/dev_Y",
        );
        assert_eq!(r.error_name.as_deref(), Some(REJECTED), "another device");
    }
}
