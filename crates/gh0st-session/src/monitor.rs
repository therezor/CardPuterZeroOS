//! The portal's NetworkMonitor and PowerProfileMonitor.
//!
//! NetworkMonitor follows NetworkManager on the system bus. The connection
//! opens when an app first asks, so a session no app asks keeps none. Its
//! PropertiesChanged signals then join the poll loop; `changed` goes out
//! when what apps see moves.
//!
//! PowerProfileMonitor is Settings' CPU mode: Power Saver is the
//! `powersave` governor. Settings writes the mode as a udev rule as well
//! as to sysfs, and the rule's folder is watched: sysfs sends no events.

use gh0st_dbus::bus::{self, err};
use gh0st_dbus::{Conn, Kind, Message, Value};

pub const NETWORK: &str = "org.freedesktop.portal.NetworkMonitor";
pub const NETWORK_VERSION: u32 = 3;
pub const POWER: &str = "org.freedesktop.portal.PowerProfileMonitor";
pub const POWER_VERSION: u32 = 1;
const PATH: &str = "/org/freedesktop/portal/desktop";
const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
/// Where Settings keeps the CPU mode for the next boot.
pub const RULES: &str = "/etc/udev/rules.d";
const GOVERNOR: &str = "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor";

/// What NetworkManager says, as it says it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nm {
    /// NMConnectivityState: 0 unknown, 1 none, 2 portal, 3 limited, 4 full.
    pub connectivity: u32,
    /// NMMetered: 1 yes and 3 guess-yes are metered.
    pub metered: u32,
    /// NMState: 70 connected to the internet, 60 to a site, 50 local.
    pub state: u32,
}

/// What the portal says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub available: bool,
    pub metered: bool,
    /// 1 local only, 2 limited, 3 behind a captive portal, 4 full.
    pub connectivity: u32,
}

impl Status {
    /// Without NetworkManager there is nothing to say no with.
    const UNKNOWN: Status = Status {
        available: true,
        metered: false,
        connectivity: 4,
    };
}

impl Nm {
    pub fn status(&self) -> Status {
        // With its check off, as the image ships it, NetworkManager knows
        // no connectivity: its state stands in.
        let connectivity = match (self.connectivity, self.state) {
            (4, _) => 4,
            (3, _) => 2,
            (2, _) => 3,
            (1, _) => 1,
            (_, 70..) => 4,
            (_, 60..) => 2,
            _ => 1,
        };
        Status {
            available: connectivity >= 2,
            metered: matches!(self.metered, 1 | 3),
            connectivity,
        }
    }

    /// Takes NetworkManager's properties: from GetAll, or the changed ones
    /// of a PropertiesChanged.
    pub fn apply(&mut self, props: &Value) {
        let u = |k| props.get(k).map(Value::inner).and_then(Value::as_u32);
        if let Some(c) = u("Connectivity") {
            self.connectivity = c;
        }
        if let Some(m) = u("Metered") {
            self.metered = m;
        }
        if let Some(s) = u("State") {
            self.state = s;
        }
    }
}

#[derive(Default)]
pub struct Network {
    conn: Option<Conn>,
    /// The connection was tried and failed: answers stay unknown.
    tried: bool,
    nm: Nm,
    /// Signals for the session bus.
    pub out: Vec<Message>,
}

impl Network {
    /// The system bus, while one is open: for the poll loop.
    pub fn fd(&self) -> Option<std::os::fd::RawFd> {
        self.conn.as_ref().map(Conn::fd)
    }

    pub fn wants_write(&self) -> bool {
        self.conn.as_ref().is_some_and(Conn::wants_write)
    }

    fn status(&self) -> Status {
        match self.conn {
            Some(_) => self.nm.status(),
            None => Status::UNKNOWN,
        }
    }

    /// Opens the system bus and reads NetworkManager once. A missing bus
    /// or NetworkManager leaves the status unknown.
    fn connect(&mut self) {
        if self.conn.is_some() || self.tried {
            return;
        }
        self.tried = true;
        let Ok(mut c) = Conn::system() else {
            return;
        };
        let rule = format!(
            "type='signal',sender='{NM}',path='{NM_PATH}',interface='{}',member='PropertiesChanged'",
            bus::PROPERTIES
        );
        if bus::add_match(&mut c, &rule).is_err() {
            return;
        }
        // A bus without NetworkManager knows nothing: the status stays
        // unknown, as with no bus.
        let get = Message::call(NM, NM_PATH, bus::PROPERTIES, "GetAll").arg(Value::str(NM));
        let Some(props) = c
            .call(get, 1000)
            .ok()
            .and_then(|r| r.body.into_iter().next())
        else {
            return;
        };
        self.nm.apply(&props);
        self.conn = Some(c);
    }

