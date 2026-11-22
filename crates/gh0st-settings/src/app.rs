//! Settings as an app: the page stack, its keys, and what each setting
//! writes. Every config write loads config.json fresh, changes one key and
//! saves, so stock apps' keys survive. The shell reads the file again each
//! time it takes the screen back, so home follows what changed here.

use crate::model::pages::{
    self, APP_ICONS, AppRow, DARK_TIMES, DARK_WIFI, DEFAULT_LOCK_AFTER, LOCK_AFTERS, Logs,
    State as PageState,
};
use crate::model::{
    self, Act, Body, Choice, Command, ConnField, Cursor, Dialog, Page as View, PageId, Setting,
    Value,
};
use crate::model::{bt, net};
use gh0st_app::gfx::theme::{self, Theme};
use gh0st_app::prelude::*;
use gh0st_app::{codes, system};
use gh0st_core::apps::{self, Dirs, Layout};
use gh0st_core::backlight::Backlight;
use gh0st_core::config::{self, Config};
use gh0st_core::sysconf::{self, CpuMode, GpuMem};
use gh0st_core::{desktop, rfkill, rt};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// pkexec runs the root helper by this path (see packaging/polkit).
const HELPER: &str = "/usr/libexec/gh0st-helper";
const LOGGING_DIR: &str = "/etc/systemd/journald.conf.d";
const GOVERNOR: &str = "/sys/devices/system/cpu/cpufreq/policy0/scaling_governor";
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
    /// The password for a helper call that asked for one.
    Auth(HelperCall),
    DeviceName,
    Swap,
    DateTime,
}

/// Settings as one step of first-boot setup: one page with its label in the
/// top bar, and the chips the step needs. It calls `cx.done()` once the
/// step's work has finished: a country or a time zone set, a network joined.
#[derive(Clone, Debug)]
pub struct Embed {
    /// The page the step shows.
    pub page: PageId,
    /// "Setup 3 of 8".
    pub label: String,
    /// Skip on 7: on to the next step, changing nothing.
    pub skip: bool,
    /// Done on 6, for a page where Enter works the rows.
    pub done: bool,
}

/// What a chip does: the key it stands for, or the step is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChipDo {
    Key(Key),
    Finish,
}

/// A root helper run, kept so a password prompt can run it again. It never
/// holds the password.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HelperCall {
    args: Vec<String>,
    /// What to say when it works.
    done: String,
    /// The change waits for a restart, so offer one.
    restart: bool,
}

impl HelperCall {
    fn new(args: &[&str], done: impl Into<String>) -> HelperCall {
        HelperCall {
            args: args.iter().map(|a| a.to_string()).collect(),
            done: done.into(),
            restart: false,
        }
    }
}

/// What a finished helper run means for the user.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Done,
    /// Ask for the password, with this title.
    Ask(&'static str),
    Locked(u64),
    Failed(String),
}

/// Reads the helper's exit codes (see `gh0st/src/helper.rs`).
fn auth_step(code: Option<i32>, out: &str) -> Step {
    match code {
        Some(0) => Step::Done,
        Some(10) => Step::Ask("Password"),
        Some(11) => Step::Ask("Wrong password"),
        Some(12) => Step::Locked(
            out.lines()
                .find_map(|l| l.trim().strip_prefix("gh0st-helper: locked "))
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(0),
        ),
        _ => Step::Failed(helper_failure(out)),
    }
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
    Helper(HelperCall),
    /// `vcgencmd get_mem gpu`: what the firmware runs with.
    GpuMem,
    /// After the helper unblocks the radio, power it on.
    BtUnblock,
    /// The image boots with Wi-Fi blocked: setup's network step unblocks it,
    /// then turns it on.
    WifiUnblock,
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
    /// A dialog the page wants, and what its answer is for. `asked` is set
    /// once the SDK shows it.
    dialog: Option<(Dialog, Ask)>,
    asked: bool,
    /// Running helpers, with what each is for and its output so far.
    jobs: HashMap<Job, (NetJob, String)>,
    backlight: Option<Backlight>,
    /// vcgencmd was asked once; the running value holds until a restart.
    gpu_asked: bool,
    /// A step of setup, when Settings runs as one.
    embed: Option<Embed>,
    /// The step is done once the helper or job now running succeeds.
    finish_pending: bool,
    /// Done or Skip was pressed while a change was still being made: go on
    /// once it is, as closing the step would stop it halfway.
    finish_when_idle: bool,
}

