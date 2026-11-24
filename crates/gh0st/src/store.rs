//! `gh0st store job` and `gh0st store refresh`: the Store's engine.
//! systemd runs it as root (`gh0st-store.service`), never the app. It
//! fetches and checks the signed catalog, and installs, updates and removes
//! apps with apt. It writes the files `gh0st_core::store` reads. The design
//! is in docs/store.md.

use crate::update::{self, Reporter, STOP, apt};
use gh0st_core::notify::{self, Notice};
use gh0st_core::store::{self, App, Catalog, Job, Mirror, Outcome, Pinned, Request, State};
use gh0st_core::update::{Outcome as UpOutcome, Phase};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

/// Lists older than this are fetched again before an install.
const LISTS_FRESH_SECS: u64 = 86_400;
/// A catalog older than this is fetched again before an install.
const CATALOG_FRESH_SECS: u64 = 86_400;
/// Room kept free past what an install needs.
const SPARE_BYTES: u64 = 100_000_000;
const CATALOG_MAX: &str = "4000000";
const SIG_MAX: &str = "100000";
/// About and icons are pinned by size; this only bounds a wrong file.
const ART_MAX: &str = "16000000";
/// How often the download percent is sampled.
const SAMPLE_MS: u64 = 250;

/// The Store's desktop stem, what the notice opens; and the notice's
/// poster and tag.
const APP: &str = "store";
const POSTER: &str = "gh0st-store";
const TAG: &str = "apps";

type Failure = (Outcome, String);

pub fn main(args: &[String]) -> i32 {
    // SAFETY: geteuid has no failure mode.
    let root = unsafe { libc::geteuid() } == 0;
    if !root && std::env::var_os("GH0ST_STORE_STATE").is_none() {
        eprintln!("gh0st store: run by systemd as root (gh0st-store.service)");
        return 13;
    }
    update::on_sigterm();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["job"] => job(),
        ["refresh"] => run_job(Request {
            job: Job::Refresh,
            mirror: Mirror::Auto,
        }),
        _ => {
            eprintln!("usage: gh0st store job | refresh");
            2
        }
    }
}

/// What the helper asked for, from the job file. It goes once read, so a
/// failed run never repeats by itself.
fn job() -> i32 {
    let path = store::job_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    match Request::parse(&text) {
        Some(r) => run_job(r),
        None => {
            eprintln!("gh0st store: no job, or a bad one");
            2
        }
    }
}

fn run_job(req: Request) -> i32 {
    let mut st = State::load();
    st.op = match &req.job {
        Job::Refresh => "refresh",
        Job::Remove(_) => "remove",
        Job::Install(_) | Job::Upgrade => "install",
    }
    .into();
    st.packages = match &req.job {
        Job::Install(p) => p.clone(),
        Job::Remove(p) => vec![p.clone()],
        _ => Vec::new(),
    };
    st.failed.clear();
    st.detail.clear();
    // The update engine's lock: apt never runs twice.
    let lock = match update::try_lock() {
        Ok(Some(l)) => l,
        Ok(None) => return finish(&mut st, Err((Outcome::Locked, String::new()))),
        Err(e) => return finish(&mut st, Err((Outcome::Failed, format!("lock: {e}")))),
    };
    let r = match req.job {
        Job::Refresh => {
            let mut rep = Reporter::at(store::progress_path(), false);
            refresh(&mut st, &mut rep)
        }
        Job::Install(pkgs) => install_job(&mut st, &pkgs, req.mirror),
        Job::Upgrade => upgrade(&mut st, req.mirror),
        Job::Remove(p) => remove(&p),
    };
    let code = finish(&mut st, r);
    drop(lock);
    code
}

fn finish(st: &mut State, r: Result<(), Failure>) -> i32 {
    match r {
        Ok(()) => {
            st.outcome = Outcome::Ok;
            st.failed.clear();
        }
        Err((o, why)) => {
            st.outcome = o;
            st.detail = why;
        }
    }
    st.ended = update::now();
    let _ = std::fs::create_dir_all(store::state_dir());
    if let Err(e) = gh0st_core::update::write_atomic(&store::state_path(), &st.text()) {
        eprintln!("gh0st store: state: {e}");
    }
    post_notice();
    0
}