    pub fn call(&mut self, m: &Message) -> Message {
        self.connect();
        let s = self.status();
        match m.member.as_deref().unwrap_or("") {
            "GetAvailable" => Message::reply(m).arg(Value::Bool(s.available)),
            "GetMetered" => Message::reply(m).arg(Value::Bool(s.metered)),
            "GetConnectivity" => Message::reply(m).arg(Value::U32(s.connectivity)),
            "GetStatus" => Message::reply(m).arg(Value::props([
                ("available", Value::Bool(s.available)),
                ("metered", Value::Bool(s.metered)),
                ("connectivity", Value::U32(s.connectivity)),
            ])),
            // No name lookup in the loop: a host is reachable when the
            // network is.
            "CanReach" => Message::reply(m).arg(Value::Bool(s.available)),
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// The system bus woke the loop: NetworkManager's changes.
    pub fn ready(&mut self) {
        let before = self.status();
        let mut lost = false;
        if let Some(c) = &mut self.conn {
            if c.flush().and_then(|()| c.fill()).is_err() || c.closed() {
                lost = true;
            }
            while let Ok(Some(m)) = c.recv() {
                if m.kind == Kind::Signal
                    && m.is(bus::PROPERTIES, "PropertiesChanged")
                    && m.body.first().and_then(Value::as_str) == Some(NM)
                {
                    if let Some(changed) = m.body.get(1) {
                        self.nm.apply(changed);
                    }
                }
            }
        }
        if lost {
            // The bus went away: ask again on the next call.
            self.conn = None;
            self.tried = false;
        }
        if self.status() != before {
            self.out.push(Message::signal(PATH, NETWORK, "changed"));
        }
    }
}

/// Power Saver is on.
#[derive(Default)]
pub struct Power {
    pub saver: bool,
    /// Signals for the session bus.
    pub out: Vec<Message>,
}

fn power_saver() -> bool {
    std::fs::read_to_string(GOVERNOR).is_ok_and(|g| g.trim() == "powersave")
}

impl Power {
    pub fn new() -> Power {
        Power {
            saver: power_saver(),
            out: Vec::new(),
        }
    }

    /// The rules folder changed: the CPU mode may have.
    pub fn changed(&mut self) {
        let now = power_saver();
        if now != self.saver {
            self.saver = now;
            self.out.push(
                Message::signal(PATH, bus::PROPERTIES, "PropertiesChanged")
                    .arg(Value::str(POWER))
                    .arg(Value::props([("power-saver-enabled", Value::Bool(now))]))
                    .arg(Value::strs(Vec::<String>::new())),
            );
        }
    }

    /// Properties.Get and GetAll for the interface.
    pub fn properties(&self, m: &Message) -> Message {
        let all = [
            ("power-saver-enabled", Value::Bool(self.saver)),
            ("version", Value::U32(POWER_VERSION)),
        ];
        if m.is(bus::PROPERTIES, "GetAll") {
            return Message::reply(m).arg(Value::props(all));
        }
        let name = m.body.get(1).and_then(Value::as_str).unwrap_or("");
        match all.into_iter().find(|(k, _)| *k == name) {
            Some((_, v)) => Message::reply(m).arg(Value::variant(v)),
            None => Message::error(m, err::UNKNOWN_PROPERTY, "no such property"),
        }
    }
}

pub const NETWORK_XML: &str = r#"<interface name="org.freedesktop.portal.NetworkMonitor"><method name="GetAvailable"><arg type="b" direction="out"/></method><method name="GetMetered"><arg type="b" direction="out"/></method><method name="GetConnectivity"><arg type="u" direction="out"/></method><method name="GetStatus"><arg type="a{sv}" direction="out"/></method><method name="CanReach"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="b" direction="out"/></method><signal name="changed"/><property name="version" type="u" access="read"/></interface>"#;
pub const POWER_XML: &str = r#"<interface name="org.freedesktop.portal.PowerProfileMonitor"><property name="power-saver-enabled" type="b" access="read"/><property name="version" type="u" access="read"/></interface>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networkmanager_maps_to_what_apps_see() {
        let s = |connectivity, metered, state| {
            Nm {
                connectivity,
                metered,
                state,
            }
            .status()
        };
        assert_eq!(
            s(4, 2, 70),
            Status {
                available: true,
                metered: false,
                connectivity: 4
            }
        );
        assert_eq!(s(2, 2, 60).connectivity, 3, "a captive portal");
        assert_eq!(
            s(3, 1, 60),
            Status {
                available: true,
                metered: true,
                connectivity: 2
            }
        );
        assert!(!s(1, 0, 20).available);
        // The check off: the state says it.
        assert_eq!(s(0, 4, 70).connectivity, 4);
        assert_eq!(
            s(0, 3, 50),
            Status {
                available: false,
                metered: true,
                connectivity: 1
            }
        );
    }

    #[test]
    fn changed_properties_update_only_what_they_name() {
        let mut nm = Nm {
            connectivity: 4,
            metered: 2,
            state: 70,
        };
        nm.apply(&Value::props([("Metered", Value::variant(Value::U32(1)))]));
        assert_eq!(
            nm,
            Nm {
                connectivity: 4,
                metered: 1,
                state: 70
            }
        );
    }

    #[test]
    fn without_networkmanager_the_network_is_there() {
        let mut n = Network {
            tried: true,
            ..Network::default()
        };
        let mut m = Message::call("x", PATH, NETWORK, "GetConnectivity");
        m.serial = 1;
        assert_eq!(n.call(&m).body[0], Value::U32(4));
    }

    #[test]
    fn power_saver_is_a_property() {
        let p = Power {
            saver: true,
            out: Vec::new(),
        };
        let mut m = Message::call("x", PATH, bus::PROPERTIES, "Get")
            .arg(Value::str(POWER))
            .arg(Value::str("power-saver-enabled"));
        m.serial = 1;
        assert_eq!(p.properties(&m).body[0].inner(), &Value::Bool(true));
    }
}
