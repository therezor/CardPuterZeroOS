//! Every Settings page, built from a snapshot of the system. The snapshot is
//! filled by the shell (config, sysfs, the helpers' output); here it only
//! becomes rows.

use super::{Act, Body, Choice, Command, ConnField, Item, Line, Page, PageId, Setting, Value};
use gh0st_core::apps::Layout;
use gh0st_core::sysconf::{self, CpuMode, GpuMem, NmPrivacy};

/// Screen-off choices, in seconds. 0 is never.
pub const DARK_TIMES: &[i64] = &[15, 30, 60, 120, 300, 0];
/// `gh0st.lock.after` in seconds of dark; -1 locks only by hand.
pub const LOCK_AFTERS: &[i64] = &[0, 60, 300, 900, -1];
pub const DEFAULT_LOCK_AFTER: i64 = 300;

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

    fn note(self) -> &'static str {
        match self {
            Logs::Off => "Nothing kept",
            Logs::Ram => "Gone at restart",
            Logs::Persistent => "Kept on the card",
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

/// A saved connection's settings.
#[derive(Clone, Debug, Default)]
pub struct Conn {
    pub name: String,
    pub wifi: bool,
    pub autoconnect: bool,
    /// Manual IPv4 rather than DHCP.
    pub manual: bool,
    pub address: String,
    pub gateway: String,
    pub dns: String,
    /// `cloned-mac-address`; empty is NetworkManager's default.
    pub mac: String,
    pub active: bool,
    /// The address in use now, whatever gave it.
    pub current: String,
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
    /// A built-in symbol in place of the app's own icon; empty for its own.
    pub icon: String,
    /// Keeps running when sent to the background instead of pausing.
    pub background: bool,
    pub pinned: bool,
}

/// Symbols an app's icon can be swapped for, after its own.
pub const APP_ICONS: &[&str] = &[
    "square-terminal",
    "code-xml",
    "folder",
    "camera",
    "globe",
    "keyboard",
    "monitor",
    "shopping-bag",
    "hard-drive",
    "shield",
    "user",
    "clock",
    "volume-2",
    "network",
    "bluetooth",
    "usb",
    "lock",
    "settings",
    "gh0st-components",
];

pub fn icon_label(icon: &str) -> String {
    if icon.is_empty() {
        return "Own".into();
    }
    let mut s = icon.replace(['-', '_'], " ");
    if let Some(f) = s.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    s
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
    /// Every saved Wi-Fi and wired profile, as (name, type).
    pub conns: Vec<(String, String)>,
    /// The profile open on a connection page.
    pub conn: Option<Conn>,
    pub ifaces: Vec<Iface>,
    pub bt_on: bool,
    pub bt_scanning: bool,
    pub devices: Vec<Device>,
    pub zone: String,
    pub zones: Vec<String>,
    pub network_time: bool,
    pub ssh: bool,
    pub logs: Logs,
    /// NetworkManager's privacy settings, read from its config files.
    pub nm: NmPrivacy,
    /// `gh0st.lock.password`: the lock screen asks for the password.
    pub lock_password: bool,
    /// `gh0st.lock.after`: seconds of dark before it asks.
    pub lock_after: i64,
    /// The local date and time, "2026-11-21 14:32", when System opened.
    pub now: String,
    /// `gpu_mem` in config.txt, and what the firmware runs with now.
    pub gpu_mem: GpuMem,
    pub gpu_running: Option<u32>,
    /// The Wi-Fi country in cmdline.txt, and the one booted with: `None`
    /// when unknown, empty when the kernel was given none.
    pub country: Option<String>,
    pub country_running: Option<String>,
    /// (code, name), sorted by name. Loaded when the picker opens.
    pub countries: Vec<(String, String)>,
    /// The Wi-Fi country's zones from `zone.tab`.
    pub country_zones: Vec<String>,
    pub governor: String,
    pub hostname: String,
    /// Swap in MB from our drop-in (`None` with none), and the zram swap
    /// running now. 0 is off.
    pub swap_mb: Option<u32>,
    pub swap_running: Option<u32>,
    /// The compression in our drop-in, and the one zram runs with.
    pub swap_comp: Option<String>,
    pub swap_comp_running: Option<String>,
    pub swappiness: Option<u32>,
    /// The pinned app being moved on the Apps page.
    pub moving: Option<String>,
    pub about: Vec<(String, String)>,
}

impl State {
    pub fn ip(&self) -> Option<&str> {
        self.ifaces
            .iter()
            .find(|i| !i.ipv4.is_empty() && i.name != "lo")
            .map(|i| i.ipv4.split('/').next().unwrap_or(&i.ipv4))
    }

    /// A boot file says something the running system doesn't yet.
    pub fn restart_needed(&self) -> bool {
        let gpu = match (self.gpu_mem, self.gpu_running) {
            (GpuMem::Set(n), Some(r)) => n != r,
            _ => false,
        };
        let country = match (&self.country, &self.country_running) {
            (Some(file), Some(now)) => file != now,
            _ => false,
        };
        gpu || country || self.swap_restart_needed()
    }

    /// A swap size or compression waits for a restart. The compression
    /// matters only while swap runs.
    pub fn swap_restart_needed(&self) -> bool {
        let size = match (self.swap_mb, self.swap_running) {
            (Some(file), Some(now)) => file != now,
            _ => false,
        };
        let comp = match (&self.swap_comp, &self.swap_comp_running, self.swap_running) {
            (Some(file), Some(now), Some(mb)) => mb > 0 && file != now,
            _ => false,
        };
        size || comp
    }

    /// The swap size Settings shows: what boots next, else what runs.
    pub fn swap_shown(&self) -> Option<u32> {
        self.swap_mb.or(self.swap_running)
    }

    fn country_name(&self, cc: &str) -> String {
        self.countries
            .iter()
            .find(|(c, _)| c == cc)
            .map_or_else(|| cc.to_string(), |(_, n)| n.clone())
    }

    pub fn active_ssid(&self) -> Option<&str> {
        self.networks
            .iter()
            .find(|n| n.active)
            .map(|n| n.ssid.as_str())
    }
}

pub fn lock_after_label(secs: i64) -> String {
    match secs {
        s if s < 0 => "By hand".into(),
        0 => "At once".into(),
        s => format!("{} min", s / 60),
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
                        icon: Some(match (a.pinned, s.moving.as_deref()) {
                            (_, Some(m)) if m == a.file => "arrow-up-down",
                            (true, _) => "pin",
                            _ => "",
                        }),
                        label: a.name.clone(),
                        value: if s.moving.as_deref() == Some(a.file.as_str()) {
                            Value::None
                        } else {
                            Value::TextChevron(
                                match (a.shown, a.background) {
                                    (false, _) => "Hidden",
                                    (true, true) => "Runs in background",
                                    (true, false) => "",
                                }
                                .into(),
                            )
                        },
                        act: if s.moving.as_deref() == Some(a.file.as_str()) {
                            Act::None
                        } else {
                            Act::Open(PageId::App(a.file.clone()))
                        },
                        danger: false,
                    })
                    .collect(),
            ),
            if s.moving.is_some() {
                vec![("Up Down", "move"), ("Enter", "done")]
            } else if s.apps.iter().filter(|a| a.pinned).count() > 1 {
                vec![("M", "move pin")]
            } else {
                vec![]
            },
        ),
        PageId::App(file) => {
            let a = s
                .apps
                .iter()
                .find(|a| a.file == *file)
                .cloned()
                .unwrap_or_default();
            let f = || a.file.clone();
            (
                "App",
                Body::List(vec![
                    Item::new(
                        "layout-grid",
                        "Show on home",
                        Value::Toggle(a.shown),
                        Act::Set(Setting::AppShown(f())),
                    ),
                    Item::new(
                        "palette",
                        "Icon",
                        Value::Choice(icon_label(&a.icon)),
                        Act::Set(Setting::AppIcon(f())),
                    ),
                    Item::new(
                        "arrow-left-right",
                        "Run in background",
                        Value::Toggle(a.background),
                        Act::Set(Setting::AppBackground(f())),
                    ),
                    Item::new(
                        "pin",
                        "Pinned",
                        Value::Toggle(a.pinned),
                        Act::Set(Setting::AppPinned(f())),
                    ),
                ]),
                vec![],
            )
        }
        PageId::Wifi => (
            "Wi-Fi",
            Body::List(wifi(s)),
            vec![("R", "scan"), ("Del", "forget")],
        ),
        PageId::Network => ("Network", Body::List(network(s)), vec![]),
        PageId::Conn(name) => ("Connection", Body::List(conn(s, name)), vec![]),
        PageId::Bluetooth => (
            "Bluetooth",
            Body::List(bluetooth(s)),
            vec![("S", "scan"), ("Del", "forget")],
        ),
        PageId::System => ("System", Body::List(system(s)), vec![]),
        PageId::TimeZone(region) => ("Time zone", Body::List(zones(s, region.as_deref())), vec![]),
        PageId::CountryZones => ("Time zone", Body::List(country_zones(s)), vec![]),
        PageId::Advanced => ("Advanced", Body::List(advanced(s)), vec![]),
        PageId::Swap => ("Swap", Body::List(swap(s)), vec![]),
        PageId::Choose(c) => (
            match c {
                Choice::GpuMem => "GPU memory",
                Choice::CpuMode => "CPU mode",
                Choice::Logs => "Logs",
                Choice::Compression => "Compression",
                Choice::Swappiness => "Swappiness",
            },
            Body::List(options(s, *c)),
            vec![],
        ),
        PageId::Country(initial) => (
            "Wi-Fi country",
            Body::List(countries(s, *initial)),
            if initial.is_none() {
                vec![("A-Z", "jump")]
            } else {
                vec![]
            },
        ),
        PageId::Privacy => ("Privacy", Body::List(privacy(s)), vec![]),
        PageId::Look => ("Device and look", Body::List(look(s)), vec![]),
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
    let title = match id {
        PageId::App(file) => s
            .apps
            .iter()
            .find(|a| a.file == *file)
            .map_or_else(|| title.to_string(), |a| a.name.clone()),
        PageId::Conn(name) => name.clone(),
        _ => title.into(),
    };
    Page { title, body, keys }
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
        link("bluetooth", "Bluetooth", on_off(s.bt_on), PageId::Bluetooth),
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
            act: if n.active && n.known {
                Act::Open(PageId::Conn(n.ssid.clone()))
            } else {
                Act::Run(Command::Join(n.ssid.clone()))
            },
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