impl Settings {
    pub fn new() -> Settings {
        let state = PageState::default();
        Settings {
            stack: vec![(PageId::Root, Cursor::default())],
            view: pages::build(&PageId::Root, &state),
            state,
            dialog: None,
            asked: false,
            jobs: HashMap::new(),
            backlight: Backlight::find(Path::new("/sys/class/backlight")),
            gpu_asked: false,
            embed: None,
            finish_pending: false,
            finish_when_idle: false,
        }
    }

    /// One page of Settings as a step of first-boot setup.
    pub fn embedded(embed: Embed) -> Settings {
        let mut s = Settings::new();
        s.stack = vec![(embed.page.clone(), Cursor::default())];
        s.view = pages::build(&embed.page, &s.state);
        s.embed = Some(embed);
        s
    }

    /// The step's work is done: on to the next.
    fn finish(&mut self, cx: &mut Cx) {
        if self.embed.is_some() {
            self.finish_pending = false;
            self.finish_when_idle = false;
            cx.done();
        }
    }

    /// A change is still being made: a helper, a time zone, a radio, a join.
    /// Closing the step now would stop it halfway.
    fn changing(&self) -> bool {
        self.jobs.values().any(|(kind, _)| {
            matches!(
                kind,
                NetJob::Helper(_)
                    | NetJob::TimeSet(_)
                    | NetJob::SetRadio
                    | NetJob::WifiUnblock
                    | NetJob::BtPower
                    | NetJob::BtUnblock
                    | NetJob::Join { .. }
                    | NetJob::Forget(_)
            )
        })
    }

    /// Done or Skip: on now, or once the change in hand is made.
    fn finish_soon(&mut self, cx: &mut Cx) {
        if self.changing() {
            self.finish_when_idle = true;
            cx.toast("One moment");
        } else {
            self.finish(cx);
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
        // The SDK switches the theme only after this key is handled, so the
        // file, not the theme, has the palette just picked.
        s.palette = config
            .get_str("gh0st.palette")
            .and_then(theme::find)
            .map_or(cx.theme().c.name, |p| p.name);
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
        // Pins first in their order, as home shows them.
        rows.sort_by_key(|a| {
            let pin = pins.iter().position(|p| *p == a.file);
            (pin.unwrap_or(usize::MAX), a.name.to_lowercase())
        });
        s.apps = rows;
        s.about = about();
        s.logs = logs_level();
        s.nm = sysconf::nm_privacy(&nm_files());
        s.lock_password = config.get_bool("gh0st.lock.password").unwrap_or(true);
        s.lock_after = config
            .get_int("gh0st.lock.after")
            .unwrap_or(DEFAULT_LOCK_AFTER);
        s.dark_wifi = config.get_int("gh0st.dark.wifi_off").filter(|&t| t >= 0);
        let (y, mon, d, h, min) = rt::local_ymdhm();
        s.now = format!("{y}-{mon:02}-{d:02} {h:02}:{min:02}");
    }

    /// Rebuilds the page on top from fresh local state.
    fn refresh(&mut self, cx: &mut Cx) {
        self.fill_local(cx);
        let id = self.top().0.clone();
        self.view = pages::build(&id, &self.state);
        let per = page_rows(cx.theme(), &self.view);
        let len = self.view.len();
        let info = matches!(self.view.body, Body::Info(_));
        // In setup the radio is on already: the cursor starts, and stays,
        // on the networks, so Enter can't turn Wi-Fi off.
        let wifi_step = self.embed.is_some() && id == PageId::Wifi;
        let cur = self.cursor_mut();
        if !info {
            if wifi_step && cur.selected == 0 && len > 1 {
                cur.selected = 1;
            }
            cur.fit(len, per);
        }
    }

    fn move_cursor(&mut self, d: i32, cx: &Cx) {
        let per = page_rows(cx.theme(), &self.view);
        let len = self.view.len();
        let info = matches!(self.view.body, Body::Info(_));
        let wifi_step = self.embed.is_some() && self.top().0 == PageId::Wifi;
        let cur = self.cursor_mut();
        cur.step(d, len, per, info);
        // Setup's network step keeps off the Wi-Fi switch (see refresh).
        if wifi_step && cur.selected == 0 && len > 1 {
            cur.selected = 1;
        }
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
                if matches!(id, PageId::System | PageId::Advanced | PageId::Swap) {
                    self.query_advanced(cx);
                }
                self.stack.push((id, Cursor::default()));
                self.refresh(cx);
                // A picker opens on the option in use.
                let here = match &self.view.body {
                    Body::List(items) => items.iter().position(|i| i.icon == Some("check")),
                    Body::Info(_) => None,
                };
                if let Some(i) = here {
                    self.cursor_mut().selected = i;
                    self.refresh(cx);
                }
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
                // Built-ins, then the user's colour schemes. Phantom joins
                // once setup's training is done.
                let mut all = theme::all();
                if cx.settings().get_bool("gh0st.tour_done") == Some(true) {
                    all.splice(
                        theme::PALETTES.len()..theme::PALETTES.len(),
                        theme::EARNED.iter().copied(),
                    );
                }
                let i = all
                    .iter()
                    .position(|p| p.name == self.state.palette)
                    .unwrap_or(0);
                let p = all[cycle(i, d, all.len())];
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
            Setting::LockPassword => {
                let on = self.state.lock_password;
                cx.settings().set_bool("gh0st.lock.password", !on);
            }
            Setting::DarkWifi => {
                let i = DARK_WIFI
                    .iter()
                    .position(|&t| t == self.state.dark_wifi)
                    .unwrap_or(0);
                let next = DARK_WIFI[cycle(i, d, DARK_WIFI.len())];
                cx.settings()
                    .set_int("gh0st.dark.wifi_off", next.unwrap_or(-1));
            }
            Setting::UsbPower => {
                let on = self.state.usb_power == Some(true);
                let done = if on {
                    "USB power saving off"
                } else {
                    "USB power saving on"
                };
                self.helper(cx, &["usb-power", if on { "off" } else { "on" }], done);
            }
            Setting::LockAfter => {
                let i = LOCK_AFTERS
                    .iter()
                    .position(|&t| t == self.state.lock_after)
                    .unwrap_or(2);
                let next = LOCK_AFTERS[cycle(i, d, LOCK_AFTERS.len())];
                cx.settings().set_int("gh0st.lock.after", next);
            }
            _ => self.apply_system(s, d, cx),
        }
        self.refresh(cx);
    }

