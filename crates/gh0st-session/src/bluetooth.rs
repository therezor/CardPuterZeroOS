//! Bluetooth headphones and speakers come back on their own.
//!
//! BlueZ reconnects a classic audio device only after a dropped link, never
//! when Bluetooth turns on. So when the adapter powers on, or the session
//! starts with it on, this connects every paired, trusted device that
//! plays sound, as a phone does. It follows the adapter's Powered on the
//! system bus and nothing else: no timer, and devices a scan finds never
//! reach it.

use gh0st_dbus::bus::{self, Pending};
use gh0st_dbus::{Conn, Kind, Message, Value};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
/// Audio Sink, Handsfree and Headset: what plays sound to the user.
const AUDIO: [&str; 3] = [
    "0000110b-0000-1000-8000-00805f9b34fb",
    "0000111e-0000-1000-8000-00805f9b34fb",
    "00001108-0000-1000-8000-00805f9b34fb",
];

#[derive(Debug)]
enum Call {
    Quiet,
    Objects,
}

#[derive(Default)]
pub struct Reconnect {
    conn: Option<Conn>,
    pending: Pending<Call>,
    powered: bool,
}

impl Reconnect {
    /// Joins the system bus. Without one, or without BlueZ, nothing runs.
    pub fn open() -> Reconnect {
        match Conn::system() {
            Ok(c) => Reconnect::new(c),
            Err(_) => Reconnect::default(),
        }
    }

    /// Over `conn`, which tests connect to a fake BlueZ.
    pub fn new(conn: Conn) -> Reconnect {
        let mut r = Reconnect {
            conn: Some(conn),
            ..Reconnect::default()
        };
        for rule in [
            format!(
                "type='signal',sender='{BLUEZ}',interface='{}',member='PropertiesChanged',arg0='{ADAPTER}'",
                bus::PROPERTIES
            ),
            format!(
                "type='signal',sender='{}',member='NameOwnerChanged',arg0='{BLUEZ}'",
                bus::NAME
            ),
        ] {
            let m =
                Message::call(bus::NAME, bus::PATH, bus::NAME, "AddMatch").arg(Value::str(rule));
            r.call(m, Call::Quiet);
        }
        r.ask();
        r
    }

    pub fn fd(&self) -> Option<std::os::fd::RawFd> {
        self.conn.as_ref().map(Conn::fd)
    }

    pub fn wants_write(&self) -> bool {
        self.conn.as_ref().is_some_and(Conn::wants_write)
    }

    fn call(&mut self, m: Message, tag: Call) {
        let Some(c) = self.conn.as_mut() else { return };
        if self.pending.send(c, m, tag).is_err() {
            self.conn = None;
        }
    }

    /// What BlueZ has: the reply reconnects, if the adapter is on.
    fn ask(&mut self) {
        let m = Message::call(BLUEZ, "/", OBJECT_MANAGER, "GetManagedObjects");
        self.call(m, Call::Objects);
        if let Some(c) = self.conn.as_mut() {
            let _ = c.flush();
        }
    }

    /// The system bus woke the loop.
    pub fn ready(&mut self) {
        let mut got = Vec::new();
        if let Some(c) = &mut self.conn {
            if c.flush().and_then(|()| c.fill()).is_err() || c.closed() {
                self.conn = None;
                return;
            }
            while let Ok(Some(m)) = c.recv() {
                got.push(m);
            }
        }
        for m in got {
            self.take(&m);
        }
        if let Some(c) = self.conn.as_mut() {
            let _ = c.flush();
        }
    }

    fn take(&mut self, m: &Message) {
        match m.kind {
            Kind::Return | Kind::Error => {
                if let Some(Call::Objects) = self.pending.take(m)
                    && m.kind == Kind::Return
                    && let Some(objects) = m.body.first()
                {
                    self.objects(objects);
                }
            }
            Kind::Signal if m.is(bus::PROPERTIES, "PropertiesChanged") => {
                let on = m
                    .body
                    .get(1)
                    .and_then(|c| c.get("Powered"))
                    .map(Value::inner)
                    .and_then(Value::as_bool);
                match on {
                    Some(true) if !self.powered => self.ask(),
                    Some(on) => self.powered = on,
                    None => {}
                }
            }
            // bluetoothd started again.
            Kind::Signal if m.is(bus::NAME, "NameOwnerChanged") => {
                let new = m.body.get(2).and_then(Value::as_str).unwrap_or("");
                if !new.is_empty() {
                    self.powered = false;
                    self.ask();
                }
            }
            _ => {}
        }
    }