/// One row per device. Enter opens its connection: the active one, or for a
/// wired port with no cable, the saved wired profile.
fn network(s: &State) -> Vec<Item> {
    s.ifaces
        .iter()
        .map(|i| {
            let wifi = i.name.starts_with("wl");
            let profile = if !i.connection.is_empty() {
                Some(i.connection.clone())
            } else if !wifi {
                s.conns
                    .iter()
                    .find(|(_, t)| t == "802-3-ethernet")
                    .map(|(n, _)| n.clone())
            } else {
                None
            };
            let value = if i.ipv4.is_empty() {
                i.state.clone()
            } else {
                i.ipv4.split('/').next().unwrap_or(&i.ipv4).to_string()
            };
            Item {
                icon: Some(if wifi { "wifi" } else { "ethernet-port" }),
                label: if i.connection.is_empty() {
                    i.name.clone()
                } else {
                    format!("{} \u{00b7} {}", i.name, i.connection)
                },
                value: if profile.is_some() {
                    Value::TextChevron(value)
                } else {
                    Value::Text(value)
                },
                act: profile.map_or(Act::None, |p| Act::Open(PageId::Conn(p))),
                danger: false,
            }
        })
        .collect()
}

fn conn(s: &State, name: &str) -> Vec<Item> {
    let Some(c) = s.conn.as_ref().filter(|c| c.name == name) else {
        return vec![Item::new("", "Reading\u{2026}", Value::None, Act::None)];
    };
    let shown = |v: &str, fallback: &str| {
        if v.is_empty() {
            fallback.to_string()
        } else {
            v.to_string()
        }
    };
    let auto_ip = !c.manual;
    let address = if auto_ip {
        shown(&c.current, "Automatic")
    } else {
        shown(&c.address, "None")
    };
    vec![
        Item::new(
            "check",
            "Connect on its own",
            Value::Toggle(c.autoconnect),
            Act::Set(Setting::ConnAuto),
        ),
        Item::new(
            "network",
            "IPv4",
            Value::Choice(if c.manual { "Manual" } else { "Automatic" }.into()),
            Act::Set(Setting::ConnMethod),
        ),
        Item::new(
            "",
            "Address",
            Value::TextChevron(address),
            Act::Run(Command::ConnEdit(ConnField::Address)),
        ),
        Item::new(
            "",
            "Gateway",
            Value::TextChevron(shown(
                &c.gateway,
                if auto_ip { "Automatic" } else { "None" },
            )),
            Act::Run(Command::ConnEdit(ConnField::Gateway)),
        ),
        Item::new(
            "",
            "DNS",
            Value::TextChevron(shown(&c.dns, "Automatic")),
            Act::Run(Command::ConnEdit(ConnField::Dns)),
        ),
        Item::new(
            "shield",
            "MAC address",
            Value::Choice(super::net::mac_label(&c.mac).into()),
            Act::Set(Setting::ConnMac),
        ),
        Item::new(
            "arrow-left-right",
            "Reconnect",
            Value::None,
            Act::Run(Command::ConnUp),
        ),
        Item {
            danger: true,
            ..Item::new(
                "power",
                "Forget",
                Value::None,
                Act::Run(Command::ConnForget),
            )
        },
    ]
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

/// The private defaults, each switchable. Settings > Privacy and setup's
/// privacy step show the same rows. SSH and Bluetooth stay on their own
/// pages, System and Bluetooth, not here too.
pub fn privacy(s: &State) -> Vec<Item> {
    let set = |icon, label, on, what| Item::new(icon, label, Value::Toggle(on), Act::Set(what));
    vec![
        set(
            "wifi",
            "Private Wi-Fi address",
            s.nm.mac_private,
            Setting::WifiMac,
        ),
        set(
            "search",
            "Random address in scans",
            s.nm.scan_random,
            Setting::ScanMac,
        ),
        set(
            "tag",
            "Send device name",
            s.nm.send_hostname,
            Setting::SendHostname,
        ),
        set(
            "globe",
            "IPv6 privacy address",
            s.nm.ip6_privacy,
            Setting::Ip6Privacy,
        ),
        set(
            "clock",
            "Network time",
            s.network_time,
            Setting::NetworkTime,
        ),
        Item::new(
            "hard-drive",
            "Keep logs",
            Value::TextChevron(s.logs.label().into()),
            Act::Open(PageId::Choose(Choice::Logs)),
        ),
    ]
}

/// Setup's device name and look.
fn look(s: &State) -> Vec<Item> {
    vec![
        Item::new(
            "tag",
            "Device name",
            Value::TextChevron(s.hostname.clone()),
            Act::Run(Command::DeviceName),
        ),
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
            "clock",
            "Clock",
            Value::Choice(if s.clock_24h { "24 h" } else { "12 h" }.into()),
            Act::Set(Setting::Clock),
        ),
    ]
}

