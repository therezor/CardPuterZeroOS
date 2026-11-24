//! Runs a page without a panel or keyboard, for unit tests: keys in, frames
//! out.
//!
//! ```
//! use gh0st_app::prelude::*;
//! use gh0st_app::testing::Driver;
//!
//! struct Count(u32);
//!
//! impl Page for Count {
//!     fn title(&self) -> &str { "Count" }
//!     fn key(&mut self, key: Key, _: &mut Cx) {
//!         if key == Key::Enter { self.0 += 1 }
//!     }
//!     fn draw(&self, ui: &mut Ui) { ui.big(&self.0.to_string()) }
//! }
//!
//! let mut d = Driver::new(Count(0));
//! d.keys("enter\nenter");
//! d.frame();
//! // On the first page Esc closes the app only when held.
//! d.keys("down esc").wait(800);
//! assert!(d.quit());
//! ```

use crate::page::Page;
use crate::run::Options;
use crate::runtime::Runtime;
use gh0st_core::script::Script;
use gh0st_ui::Canvas;
use std::path::Path;

pub struct Driver {
    rt: Runtime,
    now: u64,
    /// The shell's end of the app's link, once [`listen`](Driver::listen)ed.
    shell: Option<gh0st_core::link::Link>,
}

impl Driver {
    /// Starts `page`. Settings go to a fresh file in the temp directory, so a
    /// test never touches the real config.
    pub fn new(page: impl Page + 'static) -> Driver {
        Driver::with(page, Options::default())
    }

