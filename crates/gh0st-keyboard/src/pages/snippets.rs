//! Snippets mode: the host system's shortcuts, then the user's own texts.
//! Enter sends the row. A text is typed a report per tick, so any host
//! keeps up; Esc stops it.

use std::collections::VecDeque;

use gh0st_app::prelude::*;

use crate::app::Shared;
use crate::engine;
use crate::hid::report::Report;
use crate::shortcuts::{self, Send};
use crate::store::Snippet;
use crate::words;

/// A report this often: about 40 characters a second.
pub const PACE_MS: u64 = 12;

pub const NEW: u32 = 400;
pub const DELETE: u32 = 401;
const ASK_NAME: u32 = 410;
const ASK_TEXT: u32 = 411;

#[derive(Default)]
pub struct Snippets {
    list: List,
    queue: VecDeque<Report>,
    /// Characters in the text being typed, and how many reports it makes.
    total: usize,
    tick: Option<Timer>,
    /// The name of a new snippet, while its text is asked for.
    naming: Option<String>,
}

impl Snippets {
    pub fn new() -> Snippets {
        Snippets {
            list: List::stopping(),
            ..Snippets::default()
        }
    }

    pub fn sending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// The selected row's snippet, if it is one of the user's.
    fn mine(&self, s: &Shared) -> Option<usize> {
        let n = shortcuts::of(s.system()).len();
        self.list
            .selected()
            .checked_sub(n)
            .filter(|&i| i < s.snippets.len())
    }

    pub fn bars(&self, s: &Shared) -> Bars {
        if self.sending() {
            return Bars::top().hints([("Esc", "stop")]);
        }
        let new = Chip::new("plus", words::NEW).runs(NEW);
        if self.mine(s).is_none() {
            return Bars::top().chip(6, new);
        }
        let delete = Chip::new("trash-2", words::DELETE)
            .danger()
            .hold()
            .runs(DELETE);
        Bars::top().chip(5, new).chip(7, delete)
    }

    pub fn commands(c: &mut Commands) {
        c.group("Macros");
        c.add(Command::action(NEW, "New macro").keys("n").opens());
    }

    pub fn command(&mut self, s: &mut Shared, id: u32, cx: &mut Cx) -> bool {
        match id {
            NEW => cx.ask(
                ASK_NAME,
                Dialog::text(words::SNIPPET_NAME, "name").yes("next"),
            ),
            DELETE => {
                if let Some(i) = self.mine(s) {
                    s.snippets.remove(i);
                    s.save_snippets();
                }
            }
            _ => return false,
        }
        true
    }

    pub fn answer(&mut self, s: &mut Shared, id: u32, a: Answer, cx: &mut Cx) {
        match (id, a) {
            (ASK_NAME, Answer::Yes(name)) if !name.trim().is_empty() => {
                self.naming = Some(name.trim().to_string());
                cx.ask(
                    ASK_TEXT,
                    Dialog::text(words::SNIPPET_TEXT, "text").yes("save"),
                );
            }
            (ASK_TEXT, Answer::Yes(text)) => {
                if let Some(name) = self.naming.take()
                    && !text.is_empty()
                {
                    s.snippets.push(Snippet { name, text });
                    s.save_snippets();
                    let n = shortcuts::of(s.system()).len();
                    self.list.select(n + s.snippets.len() - 1);
                }
            }
            _ => self.naming = None,
        }
    }

    pub fn key(&mut self, s: &mut Shared, key: Key, cx: &mut Cx) {
        if self.sending() {
            return;
        }
        let Some(Action::Open(i)) = self.list.key(key) else {
            return;
        };
        let cuts = shortcuts::of(s.system());
        let mut out = Vec::new();
        if let Some(cut) = cuts.get(i) {
            match cut.send {
                Send::Keys(m, u) => s.engine.tap(m, u, &mut out),
                Send::Media(u) => s.engine.media_tap(u, &mut out),
            }
            if !s.send(&out) {
                cx.toast(words::NOT_LIVE);
            }
            return;
        }
        let Some(text) = s.snippets.get(i - cuts.len()).map(|x| x.text.clone()) else {
            return;
        };
        if !s.live() {
            cx.toast(words::NOT_LIVE);
            return;
        }
        s.engine.release_all(&mut out);
        s.send(&out);
        out.clear();
        let missing = engine::text(&text, &mut out);
        if missing > 0 {
            cx.toast(&words::skipped(missing));
        }
        self.total = out.len();
        self.queue = out.into();
        self.tick = Some(cx.every(PACE_MS));
    }

    pub fn timer(&mut self, s: &mut Shared, t: Timer, cx: &mut Cx) -> bool {
        if self.tick != Some(t) {
            return false;
        }
        match self.queue.pop_front() {
            Some(r) if s.send(&[r]) => {}
            // The host went: stop, and let go of whatever was down.
            Some(_) | None => self.stop(s, cx),
        }
        if self.queue.is_empty() {
            self.stop(s, cx);
        }
        true
    }

    /// Stops typing; the host gets a key up so nothing stays down.
    pub fn stop(&mut self, s: &mut Shared, cx: &mut Cx) {
        if let Some(t) = self.tick.take() {
            cx.cancel(t);
        }
        if !self.queue.is_empty() {
            self.queue.clear();
            s.send(&[Report::Keyboard([0; 8])]);
        }
    }

    pub fn draw(&self, s: &Shared, ui: &mut Ui) {
        if self.sending() {
            let all = self.total.max(1);
            let done = all - self.queue.len();
            let pct = (done * 100 / all) as u8;
            ui.progress(&words::typing_progress(done / 2, all / 2), Some(pct), 0);
            return;
        }
        let system = s.system();
        let cuts = shortcuts::of(system);
        let keys: Vec<String> = cuts
            .iter()
            .map(|c| shortcuts::keys(system, c.send))
            .collect();
        let mut rows: Vec<Row> = cuts
            .iter()
            .zip(&keys)
            .map(|(c, k)| Row::info(c.label, k).icon("keyboard"))
            .collect();
        for snip in &s.snippets {
            let first = snip.text.lines().next().unwrap_or("");
            rows.push(Row::info(&snip.name, first).icon("file-text"));
        }
        ui.list(&self.list, &rows);
    }
}