/// "2026-11-21 14:32" as year, month, day, hour and minute.
pub fn parse_date_time(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let (date, time) = s.trim().split_once(' ')?;
    let mut d = date.split('-').map(|p| p.parse::<u32>().ok());
    let (y, mon, day) = (d.next()??, d.next()??, d.next()??);
    let (h, min) = time.trim().split_once(':')?;
    let (h, min) = (h.parse::<u32>().ok()?, min.parse::<u32>().ok()?);
    let ok = d.next().is_none()
        && (2024..2100).contains(&y)
        && (1..=12).contains(&mon)
        && (1..=31).contains(&day)
        && h < 24
        && min < 60;
    ok.then_some((y as i32, mon, day, h, min))
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
            "clock",
            "Date and time",
            Value::TextChevron(s.now.clone()),
            Act::Run(Command::SetTime),
        ),
        Item::new(
            "lock",
            "Password",
            Value::Chevron,
            Act::Run(Command::Password),
        ),
        Item::new(
            "lock",
            "Unlock with password",
            Value::Toggle(s.lock_password),
            Act::Set(Setting::LockPassword),
        ),
        Item::new(
            "timer",
            "Lock after",
            Value::Choice(lock_after_label(s.lock_after)),
            Act::Set(Setting::LockAfter),
        ),
        Item::new(
            "sliders-horizontal",
            "Advanced",
            Value::TextChevron(if s.restart_needed() {
                "Restart needed".into()
            } else {
                String::new()
            }),
            Act::Open(PageId::Advanced),
        ),
        Item::new("power", "Restart", Value::None, Act::Run(Command::Restart)),
        Item {
            danger: true,
            ..Item::new(
                "power",
                "Power off",
                Value::None,
                Act::Run(Command::PowerOff),
            )
        },
    ]
}