    /// As [`new`](Driver::new), run as [`run_with`](crate::run_with) runs it.
    pub fn with(page: impl Page + 'static, opts: Options) -> Driver {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let config = std::env::temp_dir().join(format!(
            "gh0st-app-test-{}-{n}/config.json",
            std::process::id()
        ));
        let notices = config.with_file_name("notify");
        let home = config.with_file_name("home");
        let mut rt = Runtime::new(Box::new(page), Some(config));
        rt.s.notices = notices;
        rt.s.home = home;
        rt.s.closable = opts.closable;
        rt.off_device();
        rt.own_time();
        rt.portal_offline = true;
        rt.s.clip.offline = true;
        rt.start(0);
        Driver {
            rt,
            now: 0,
            shell: None,
        }
    }

    /// Presses keys, one script line each: `down`, `enter`, `esc`, `6`,
    /// `fn+z`, `type hello`. The format is `gh0st_core::script`'s.
    pub fn keys(&mut self, script: &str) -> &mut Driver {
        for (code, value) in Script::from_text(script).drain_all() {
            self.now += 30;
            self.rt.raw_key(code, value, self.now);
            self.rt.tick(self.now);
        }
        self
    }

    /// Lets `ms` pass, firing timers on the way.
    pub fn wait(&mut self, ms: u64) -> &mut Driver {
        let end = self.now + ms;
        while let Some(at) = self.rt.next_at().filter(|&at| at <= end) {
            self.now = self.now.max(at);
            self.rt.tick(self.now);
            // As the real loop does: a due frame clears its deadline.
            if self.rt.frame_due(self.now) {
                self.rt.render();
                self.rt.last_frame = self.now;
            }
        }
        self.now = end;
        self
    }

    /// Lets `ms` pass with no key and counts the times the loop would wake.
    /// An idle page wakes for the clock's minute and for nothing else
    /// (CLAUDE.md, Performance first).
    pub fn idle_wakes(&mut self, ms: u64) -> u32 {
        let end = self.now + ms;
        let mut wakes = 0;
        while let Some(at) = self.rt.next_at().filter(|&at| at <= end) {
            self.now = self.now.max(at);
            self.rt.tick(self.now);
            if self.rt.frame_due(self.now) {
                self.rt.render();
                self.rt.last_frame = self.now;
            }
            wakes += 1;
            if wakes > 10_000 {
                break;
            }
        }
        self.now = end;
        wakes
    }

    /// Runs the real loop over watched fds for `ms` of wall time: reads,
    /// queued writes and child exits reach the page as on the device.
    pub fn pump(&mut self, ms: u64) -> &mut Driver {
        let end = gh0st_core::rt::now_ms() + ms;
        loop {
            let left = end.saturating_sub(gh0st_core::rt::now_ms());
            let watches = self.rt.watch_fds();
            let mut fds: Vec<_> = watches
                .iter()
                .map(|&(_, fd, out)| gh0st_core::rt::pollfd_rw(fd, true, out))
                .collect();
            let _ = gh0st_core::rt::wait(&mut fds, Some(gh0st_core::rt::now_ms() + left.min(10)));
            self.now += 1;
            for (f, &(id, _, _)) in fds.iter().zip(&watches) {
                if gh0st_core::rt::writable(f) {
                    self.rt.writable(id);
                }
                if gh0st_core::rt::readable(f) {
                    self.rt.ready(id, self.now);
                }
            }
            self.rt.reap(self.now);
            self.rt.tick(self.now);
            if left == 0 {
                return self;
            }
        }
    }

    /// Runs the real loop over jobs from [`Cx::spawn`](crate::Cx::spawn)
    /// until none is left, or `ms` of wall time: their lines and exit codes
    /// reach the page as on the device.
    pub fn jobs(&mut self, ms: u64) -> &mut Driver {
        let end = gh0st_core::rt::now_ms() + ms;
        while self.rt.has_jobs() {
            let now = gh0st_core::rt::now_ms();
            if now >= end {
                break;
            }
            let jobs = self.rt.job_fds();
            let mut fds: Vec<_> = jobs
                .iter()
                .map(|&(_, fd)| gh0st_core::rt::pollfd(fd))
                .collect();
            let _ = gh0st_core::rt::wait(&mut fds, Some(now + (end - now).min(10)));
            self.now += 1;
            for (f, &(id, _)) in fds.iter().zip(&jobs) {
                if gh0st_core::rt::readable(f) {
                    self.rt.read_job(id, self.now);
                }
            }
            self.rt.reap(self.now);
            self.rt.tick(self.now);
        }
        self
    }

    /// Another app comes in front, as Fn+K does: no keys, no frames.
    pub fn pause(&mut self) -> &mut Driver {
        self.rt.pause_input(self.now);
        self
    }

    /// Back in front.
    pub fn resume(&mut self) -> &mut Driver {
        self.rt.resume_input(self.now);
        self
    }

    /// The shell hands the app `files`, as when another app opens them
    /// here, or as the app's argv at start.
    pub fn open(&mut self, files: &[&Path]) -> &mut Driver {
        let files = files.iter().map(|f| f.to_path_buf()).collect();
        self.rt.opened(files, self.now);
        self
    }

    /// Memory is short (see [`Page::low_memory`](crate::Page::low_memory)).
    /// The test's own home folder: where its recent list is kept.
    pub fn home(&self) -> &std::path::Path {
        &self.rt.s.home
    }

    /// The recent list, newest first, as the app left it.
    pub fn recent(&self) -> Vec<std::path::PathBuf> {
        gh0st_core::recent::list(&self.rt.s.home, gh0st_core::recent::KEEP)
    }

    /// What the app has copied, as the clipboard holds it.
    pub fn clipboard(&self) -> Option<&[u8]> {
        self.rt.s.clip.local.as_deref()
    }

    /// Something copied, as if by another app.
    pub fn set_clipboard(&mut self, data: &[u8]) -> &mut Driver {
        self.rt.s.clip.local = Some(data.to_vec());
        self
    }

    /// A link or an address changed (see
    /// [`Page::network_changed`](crate::Page::network_changed)).
    pub fn network_changed(&mut self) -> &mut Driver {
        self.rt.network_changed(self.now);
        self
    }

    pub fn low_memory(&mut self, level: u8) -> &mut Driver {
        self.rt.low_memory(level, self.now);
        self
    }

    /// The notices the app has up now, as the shell would read them.
    pub fn notices(&self) -> Vec<crate::notify::Notice> {
        crate::notify::read_dir(&self.rt.s.notices, false)
    }

    /// The notifications panel ran action `id` of notice `tag`.
    pub fn notice_action(&mut self, tag: &str, id: &str) -> &mut Driver {
        self.rt.notice_action(tag, id, self.now);
        self
    }

    /// A page asked to step back to the app that opened this one
    /// ([`Cx::leave`](crate::Cx::leave)).
    pub fn left(&self) -> bool {
        self.rt.left
    }

    /// What the pages asked the file chooser, oldest first, not yet
    /// answered.
    pub fn picks(&self) -> Vec<&crate::portal::PickSpec> {
        self.rt.pick_log.iter().map(|(_, _, s)| s).collect()
    }

    /// Answers the oldest pick: the paths, or None for a user who gave up.
    pub fn answer_pick(&mut self, paths: Option<Vec<std::path::PathBuf>>) -> &mut Driver {
        if !self.rt.pick_log.is_empty() {
            let (token, owner, _) = self.rt.pick_log.remove(0);
            self.rt.picked(token, owner, paths, self.now);
        }
        self
    }

    /// Every file a page asked to open, and the app it named, oldest
    /// first: [`Cx::open_file`](crate::Cx::open_file) and
    /// [`Cx::open_with`](crate::Cx::open_with).
    pub fn opened_requests(&self) -> &[(Option<String>, std::path::PathBuf)] {
        &self.rt.open_log
    }

    /// Shows the app in another palette, as when `gh0st.palette` changes.
    pub fn palette(&mut self, name: &str) -> &mut Driver {
        self.rt
            .s
            .settings
            .set_str(gh0st_core::config::key::PALETTE, name);
        self.rt.retheme();
        self
    }

    /// The app asked the shell to keep the screen on.
    pub fn awake(&self) -> bool {
        self.rt.awake
    }

    /// The busy card showing now, if any.
    pub fn busy(&self) -> Option<&str> {
        self.rt.busy_text()
    }

    /// The toast showing now, if any.
    pub fn toast(&self) -> Option<&str> {
        self.rt.toast_text()
    }

    /// The top page's title, as the top bar shows it.
    pub fn title(&self) -> String {
        self.rt.top_title()
    }

    /// The word the top page puts in the top bar's status, as "Playing".
    pub fn status(&self) -> Option<String> {
        self.rt.top_status()
    }

    /// The open dialog's title, and what is wrong under its field.
    pub fn dialog(&self) -> Option<(&str, Option<&str>)> {
        self.rt.dialog_text()
    }

    /// Draws the screen under the design lint (`gh0st_ui::lint`): text that
    /// doesn't fit its box or the screen, faces under 15 px, chips off their
    /// keys. Empty when the frame keeps the rules.
    pub fn lint(&mut self) -> Vec<String> {
        gh0st_ui::lint::start();
        self.rt.render();
        let found = gh0st_ui::lint::take();
        gh0st_ui::lint::stop();
        found
    }

    /// The screen as the panel would show it now.
    pub fn frame(&mut self) -> &Canvas {
        self.rt.render();
        &self.rt.canvas
    }

    /// Saves the frame as a PNG at 2x, for a look by eye.
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.rt.render();
        gh0st_ui::snapshot::write_png(&self.rt.canvas, path, 2)
    }

    /// How many pages are open.
    pub fn depth(&self) -> usize {
        self.rt.depth()
    }

    /// True once the last page has closed.
    pub fn quit(&self) -> bool {
        self.rt.quit
    }

    /// Puts a shell on the other end of the app's link, so a test can read
    /// what the app tells it: [`heard`](Driver::heard).
    pub fn listen(&mut self) -> &mut Driver {
        let pair = gh0st_core::link::Link::pair()
            .and_then(|(shell, app)| Ok((shell, gh0st_core::link::Link::from_fd(app)?)));
        if let Ok((shell, app)) = pair {
            self.rt.link = Some(app);
            self.shell = Some(shell);
        }
        self
    }

    /// What the app said to the shell since the last call, a line each:
    /// `sound move`, `awake 1`.
    pub fn heard(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(m) = self.shell.as_ref().and_then(|l| l.recv()) {
            out.push(m.join(" "));
        }
        out
    }
}
