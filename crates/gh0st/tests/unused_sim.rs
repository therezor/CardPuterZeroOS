//! Unused packages end to end: the real `gh0st update autoremove` and
//! `install` with fake
//! apt-get, apt-cache, dpkg-query and dpkg first on PATH. Nothing on the
//! host changes.

use gh0st_core::update::{Group, Outcome, Package, Restart, State, Unused};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

/// Logs each call. `installed` is what dpkg has, `unneeded` what nothing
/// needs. autoremove lists each unneeded package no
/// `APT::NeverAutoRemove` pattern keeps; a real remove takes it off both.
/// `remove_more` makes a remove take a library with it.
const APT_GET: &str = r#"#!/bin/sh
echo "apt-get $*" >> "$FAKE/log"
sim=; verb=; names=; keep=
for a in "$@"; do
    case $a in
        -s) sim=1 ;;
        APT::NeverAutoRemove::=*) keep="$keep ${a#APT::NeverAutoRemove::=}" ;;
        -*|*=*) ;;
        install|upgrade|clean|update|remove|autoremove) [ -z "$verb" ] && verb=$a ;;
        *) [ "$verb" = remove ] && names="$names $a" ;;
    esac
done
has() { grep -qx "$1" "$FAKE/installed" 2>/dev/null; }
drop() { grep -vx "$1" "$FAKE/$2" > "$FAKE/$2.new"; mv "$FAKE/$2.new" "$FAKE/$2"; }
case $verb in
    autoremove)
        for n in $(cat "$FAKE/unneeded" 2>/dev/null); do
            kept=
            for p in $keep; do echo "$n" | grep -Eq "$p" && kept=1; done
            [ -z "$kept" ] && has "$n" && echo "Remv $n [1.0]"
        done ;;
    remove)
        for n in $names; do
            if [ -n "$sim" ]; then
                echo "Remv $n [1.0]"
                [ -e "$FAKE/remove_more" ] && echo "Remv libgl1 [1.0]"
            else
                drop "$n" installed; drop "$n" unneeded
            fi
        done ;;
    upgrade)
        for n in $(cat "$FAKE/upgradable" 2>/dev/null); do
            [ -n "$sim" ] && echo "Inst $n [1] (2 Debian:13/stable [arm64])"
        done
        [ -n "$sim" ] || : > "$FAKE/upgradable"
        # The upgrade leaves one more library unneeded.
        [ -n "$sim" ] || { echo libnew >> "$FAKE/installed"; echo libnew >> "$FAKE/unneeded"; } ;;
esac
exit 0
"#;

