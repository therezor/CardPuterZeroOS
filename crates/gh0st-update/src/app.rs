//! The Updates app's first page: what the engine found, how far it is,
//! and the one thing Enter does now (check, install, restart, try again).
//! It watches the engine's files with inotify and draws only when they
//! change; the one timer moves a progress segment while it is on screen.

use crate::packages::Packages;
use crate::words;
use gh0st_app::gfx::Rect;
use gh0st_app::gfx::widgets::{self, Hint, Row as WRow, Trailing};
use gh0st_app::helper::{Finished, Helper};
use gh0st_app::prelude::*;
use gh0st_app::system;
use gh0st_core::update::{self, Group, KERNEL_BATTERY, Outcome, Phase, Progress, Restart, State};

const APP_ICON: &str = "refresh-cw";
/// Dialog ids.
const ASK_PASSWORD: u32 = 900;
const ASK_INSTALL: u32 = 1;
/// Helper calls.
const CHECK: u32 = 1;
const INSTALL: u32 = 2;
const STOP: u32 = 3;
const CLEAN: u32 = 4;
const REBOOT: u32 = 5;
/// Menu commands.
const CMD_INSTALL: u32 = 10;
const CMD_CHECK: u32 = 11;
const CMD_LIST: u32 = 12;
const CMD_CLEAN: u32 = 13;
/// A started unit has this long to show progress before the page stops
/// saying "Starting".
const START_MS: u64 = 15_000;
/// A frame of the moving segment.
const SEGMENT_MS: u64 = 60;
const ARCHIVES: &str = "/var/cache/apt/archives";

/// What the page shows.
#[derive(Clone, Debug, PartialEq)]
enum View {
    Never,
    Current,
    Ready,
    /// The kernel set is in the list and the battery is at 50% or less.
    KernelBattery(u8),
    Busy(Progress),
    Done(Restart),
    Problem(Outcome),
}

pub struct Updates {
    state: State,
    progress: Option<Progress>,
    /// A check or install was asked for and the engine hasn't shown up yet.
    starting: Option<bool>,
    /// The state file's mtime when it was asked: a run that ends before
    /// writing progress (offline, the battery) still rewrites the state.
    starting_from: Option<std::time::SystemTime>,
    /// The last install's restart is still owed. Read in `reload`: it walks
    /// /proc.
    restart_owed: bool,
    start_timer: Option<Timer>,
    helper: Helper,
    battery: Option<system::Battery>,
    /// Bytes of downloaded debs apt keeps.
    cache: u64,
    phase: u8,
    segment: Option<Timer>,
    watch: Option<(gh0st_core::status::Watch, Watch)>,
    /// `systemctl --user` steps of a Gh0st restart.
    shell_jobs: Vec<Job>,
    h24: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Updates::new()
    }
}

impl Updates {
    pub fn new() -> Updates {
        Updates {
            state: State::default(),
            progress: None,
            starting: None,
            starting_from: None,
            restart_owed: false,
            start_timer: None,
            helper: Helper::new(ASK_PASSWORD),
            battery: None,
            cache: 0,
            phase: 0,
            segment: None,
            watch: None,
            shell_jobs: Vec::new(),
            h24: true,
        }
    }

    fn reload(&mut self) {
        self.state = State::load();
        self.progress = Progress::load();
        let rewritten = state_mtime() != self.starting_from;
        if self.progress.is_some() || (rewritten && !update::engine_running()) {
            self.starting = None;
        }
        self.restart_owed = !update::restart_done(&self.state);
        self.battery = system::battery();
        self.cache = cache_bytes();
    }

    fn view(&self) -> View {
        let st = &self.state;
        if let Some(p) = &self.progress {
            return View::Busy(p.clone());
        }
        if let Some(install) = self.starting {
            return View::Busy(Progress {
                install,
                phase: if install {
                    Phase::Download
                } else {
                    Phase::Lists
                },
                percent: None,
                ..Progress::default()
            });
        }
        if st.restart != Restart::None && self.restart_owed {
            return View::Done(st.restart);
        }
        let ready = !st.packages.is_empty();
        match st.outcome {
            Outcome::None if st.checked == 0 => View::Never,
            Outcome::Ok | Outcome::None if ready => match self.kernel_short() {
                Some(pct) => View::KernelBattery(pct),
                None => View::Ready,
            },
            Outcome::Ok | Outcome::None => View::Current,
            Outcome::BatteryKernel => View::KernelBattery(self.battery.map_or(0, |b| b.percent)),
            o => View::Problem(o),
        }
    }

