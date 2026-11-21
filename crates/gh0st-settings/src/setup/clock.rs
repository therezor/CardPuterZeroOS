//! Date and time, only while network time hasn't set the clock.

use super::{HELPER, failure, power_answer, power_command, power_menu};
use crate::model::pages;
use gh0st_app::prelude::*;
use gh0st_core::rt;

/// timesyncd writes this once the clock has synced.
const SYNCED: &str = "/run/systemd/timesync/synchronized";

pub fn synced() -> bool {
    super::path(SYNCED).exists()
}

pub struct DateTime {
    label: String,
    form: Form,
    /// The `set-time` run, and what it printed.
    job: Option<(Job, String)>,
}

impl DateTime {
    pub fn new(label: &str) -> DateTime {
        let (y, mon, d, h, min) = rt::local_ymdhm();
        let form = Form::new()
            .text("Date")
            .with(&format!("{y}-{mon:02}-{d:02}"))
            .text("Time, 24 hour")
            .with(&format!("{h:02}:{min:02}"));
        DateTime {
            label: label.into(),
            form,
            job: None,
        }
    }

    fn submit(&mut self, cx: &mut Cx) {
        // Network time may have set the clock since this opened.
        if synced() {
            return cx.done();
        }
        let text = format!("{} {}", self.form.value(0), self.form.value(1));
        let Some((y, mon, d, h, min)) = pages::parse_date_time(&text) else {
            let bad_date = pages::parse_date_time(&format!("{} 00:00", self.form.value(0)));
            match bad_date {
                None => self.form.error(0, "Type it as 2026-11-21"),
                Some(_) => self.form.error(1, "Type it as 14:32"),
            }
            return;
        };
        let Some(epoch) = rt::local_epoch(y, mon, d, h, min) else {
            self.form.error(0, "That date can't be set");
            return;
        };
        let job = cx.spawn(&["pkexec", HELPER, "set-time", &epoch.to_string()]);
        self.job = Some((job, String::new()));
    }
}

impl Page for DateTime {
    fn title(&self) -> &str {
        "Date and time"
    }

    fn icon(&self) -> &str {
        "clock"
    }

    fn context(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn typing(&self) -> bool {
        true
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(self.form.hints("back", "set"))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.job.is_none() && self.form.key(key) == Some(Edit::Submit) {
            self.submit(cx);
        }
    }

    fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
        let Some((mine, out)) = self.job.as_mut().filter(|(j, _)| *j == job) else {
            return;
        };
        let _ = mine;
        match event {
            JobEvent::Line(l) => {
                out.push_str(&l);
                out.push('\n');
            }
            JobEvent::Done(Some(0)) => {
                self.job = None;
                cx.done();
            }
            JobEvent::Done(_) => {
                let why = failure(out);
                self.job = None;
                self.form.error(1, &why);
            }
        }
    }

    fn commands(&self, c: &mut Commands) {
        power_menu(c);
    }

    fn command(&mut self, id: u32, _: gh0st_app::Step, cx: &mut Cx) {
        power_command(id, cx);
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        power_answer(id, &answer, cx);
    }

    fn draw(&self, ui: &mut Ui) {
        ui.note("No network time yet. Check these.");
        ui.form(&self.form);
    }
}
