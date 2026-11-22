//! `gh0st update check [--auto]` and `gh0st update install`: the update
//! engine. systemd runs it as root (`gh0st-update-*.service`), never an
//! app, so closing the Updates app or restarting the shell can't cut dpkg
//! off mid-install. It writes what it finds and how far it is into the
//! files `gh0st_core::update` reads, and posts the notice. The design is
//! in docs/updates.md.

mod apt;

use gh0st_core::battery;
use gh0st_core::notify::{self, Notice};
use gh0st_core::update::{
    self, Group, KERNEL_BATTERY, MIN_BATTERY, Outcome, Package, Phase, Progress, Restart, State,
};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

/// The Updates app's desktop stem: what the notice opens.
const APP: &str = "update";
/// The notice's poster and tag.
const POSTER: &str = "gh0st-update";
const TAG: &str = "updates";
const EDGE_SOURCES: &str = "/etc/apt/sources.list.d/gh0st-edge.sources";
/// Lists older than this are fetched again before an install: Debian
/// removes superseded debs.
const LISTS_FRESH_SECS: u64 = 3600;
/// Room kept free past what the install needs.
const SPARE_BYTES: u64 = 100_000_000;
/// The progress file is rewritten at most this often.
const PROGRESS_MS: u64 = 250;
/// What is kept of apt's own output, for the error.
const OUT_MAX: usize = 32 * 1024;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

pub fn main(args: &[String]) -> i32 {
    // SAFETY: geteuid has no failure mode.
    let root = unsafe { libc::geteuid() } == 0;
    if !root && std::env::var_os("GH0ST_UPDATE_STATE").is_none() {
        eprintln!("gh0st update: run by systemd as root (gh0st-update-*.service)");
        return 13;
    }
    // SAFETY: the handler only stores to an atomic.
    unsafe { libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t) };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["check"] => check(false),
        ["check", "--auto"] => check(true),
        ["install"] => install(),
        _ => {
            eprintln!("usage: gh0st update check [--auto] | install");
            2
        }
    }
}

/// The engine's lock, held for its life. A second run waits.
fn lock() -> std::io::Result<std::fs::File> {
    let dir = update::run_dir();
    std::fs::create_dir_all(&dir)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(update::lock_path())?;
    // SAFETY: flock on a descriptor we own; blocks until the other run ends.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(f)
}

fn now() -> u64 {
    notify::now_secs()
}

fn channel() -> String {
    if Path::new(EDGE_SOURCES).exists() {
        "edge".into()
    } else {
        "stable".into()
    }
}

fn save(st: &State) {
    let dir = update::state_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = update::write_atomic(&State::path(), &st.text()) {
        eprintln!("gh0st update: state: {e}");
    }
}

fn set_pending(on: bool) {
    let p = update::pending_path();
    if on {
        let _ = std::fs::write(p, b"");
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Rewrites the progress line, at most every `PROGRESS_MS`.
struct Reporter {
    p: Progress,
    at: u64,
}

impl Reporter {
    fn new(install: bool) -> Reporter {
        Reporter {
            p: Progress {
                install,
                ..Progress::default()
            },
            at: 0,
        }
    }

    fn write(&mut self, force: bool) {
        let t = gh0st_core::rt::now_ms();
        if !force && t < self.at + PROGRESS_MS {
            return;
        }
        self.at = t;
        let _ = update::write_atomic(&Progress::path(), &self.p.text());
    }

    fn phase(&mut self, phase: Phase, item: &str) {
        self.p.phase = phase;
        self.p.percent = None;
        self.p.item = item.into();
        self.write(true);
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(Progress::path());
    }
}

fn check(auto: bool) -> i32 {
    let _lock = match lock() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gh0st update: lock: {e}");
            return 1;
        }
    };
    let mut st = State::load();
    st.channel = channel();
    clear_done_restart(&mut st);
    let mut rep = Reporter::new(false);
    if let Err((outcome, why)) = ready_to_check(auto) {
        st.outcome = outcome;
        st.why = why;
        set_pending(matches!(outcome, Outcome::Offline | Outcome::Metered));
        save(&st);
        return 0;
    }
    match refresh(&mut st, &mut rep) {
        Ok(()) => {
            st.outcome = Outcome::Ok;
            st.why.clear();
        }
        Err((outcome, why)) => {
            st.outcome = outcome;
            st.why = why;
        }
    }
    st.checked = now();
    set_pending(st.outcome == Outcome::Offline);
    save(&st);
    post_notice(&st);
    0
}

