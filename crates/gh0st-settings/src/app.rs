//! Settings as an app: the page stack, its keys, and what each setting
//! writes. Every config write loads config.json fresh, changes one key and
//! saves, so stock apps' keys survive. The shell reads the file again each
//! time it takes the screen back, so home follows what changed here.

use crate::model::pages::{self, APP_ICONS, AppRow, DARK_TIMES, Logs, State as PageState};
use crate::model::{
    self, Act, Body, Command, ConnField, Cursor, Dialog, Page as View, PageId, Setting, Value,
};
use crate::model::{bt, net};
use gh0st_app::gfx::theme::{self, Theme};
use gh0st_app::prelude::*;
use gh0st_app::{codes, system};
use gh0st_core::apps::{self, Dirs, Layout};
use gh0st_core::backlight::Backlight;
use gh0st_core::config::{self, Config};
use gh0st_core::{desktop, rfkill};
use std::collections::HashMap;
use std::path::Path;

/// pkexec runs the root helper by this path (see packaging/polkit).
const HELPER: &str = "/usr/libexec/gh0st-helper";
const LOGGING_DIR: &str = "/etc/systemd/journald.conf.d";
const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const DEFAULT_DARK_SECS: i64 = 30;
const VOLUME_STEP: i32 = 10;

/// What a dialog's answer is for.
enum Ask {
    Confirm(Command),
    WifiPassword(String),
    HiddenName,
    HiddenPassword(String),
    PasswordCurrent,
    PasswordNew(String),
    PasswordRepeat(String, String),
    Forget(String),
    /// A new value for a field of connection `.0`.
    ConnField(String, ConnField),
    ConnForget(String),
}

/// A helper run whose answer Settings waits on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NetJob {
    Radio,
    List,
    Profiles,
    Devices,
    SetRadio,
    Join {
        ssid: String,
        known: bool,
        gave_password: bool,
    },
    Forget(String),
    BtQuery,
    BtPower,
    BtScan,
    /// What to say when it works.
    BtDevice(String),
    BtRemove(String),
    /// `timedatectl show` and whether SSH runs.
    SysQuery,
    Zones,
    /// A time zone change, or network time when `None`.
    TimeSet(Option<String>),
    /// The root helper; what to say when it works.
    Helper(String),
    /// After the helper unblocks the radio, power it on.
    BtUnblock,
    Password,
    ConnQuery,
    /// A profile change; reconnect after it when the profile is in use.
    ConnModify {
        name: String,
        reconnect: bool,
    },
    ConnUp(String),
    ConnDelete(String),
    Volume,
    /// Its answer doesn't matter: `wpctl set-volume`, a leftover delete.
    Ignore,
}

pub struct Settings {
    stack: Vec<(PageId, Cursor)>,
    view: View,
    state: PageState,
    dialog: Option<(Dialog, Ask)>,
    /// Running helpers, with what each is for and its output so far.
    jobs: HashMap<Job, (NetJob, String)>,
    backlight: Option<Backlight>,
}

impl Settings {
    pub fn new() -> Settings {
        let state = PageState::default();
        Settings {
            stack: vec![(PageId::Root, Cursor::default())],
            view: pages::build(&PageId::Root, &state),
            state,
            dialog: None,
            jobs: HashMap::new(),
            backlight: Backlight::find(Path::new("/sys/class/backlight")),
        }
    }

    fn top(&self) -> &(PageId, Cursor) {
        self.stack.last().expect("the root page is never popped")
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.stack.last_mut().expect("root").1
    }

    fn run(&mut self, cx: &mut Cx, kind: NetJob, argv: &[&str]) {
        let job = cx.spawn(argv);
        self.jobs.insert(job, (kind, String::new()));
    }

    fn run_with(&mut self, cx: &mut Cx, kind: NetJob, argv: &[&str], input: &[u8]) {
        let job = cx.spawn_with_input(argv, input);
        self.jobs.insert(job, (kind, String::new()));
    }

    /// What Settings knows without asking a helper.
    fn fill_local(&mut self, cx: &mut Cx) {
        let config = Config::load(&config::default_path());
        let s = &mut self.state;
        s.brightness = self
            .backlight
            .as_ref()
            .and_then(|bl| bl.get().map(|r| bl.percent(r)));
        s.dark_secs = config.get_int("dark_time").unwrap_or(DEFAULT_DARK_SECS);
        s.layout = config
            .get_str("gh0st.home_layout")
            .and_then(Layout::from_config)
            .unwrap_or_default();
        s.palette = cx.theme().c.name;
        s.animations = config.get_bool("gh0st.animations").unwrap_or(true);
        s.clock_24h = config.get_str("gh0st.clock") != Some("12h");
        let pins = apps::load_pins();
        let mut rows: Vec<AppRow> =
            apps::scan_all(&Dirs::from_env(), &config, &pins, desktop::is_executable)
                .into_iter()
                .filter(|a| a.file != "settings.desktop")
                .map(|a| AppRow {
                    icon: config
                        .get_str(&apps::app_key(&a.file, "icon"))
                        .unwrap_or("")
                        .to_string(),
                    background: config
                        .get_bool(&apps::app_key(&a.file, "background"))
                        .unwrap_or(a.entry.background),
                    pinned: pins.contains(&a.file),
                    file: a.file,
                    name: a.entry.name,
                    shown: a.shown,
                })
                .collect();
        rows.sort_by_key(|a| a.name.to_lowercase());
        s.apps = rows;
        s.about = about();
        s.logs = logs_level();
    }

    /// Rebuilds the page on top from fresh local state.
    fn refresh(&mut self, cx: &mut Cx) {
        self.fill_local(cx);
        let id = self.top().0.clone();
        self.view = pages::build(&id, &self.state);
        let per = page_rows(cx.theme(), &self.view);
        let len = self.view.len();
        let info = matches!(self.view.body, Body::Info(_));
        let cur = self.cursor_mut();
        if !info {
            cur.fit(len, per);
        }
    }

