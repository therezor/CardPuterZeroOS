//! Every Settings page, built from a snapshot of the system. The snapshot is
//! filled by the shell (config, sysfs, the helpers' output); here it only
//! becomes rows.

use super::{Act, Body, Command, Item, Line, Page, PageId, Setting, Value};
use crate::home::Layout;

/// Screen-off choices, in seconds. 0 is never.
pub const DARK_TIMES: &[i64] = &[15, 30, 60, 120, 300, 0];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Logs {
    #[default]
    Off,
    Ram,
    Persistent,
}

impl Logs {
    pub const ALL: [Logs; 3] = [Logs::Off, Logs::Ram, Logs::Persistent];

    pub fn label(self) -> &'static str {
        match self {
            Logs::Off => "Off",
            Logs::Ram => "RAM only",
            Logs::Persistent => "Persistent",
        }
    }

    /// The helper's word for it.
    pub fn verb(self) -> &'static str {
        match self {
            Logs::Off => "off",
            Logs::Ram => "ram",
            Logs::Persistent => "persistent",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Network {
    pub ssid: String,
    /// 0-4 bars.
    pub bars: u8,
    pub secure: bool,
    /// A saved profile exists, so Enter joins without asking.
    pub known: bool,
    pub active: bool,
    /// 802.1X: wants a user name too, so it is joined from a terminal.
    pub enterprise: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Iface {
    pub name: String,
    pub state: String,
    pub connection: String,
    pub ipv4: String,
    pub mac: String,
    pub gateway: String,
    pub dns: String,
}

#[derive(Clone, Debug, Default)]
pub struct Device {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AppRow {
    pub file: String,
    pub name: String,
    pub shown: bool,
}

/// What the pages show. `None` means not known yet: a helper is still out.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub brightness: Option<u8>,
    pub dark_secs: i64,
    pub volume: Option<u8>,
    pub muted: bool,
    pub layout: Layout,
    pub palette: &'static str,
    pub animations: bool,
    pub clock_24h: bool,
    pub apps: Vec<AppRow>,
    pub wifi_on: bool,
    pub wifi_busy: Option<String>,
    pub networks: Vec<Network>,
    /// Saved Wi-Fi profiles, by name.
    pub profiles: Vec<String>,
    pub ifaces: Vec<Iface>,
    pub bt_on: bool,
    pub bt_scanning: bool,
    pub devices: Vec<Device>,
    pub zone: String,
    pub zones: Vec<String>,
    pub network_time: bool,
    pub ssh: bool,
    pub logs: Logs,
    pub about: Vec<(String, String)>,
}

impl State {
    pub fn ip(&self) -> Option<&str> {
        self.ifaces
            .iter()
            .find(|i| !i.ipv4.is_empty() && i.name != "lo")
            .map(|i| i.ipv4.split('/').next().unwrap_or(&i.ipv4))
    }

    pub fn active_ssid(&self) -> Option<&str> {
        self.networks
            .iter()
            .find(|n| n.active)
            .map(|n| n.ssid.as_str())
    }
}

pub fn dark_label(secs: i64) -> String {
    match secs {
        0 => "Never".into(),
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

fn pct(p: Option<u8>) -> String {
    p.map_or_else(|| "\u{2026}".into(), |p| format!("{p}%"))
}

fn on_off(on: bool) -> String {
    if on { "On" } else { "Off" }.into()
}

pub fn build(id: &PageId, s: &State) -> Page {
    let (title, body, keys): (&str, Body, Vec<(&'static str, &'static str)>) = match id {
        PageId::Root => ("Settings", Body::List(root(s)), vec![]),
        PageId::Display => (
            "Display",
            Body::List(vec![
                Item::new(
                    "sun",
                    "Brightness",
                    Value::Level(s.brightness.unwrap_or(0)),
                    Act::Set(Setting::Brightness),
                ),
                Item::new(
                    "clock",
                    "Screen off after",
                    Value::Choice(dark_label(s.dark_secs)),
                    Act::Set(Setting::DarkTime),
                ),
            ]),
            vec![],
        ),
        PageId::Sound => (
            "Sound",
            Body::List(vec![
                Item::new(
                    "volume-2",
                    "Volume",
                    Value::Level(s.volume.unwrap_or(0)),
                    Act::Set(Setting::Volume),
                ),
                Item::new("", "Mute", Value::Toggle(s.muted), Act::Set(Setting::Mute)),
            ]),
            vec![],
        ),
        PageId::Appearance => (
            "Appearance",
            Body::List(vec![
                Item::new(
                    "layout-grid",
                    "Home layout",
                    Value::Choice(s.layout.label().into()),
                    Act::Set(Setting::Layout),
                ),
                Item::new(
                    "palette",
                    "Palette",
                    Value::Choice(s.palette.into()),
                    Act::Set(Setting::Palette),
                ),
                Item::new(
                    "arrow-left-right",
                    "Animations",
                    Value::Toggle(s.animations),
                    Act::Set(Setting::Animations),
                ),
                Item::new(
                    "clock",
                    "Clock",
                    Value::Choice(if s.clock_24h { "24 h" } else { "12 h" }.into()),
                    Act::Set(Setting::Clock),
                ),
            ]),
            vec![],
        ),
        PageId::Apps => (
            "Apps",
            Body::List(
                s.apps
                    .iter()
                    .map(|a| Item {
                        icon: None,
                        label: a.name.clone(),
                        value: Value::Toggle(a.shown),
                        act: Act::Set(Setting::AppShown(a.file.clone())),
                        danger: false,
                    })
                    .collect(),
            ),
            vec![],
        ),
        PageId::Wifi => (
            "Wi-Fi",
            Body::List(wifi(s)),
            vec![("R", "scan"), ("Del", "forget")],
        ),
        PageId::Network => ("Network", Body::Info(network(s)), vec![]),
        PageId::Bluetooth => (
            "Bluetooth",
            Body::List(bluetooth(s)),
            vec![("S", "scan"), ("Del", "forget")],
        ),
        PageId::System => ("System", Body::List(system(s)), vec![]),
        PageId::TimeZone(region) => (
            "Time zone",
            Body::List(zones(s, region.as_deref())),
            vec![],
        ),
        PageId::Privacy => (
            "Privacy",
            Body::List(vec![Item::new(
                "hard-drive",
                "Logs",
                Value::Choice(s.logs.label().into()),
                Act::Set(Setting::Logs),
            )]),
            vec![],
        ),
        PageId::About => (
            "About",
            Body::Info(
                s.about
                    .iter()
                    .map(|(k, v)| Line::Readout(k.clone(), v.clone()))
                    .collect(),
            ),
            vec![],
        ),
    };
    Page {
        id: id.clone(),
        title: title.into(),
        body,
        keys,
    }
}

fn root(s: &State) -> Vec<Item> {
    let link = |icon, label: &str, value: String, page| {
        Item::new(icon, label, Value::TextChevron(value), Act::Open(page))
    };
    let wifi = if !s.wifi_on {
        "Off".into()
    } else {
        s.active_ssid().unwrap_or("Not connected").into()
    };
    let sound = if s.muted {
        "Muted".into()
    } else {
        pct(s.volume)
    };
    vec![
        link("wifi", "Wi-Fi", wifi, PageId::Wifi),
        link(
            "network",
            "Network",
            s.ip().unwrap_or("").into(),
            PageId::Network,
        ),
        link(
            "bluetooth",
            "Bluetooth",
            on_off(s.bt_on),
            PageId::Bluetooth,
        ),
        link("sun", "Display", pct(s.brightness), PageId::Display),
        link("volume-2", "Sound", sound, PageId::Sound),
        link(
            "palette",
            "Appearance",
            s.palette.into(),
            PageId::Appearance,
        ),
        link(
            "layout-grid",
            "Apps",
            s.apps.iter().filter(|a| a.shown).count().to_string(),
            PageId::Apps,
        ),
        link("settings", "System", String::new(), PageId::System),
        link("shield", "Privacy", s.logs.label().into(), PageId::Privacy),
        link("info", "About", String::new(), PageId::About),
    ]
}

fn wifi(s: &State) -> Vec<Item> {
    let mut out = vec![Item::new(
        "wifi",
        "Wi-Fi",
        Value::Toggle(s.wifi_on),
        Act::Set(Setting::WifiRadio),
    )];
    if !s.wifi_on {
        return out;
    }
    for n in &s.networks {
        let busy = s.wifi_busy.as_deref() == Some(n.ssid.as_str());
        out.push(Item {
            icon: Some(if n.active { "check" } else { "" }),
            label: n.ssid.clone(),
            value: if busy {
                Value::Text("Joining\u{2026}".into())
            } else {
                Value::Signal {
                    level: n.bars,
                    secure: n.secure,
                }
            },
            act: Act::Run(Command::Join(n.ssid.clone())),
            danger: false,
        });
    }
    out.push(Item {
        icon: Some("search"),
        label: "Join hidden network".into(),
        value: Value::Chevron,
        act: Act::Run(Command::JoinHidden),
        danger: false,
    });
    out
}

fn network(s: &State) -> Vec<Line> {
    let mut out = Vec::new();
    for i in &s.ifaces {
        let head = if i.connection.is_empty() {
            format!("{} \u{00b7} {}", i.name, i.state)
        } else {
            format!("{} \u{00b7} {}", i.name, i.connection)
        };
        out.push(Line::Section(head));
        for (k, v) in [
            ("IPv4", &i.ipv4),
            ("Gateway", &i.gateway),
            ("DNS", &i.dns),
            ("MAC", &i.mac),
        ] {
            if !v.is_empty() {
                out.push(Line::Readout(k.into(), v.clone()));
            }
        }
    }
    if out.is_empty() {
        out.push(Line::Section("No network devices".into()));
    }
    out
}

fn bluetooth(s: &State) -> Vec<Item> {
    let mut out = vec![Item::new(
        "bluetooth",
        "Bluetooth",
        Value::Toggle(s.bt_on),
        Act::Set(Setting::Bluetooth),
    )];
    if !s.bt_on {
        return out;
    }
    for d in &s.devices {
        let state = if d.connected {
            "Connected"
        } else if d.paired {
            "Paired"
        } else {
            ""
        };
        out.push(Item {
            icon: Some(if d.connected { "check" } else { "" }),
            label: if d.name.is_empty() {
                d.mac.clone()
            } else {
                d.name.clone()
            },
            value: Value::Text(state.into()),
            act: Act::Run(Command::Device(d.mac.clone())),
            danger: false,
        });
    }
    out.push(Item {
        icon: Some("search"),
        label: "Scan for devices".into(),
        value: if s.bt_scanning {
            Value::Text("Scanning\u{2026}".into())
        } else {
            Value::None
        },
        act: Act::Run(Command::BtScan),
        danger: false,
    });
    out
}

fn system(s: &State) -> Vec<Item> {
    vec![
        Item::new(
            "globe",
            "Time zone",
            Value::TextChevron(s.zone.clone()),
            Act::Open(PageId::TimeZone(None)),
        ),
        Item::new(
            "clock",
            "Network time",
            Value::Toggle(s.network_time),
            Act::Set(Setting::NetworkTime),
        ),
        Item::new(
            "square-terminal",
            "SSH",
            Value::Toggle(s.ssh),
            Act::Set(Setting::Ssh),
        ),
        Item::new(
            "lock",
            "Password",
            Value::Chevron,
            Act::Run(Command::Password),
        ),
        Item::new("power", "Restart", Value::None, Act::Run(Command::Restart)),
        Item {
            danger: true,
            ..Item::new("power", "Power off", Value::None, Act::Run(Command::PowerOff))
        },
    ]
}

/// Regions first (the part before the slash), then a region's cities.
fn zones(s: &State, region: Option<&str>) -> Vec<Item> {
    match region {
        None => {
            let mut regions: Vec<&str> = s
                .zones
                .iter()
                .map(|z| z.split('/').next().unwrap_or(z))
                .collect();
            regions.dedup();
            regions
                .into_iter()
                .map(|r| {
                    let here = s.zone.split('/').next() == Some(r);
                    Item {
                        icon: Some(if here { "check" } else { "" }),
                        label: r.into(),
                        value: Value::Chevron,
                        act: Act::Open(PageId::TimeZone(Some(r.into()))),
                        danger: false,
                    }
                })
                .collect()
        }
        Some(r) => s
            .zones
            .iter()
            .filter_map(|z| {
                let city = z.strip_prefix(r)?.strip_prefix('/')?;
                Some(Item {
                    icon: Some(if *z == s.zone { "check" } else { "" }),
                    label: city.replace('_', " "),
                    value: Value::None,
                    act: Act::Run(Command::SetZone(z.clone())),
                    danger: false,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State {
            wifi_on: true,
            networks: vec![Network {
                ssid: "HomeNet".into(),
                bars: 3,
                secure: true,
                known: true,
                active: true,
                enterprise: false,
            }],
            zone: "Europe/London".into(),
            zones: ["Africa/Cairo", "Europe/Kyiv", "Europe/London"]
                .map(String::from)
                .to_vec(),
            ..State::default()
        }
    }

    #[test]
    fn root_links_every_page_and_shows_the_network() {
        let p = build(&PageId::Root, &state());
        assert_eq!(p.len(), 10);
        assert_eq!(p.item(0).unwrap().value, Value::TextChevron("HomeNet".into()));
    }

    #[test]
    fn wifi_off_hides_the_list() {
        let mut s = state();
        assert_eq!(build(&PageId::Wifi, &s).len(), 3);
        s.wifi_on = false;
        assert_eq!(build(&PageId::Wifi, &s).len(), 1);
    }

    #[test]
    fn time_zones_go_region_then_city() {
        let s = state();
        let regions = build(&PageId::TimeZone(None), &s);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions.item(1).unwrap().icon, Some("check"));
        let cities = build(&PageId::TimeZone(Some("Europe".into())), &s);
        assert_eq!(cities.len(), 2);
        assert_eq!(
            cities.item(1).unwrap().act,
            Act::Run(Command::SetZone("Europe/London".into()))
        );
    }
}