/// Network, metering and the clock, before any traffic.
fn ready_to_check(auto: bool) -> Result<(), (Outcome, String)> {
    if !online() {
        return Err((Outcome::Offline, "No network".into()));
    }
    if auto && metered() {
        return Err((
            Outcome::Metered,
            "Waits for a network that isn't metered".into(),
        ));
    }
    if clock_behind() {
        return Err((Outcome::Clock, "The clock is behind".into()));
    }
    Ok(())
}

/// `apt-get update`, then what an upgrade would bring.
fn refresh(st: &mut State, rep: &mut Reporter) -> Result<(), (Outcome, String)> {
    rep.phase(Phase::Lists, "");
    let (code, out) = apt(
        "apt-get",
        &["update", "-q", "--error-on=any", "-o", "APT::Status-Fd=3"],
        rep,
        |_| true,
    );
    if STOP.load(Ordering::SeqCst) {
        return Err((Outcome::Stopped, "Stopped".into()));
    }
    // --error-on=any: a source that failed, offline or unsigned, fails the
    // run instead of leaving old lists in place.
    if code != Some(0) {
        return Err(apt::failure(&out));
    }
    list_upgrades(st)
}

/// Fills `st` from a simulated upgrade, without touching the lists.
fn list_upgrades(st: &mut State) -> Result<(), (Outcome, String)> {
    let sim = run(&["apt-get", "-s", "-q", "upgrade", "--with-new-pkgs"])
        .map_err(|e| (Outcome::Error, e))?;
    let (insts, held) = apt::simulate(&sim);
    let wanted: Vec<String> = insts
        .iter()
        .map(|i| format!("{}={}", i.name, i.to))
        .collect();
    let records = if wanted.is_empty() {
        Vec::new()
    } else {
        let mut argv = vec!["apt-cache", "show", "--no-all-versions"];
        argv.extend(wanted.iter().map(String::as_str));
        apt::records(&run(&argv).unwrap_or_default())
    };
    let old = installed_kb(&insts.iter().map(|i| i.name.as_str()).collect::<Vec<_>>());
    let mut grow: i64 = 0;
    st.packages = insts
        .iter()
        .map(|i| {
            let r = records
                .iter()
                .find(|r| r.name == i.name && r.version == i.to);
            let new_kb = r.map_or(0, |r| r.installed_kb) as i64;
            let old_kb = old
                .iter()
                .find(|(n, _)| *n == i.name)
                .map_or(0, |(_, k)| *k) as i64;
            grow += new_kb - old_kb;
            Package {
                name: i.name.clone(),
                from: i.from.clone(),
                to: i.to.clone(),
                size: r.map_or(0, |r| r.size),
                group: Group::of_origin(&i.origin),
                restart: Restart::of(&i.name),
                summary: r.map(|r| r.summary.clone()).unwrap_or_default(),
            }
        })
        .collect();
    st.packages
        .sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
    st.held = held;
    st.grow = grow.max(0) as u64 * 1024;
    Ok(())
}

fn installed_kb(names: &[&str]) -> Vec<(String, u64)> {
    if names.is_empty() {
        return Vec::new();
    }
    let mut argv = vec!["dpkg-query", "-W", "-f", "${Package}\t${Installed-Size}\n"];
    argv.extend(names);
    run(&argv)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (n, k) = l.split_once('\t')?;
            Some((n.to_string(), k.trim().parse().ok()?))
        })
        .collect()
}

