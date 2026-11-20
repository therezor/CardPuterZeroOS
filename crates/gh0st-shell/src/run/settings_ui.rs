//! Settings as a screen of the shell: the page stack, its keys, and what
//! each setting writes. Every write loads config.json fresh, changes one key
//! and saves, so stock apps' keys survive, and applies at once.

use super::*;
use crate::settings::pages::{self, AppRow, DARK_TIMES, State as PageState};
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
                if let Dialog::Text { masked, .. } = dialog {
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
        let _ = dialog;
        match ask {
            Ask::Confirm(Command::Restart) => self.power(true),
            Ask::Confirm(Command::PowerOff) => self.power(false),
            Ask::Confirm(_) => {}
        }
    }

    /// Closes every app, then restarts or powers off.
    fn power(&mut self, restart: bool) {
        self.tasks.close_all(rt::now_ms());
        child::fire(&["systemctl", if restart { "reboot" } else { "poweroff" }]);
        self.toast(if restart { "Restarting\u{2026}" } else { "Powering off\u{2026}" });
    }

    /// Wi-Fi, Bluetooth and system settings, which go through helpers.
    fn apply_system(&mut self, s: &Setting, d: i32) {
        let _ = (s, d);
        self.toast("Not available yet");
    }

    fn run_system(&mut self, c: Command) {
        let _ = c;
        self.toast("Not available yet");
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