    fn move_cursor(&mut self, d: i32, cx: &Cx) {
        let per = page_rows(cx.theme(), &self.view);
        let len = self.view.len();
        let info = matches!(self.view.body, Body::Info(_));
        self.cursor_mut().step(d, len, per, info);
    }

    /// Digit `n` picks the nth row on screen and acts on it.
    fn pick(&mut self, n: usize, cx: &mut Cx) {
        let target = self.top().1.top + n;
        if self.view.item(target).is_none() {
            return;
        }
        self.cursor_mut().selected = target;
        self.activate(cx);
    }

    fn selected_act(&self) -> Option<(Act, Value)> {
        let item = self.view.item(self.top().1.selected)?;
        Some((item.act.clone(), item.value.clone()))
    }

    /// Enter on a row.
    fn activate(&mut self, cx: &mut Cx) {
        let Some((act, value)) = self.selected_act() else {
            return;
        };
        match act {
            Act::Open(id) => {
                if matches!(id, PageId::TimeZone(_)) && self.state.zones.is_empty() {
                    self.run(cx, NetJob::Zones, &["timedatectl", "list-timezones"]);
                }
                if let PageId::Conn(name) = &id {
                    self.query_conn(cx, name);
                }
                self.stack.push((id, Cursor::default()));
                self.refresh(cx);
            }
            Act::Set(s) => match value {
                Value::Toggle(_) => self.apply(&s, 0, cx),
                _ => self.apply(&s, 1, cx),
            },
            Act::Run(c) => self.run_command(c, cx),
            Act::None => {}
        }
    }

    /// Z or C on a row: step a choice or a level, or flip a toggle.
    fn change(&mut self, d: i32, cx: &mut Cx) {
        if let Some((Act::Set(s), value)) = self.selected_act() {
            let d = if matches!(value, Value::Toggle(_)) {
                0
            } else {
                d
            };
            self.apply(&s, d, cx);
        }
    }

    /// Changes one setting. `d` is the step for a choice or level, 0 for a
    /// toggle.
    fn apply(&mut self, s: &Setting, d: i32, cx: &mut Cx) {
        match s {
            Setting::Brightness => {
                if let Some(bl) = &self.backlight {
                    let raw = bl.step(bl.get().unwrap_or(100), d >= 0);
                    let _ = bl.set(raw);
                    cx.settings().set_int("brightness", raw as i64);
                }
            }
            Setting::DarkTime => {
                let i = DARK_TIMES
                    .iter()
                    .position(|&t| t == self.state.dark_secs)
                    .unwrap_or(1);
                let next = DARK_TIMES[cycle(i, d, DARK_TIMES.len())];
                cx.settings().set_int("dark_time", next);
            }
            Setting::Volume => {
                let v = (self.state.volume.unwrap_or(50) as i32 + d.signum() * VOLUME_STEP)
                    .clamp(0, 100) as u8;
                let arg = format!("{v}%");
                self.run(cx, NetJob::Ignore, &["wpctl", "set-volume", SINK, &arg]);
                cx.settings().set_int("volume", v as i64);
                self.state.volume = Some(v);
            }
            Setting::Mute => {
                let muted = !self.state.muted;
                let flag = if muted { "1" } else { "0" };
                self.run(cx, NetJob::Ignore, &["wpctl", "set-mute", SINK, flag]);
                self.state.muted = muted;
            }
            Setting::Layout => {
                let l = self.state.layout.toggled();
                cx.settings().set_str("gh0st.home_layout", l.config_name());
            }
            Setting::Palette => {
                let i = theme::PALETTES
                    .iter()
                    .position(|p| p.name == self.state.palette)
                    .unwrap_or(0);
                let p = theme::PALETTES[cycle(i, d, theme::PALETTES.len())];
                // The SDK follows this key, so the new palette shows at once.
                cx.settings().set_str("gh0st.palette", p.name);
            }
            Setting::Animations => {
                cx.settings()
                    .set_bool("gh0st.animations", !self.state.animations);
            }
            Setting::Clock => {
                let h24 = !self.state.clock_24h;
                cx.settings()
                    .set_str("gh0st.clock", if h24 { "24h" } else { "12h" });
            }
            Setting::AppShown(file) => {
                let shown = self.app(file).is_some_and(|a| a.shown);
                cx.settings().set_int(&apps::stock_key(file), !shown as i64);
            }
            Setting::AppIcon(file) => {
                let key = apps::app_key(file, "icon");
                let now = self.app(file).map(|a| a.icon.clone()).unwrap_or_default();
                // Its own icon first, then the symbols.
                let pos = APP_ICONS
                    .iter()
                    .position(|i| *i == now)
                    .map_or(0, |i| i + 1);
                let next = cycle(pos, d, APP_ICONS.len() + 1);
                match next.checked_sub(1) {
                    Some(i) => cx.settings().set_str(&key, APP_ICONS[i]),
                    None => cx.settings().remove(&key),
                }
            }
            Setting::AppBackground(file) => {
                let on = !self.app(file).is_some_and(|a| a.background);
                cx.settings()
                    .set_bool(&apps::app_key(file, "background"), on);
                if on {
                    cx.toast("Runs on when you leave it; it may draw over the screen");
                }
            }
            Setting::AppPinned(file) => {
                let mut pins = apps::load_pins();
                let name = self.app(file).map(|a| a.name.clone()).unwrap_or_default();
                if let Some(i) = pins.iter().position(|p| p == file) {
                    pins.remove(i);
                    cx.toast(&format!("Unpinned {name}"));
                } else {
                    pins.push(file.clone());
                    cx.toast(&format!("Pinned {name}"));
                }
                let _ = apps::save_pins(&pins);
            }
            _ => self.apply_system(s, d, cx),
        }
        self.refresh(cx);
    }