fn install() -> i32 {
    let _lock = match lock() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gh0st update: lock: {e}");
            return 1;
        }
    };
    let mut st = State::load();
    st.channel = channel();
    let mut rep = Reporter::new(true);
    let result = install_steps(&mut st, &mut rep);
    match result {
        Ok(()) => {
            st.outcome = Outcome::Ok;
            st.why.clear();
        }
        Err((outcome, why)) => {
            st.outcome = outcome;
            st.why = why;
        }
    }
    save(&st);
    post_notice(&st);
    0
}

fn install_steps(st: &mut State, rep: &mut Reporter) -> Result<(), (Outcome, String)> {
    if now().saturating_sub(st.checked) > LISTS_FRESH_SECS || st.packages.is_empty() {
        ready_to_check(false)?;
        refresh(st, rep)?;
        st.checked = now();
    }
    if st.packages.is_empty() {
        return Ok(());
    }
    power_ok(st.kernel())?;
    space_ok(st)?;
    if dpkg_interrupted() {
        rep.phase(Phase::Install, "an earlier install");
        let out = run(&["dpkg", "--configure", "-a"]).map_err(|e| (Outcome::Error, e))?;
        drop(out);
    }
    let count = st.packages.len() as u32;
    let needs = st.needs();
    let total = st.download();
    let mut retried = false;
    loop {
        let (code, out) = upgrade(rep, count, total);
        if code == Some(0) {
            break;
        }
        if STOP.load(Ordering::SeqCst) {
            return Err((Outcome::Stopped, "Stopped".into()));
        }
        if !retried && apt::retryable(&out) {
            retried = true;
            refresh(st, rep)?;
            continue;
        }
        return Err(apt::failure(&out));
    }
    // SAFETY: sync has no failure mode.
    unsafe { libc::sync() };
    rep.phase(Phase::Clean, "");
    let _ = run(&["apt-get", "clean"]);
    st.installed = now();
    st.installed_count = count;
    st.restart = st.restart.max(needs);
    list_upgrades(st)?;
    Ok(())
}

/// The upgrade itself. Stop works until dpkg starts; past that the engine
/// finishes, whatever asks.
fn upgrade(rep: &mut Reporter, count: u32, total: u64) -> (Option<i32>, String) {
    rep.p.phase = Phase::Download;
    rep.p.total = total;
    rep.p.m = count;
    rep.write(true);
    let mut dpkg = false;
    apt(
        "apt-get",
        &[
            "-y",
            "-q",
            "-o",
            "APT::Status-Fd=3",
            "-o",
            "Dpkg::Options::=--force-confdef",
            "-o",
            "Dpkg::Options::=--force-confold",
            "-o",
            "DPkg::Lock::Timeout=120",
            "upgrade",
            "--with-new-pkgs",
        ],
        rep,
        |s| {
            dpkg |= matches!(s, apt::Status::Pm { .. });
            !dpkg
        },
    )
}

