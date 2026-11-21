//! Settings: an ordinary gh0st app, built on the SDK alone. The pages that
//! need root (Wi-Fi, Privacy, Time, Updates) wait for the root helper.

use gh0st_app::gfx::theme::PALETTES;
use gh0st_app::prelude::*;
use gh0st_app::{codes, system};

fn main() {
    gh0st_app::run(Root::default());
}

const CHANGE: [(&str, &str); 2] = [("Z C", "change"), ("Esc", "back")];

#[derive(Default)]
struct Root {
    list: List,
    brightness: String,
    palette: String,
}

impl Root {
    fn refresh(&mut self, cx: &mut Cx) {
        self.brightness = system::brightness()
            .map(|p| format!("{p}%"))
            .unwrap_or_default();
        self.palette = cx.theme().c.name.to_string();
    }
}

impl Page for Root {
    fn title(&self) -> &str {
        "Settings"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints([("Enter", "open"), ("Esc", "back")])
    }

    fn start(&mut self, cx: &mut Cx) {
        self.refresh(cx);
    }

    fn resume(&mut self, cx: &mut Cx) {
        self.refresh(cx);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.list.key(key) {
            Some(Action::Open(0)) => cx.push(Appearance::default()),
            Some(Action::Open(1)) => cx.push(Display::default()),
            Some(Action::Open(2)) => cx.push(Sound::default()),
            Some(Action::Open(3)) => cx.push(About::default()),
            Some(Action::Open(_)) => cx.toast("Needs the system helper. Coming soon."),
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.list(
            &self.list,
            &[
                Row::link("Appearance").value(&self.palette).icon("palette"),
                Row::link("Display").value(&self.brightness).icon("sun"),
                Row::link("Sound").icon("volume-2"),
                Row::link("About").icon("info"),
                Row::info("Wi-Fi", "soon").icon("wifi").dim(),
                Row::info("Network", "soon").icon("network").dim(),
                Row::info("Bluetooth", "soon").icon("bluetooth").dim(),
                Row::info("Privacy", "soon").icon("shield").dim(),
                Row::info("Time", "soon").icon("clock").dim(),
            ],
        );
    }
}

#[derive(Default)]
struct Appearance {
    list: List,
    list_layout: bool,
    animations: bool,
}

impl Page for Appearance {
    fn title(&self) -> &str {
        "Appearance"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(CHANGE)
    }

