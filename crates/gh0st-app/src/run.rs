//! The event loop: one `poll` over the keyboard, signals and job output.
//! On the device frames go to the panel; with `--sim DIR` keys come from a
//! script on stdin and frames land in DIR as PNGs.

use crate::page::Page;
use crate::runtime::Runtime;
use gh0st_core::evdev;
use gh0st_core::rt::{self, Signals};
use gh0st_core::script::Script;
use gh0st_ui::display::{Display, Frames};
use gh0st_ui::fb::{self, Framebuffer};
use std::path::PathBuf;

/// Runs an app whose first page is `page`, and exits when its last page
/// closes. Pass `--sim DIR` to run it on a host: keys come from stdin in the
/// format of `gh0st_core::script`, and each new frame is saved to DIR.
pub fn run(page: impl Page + 'static) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sim = args
        .iter()
        .position(|a| a == "--sim")
        .map(|i| PathBuf::from(args.get(i + 1).map(String::as_str).unwrap_or("frames")));
    let code = match main_loop(Box::new(page), sim) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gh0st app: {e}");
            1
        }
    };
    std::process::exit(code)
}

enum Input {
    Keyboard(evdev::Device),
    Script(Script),
}

fn main_loop(page: Box<dyn Page>, sim: Option<PathBuf>) -> Result<(), String> {
    let mut rt_ = Runtime::new(page, None);
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
            (Box::new(fb), Input::Keyboard(dev))
        }
    };
    let signals = Signals::install(&[libc::SIGTERM, libc::SIGINT, libc::SIGCHLD])
        .map_err(|e| format!("signals: {e}"))?;

    rt_.start(rt::now_ms());
    rt_.render();
    display.flush_all(&rt_.canvas);
    rt_.last_frame = rt::now_ms();

    let mut raw = Vec::new();
    while !rt_.quit {
        let jobs = rt_.job_fds();
        let input_fd = match &input {
            Input::Keyboard(d) => d.fd(),
            Input::Script(s) => s.fd(),
        };
        let watches = rt_.watch_fds();
        let mut fds = vec![rt::pollfd(input_fd), rt::pollfd(signals.fd())];
        fds.extend(jobs.iter().map(|&(_, fd)| rt::pollfd(fd)));
        let watch_at = fds.len();
        fds.extend(
            watches
                .iter()
                .map(|&(_, fd, out)| rt::pollfd_rw(fd, true, out)),
        );
        let script_at = match &input {
            Input::Script(s) => s.next_at(),
            Input::Keyboard(_) => None,
        };
        let deadline = rt::earliest([rt_.next_at(), script_at]);
        rt::wait(&mut fds, deadline).map_err(|e| format!("poll: {e}"))?;
        let now = rt::now_ms();

        if rt::readable(&fds[1]) {
            for sig in signals.drain() {
                match sig {
                    libc::SIGCHLD => rt_.reap(now),
                    _ => rt_.quit_all(),
                }
            }
        }
        raw.clear();
        match &mut input {
            Input::Keyboard(d) => {
                if rt::readable(&fds[0]) {
                    d.read_keys(&mut raw);
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
            if rt::readable(&fds[2 + i]) {
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
            display.flush(&rt_.canvas, rt_.canvas.bounds());
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
