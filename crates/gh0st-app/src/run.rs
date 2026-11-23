//! The event loop: one `poll` over the keyboard, signals and job output.
//! On the device frames go to the panel; with `--sim DIR` keys come from a
//! script on stdin and frames land in DIR as PNGs.

use crate::page::Page;
use crate::runtime::Runtime;
use gh0st_core::evdev;
use gh0st_core::link::Link;
use gh0st_core::rt::{self, Signals};
use gh0st_core::script::Script;
use gh0st_ui::display::{Display, Frames};
use gh0st_ui::fb::{self, Framebuffer};
use std::path::PathBuf;

/// How an app runs. [`run`] takes the default.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// False for an app the user must finish, as first-boot setup is: Esc
    /// held on the first page does nothing, the menu has no Close app, and
    /// the last page never closes. [`Cx::quit`](crate::Cx::quit) and
    /// SIGTERM still end it.
    pub closable: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { closable: true }
    }
}

/// Runs an app whose first page is `page`, and exits when its last page
/// closes. Pass `--sim DIR` to run it on a host: keys come from stdin in the
/// format of `gh0st_core::script`, and each new frame is saved to DIR.
pub fn run(page: impl Page + 'static) -> ! {
    run_with(page, Options::default())
}

/// As [`run`], with [`Options`].
pub fn run_with(page: impl Page + 'static, opts: Options) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sim = args
        .iter()
        .position(|a| a == "--sim")
        .map(|i| PathBuf::from(args.get(i + 1).map(String::as_str).unwrap_or("frames")));
    let code = match main_loop(Box::new(page), sim, opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gh0st app: {e}");
            1
        }
    };
    std::process::exit(code)
}

enum Input {
    /// The device, and its path to open it again after a pause. None while
    /// paused: the app reads no keys then.
    Keyboard(Option<evdev::Device>, PathBuf),
    Script(Script),
}

fn main_loop(page: Box<dyn Page>, sim: Option<PathBuf>, opts: Options) -> Result<(), String> {
    let mut rt_ = Runtime::new(page, None);
    rt_.s.closable = opts.closable;
    if sim.is_some() {
        rt_.off_device();
    }
    let (w, h) = (rt_.canvas.width(), rt_.canvas.height());
    let (mut display, mut input): (Box<dyn Display>, Input) = match &sim {
        Some(dir) => (
            Box::new(
                Frames::new(dir.clone(), w, h).map_err(|e| format!("{}: {e}", dir.display()))?,
            ),
            Input::Script(Script::stdin()),
        ),
        None => {
            let path = fb::find();
            let fb = Framebuffer::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let kb = evdev::find().ok_or("no keyboard found")?;
            let dev = evdev::Device::open(&kb).map_err(|e| format!("{}: {e}", kb.display()))?;
            (Box::new(fb), Input::Keyboard(Some(dev), kb))
        }
    };
    let signals = Signals::install(&[libc::SIGTERM, libc::SIGINT, libc::SIGCHLD])
        .map_err(|e| format!("signals: {e}"))?;
    // Under --sim too when a test hands one over.
    rt_.link = Link::from_env();
    if let Some(l) = &rt_.link {
        l.send(&["hello", "1"]);
    }

    rt_.start(rt::now_ms());
    rt_.render();
    display.flush_all(&rt_.canvas);
    rt_.last_frame = rt::now_ms();

    let mut raw = Vec::new();
    let mut full = false;
    while !rt_.quit {
        let jobs = rt_.job_fds();
        let watches = rt_.watch_fds();
        let input_fd = match &input {
            Input::Keyboard(d, _) => d.as_ref().map(|d| d.fd()),
            Input::Script(s) => Some(s.fd()),
        };
        let mut fds = vec![rt::pollfd(signals.fd())];
        let link_at = rt_.link.as_ref().map(|l| {
            fds.push(rt::pollfd(l.fd()));
            fds.len() - 1
        });
        let input_at = input_fd.map(|fd| {
            fds.push(rt::pollfd(fd));
            fds.len() - 1
        });
        let jobs_at = fds.len();
        fds.extend(jobs.iter().map(|&(_, fd)| rt::pollfd(fd)));
        let watch_at = fds.len();
        fds.extend(
            watches
                .iter()
                .map(|&(_, fd, out)| rt::pollfd_rw(fd, true, out)),
        );
        let script_at = match &input {
            Input::Script(s) => s.next_at(),
            Input::Keyboard(..) => None,
        };
        let deadline = rt::earliest([rt_.next_at(), script_at]);
        rt::wait(&mut fds, deadline).map_err(|e| format!("poll: {e}"))?;
        let now = rt::now_ms();

        if rt::readable(&fds[0]) {
            for sig in signals.drain() {
                match sig {
                    libc::SIGCHLD => rt_.reap(now),
                    _ => rt_.quit_all(),
                }
            }
        }
        // The shell's word comes before keys: after a pause, keys belong
        // to whatever is in front now.
        if link_at.is_some_and(|i| rt::readable(&fds[i])) {
            while let Some(m) = rt_.link.as_ref().and_then(|l| l.recv()) {
                match m.first().map(String::as_str) {
                    Some("pause") => {
                        if let Input::Keyboard(dev, _) = &mut input {
                            *dev = None;
                        }
                        rt_.pause_input(now);
                        if let Some(l) = &rt_.link {
                            l.send(&["paused"]);
                        }
                    }
                    Some("resume") => {
                        if let Input::Keyboard(dev, path) = &mut input {
                            *dev = evdev::Device::open(path).ok();
                        }
                        rt_.resume_input(now);
                        full = true;
                    }
                    Some("redraw") => {
                        rt_.dirty = true;
                        full = true;
                    }
                    _ => {}
                }
            }
            // The shell is gone: run on alone rather than spin on its socket.
            if rt_.link.as_ref().is_some_and(|l| l.closed()) {
                rt_.link = None;
            }
        }
        raw.clear();
        match &mut input {
            Input::Keyboard(d, _) => {
                if let (Some(d), Some(i)) = (d.as_mut(), input_at) {
                    if rt::readable(&fds[i]) {
                        d.read_keys(&mut raw);
                    }
                }
            }
            Input::Script(s) => {
                // Always try: stdin is non-blocking, and macOS poll answers
                // POLLNVAL for /dev/null instead of readable.
                s.read(now);
                s.due(now, &mut raw);
            }
        }
        for &(code, value) in &raw {
            rt_.raw_key(code, value, now);
        }
        for (i, &(id, _)) in jobs.iter().enumerate() {
            if rt::readable(&fds[jobs_at + i]) {
                rt_.read_job(id, now);
            }
        }
        for (i, &(id, _, _)) in watches.iter().enumerate() {
            let f = &fds[watch_at + i];
            if rt::writable(f) {
                rt_.writable(id);
            }
            if rt::readable(f) {
                rt_.ready(id, now);
            }
        }
        rt_.tick(now);
        if rt_.frame_due(now) {
            rt_.render();
            if std::mem::take(&mut full) {
                display.flush_all(&rt_.canvas);
            } else {
                display.flush(&rt_.canvas, rt_.canvas.bounds());
            }
            rt_.last_frame = now;
        }
        if let Input::Script(s) = &input {
            if s.finished(now) && raw.is_empty() {
                break;
            }
        }
    }
    Ok(())
}