/// The daily check's turn, from the update engine, which holds the lock
/// already: refresh the list, then the notice.
pub(crate) fn daily() {
    let mut st = State::load();
    st.op = "refresh".into();
    st.packages.clear();
    st.failed.clear();
    st.detail.clear();
    let mut rep = Reporter::at(store::progress_path(), false);
    let r = refresh(&mut st, &mut rep);
    drop(rep);
    finish(&mut st, r);
}

/// "3 app updates", or none. The same news keeps its time, so a dismissed
/// notice stays dismissed until something changes.
fn post_notice() {
    let dir = notify::system_dir();
    let Some(c) = Catalog::load() else {
        let _ = notify::remove(&dir, POSTER, TAG);
        return;
    };
    let installed = store::installed_apps(&c);
    let ups = store::updates(&c, &installed);
    if ups.is_empty() {
        let _ = notify::remove(&dir, POSTER, TAG);
        return;
    }
    let mut n = Notice {
        app: POSTER.into(),
        tag: TAG.into(),
        open: APP.into(),
        title: if ups.len() == 1 {
            "1 app update".into()
        } else {
            format!("{} app updates", ups.len())
        },
        body: names(&ups.iter().map(|a| a.title.as_str()).collect::<Vec<_>>()),
        count: ups.len() as u32,
        time: update::now(),
        ..Notice::default()
    };
    let old = notify::read_dir(&dir, true)
        .into_iter()
        .find(|o| o.app == POSTER && o.tag == TAG);
    if let Some(o) = old.filter(|o| o.title == n.title && o.body == n.body) {
        n.time = o.time;
    }
    if let Err(e) = notify::post(&dir, &n) {
        eprintln!("gh0st store: notice: {e}");
    }
}

/// "AcidZero, Weather and 2 more".
fn names(titles: &[&str]) -> String {
    match titles {
        [] => String::new(),
        [one] => one.to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// The update engine's words for a failure, in the Store's.
fn from_update((o, why): (UpOutcome, String)) -> Failure {
    let o = match o {
        UpOutcome::Offline => Outcome::Offline,
        UpOutcome::Clock => Outcome::Clock,
        UpOutcome::Space => Outcome::Space,
        UpOutcome::Battery | UpOutcome::BatteryKernel => Outcome::Battery,
        UpOutcome::Locked => Outcome::Locked,
        _ => Outcome::Failed,
    };
    (o, why)
}

/// apt's output on a failed install or simulation.
fn apt_failure(out: &str) -> Failure {
    let deps = [
        "Unable to satisfy dependencies",
        "Unable to locate package",
        "has no installation candidate",
        "is not installable",
        "unmet dependencies",
    ];
    let r = from_update(apt::failure(out));
    if r.0 == Outcome::Failed && deps.iter().any(|d| out.contains(d)) {
        return (Outcome::Depends, r.1);
    }
    r
}

// ------------------------------------------------------------ refresh

/// Fetches the catalog and its signature, checks them, then whatever
/// about and icons it names. All of it lands, or none: the catalog last.
fn refresh(st: &mut State, rep: &mut Reporter) -> Result<(), Failure> {
    // No route check first: curl says when it can't connect.
    if update::clock_behind() {
        return Err((Outcome::Clock, String::new()));
    }
    let site = store::site().ok_or((Outcome::Failed, "No Gh0st OS source".into()))?;
    // Inside an install it stays that app's run, so the app shows it busy.
    let item = if rep.keep_item {
        rep.p.item.clone()
    } else {
        String::new()
    };
    rep.phase(Phase::Lists, &item);
    let dir = store::state_dir();
    let tmp = dir.join(".new");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| (Outcome::Failed, e.to_string()))?;
    let r = refresh_into(st, &site, &dir, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn refresh_into(st: &mut State, site: &str, dir: &Path, tmp: &Path) -> Result<(), Failure> {
    let (cat, sig) = (tmp.join("catalog"), tmp.join("catalog.sig"));
    curl(&format!("{site}/catalog"), &cat, CATALOG_MAX)?;
    curl(&format!("{site}/catalog.sig"), &sig, SIG_MAX)?;
    let old = Catalog::load();
    let new =
        store::verify(&cat, &sig, old.as_ref(), update::now()).map_err(|o| (o, String::new()))?;
    let Some(c) = new.as_ref().or(old.as_ref()) else {
        return Err((Outcome::Failed, "no catalog".into()));
    };
    let mut landed = Vec::new();
    for (name, pin) in [("about", &c.about), ("icons", &c.icons)] {
        if pin.matches(&dir.join(name)) {
            continue;
        }
        let to = tmp.join(name);
        curl(&format!("{site}/{name}-{}", pin.sha256), &to, ART_MAX)
            .and_then(|()| pinned(pin, &to))
            // A CDN edge between versions: the next refresh gets it.
            .map_err(|(o, _)| (o, "The app list is changing. Try again soon".into()))?;
        landed.push(name);
    }
    for name in landed {
        rename(&tmp.join(name), &dir.join(name))?;
    }
    if new.is_some() {
        rename(&sig, &dir.join("catalog.sig"))?;
        rename(&cat, &store::catalog_path())?;
    }
    st.refreshed = update::now();
    Ok(())
}

fn pinned(pin: &Pinned, path: &Path) -> Result<(), Failure> {
    if pin.matches(path) {
        Ok(())
    } else {
        Err((Outcome::Checksum, String::new()))
    }
}

fn rename(from: &Path, to: &Path) -> Result<(), Failure> {
    std::fs::rename(from, to).map_err(|e| (Outcome::Failed, format!("{}: {e}", to.display())))
}

/// curl, HTTPS only, redirects too, into `to`.
fn curl(url: &str, to: &Path, max: &str) -> Result<(), Failure> {
    let status = Command::new("curl")
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--compressed",
        ])
        .args([
            "--connect-timeout",
            "20",
            "--max-time",
            "300",
            "--retry",
            "2",
        ])
        .args(["--max-filesize", max, "-o"])
        .arg(to)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|e| (Outcome::Failed, format!("curl: {e}")))?;
    curl_result(status.code())
}

