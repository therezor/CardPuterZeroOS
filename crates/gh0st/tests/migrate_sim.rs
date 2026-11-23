//! One-time migrations end to end: the real `gh0st update install` with
//! fake apt-get, apt-cache, dpkg-query and dpkg first on PATH. Nothing on
//! the host changes.

use gh0st_core::update::{Group, Outcome, Package, Restart, State};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// Logs each call. `upgradable` holds what an upgrade brings. Each package
/// a migration names needs `lib<name>` too, as apt would pull it. `-s`
/// prints the Inst lines; the real run adds the names to `installed`.
/// `sim_fail` and `install_fail` make an install fail as apt does.
const APT_GET: &str = r#"#!/bin/sh
echo "apt-get $*" >> "$FAKE/log"
sim=; verb=; names=
for a in "$@"; do
    case $a in
        -s) sim=1 ;;
        -*|*=*) ;;
        install|upgrade|clean|update|remove) [ -z "$verb" ] && verb=$a ;;
        *) case $verb in install|remove) names="$names $a" ;; esac ;;
    esac
done
has() { grep -qx "$1" "$FAKE/installed" 2>/dev/null; }
if [ "$verb" = upgrade ]; then
    for n in $(cat "$FAKE/upgradable" 2>/dev/null); do
        if [ -n "$sim" ]; then
            echo "Inst $n [1] (2 Debian:13/stable [arm64])"
        else
            echo "$n" >> "$FAKE/installed"
        fi
    done
    [ -n "$sim" ] || : > "$FAKE/upgradable"
    exit 0
fi
if [ "$verb" = remove ]; then
    for n in $names; do
        if [ -n "$sim" ]; then
            echo "Remv $n [1]"
            [ -e "$FAKE/remove_more" ] && echo "Remv lib$n [1]"
        else
            grep -vx "$n" "$FAKE/installed" > "$FAKE/installed.new"
            mv "$FAKE/installed.new" "$FAKE/installed"
        fi
    done
    exit 0
fi
[ "$verb" = install ] || exit 0
all=
for n in $names; do
    has "$n" || all="$all $n"
    has "lib$n" || all="$all lib$n"
done
if [ -n "$sim" ]; then
    if [ -e "$FAKE/sim_fail" ]; then
        echo "E: Unable to locate package nope" >&2
        exit 100
    fi
    for n in $all; do echo "Inst $n (1.0 Debian:13/stable [arm64])"; done
    exit 0
fi
if [ -e "$FAKE/install_fail" ]; then
    echo "E: Failed to fetch: Temporary failure resolving 'deb.debian.org'" >&2
    exit 100
fi
for n in $all; do echo "$n" >> "$FAKE/installed"; done
"#;

/// Answers from `installed`, failing on an unknown name as dpkg-query does.
const DPKG_QUERY: &str = r#"#!/bin/sh
fmt=$3; shift 3; code=0
for n in "$@"; do
    if grep -qx "$n" "$FAKE/installed" 2>/dev/null; then
        case $fmt in
            *Installed-Size*) printf '%s\t100\n' "$n" ;;
            *) printf '%s\tinstalled\n' "$n" ;;
        esac
    else
        echo "dpkg-query: no packages found matching $n" >&2
        code=1
    fi
done
exit $code
"#;

const OK: &str = "#!/bin/sh\nexit 0\n";

struct Sim {
    dir: PathBuf,
}