    fn app(&self, file: &str) -> Option<&AppRow> {
        self.state.apps.iter().find(|a| a.file == file)
    }

    /// Wi-Fi, Bluetooth and system settings, which go through helpers.
    fn apply_system(&mut self, s: &Setting, d: i32, cx: &mut Cx) {
        match s {
            Setting::WifiRadio => {
                let on = self.state.wifi_on;
                self.state.wifi_on = !on;
                let arg = if on { "off" } else { "on" };
                self.run(cx, NetJob::SetRadio, &["nmcli", "radio", "wifi", arg]);
            }
            Setting::Bluetooth => {
                let on = self.state.bt_on;
                self.state.bt_on = !on;
                // Stock boots with the radio blocked. The session ACL
                // usually allows the unblock; the helper covers the rest.
                let blocked = !on && rfkill::set(rfkill::Radio::Bluetooth, false).is_err();
                if blocked {
                    let argv = ["pkexec", HELPER, "rfkill", "bluetooth", "on"];
                    self.run(cx, NetJob::BtUnblock, &argv);
                } else {
                    let arg = if on { "off" } else { "on" };
                    self.run(cx, NetJob::BtPower, &["bluetoothctl", "power", arg]);
                }
            }
            Setting::NetworkTime => {
                let arg = if self.state.network_time {
                    "false"
                } else {
                    "true"
                };
                self.run(cx, NetJob::TimeSet(None), &["timedatectl", "set-ntp", arg]);
            }
            Setting::ConnAuto => {
                if let Some(c) = self.open_conn() {
                    let v = if c.autoconnect { "no" } else { "yes" };
                    self.modify(cx, &c, &[("connection.autoconnect", v)], false);
                    self.update_conn(|c| c.autoconnect = !c.autoconnect);
                }
            }
            Setting::ConnMethod => {
                if let Some(c) = self.open_conn() {
                    if c.manual {
                        let props = [
                            ("ipv4.method", "auto"),
                            ("ipv4.addresses", ""),
                            ("ipv4.gateway", ""),
                        ];
                        self.modify(cx, &c, &props, true);
                        self.update_conn(|c| {
                            c.manual = false;
                            c.address.clear();
                            c.gateway.clear();
                        });
                    } else if c.current.is_empty() {
                        // Manual needs an address; ask for one.
                        self.ask_conn_field(&c, ConnField::Address);
                    } else {
                        // Keep what DHCP gave, now fixed.
                        let gw = self
                            .state
                            .ifaces
                            .iter()
                            .find(|i| i.connection == c.name)
                            .map(|i| i.gateway.clone())
                            .unwrap_or_default();
                        let props = [
                            ("ipv4.method", "manual"),
                            ("ipv4.addresses", c.current.as_str()),
                            ("ipv4.gateway", gw.as_str()),
                        ];
                        self.modify(cx, &c, &props, true);
                        self.update_conn(|x| {
                            x.manual = true;
                            x.address = c.current.clone();
                            x.gateway = gw.clone();
                        });
                    }
                }
            }
            Setting::ConnMac => {
                if let Some(c) = self.open_conn() {
                    let i = net::MACS.iter().position(|(k, _)| *k == c.mac).unwrap_or(0);
                    let (next, _) = net::MACS[cycle(i, d, net::MACS.len())];
                    let prop = if c.wifi {
                        "802-11-wireless.cloned-mac-address"
                    } else {
                        "802-3-ethernet.cloned-mac-address"
                    };
                    self.modify(cx, &c, &[(prop, next)], true);
                    self.update_conn(|c| c.mac = next.to_string());
                }
            }
            Setting::Ssh => {
                let (arg, done) = if self.state.ssh {
                    ("off", "SSH off")
                } else {
                    ("on", "SSH on")
                };
                self.helper(cx, &["ssh", arg], done);
            }
            Setting::Logs => {
                let i = Logs::ALL
                    .iter()
                    .position(|&l| l == self.state.logs)
                    .unwrap_or(0);
                let next = Logs::ALL[cycle(i, d, Logs::ALL.len())];
                let done = format!("Logs: {}", next.label());
                self.helper(cx, &["logs", next.verb()], &done);
            }
            _ => cx.toast("Not available yet"),
        }
    }