    /// The battery's percent when the kernel set is in the list and the
    /// battery is too low for it.
    fn kernel_short(&self) -> Option<u8> {
        let b = self.battery?;
        (self.state.kernel() && b.percent <= KERNEL_BATTERY).then_some(b.percent)
    }

    fn busy(&self) -> bool {
        matches!(self.view(), View::Busy(_))
    }

    /// Stop works for a check, and for an install until dpkg starts.
    fn stoppable(&self) -> bool {
        match self.view() {
            View::Busy(p) => !p.install || matches!(p.phase, Phase::Lists | Phase::Download),
            _ => false,
        }
    }

    fn check(&mut self, cx: &mut Cx) {
        if !self.helper.busy(CHECK) && !self.busy() {
            self.helper.run(cx, CHECK, &["update-check"]);
        }
    }

    /// Enter on the ready list: the battery first, then a card to confirm,
    /// then the password.
    fn ask_install(&mut self, cx: &mut Cx) {
        self.battery = system::battery();
        if let Some(pct) = self.kernel_short() {
            cx.toast_tone(
                Tone::Warning,
                &format!("Kernel update needs over {KERNEL_BATTERY}%, at {pct}%"),
            );
            return;
        }
        let st = &self.state;
        let title = format!("Install {}?", words::updates(st.packages.len()));
        let mut line = format!("{} to download.", update::size_text(st.download()));
        match st.needs() {
            Restart::Device => line.push_str(" The device restarts at the end."),
            Restart::Shell => line.push_str(" Gh0st restarts at the end."),
            Restart::None => {}
        }
        cx.ask(ASK_INSTALL, Dialog::confirm(&title, &line).yes("install"));
    }

    fn restart(&mut self, cx: &mut Cx, r: Restart) {
        match r {
            Restart::Device => self.helper.run(cx, REBOOT, &["power", "reboot"]),
            Restart::Shell => {
                // The shell is a user unit; restarting it closes this app.
                self.shell_jobs
                    .push(cx.spawn(&["systemctl", "--user", "daemon-reload"]));
            }
            Restart::None => {}
        }
    }

    fn enter(&mut self, cx: &mut Cx) {
        match self.view() {
            View::Never | View::Current => self.check(cx),
            View::Ready => self.ask_install(cx),
            View::Done(r) => self.restart(cx, r),
            View::Problem(o) => {
                // An install that couldn't run tries again as an install.
                let install_problem = matches!(
                    o,
                    Outcome::Battery | Outcome::Space | Outcome::Locked | Outcome::Stopped
                );
                if install_problem && !self.state.packages.is_empty() {
                    self.ask_install(cx);
                } else {
                    self.check(cx);
                }
            }
            View::KernelBattery(_) | View::Busy(_) => {}
        }
    }

    fn finished(&mut self, cx: &mut Cx, f: Finished) {
        match (f.tag, f.result) {
            (CHECK | INSTALL, Ok(_)) => {
                self.starting = Some(f.tag == INSTALL);
                self.starting_from = state_mtime();
                if let Some(t) = self.start_timer.take() {
                    cx.cancel(t);
                }
                self.start_timer = Some(cx.after(START_MS));
            }
            (STOP, Ok(_)) => cx.toast("Stopping"),
            (CLEAN, Ok(_)) => {
                self.cache = cache_bytes();
                cx.toast_tone(Tone::Ok, "Downloads cleared");
            }
            (_, Ok(_)) => {}
            (_, Err(why)) => {
                if why != "Cancelled" {
                    cx.toast_tone(Tone::Critical, &why);
                }
            }
        }
        self.sync_segment(cx);
    }

    /// The moving segment runs only while a bar has no percent and the
    /// page is in front.
    fn sync_segment(&mut self, cx: &mut Cx) {
        let want = cx.in_front() && matches!(self.view(), View::Busy(p) if p.percent.is_none());
        match (want, self.segment) {
            (true, None) => self.segment = Some(cx.every(SEGMENT_MS)),
            (false, Some(t)) => {
                cx.cancel(t);
                self.segment = None;
            }
            _ => {}
        }
    }

