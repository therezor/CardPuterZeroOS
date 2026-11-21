//! A run of pages the user walks in order, as first-boot setup is.

use crate::page::{Cx, Page, Request};
use crate::ui::Ui;

type Maker = Box<dyn FnMut(&str) -> Option<Box<dyn Page>>>;

/// Pages the user walks through in order, one per step. `Steps` is a page
/// itself and opens one step at a time above it, so each step is an
/// ordinary page: its timers, jobs and dialogs end when it closes.
///
/// A step calls [`Cx::done`] to go on; Esc, or [`Cx::back`], goes to the
/// step before. Each step is made when it opens, from a maker that gets the
/// step's label for the top bar ("Setup 3 of 10"); return it from
/// [`Page::context`]. A maker that returns `None` is skipped, in whichever
/// direction the user is going: a step that applies only sometimes. Going
/// back makes the step again, so keep what it collected outside it.
///
/// Esc on the first step makes it again rather than leaving. After the last
/// step `Steps` closes itself as if done. In an app that can't close it
/// makes the last step again instead, so that step quits with [`Cx::quit`]
/// once its work is done.
///
/// ```
/// use gh0st_app::prelude::*;
///
/// struct Ask(String);
///
/// impl Page for Ask {
///     fn title(&self) -> &str { "Name" }
///     fn context(&self) -> Option<&str> { Some(&self.0) }
///     fn draw(&self, ui: &mut Ui) { ui.text("Press Enter.") }
///     fn key(&mut self, key: Key, cx: &mut Cx) {
///         if key == Key::Enter { cx.done() }
///     }
/// }
///
/// let synced = false;
/// let setup = Steps::new("Setup")
///     .step(|label| Ask(label.into()))
///     .optional(move |label| (!synced).then(|| Ask(label.into())))
///     .step(|label| Ask(label.into()));
/// # let _ = setup;
/// ```
pub struct Steps {
    name: String,
    makers: Vec<Maker>,
    at: usize,
    on_step: Option<Box<dyn FnMut(usize)>>,
}

impl Steps {
    /// Steps whose labels start with `name`: "Setup 3 of 10".
    pub fn new(name: &str) -> Steps {
        Steps {
            name: name.to_string(),
            makers: Vec::new(),
            at: 0,
            on_step: None,
        }
    }

    /// A step that always applies.
    pub fn step<P: Page + 'static>(mut self, mut make: impl FnMut(&str) -> P + 'static) -> Steps {
        self.makers.push(Box::new(move |label| {
            Some(Box::new(make(label)) as Box<dyn Page>)
        }));
        self
    }

    /// A step that applies when its maker returns a page.
    pub fn optional<P: Page + 'static>(
        mut self,
        mut make: impl FnMut(&str) -> Option<P> + 'static,
    ) -> Steps {
        self.makers.push(Box::new(move |label| {
            make(label).map(|p| Box::new(p) as Box<dyn Page>)
        }));
        self
    }

    /// Opens at step `i`, counting from 0, as when resuming after a power
    /// cut.
    pub fn from(mut self, i: usize) -> Steps {
        self.at = i;
        self
    }

    /// Hears each step's index as it opens, to save where the user is.
    pub fn on_step(mut self, f: impl FnMut(usize) + 'static) -> Steps {
        self.on_step = Some(Box::new(f));
        self
    }

    /// Opens the first step that applies from `i`, going forward or back.
    /// Back past the first step turns forward again from 0. Past the last,
    /// an app that can't close turns back to the last that applies.
    fn open(&mut self, i: isize, forward: bool, cx: &mut Cx) {
        let total = self.makers.len() as isize;
        let (mut i, mut forward) = (i.min(total), forward);
        let (mut front, mut end) = (false, false);
        loop {
            if i < 0 {
                if front {
                    break;
                }
                (i, forward, front) = (0, true, true);
                continue;
            }
            if i >= total {
                if cx.s.closable || end {
                    break;
                }
                (i, forward, end) = (total - 1, false, true);
                continue;
            }
            let label = format!("{} {} of {}", self.name, i + 1, total);
            if let Some(page) = (self.makers[i as usize])(&label) {
                self.at = i as usize;
                if let Some(f) = self.on_step.as_mut() {
                    f(self.at);
                }
                cx.s.requests.push(Request::Push(page));
                return;
            }
            i += if forward { 1 } else { -1 };
        }
        cx.done();
    }
}

impl Page for Steps {
    fn title(&self) -> &str {
        &self.name
    }

    /// A step is always on top, so this shows only between two of them.
    fn draw(&self, _: &mut Ui) {}

    fn start(&mut self, cx: &mut Cx) {
        self.open(self.at as isize, true, cx);
    }

    fn resume(&mut self, cx: &mut Cx) {
        let at = self.at as isize;
        if cx.s.done {
            self.open(at + 1, true, cx);
        } else {
            self.open(at - 1, false, cx);
        }
    }
}