    /// Connects the audio devices, once the adapter is on.
    fn objects(&mut self, objects: &Value) {
        let mut adapter_on = false;
        let mut wanted = Vec::new();
        for (path, ifaces) in objects.entries() {
            let Some(path) = path.as_str() else { continue };
            if let Some(a) = ifaces.get(ADAPTER) {
                adapter_on |= flag(a, "Powered");
            }
            if let Some(d) = ifaces.get(DEVICE)
                && comes_back(d)
            {
                wanted.push(path.to_string());
            }
        }
        self.powered = adapter_on;
        if !adapter_on {
            return;
        }
        for path in wanted {
            self.call(Message::call(BLUEZ, &path, DEVICE, "Connect"), Call::Quiet);
        }
    }
}

fn flag(props: &Value, key: &str) -> bool {
    props
        .get(key)
        .map(Value::inner)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Paired, trusted, not connected, and it plays sound.
fn comes_back(d: &Value) -> bool {
    let audio = d
        .get("UUIDs")
        .map(Value::inner)
        .and_then(Value::as_array)
        .is_some_and(|u| {
            u.iter()
                .any(|u| u.as_str().is_some_and(|u| AUDIO.contains(&u)))
        });
    flag(d, "Paired") && flag(d, "Trusted") && !flag(d, "Connected") && audio
}

#[cfg(test)]
mod tests {
    use super::*;
    use gh0st_dbus::Ty;

    fn props_ty() -> Ty {
        Ty::Array(Box::new(Ty::Entry(
            Box::new(Ty::Str),
            Box::new(Ty::Variant),
        )))
    }

    fn objects(powered: bool) -> Value {
        let ifaces_ty = Ty::Array(Box::new(Ty::Entry(Box::new(Ty::Str), Box::new(props_ty()))));
        let adapter = Value::dict(
            Ty::Str,
            props_ty(),
            [(
                Value::str(ADAPTER),
                Value::props([("Powered", Value::Bool(powered))]),
            )],
        );
        let dev = |paired, trusted, connected, uuid: &str| {
            Value::dict(
                Ty::Str,
                props_ty(),
                [(
                    Value::str(DEVICE),
                    Value::props([
                        ("Paired", Value::Bool(paired)),
                        ("Trusted", Value::Bool(trusted)),
                        ("Connected", Value::Bool(connected)),
                        ("UUIDs", Value::strs([uuid])),
                    ]),
                )],
            )
        };
        let sink = AUDIO[0];
        let phone = "0000111f-0000-1000-8000-00805f9b34fb";
        Value::dict(
            Ty::Path,
            ifaces_ty,
            [
                (Value::path("/org/bluez/hci0"), adapter),
                (
                    Value::path("/org/bluez/hci0/dev_SPEAKER"),
                    dev(true, true, false, sink),
                ),
                (
                    Value::path("/org/bluez/hci0/dev_UNTRUSTED"),
                    dev(true, false, false, sink),
                ),
                (
                    Value::path("/org/bluez/hci0/dev_ON"),
                    dev(true, true, true, sink),
                ),
                (
                    Value::path("/org/bluez/hci0/dev_PHONE"),
                    dev(true, true, false, phone),
                ),
            ],
        )
    }

    /// Answers GetManagedObjects with `powered` and returns the Connects.
    fn round(r: &mut Reconnect, peer: &mut Conn, powered: bool) -> Vec<String> {
        let mut connects = Vec::new();
        for _ in 0..50 {
            peer.fill().unwrap();
            while let Some(m) = peer.recv().unwrap() {
                match m.member.as_deref() {
                    Some("GetManagedObjects") => {
                        peer.send(Message::reply(&m).arg(objects(powered))).unwrap();
                    }
                    Some("Connect") => connects.push(m.path.clone().unwrap_or_default()),
                    _ => {
                        peer.send(Message::reply(&m)).unwrap();
                    }
                }
            }
            peer.flush().unwrap();
            r.ready();
        }
        connects
    }

    #[test]
    fn audio_devices_come_back_when_the_adapter_powers_on() {
        let (a, b) = gh0st_dbus::conn::pair().unwrap();
        let mut peer = Conn::raw(b).unwrap();
        let mut r = Reconnect::new(Conn::raw(a).unwrap());
        assert!(round(&mut r, &mut peer, false).is_empty(), "off: nothing");

        let on = Message::signal("/org/bluez/hci0", bus::PROPERTIES, "PropertiesChanged")
            .arg(Value::str(ADAPTER))
            .arg(Value::props([("Powered", Value::Bool(true))]))
            .arg(Value::strs::<&str>([]));
        peer.send(on).unwrap();
        peer.flush().unwrap();
        assert_eq!(
            round(&mut r, &mut peer, true),
            ["/org/bluez/hci0/dev_SPEAKER"]
        );
    }
}