    /// Runs a picked option through the helper, unless it is the one set.
    fn choose(&mut self, cx: &mut Cx, c: Choice, word: &str) {
        let s = &self.state;
        let call = match c {
            Choice::GpuMem => {
                if word.parse().ok().map(GpuMem::Set) == Some(s.gpu_mem) {
                    return;
                }
                let mut call =
                    HelperCall::new(&["gpu-mem", word], format!("GPU memory: {word} MB"));
                call.restart = true;
                call
            }
            Choice::CpuMode => {
                let Some(m) = CpuMode::from_verb(word) else {
                    return;
                };
                if CpuMode::from_governor(&s.governor) == Some(m) {
                    return;
                }
                HelperCall::new(&["cpu", word], format!("CPU mode: {}", m.label()))
            }
            Choice::Logs => {
                let Some(l) = Logs::ALL.into_iter().find(|l| l.verb() == word) else {
                    return;
                };
                if l == s.logs {
                    return;
                }
                HelperCall::new(&["logs", word], format!("Logs: {}", l.label()))
            }
            Choice::Compression => {
                let now = s.swap_comp.as_deref().or(s.swap_comp_running.as_deref());
                if !sysconf::COMPRESSIONS.contains(&word) || now == Some(word) {
                    return;
                }
                let mut call =
                    HelperCall::new(&["swap-comp", word], format!("Compression: {word}"));
                call.restart = true;
                call
            }
            Choice::Swappiness => {
                if word.parse().ok() == s.swappiness {
                    return;
                }
                HelperCall::new(&["swappiness", word], format!("Swappiness: {word}"))
            }
        };
        self.helper_call(cx, call);
    }

    /// The app under the cursor on the Apps page.
    fn selected_app(&self) -> Option<&AppRow> {
        self.state.apps.get(self.top().1.selected)
    }

