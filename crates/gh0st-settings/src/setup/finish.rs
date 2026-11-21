//! Done: what was set, then the hand-over. Enter makes the user, then
//! `setup-finish` hands the device to them and starts the shell.

use super::{HELPER, Shared, failure, power_answer, power_command, power_menu};
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::widgets::{self, Hint, Message};
use gh0st_app::gfx::{Rect, Theme};
use gh0st_app::prelude::*;
use gh0st_core::sysconf;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Ready,
    /// `user-create` runs.
    Making,
    /// `setup-finish` runs; the shell comes up after it.
    Starting,
    /// What failed, and whether `setup-finish` had begun: a retry may find
    /// the marker gone, so Restart is the way on then.
    Failed(String, bool),
}

const RESTART: u32 = 910;

pub struct Finish {
    label: String,
    a: Shared,
    phase: Phase,
    job: Option<(Job, String)>,
    net_job: Option<(Job, String)>,
    country: String,
    zone: String,
    network: String,
}

impl Finish {
    pub fn new(label: &str, a: Shared) -> Finish {
        let boot = sysconf::boot_dir();
        let read = |p: std::path::PathBuf| std::fs::read_to_string(p).unwrap_or_default();
        let cc = sysconf::regdom(&read(boot.join(sysconf::CMDLINE))).unwrap_or_default();
        let country = sysconf::countries(&read(sysconf::COUNTRIES.into()))
            .into_iter()
            .find(|(c, _)| *c == cc)
            .map_or(cc, |(_, n)| n);
        let zone = std::fs::read_link(super::path("/etc/localtime"))
            .ok()
            .and_then(|p| {
                let p = p.to_string_lossy().into_owned();
                p.split_once("zoneinfo/").map(|(_, z)| z.to_string())
            })
            .unwrap_or_default();
        Finish {
            label: label.into(),
            a,
            phase: Phase::Ready,
            job: None,
            net_job: None,
            country,
            zone,
            network: "None".into(),
        }
    }

    fn make_user(&mut self, cx: &mut Cx) {
        let a = self.a.borrow();
        if a.password.is_empty() {
            drop(a);
            cx.toast("Type the password again");
            return;
        }
        let mut input = format!("{}\n", a.password);
        let job = cx.spawn_with_input(
            &["pkexec", HELPER, "user-create", &a.name],
            input.as_bytes(),
        );
        drop(a);
        std::mem::take(&mut input).into_bytes().fill(0);
        self.job = Some((job, String::new()));
        self.phase = Phase::Making;
    }

    fn job_done(&mut self, code: Option<i32>, out: &str, cx: &mut Cx) {
        match (&self.phase, code) {
            (Phase::Making, Some(0)) => {
                let pw = std::mem::take(&mut self.a.borrow_mut().password);
                pw.into_bytes().fill(0);
                // "Starting" is on the panel before the shell takes it.
                self.phase = Phase::Starting;
                let job = cx.spawn(&["pkexec", HELPER, "setup-finish"]);
                self.job = Some((job, String::new()));
            }
            (Phase::Making, _) => self.phase = Phase::Failed(failure(out), false),
            (Phase::Starting, Some(0)) => cx.quit(),
            (Phase::Starting, _) => self.phase = Phase::Failed(failure(out), true),
            _ => {}
        }
    }
}

impl Page for Finish {
    fn title(&self) -> &str {
        "Ready"
    }

    fn icon(&self) -> &str {
        "circle-check"
    }

    fn context(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn bars(&self) -> Bars {
        match self.phase {
            Phase::Starting => Bars::none(),
            Phase::Failed(_, true) => {
                Bars::top().chip(6, Chip::new("rotate-cw", "Restart").runs(RESTART))
            }
            _ => Bars::top(),
        }
    }

    fn start(&mut self, cx: &mut Cx) {
        let argv = [
            "nmcli",
            "-t",
            "-f",
            "NAME,TYPE",
            "connection",
            "show",
            "--active",
        ];
        let job = cx.spawn(&argv);
        self.net_job = Some((job, String::new()));
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter && matches!(self.phase, Phase::Ready | Phase::Failed(_, false)) {
            self.make_user(cx);
        }
    }

    /// Not while the user is being made or the shell is starting.
    fn back(&mut self, _: &mut Cx) -> bool {
        !matches!(self.phase, Phase::Making | Phase::Starting)
    }

    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        if let Some((j, out)) = self.net_job.as_mut().filter(|(j, _)| *j == job) {
            let _ = j;
            match event {
                JobEvent::Line(l) => {
                    out.push_str(&l);
                    out.push('\n');
                }
                JobEvent::Done(_) => {
                    let out = std::mem::take(out);
                    self.net_job = None;
                    if let Some(name) = out
                        .lines()
                        .filter_map(|l| l.rsplit_once(':'))
                        .find(|(_, kind)| *kind != "loopback")
                        .map(|(n, _)| n.to_string())
                    {
                        self.network = name;
                    }
                }
            }
            return;
        }
        let Some((_, out)) = self.job.as_mut().filter(|(j, _)| *j == job) else {
            return;
        };
        match event {
            JobEvent::Line(l) => {
                out.push_str(&l);
                out.push('\n');
            }
            JobEvent::Done(code) => {
                let out = std::mem::take(out);
                self.job = None;
                self.job_done(code, &out, cx);
            }
        }
    }

    fn commands(&self, c: &mut Commands) {
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        if id == RESTART {
            cx.spawn(&["pkexec", HELPER, "power", "reboot"]);
            return;
        }
        power_command(id, cx);
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        power_answer(id, &answer, cx);
    }

    fn draw(&self, ui: &mut Ui) {
        let t = *ui.theme();
        let area = ui.area();
        match &self.phase {
            Phase::Starting => starting(ui.canvas(), &t, area),
            Phase::Failed(why, finishing) => {
                let keys = [Hint {
                    key: "Enter",
                    label: "try again",
                }];
                let m = Message {
                    icon: "circle-x",
                    tone: Some(Tone::Critical),
                    text: "Couldn't finish",
                    note: why,
                    keys: if *finishing { &[] } else { &keys },
                };
                widgets::message(ui.canvas(), &t, area, &m);
            }
            Phase::Ready | Phase::Making => {
                let name = self.a.borrow().name.clone();
                for (k, v) in [
                    ("Country", self.country.as_str()),
                    ("Time zone", self.zone.as_str()),
                    ("Network", self.network.as_str()),
                    ("Account", name.as_str()),
                ] {
                    ui.readout(k, v);
                }
                if self.phase == Phase::Making {
                    ui.progress(&format!("Making {name}"), None, 0);
                } else {
                    let r = ui.rest();
                    let keys = [Hint {
                        key: "Enter",
                        label: "start",
                    }];
                    widgets::key_row(ui.canvas(), &t, r.x + t.m.margin + 3, r.y + 4, &keys);
                }
            }
        }
    }
}

/// The mark and "Starting", the whole screen.
fn starting(c: &mut gh0st_app::gfx::Canvas, t: &Theme, area: Rect) {
    let s = IconSize::Large as i32;
    let y = area.y + (area.h - s - 30) / 2;
    icon::draw(
        c,
        widgets::LOGO,
        IconSize::Large,
        area.x + (area.w - s) / 2,
        y,
        t.c.accent,
    );
    let tr = Rect::new(area.x, y + s + 12, area.w, 18);
    widgets::text_centre(c, t.f.body, tr, "Starting\u{2026}", t.c.fg);
}