fn advanced(s: &State) -> Vec<Item> {
    let pick = |c| Act::Open(PageId::Choose(c));
    let gpu = match s.gpu_mem {
        GpuMem::Set(n) => Item::new(
            "memory-stick",
            "GPU memory",
            Value::TextChevron(format!("{n} MB")),
            pick(Choice::GpuMem),
        ),
        GpuMem::Absent => Item::new(
            "memory-stick",
            "GPU memory",
            Value::TextChevron("Default".into()),
            pick(Choice::GpuMem),
        ),
        GpuMem::Unsupported => Item::new(
            "memory-stick",
            "GPU memory",
            Value::Text("Set by hand".into()),
            Act::None,
        ),
    };
    let cpu = match CpuMode::from_governor(&s.governor) {
        Some(m) => m.label().to_string(),
        None if s.governor.is_empty() => "Unknown".into(),
        None => s.governor.clone(),
    };
    let mut out = vec![
        gpu,
        Item::new(
            "arrow-up-down",
            "Swap",
            Value::TextChevron(swap_size(s)),
            Act::Open(PageId::Swap),
        ),
        Item::new(
            "flag",
            "Wi-Fi country",
            Value::TextChevron(
                s.country
                    .as_deref()
                    .map_or_else(String::new, |c| s.country_name(c)),
            ),
            Act::Open(PageId::Country(None)),
        ),
        Item::new(
            "cpu",
            "CPU mode",
            Value::TextChevron(cpu),
            pick(Choice::CpuMode),
        ),
        Item::new(
            "tag",
            "Device name",
            Value::TextChevron(s.hostname.clone()),
            Act::Run(Command::DeviceName),
        ),
    ];
    // Last, so the rows above keep their places when it comes and goes.
    if s.restart_needed() {
        out.push(Item::new(
            "rotate-cw",
            "Restart to apply",
            Value::None,
            Act::Run(Command::Restart),
        ));
    }
    out
}

