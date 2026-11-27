//! A fake BlueZ in a thread, on the other end of a socket pair: the bus
//! daemon's AddMatch, the adapter, devices that pair, connect and go, and
//! the signals bluetoothd sends for each. It logs every call it takes.

#![allow(dead_code)]

use gh0st_dbus::{Conn, Kind, Message, Ty, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const ADAPTER: &str = "/org/bluez/hci0";
const DEVICE: &str = "org.bluez.Device1";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const OM: &str = "org.freedesktop.DBus.ObjectManager";

#[derive(Clone, Debug)]
pub struct Dev {
    pub path: String,
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// Unknown until a scan finds it.
    pub hidden: bool,
    /// Its Pair call fails with this error.
    pub refuse: Option<&'static str>,
}

impl Dev {
    pub fn new(address: &str, name: &str) -> Dev {
        Dev {
            path: format!("{ADAPTER}/dev_{}", address.replace(':', "_")),
            address: address.into(),
            name: name.into(),
            paired: false,
            connected: false,
            hidden: false,
            refuse: None,
        }
    }
}

struct State {
    powered: bool,
    devices: Vec<Dev>,
}

pub struct Fake {
    pub log: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    /// Every call so far, as "Member path" or "Set path Prop=value".
    pub fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    pub fn called(&self, what: &str) -> bool {
        self.calls().iter().any(|c| c == what)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// BlueZ with `devices`, powered or not. The Conn is the app's end.
pub fn start(powered: bool, devices: Vec<Dev>) -> (Conn, Fake) {
    let (a, b) = gh0st_dbus::conn::pair().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (log2, stop2) = (log.clone(), stop.clone());
    let thread = std::thread::spawn(move || {
        let mut conn = Conn::raw(b).unwrap();
        let mut st = State { powered, devices };
        while !stop2.load(Ordering::SeqCst) {
            let mut fds = [gh0st_core::rt::pollfd(conn.fd())];
            let _ = gh0st_core::rt::wait(&mut fds, Some(gh0st_core::rt::now_ms() + 20));
            if conn.fill().is_err() || conn.closed() {
                return;
            }
            while let Ok(Some(m)) = conn.recv() {
                if m.kind == Kind::Call {
                    handle(&mut conn, &mut st, &log2, m);
                }
            }
            let _ = conn.flush();
        }
    });
    (
        Conn::raw(a).unwrap(),
        Fake {
            log,
            stop,
            thread: Some(thread),
        },
    )
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

fn dev_props(d: &Dev, rssi: bool) -> Value {
    let mut p = vec![
        ("Address", Value::str(&d.address)),
        ("Name", Value::str(&d.name)),
        ("Alias", Value::str(&d.name)),
        ("Icon", Value::str("audio-card")),
        ("Paired", Value::Bool(d.paired)),
        ("Connected", Value::Bool(d.connected)),
    ];
    if rssi {
        p.push(("RSSI", Value::I16(-60)));
    }
    Value::props(p)
}

fn objects(st: &State) -> Value {
    let adapter = Value::dict(
        Ty::Str,
        props_ty(),
        [(
            Value::str("org.bluez.Adapter1"),
            Value::props([
                ("Powered", Value::Bool(st.powered)),
                ("Discovering", Value::Bool(false)),
                ("Pairable", Value::Bool(false)),
            ]),
        )],
    );
    let mut all = vec![(Value::path(ADAPTER), adapter)];
    for d in st.devices.iter().filter(|d| !d.hidden) {
        let ifaces = Value::dict(
            Ty::Str,
            props_ty(),
            [(Value::str(DEVICE), dev_props(d, false))],
        );
        all.push((Value::path(&d.path), ifaces));
    }
    Value::dict(Ty::Path, ifaces_ty(), all)
}

fn changed(conn: &mut Conn, path: &str, iface: &str, prop: &str, v: Value) {
    let s = Message::signal(path, PROPS, "PropertiesChanged")
        .arg(Value::str(iface))
        .arg(Value::props([(prop, v)]))
        .arg(Value::strs::<&str>([]));
    let _ = conn.send(s);
}

fn handle(conn: &mut Conn, st: &mut State, log: &Mutex<Vec<String>>, m: Message) {
    let path = m.path.clone().unwrap_or_default();
    let member = m.member.clone().unwrap_or_default();
    let mut reply = Message::reply(&m);
    match member.as_str() {
        "AddMatch" | "RegisterAgent" => {}
        "GetManagedObjects" => reply = reply.arg(objects(st)),
        "Set" => {
            let prop = m
                .body
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let raw = m.body.get(2).map(Value::inner);
            let v = raw.and_then(Value::as_bool);
            let shown = match (v, raw.and_then(Value::as_u32)) {
                (Some(b), _) => b.to_string(),
                (None, Some(n)) => n.to_string(),
                _ => "?".into(),
            };
            log.lock()
                .unwrap()
                .push(format!("Set {path} {prop}={shown}"));
            if path == ADAPTER && prop == "Powered" {
                st.powered = v.unwrap_or(false);
                let _ = conn.send(reply);
                changed(
                    conn,
                    ADAPTER,
                    "org.bluez.Adapter1",
                    "Powered",
                    Value::Bool(st.powered),
                );
                return;
            }
        }
        "StartDiscovery" => {
            log.lock().unwrap().push(format!("{member} {path}"));
            let _ = conn.send(reply);
            changed(
                conn,
                ADAPTER,
                "org.bluez.Adapter1",
                "Discovering",
                Value::Bool(true),
            );
            for d in st.devices.iter_mut().filter(|d| d.hidden) {
                d.hidden = false;
                let ifaces = Value::dict(
                    Ty::Str,
                    props_ty(),
                    [(Value::str(DEVICE), dev_props(d, true))],
                );
                let s = Message::signal("/", OM, "InterfacesAdded")
                    .arg(Value::path(&d.path))
                    .arg(ifaces);
                let _ = conn.send(s);
            }
            return;
        }
        "StopDiscovery" => {
            log.lock().unwrap().push(format!("{member} {path}"));
            let _ = conn.send(reply);
            changed(
                conn,
                ADAPTER,
                "org.bluez.Adapter1",
                "Discovering",
                Value::Bool(false),
            );
            // BlueZ drops what it found and never paired, some 30 s later.
            for d in st.devices.iter_mut().filter(|d| !d.paired && !d.hidden) {
                d.hidden = true;
                let s = Message::signal("/", OM, "InterfacesRemoved")
                    .arg(Value::path(&d.path))
                    .arg(Value::strs([DEVICE]));
                let _ = conn.send(s);
            }
            return;
        }
        "Pair" | "Connect" | "Disconnect" => {
            log.lock().unwrap().push(format!("{member} {path}"));
            let Some(d) = st.devices.iter_mut().find(|d| d.path == path) else {
                let e = Message::error(&m, "org.bluez.Error.DoesNotExist", "Does Not Exist");
                let _ = conn.send(e);
                return;
            };
            let (prop, on) = match member.as_str() {
                "Pair" => {
                    if let Some(err) = d.refuse {
                        let _ = conn.send(Message::error(&m, err, "refused"));
                        return;
                    }
                    d.paired = true;
                    ("Paired", true)
                }
                "Connect" => {
                    d.connected = true;
                    ("Connected", true)
                }
                _ => {
                    d.connected = false;
                    ("Connected", false)
                }
            };
            let p = d.path.clone();
            changed(conn, &p, DEVICE, prop, Value::Bool(on));
        }
        "RemoveDevice" => {
            let gone = m
                .body
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            log.lock().unwrap().push(format!("{member} {gone}"));
            st.devices.retain(|d| d.path != gone);
            let s = Message::signal("/", OM, "InterfacesRemoved")
                .arg(Value::path(&gone))
                .arg(Value::strs([DEVICE]));
            let _ = conn.send(reply);
            let _ = conn.send(s);
            return;
        }
        _ => {
            log.lock().unwrap().push(format!("{member} {path}"));
        }
    }
    let _ = conn.send(reply);
}