    fn draw_ready(&self, ui: &mut Ui) {
        let st = &self.state;
        let a = ui.area();
        let t = *ui.theme();
        let c = ui.canvas();
        let head = Rect::new(a.x + 15, a.y + 4, a.w - 30, 18);
        let title = format!(
            "{} · {}",
            words::updates(st.packages.len()),
            update::size_text(st.download())
        );
        widgets::text_in(c, t.f.body_bold, head, &title, t.c.fg);
        let keys = [Hint {
            key: "Enter",
            label: "install",
        }];
        let kw = widgets::key_row_w(&t, &keys);
        widgets::key_row(c, &t, head.right() - kw, head.y + 1, &keys);
        let mut y = head.bottom() + 3;
        for g in [Group::Gh0st, Group::Security, Group::System] {
            let mine: Vec<&update::Package> = st.packages.iter().filter(|p| p.group == g).collect();
            if mine.is_empty() {
                continue;
            }
            let count = mine.len().to_string();
            let tone = if g == Group::Security {
                Tone::Ok
            } else {
                Tone::Info
            };
            let tint = if g == Group::Security {
                t.c.ok
            } else {
                t.c.accent
            };
            let row = WRow {
                icon: Some(words::group_icon(g)),
                tint,
                label: words::group_name(g),
                trailing: Trailing::Tag(&count, tone),
            };
            let r = Rect::new(a.x + t.m.margin, y, a.w - 2 * t.m.margin, t.m.app_row_h);
            widgets::note_row(c, &t, r, &row, &words::group_note(&mine), false);
            y += t.m.app_row_h;
        }
        if st.held > 0 && y + 14 <= a.bottom() {
            let note = format!("{} more wait for the rest of their set", st.held);
            let r = Rect::new(a.x + 15, a.bottom() - 16, a.w - 30, 14);
            widgets::text_in(c, t.f.small, r, &note, t.c.muted);
        }
    }

    fn draw_busy(&self, ui: &mut Ui, p: &Progress) {
        let a = ui.area();
        let t = *ui.theme();
        let n = self.state.packages.len();
        let title = match (p.install, p.phase) {
            (false, _) => "Checking for updates".to_string(),
            (true, Phase::Lists) => "Checking before installing".to_string(),
            (true, Phase::Download) => {
                format!("Downloading {}", words::updates(n.max(p.m as usize)))
            }
            (true, Phase::Install) => format!("Installing {}", words::updates(n.max(p.m as usize))),
            (true, Phase::Clean) => "Cleaning up".to_string(),
        };
        let label = match p.phase {
            _ if self.progress.is_none() => "Starting\u{2026}".to_string(),
            Phase::Lists if p.m > 0 => format!("Package lists · {} of {}", p.n, p.m),
            Phase::Lists => "Package lists".to_string(),
            Phase::Download if p.total > 0 => format!(
                "{} of {}",
                update::size_text(p.done),
                update::size_text(p.total)
            ),
            Phase::Download => "Downloading".to_string(),
            Phase::Install if !p.item.is_empty() => {
                format!("{} · {} of {}", p.item, (p.n + 1).min(p.m.max(1)), p.m)
            }
            Phase::Install => "Installing".to_string(),
            Phase::Clean => "Clearing the download cache".to_string(),
        };
        let note = if self.stoppable() {
            "You can leave this screen. It carries on."
        } else {
            "Keep it on until it is done."
        };
        let c = ui.canvas();
        let w = a.w - 30;
        widgets::text_in(
            c,
            t.f.body_bold,
            Rect::new(a.x + 15, a.y + 8, w, 18),
            &title,
            t.c.fg,
        );
        widgets::progress(
            c,
            &t,
            Rect::new(a.x + 15, a.y + 30, w, 24),
            &label,
            p.percent,
            self.phase,
        );
        widgets::text_in(
            c,
            t.f.small,
            Rect::new(a.x + 15, a.y + 58, w, 14),
            note,
            t.c.muted,
        );
        if p.install || self.starting == Some(true) {
            let step = match p.phase {
                Phase::Lists | Phase::Download => 1,
                Phase::Install | Phase::Clean => 2,
            };
            let r = Rect::new(a.x + 15, a.bottom() - 34, w, 26);
            widgets::stepper(c, &t, r, &["Check", "Download", "Install", "Restart"], step);
        }
    }