/// Runs an apt program with Status-Fd on fd 3. Status lines update `rep`;
/// `stoppable` hears each one and says whether SIGTERM may still stop the
/// run. Returns the exit code and the tail of its output.
fn apt(
    program: &str,
    args: &[&str],
    rep: &mut Reporter,
    mut stoppable: impl FnMut(&apt::Status) -> bool,
) -> (Option<i32>, String) {
    let Ok((status_r, status_w)) = pipe() else {
        return (None, "pipe failed".into());
    };
    let w = status_w.as_raw_fd();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("APT_LISTCHANGES_FRONTEND", "none")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: dup2 and close are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(w, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (None, format!("{program}: {e}")),
    };
    drop(status_w);
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let mut status_file = std::fs::File::from(status_r);
    let fds: Vec<RawFd> = [
        Some(status_file.as_raw_fd()),
        out_pipe.as_ref().map(|p| p.as_raw_fd()),
        err_pipe.as_ref().map(|p| p.as_raw_fd()),
    ]
    .into_iter()
    .flatten()
    .collect();
    for &fd in &fds {
        let _ = gh0st_core::rt::set_nonblock_cloexec(fd);
    }
    let mut out = String::new();
    let mut partial = String::new();
    let mut open = [true, out_pipe.is_some(), err_pipe.is_some()];
    let mut can_stop = true;
    let mut signalled = false;
    while open.iter().any(|&o| o) {
        if STOP.load(Ordering::SeqCst) && can_stop && !signalled {
            // SAFETY: signalling our own child.
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            signalled = true;
        }
        let mut pfds: Vec<libc::pollfd> =
            fds.iter().map(|&fd| gh0st_core::rt::pollfd(fd)).collect();
        let _ = gh0st_core::rt::wait(&mut pfds, Some(gh0st_core::rt::now_ms() + 500));
        let mut buf = [0u8; 4096];
        for (i, pfd) in pfds.iter().enumerate() {
            if !open[i] || pfd.revents == 0 {
                continue;
            }
            let n = match i {
                0 => status_file.read(&mut buf),
                1 => out_pipe.as_mut().map_or(Ok(0), |p| p.read(&mut buf)),
                _ => err_pipe.as_mut().map_or(Ok(0), |p| p.read(&mut buf)),
            };
            match n {
                Ok(0) => open[i] = false,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if i == 0 {
                        partial.push_str(&text);
                        while let Some(nl) = partial.find('\n') {
                            let line: String = partial.drain(..=nl).collect();
                            let s = apt::status(line.trim_end());
                            can_stop = stoppable(&s);
                            report(rep, &s);
                        }
                    } else {
                        out.push_str(&text);
                        if out.len() > OUT_MAX {
                            let cut = out.len() - OUT_MAX;
                            let cut = (cut..out.len())
                                .find(|&c| out.is_char_boundary(c))
                                .unwrap_or(cut);
                            out.drain(..cut);
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => open[i] = false,
            }
        }
    }
    let code = child.wait().ok().and_then(|s| s.code());
    rep.write(true);
    (code, out)
}

/// One Status-Fd line into the progress file.
fn report(rep: &mut Reporter, s: &apt::Status) {
    let p = &mut rep.p;
    match s {
        apt::Status::Dl { percent, n, m } => {
            if p.phase == Phase::Lists {
                // The percent stays 0 for most of a list fetch: the file
                // count says more.
                p.n = *n;
                p.m = *m;
                let files = (*n * 100).checked_div(*m).unwrap_or(0);
                p.percent = Some(files.max(*percent as u32).min(100) as u8);
            } else {
                p.phase = Phase::Download;
                p.percent = Some(percent.clamp(0.0, 100.0) as u8);
                p.done = (p.total as f64 * (*percent as f64) / 100.0) as u64;
            }
        }
        apt::Status::Pm { pkg, percent, what } => {
            if p.phase != Phase::Install {
                p.phase = Phase::Install;
                p.n = 0;
            }
            if what.starts_with("Installed ") {
                p.n = (p.n + 1).min(p.m.max(1));
            }
            p.percent = Some(percent.clamp(0.0, 100.0) as u8);
            p.item = pkg.clone();
        }
        _ => return,
    }
    rep.write(false);
}

/// A close-on-exec pipe. Blocking: apt's end becomes its fd 3, and a
/// non-blocking Status-Fd would lose lines.
fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe fills two descriptors we then own; fcntl sets a flag on
    // each.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        for fd in fds {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])))
    }
}

