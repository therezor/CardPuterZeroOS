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
//! d.keys("esc");
//! assert!(d.quit());
//! ```

use crate::page::Page;
use crate::runtime::Runtime;
use gh0st_core::script::Script;
use gh0st_ui::Canvas;
use std::path::Path;

pub struct Driver {
    rt: Runtime,
    now: u64,
}

impl Driver {
    /// Starts `page`. Settings go to a fresh file in the temp directory, so a
    /// test never touches the real config.
    pub fn new(page: impl Page + 'static) -> Driver {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let config = std::env::temp_dir().join(format!(
            "gh0st-app-test-{}-{n}/config.json",
            std::process::id()
        ));
        let mut rt = Runtime::new(Box::new(page), Some(config));
        rt.start(0);
        Driver { rt, now: 0 }
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
        }
        self.now = end;
        self
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
}
