//! Settings as a screen of the shell: the page stack, its keys, and what
//! each setting writes. Every write loads config.json fresh, changes one key
//! and saves, so stock apps' keys survive, and applies at once.

use super::*;
use crate::settings::{bt, net};
use crate::settings::pages::{self, AppRow, DARK_TIMES, Logs, State as PageState};
use crate::settings::{self, Act, Body, Command, Cursor, Dialog, Page, PageId, Setting, Value};
use gh0st_core::desktop;

pub(super) struct SettingsUi {
    pub stack: Vec<(PageId, Cursor)>,
    pub page: Page,
    pub state: PageState,
    pub dialog: Option<(Dialog, Ask)>,
}

/// What a dialog's answer is for.
pub(super) enum Ask {
    Confirm(Command),
    WifiPassword(String),
    HiddenName,
    HiddenPassword(String),
    PasswordCurrent,
    PasswordNew(String),
    PasswordRepeat(String, String),
    Forget(String),
}

/// pkexec runs the root helper by this path (see packaging/polkit).
const HELPER: &str = "/usr/libexec/gh0st-helper";
const LOGGING_DIR: &str = "/etc/systemd/journald.conf.d";

/// An nmcli run Settings is waiting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NetJob {
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
}

impl SettingsUi {
    fn top(&self) -> &(PageId, Cursor) {
        self.stack.last().expect("the root page is never popped")
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.stack.last_mut().expect("root").1
    }
}

impl Shell {
    pub(super) fn open_settings(&mut self) {
        let mut state = PageState::default();
        self.fill_local(&mut state);
        let page = pages::build(&PageId::Root, &state);
        self.settings = Some(SettingsUi {
            stack: vec![(PageId::Root, Cursor::default())],
            page,
            state,
            dialog: None,
        });
        self.screen = Screen::Settings;
        self.dirty = true;
        self.query_net(false);
        self.query_bt();
        self.query_sys();
    }

    fn close_settings(&mut self) {
        self.settings = None;
        self.screen = Screen::Home;
        self.dirty = true;
    }

    /// What the shell knows without asking a helper.
    fn fill_local(&self, s: &mut PageState) {
        s.brightness = self
            .backlight
            .as_ref()
            .and_then(|bl| bl.get().map(|r| bl.percent(r)));
        s.dark_secs = self
            .config
            .get_int("dark_time")
            .unwrap_or(DEFAULT_DARK_SECS);
        s.volume = self.volume;
        s.muted = self.muted.unwrap_or(false);
        s.layout = self.home.layout();
        s.palette = self.t.c.name;
        s.animations = self.config.get_bool("gh0st.animations").unwrap_or(true);
        s.clock_24h = clock_24h(&self.config);
        let mut apps: Vec<AppRow> =
            apps::scan_all(&self.dirs, &self.config, &self.pins, desktop::is_executable)
                .into_iter()
                .filter(|a| a.builtin.is_none())
                .map(|a| AppRow {
                    file: a.file,
                    name: a.entry.name,
                    shown: a.shown,
                })
                .collect();
        apps.sort_by_key(|a| a.name.to_lowercase());
        s.apps = apps;
        s.about = about(self.battery);
        s.logs = logs_level();
    }

    /// Rebuilds the page on top from fresh local state.
    pub(super) fn refresh_settings(&mut self) {
        let Some(mut ui) = self.settings.take() else {
            return;
        };
        self.fill_local(&mut ui.state);
        let (id, _) = ui.top().clone();
        ui.page = pages::build(&id, &ui.state);
        let per = self.page_rows(&ui.page);
        let len = ui.page.len();
        let info = matches!(ui.page.body, Body::Info(_));
        let cur = ui.cursor_mut();
        if !info {
            cur.fit(len, per);
        }
        self.settings = Some(ui);
        self.dirty = true;
    }

    fn page_rows(&self, page: &Page) -> usize {
        match page.body {
            Body::List(_) => settings::rows(&self.t).1,
            Body::Info(_) => settings::lines(&self.t).1,
        }
    }

