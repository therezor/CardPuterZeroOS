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
use crate::page::{Cx, Job, JobEvent, Watch};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};

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

/// A helper verb that stays up, as Files' root worker: the helper reads
/// one line on its socket, the password or nothing under grace, then
/// becomes the program the page talks to over the same socket.
///
/// [`start`](Held::start) hands back the page's end of the socket; the
/// page queues what it has to say until the program says it is up, and
/// calls [`up`](Held::up). Pass it [`Page::exited`](crate::Page::exited),
/// [`Page::answer`](crate::Page::answer) and [`Page::ready`](crate::Page::ready).
/// A helper that exits before it is up asked for the password, or failed:
/// the dialog opens as for [`Helper`], and a password starts it again.
pub struct Held {
    ask_id: u32,
    args: Vec<String>,
    run: Option<HeldRun>,
    asking: bool,
}

struct HeldRun {
    pid: i32,
    err: std::fs::File,
    watch: Watch,
    out: String,
    up: bool,
}

/// What a started or answered [`Held`] became.
#[derive(Debug)]
pub enum HeldEvent {
    /// Running: the page's end of the socket.
    Started(OwnedFd),
    /// Over for good: refused, cancelled, failed, or ended after it was up.
    Ended(Result<(), String>),
}

impl Held {
    /// `ask_id`: the password dialog's id, used for nothing else.
    pub fn new(ask_id: u32, args: &[&str]) -> Held {
        Held {
            ask_id,
            args: args.iter().map(|a| a.to_string()).collect(),
            run: None,
            asking: false,
        }
    }

    /// Starts the helper with no password: grace, or the dialog, follows.
    pub fn start(&mut self, cx: &mut Cx) -> HeldEvent {
        self.spawn(cx, "\n".to_string())
    }

    pub fn running(&self) -> bool {
        self.run.is_some() || self.asking
    }

    fn spawn(&mut self, cx: &mut Cx, mut line: String) -> HeldEvent {
        let r = self.try_spawn(cx, line.as_bytes());
        wipe(&mut line);
        match r {
            Ok(fd) => HeldEvent::Started(fd),
            Err(e) => HeldEvent::Ended(Err(format!("Couldn't start the helper: {e}"))),
        }
    }

    fn try_spawn(&mut self, cx: &mut Cx, line: &[u8]) -> std::io::Result<OwnedFd> {
        let mut fds = [0; 2];
        // SAFETY: socketpair fills a two-int array.
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: both fds are fresh and owned from here.
        let (mine, theirs) =
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        // SAFETY: fcntl on an fd we own; the helper gets its end as stdin.
        unsafe { libc::fcntl(mine.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        let mut child = Command::new("pkexec")
            .arg(PATH)
            .args(&self.args)
            .stdin(Stdio::from(theirs))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let err = child
            .stderr
            .take()
            .map(|e| std::fs::File::from(OwnedFd::from(e)))
            .ok_or_else(|| std::io::Error::other("no stderr"))?;
        // SAFETY: fcntl on an fd we own.
        unsafe {
            let fl = libc::fcntl(err.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(err.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        let pid = child.id() as i32;
        cx.adopt(pid);
        let watch = cx.watch(err.as_raw_fd());
        self.run = Some(HeldRun {
            pid,
            err,
            watch,
            out: String::new(),
            up: false,
        });
        let mut off = 0;
        while off < line.len() {
            // SAFETY: writes from a live buffer to our end of the socket.
            let n = unsafe {
                libc::write(
                    mine.as_raw_fd(),
                    line[off..].as_ptr().cast(),
                    line.len() - off,
                )
            };
            if n <= 0 {
                return Err(std::io::Error::last_os_error());
            }
            off += n as usize;
        }
        Ok(mine)
    }

    /// The program said it is up: past this, its exit is the end, not a
    /// question.
    pub fn up(&mut self) {
        if let Some(r) = &mut self.run {
            r.up = true;
        }
    }

    /// The helper's stderr is readable. True if `watch` was its.
    pub fn ready(&mut self, watch: Watch) -> bool {
        let Some(r) = self.run.as_mut().filter(|r| r.watch == watch) else {
            return false;
        };
        drain(&mut r.err, &mut r.out);
        true
    }

    /// A child ended. `Some` when it was the helper and that settles
    /// something; a password dialog opens instead when one is wanted.
    pub fn exited(&mut self, cx: &mut Cx, pid: i32, code: Option<i32>) -> Option<HeldEvent> {
        if self.run.as_ref().is_none_or(|r| r.pid != pid) {
            return None;
        }
        let mut r = self.run.take()?;
        drain(&mut r.err, &mut r.out);
        cx.unwatch(r.watch);
        if r.up {
            return Some(HeldEvent::Ended(match code {
                Some(0) => Ok(()),
                _ => Err(failure(&r.out)),
            }));
        }
        Some(HeldEvent::Ended(Err(match outcome(code, &r.out) {
            Outcome::Ask(title) => {
                cx.ask(self.ask_id, Dialog::password(title).yes("ok"));
                self.asking = true;
                return None;
            }
            Outcome::Locked(secs) => format!("Too many tries. Wait {secs} s"),
            Outcome::Failed(why) => why,
            Outcome::Done => "It ended at once".into(),
        })))
    }

    /// A dialog's answer: a password starts the helper again with it.
    pub fn answer(&mut self, cx: &mut Cx, id: u32, answer: Answer) -> Option<HeldEvent> {
        if id != self.ask_id || !self.asking {
            return None;
        }
        self.asking = false;
        match answer {
            Answer::Yes(mut password) if !password.is_empty() => {
                let line = format!("{password}\n");
                wipe(&mut password);
                Some(self.spawn(cx, line))
            }
            _ => Some(HeldEvent::Ended(Err("Cancelled".into()))),
        }
    }

    /// Lets go: the page closes its end of the socket, and the program
    /// ends when it reads that.
    pub fn stop(&mut self, cx: &mut Cx) {
        if let Some(r) = self.run.take() {
            cx.unwatch(r.watch);
        }
        self.asking = false;
    }
}

fn drain(f: &mut std::fs::File, out: &mut String) {
    let mut buf = [0u8; 1024];
    while let Ok(n) = f.read(&mut buf) {
        if n == 0 || out.len() > 16 * 1024 {
            break;
        }
        out.push_str(&String::from_utf8_lossy(&buf[..n]));
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