/// Answers from `installed`, failing on an unknown name as dpkg-query does.
const DPKG_QUERY: &str = r#"#!/bin/sh
fmt=$3; shift 3; code=0
for n in "$@"; do
    if grep -qx "$n" "$FAKE/installed" 2>/dev/null; then
        case $fmt in
            *Summary*) printf '%s\t2048\tthe %s library\n' "$n" "$n" ;;
            *Installed-Size*) printf '%s\t100\n' "$n" ;;
            *) printf '%s\tinstalled\n' "$n" ;;
        esac
    else
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
    /// A device with `installed`, of which `unneeded` nothing needs, and
    /// one update waiting from a check that showed `seen` as unused.
    fn new(name: &str, installed: &[&str], unneeded: &[&str], seen: &[&str]) -> Sim {
        let dir = std::env::temp_dir().join(format!("gh0st-unused-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in [
            "bin",
            "state",
            "run",
            "notify",
            "migrations",
            "power",
            "store",
        ] {
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
        let lines = |v: &[&str]| v.iter().map(|n| format!("{n}\n")).collect::<String>();
        std::fs::write(dir.join("installed"), lines(installed)).unwrap();
        std::fs::write(dir.join("unneeded"), lines(unneeded)).unwrap();
        std::fs::write(dir.join("upgradable"), "tzdata\n").unwrap();
        std::fs::write(dir.join("status"), "").unwrap();
        let st = State {
            checked: now(),
            packages: vec![Package {
                name: "tzdata".into(),
                from: "1".into(),
                to: "2".into(),
                size: 1000,
                group: Group::System,
                restart: Restart::None,
                summary: String::new(),
            }],
            unused: seen
                .iter()
                .map(|n| Unused {
                    name: n.to_string(),
                    version: "1.0".into(),
                    ..Unused::default()
                })
                .collect(),
            ..State::default()
        };
        std::fs::write(dir.join("state/state"), st.text()).unwrap();
        Sim { dir }
    }

    /// A Store app installed whose catalog line names `extras`.
    fn store_app(&self, extras: &str) {
        let line = format!(
            "app\tdemo\t1.0\t10\t{}\tGames\t-\tDemo\tA demo\tme\tMIT\thttps://x/d.deb\t\t\tdemo\t{extras}\n",
            "e".repeat(64)
        );
        let cat = format!("gh0st-store 1\ndate 1\nexpires 9999999999\n{line}");
        std::fs::write(self.dir.join("store/catalog"), cat).unwrap();
        std::fs::write(
            self.dir.join("status"),
            "Package: demo\nStatus: install ok installed\nVersion: 1.0\n\n",
        )
        .unwrap();
    }

    fn install(&self) -> State {
        self.run("install")
    }

    fn autoremove(&self) -> State {
        self.run("autoremove")
    }

    fn run(&self, verb: &str) -> State {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["update", verb])
            .env("PATH", path)
            .env("FAKE", &self.dir)
            .env("GH0ST_UPDATE_STATE", self.dir.join("state"))
            .env("GH0ST_UPDATE_RUN", self.dir.join("run"))
            .env("GH0ST_NOTIFY_SYSTEM", self.dir.join("notify"))
            .env("GH0ST_MIGRATIONS", self.dir.join("migrations"))
            .env("GH0ST_POWER_SUPPLY", self.dir.join("power"))
            .env("GH0ST_STORE_STATE", self.dir.join("store"))
            .env("GH0ST_DPKG_STATUS", self.dir.join("status"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        State::read(&self.dir.join("state/state"))
    }

    fn installed(&self) -> String {
        std::fs::read_to_string(self.dir.join("installed")).unwrap()
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

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn names(st: &State) -> Vec<&str> {
    st.unused.iter().map(|u| u.name.as_str()).collect()
}

#[test]
fn an_install_leaves_unused_packages_alone() {
    let sim = Sim::new("install", &["tzdata", "liba"], &["liba"], &["liba"]);
    let st = sim.install();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    assert!(sim.installed().contains("liba\n"), "{}", sim.log());
    assert_eq!(names(&st), ["liba", "libnew"], "listed for the menu");
}

#[test]
fn remove_unused_takes_what_the_check_showed_and_lists_what_is_left() {
    let sim = Sim::new(
        "seen",
        &["tzdata", "liba", "libb", "libc"],
        &["liba", "libb", "libc"],
        &["liba", "libb"],
    );
    let st = sim.autoremove();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    let left = sim.installed();
    assert!(
        !left.contains("liba\n") && !left.contains("libb\n"),
        "{left}"
    );
    assert!(left.contains("libc\n"), "not shown, so not removed: {left}");
    assert_eq!(names(&st), ["libc"], "the next run shows it");
    assert_eq!(st.unused[0].size, 2048 * 1024);
    assert_eq!(st.unused[0].summary, "the libc library");
    assert!(
        !sim.log()
            .lines()
            .any(|l| l.contains("upgrade") && !l.contains(" -s ")),
        "it upgrades nothing: {}",
        sim.log()
    );
}

#[test]
fn gh0st_the_kernel_set_and_store_extras_are_never_unused() {
    let all = [
        "gh0st-term",
        "linux-image-6.18.50+rpt-rpi-v8",
        "raspi-firmware",
        "libstdc++6",
        "libx",
    ];
    let sim = Sim::new("keep", &all, &all, &all);
    sim.store_app("libstdc++6");
    let st = sim.autoremove();
    assert_eq!(st.outcome, Outcome::Ok, "{}", st.why);
    for n in &all[..4] {
        assert!(sim.installed().contains(&format!("{n}\n")), "{n} stays");
    }
    assert!(!sim.installed().contains("libx\n"));
    assert!(names(&st).is_empty(), "{:?}", st.unused);
}

#[test]
fn a_removal_that_would_take_more_says_so_and_removes_nothing() {
    let sim = Sim::new("more", &["tzdata", "liba"], &["liba"], &["liba"]);
    std::fs::write(sim.dir.join("remove_more"), b"").unwrap();
    let st = sim.autoremove();
    assert_eq!(st.outcome, Outcome::Error);
    assert!(st.why.contains("libgl1"), "{}", st.why);
    assert!(sim.installed().contains("liba\n"), "{}", sim.log());
}