fn curl_result(code: Option<i32>) -> Result<(), Failure> {
    match code {
        Some(0) => Ok(()),
        // Resolve, connect, timeout, TLS connect: the network.
        Some(6 | 7 | 28 | 35 | 56) => Err((Outcome::Offline, String::new())),
        Some(c) => Err((Outcome::Failed, format!("curl exit {c}"))),
        None => Err((Outcome::Failed, "curl stopped".into())),
    }
}

// ------------------------------------------------------------ install

/// The catalog an install may use: refreshed when a day old. A failed
/// refresh is fine while the one on disk is still valid.
fn catalog_for_install(st: &mut State) -> Result<Catalog, Failure> {
    let now = update::now();
    let c = Catalog::load();
    if c.is_none() || now.saturating_sub(st.refreshed) > CATALOG_FRESH_SECS {
        let mut rep = Reporter::at(store::progress_path(), true);
        rep.keep_item = true;
        rep.p.item = st.packages.first().cloned().unwrap_or_default();
        let r = refresh(st, &mut rep);
        drop(rep);
        let c = Catalog::load();
        if let Err(e) = r {
            if c.as_ref().is_none_or(|c| c.expires <= now) {
                return Err(e);
            }
        }
    }
    let c = Catalog::load().ok_or((Outcome::Failed, "No app list".into()))?;
    if c.expires <= now {
        return Err((Outcome::Stale, String::new()));
    }
    Ok(c)
}

fn install_job(st: &mut State, pkgs: &[String], mirror: Mirror) -> Result<(), Failure> {
    let c = catalog_for_install(st)?;
    let mut apps = Vec::new();
    for p in pkgs {
        match c.find(p) {
            Some(a) => apps.push(a.clone()),
            None => {
                st.failed = p.clone();
                return Err((Outcome::Failed, format!("{p} isn't in the app list")));
            }
        }
    }
    install(st, &apps, mirror)
}

fn upgrade(st: &mut State, mirror: Mirror) -> Result<(), Failure> {
    let c = catalog_for_install(st)?;
    let installed = store::installed_apps(&c);
    let apps: Vec<App> = store::updates(&c, &installed)
        .into_iter()
        .cloned()
        .collect();
    st.packages = apps.iter().map(|a| a.package.clone()).collect();
    install(st, &apps, mirror)
}