    fn run_command(&mut self, c: Command, cx: &mut Cx) {
        match c {
            Command::Restart => self.ask(
                Dialog::Confirm {
                    title: "Restart?".into(),
                    line: "Open apps close first.".into(),
                    yes: "restart",
                },
                Ask::Confirm(c),
            ),
            Command::PowerOff => self.ask(
                Dialog::Confirm {
                    title: "Power off?".into(),
                    line: "Open apps close first.".into(),
                    yes: "power off",
                },
                Ask::Confirm(c),
            ),
            Command::Join(ssid) => self.join(ssid, cx),
            Command::JoinHidden => self.ask(
                Dialog::Text {
                    title: "Hidden network".into(),
                    placeholder: "Network name",
                    text: String::new(),
                    masked: false,
                    secret: false,
                },
                Ask::HiddenName,
            ),
            Command::BtScan => {
                if !self.state.bt_scanning {
                    self.state.bt_scanning = true;
                    let argv = ["bluetoothctl", "--timeout", "10", "scan", "on"];
                    self.run(cx, NetJob::BtScan, &argv);
                    self.refresh(cx);
                }
            }
            Command::Device(mac) => self.bt_enter(mac, cx),
            Command::ConnEdit(field) => {
                if let Some(c) = self.open_conn() {
                    self.ask_conn_field(&c, field);
                }
            }
            Command::ConnUp => {
                if let Some(c) = self.open_conn() {
                    let argv = ["nmcli", "--wait", "30", "connection", "up", &c.name];
                    self.run(cx, NetJob::ConnUp(c.name.clone()), &argv);
                    cx.toast("Reconnecting\u{2026}");
                }
            }
            Command::ConnForget => {
                if let Some(c) = self.open_conn() {
                    self.ask(
                        Dialog::Confirm {
                            title: format!("Forget \u{201c}{}\u{201d}?", c.name),
                            line: "Its password and settings go.".into(),
                            yes: "forget",
                        },
                        Ask::ConnForget(c.name),
                    );
                }
            }
            Command::SetZone(zone) => {
                if self.state.zones.contains(&zone) {
                    let argv = ["timedatectl", "set-timezone", &zone];
                    self.run(cx, NetJob::TimeSet(Some(zone.clone())), &argv);
                    self.state.zone = zone;
                    self.stack
                        .retain(|(p, _)| !matches!(p, PageId::TimeZone(_)));
                    self.refresh(cx);
                }
            }
            Command::Password => self.ask(
                Dialog::Text {
                    title: "Current password".into(),
                    placeholder: "Password",
                    text: String::new(),
                    masked: true,
                    secret: true,
                },
                Ask::PasswordCurrent,
            ),
        }
    }

    fn ask(&mut self, d: Dialog, a: Ask) {
        self.dialog = Some((d, a));
    }

    fn dialog_key(&mut self, k: Key, cx: &mut Cx) {
        let Some((dialog, _)) = self.dialog.as_mut() else {
            return;
        };
        match k {
            Key::Enter => {
                if let Some((dialog, ask)) = self.dialog.take() {
                    self.answer(dialog, ask, cx);
                }
            }
            Key::Tab => {
                if let Dialog::Text {
                    masked,
                    secret: true,
                    ..
                } = dialog
                {
                    *masked = !*masked;
                }
            }
            Key::Backspace => {
                if let Dialog::Text { text, .. } = dialog {
                    text.pop();
                }
            }
            Key::Char(c) => {
                if let Dialog::Text { text, .. } = dialog {
                    if text.chars().count() < 64 {
                        text.push(c);
                    }
                }
            }
            _ => {}
        }
    }

    fn answer(&mut self, dialog: Dialog, ask: Ask, cx: &mut Cx) {
        let mut text = match dialog {
            Dialog::Text { text, .. } => text,
            Dialog::Confirm { .. } => String::new(),
        };
        match ask {
            Ask::Confirm(Command::Restart) => self.power(true, cx),
            Ask::Confirm(Command::PowerOff) => self.power(false, cx),
            Ask::Confirm(_) => {}
            Ask::WifiPassword(ssid) => {
                let argv = [
                    "nmcli", "--wait", "45", "--ask", "device", "wifi", "connect", &ssid,
                ];
                self.connect(cx, &ssid, &argv, false, Some(&mut text));
            }
            Ask::HiddenName if !text.is_empty() => self.ask(
                Dialog::Text {
                    title: format!("Join \u{201c}{text}\u{201d}"),
                    placeholder: "Password, empty if open",
                    text: String::new(),
                    masked: true,
                    secret: true,
                },
                Ask::HiddenPassword(text.clone()),
            ),
            Ask::HiddenName => {}
            Ask::PasswordCurrent => self.ask(
                Dialog::Text {
                    title: "New password".into(),
                    placeholder: "Password",
                    text: String::new(),
                    masked: true,
                    secret: true,
                },
                Ask::PasswordNew(std::mem::take(&mut text)),
            ),
            Ask::PasswordNew(cur) => self.ask(
                Dialog::Text {
                    title: "New password again".into(),
                    placeholder: "Password",
                    text: String::new(),
                    masked: true,
                    secret: true,
                },
                Ask::PasswordRepeat(cur, std::mem::take(&mut text)),
            ),
            Ask::PasswordRepeat(mut cur, mut new) => {
                if new != text {
                    cx.toast("The passwords differ");
                } else if new.is_empty() {
                    cx.toast("The password is empty");
                } else {
                    // passwd as the user: it checks the current password
                    // itself, so no root helper can be asked to set one.
                    let mut input = format!("{cur}\n{new}\n{new}\n");
                    self.run_with(cx, NetJob::Password, &["passwd"], input.as_bytes());
                    wipe(&mut input);
                }
                wipe(&mut cur);
                wipe(&mut new);
            }
            Ask::Forget(ssid) => self.forget(cx, &ssid),
            Ask::ConnForget(name) => {
                let argv = ["nmcli", "connection", "delete", "id", &name];
                self.run(cx, NetJob::ConnDelete(name.clone()), &argv);
            }
            Ask::ConnField(name, field) => self.set_conn_field(cx, &name, field, &text),
            Ask::HiddenPassword(ssid) => {
                let argv = [
                    "nmcli", "--wait", "45", "--ask", "device", "wifi", "connect", &ssid, "hidden",
                    "yes",
                ];
                let pw = (!text.is_empty()).then_some(&mut text);
                self.connect(cx, &ssid, &argv, false, pw);
            }
        }
        wipe(&mut text);
    }