/// A short program's stdout, or its error.
fn run(argv: &[&str]) -> Result<String, String> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .env("LC_ALL", "C.UTF-8")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(apt::failure(&err).1);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A default route, IPv4 or IPv6.
fn online() -> bool {
    let v4 = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let has_v4 = v4
        .lines()
        .skip(1)
        .any(|l| l.split_whitespace().nth(1) == Some("00000000"));
    let v6 = std::fs::read_to_string("/proc/net/ipv6_route").unwrap_or_default();
    let has_v6 = v6.lines().any(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() >= 10 && f[0] == "00000000000000000000000000000000" && f[1] == "00" && f[9] != "lo"
    });
    has_v4 || has_v6
}

/// NetworkManager calls a connected device metered: a phone's hotspot.
fn metered() -> bool {
    run(&[
        "nmcli",
        "-t",
        "-f",
        "GENERAL.STATE,GENERAL.METERED",
        "device",
        "show",
    ])
    .unwrap_or_default()
    .lines()
    .collect::<Vec<_>>()
    .windows(2)
    .any(|w| w[0].contains("(connected)") && w[1].starts_with("GENERAL.METERED:yes"))
}

/// Earlier than the last thing dpkg did: apt would call every signature
/// "not valid yet".
fn clock_behind() -> bool {
    let Ok(m) = std::fs::metadata("/var/lib/dpkg/status").and_then(|m| m.modified()) else {
        return false;
    };
    let then = m
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now() + 86_400 < then
}

fn dpkg_interrupted() -> bool {
    std::fs::read_dir("/var/lib/dpkg/updates").is_ok_and(|mut d| d.next().is_some())
}

/// Battery rules: the kernel set needs more than 50%, charging or not;
/// anything else 30% or the charger. No battery found (a bench supply):
/// fine.
fn power_ok(kernel: bool) -> Result<(), (Outcome, String)> {
    let Some(r) = battery::find(Path::new("/sys/class/power_supply"))
        .as_deref()
        .and_then(battery::sample)
    else {
        return Ok(());
    };
    if kernel && r.percent <= KERNEL_BATTERY {
        return Err((
            Outcome::BatteryKernel,
            format!(
                "Kernel update needs over {KERNEL_BATTERY}%, at {}%",
                r.percent
            ),
        ));
    }
    if !r.charging && r.percent < MIN_BATTERY {
        return Err((
            Outcome::Battery,
            format!("Battery at {}%: charge it first", r.percent),
        ));
    }
    Ok(())
}

fn space_ok(st: &State) -> Result<(), (Outcome, String)> {
    let need = st.download() + st.grow + SPARE_BYTES;
    let free = free_bytes("/var/cache/apt/archives").min(free_bytes("/"));
    if free < need {
        return Err((
            Outcome::Space,
            format!(
                "Needs {} free, {} is",
                update::size_text(need),
                update::size_text(free)
            ),
        ));
    }
    Ok(())
}

fn free_bytes(path: &str) -> u64 {
    let Ok(c) = std::ffi::CString::new(path) else {
        return u64::MAX;
    };
    // SAFETY: statvfs fills a zeroed struct for a NUL-terminated path.
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) != 0 {
            return u64::MAX;
        }
        s.f_bavail as u64 * s.f_frsize as u64
    }
}

/// A restart the last install needed has happened: the device booted, or
/// the shell started, after it.
fn clear_done_restart(st: &mut State) {
    if update::restart_done(st) {
        st.restart = Restart::None;
    }
}