/// One apt transaction per app, so a failure names its app and the ones
/// before it stay installed.
fn install(st: &mut State, apps: &[App], mirror: Mirror) -> Result<(), Failure> {
    if apps.is_empty() {
        return Ok(());
    }
    update::power_ok(false).map_err(from_update)?;
    let mut rep = Reporter::at(store::progress_path(), true);
    rep.keep_item = true;
    rep.p.m = apps.len() as u32;
    let zone = store::local_zone();
    let mut lists_fresh = lists_age() < LISTS_FRESH_SECS;
    for (i, a) in apps.iter().enumerate() {
        rep.p.n = i as u32;
        let r = install_one(&mut rep, a, mirror.url(a, &zone), &mut lists_fresh);
        if let Err(e) = r {
            st.failed = a.package.clone();
            return Err(e);
        }
    }
    Ok(())
}

fn install_one(
    rep: &mut Reporter,
    a: &App,
    url: &str,
    lists_fresh: &mut bool,
) -> Result<(), Failure> {
    if STOP.load(Ordering::SeqCst) {
        return Err((Outcome::Failed, "Stopped".into()));
    }
    if !url.starts_with("https://") {
        return Err((Outcome::Failed, "not an https URL".into()));
    }
    let deb = download(rep, a, url)?;
    let r = install_deb(rep, a, &deb, lists_fresh);
    let _ = std::fs::remove_file(&deb);
    r
}

/// The deb into the cache, named by its SHA-256, and checked against the
/// catalog before apt sees it. A part left by an earlier run resumes.
fn download(rep: &mut Reporter, a: &App, url: &str) -> Result<PathBuf, Failure> {
    let dir = store::cache_dir();
    create_private(&dir)?;
    let part = dir.join(format!("{}.part", a.sha256));
    let deb = dir.join(format!("{}.deb", a.sha256));
    let have = std::fs::metadata(&part).map_or(0, |m| m.len());
    if have > a.size {
        let _ = std::fs::remove_file(&part);
    }
    free_for(a.size.saturating_sub(have), &dir)?;
    rep.p.phase = Phase::Download;
    rep.p.item = a.package.clone();
    rep.p.total = a.size;
    rep.p.done = have.min(a.size);
    rep.p.percent = Some(percent(rep.p.done, a.size));
    rep.write(true);
    if have != a.size {
        fetch_deb(rep, a, url, &part)?;
    }
    if store::file_sha256(&part).as_deref() != Some(a.sha256.as_str()) {
        let _ = std::fs::remove_file(&part);
        return Err((Outcome::Checksum, String::new()));
    }
    // The bytes match the catalog; the control file must agree too, or
    // apt would install some other package or version.
    let f = update::run(&["dpkg-deb", "-f", &path_str(&part), "Package", "Version"])
        .map_err(|e| (Outcome::Checksum, e))?;
    let (name, version) = control(&f);
    if name != a.package || version != a.version {
        let _ = std::fs::remove_file(&part);
        return Err((Outcome::Checksum, format!("the deb is {name} {version}")));
    }
    rename(&part, &deb)?;
    Ok(deb)
}

/// curl as a child, its file's size sampled for the percent.
fn fetch_deb(rep: &mut Reporter, a: &App, url: &str, part: &Path) -> Result<(), Failure> {
    let mut child = Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "20", "--retry", "2", "-C", "-"])
        .args(["--speed-limit", "1000", "--speed-time", "60"])
        .args(["--max-filesize", &a.size.to_string(), "-o"])
        .arg(part)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| (Outcome::Failed, format!("curl: {e}")))?;
    let code = loop {
        if let Ok(Some(s)) = child.try_wait() {
            break s.code();
        }
        if STOP.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err((Outcome::Failed, "Stopped".into()));
        }
        rep.p.done = std::fs::metadata(part).map_or(0, |m| m.len()).min(a.size);
        rep.p.percent = Some(percent(rep.p.done, a.size));
        rep.write(false);
        std::thread::sleep(std::time::Duration::from_millis(SAMPLE_MS));
    };
    rep.p.done = a.size;
    rep.p.percent = Some(100);
    rep.write(true);
    curl_result(code)
}