    fn set_conn_field(&mut self, cx: &mut Cx, name: &str, field: ConnField, text: &str) {
        let Some(c) = self.open_conn().filter(|c| c.name == name) else {
            return;
        };
        let v = text.trim().to_string();
        match field {
            ConnField::Address if net::valid_ipv4(&v, true) => {
                let v = if v.contains('/') {
                    v
                } else {
                    format!("{v}/24")
                };
                self.modify(
                    cx,
                    &c,
                    &[("ipv4.method", "manual"), ("ipv4.addresses", &v)],
                    true,
                );
            }
            ConnField::Gateway if v.is_empty() || net::valid_ipv4(&v, false) => {
                self.modify(cx, &c, &[("ipv4.gateway", &v)], true);
            }
            ConnField::Dns => {
                let list: Vec<&str> = v.split([',', ' ']).filter(|s| !s.is_empty()).collect();
                if list.iter().all(|s| net::valid_ipv4(s, false)) {
                    let joined = list.join(",");
                    // Typed servers replace the ones DHCP offers.
                    let ignore = if joined.is_empty() { "no" } else { "yes" };
                    self.modify(
                        cx,
                        &c,
                        &[("ipv4.dns", &joined), ("ipv4.ignore-auto-dns", ignore)],
                        true,
                    );
                } else {
                    cx.toast("Not an IPv4 address");
                }
            }
            _ => cx.toast("Not an IPv4 address"),
        }
    }

    /// Restarts or powers off. systemd stops the session first, so every app
    /// gets SIGTERM and closes.
    fn power(&mut self, restart: bool, cx: &mut Cx) {
        let (arg, msg) = if restart {
            ("reboot", "Restarting\u{2026}")
        } else {
            ("off", "Powering off\u{2026}")
        };
        self.helper(cx, &["power", arg], msg);
        cx.toast(msg);
    }