    fn start(&mut self, cx: &mut Cx) {
        let s = cx.settings();
        self.list_layout = s.get_str("gh0st.home_layout") == Some("list");
        self.animations = s.get_bool("gh0st.animations").unwrap_or(true);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        let s = cx.settings();
        match self.list.key(key) {
            Some(Action::Change(0, step)) => {
                let now = s.get_str("gh0st.palette").unwrap_or("mint");
                let i = PALETTES
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(now))
                    .unwrap_or(0);
                let next = step_index(i, step, PALETTES.len());
                if next == i {
                    return;
                }
                // The SDK and the shell both follow this key: the new
                // palette shows at once.
                s.set_str("gh0st.palette", &PALETTES[next].name.to_lowercase());
            }
            Some(Action::Change(1, _) | Action::Open(1)) => {
                self.list_layout = !self.list_layout;
                let v = if self.list_layout { "list" } else { "grid" };
                s.set_str("gh0st.home_layout", v);
            }
            Some(Action::Change(2, _) | Action::Open(2)) => {
                self.animations = !self.animations;
                s.set_bool("gh0st.animations", self.animations);
            }
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let palette = ui.theme().c.name;
        let layout = if self.list_layout { "List" } else { "Grid" };
        ui.list(
            &self.list,
            &[
                Row::choice("Palette", palette).icon("palette"),
                Row::choice("Home layout", layout).icon("layout-grid"),
                Row::toggle("Animations", self.animations).icon("arrow-left-right"),
            ],
        );
    }
}

const TIMEOUTS: [(i64, &str); 6] = [
    (15, "15 s"),
    (30, "30 s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (0, "Never"),
];

#[derive(Default)]
struct Display {
    list: List,
    brightness: Option<u8>,
    timeout: usize,
}

impl Page for Display {
    fn title(&self) -> &str {
        "Display"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(CHANGE)
    }

    fn start(&mut self, cx: &mut Cx) {
        self.brightness = system::brightness();
        let secs = cx.settings().get_int("dark_time").unwrap_or(30);
        self.timeout = TIMEOUTS.iter().position(|t| t.0 == secs).unwrap_or(1);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.list.key(key) {
            Some(Action::Change(0, step)) => {
                let now = self.brightness.unwrap_or(80) as i32;
                let next = ((now / 10 + step) * 10).clamp(10, 100) as u8;
                if Some(next) == self.brightness {
                    return;
                }
                match system::set_brightness(next) {
                    Ok(raw) => {
                        self.brightness = Some(next);
                        cx.settings().set_int("brightness", raw as i64);
                    }
                    Err(e) => cx.toast(&format!("Brightness: {e}")),
                }
            }
            Some(Action::Change(1, step)) => {
                let next = step_index(self.timeout, step, TIMEOUTS.len());
                if next == self.timeout {
                    return;
                }
                self.timeout = next;
                cx.settings().set_int("dark_time", TIMEOUTS[self.timeout].0);
            }
            _ => {}
        }
        if let Key::Other(codes::BRIGHTNESSUP | codes::BRIGHTNESSDOWN) = key {
            self.brightness = system::brightness();
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let level = self
            .brightness
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "n/a".into());
        ui.list(
            &self.list,
            &[
                Row::choice("Brightness", &level).icon("sun"),
                Row::choice("Screen off after", TIMEOUTS[self.timeout].1).icon("timer"),
            ],
        );
        ui.note("The screen-off time applies on the home screen.");
    }
}

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

#[derive(Default)]
struct Sound {
    list: List,
    /// Volume in percent and mute, once `wpctl` has answered.
    volume: Option<(u8, bool)>,
    failed: bool,
}

impl Sound {
    fn read(&self, cx: &mut Cx) {
        cx.spawn(&["wpctl", "get-volume", SINK]);
    }
}

impl Page for Sound {
    fn title(&self) -> &str {
        "Sound"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints([("Z C", "change"), ("Enter", "toggle"), ("Esc", "back")])
    }

    fn start(&mut self, cx: &mut Cx) {
        self.read(cx);
    }

    fn job(&mut self, _: Job, event: JobEvent, _: &mut Cx) {
        match event {
            // "Volume: 0.60" or "Volume: 0.60 [MUTED]"
            JobEvent::Line(line) => {
                if let Some(v) = line
                    .strip_prefix("Volume: ")
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|n| n.parse::<f32>().ok())
                {
                    let pct = (v * 100.0).round().clamp(0.0, 150.0) as u8;
                    self.volume = Some((pct, line.contains("[MUTED]")));
                }
            }
            JobEvent::Done(code) => self.failed = code != Some(0) && self.volume.is_none(),
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        let Some((pct, muted)) = self.volume else {
            return;
        };
        match self.list.key(key) {
            Some(Action::Change(0, step)) => {
                let next = (pct as i32 + 5 * step).clamp(0, 100) as u8;
                if next == pct {
                    return;
                }
                let level = format!("{:.2}", next as f32 / 100.0);
                cx.spawn(&["wpctl", "set-volume", SINK, &level]);
                cx.settings().set_int("volume", next as i64);
                self.volume = Some((next, muted));
            }
            Some(Action::Change(1, _) | Action::Open(1)) => {
                cx.spawn(&["wpctl", "set-mute", SINK, if muted { "0" } else { "1" }]);
                self.volume = Some((pct, !muted));
            }
            _ => {}
        }
        if let Key::Other(codes::VOLUMEUP | codes::VOLUMEDOWN | codes::MUTE) = key {
            self.read(cx);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        match self.volume {
            Some((pct, muted)) => {
                let level = format!("{pct}%");
                ui.list(
                    &self.list,
                    &[
                        Row::choice("Volume", &level).icon("volume-2"),
                        Row::toggle("Mute", muted).icon("volume-x"),
                    ],
                );
            }
            None if self.failed => ui.message("volume-x", "No audio output found"),
            None => ui.message("volume-2", "Reading the volume"),
        }
    }
}

#[derive(Default)]
struct About {
    lines: Vec<(&'static str, String)>,
    battery: Vec<(&'static str, String)>,
}

impl Page for About {
    fn title(&self) -> &str {
        "About"
    }

    fn start(&mut self, _: &mut Cx) {
        let mb = |(used, total): (u64, u64)| format!("{used} of {total} MB used");
        let gb = |(used, total): (u64, u64)| {
            format!(
                "{:.1} of {:.1} GB used",
                used as f64 / 1024.0,
                total as f64 / 1024.0
            )
        };
        self.lines = vec![
            ("System", format!("CPZeroOS, gh0st {}", system::version())),
            ("Kernel", system::kernel().unwrap_or_else(|| "n/a".into())),
            (
                "Memory",
                system::memory().map(mb).unwrap_or_else(|| "n/a".into()),
            ),
            (
                "Storage",
                system::storage().map(gb).unwrap_or_else(|| "n/a".into()),
            ),
        ];
        if let Some(b) = system::battery() {
            let charge = match b.charge_mah {
                Some(mah) => format!("{}%, {mah} mAh", b.percent),
                None => format!("{}%", b.percent),
            };
            self.battery.push(("Charge", charge));
            if let (Some(h), Some(d)) = (b.health(), b.design_mah) {
                self.battery.push(("Health", format!("{h}% of {d} mAh")));
            }
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.section("CardputerZero");
        for (k, v) in &self.lines {
            ui.readout(k, v);
        }
        if !self.battery.is_empty() {
            ui.section("Battery");
            for (k, v) in &self.battery {
                ui.readout(k, v);
            }
        }
    }
}

/// `i` moved by `step`, held inside `0..len`.
fn step_index(i: usize, step: i32, len: usize) -> usize {
    (i as i32 + step).clamp(0, len as i32 - 1) as usize
}