fn install_deb(
    rep: &mut Reporter,
    a: &App,
    deb: &Path,
    lists_fresh: &mut bool,
) -> Result<(), Failure> {
    let local = path_str(deb);
    // apt reads its lists for some seconds before it says anything: a
    // moving bar, not a percent stuck at 0.
    rep.phase(Phase::Lists, &a.package);
    let sim = simulate(&local).or_else(|e| {
        // A dependency apt can't find may only be missing from old lists.
        if *lists_fresh {
            return Err(e);
        }
        rep.phase(Phase::Lists, &a.package);
        rep.p.item = a.package.clone();
        refresh_lists(rep)?;
        *lists_fresh = true;
        simulate(&local)
    })?;
    // --no-remove refuses too, but the reason reads better from here.
    if let Some(other) = removes(&sim).into_iter().next() {
        return Err((Outcome::Failed, format!("It would remove {other}")));
    }
    let deps = space_for(&sim, a)?;
    if update::dpkg_interrupted() {
        let _ = update::configure_pending(rep);
    }
    // What apt downloads now is the dependencies: their size, not the app's.
    rep.p.phase = Phase::Install;
    rep.p.percent = Some(0);
    rep.p.item = a.package.clone();
    rep.p.total = deps;
    rep.p.done = 0;
    rep.write(true);
    let (code, out) = update::apt(
        "apt-get",
        &[
            "install",
            "-y",
            "-q",
            "--no-remove",
            "-o",
            "APT::Status-Fd=3",
            "-o",
            "Dpkg::Options::=--force-confdef",
            "-o",
            "Dpkg::Options::=--force-confold",
            "-o",
            "DPkg::Lock::Timeout=120",
            &local,
        ],
        rep,
        until_dpkg(),
    );
    if code != Some(0) {
        return Err(apt_failure(&out));
    }
    // SAFETY: sync has no failure mode.
    unsafe { libc::sync() };
    Ok(())
}

/// SIGTERM may stop apt until dpkg starts; past that it finishes.
fn until_dpkg() -> impl FnMut(&apt::Status) -> bool {
    let mut dpkg = false;
    move |s| {
        dpkg |= matches!(s, apt::Status::Pm { .. });
        !dpkg
    }
}

fn simulate(local: &str) -> Result<String, Failure> {
    let out = Command::new("apt-get")
        .args(["-s", "-q", "install", "--no-remove", local])
        .env("LC_ALL", "C.UTF-8")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| (Outcome::Failed, format!("apt-get: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(apt_failure(&format!("{text}\n{err}")));
    }
    Ok(text)
}

fn refresh_lists(rep: &mut Reporter) -> Result<(), Failure> {
    let (code, out) = update::apt(
        "apt-get",
        &["update", "-q", "-o", "APT::Status-Fd=3"],
        rep,
        |_| true,
    );
    if code != Some(0) {
        return Err(apt_failure(&out));
    }
    Ok(())
}

/// Seconds since apt last fetched its lists.
fn lists_age() -> u64 {
    let newest = std::fs::read_dir("/var/lib/apt/lists")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with("InRelease"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max();
    newest
        .and_then(|t| t.elapsed().ok())
        .map_or(u64::MAX, |d| d.as_secs())
}

/// The packages a simulation would remove.
fn removes(sim: &str) -> Vec<String> {
    sim.lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|r| r.split_whitespace().next())
        .map(String::from)
        .collect()
}

/// Room for the app and what apt brings with it: the download, then each
/// package unpacked. Returns what the dependencies download.
fn space_for(sim: &str, a: &App) -> Result<u64, Failure> {
    let (insts, _) = apt::simulate(sim);
    let names: Vec<String> = insts
        .iter()
        .filter(|i| i.name != a.package)
        .map(|i| format!("{}={}", i.name, i.to))
        .collect();
    let mut need = SPARE_BYTES + a.size * 3;
    let mut download = 0;
    if !names.is_empty() {
        let mut argv = vec!["apt-cache", "show", "--no-all-versions"];
        argv.extend(names.iter().map(String::as_str));
        for r in apt::records(&update::run(&argv).unwrap_or_default()) {
            need += r.size + r.installed_kb * 1024;
            download += r.size;
        }
    }
    let free = update::free_bytes("/var/cache/apt/archives").min(update::free_bytes("/"));
    if free < need {
        return Err((
            Outcome::Space,
            format!(
                "Needs {} free, {} is",
                gh0st_core::update::size_text(need),
                gh0st_core::update::size_text(free)
            ),
        ));
    }
    Ok(download)
}

fn free_for(bytes: u64, dir: &Path) -> Result<(), Failure> {
    let free = update::free_bytes(&path_str(dir));
    if free < bytes + SPARE_BYTES {
        return Err((Outcome::Space, String::new()));
    }
    Ok(())
}

/// The cache holds debs on their way in: root's alone.
fn create_private(dir: &Path) -> Result<(), Failure> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| (Outcome::Failed, format!("{}: {e}", dir.display())))?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    Ok(())
}