fn swap_size(s: &State) -> String {
    match s.swap_shown() {
        Some(0) => "Off".into(),
        Some(n) => format!("{n} MB"),
        None => "\u{2026}".into(),
    }
}

fn swap(s: &State) -> Vec<Item> {
    let known = |v: Option<String>| v.unwrap_or_else(|| "\u{2026}".into());
    let mut out = vec![
        Item::new(
            "memory-stick",
            "Size",
            Value::TextChevron(swap_size(s)),
            Act::Run(Command::SwapSize),
        ),
        Item::new(
            "zoom-out",
            "Compression",
            Value::TextChevron(known(
                s.swap_comp.clone().or_else(|| s.swap_comp_running.clone()),
            )),
            Act::Open(PageId::Choose(Choice::Compression)),
        ),
        Item::new(
            "arrow-left-right",
            "Swappiness",
            Value::TextChevron(known(s.swappiness.map(|n| n.to_string()))),
            Act::Open(PageId::Choose(Choice::Swappiness)),
        ),
    ];
    if s.swap_restart_needed() {
        out.push(Item::new(
            "rotate-cw",
            "Restart to apply",
            Value::None,
            Act::Run(Command::Restart),
        ));
    }
    out
}

fn comp_note(alg: &str) -> &'static str {
    match alg {
        "zstd" => "Default, packs the most",
        "lz4" => "Fastest, packs less",
        "lzo-rle" => "Fast, packs less",
        _ => "",
    }
}

fn swappiness_note(n: u32) -> &'static str {
    match n {
        10 => "Drops file cache first",
        60 => "Kernel default",
        100 => "Gh0st OS default",
        150 => "Swaps apps sooner",
        200 => "Swaps apps first",
        _ => "",
    }
}

fn gpu_note(mb: u32) -> &'static str {
    match mb {
        16 => "No camera or video",
        32 => "Camera may fail",
        64 => "Default",
        128 => "Stock",
        n if n > 128 => "Less RAM for apps",
        _ => "",
    }
}

fn cpu_note(m: CpuMode) -> &'static str {
    match m {
        CpuMode::Balanced => "Follows the load",
        CpuMode::PowerSaver => "Holds 600 MHz",
        CpuMode::Performance => "Holds 1 GHz",
    }
}

/// A choice's options, the current one checked.
fn options(s: &State, c: Choice) -> Vec<Item> {
    let row = |label: String, note: &str, word: String, here: bool| Item {
        icon: Some(if here { "check" } else { "" }),
        label,
        value: Value::Text(note.into()),
        act: Act::Run(Command::Choose(c, word)),
        danger: false,
    };
    match c {
        Choice::GpuMem => sysconf::GPU_MEMS
            .iter()
            .map(|&mb| {
                row(
                    format!("{mb} MB"),
                    gpu_note(mb),
                    mb.to_string(),
                    s.gpu_mem == GpuMem::Set(mb),
                )
            })
            .collect(),
        Choice::CpuMode => {
            let now = CpuMode::from_governor(&s.governor);
            CpuMode::ALL
                .iter()
                .map(|&m| {
                    row(
                        m.label().into(),
                        cpu_note(m),
                        m.verb().into(),
                        now == Some(m),
                    )
                })
                .collect()
        }
        Choice::Logs => Logs::ALL
            .iter()
            .map(|&l| row(l.label().into(), l.note(), l.verb().into(), s.logs == l))
            .collect(),
        Choice::Compression => {
            let now = s.swap_comp.as_deref().or(s.swap_comp_running.as_deref());
            sysconf::COMPRESSIONS
                .iter()
                .map(|&a| row(a.into(), comp_note(a), a.into(), now == Some(a)))
                .collect()
        }
        Choice::Swappiness => sysconf::SWAPPINESS
            .iter()
            .map(|&n| {
                row(
                    n.to_string(),
                    swappiness_note(n),
                    n.to_string(),
                    s.swappiness == Some(n),
                )
            })
            .collect(),
    }
}

