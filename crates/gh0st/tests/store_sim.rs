//! The Store's engine end to end: the real `gh0st store job` with a fake
//! curl that serves a folder, a fake sqv that says yes or no, and a fake
//! apt-get that logs. Nothing on the host changes. Real installs as root
//! are checked in the build container (docs/store.md).

use gh0st_core::store::{Catalog, Outcome, State};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// Copies `$SITE/<name>` to the `-o` file, as curl -f would; exit 22 when
/// the site lacks it, 7 when `$FAKE/offline` exists.
const CURL: &str = r#"#!/bin/sh
echo "curl $*" >> "$FAKE/log"
[ -e "$GH0ST_STORE_RUN/progress" ] && cp "$GH0ST_STORE_RUN/progress" "$FAKE/progress-seen"
[ -e "$FAKE/offline" ] && exit 7
out=; url=; next=
for a in "$@"; do
    if [ "$next" = o ]; then out=$a; next=; continue; fi
    if [ "$next" = url ]; then url=$a; continue; fi
    case $a in -o) next=o ;; --) next=url ;; esac
done
name=${url##*/}
[ -f "$SITE/$name" ] || exit 22
cp "$SITE/$name" "$out"
"#;

/// Yes, unless `$FAKE/bad_sig` exists.
const SQV: &str = r#"#!/bin/sh
echo "sqv $*" >> "$FAKE/log"
[ -e "$FAKE/bad_sig" ] && exit 1
exit 0
"#;

const APT_GET: &str = r#"#!/bin/sh
echo "apt-get $*" >> "$FAKE/log"
exit 0
"#;

struct Sim {
    dir: PathBuf,
}

fn sha256(data: &[u8]) -> String {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("gh0st-store-sha-{}-{n}", std::process::id()));
    std::fs::write(&p, data).unwrap();
    let h = gh0st_core::store::file_sha256(&p).unwrap();
    std::fs::remove_file(&p).unwrap();
    h
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

impl Sim {
    fn new(name: &str) -> Sim {
        let dir = std::env::temp_dir().join(format!("gh0st-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["bin", "site", "state", "run", "cache", "urun", "notify"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for (n, body) in [("curl", CURL), ("sqv", SQV), ("apt-get", APT_GET)] {
            let p = dir.join("bin").join(n);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(dir.join("status"), "").unwrap();
        Sim { dir }
    }

    /// Publishes a catalog dated `date` with one app, and its about and
    /// icons under their hashes.
    fn publish(&self, date: u64, about: &str) -> String {
        let about = about.as_bytes();
        let icons = b"gh0st-icons 1\n\n";
        let site = self.dir.join("site");
        let (ah, ih) = (sha256(about), sha256(icons));
        std::fs::write(site.join(format!("about-{ah}")), about).unwrap();
        std::fs::write(site.join(format!("icons-{ih}")), icons).unwrap();
        let text = format!(
            "gh0st-store 1\ndate {date}\nexpires {}\nabout {ah} {}\nicons {ih} {}\n\
             app\tacidzero\t0.2.0\t58940\t{}\tGames\t-\tAcidZero\tSynth\tme\tMIT\t\
             https://github.com/x/a.deb\t\tACID\tacidzero\n",
            date + 30 * 86_400,
            about.len(),
            icons.len(),
            "e".repeat(64)
        );
        std::fs::write(site.join("catalog"), &text).unwrap();
        std::fs::write(site.join("catalog.sig"), b"sig").unwrap();
        text
    }

    fn set(&self, what: &str, on: bool) {
        let p = self.dir.join(what);
        if on {
            std::fs::write(p, b"").unwrap();
        } else {
            let _ = std::fs::remove_file(p);
        }
    }

    fn job(&self, job: &str) -> State {
        std::fs::write(self.dir.join("run/job"), job).unwrap();
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["store", "job"])
            .env("PATH", path)
            .env("FAKE", &self.dir)
            .env("SITE", self.dir.join("site"))
            .env("GH0ST_STORE_URL", "https://store.example/store")
            .env("GH0ST_STORE_STATE", self.dir.join("state"))
            .env("GH0ST_STORE_RUN", self.dir.join("run"))
            .env("GH0ST_STORE_CACHE", self.dir.join("cache"))
            .env("GH0ST_UPDATE_RUN", self.dir.join("urun"))
            .env("GH0ST_POWER_SUPPLY", self.dir.join("power"))
            .env("GH0ST_NOTIFY_SYSTEM", self.dir.join("notify"))
            .env("GH0ST_DPKG_STATUS", self.dir.join("status"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !self.dir.join("run/job").exists(),
            "the job is gone once read"
        );
        State::parse(&std::fs::read_to_string(self.dir.join("state/state")).unwrap())
    }

    fn catalog(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join("state/catalog")).ok()
    }

    fn about(&self) -> Option<String> {
        std::fs::read_to_string(self.dir.join("state/about")).ok()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_refresh_lands_the_catalog_and_what_it_names() {
    let sim = Sim::new("refresh");
    let text = sim.publish(now() - 60, "acidzero\tA synth.\n");
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!((st.op.as_str(), st.outcome), ("refresh", Outcome::Ok));
    assert!(st.refreshed > 0 && st.ended > 0);
    assert_eq!(sim.catalog().as_deref(), Some(text.as_str()));
    assert_eq!(sim.about().as_deref(), Some("acidzero\tA synth.\n"));
    assert!(sim.dir.join("state/catalog.sig").exists());
    assert!(
        sim.log().contains("--proto =https") && sim.log().contains("sqv --keyring"),
        "{}",
        sim.log()
    );
    assert!(
        !sim.dir.join("run/progress").exists(),
        "progress goes at the end"
    );

    // The same catalog again: nothing to fetch past the catalog.
    std::fs::write(sim.dir.join("log"), "").unwrap();
    let again = sim.job("refresh\nmirror auto\n");
    assert_eq!(again.outcome, Outcome::Ok);
    assert!(again.ended >= st.ended);
    assert!(!sim.log().contains("about-"), "{}", sim.log());
}

#[test]
fn an_update_posts_one_notice_and_it_goes_when_none_are_left() {
    let sim = Sim::new("notice");
    std::fs::write(
        sim.dir.join("status"),
        "Package: acidzero\nStatus: install ok installed\nVersion: 0.1.0\n\n",
    )
    .unwrap();
    sim.publish(now() - 60, "acidzero\tA synth.\n");
    sim.job("refresh\nmirror auto\n");
    let notice = std::fs::read_to_string(sim.dir.join("notify/gh0st-store.apps")).unwrap();
    assert!(notice.contains("1 app update"), "{notice}");
    assert!(notice.contains("AcidZero"), "{notice}");
    std::fs::write(
        sim.dir.join("status"),
        "Package: acidzero\nStatus: install ok installed\nVersion: 0.2.0\n\n",
    )
    .unwrap();
    sim.job("refresh\nmirror auto\n");
    assert!(!sim.dir.join("notify/gh0st-store.apps").exists());
}

#[test]
fn a_bad_signature_changes_nothing() {
    let sim = Sim::new("badsig");
    let old = sim.publish(now() - 600, "acidzero\tOld.\n");
    sim.job("refresh\nmirror auto\n");
    sim.publish(now() - 60, "acidzero\tNew.\n");
    sim.set("bad_sig", true);
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::BadSignature);
    assert_eq!(sim.catalog().as_deref(), Some(old.as_str()));
    assert_eq!(sim.about().as_deref(), Some("acidzero\tOld.\n"));
}

#[test]
fn a_catalog_from_the_future_blames_the_clock() {
    let sim = Sim::new("clock");
    sim.publish(now() + 7 * 86_400, "acidzero\tA synth.\n");
    // sqv refuses a signature dated after the clock.
    sim.set("bad_sig", true);
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::Clock);
    assert!(sim.catalog().is_none());
}

#[test]
fn an_older_catalog_is_not_an_error_and_lands_nothing() {
    let sim = Sim::new("older");
    let new = sim.publish(now() - 60, "acidzero\tNew.\n");
    sim.job("refresh\nmirror auto\n");
    sim.publish(now() - 6000, "acidzero\tOld.\n");
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::Ok);
    assert_eq!(sim.catalog().as_deref(), Some(new.as_str()));
    assert_eq!(sim.about().as_deref(), Some("acidzero\tNew.\n"));
}

#[test]
fn a_missing_about_keeps_the_old_catalog() {
    let sim = Sim::new("half");
    let old = sim.publish(now() - 600, "acidzero\tOld.\n");
    sim.job("refresh\nmirror auto\n");
    sim.publish(now() - 60, "acidzero\tNew.\n");
    for e in std::fs::read_dir(sim.dir.join("site")).unwrap().flatten() {
        if e.file_name().to_string_lossy().starts_with("about-") {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::Failed);
    assert_eq!(sim.catalog().as_deref(), Some(old.as_str()));
    assert_eq!(sim.about().as_deref(), Some("acidzero\tOld.\n"));
}

#[test]
fn offline_says_so() {
    let sim = Sim::new("offline");
    sim.set("offline", true);
    let st = sim.job("refresh\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::Offline);
}

#[test]
fn a_deb_that_doesnt_match_never_reaches_apt() {
    let sim = Sim::new("checksum");
    sim.publish(now() - 60, "acidzero\tA synth.\n");
    sim.job("refresh\nmirror auto\n");
    assert!(Catalog::parse(&sim.catalog().unwrap()).is_ok());
    // The site serves a.deb, but not the bytes the catalog pins.
    std::fs::write(sim.dir.join("site/a.deb"), vec![0u8; 58_940]).unwrap();
    let st = sim.job("install acidzero\nmirror github\n");
    assert_eq!(st.outcome, Outcome::Checksum, "{}", st.detail);
    assert_eq!(st.failed, "acidzero");
    assert!(!sim.log().contains("apt-get"), "{}", sim.log());
    let left: Vec<_> = std::fs::read_dir(sim.dir.join("cache")).unwrap().collect();
    assert!(left.is_empty(), "a bad deb is deleted");
}

#[test]
fn an_app_that_changed_since_the_user_looked_is_not_installed() {
    let sim = Sim::new("seen");
    sim.publish(now() - 60, "acidzero\tA synth.\n");
    sim.job("refresh\nmirror auto\n");
    // The user saw 0.1.0; the list now has 0.2.0.
    let st = sim.job("install acidzero=0.1.0\nmirror github\n");
    assert_eq!(st.outcome, Outcome::Failed, "{}", st.detail);
    assert_eq!(st.failed, "acidzero");
    assert_eq!(st.packages, ["acidzero"]);
    assert!(!sim.log().contains("apt-get"), "{}", sim.log());
    // The version shown passes the check (the sim has no deb to fetch).
    let st = sim.job("install acidzero=0.2.0\nmirror github\n");
    assert!(!st.detail.contains("changed"), "{}", st.detail);
    assert!(sim.log().contains("curl"), "{}", sim.log());
}

#[test]
fn an_installs_own_refresh_shows_as_that_install() {
    let sim = Sim::new("stale");
    sim.publish(now() - 60, "acidzero\tA synth.\n");
    sim.job("refresh\nmirror auto\n");
    // A day later the install refreshes the list first.
    let st = std::fs::read_to_string(sim.dir.join("state/state")).unwrap();
    let st = st.replace(
        &format!("refreshed={}", State::parse(&st).refreshed),
        "refreshed=1",
    );
    std::fs::write(sim.dir.join("state/state"), st).unwrap();
    sim.job("install acidzero\nmirror github\n");
    let seen = std::fs::read_to_string(sim.dir.join("progress-seen")).unwrap();
    let p = gh0st_core::update::Progress::parse(&seen).unwrap();
    assert!(p.install, "{seen}");
    assert_eq!(p.item, "acidzero", "{seen}");
}

#[test]
fn a_held_lock_reads_as_locked() {
    let sim = Sim::new("locked");
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(sim.dir.join("urun/lock"))
        .unwrap();
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a file this test owns.
    assert_eq!(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) }, 0);
    let st = sim.job("install acidzero\nmirror auto\n");
    assert_eq!(st.outcome, Outcome::Locked);
    assert_eq!(st.packages, ["acidzero"]);
}
