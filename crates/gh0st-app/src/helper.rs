//! The root helper, `gh0st-helper`, from a page: its verbs run through
//! pkexec, and the ones that need the user's password ask for it.
//!
//! A call first runs without a password: inside the 5 minutes after the
//! last one, or for a verb that needs none, that is enough. When the helper
//! wants one (exit 10, or 11 after a wrong one) a password dialog opens, and
//! the call runs again with the password on stdin. It never goes in argv,
//! and the SDK wipes its copies once written.
//!
//! ```
//! use gh0st_app::helper::{Finished, Helper};
//! use gh0st_app::prelude::*;
//!
//! const CHECK: u32 = 1;
//!
//! struct Updates {
//!     helper: Helper,
//! }
//!
//! impl Page for Updates {
//!     fn title(&self) -> &str {
//!         "Updates"
//!     }
//!
//!     fn key(&mut self, key: Key, cx: &mut Cx) {
//!         if key == Key::Enter {
//!             self.helper.run(cx, CHECK, &["update-check"]);
//!         }
//!     }
//!
//!     fn job(&mut self, job: Job, event: JobEvent, cx: &mut Cx) {
//!         if let Some(f) = self.helper.job(cx, job, event) {
//!             self.finished(cx, f);
//!         }
//!     }
//!
//!     fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
//!         if let Some(f) = self.helper.answer(cx, id, answer) {
//!             self.finished(cx, f);
//!         }
//!     }
//!
//!     fn draw(&self, ui: &mut Ui) {
//!         ui.text("Enter checks");
//!     }
//! }
//!
//! impl Updates {
//!     fn finished(&mut self, cx: &mut Cx, f: Finished) {
//!         match f.result {
//!             Ok(_) => cx.toast("Checking\u{2026}"),
//!             Err(why) => cx.toast(&why),
//!         }
//!     }
//! }
//! # let _ = Updates { helper: Helper::new(900) };
//! ```

use crate::dialog::{Answer, Dialog};
use crate::page::{Cx, Job, JobEvent};

/// pkexec runs the helper by this path (see packaging/polkit).
pub const PATH: &str = "/usr/libexec/gh0st-helper";

/// What a finished helper run means for the user.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Ask for the password, with this title.
    Ask(&'static str),
    /// Too many wrong passwords: wait this many seconds.
    Locked(u64),
    Failed(String),
}

/// Reads the helper's exit code and output (see `gh0st/src/helper.rs`).
pub fn outcome(code: Option<i32>, out: &str) -> Outcome {
    match code {
        Some(0) => Outcome::Done,
        Some(10) => Outcome::Ask("Password"),
        Some(11) => Outcome::Ask("Wrong password"),
        Some(12) => Outcome::Locked(
            out.lines()
                .find_map(|l| l.trim().strip_prefix("gh0st-helper: locked "))
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(0),
        ),
        _ => Outcome::Failed(failure(out)),
    }
}

/// The words for a failed run: the helper's last line, or why pkexec
/// refused.
pub fn failure(out: &str) -> String {
    if out.contains("Not authorized") || out.contains("dismissed") || out.contains("polkit") {
        return "Not allowed: polkit rule missing".into();
    }
    out.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start_matches("gh0st-helper: ").to_string())
        .unwrap_or_else(|| "Failed".into())
}

/// A call that ended for good: done, failed, or the password was refused.
#[derive(Debug, PartialEq, Eq)]
pub struct Finished {
    /// What the page passed to [`Helper::run`].
    pub tag: u32,
    /// The helper's output, or what to tell the user.
    pub result: Result<String, String>,
}

#[derive(Clone, Debug)]
struct Call {
    tag: u32,
    args: Vec<String>,
}

/// Runs helper verbs for one page, and asks for the password when one is
/// needed. Pass it the page's [`Page::job`](crate::Page::job) and
/// [`Page::answer`](crate::Page::answer) events.
pub struct Helper {
    ask_id: u32,
    running: Vec<(Job, Call, String)>,
    asking: Option<Call>,
}