/// The first letter of a country's name, which groups the picker.
pub fn initial(name: &str) -> char {
    name.chars().next().map_or(' ', |c| c.to_ascii_uppercase())
}

/// Initials first, then the countries under one, like the time zones.
fn countries(s: &State, at: Option<char>) -> Vec<Item> {
    let here = s.country.as_deref();
    let mark = |yes: bool| Some(if yes { "check" } else { "" });
    match at {
        None => {
            let mut letters: Vec<char> = s.countries.iter().map(|(_, n)| initial(n)).collect();
            letters.dedup();
            let current = here.map(|c| initial(&s.country_name(c)));
            letters
                .into_iter()
                .map(|l| Item {
                    icon: mark(current == Some(l)),
                    label: l.to_string(),
                    value: Value::Chevron,
                    act: Act::Open(PageId::Country(Some(l))),
                    danger: false,
                })
                .collect()
        }
        Some(l) => s
            .countries
            .iter()
            .filter(|(_, n)| initial(n) == l)
            .map(|(cc, n)| Item {
                icon: mark(here == Some(cc.as_str())),
                label: n.clone(),
                value: Value::Text(cc.clone()),
                act: Act::Run(Command::SetCountry(cc.clone())),
                danger: false,
            })
            .collect(),
    }
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

/// The country's zones that the system knows, by city, then Other for the
/// full list.
fn country_zones(s: &State) -> Vec<Item> {
    let known = |z: &&String| s.zones.is_empty() || s.zones.contains(z);
    let mut items: Vec<Item> = s
        .country_zones
        .iter()
        .filter(known)
        .map(|z| Item {
            icon: Some(if *z == s.zone { "check" } else { "" }),
            label: z.rsplit('/').next().unwrap_or(z).replace('_', " "),
            value: Value::None,
            act: Act::Run(Command::SetZone(z.clone())),
            danger: false,
        })
        .collect();
    items.push(Item {
        icon: Some(""),
        label: "Other\u{2026}".into(),
        value: Value::Chevron,
        act: Act::Open(PageId::TimeZone(None)),
        danger: false,
    });
    items
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
        assert_eq!(
            p.item(0).unwrap().value,
            Value::TextChevron("HomeNet".into())
        );
    }

    #[test]
    fn wifi_off_hides_the_list() {
        let mut s = state();
        assert_eq!(build(&PageId::Wifi, &s).len(), 3);
        s.wifi_on = false;
        assert_eq!(build(&PageId::Wifi, &s).len(), 1);
    }

    #[test]
    fn a_pending_boot_change_asks_for_a_restart() {
        let mut s = state();
        s.gpu_mem = GpuMem::Set(128);
        s.gpu_running = Some(128);
        s.country = Some("AE".into());
        s.country_running = Some("AE".into());
        assert_eq!(build(&PageId::Advanced, &s).len(), 5);
        let link = |s: &State| {
            let p = build(&PageId::System, s);
            let i = (0..p.len()).find(|&i| p.item(i).unwrap().label == "Advanced");
            p.item(i.unwrap()).unwrap().value.clone()
        };
        assert_eq!(link(&s), Value::TextChevron(String::new()));
        s.gpu_mem = GpuMem::Set(64);
        assert_eq!(link(&s), Value::TextChevron("Restart needed".into()));
        let p = build(&PageId::Advanced, &s);
        assert_eq!(p.len(), 6);
        assert_eq!(p.item(5).unwrap().act, Act::Run(Command::Restart));
        s.gpu_mem = GpuMem::Set(128);
        s.country = Some("GB".into());
        assert!(s.restart_needed());
    }

    #[test]
    fn a_moving_pin_is_marked_and_holds_still() {
        let app = |file: &str, pinned| AppRow {
            file: file.into(),
            name: file.into(),
            shown: true,
            pinned,
            ..AppRow::default()
        };
        let mut s = State {
            apps: vec![app("a", true), app("b", true), app("c", false)],
            ..state()
        };
        let p = build(&PageId::Apps, &s);
        assert_eq!(p.item(0).unwrap().icon, Some("pin"));
        assert_eq!(p.item(2).unwrap().icon, Some(""));
        assert_eq!(p.keys, vec![("M", "move pin")]);
        s.moving = Some("b".into());
        let p = build(&PageId::Apps, &s);
        assert_eq!(p.item(1).unwrap().icon, Some("arrow-up-down"));
        assert_eq!(p.item(1).unwrap().act, Act::None);
        assert_eq!(p.keys[1], ("Enter", "done"));
    }

    #[test]
    fn a_pick_names_the_current_option() {
        let s = State {
            gpu_mem: GpuMem::Set(128),
            governor: "ondemand".into(),
            ..state()
        };
        let gpu = build(&PageId::Choose(Choice::GpuMem), &s);
        assert_eq!(gpu.len(), sysconf::GPU_MEMS.len());
        let checked: Vec<_> = (0..gpu.len())
            .filter(|&i| gpu.item(i).unwrap().icon == Some("check"))
            .collect();
        assert_eq!(checked, [4]);
        assert_eq!(
            gpu.item(0).unwrap().act,
            Act::Run(Command::Choose(Choice::GpuMem, "16".into()))
        );
        let cpu = build(&PageId::Choose(Choice::CpuMode), &s);
        assert!(
            (0..3).all(|i| cpu.item(i).unwrap().icon == Some("")),
            "ondemand is none of them"
        );
        let adv = build(&PageId::Advanced, &s);
        assert_eq!(
            adv.item(3).unwrap().value,
            Value::TextChevron("ondemand".into())
        );
    }

    #[test]
    fn swap_shows_what_boots_next() {
        let mut s = State {
            swap_running: Some(414),
            ..state()
        };
        let row = |s: &State| build(&PageId::Advanced, s).item(1).unwrap().value.clone();
        assert_eq!(row(&s), Value::TextChevron("414 MB".into()));
        assert_eq!(
            build(&PageId::Advanced, &s).item(1).unwrap().act,
            Act::Open(PageId::Swap)
        );
        assert!(!s.restart_needed(), "no drop-in, nothing pending");
        s.swap_mb = Some(0);
        assert_eq!(row(&s), Value::TextChevron("Off".into()));
        assert!(s.restart_needed());
        let p = build(&PageId::Swap, &s);
        assert_eq!(p.item(0).unwrap().act, Act::Run(Command::SwapSize));
        assert_eq!(p.item(3).unwrap().act, Act::Run(Command::Restart));
        s.swap_mb = Some(414);
        assert!(!s.restart_needed());
        assert_eq!(build(&PageId::Swap, &s).len(), 3);
    }

    #[test]
    fn compression_waits_for_a_restart_only_while_swap_runs() {
        let mut s = State {
            swap_running: Some(414),
            swap_comp_running: Some("zstd".into()),
            ..state()
        };
        let comp = |s: &State| build(&PageId::Swap, s).item(1).unwrap().value.clone();
        assert_eq!(comp(&s), Value::TextChevron("zstd".into()));
        s.swap_comp = Some("lz4".into());
        assert_eq!(comp(&s), Value::TextChevron("lz4".into()));
        assert!(s.restart_needed());
        s.swap_running = Some(0);
        assert!(!s.restart_needed());
        let pick = build(&PageId::Choose(Choice::Compression), &s);
        assert_eq!(pick.len(), sysconf::COMPRESSIONS.len());
        assert_eq!(pick.item(1).unwrap().icon, Some("check"));
        assert_eq!(
            pick.item(1).unwrap().act,
            Act::Run(Command::Choose(Choice::Compression, "lz4".into()))
        );
    }

    #[test]
    fn swappiness_checks_a_listed_value_only() {
        let mut s = State {
            swappiness: Some(60),
            ..state()
        };
        let checked = |s: &State| {
            let p = build(&PageId::Choose(Choice::Swappiness), s);
            (0..p.len())
                .filter(|&i| p.item(i).unwrap().icon == Some("check"))
                .collect::<Vec<_>>()
        };
        assert_eq!(checked(&s), [1]);
        s.swappiness = Some(80);
        assert!(checked(&s).is_empty());
        assert_eq!(
            build(&PageId::Swap, &s).item(2).unwrap().value,
            Value::TextChevron("80".into())
        );
    }

    #[test]
    fn hand_set_gpu_memory_is_read_only() {
        let s = State {
            gpu_mem: GpuMem::Unsupported,
            ..state()
        };
        assert_eq!(build(&PageId::Advanced, &s).item(0).unwrap().act, Act::None);
    }

    #[test]
    fn countries_go_initial_then_name() {
        let s = State {
            countries: [
                ("AD", "Andorra"),
                ("AE", "United Arab Emirates"),
                ("GB", "Britain (UK)"),
            ]
            .map(|(c, n)| (c.to_string(), n.to_string()))
            .to_vec(),
            country: Some("AE".into()),
            ..state()
        };
        let mut sorted = s.clone();
        sorted.countries.sort_by(|a, b| a.1.cmp(&b.1));
        let letters = build(&PageId::Country(None), &sorted);
        assert_eq!(letters.len(), 3);
        assert_eq!(letters.item(2).unwrap().icon, Some("check"));
        let u = build(&PageId::Country(Some('U')), &sorted);
        assert_eq!(u.len(), 1);
        assert_eq!(
            u.item(0).unwrap().act,
            Act::Run(Command::SetCountry("AE".into()))
        );
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
    #[test]
    fn privacy_shows_every_private_default_as_a_switch() {
        let mut s = state();
        s.nm = NmPrivacy {
            mac_private: true,
            scan_random: true,
            send_hostname: false,
            ip6_privacy: true,
        };
        s.network_time = true;
        let p = build(&PageId::Privacy, &s);
        let labels: Vec<&str> = (0..p.len())
            .map(|i| p.item(i).unwrap().label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "Private Wi-Fi address",
                "Random address in scans",
                "Send device name",
                "IPv6 privacy address",
                "Network time",
                "Keep logs",
            ]
        );
        let on: Vec<Value> = (0..5).map(|i| p.item(i).unwrap().value.clone()).collect();
        assert_eq!(
            on,
            [true, true, false, true, true].map(Value::Toggle).to_vec()
        );
        assert_eq!(p.item(1).unwrap().act, Act::Set(Setting::ScanMac));
    }

    #[test]
    fn system_has_the_clock_and_the_lock() {
        let mut s = state();
        s.now = "2026-11-21 14:32".into();
        s.lock_password = true;
        let p = build(&PageId::System, &s);
        let find = |l: &str| (0..p.len()).find_map(|i| p.item(i).filter(|it| it.label == l));
        assert_eq!(
            find("Date and time").unwrap().value,
            Value::TextChevron("2026-11-21 14:32".into())
        );
        assert_eq!(
            find("Unlock with password").unwrap().value,
            Value::Toggle(true)
        );
        s.lock_after = 300;
        let p = build(&PageId::System, &s);
        let find = |l: &str| (0..p.len()).find_map(|i| p.item(i).filter(|it| it.label == l));
        assert_eq!(
            find("Lock after").unwrap().value,
            Value::Choice("5 min".into())
        );
        assert_eq!(lock_after_label(0), "At once");
        assert_eq!(lock_after_label(-1), "By hand");
    }

    #[test]
    fn dates_parse_only_whole_and_in_range() {
        assert_eq!(
            parse_date_time("2026-11-21 14:32"),
            Some((2026, 11, 21, 14, 32))
        );
        assert_eq!(
            parse_date_time(" 2030-01-05  09:07 "),
            Some((2030, 1, 5, 9, 7))
        );
        for bad in [
            "",
            "2026-11-21",
            "14:32",
            "2026-13-01 10:00",
            "2026-11-32 10:00",
            "2026-11-21 24:00",
            "2026-11-21 10:60",
            "2023-12-31 23:59",
            "2026-11-21-1 10:00",
            "2026/11/21 10:00",
        ] {
            assert_eq!(parse_date_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn look_is_the_name_then_the_look() {
        let mut s = state();
        s.hostname = "zero".into();
        let p = build(&PageId::Look, &s);
        assert_eq!(p.item(0).unwrap().value, Value::TextChevron("zero".into()));
        assert_eq!(p.item(2).unwrap().act, Act::Set(Setting::Palette));
    }
    #[test]
    fn the_country_s_zones_come_first_then_other() {
        let mut s = state();
        s.country_zones = ["Europe/London", "Atlantic/Nowhere"]
            .map(String::from)
            .to_vec();
        let p = build(&PageId::CountryZones, &s);
        assert_eq!(p.len(), 2, "London, then Other; the unknown zone left out");
        assert_eq!(p.item(0).unwrap().label, "London");
        assert_eq!(p.item(0).unwrap().icon, Some("check"));
        assert_eq!(p.item(1).unwrap().act, Act::Open(PageId::TimeZone(None)));
    }
}