    fn draw_problem(&self, ui: &mut Ui, o: Outcome) {
        let why = self.state.why.as_str();
        let retry = [("Enter", "try again")];
        let (icon, tone, text, note): (&str, Tone, &str, String) = match o {
            Outcome::Offline => (
                "wifi-off",
                Tone::Critical,
                "No network",
                "Join Wi-Fi in Settings, then try again.".into(),
            ),
            Outcome::Metered => (
                "wifi",
                Tone::Warning,
                "Waiting for Wi-Fi",
                "The daily check skips metered networks.".into(),
            ),
            Outcome::Clock => (
                "clock",
                Tone::Warning,
                "The clock is behind",
                "Set the date in Settings > System, then try again.".into(),
            ),
            Outcome::Space => (
                "hard-drive",
                Tone::Warning,
                "Not enough space",
                format!("{why}\nClear downloads from the menu (8)."),
            ),
            Outcome::Battery => ("battery-full", Tone::Warning, "Battery low", why.into()),
            Outcome::Locked => (
                "triangle-alert",
                Tone::Warning,
                "Another install is running",
                "The Store may be installing. Try again soon.".into(),
            ),
            Outcome::Stopped => ("square", Tone::Info, "Stopped", "Nothing changed.".into()),
            _ => ("circle-x", Tone::Critical, "Update failed", why.into()),
        };
        ui.notice(icon, Some(tone), text, &note, &retry);
    }
}

impl Page for Updates {
    fn title(&self) -> &str {
        "Updates"
    }

    fn icon(&self) -> &str {
        APP_ICON
    }

    fn status(&self) -> Option<(&str, Tone)> {
        (self.state.channel == "edge").then_some(("EDGE", Tone::Warning))
    }

    fn start(&mut self, cx: &mut Cx) {
        self.h24 = cx.settings().get_str("gh0st.clock") != Some("12h");
        self.reload();
        if let Some(w) = watch_dirs() {
            let id = cx.watch(w.fd());
            self.watch = Some((w, id));
        }
        self.sync_segment(cx);
    }

    fn stop(&mut self, cx: &mut Cx) {
        if let Some((_, id)) = self.watch.take() {
            cx.unwatch(id);
        }
    }

    fn resume(&mut self, cx: &mut Cx) {
        self.reload();
        self.sync_segment(cx);
    }

    fn front(&mut self, _: bool, cx: &mut Cx) {
        self.sync_segment(cx);
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        if let Some((w, _)) = &self.watch {
            w.drain();
        }
        let before = self.view();
        self.reload();
        if self.view() != before || matches!(before, View::Busy(_)) {
            cx.redraw();
        }
        self.sync_segment(cx);
    }