    pub(super) fn settings_key(&mut self, k: Key, now: u64) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        if ui.dialog.is_some() {
            self.dialog_key(k);
            return;
        }
        let armed = |flag: &mut bool| match k.state {
            State::Press => {
                *flag = true;
                false
            }
            State::Release if *flag => {
                *flag = false;
                true
            }
            _ => false,
        };
        if k.is(code::ENTER) {
            if armed(&mut self.enter_armed) {
                self.activate();
            }
            return;
        }
        if k.is(code::ESC) {
            if armed(&mut self.esc_armed) {
                self.settings_back();
            }
            return;
        }
        if k.is(code::HOME) {
            if armed(&mut self.home_armed) {
                let at = self.tasks.last_used().map_or(0, |i| i + 1);
                self.open_switcher(Back::Settings, at);
            }
            return;
        }
        if k.state == State::Release {
            return;
        }
        if self.page_key(&k) {
            return;
        }
        match k.nav(true) {
            Some(keys::Nav::Up) => self.settings_move(-1),
            Some(keys::Nav::Down) => self.settings_move(1),
            Some(keys::Nav::Left) => self.settings_step(-1),
            Some(keys::Nav::Right) => self.settings_step(1),
            None => {
                if let (State::Press, Some(d @ '1'..='9')) = (k.state, k.ch) {
                    self.settings_pick(d as usize - '1' as usize);
                }
            }
        }
        let _ = now;
    }

    fn settings_move(&mut self, d: i32) {
        let Some(ui) = self.settings.as_ref() else {
            return;
        };
        let per = self.page_rows(&ui.page);
        let len = ui.page.len();
        let info = matches!(ui.page.body, Body::Info(_));
        if let Some(ui) = self.settings.as_mut() {
            ui.cursor_mut().step(d, len, per, info);
        }
        self.dirty = true;
    }

    /// Digit `n` picks the nth row on screen and acts on it.
    fn settings_pick(&mut self, n: usize) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        let target = ui.top().1.top + n;
        if ui.page.item(target).is_none() {
            return;
        }
        ui.cursor_mut().selected = target;
        self.activate();
    }

    fn selected_act(&self) -> Option<(Act, Value)> {
        let ui = self.settings.as_ref()?;
        let item = ui.page.item(ui.top().1.selected)?;
        Some((item.act.clone(), item.value.clone()))
    }

    /// Enter on a row.
    fn activate(&mut self) {
        let Some((act, value)) = self.selected_act() else {
            return;
        };
        match act {
            Act::Open(id) => {
                let zones = matches!(id, PageId::TimeZone(_))
                    && self.settings.as_ref().is_some_and(|ui| ui.state.zones.is_empty());
                if zones {
                    self.start_job(JobKind::Net(NetJob::Zones), &["timedatectl", "list-timezones"]);
                }
                if let Some(ui) = self.settings.as_mut() {
                    ui.stack.push((id, Cursor::default()));
                }
                self.refresh_settings();
            }
            Act::Set(s) => match value {
                Value::Toggle(_) => self.apply(&s, 0),
                _ => self.apply(&s, 1),
            },
            Act::Run(c) => self.run_command(c),
            Act::None => {}
        }
    }

    /// Z or C on a row: step a choice or a level, or flip a toggle.
    fn settings_step(&mut self, d: i32) {
        if let Some((Act::Set(s), value)) = self.selected_act() {
            let d = if matches!(value, Value::Toggle(_)) { 0 } else { d };
            self.apply(&s, d);
        }
    }

    fn settings_back(&mut self) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        if ui.stack.len() <= 1 {
            self.close_settings();
            return;
        }
        ui.stack.pop();
        self.refresh_settings();
    }

    /// Loads config.json, lets `f` change it, and saves.
    fn write_config(&mut self, f: impl FnOnce(&mut Config)) {
        self.config = Config::load(&config::default_path());
        f(&mut self.config);
        let _ = self.config.save();
    }

    /// Changes one setting. `d` is the step for a choice or level, 0 for a
    /// toggle.
    fn apply(&mut self, s: &Setting, d: i32) {
        match s {
            Setting::Brightness => {
                if let Some(bl) = &self.backlight {
                    let raw = bl.step(bl.get().unwrap_or(bl.max), d >= 0);
                    let _ = bl.set(raw);
                    self.write_config(|c| c.set_int("brightness", raw as i64));
                }
            }
            Setting::DarkTime => {
                let now = self.config.get_int("dark_time").unwrap_or(DEFAULT_DARK_SECS);
                let i = DARK_TIMES.iter().position(|&t| t == now).unwrap_or(1);
                let next = DARK_TIMES[cycle(i, d, DARK_TIMES.len())];
                self.write_config(|c| c.set_int("dark_time", next));
                self.dark_ms = dark_ms_of(&self.config);
            }
            Setting::Volume => {
                let v = (self.volume.unwrap_or(50) as i32 + d.signum() * VOLUME_STEP).clamp(0, 100);
                self.apply_volume(v as u8);
            }
            Setting::Mute => self.apply_mute(!self.muted.unwrap_or(false)),
            Setting::Layout => {
                let l = self.home.layout().toggled();
                self.write_config(|c| c.set_str("gh0st.home_layout", l.config_name()));
                self.home.set_layout(l);
            }
            Setting::Palette => {
                let i = theme::PALETTES
                    .iter()
                    .position(|p| p.name == self.t.c.name)
                    .unwrap_or(0);
                let p = theme::PALETTES[cycle(i, d, theme::PALETTES.len())];
                self.write_config(|c| c.set_str("gh0st.palette", p.name));
                self.t = Theme::new(p);
            }
            Setting::Animations => {
                let on = !self.config.get_bool("gh0st.animations").unwrap_or(true);
                self.write_config(|c| c.set_bool("gh0st.animations", on));
                self.set_animations();
            }
            Setting::Clock => {
                let h24 = !clock_24h(&self.config);
                self.write_config(|c| c.set_str("gh0st.clock", if h24 { "24h" } else { "12h" }));
                self.refresh_status();
            }
            Setting::AppShown(file) => {
                let key = apps::stock_key(file);
                let shown = self
                    .settings
                    .as_ref()
                    .and_then(|ui| ui.state.apps.iter().find(|a| a.file == *file))
                    .is_some_and(|a| a.shown);
                self.write_config(|c| c.set_int(&key, !shown as i64));
                self.rescan();
            }
            _ => self.apply_system(s, d),
        }
        self.refresh_settings();
    }

    fn run_command(&mut self, c: Command) {
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
            other => self.run_system(other),
        }
    }

    fn ask(&mut self, d: Dialog, a: Ask) {
        if let Some(ui) = self.settings.as_mut() {
            ui.dialog = Some((d, a));
        }
        self.dirty = true;
    }

    fn dialog_key(&mut self, k: Key) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        let Some((dialog, _)) = ui.dialog.as_mut() else {
            return;
        };
        let mut done = None;
        match k.code {
            code::ENTER => match k.state {
                State::Press => self.enter_armed = true,
                State::Release if self.enter_armed => {
                    self.enter_armed = false;
                    done = Some(true);
                }
                _ => {}
            },
            code::ESC => match k.state {
                State::Press => self.esc_armed = true,
                State::Release if self.esc_armed => {
                    self.esc_armed = false;
                    done = Some(false);
                }
                _ => {}
            },
            _ if k.state == State::Release => {}
            code::TAB => {
                if let Dialog::Text {
                    masked,
                    secret: true,
                    ..
                } = dialog
                {
                    *masked = !*masked;
                }
            }
            code::BACKSPACE => {
                if let Dialog::Text { text, .. } = dialog {
                    text.pop();
                }
            }
            _ => {
                if let (Dialog::Text { text, .. }, Some(c)) = (dialog, k.ch) {
                    if text.chars().count() < 64 {
                        text.push(c);
                    }
                }
            }
        }
        self.dirty = true;
        let Some(ok) = done else {
            return;
        };
        let Some((dialog, ask)) = self.settings.as_mut().and_then(|ui| ui.dialog.take()) else {
            return;
        };
        if ok {
            self.answer(dialog, ask);
        }
    }

    fn answer(&mut self, dialog: Dialog, ask: Ask) {
        let mut text = match dialog {
            Dialog::Text { text, .. } => text,
            Dialog::Confirm { .. } => String::new(),
        };
        match ask {
            Ask::Confirm(Command::Restart) => self.power(true),
            Ask::Confirm(Command::PowerOff) => self.power(false),
            Ask::Confirm(_) => {}
            Ask::WifiPassword(ssid) => {
                let argv = ["nmcli", "--wait", "45", "--ask", "device", "wifi", "connect", &ssid];
                self.connect(&ssid, &argv, false, Some(&mut text));
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
                    self.toast("The passwords differ");
                } else if new.is_empty() {
                    self.toast("The password is empty");
                } else {
                    // passwd as the user: it checks the current password
                    // itself, so no root helper can be asked to set one.
                    let mut input = format!("{cur}\n{new}\n{new}\n");
                    self.start_job_with(
                        JobKind::Net(NetJob::Password),
                        &["passwd"],
                        Some(input.as_bytes()),
                    );
                    wipe(&mut input);
                }
                wipe(&mut cur);
                wipe(&mut new);
            }
            Ask::Forget(ssid) => self.forget(&ssid),
            Ask::HiddenPassword(ssid) => {
                let argv = [
                    "nmcli", "--wait", "45", "--ask", "device", "wifi", "connect", &ssid, "hidden",
                    "yes",
                ];
                let pw = (!text.is_empty()).then_some(&mut text);
                self.connect(&ssid, &argv, false, pw);
            }
        }
        wipe(&mut text);
    }

    /// Closes every app, then restarts or powers off.
    fn power(&mut self, restart: bool) {
        self.tasks.close_all(rt::now_ms());
        let (arg, msg) = if restart {
            ("reboot", "Restarting\u{2026}")
        } else {
            ("off", "Powering off\u{2026}")
        };
        self.helper(&["power", arg], msg.into());
        self.toast(msg);
    }

    /// Keys only some pages take. True if the key was used.
    fn page_key(&mut self, k: &Key) -> bool {
        if k.state != State::Press {
            return false;
        }
        let Some(ui) = self.settings.as_ref() else {
            return false;
        };
        let id = ui.top().0.clone();
        match id {
            PageId::Wifi if k.ch == Some('r') => {
                self.query_net(true);
                self.toast("Scanning\u{2026}");
                true
            }
            PageId::Wifi if k.is(code::BACKSPACE) => {
                if let Some((Act::Run(Command::Join(ssid)), _)) = self.selected_act() {
                    let (known, active) = self.settings.as_ref().map_or((false, false), |ui| {
                        let active = ui.state.networks.iter().any(|n| n.active && n.ssid == ssid);
                        (ui.state.profiles.contains(&ssid), active)
                    });
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
                        self.forget(&ssid);
                    } else {
                        self.toast(format!("{ssid} is not saved"));
                    }
                }
                true
            }
            PageId::Bluetooth if k.ch == Some('s') => {
                self.run_system(Command::BtScan);
                true
            }
            PageId::Bluetooth if k.is(code::BACKSPACE) => {
                if let Some((Act::Run(Command::Device(mac)), _)) = self.selected_act() {
                    let paired = self.bt_device(&mac).is_some_and(|d| d.paired);
                    if paired {
                        self.start_job(
                            JobKind::Net(NetJob::BtRemove(mac.clone())),
                            &["bluetoothctl", "remove", &mac],
                        );
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn query_sys(&mut self) {
        self.start_job(
            JobKind::Net(NetJob::SysQuery),
            &[
                "sh",
                "-c",
                "timedatectl show -p Timezone -p NTP; echo SSH=$(systemctl is-active ssh)",
            ],
        );
    }

    /// Runs the root helper through pkexec.
    fn helper(&mut self, args: &[&str], done: String) {
        let mut argv = vec!["pkexec", HELPER];
        argv.extend_from_slice(args);
        self.start_job(JobKind::Net(NetJob::Helper(done)), &argv);
    }

    fn forget(&mut self, ssid: &str) {
        self.start_job(
            JobKind::Net(NetJob::Forget(ssid.to_string())),
            &["nmcli", "connection", "delete", "id", ssid],
        );
    }

    fn query_bt(&mut self) {
        self.start_job(JobKind::Net(NetJob::BtQuery), &["sh", "-c", bt::QUERY]);
    }

    fn bt_device(&self, mac: &str) -> Option<pages::Device> {
        self.settings
            .as_ref()?
            .state
            .devices
            .iter()
            .find(|d| d.mac == mac)
            .cloned()
    }

    /// Enter on a device: connect a paired one, pair, trust and connect a
    /// new one, disconnect a connected one.
    fn bt_enter(&mut self, mac: String) {
        let Some(d) = self.bt_device(&mac) else {
            return;
        };
        if !bt::is_mac(&mac) {
            return;
        }
        let name = if d.name.is_empty() { mac.clone() } else { d.name.clone() };
        let done = if d.connected {
            format!("Disconnected {name}")
        } else if d.paired {
            format!("Connected to {name}")
        } else {
            format!("Paired with {name}")
        };
        let job = JobKind::Net(NetJob::BtDevice(done));
        if d.connected {
            self.start_job(job, &["bluetoothctl", "disconnect", &mac]);
            self.toast(format!("Disconnecting {name}"));
        } else if d.paired {
            self.start_job(job, &["bluetoothctl", "--timeout", "20", "connect", &mac]);
            self.toast(format!("Connecting {name}\u{2026}"));
        } else {
            let script = "bluetoothctl --timeout 20 pair \"$1\" && bluetoothctl trust \"$1\" && bluetoothctl --timeout 20 connect \"$1\"";
            self.start_job(job, &["sh", "-c", script, "sh", &mac]);
            self.toast(format!("Pairing {name}\u{2026}"));
        }
    }

    /// Asks nmcli for the radio, the networks, the saved profiles and the
    /// devices. `rescan` makes the radio look again, which takes seconds.
    fn query_net(&mut self, rescan: bool) {
        let net = |j| JobKind::Net(j);
        self.start_job(net(NetJob::Radio), &["nmcli", "-t", "-f", "WIFI", "radio"]);
        self.start_job(
            net(NetJob::Profiles),
            &["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show"],
        );
        let scan = if rescan { "yes" } else { "no" };
        self.start_job(
            net(NetJob::List),
            &[
                "nmcli", "-t", "-f", "IN-USE,SIGNAL,SECURITY,SSID", "device", "wifi", "list",
                "--rescan", scan,
            ],
        );
        self.start_job(
            net(NetJob::Devices),
            &[
                "nmcli",
                "-t",
                "-f",
                "GENERAL.DEVICE,GENERAL.TYPE,GENERAL.STATE,GENERAL.CONNECTION,GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
                "device",
                "show",
            ],
        );
    }

    pub(super) fn net_job_done(&mut self, kind: NetJob, j: Job) {
        let out = j.output();
        let mut requery = false;
        match &kind {
            NetJob::Join {
                ssid,
                known,
                gave_password,
            } => {
                if j.ok() {
                    self.toast(format!("Connected to {ssid}"));
                } else {
                    // A failed first join leaves a profile behind, wrong
                    // password and all; the next try would not ask again.
                    if !known {
                        child::fire(&["nmcli", "connection", "delete", "id", ssid]);
                    }
                    self.toast(net::failure(&out, *gave_password));
                }
                requery = true;
            }
            NetJob::Forget(ssid) => {
                self.toast(if j.ok() {
                    format!("Forgot {ssid}")
                } else {
                    net::failure(&out, false)
                });
                requery = true;
            }
            NetJob::SetRadio => {
                if !j.ok() {
                    self.toast(net::failure(&out, false));
                }
                requery = true;
            }
            NetJob::BtPower | NetJob::BtRemove(_) => {
                if !j.ok() {
                    self.toast(bt::failure(&out));
                }
                self.query_bt();
            }
            NetJob::BtScan => self.query_bt(),
            NetJob::BtUnblock => {
                if j.ok() {
                    self.start_job(JobKind::Net(NetJob::BtPower), &["bluetoothctl", "power", "on"]);
                } else {
                    self.toast(helper_failure(&out));
                    self.query_bt();
                }
            }
            NetJob::TimeSet(zone) => {
                match (j.ok(), zone) {
                    // The C library reads /etc/localtime once; naming the
                    // zone makes the clock follow at once.
                    // SAFETY: the shell is single-threaded.
                    (true, Some(z)) => unsafe { std::env::set_var("TZ", z) },
                    (true, None) => {}
                    (false, _) => self.toast(helper_failure(&out)),
                }
                self.refresh_status();
                self.query_sys();
            }
            NetJob::Helper(done) => {
                self.toast(if j.ok() { done.clone() } else { helper_failure(&out) });
                self.query_sys();
            }
            NetJob::Password => self.toast(if j.ok() {
                "Password changed".to_string()
            } else {
                password_failure(&out)
            }),
            NetJob::BtDevice(done) => {
                if j.ok() {
                    self.toast(done.clone());
                } else {
                    self.toast(bt::failure(&out));
                }
                self.query_bt();
            }
            _ => {}
        }
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        let st = &mut ui.state;
        match kind {
            NetJob::Radio => {
                if let Some(on) = net::parse_radio(&out) {
                    st.wifi_on = on;
                }
            }
            NetJob::Profiles if j.ok() => {
                st.profiles = net::parse_profiles(&out);
                for n in &mut st.networks {
                    n.known = st.profiles.contains(&n.ssid);
                }
            }
            NetJob::List if j.ok() => st.networks = net::parse_list(&out, &st.profiles),
            NetJob::Devices if j.ok() => st.ifaces = net::parse_devices(&out),
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
            NetJob::Zones if j.ok() => {
                st.zones = out.lines().map(str::to_string).filter(|z| z.contains('/')).collect();
            }
            _ => {}
        }
        if requery {
            self.query_net(false);
        }
        self.refresh_settings();
    }

    /// Wi-Fi, Bluetooth and system settings, which go through helpers.
    fn apply_system(&mut self, s: &Setting, d: i32) {
        let _ = d;
        match s {
            Setting::WifiRadio => {
                let on = self.settings.as_ref().is_some_and(|ui| ui.state.wifi_on);
                if let Some(ui) = self.settings.as_mut() {
                    ui.state.wifi_on = !on;
                }
                self.start_job(
                    JobKind::Net(NetJob::SetRadio),
                    &["nmcli", "radio", "wifi", if on { "off" } else { "on" }],
                );
            }
            Setting::Bluetooth => {
                let on = self.settings.as_ref().is_some_and(|ui| ui.state.bt_on);
                if let Some(ui) = self.settings.as_mut() {
                    ui.state.bt_on = !on;
                }
                // Stock boots with the radio blocked. The session ACL
                // usually allows the unblock; the helper covers the rest.
                let blocked = !on
                    && gh0st_core::rfkill::set(gh0st_core::rfkill::Radio::Bluetooth, false).is_err();
                if blocked {
                    let argv = ["pkexec", HELPER, "rfkill", "bluetooth", "on"];
                    self.start_job(JobKind::Net(NetJob::BtUnblock), &argv);
                } else {
                    self.start_job(
                        JobKind::Net(NetJob::BtPower),
                        &["bluetoothctl", "power", if on { "off" } else { "on" }],
                    );
                }
            }
            Setting::NetworkTime => {
                let on = self.settings.as_ref().is_some_and(|ui| ui.state.network_time);
                let argv = ["timedatectl", "set-ntp", if on { "false" } else { "true" }];
                self.start_job(JobKind::Net(NetJob::TimeSet(None)), &argv);
            }
            Setting::Ssh => {
                let on = self.settings.as_ref().is_some_and(|ui| ui.state.ssh);
                let (arg, done) = if on { ("off", "SSH off") } else { ("on", "SSH on") };
                self.helper(&["ssh", arg], done.into());
            }
            Setting::Logs => {
                let now = logs_level();
                let i = Logs::ALL.iter().position(|&l| l == now).unwrap_or(0);
                let next = Logs::ALL[cycle(i, d, Logs::ALL.len())];
                self.helper(&["logs", next.verb()], format!("Logs: {}", next.label()));
            }
            _ => self.toast("Not available yet"),
        }
    }

    fn run_system(&mut self, c: Command) {
        match c {
            Command::Join(ssid) => self.join(ssid),
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
                let busy = self.settings.as_ref().is_some_and(|ui| ui.state.bt_scanning);
                if !busy {
                    if let Some(ui) = self.settings.as_mut() {
                        ui.state.bt_scanning = true;
                    }
                    self.start_job(
                        JobKind::Net(NetJob::BtScan),
                        &["bluetoothctl", "--timeout", "10", "scan", "on"],
                    );
                    self.refresh_settings();
                }
            }
            Command::Device(mac) => self.bt_enter(mac),
            Command::SetZone(zone) => {
                let known = self.settings.as_ref().is_some_and(|ui| ui.state.zones.contains(&zone));
                if known {
                    self.start_job(
                        JobKind::Net(NetJob::TimeSet(Some(zone.clone()))),
                        &["timedatectl", "set-timezone", &zone],
                    );
                    if let Some(ui) = self.settings.as_mut() {
                        ui.state.zone = zone;
                        ui.stack.retain(|(p, _)| !matches!(p, PageId::TimeZone(_)));
                    }
                    self.refresh_settings();
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
            _ => self.toast("Not available yet"),
        }
    }

    /// Enter on a network: a saved one comes up, an open one joins, a
    /// secured one asks for its password first.
    fn join(&mut self, ssid: String) {
        let Some(n) = self
            .settings
            .as_ref()
            .and_then(|ui| ui.state.networks.iter().find(|n| n.ssid == ssid))
            .cloned()
        else {
            return;
        };
        if n.active {
            self.toast(format!("Connected to {ssid}"));
        } else if n.known {
            self.connect(&ssid, &["nmcli", "--wait", "30", "connection", "up", "id", &ssid], true, None);
        } else if n.enterprise {
            self.toast("Needs a user name: join it from the terminal");
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
            self.connect(
                &ssid,
                &["nmcli", "--wait", "45", "device", "wifi", "connect", &ssid],
                false,
                None,
            );
        }
    }

    /// Runs a join. The password, if any, goes on stdin (`--ask`), never in
    /// argv, where any user could read it in /proc.
    fn connect(&mut self, ssid: &str, argv: &[&str], known: bool, password: Option<&mut String>) {
        let gave_password = password.is_some();
        let mut line = password.map(|p| {
            let l = format!("{p}\n");
            wipe(p);
            l
        });
        self.start_job_with(
            JobKind::Net(NetJob::Join {
                ssid: ssid.to_string(),
                known,
                gave_password,
            }),
            argv,
            line.as_deref().map(str::as_bytes),
        );
        if let Some(l) = line.as_mut() {
            wipe(l);
        }
        if let Some(ui) = self.settings.as_mut() {
            ui.state.wifi_busy = Some(ssid.to_string());
        }
        self.refresh_settings();
    }
}

pub(super) fn draw(c: &mut Canvas, t: &Theme, status: &Status, ui: Option<&SettingsUi>) {
    let Some(ui) = ui else {
        return;
    };
    let status = Status {
        title: &ui.page.title,
        ..*status
    };
    let (_, cur) = ui.top();
    let dialog = ui.dialog.as_ref().map(|(d, _)| d);
    settings::draw(c, t, &status, &ui.page, *cur, dialog);
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
        _ if std::path::Path::new("/var/log/journal").is_dir() => Logs::Persistent,
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

/// Overwrites a secret before its memory is freed.
fn wipe(s: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { s.as_bytes_mut().fill(0) };
    s.clear();
}

fn cycle(i: usize, d: i32, n: usize) -> usize {
    (i as i64 + if d < 0 { -1 } else { 1 }).rem_euclid(n as i64) as usize
}

pub(super) fn clock_24h(c: &Config) -> bool {
    c.get_str("gh0st.clock") != Some("12h")
}

/// Readouts for About, from procfs and the C library.
fn about(battery: Option<Reading>) -> Vec<(String, String)> {
    let mut out = vec![(
        "Version".to_string(),
        format!("Gh0stdeck {}", env!("CARGO_PKG_VERSION")),
    )];
    let read = |p: &str| std::fs::read_to_string(p).ok();
    if let Some(os) = read("/etc/os-release").and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    }) {
        out.push(("OS".into(), os));
    }
    if let Some(k) = read("/proc/sys/kernel/osrelease") {
        out.push(("Kernel".into(), k.trim().into()));
    }
    if let Some(m) = read("/proc/meminfo") {
        let kb = |key: &str| {
            m.lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        };
        if let (Some(total), Some(avail)) = (kb("MemTotal:"), kb("MemAvailable:")) {
            out.push((
                "Memory".into(),
                format!("{} MB of {} MB used", (total - avail) / 1024, total / 1024),
            ));
        }
    }
    if let Some((used, total)) = disk("/") {
        out.push((
            "Storage".into(),
            format!("{} of {} used", size(used), size(total)),
        ));
    }
    if let Some(b) = battery {
        let charging = if b.charging { ", charging" } else { "" };
        out.push(("Battery".into(), format!("{}%{charging}", b.percent)));
    }
    if let Some(up) = read("/proc/uptime")
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
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

/// Used and total bytes of the filesystem holding `path`.
fn disk(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs fills the zeroed struct for a NUL-terminated path.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let f = st.f_frsize as u64;
    let total = st.f_blocks as u64 * f;
    let free = st.f_bfree as u64 * f;
    Some((total - free, total))
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
}