    /// Moves the pin being moved one place, saves, and keeps the cursor on it.
    fn move_pin(&mut self, d: i32, cx: &mut Cx) {
        let Some(file) = self.state.moving.clone() else {
            return;
        };
        let visible: Vec<String> = self.state.apps.iter().map(|a| a.file.clone()).collect();
        if let Some(pins) = apps::move_pin(&apps::load_pins(), &visible, &file, d) {
            let _ = apps::save_pins(&pins);
            self.refresh(cx);
            if let Some(i) = self.state.apps.iter().position(|a| a.file == file) {
                self.cursor_mut().selected = i;
                self.refresh(cx);
            }
        }
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
            Setting::WifiMac => {
                let (arg, done) = match self.state.nm.mac_private {
                    true => ("device", "Wi-Fi uses the device's address"),
                    false => ("stable", "An address for each network"),
                };
                self.helper(cx, &["mac", arg], done);
            }
            Setting::ScanMac => {
                let (arg, done) = match self.state.nm.scan_random {
                    true => ("device", "Scans use the device's address"),
                    false => ("random", "Scans use a random address"),
                };
                self.helper(cx, &["scan-mac", arg], done);
            }
            Setting::SendHostname => {
                let (arg, done) = match self.state.nm.send_hostname {
                    true => ("off", "The device name stays private"),
                    false => ("on", "Networks see the device name"),
                };
                self.helper(cx, &["dhcp-hostname", arg], done);
            }
            Setting::Ip6Privacy => {
                let (arg, done) = match self.state.nm.ip6_privacy {
                    true => ("off", "IPv6 privacy off"),
                    false => ("on", "IPv6 privacy on"),
                };
                self.helper(cx, &["ip6-privacy", arg], done);
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
                    no: "cancel",
                },
                Ask::Confirm(c),
            ),
            Command::PowerOff => self.ask(
                Dialog::Confirm {
                    title: "Power off?".into(),
                    line: "Open apps close first.".into(),
                    yes: "power off",
                    no: "cancel",
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
                            no: "cancel",
                        },
                        Ask::ConnForget(c.name),
                    );
                }
            }
            Command::SetZone(zone) => {
                // Before the full list arrives, timedatectl checks the name.
                if self.state.zones.is_empty() || self.state.zones.contains(&zone) {
                    let argv = ["timedatectl", "set-timezone", &zone];
                    self.run(cx, NetJob::TimeSet(Some(zone.clone())), &argv);
                    self.state.zone = zone;
                    if self.embed.is_some() {
                        self.finish_pending = true;
                    } else {
                        self.stack
                            .retain(|(p, _)| !matches!(p, PageId::TimeZone(_)));
                    }
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
            Command::SetCountry(cc) => {
                let change = self.state.country.as_deref() != Some(cc.as_str());
                if change {
                    let name = self
                        .state
                        .countries
                        .iter()
                        .find(|(c, _)| *c == cc)
                        .map_or_else(|| cc.clone(), |(_, n)| n.clone());
                    let mut call =
                        HelperCall::new(&["regdom", &cc], format!("Wi-Fi country: {name}"));
                    // Setup goes on; the country applies from the next boot.
                    call.restart = self.embed.is_none();
                    self.helper_call(cx, call);
                }
                match (&self.embed, change) {
                    (Some(_), true) => self.finish_pending = true,
                    (Some(_), false) => self.finish(cx),
                    (None, _) => self.stack.retain(|(p, _)| !matches!(p, PageId::Country(_))),
                }
                self.refresh(cx);
            }
            Command::Choose(c, word) => {
                self.stack.retain(|(p, _)| !matches!(p, PageId::Choose(_)));
                self.choose(cx, c, &word);
                self.refresh(cx);
            }
            Command::DeviceName => self.ask(
                Dialog::Text {
                    title: "Device name".into(),
                    placeholder: "Letters, digits and hyphens",
                    text: self.state.hostname.clone(),
                    masked: false,
                    secret: false,
                },
                Ask::DeviceName,
            ),
            Command::SetTime => self.ask(
                Dialog::Text {
                    title: "Date and time".into(),
                    placeholder: "2026-11-21 14:32",
                    text: self.state.now.clone(),
                    masked: false,
                    secret: false,
                },
                Ask::DateTime,
            ),
            Command::SwapSize => self.ask(
                Dialog::Text {
                    title: "Swap size".into(),
                    placeholder: "MB, 0 for off",
                    text: self
                        .state
                        .swap_shown()
                        .map_or_else(String::new, |n| n.to_string()),
                    masked: false,
                    secret: false,
                },
                Ask::Swap,
            ),
        }
    }

    fn ask(&mut self, d: Dialog, a: Ask) {
        self.dialog = Some((d, a));
    }

    /// Hands a dialog the page wants to the SDK, once.
    fn flush(&mut self, cx: &mut Cx) {
        let Some((d, _)) = self.dialog.as_ref().filter(|_| !self.asked) else {
            return;
        };
        self.asked = true;
        cx.ask(ASK, model::sdk_dialog(d));
    }

    fn answered(&mut self, mut text: String, ask: Ask, cx: &mut Cx) {
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
            Ask::Auth(call) => {
                if !text.is_empty() {
                    self.helper_with_password(cx, call, &mut text);
                }
            }
            Ask::DeviceName => {
                let name = text.trim().to_string();
                if !sysconf::valid_hostname(&name) {
                    cx.toast("Letters, digits and hyphens only");
                } else if name != self.state.hostname {
                    let done = format!("Device name: {name}");
                    self.helper(cx, &["hostname", &name], &done);
                }
            }
            Ask::Swap => self.set_swap(cx, text.trim()),
            Ask::DateTime => {
                let epoch = pages::parse_date_time(&text)
                    .and_then(|(y, mon, d, h, min)| rt::local_epoch(y, mon, d, h, min));
                match epoch {
                    Some(t) => self.helper(cx, &["set-time", &t.to_string()], "Clock set"),
                    None => cx.toast("Type it as 2026-11-21 14:32"),
                }
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

    /// Takes effect at the next boot, so the helper call offers a restart.
    fn set_swap(&mut self, cx: &mut Cx, text: &str) {
        let max = sysconf::swap_max(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
        let mb = text
            .parse::<u32>()
            .ok()
            .filter(|&n| n <= max && text.bytes().all(|b| b.is_ascii_digit()));
        let Some(mb) = mb else {
            cx.toast(&format!("0 to {max} MB"));
            return;
        };
        if Some(mb) == self.state.swap_shown() {
            return;
        }
        let done = if mb == 0 {
            "Swap: off".to_string()
        } else {
            format!("Swap: {mb} MB")
        };
        let mut call = HelperCall::new(&["swap", &mb.to_string()], done);
        call.restart = true;
        self.helper_call(cx, call);
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
            PageId::Apps if self.state.moving.is_some() => match k {
                Key::Up | Key::Down => {
                    self.move_pin(if k == Key::Up { -1 } else { 1 }, cx);
                    true
                }
                Key::Enter | Key::Char('m') => {
                    self.state.moving = None;
                    self.refresh(cx);
                    true
                }
                // A digit would open another app's page mid-move.
                _ => true,
            },
            PageId::Apps if k == Key::Char('m') => {
                match self.selected_app().filter(|a| a.pinned) {
                    Some(a) => {
                        self.state.moving = Some(a.file.clone());
                        self.refresh(cx);
                        cx.toast("Up and Down move it; Enter when done");
                    }
                    None => cx.toast("Pin an app to move it"),
                }
                true
            }
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
                                no: "cancel",
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
            PageId::Country(None) => {
                let Key::Char(c) = k else { return false };
                if !c.is_ascii_alphabetic() {
                    return false;
                }
                let label = c.to_ascii_uppercase().to_string();
                let found = match &self.view.body {
                    Body::List(items) => items.iter().position(|i| i.label == label),
                    Body::Info(_) => None,
                };
                if let Some(i) = found {
                    let per = page_rows(cx.theme(), &self.view);
                    let len = self.view.len();
                    let cur = self.cursor_mut();
                    cur.selected = i;
                    cur.fit(len, per);
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
        self.helper_call(cx, HelperCall::new(args, done));
    }

    /// First without a password: within 5 minutes of the last one, or for
    /// an open verb, none is needed. The helper says when it is.
    fn helper_call(&mut self, cx: &mut Cx, call: HelperCall) {
        let mut argv = vec!["pkexec", HELPER];
        argv.extend(call.args.iter().map(String::as_str));
        let job = cx.spawn(&argv);
        self.jobs.insert(job, (NetJob::Helper(call), String::new()));
    }

    /// The same call again, with the password on stdin.
    fn helper_with_password(&mut self, cx: &mut Cx, call: HelperCall, password: &mut String) {
        let mut argv = vec!["pkexec", HELPER];
        argv.extend(call.args.iter().map(String::as_str));
        let mut input = format!("{password}\n");
        let job = cx.spawn_with_input(&argv, input.as_bytes());
        self.jobs.insert(job, (NetJob::Helper(call), String::new()));
        wipe(&mut input);
        wipe(password);
    }

    fn ask_password(&mut self, cx: &mut Cx, call: HelperCall, title: &'static str) {
        if self.dialog.is_some() {
            cx.toast("A password is needed");
            return;
        }
        self.ask(
            Dialog::Text {
                title: title.into(),
                placeholder: "Your password",
                text: String::new(),
                masked: true,
                secret: true,
            },
            Ask::Auth(call),
        );
    }

    /// The boot files, the governor, the hostname and swap, read directly. The
    /// firmware's GPU memory needs vcgencmd, asked once: it holds until a
    /// restart.
    fn query_advanced(&mut self, cx: &mut Cx) {
        let boot = sysconf::boot_dir();
        let read = |p: &Path| std::fs::read_to_string(p).ok();
        let s = &mut self.state;
        s.gpu_mem =
            read(&boot.join(sysconf::CONFIG)).map_or(GpuMem::Unsupported, |t| sysconf::gpu_mem(&t));
        s.country = read(&boot.join(sysconf::CMDLINE)).and_then(|t| sysconf::regdom(&t));
        s.country_running =
            read(Path::new("/proc/cmdline")).map(|t| sysconf::regdom(&t).unwrap_or_default());
        if s.countries.is_empty() {
            s.countries =
                sysconf::countries(&read(Path::new(sysconf::COUNTRIES)).unwrap_or_default());
        }
        if let Some(cc) = &s.country {
            let tab = read(Path::new(sysconf::ZONE_TAB)).unwrap_or_default();
            s.country_zones = sysconf::zones_for(&tab, cc);
        }
        s.governor = read(Path::new(GOVERNOR))
            .unwrap_or_default()
            .trim()
            .to_string();
        s.usb_power = sysconf::usb_power(Path::new("/sys/bus/usb/devices"));
        s.hostname = read(Path::new("/proc/sys/kernel/hostname"))
            .unwrap_or_default()
            .trim()
            .to_string();
        s.swap_mb =
            read(Path::new(sysconf::swap_backend().dropin())).and_then(|t| sysconf::swap_conf(&t));
        // No zram0 is no swap.
        s.swap_running =
            Some(read(Path::new(sysconf::ZRAM_DISKSIZE)).map_or(0, |t| sysconf::swap_running(&t)));
        s.swap_comp =
            read(Path::new(sysconf::ZRAM_COMP_DROPIN)).and_then(|t| sysconf::comp_from_conf(&t));
        s.swap_comp_running =
            read(Path::new(sysconf::ZRAM_COMP)).and_then(|t| sysconf::comp_algorithms(&t).1);
        s.swappiness = read(Path::new(sysconf::SWAPPINESS_NOW)).and_then(|t| t.trim().parse().ok());
        if !self.gpu_asked {
            self.gpu_asked = true;
            self.run(cx, NetJob::GpuMem, &["vcgencmd", "get_mem", "gpu"]);
        }
    }

    /// After a change that waits for a restart, offer one, unless the
    /// change put things back as they run.
    fn offer_restart(&mut self, cx: &mut Cx) {
        self.query_advanced(cx);
        if self.state.restart_needed() && self.dialog.is_none() {
            self.ask(
                Dialog::Confirm {
                    title: "Restart to apply?".into(),
                    line: "Open apps close first.".into(),
                    yes: "restart",
                    no: "later",
                },
                Ask::Confirm(Command::Restart),
            );
        }
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

    fn job_done(&mut self, kind: NetJob, out: String, code: Option<i32>, cx: &mut Cx) {
        let ok = code == Some(0);
        let mut requery = false;
        match &kind {
            NetJob::Join {
                ssid,
                known,
                gave_password,
            } => {
                if ok {
                    cx.toast(&format!("Connected to {ssid}"));
                    if self.embed.is_some() && self.stack.len() == 1 {
                        self.finish(cx);
                    }
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
            NetJob::WifiUnblock => {
                if !ok {
                    cx.toast(&helper_failure(&out));
                }
                self.state.wifi_on = true;
                self.run(cx, NetJob::SetRadio, &["nmcli", "radio", "wifi", "on"]);
            }
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
                if ok && zone.is_some() && self.finish_pending {
                    self.finish(cx);
                }
                self.finish_pending &= ok;
                self.query_sys(cx);
            }
            NetJob::Helper(call) => {
                match auth_step(code, &out) {
                    Step::Done => {
                        // The row flips now, not after the re-query.
                        if call.args.first().map(String::as_str) == Some("ssh") {
                            self.state.ssh = call.args.get(1).map(String::as_str) == Some("on");
                        }
                        cx.toast(&call.done);
                        if call.restart {
                            self.offer_restart(cx);
                        }
                        if self.finish_pending {
                            self.finish(cx);
                        }
                    }
                    Step::Ask(title) => self.ask_password(cx, call.clone(), title),
                    Step::Locked(secs) => {
                        self.finish_pending = false;
                        cx.toast(&format!("Too many tries. Wait {secs} s"));
                    }
                    Step::Failed(why) => {
                        self.finish_pending = false;
                        cx.toast(&why);
                    }
                }
                self.query_sys(cx);
                self.query_advanced(cx);
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
            NetJob::GpuMem if ok => {
                st.gpu_running = out
                    .trim()
                    .strip_prefix("gpu=")
                    .and_then(|v| v.strip_suffix('M'))
                    .and_then(|v| v.parse().ok());
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
        if self.finish_when_idle && !self.changing() {
            self.finish(cx);
        }
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

    fn icon(&self) -> &str {
        model::icon(&self.top().0)
    }

    fn context(&self) -> Option<&str> {
        if let Some(e) = &self.embed {
            return Some(&e.label);
        }
        // One SDK page holds Settings' own stack: name the app above the
        // first page, as the SDK would. The country list says how to jump.
        match self.top().0 {
            PageId::Country(None) => Some("type a letter"),
            _ => (self.stack.len() > 1).then_some("Settings"),
        }
    }

    /// The page's own letter keys, so Help lists them.
    fn commands(&self, c: &mut Commands) {
        for (i, &(key, label)) in self.view.keys.iter().enumerate() {
            if let Some((spec, _)) = page_key_of(key) {
                let mut name = label.to_string();
                if let Some(f) = name.get_mut(..1) {
                    f.make_ascii_uppercase();
                }
                let cmd = gh0st_app::Command::action(PAGE_KEYS + i as u32, name);
                c.add(cmd.keys(spec).keys_only());
            }
        }
        // Setup's menu: no shell runs yet to power off from.
        if self.embed.is_some() {
            c.group("Setup");
            c.add(gh0st_app::Command::action(POWER_OFF, "Power off"));
        }
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        if id == POWER_OFF {
            self.run_command(Command::PowerOff, cx);
            self.flush(cx);
            return;
        }
        let key = id
            .checked_sub(PAGE_KEYS)
            .and_then(|i| self.view.keys.get(i as usize))
            .and_then(|&(k, _)| page_key_of(k));
        if let Some((_, key)) = key {
            self.page_or_list_key(key, cx);
            self.flush(cx);
        }
    }

    fn bars(&self) -> Bars {
        self.chips()
            .into_iter()
            .fold(Bars::top(), |b, (n, chip, _)| b.chip(n, chip))
    }

    fn start(&mut self, cx: &mut Cx) {
        self.refresh(cx);
        // A setup step asks only what its page shows. No volume: PipeWire
        // runs in the user's session, and setup comes before one.
        match self.embed.as_ref().map(|e| e.page.clone()) {
            None => {
                self.query_net(cx, false);
                self.query_bt(cx);
                self.query_sys(cx);
                self.query_volume(cx);
            }
            Some(PageId::Wifi) => {
                let argv = ["pkexec", HELPER, "rfkill", "wlan", "on"];
                self.run(cx, NetJob::WifiUnblock, &argv);
            }
            Some(PageId::Privacy) => {
                self.query_sys(cx);
                self.query_bt(cx);
            }
            Some(PageId::TimeZone(_)) => {
                self.run(cx, NetJob::Zones, &["timedatectl", "list-timezones"]);
                self.query_sys(cx);
            }
            Some(PageId::CountryZones) => {
                self.run(cx, NetJob::Zones, &["timedatectl", "list-timezones"]);
                self.query_sys(cx);
                self.query_advanced(cx);
            }
            Some(_) => self.query_advanced(cx),
        }
        self.refresh(cx);
        self.flush(cx);
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if id != ASK {
            return;
        }
        self.asked = false;
        let Some((_, ask)) = self.dialog.take() else {
            return;
        };
        if let Answer::Yes(text) = answer {
            self.answered(text, ask, cx);
        }
        self.flush(cx);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        // A chip runs what its key would.
        let key = match key {
            Key::Soft(n) => match self.chips().into_iter().find(|c| c.0 == n) {
                Some((_, _, ChipDo::Key(k))) => k,
                Some((_, _, ChipDo::Finish)) => return self.finish_soon(cx),
                None => return,
            },
            k => k,
        };
        self.page_or_list_key(key, cx);
        self.flush(cx);
    }

    /// Esc goes up a page. On the first page it leaves the app, like any
    /// other.
    fn back(&mut self, cx: &mut Cx) -> bool {
        if self.embed.is_some() && self.stack.len() == 1 && self.changing() {
            cx.toast("One moment");
            return false;
        }
        if self.state.moving.take().is_some() {
            self.refresh(cx);
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
                    self.job_done(kind, out, code, cx);
                }
            }
        }
        self.flush(cx);
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        model::draw(ui.canvas(), &t, &self.view, self.top().1);
    }
}

impl Settings {
    fn page_or_list_key(&mut self, key: Key, cx: &mut Cx) {
        if self.page_key(key, cx) {
            return;
        }
        match key {
            Key::Enter => self.activate(cx),
            Key::Up => self.move_cursor(-1, cx),
            Key::Down => self.move_cursor(1, cx),
            Key::Left => self.change(-1, cx),
            Key::Right => self.change(1, cx),
            // 4 is Esc and 5 to 8 are the chips and the menu: 1 to 3 pick.
            // Not in setup, where a stray digit would flip a privacy row.
            Key::Char(d @ '1'..='3') if self.embed.is_none() => {
                self.pick(d as usize - '1' as usize, cx)
            }
            // The shell acts on these keys; show the new values.
            Key::Other(codes::VOLUMEUP | codes::VOLUMEDOWN | codes::MUTE) => self.query_volume(cx),
            Key::Other(codes::BRIGHTNESSUP | codes::BRIGHTNESSDOWN) => self.refresh(cx),
            _ => {}
        }
    }

    /// The page's chips, then a setup step's Skip on 7 and Done on 6.
    fn chips(&self) -> Vec<(u8, Chip, ChipDo)> {
        let (id, cur) = self.top();
        let mut v: Vec<(u8, Chip, ChipDo)> =
            model::chips(id, &self.view, *cur, self.state.moving.is_some())
                .into_iter()
                .map(|(n, chip, k)| (n, chip, ChipDo::Key(k)))
                .collect();
        if let Some(e) = self.embed.as_ref().filter(|_| self.stack.len() == 1) {
            if e.skip {
                v.retain(|c| c.0 != 7);
                v.push((7, Chip::new("skip-forward", "Skip"), ChipDo::Finish));
            }
            if e.done {
                v.retain(|c| c.0 != 6);
                v.push((6, Chip::new("check", "Done").main(), ChipDo::Finish));
            }
        }
        v
    }
}

/// What a chip or a dialog answer is for: the one dialog Settings shows.
const ASK: u32 = 1;

/// Command ids for a page's own keys, by their place in its list.
const PAGE_KEYS: u32 = 100;

/// Setup's Power off, in its menu.
const POWER_OFF: u32 = 90;

/// NetworkManager's config files' text, in the order it reads them.
fn nm_files() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_to_string(sysconf::NM_MAIN_CONF)
        .into_iter()
        .collect();
    for dir in sysconf::NM_CONF_DIRS {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "conf"))
            .collect();
        files.sort();
        out.extend(files.iter().filter_map(|f| std::fs::read_to_string(f).ok()));
    }
    out
}

/// A page key as the SDK spells it, and the key it stands for.
fn page_key_of(key: &str) -> Option<(&'static str, Key)> {
    Some(match key {
        "R" => ("r", Key::Char('r')),
        "S" => ("s", Key::Char('s')),
        "M" => ("m", Key::Char('m')),
        "Del" => ("Bksp", Key::Backspace),
        _ => return None,
    })
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
    if let Some(b) = system::build() {
        out.push(("Build".into(), b.into()));
    }
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
    let used =
        |(used, total): (u64, u64)| format!("{} of {} used", size(used << 20), size(total << 20));
    let home = std::env::var_os("HOME").map_or("/home".into(), PathBuf::from);
    if let Some(space) = system::storage(&home) {
        out.push(("Storage".into(), used(space)));
    }
    // With /home on its own partition, the system's space is apart from it.
    let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| m.dev());
    if dev(Path::new("/")) != dev(&home) {
        if let Some(space) = system::storage(Path::new("/")) {
            out.push(("System".into(), used(space)));
        }
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
    fn helper_codes_become_steps() {
        assert_eq!(auth_step(Some(0), ""), Step::Done);
        assert_eq!(auth_step(Some(10), ""), Step::Ask("Password"));
        assert_eq!(auth_step(Some(11), ""), Step::Ask("Wrong password"));
        assert_eq!(
            auth_step(Some(12), "gh0st-helper: locked 40\n"),
            Step::Locked(40)
        );
        assert_eq!(
            auth_step(
                Some(1),
                "gh0st-helper: the kernel has no powersave governor\n"
            ),
            Step::Failed("the kernel has no powersave governor".into())
        );
        assert_eq!(
            auth_step(
                Some(126),
                "Error executing command as another user: Not authorized\n"
            ),
            Step::Failed("Not allowed: polkit rule missing".into())
        );
        assert_eq!(auth_step(None, ""), Step::Failed("Failed".into()));
    }

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