impl Sim {
    /// A device with `installed` and one migration wanting `wants`, and one
    /// update waiting.
    fn new(name: &str, installed: &[&str], wants: &str) -> Sim {
        let dir = std::env::temp_dir().join(format!("gh0st-migrate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["bin", "state", "run", "notify", "migrations", "power"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for (n, body) in [
            ("apt-get", APT_GET),
            ("dpkg-query", DPKG_QUERY),
            ("apt-cache", OK),
            ("dpkg", OK),
        ] {
            let p = dir.join("bin").join(n);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut list = installed.join("\n");
        list.push('\n');
        std::fs::write(dir.join("installed"), list).unwrap();
        std::fs::write(
            dir.join("migrations/0001-tools"),
            format!("# tools\ninstall {wants}\n"),
        )
        .unwrap();
        let sim = Sim { dir };
        sim.queue(&[]);
        sim
    }

    /// One update waiting, from a check a moment ago that also listed
    /// `listed`, as a check lists what migrations add.
    fn queue(&self, listed: &[&str]) {
        let pkg = |name: &str| Package {
            name: name.into(),
            from: "1".into(),
            to: "2".into(),
            size: 1000,
            group: Group::System,
            restart: Restart::None,
            summary: String::new(),
        };
        let st = State {
            checked: now(),
            packages: std::iter::once("tzdata")
                .chain(listed.iter().copied())
                .map(pkg)
                .collect(),
            ..State::default()
        };
        std::fs::write(self.dir.join("state/state"), st.text()).unwrap();
        std::fs::write(self.dir.join("upgradable"), "tzdata\n").unwrap();
    }

    fn fail(&self, what: &str) {
        std::fs::write(self.dir.join(what), b"").unwrap();
    }

    fn install(&self) -> State {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["update", "install"])
            .env("PATH", path)
            .env("FAKE", &self.dir)
            .env("GH0ST_UPDATE_STATE", self.dir.join("state"))
            .env("GH0ST_UPDATE_RUN", self.dir.join("run"))
            .env("GH0ST_NOTIFY_SYSTEM", self.dir.join("notify"))
            .env("GH0ST_MIGRATIONS", self.dir.join("migrations"))
            // No battery, as on a bench supply: the host's own charge
            // never decides.
            .env("GH0ST_POWER_SUPPLY", self.dir.join("power"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        State::read(&self.dir.join("state/state"))
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }

    /// The real installs a migration asked for.
    fn installs(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter(|l| l.contains(" install ") && !l.contains(" -s "))
            .map(|l| l.rsplit(" install ").next().unwrap_or("").to_string())
            .collect()
    }

    fn done(&self) -> String {
        std::fs::read_to_string(self.dir.join("state/migrations")).unwrap_or_default()
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn a_migration_installs_what_is_missing_once() {
    let sim = Sim::new("once", &["curl"], "curl wget jq");
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    assert_eq!(sim.installs(), ["wget jq"]);
    assert_eq!(sim.done(), "0001-tools\n");
    assert_eq!(
        st.installed_count, 5,
        "the update, the two tools and their libraries"
    );
    assert!(st.packages.is_empty(), "{:?}", st.packages);

    // The user removes one. The next install leaves it removed.
    std::fs::write(sim.dir.join("installed"), "curl\nwget\n").unwrap();
    sim.queue(&[]);
    let st = sim.install();
    assert_eq!(st.installed_count, 1, "{}", st.why);
    assert_eq!(sim.installs().len(), 1, "{}", sim.log());
}

#[test]
fn a_removal_takes_the_package_off_once() {
    let sim = Sim::new("remove", &["curl", "gh0st-m5-appstore"], "curl");
    std::fs::write(
        sim.dir.join("migrations/0002-store"),
        "# ours replaces it\nremove gh0st-m5-appstore\n",
    )
    .unwrap();
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    let installed = std::fs::read_to_string(sim.dir.join("installed")).unwrap();
    assert!(!installed.contains("gh0st-m5-appstore"), "{installed}");
    assert_eq!(sim.done(), "0001-tools\n0002-store\n");
}

#[test]
fn a_removal_that_would_take_more_waits() {
    let sim = Sim::new("remove-more", &["curl", "gh0st-m5-appstore"], "curl");
    std::fs::write(
        sim.dir.join("migrations/0002-store"),
        "remove gh0st-m5-appstore\n",
    )
    .unwrap();
    sim.fail("remove_more");
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    let installed = std::fs::read_to_string(sim.dir.join("installed")).unwrap();
    assert!(installed.contains("gh0st-m5-appstore"), "{installed}");
    assert_eq!(sim.done(), "0001-tools\n");
}

#[test]
fn a_failed_migration_stays_pending_and_the_update_still_counts() {
    let sim = Sim::new("fail", &[], "wget");
    sim.fail("install_fail");
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Offline);
    assert_eq!(st.installed_count, 1, "the upgrade went in");
    assert_eq!(sim.done(), "");

    std::fs::remove_file(sim.dir.join("install_fail")).unwrap();
    sim.queue(&[]);
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    assert_eq!(sim.done(), "0001-tools\n");
}

#[test]
fn a_migration_apt_cannot_place_waits_without_failing() {
    let sim = Sim::new("unknown", &[], "nope");
    sim.fail("sim_fail");
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    assert!(sim.installs().is_empty(), "{}", sim.log());
    assert_eq!(sim.done(), "");
}

#[test]
fn a_device_that_has_everything_marks_it_done_without_apt() {
    let sim = Sim::new("has", &["wget"], "wget");
    sim.install();
    assert!(sim.installs().is_empty(), "{}", sim.log());
    assert_eq!(sim.done(), "0001-tools\n");
}

/// The check lists a migration's packages and the libraries they pull. The
/// count is what went in, each once.
#[test]
fn what_the_check_listed_counts_once() {
    let sim = Sim::new("listed", &[], "wget");
    sim.queue(&["wget", "libwget"]);
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    assert_eq!(st.installed_count, 3, "tzdata, wget and libwget");
    assert_eq!(sim.installs(), ["wget"]);
}
