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
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let (sim, files) = parse_args(&args);
    let code = match main_loop(Box::new(page), sim, files, opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gh0st app: {e}");
            1
        }
    };
    std::process::exit(code)
}

/// `--sim DIR`, and the files to open: what is left once `--sim DIR` is
/// out, if none of it is a flag, or everything after `--`. An app that
/// takes flags of its own (`-e` in the terminal) opens no files this way.
fn parse_args(args: &[std::ffi::OsString]) -> (Option<PathBuf>, Vec<PathBuf>) {
    let mut sim = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--sim" {
            sim = Some(PathBuf::from(
                args.get(i + 1).cloned().unwrap_or("frames".into()),
            ));
            i += 2;
            continue;
        }
        if args[i] == "--" {
            return (sim, args[i + 1..].iter().map(PathBuf::from).collect());
        }
        rest.push(&args[i]);
        i += 1;
    }
    let flag = rest
        .iter()
        .any(|a| a.as_encoded_bytes().first() == Some(&b'-'));
    let files = if flag {
        Vec::new()
    } else {
        rest.into_iter().map(PathBuf::from).collect()
    };
    (sim, files)
}

enum Input {
    /// The device, and its path to open it again after a pause. None while
    /// paused: the app reads no keys then.
    Keyboard(Option<evdev::Device>, PathBuf),
    Script(Script),
}

fn main_loop(
    page: Box<dyn Page>,
    sim: Option<PathBuf>,
    files: Vec<PathBuf>,
    opts: Options,
) -> Result<(), String> {
    let mut rt_ = Runtime::new(page, None);
    rt_.s.closable = opts.closable;
    rt_.files = files;
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
        l.send(&["hello", &gh0st_core::link::VERSION.to_string()]);
        // The shell says hello first, as it starts the app: know its
        // version before a page can ask it for anything.
        if let Some(m) = l.wait_msg("hello", 0) {
            rt_.shell_v = m.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
        }
    }

    rt_.start(rt::now_ms());
    rt_.render();
    display.flush_all(&rt_.canvas);
    rt_.last_frame = rt::now_ms();

    // Links and addresses, on the device: the top bar and the pages follow
    // a cable pulled at once, not at the next minute.
    #[cfg(target_os = "linux")]
    let net = match input {
        Input::Keyboard(..) => gh0st_core::status::Watch::links().ok(),
        Input::Script(_) => None,
    };
    // A sim keeps its own clipboard: a script must not reach the user's.
    if matches!(input, Input::Script(_)) {
        rt_.s.clip.offline = true;
    }
    let mut raw = Vec::new();
    let mut full = false;
    // GH0ST_LOG=debug: what woke the loop, as the shell logs it.
    let debug = std::env::var("GH0ST_LOG").is_ok_and(|v| v == "debug");
    let t0 = rt::now_ms();
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
        let portal_at = rt_.portal_fd().map(|(fd, out)| {
            fds.push(rt::pollfd_rw(fd, true, out));
            fds.len() - 1
        });
        // Not while paused: nothing shows it then, and what came waits in
        // the socket to be read once on resume.
        #[cfg(target_os = "linux")]
        let net_at = net.as_ref().filter(|_| !rt_.paused).map(|w| {
            fds.push(rt::pollfd(w.fd()));
            fds.len() - 1
        });
        #[cfg(not(target_os = "linux"))]
        let net_at: Option<usize> = None;
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
        if debug {
            let what = |i: usize| match i {
                0 => "signal",
                i if Some(i) == link_at => "shell",
                i if Some(i) == input_at => "keys",
                i if Some(i) == portal_at => "portal",
                i if Some(i) == net_at => "net",
                i if i >= watch_at => "watch",
                i if i >= jobs_at => "job",
                _ => "?",
            };
            let ready: Vec<&str> = (0..fds.len())
                .filter(|&i| rt::readable(&fds[i]))
                .map(what)
                .collect();
            eprintln!(
                "wake at {} {ready:?} deadline {:?}",
                now - t0,
                deadline.map(|d| d as i64 - now as i64)
            );
        }

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
                    Some("hello") => {
                        rt_.shell_v = m.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                    }
                    Some("action") => {
                        if let (Some(tag), Some(id)) = (m.get(1), m.get(2)) {
                            rt_.notice_action(tag, id, now);
                        }
                    }
                    Some("lowmem") => {
                        if let Some(level) = m.get(1).and_then(|v| v.parse().ok()) {
                            rt_.low_memory(level, now);
                        }
                    }
                    Some("open") => {
                        let files: Vec<PathBuf> = m[1..]
                            .iter()
                            .map(|f| gh0st_core::link::path_of(f))
                            .collect();
                        rt_.opened(files, now);
                    }
                    _ => {}
                }
            }
            // The shell is gone: run on alone rather than spin on its socket.
            if rt_.link.as_ref().is_some_and(|l| l.closed()) {
                rt_.link = None;
            }
        }
        if portal_at.is_some_and(|i| fds[i].revents != 0) {
            rt_.portal_ready(now);
        }
        #[cfg(target_os = "linux")]
        if let (Some(i), Some(w)) = (net_at, net.as_ref()) {
            if rt::readable(&fds[i]) && w.drain() {
                rt_.network_changed(now);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> (Option<PathBuf>, Vec<PathBuf>) {
        let a: Vec<std::ffi::OsString> = a.iter().map(Into::into).collect();
        parse_args(&a)
    }

    #[test]
    fn files_are_what_is_left_unless_a_flag_is_there() {
        assert_eq!(
            parse(&["/a.txt", "/b c.txt"]).1,
            [PathBuf::from("/a.txt"), "/b c.txt".into()]
        );
        let (sim, files) = parse(&["--sim", "/tmp/f", "/a.txt"]);
        assert_eq!(sim, Some("/tmp/f".into()));
        assert_eq!(files, [PathBuf::from("/a.txt")]);
        assert!(
            parse(&["-e", "ls", "-la"]).1.is_empty(),
            "the terminal's own flags"
        );
        assert!(parse(&["--setup"]).1.is_empty());
        assert_eq!(
            parse(&["--x", "--", "-odd name"]).1,
            [PathBuf::from("-odd name")]
        );
    }
}