/// The notice: updates ready, a restart owed, or none.
fn post_notice(st: &State) {
    let dir = notify::system_dir();
    let n = st.packages.len();
    let notice = if st.restart != Restart::None {
        Some(Notice {
            title: "Restart to finish updating".into(),
            body: match st.restart {
                Restart::Device => "The new kernel and system start then".into(),
                _ => "Gh0st restarts. Open apps close".into(),
            },
            tone: notify::Tone::Warning,
            ..Notice::default()
        })
    } else if n > 0 {
        // Still true after a failed run: the list is what the last good
        // check found.
        Some(Notice {
            title: if n == 1 {
                "1 update ready".into()
            } else {
                format!("{n} updates ready")
            },
            body: summary(st),
            count: n as u32,
            ..Notice::default()
        })
    } else {
        None
    };
    match notice {
        Some(mut n) => {
            n.app = POSTER.into();
            n.tag = TAG.into();
            n.open = APP.into();
            n.time = now();
            // The same news again keeps its time, so a dismissed notice
            // stays dismissed until something changes.
            let old = notify::read_dir(&dir, true)
                .into_iter()
                .find(|o| o.app == POSTER && o.tag == TAG);
            if let Some(o) = old.filter(|o| o.title == n.title && o.body == n.body) {
                n.time = o.time;
            }
            if let Err(e) = notify::post(&dir, &n) {
                eprintln!("gh0st update: notice: {e}");
            }
        }
        None => {
            let _ = notify::remove(&dir, POSTER, TAG);
        }
    }
}

/// "Gh0st 0.5, 3 security fixes · 34 MB"
fn summary(st: &State) -> String {
    let mut parts = Vec::new();
    if let Some(g) = st.packages.iter().find(|p| p.name == "gh0st") {
        parts.push(format!("Gh0st {}", short_version(&g.to)));
    }
    if st.kernel() {
        parts.push("new kernel".into());
    }
    match st.count(Group::Security) {
        0 => {}
        1 => parts.push("1 security fix".into()),
        n => parts.push(format!("{n} security fixes")),
    }
    let size = update::size_text(st.download());
    if parts.is_empty() {
        size
    } else {
        format!("{} · {size}", parts.join(", "))
    }
}

/// "0.5.0" from "0.5.0+202611221733.35a14dd".
pub fn short_version(v: &str) -> &str {
    v.split('+').next().unwrap_or(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, group: Group, size: u64) -> Package {
        Package {
            name: name.into(),
            to: "0.5.0+202611221733.35a14dd".into(),
            size,
            group,
            restart: Restart::of(name),
            ..Package::default()
        }
    }

    #[test]
    fn the_summary_names_what_matters() {
        let st = State {
            packages: vec![
                pkg("gh0st", Group::Gh0st, 4_100_000),
                pkg("gh0st-kernel", Group::Gh0st, 21_000_000),
                pkg("openssl", Group::Security, 2_000_000),
                pkg("libssl3t64", Group::Security, 2_000_000),
                pkg("tzdata", Group::System, 300_000),
            ],
            ..State::default()
        };
        assert_eq!(
            summary(&st),
            "Gh0st 0.5.0, new kernel, 2 security fixes · 29 MB"
        );
        let small = State {
            packages: vec![pkg("tzdata", Group::System, 300_000)],
            ..State::default()
        };
        assert_eq!(summary(&small), "300 kB");
    }

    #[test]
    fn a_status_stream_moves_the_progress() {
        let mut rep = Reporter::new(true);
        rep.p.phase = Phase::Download;
        rep.p.total = 1000;
        rep.p.m = 2;
        report(
            &mut rep,
            &apt::status("dlstatus:2:50.0000:Retrieving file 2 of 4"),
        );
        assert_eq!((rep.p.percent, rep.p.done), (Some(50), 500));
        report(
            &mut rep,
            &apt::status("pmstatus:libc6:10.0:Unpacking libc6 (arm64)"),
        );
        assert_eq!(rep.p.phase, Phase::Install);
        report(
            &mut rep,
            &apt::status("pmstatus:libc6:40.0:Installed libc6 (arm64)"),
        );
        report(
            &mut rep,
            &apt::status("pmstatus:gh0st:90.0:Installed gh0st (arm64)"),
        );
        report(
            &mut rep,
            &apt::status("pmstatus:x:95.0:Installed x (arm64)"),
        );
        assert_eq!((rep.p.n, rep.p.m, rep.p.item.as_str()), (2, 2, "x"));
        std::mem::forget(rep);
    }
}