    fn timer(&mut self, timer: Timer, cx: &mut Cx) {
        if Some(timer) == self.segment {
            self.phase = self.phase.wrapping_add(8);
            return;
        }
        if Some(timer) == self.start_timer {
            self.start_timer = None;
            self.starting = None;
            self.reload();
            self.sync_segment(cx);
        }
    }

    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        if let Some(i) = self.shell_jobs.iter().position(|j| *j == job) {
            if let JobEvent::Done(code) = event {
                self.shell_jobs.remove(i);
                if code == Some(0) && self.shell_jobs.is_empty() {
                    self.shell_jobs.push(cx.spawn(&[
                        "systemctl",
                        "--user",
                        "restart",
                        "gh0st.service",
                    ]));
                } else if code != Some(0) {
                    cx.toast_tone(Tone::Critical, "Couldn't restart Gh0st");
                }
            }
            return;
        }
        if let Some(f) = self.helper.job(cx, job, event) {
            self.finished(cx, f);
        }
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if id == ASK_INSTALL {
            if answer != Answer::No {
                self.helper.run(cx, INSTALL, &["update-install"]);
            }
            return;
        }
        if let Some(f) = self.helper.answer(cx, id, answer) {
            self.finished(cx, f);
        }
    }

    fn bars(&self) -> Bars {
        let bars = Bars::top();
        match self.view() {
            View::Busy(_) if self.stoppable() => bars.chip(5, Chip::new("square", "Stop").danger()),
            View::Ready | View::KernelBattery(_) => bars
                .chip(5, Chip::new("refresh-cw", "Check"))
                .chip(6, Chip::new("list", "List")),
            View::Problem(_) if !self.state.packages.is_empty() => bars
                .chip(5, Chip::new("refresh-cw", "Check"))
                .chip(6, Chip::new("list", "List")),
            _ => bars,
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Enter => self.enter(cx),
            Key::Soft(5) if self.stoppable() => self.helper.run(cx, STOP, &["update-stop"]),
            // Only where the chip shows: elsewhere Enter checks.
            Key::Soft(5) if !self.busy() && !self.state.packages.is_empty() => self.check(cx),
            Key::Soft(6) if !self.state.packages.is_empty() => {
                cx.push(Packages::new(&self.state.packages));
            }
            _ => {}
        }
        self.sync_segment(cx);
    }

    fn commands(&self, c: &mut Commands) {
        let busy = self.busy();
        let ready = matches!(self.view(), View::Ready);
        let has = !self.state.packages.is_empty();
        c.group("Updates");
        c.add(Command::action(CMD_INSTALL, "Install all").enabled(ready));
        c.add(Command::action(CMD_CHECK, "Check now").enabled(!busy));
        c.add(Command::action(CMD_LIST, "Packages").opens().enabled(has));
        let clean = if self.cache > 0 {
            format!("Clear downloads, {}", update::size_text(self.cache))
        } else {
            "Clear downloads".to_string()
        };
        c.add(Command::action(CMD_CLEAN, clean).enabled(!busy && self.cache > 0));
    }

    fn command(&mut self, id: u32, _: Step, cx: &mut Cx) {
        match id {
            CMD_INSTALL => self.ask_install(cx),
            CMD_CHECK => self.check(cx),
            CMD_LIST => cx.push(Packages::new(&self.state.packages)),
            CMD_CLEAN => self.helper.run(cx, CLEAN, &["apt-clean"]),
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let st = &self.state;
        let now = gh0st_core::notify::now_secs();
        let channel = words::channel_name(&st.channel);
        match self.view() {
            View::Never => ui.notice(
                APP_ICON,
                None,
                "Not checked yet",
                &format!("{channel} channel"),
                &[("Enter", "check now")],
            ),
            View::Current => {
                let note = words::current_note(st, system::version(), now, self.h24);
                ui.notice(
                    "circle-check",
                    Some(Tone::Ok),
                    "Up to date",
                    &note,
                    &[("Enter", "check now")],
                );
            }
            View::Ready => self.draw_ready(ui),
            View::KernelBattery(pct) => ui.notice(
                "triangle-alert",
                Some(Tone::Warning),
                &format!("Kernel update needs over {KERNEL_BATTERY}%"),
                &format!(
                    "The battery is at {pct}%. Charge it, then install.\nA power cut here can stop the device booting."
                ),
                &[],
            ),
            View::Busy(p) => self.draw_busy(ui, &p),
            View::Done(r) => {
                let text = match st.installed_count {
                    0 | 1 => "Update installed".to_string(),
                    n => format!("{n} updates installed"),
                };
                let (note, key) = match r {
                    Restart::Device => ("Restart to start the new system", "restart"),
                    _ => (
                        "Restarting Gh0st closes open apps\nand locks the screen",
                        "restart Gh0st",
                    ),
                };
                ui.notice(
                    "circle-check",
                    Some(Tone::Ok),
                    &text,
                    note,
                    &[("Enter", key)],
                );
            }
            View::Problem(o) => self.draw_problem(ui, o),
        }
    }
}

fn state_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(State::path())
        .and_then(|m| m.modified())
        .ok()
}

/// Bytes of the debs apt keeps after a download.
fn cache_bytes() -> u64 {
    std::fs::read_dir(ARCHIVES)
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "deb"))
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn watch_dirs() -> Option<gh0st_core::status::Watch> {
    let (state, run) = (update::state_dir(), update::run_dir());
    gh0st_core::status::Watch::dirs(&[state.as_path(), run.as_path()]).ok()
}

#[cfg(not(target_os = "linux"))]
fn watch_dirs() -> Option<gh0st_core::status::Watch> {
    None
}