impl Helper {
    /// `ask_id` is the dialog id it asks the password with: pick one the
    /// page uses for nothing else.
    pub fn new(ask_id: u32) -> Helper {
        Helper {
            ask_id,
            running: Vec::new(),
            asking: None,
        }
    }

    /// Runs `gh0st-helper args...`. `tag` comes back in [`Finished`].
    pub fn run(&mut self, cx: &mut Cx, tag: u32, args: &[&str]) {
        let call = Call {
            tag,
            args: args.iter().map(|a| a.to_string()).collect(),
        };
        let job = cx.spawn(&argv(&call));
        self.running.push((job, call, String::new()));
    }

    /// True while a call of `tag` runs or waits for its password.
    pub fn busy(&self, tag: u32) -> bool {
        self.running.iter().any(|(_, c, _)| c.tag == tag)
            || self.asking.as_ref().is_some_and(|c| c.tag == tag)
    }

    /// A job event. `Some` when one of its calls ended for good; `None`
    /// for another job, a line, or a call now asking for the password.
    pub fn job(&mut self, cx: &mut Cx, job: Job, event: JobEvent) -> Option<Finished> {
        let i = self.running.iter().position(|(j, ..)| *j == job)?;
        let code = match event {
            JobEvent::Line(l) => {
                let out = &mut self.running[i].2;
                out.push_str(&l);
                out.push('\n');
                return None;
            }
            JobEvent::Done(code) => code,
        };
        let (_, call, out) = self.running.remove(i);
        let result = match outcome(code, &out) {
            Outcome::Done => Ok(out),
            Outcome::Ask(title) if self.asking.is_none() => {
                cx.ask(self.ask_id, Dialog::password(title).yes("ok"));
                self.asking = Some(call);
                return None;
            }
            Outcome::Ask(_) => Err("A password is needed".into()),
            Outcome::Locked(secs) => Err(format!("Too many tries. Wait {secs} s")),
            Outcome::Failed(why) => Err(why),
        };
        Some(Finished {
            tag: call.tag,
            result,
        })
    }

    /// A dialog's answer. `Some` when the password was refused (Esc); a
    /// password runs the call again.
    pub fn answer(&mut self, cx: &mut Cx, id: u32, answer: Answer) -> Option<Finished> {
        if id != self.ask_id {
            return None;
        }
        let call = self.asking.take()?;
        match answer {
            Answer::Yes(mut password) if !password.is_empty() => {
                let mut input = format!("{password}\n");
                let job = cx.spawn_with_input(&argv(&call), input.as_bytes());
                wipe(&mut input);
                wipe(&mut password);
                self.running.push((job, call, String::new()));
                None
            }
            _ => Some(Finished {
                tag: call.tag,
                result: Err("Cancelled".into()),
            }),
        }
    }
}

fn argv(call: &Call) -> Vec<&str> {
    let mut argv = vec!["pkexec", PATH];
    argv.extend(call.args.iter().map(String::as_str));
    argv
}

fn wipe(s: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { s.as_bytes_mut().fill(0) };
    s.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_become_outcomes() {
        assert_eq!(outcome(Some(0), ""), Outcome::Done);
        assert_eq!(outcome(Some(10), ""), Outcome::Ask("Password"));
        assert_eq!(outcome(Some(11), ""), Outcome::Ask("Wrong password"));
        assert_eq!(
            outcome(Some(12), "gh0st-helper: locked 40\n"),
            Outcome::Locked(40)
        );
        assert_eq!(
            outcome(
                Some(1),
                "gh0st-helper: the kernel has no powersave governor\n"
            ),
            Outcome::Failed("the kernel has no powersave governor".into())
        );
        assert_eq!(
            outcome(
                Some(126),
                "Error executing command as another user: Not authorized\n"
            ),
            Outcome::Failed("Not allowed: polkit rule missing".into())
        );
        assert_eq!(outcome(None, ""), Outcome::Failed("Failed".into()));
    }
}