    /// Keys only some pages take. True if the key was used.
    fn page_key(&mut self, k: Key, cx: &mut Cx) -> bool {
        let id = self.top().0.clone();
        match id {
            PageId::Wifi if k == Key::Right => {
                let Some((Act::Run(Command::Join(ssid)), _)) = self.selected_act() else {
                    return false;
                };
                let known = self.state.profiles.contains(&ssid);
                if known {
                    self.query_conn(cx, &ssid);
                    self.stack.push((PageId::Conn(ssid), Cursor::default()));
                    self.refresh(cx);
                }
                known
            }
            PageId::Wifi if k == Key::Char('r') => {
                self.query_net(cx, true);
                cx.toast("Scanning\u{2026}");
                true
            }
            PageId::Wifi if k == Key::Backspace => {
                let ssid = match self.selected_act() {
                    Some((Act::Run(Command::Join(s)) | Act::Open(PageId::Conn(s)), _)) => Some(s),
                    _ => None,
                };
                if let Some(ssid) = ssid {
                    let known = self.state.profiles.contains(&ssid);
                    let active = self
                        .state
                        .networks
                        .iter()
                        .any(|n| n.active && n.ssid == ssid);
                    if known && active {
                        self.ask(
                            Dialog::Confirm {
                                title: format!("Forget \u{201c}{ssid}\u{201d}?"),
                                line: "You are on it now; it drops.".into(),
                                yes: "forget",
                            },
                            Ask::Forget(ssid),
                        );
                    } else if known {
                        self.forget(cx, &ssid);
                    } else {
                        cx.toast(&format!("{ssid} is not saved"));
                    }
                }
                true
            }
            PageId::Bluetooth if k == Key::Char('s') => {
                self.run_command(Command::BtScan, cx);
                true
            }
            PageId::Bluetooth if k == Key::Backspace => {
                if let Some((Act::Run(Command::Device(mac)), _)) = self.selected_act() {
                    if self.bt_device(&mac).is_some_and(|d| d.paired) {
                        let argv = ["bluetoothctl", "remove", &mac];
                        self.run(cx, NetJob::BtRemove(mac.clone()), &argv);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn query_sys(&mut self, cx: &mut Cx) {
        let script = "timedatectl show -p Timezone -p NTP; echo SSH=$(systemctl is-active ssh)";
        self.run(cx, NetJob::SysQuery, &["sh", "-c", script]);
    }

    fn query_volume(&mut self, cx: &mut Cx) {
        self.run(cx, NetJob::Volume, &["wpctl", "get-volume", SINK]);
    }

    /// Runs the root helper through pkexec.
    fn helper(&mut self, cx: &mut Cx, args: &[&str], done: &str) {
        let mut argv = vec!["pkexec", HELPER];
        argv.extend_from_slice(args);
        self.run(cx, NetJob::Helper(done.to_string()), &argv);
    }

    fn query_conn(&mut self, cx: &mut Cx, name: &str) {
        let argv = ["nmcli", "-t", "connection", "show", name];
        self.run(cx, NetJob::ConnQuery, &argv);
    }

    /// The connection page on top, if that is where Settings is.
    fn open_conn(&self) -> Option<pages::Conn> {
        let PageId::Conn(name) = &self.top().0 else {
            return None;
        };
        self.state.conn.clone().filter(|c| c.name == *name)
    }

    /// Changes the open connection's state at once, so a second quick step
    /// starts from the new value rather than the one nmcli last reported.
    fn update_conn(&mut self, f: impl FnOnce(&mut pages::Conn)) {
        if let Some(c) = self.state.conn.as_mut() {
            f(c);
        }
    }

    /// `nmcli connection modify` with `props` as property, value pairs.
    fn modify(&mut self, cx: &mut Cx, c: &pages::Conn, props: &[(&str, &str)], reconnect: bool) {
        let mut argv = vec!["nmcli", "connection", "modify", c.name.as_str()];
        for (k, v) in props {
            argv.push(k);
            argv.push(v);
        }
        let job = NetJob::ConnModify {
            name: c.name.clone(),
            reconnect,
        };
        self.run(cx, job, &argv);
    }

    fn forget(&mut self, cx: &mut Cx, ssid: &str) {
        let argv = ["nmcli", "connection", "delete", "id", ssid];
        self.run(cx, NetJob::Forget(ssid.to_string()), &argv);
    }

    fn query_bt(&mut self, cx: &mut Cx) {
        self.run(cx, NetJob::BtQuery, &["sh", "-c", bt::QUERY]);
    }

    fn bt_device(&self, mac: &str) -> Option<pages::Device> {
        self.state.devices.iter().find(|d| d.mac == mac).cloned()
    }

    /// Enter on a device: connect a paired one, pair, trust and connect a
    /// new one, disconnect a connected one.
    fn bt_enter(&mut self, mac: String, cx: &mut Cx) {
        let Some(d) = self.bt_device(&mac) else {
            return;
        };
        if !bt::is_mac(&mac) {
            return;
        }
        let name = if d.name.is_empty() {
            mac.clone()
        } else {
            d.name.clone()
        };
        let done = if d.connected {
            format!("Disconnected {name}")
        } else if d.paired {
            format!("Connected to {name}")
        } else {
            format!("Paired with {name}")
        };
        let job = NetJob::BtDevice(done);
        if d.connected {
            self.run(cx, job, &["bluetoothctl", "disconnect", &mac]);
            cx.toast(&format!("Disconnecting {name}"));
        } else if d.paired {
            self.run(
                cx,
                job,
                &["bluetoothctl", "--timeout", "20", "connect", &mac],
            );
            cx.toast(&format!("Connecting {name}\u{2026}"));
        } else {
            let script = "bluetoothctl --timeout 20 pair \"$1\" && bluetoothctl trust \"$1\" && bluetoothctl --timeout 20 connect \"$1\"";
            self.run(cx, job, &["sh", "-c", script, "sh", &mac]);
            cx.toast(&format!("Pairing {name}\u{2026}"));
        }
    }

    /// Asks nmcli for the radio, the networks, the saved profiles and the
    /// devices. `rescan` makes the radio look again, which takes seconds.
    fn query_net(&mut self, cx: &mut Cx, rescan: bool) {
        self.run(cx, NetJob::Radio, &["nmcli", "-t", "-f", "WIFI", "radio"]);
        let profiles = ["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"];
        self.run(cx, NetJob::Profiles, &profiles);
        let scan = if rescan { "yes" } else { "no" };
        let list = [
            "nmcli",
            "-t",
            "-f",
            "IN-USE,SIGNAL,SECURITY,SSID",
            "device",
            "wifi",
            "list",
            "--rescan",
            scan,
        ];
        self.run(cx, NetJob::List, &list);
        let devices = [
            "nmcli",
            "-t",
            "-f",
            "GENERAL.DEVICE,GENERAL.TYPE,GENERAL.STATE,GENERAL.CONNECTION,GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
            "device",
            "show",
        ];
        self.run(cx, NetJob::Devices, &devices);
    }

    fn ask_conn_field(&mut self, c: &pages::Conn, field: ConnField) {
        let (title, placeholder, now) = match field {
            ConnField::Address => ("Address", "192.168.1.50/24", &c.address),
            ConnField::Gateway => ("Gateway", "192.168.1.1", &c.gateway),
            ConnField::Dns => ("DNS", "1.1.1.1, 9.9.9.9", &c.dns),
        };
        self.ask(
            Dialog::Text {
                title: title.into(),
                placeholder,
                text: now.clone(),
                masked: false,
                secret: false,
            },
            Ask::ConnField(c.name.clone(), field),
        );
    }

    /// Enter on a network: a saved one comes up, an open one joins, a
    /// secured one asks for its password first.
    fn join(&mut self, ssid: String, cx: &mut Cx) {
        let Some(n) = self.state.networks.iter().find(|n| n.ssid == ssid).cloned() else {
            return;
        };
        if n.active {
            cx.toast(&format!("Connected to {ssid}"));
        } else if n.known {
            let argv = ["nmcli", "--wait", "30", "connection", "up", "id", &ssid];
            self.connect(cx, &ssid, &argv, true, None);
        } else if n.enterprise {
            cx.toast("Needs a user name: join it from the terminal");
        } else if n.secure {
            self.ask(
                Dialog::Text {
                    title: format!("Join \u{201c}{ssid}\u{201d}"),
                    placeholder: "Password",
                    text: String::new(),
                    masked: true,
                    secret: true,
                },
                Ask::WifiPassword(ssid),
            );
        } else {
            let argv = ["nmcli", "--wait", "45", "device", "wifi", "connect", &ssid];
            self.connect(cx, &ssid, &argv, false, None);
        }
    }

    /// Runs a join. The password, if any, goes on stdin (`--ask`), never in
    /// argv, where any user could read it in /proc.
    fn connect(
        &mut self,
        cx: &mut Cx,
        ssid: &str,
        argv: &[&str],
        known: bool,
        password: Option<&mut String>,
    ) {
        let kind = NetJob::Join {
            ssid: ssid.to_string(),
            known,
            gave_password: password.is_some(),
        };
        match password {
            Some(p) => {
                let mut line = format!("{p}\n");
                wipe(p);
                self.run_with(cx, kind, argv, line.as_bytes());
                wipe(&mut line);
            }
            None => self.run(cx, kind, argv),
        }
        self.state.wifi_busy = Some(ssid.to_string());
        self.refresh(cx);
    }

    fn job_done(&mut self, kind: NetJob, out: String, ok: bool, cx: &mut Cx) {
        let mut requery = false;
        match &kind {
            NetJob::Join {
                ssid,
                known,
                gave_password,
            } => {
                if ok {
                    cx.toast(&format!("Connected to {ssid}"));
                } else {
                    // A failed first join leaves a profile behind, wrong
                    // password and all; the next try would not ask again.
                    if !known {
                        let argv = ["nmcli", "connection", "delete", "id", ssid];
                        self.run(cx, NetJob::Ignore, &argv);
                    }
                    cx.toast(&net::failure(&out, *gave_password));
                }
                requery = true;
            }
            NetJob::Forget(ssid) => {
                cx.toast(&if ok {
                    format!("Forgot {ssid}")
                } else {
                    net::failure(&out, false)
                });
                requery = true;
            }
            NetJob::SetRadio => {
                if !ok {
                    cx.toast(&net::failure(&out, false));
                }
                requery = true;
            }
            NetJob::BtPower | NetJob::BtRemove(_) => {
                if !ok {
                    cx.toast(&bt::failure(&out));
                }
                self.query_bt(cx);
            }
            NetJob::BtScan => self.query_bt(cx),
            NetJob::BtUnblock => {
                if ok {
                    self.run(cx, NetJob::BtPower, &["bluetoothctl", "power", "on"]);
                } else {
                    cx.toast(&helper_failure(&out));
                    self.query_bt(cx);
                }
            }
            NetJob::TimeSet(zone) => {
                match (ok, zone) {
                    // The C library reads /etc/localtime once; naming the
                    // zone makes this app's clock follow at once.
                    // SAFETY: the app is single-threaded.
                    (true, Some(z)) => unsafe { std::env::set_var("TZ", z) },
                    (true, None) => {}
                    (false, _) => cx.toast(&helper_failure(&out)),
                }
                self.query_sys(cx);
            }
            NetJob::Helper(done) => {
                cx.toast(&if ok {
                    done.clone()
                } else {
                    helper_failure(&out)
                });
                self.query_sys(cx);
            }
            NetJob::ConnModify { name, reconnect } => {
                if !ok {
                    cx.toast(&net::failure(&out, false));
                } else if *reconnect {
                    // Not applied at once: on the network you work over, a
                    // new address or MAC would cut you off mid-change.
                    cx.toast("Saved. Reconnect applies it");
                }
                self.query_conn(cx, name);
            }
            NetJob::ConnUp(name) => {
                cx.toast(&if ok {
                    format!("{name} is up")
                } else {
                    net::failure(&out, false)
                });
                self.query_conn(cx, name);
                requery = true;
            }
            NetJob::ConnDelete(name) => {
                cx.toast(&if ok {
                    format!("Forgot {name}")
                } else {
                    net::failure(&out, false)
                });
                self.stack.retain(|(p, _)| !matches!(p, PageId::Conn(_)));
                requery = true;
            }
            NetJob::Password => cx.toast(&if ok {
                "Password changed".to_string()
            } else {
                password_failure(&out)
            }),
            NetJob::BtDevice(done) => {
                cx.toast(&if ok { done.clone() } else { bt::failure(&out) });
                self.query_bt(cx);
            }
            _ => {}
        }
        let st = &mut self.state;
        match kind {
            NetJob::Radio => {
                if let Some(on) = net::parse_radio(&out) {
                    st.wifi_on = on;
                }
            }
            NetJob::ConnQuery if ok => st.conn = Some(net::parse_conn(&out)),
            NetJob::Profiles if ok => {
                st.conns = net::parse_conns(&out);
                st.profiles = net::parse_profiles(&out);
                for n in &mut st.networks {
                    n.known = st.profiles.contains(&n.ssid);
                }
            }
            NetJob::List if ok => st.networks = net::parse_list(&out, &st.profiles),
            NetJob::Devices if ok => st.ifaces = net::parse_devices(&out),
            NetJob::Join { .. } => st.wifi_busy = None,
            NetJob::BtQuery => {
                let (on, devices) = bt::parse_query(&out);
                st.bt_on = on;
                st.devices = devices;
            }
            NetJob::BtScan => st.bt_scanning = false,
            NetJob::SysQuery => {
                for line in out.lines() {
                    match line.split_once('=') {
                        Some(("Timezone", z)) => st.zone = z.to_string(),
                        Some(("NTP", v)) => st.network_time = v == "yes",
                        Some(("SSH", v)) => st.ssh = v == "active",
                        _ => {}
                    }
                }
            }
            NetJob::Zones if ok => {
                st.zones = out
                    .lines()
                    .map(str::to_string)
                    .filter(|z| z.contains('/'))
                    .collect();
            }
            NetJob::Volume => {
                if let Some((v, muted)) = parse_volume(&out) {
                    st.volume = Some(v);
                    st.muted = muted;
                }
            }
            _ => {}
        }
        if requery {
            self.query_net(cx, false);
        }
        self.refresh(cx);
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings::new()
    }
}

impl Page for Settings {
    fn title(&self) -> &str {
        &self.view.title
    }

    fn bars(&self) -> Bars {
        let item = self.view.item(self.top().1.selected);
        Bars::top().hints(model::hints(&self.view, item))
    }

    fn typing(&self) -> bool {
        matches!(self.dialog, Some((Dialog::Text { .. }, _)))
    }

    fn start(&mut self, cx: &mut Cx) {
        self.refresh(cx);
        self.query_net(cx, false);
        self.query_bt(cx);
        self.query_sys(cx);
        self.query_volume(cx);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.dialog.is_some() {
            self.dialog_key(key, cx);
            return;
        }
        if self.page_key(key, cx) {
            return;
        }
        match key {
            Key::Enter => self.activate(cx),
            Key::Up => self.move_cursor(-1, cx),
            Key::Down => self.move_cursor(1, cx),
            Key::Left => self.change(-1, cx),
            Key::Right => self.change(1, cx),
            Key::Char(d @ '1'..='9') => self.pick(d as usize - '1' as usize, cx),
            // The shell acts on these keys; show the new values.
            Key::Other(codes::VOLUMEUP | codes::VOLUMEDOWN | codes::MUTE) => self.query_volume(cx),
            Key::Other(codes::BRIGHTNESSUP | codes::BRIGHTNESSDOWN) => self.refresh(cx),
            _ => {}
        }
    }

    /// Esc closes a dialog, then goes up a page. On the first page it leaves
    /// the app, like any other.
    fn back(&mut self, cx: &mut Cx) -> bool {
        if let Some((dialog, _)) = self.dialog.take() {
            if let Dialog::Text { mut text, .. } = dialog {
                wipe(&mut text);
            }
            return false;
        }
        if self.stack.len() > 1 {
            self.stack.pop();
            self.refresh(cx);
            return false;
        }
        true
    }

    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        match event {
            JobEvent::Line(line) => {
                if let Some((_, out)) = self.jobs.get_mut(&job) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            JobEvent::Done(code) => {
                if let Some((kind, out)) = self.jobs.remove(&job) {
                    self.job_done(kind, out, code == Some(0), cx);
                }
            }
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let dialog = self.dialog.as_ref().map(|(d, _)| d);
        model::draw(ui.canvas(), &t, &self.view, self.top().1, dialog);
    }
}

fn page_rows(t: &Theme, view: &View) -> usize {
    match view.body {
        Body::List(_) => model::rows(t).1,
        Body::Info(_) => model::lines(t).1,
    }
}

/// The logging level now: the last `Storage=` journald reads wins, as
/// journald itself decides.
fn logs_level() -> Logs {
    let mut files: Vec<std::path::PathBuf> = vec!["/etc/systemd/journald.conf".into()];
    if let Ok(rd) = std::fs::read_dir(LOGGING_DIR) {
        let mut drop_ins: Vec<_> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "conf"))
            .collect();
        drop_ins.sort();
        files.extend(drop_ins);
    }
    let mut storage = String::from("auto");
    for f in files {
        if let Ok(s) = std::fs::read_to_string(f) {
            for l in s.lines() {
                if let Some(v) = l.trim().strip_prefix("Storage=") {
                    storage = v.trim().to_string();
                }
            }
        }
    }
    match storage.as_str() {
        "none" => Logs::Off,
        "volatile" => Logs::Ram,
        "persistent" => Logs::Persistent,
        _ if Path::new("/var/log/journal").is_dir() => Logs::Persistent,
        _ => Logs::Ram,
    }
}

fn helper_failure(out: &str) -> String {
    if out.contains("Not authorized") || out.contains("dismissed") || out.contains("polkit") {
        return "Not allowed: polkit rule missing".into();
    }
    out.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("gh0st-helper: ").to_string())
        .unwrap_or_else(|| "Failed".into())
}

fn password_failure(out: &str) -> String {
    if out.contains("Authentication token manipulation") || out.contains("Authentication failure") {
        "Wrong current password".into()
    } else {
        out.lines()
            .rev()
            .find(|l| l.contains("BAD PASSWORD") || l.contains("passwd:"))
            .map(|l| l.trim().to_string())
            .unwrap_or_else(|| "Password not changed".into())
    }
}

/// `wpctl get-volume`: "Volume: 0.40" or "Volume: 0.65 [MUTED]".
fn parse_volume(s: &str) -> Option<(u8, bool)> {
    let v: f32 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some((
        (v * 100.0).round().clamp(0.0, 150.0) as u8,
        s.contains("[MUTED]"),
    ))
}

/// Overwrites a secret before its memory is freed.
fn wipe(s: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { s.as_bytes_mut().fill(0) };
    s.clear();
}

fn cycle(i: usize, d: i32, n: usize) -> usize {
    (i as i64 + if d < 0 { -1 } else { 1 }).rem_euclid(n as i64) as usize
}

/// Readouts for About, from procfs and sysfs.
fn about() -> Vec<(String, String)> {
    let mut out = vec![(
        "Version".to_string(),
        format!("Gh0stdeck {}", system::version()),
    )];
    let read = |p: &str| std::fs::read_to_string(p).ok();
    if let Some(os) = read("/etc/os-release").and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    }) {
        out.push(("OS".into(), os));
    }
    if let Some(k) = system::kernel() {
        out.push(("Kernel".into(), k));
    }
    if let Some((used, total)) = system::memory() {
        out.push(("Memory".into(), format!("{used} MB of {total} MB used")));
    }
    if let Some((used, total)) = system::storage() {
        out.push((
            "Storage".into(),
            format!("{} of {} used", size(used << 20), size(total << 20)),
        ));
    }
    if let Some(b) = system::battery() {
        let charging = if b.charging { ", charging" } else { "" };
        out.push(("Battery".into(), format!("{}%{charging}", b.percent)));
    }
    if let Some(up) =
        read("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
    {
        let mins = up as u64 / 60;
        let text = match mins / 60 {
            0 => format!("{mins} min"),
            h => format!("{h} h {} min", mins % 60),
        };
        out.push(("Uptime".into(), text));
    }
    if let Some(h) = read("/proc/sys/kernel/hostname") {
        out.push(("Host".into(), h.trim().into()));
    }
    out
}

fn size(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{} MB", b / 1_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_wrap_both_ways() {
        assert_eq!(cycle(0, -1, 3), 2);
        assert_eq!(cycle(2, 1, 3), 0);
    }

    #[test]
    fn sizes_read_like_a_person_wrote_them() {
        assert_eq!(size(3_100_000_000), "3.1 GB");
        assert_eq!(size(29_000_000_000), "29 GB");
        assert_eq!(size(512_000_000), "512 MB");
    }

    #[test]
    fn wpctl_volume_reads_as_percent_and_mute() {
        assert_eq!(parse_volume("Volume: 0.40\n"), Some((40, false)));
        assert_eq!(parse_volume("Volume: 0.65 [MUTED]\n"), Some((65, true)));
        assert_eq!(parse_volume("nope"), None);
    }
}