/// `dpkg-deb -f deb Package Version` into its two values.
fn control(text: &str) -> (String, String) {
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix(':'))
            .map_or(String::new(), |v| v.trim().to_string())
    };
    (field("Package"), field("Version"))
}

fn percent(done: u64, total: u64) -> u8 {
    (done * 100).checked_div(total).unwrap_or(0).min(100) as u8
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

// ------------------------------------------------------------ remove

/// Removes an app the catalog lists, and only it: when apt would take
/// anything else with it, nothing happens.
fn remove(pkg: &str) -> Result<(), Failure> {
    let c = Catalog::load().ok_or((Outcome::Failed, "No app list".into()))?;
    if c.find(pkg).is_none() {
        return Err((Outcome::Failed, format!("{pkg} isn't in the app list")));
    }
    if !store::installed_apps(&c).contains_key(pkg) {
        return Ok(());
    }
    let sim =
        update::run(&["apt-get", "-s", "-q", "remove", pkg]).map_err(|e| (Outcome::Failed, e))?;
    if let Some(other) = removes(&sim).into_iter().find(|n| n != pkg) {
        return Err((Outcome::Failed, format!("It would remove {other} too")));
    }
    let mut rep = Reporter::at(store::progress_path(), true);
    rep.keep_item = true;
    rep.phase(Phase::Install, pkg);
    let (code, out) = update::apt(
        "apt-get",
        &[
            "remove",
            "-y",
            "-q",
            "-o",
            "APT::Status-Fd=3",
            "-o",
            "DPkg::Lock::Timeout=120",
            pkg,
        ],
        &mut rep,
        until_dpkg(),
    );
    if code != Some(0) {
        return Err(apt_failure(&out));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simulation_names_what_it_would_remove() {
        let sim = "Inst foo (1 Debian:13/stable [arm64])\nRemv bar [2]\nConf foo (1)\n";
        assert_eq!(removes(sim), ["bar"]);
    }

    #[test]
    fn missing_dependencies_read_as_depends() {
        let out = "E: The following information from --solver 3.0 may provide additional context:\n   Unable to satisfy dependencies. Reached two conflicting decisions:";
        assert_eq!(apt_failure(out).0, Outcome::Depends);
        let out = "E: Unable to locate package nope";
        assert_eq!(apt_failure(out).0, Outcome::Depends);
        let out = "E: Failed to fetch x  Temporary failure resolving 'deb.debian.org'";
        assert_eq!(apt_failure(out).0, Outcome::Offline);
    }

    #[test]
    fn curl_exits_read_as_offline_or_failed() {
        assert_eq!(curl_result(Some(0)), Ok(()));
        assert_eq!(curl_result(Some(6)).unwrap_err().0, Outcome::Offline);
        assert_eq!(curl_result(Some(22)).unwrap_err().0, Outcome::Failed);
    }

    #[test]
    fn a_notice_names_two_apps_then_counts() {
        assert_eq!(names(&["A"]), "A");
        assert_eq!(names(&["A", "B"]), "A and B");
        assert_eq!(names(&["A", "B", "C", "D"]), "A, B and 2 more");
    }

    #[test]
    fn control_fields_read_back() {
        assert_eq!(
            control("Package: acidzero\nVersion: 0.2.0\n"),
            ("acidzero".to_string(), "0.2.0".to_string())
        );
    }
}
