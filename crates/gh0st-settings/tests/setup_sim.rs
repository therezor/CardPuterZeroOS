//! First-boot setup end to end: the real binary under `--sim`, keys from a
//! script, and fake pkexec, nmcli and timedatectl first on PATH that log
//! what they're asked. Nothing on the host changes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PKEXEC: &str = r#"#!/bin/sh
echo "argv: $*" >> "$SIM_LOG"
case "$2" in
  user-create) printf 'stdin: %s\n' "$(/bin/cat)" >> "$SIM_LOG" ;;
  regdom)
    f="$GH0ST_BOOT_DIR/cmdline.txt"
    /usr/bin/awk -v cc="$3" '{ sub(/regdom=[A-Z0-9]*/, "regdom=" cc); print }' "$f" > "$f.new"
    /bin/mv "$f.new" "$f" ;;
esac
exit 0
"#;

const TIMEDATECTL: &str = r#"#!/bin/sh
echo "timedatectl $*" >> "$SIM_LOG"
case "$1" in
  list-timezones) /usr/bin/awk -F'\t' '!/^#/ { print $3 }' /usr/share/zoneinfo/zone.tab ;;
  show) printf 'Timezone=Europe/London\nNTP=no\n' ;;
esac
exit 0
"#;

/// The welcome's Skip, then each settings step; waits let the fakes finish.
const KEYS: &str = "\
wait 400
enter
wait 300
6
wait 400
type u
enter
wait 300
enter
wait 1000
enter
wait 1000
6
wait 600
7
wait 600
enter
wait 1000
type alex
tab
type hunter22
enter
wait 300
type hunter22
enter
wait 400
6
wait 600
enter
wait 2500
";

/// The notice, the welcome's Enter, then every lesson passed by its keys:
/// the oath, the oath in caps, the sobriety walk, Tab past the email, the
/// Don't chip, the hat and the paranoia, the menu's Next lesson, the held
/// Boom chip, and Enter to wear the reward.
const TOUR: &str = "\
wait 300
enter
wait 300
enter
wait 300
type opsec
wait 1400
type OPSEC
wait 1400
c
c
x
enter
wait 1400
tab
wait 300
7
wait 1400
7
f
enter
wait 1400
8
wait 200
enter
wait 1400
down 7
wait 900
up 7
wait 1600
enter
wait 600
";

fn write_exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-setup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["bin", "root/etc", "boot", "home", "frames"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    write_exe(&d.join("bin/pkexec"), PKEXEC);
    write_exe(&d.join("bin/timedatectl"), TIMEDATECTL);
    for tool in ["nmcli", "bluetoothctl", "vcgencmd", "wpctl"] {
        let text = format!("#!/bin/sh\necho \"{tool} $*\" >> \"$SIM_LOG\"\nexit 0\n");
        write_exe(&d.join("bin").join(tool), &text);
    }
    std::os::unix::fs::symlink("/bin/sh", d.join("bin/sh")).unwrap();
    std::fs::write(d.join("root/etc/passwd"), "root:x:0:0:root:/root:/bin/sh\n").unwrap();
    std::fs::write(d.join("root/etc/group"), "root:x:0:\nvideo:x:44:\n").unwrap();
    std::os::unix::fs::symlink(
        "/usr/share/zoneinfo/Europe/London",
        d.join("root/etc/localtime"),
    )
    .unwrap();
    std::fs::write(
        d.join("boot/cmdline.txt"),
        "console=serial0,115200 root=/dev/mmcblk0p2 cfg80211.ieee80211_regdom=00\n",
    )
    .unwrap();
    d
}

fn have_tz() -> bool {
    let tz = Path::new("/usr/share/zoneinfo");
    let ok = tz.join("zone.tab").exists() && tz.join("iso3166.tab").exists();
    if !ok {
        eprintln!("skipped: no tz database on this host");
    }
    ok
}

/// Runs setup under --sim with `keys`; returns the fakes' log.
fn run(d: &Path, keys: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st-settings"))
        .args(["--setup", "--sim"])
        .arg(d.join("frames"))
        .env_clear()
        .env("PATH", d.join("bin"))
        .env("HOME", d.join("home"))
        .env("SIM_LOG", d.join("log"))
        .env("GH0ST_SETUP_ROOT", d.join("root"))
        .env("GH0ST_BOOT_DIR", d.join("boot"))
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    let mut input = child.stdin.take().unwrap();
    input.write_all(keys.as_bytes()).unwrap();
    drop(input);
    assert!(child.wait().unwrap().success());
    std::fs::read_to_string(d.join("log")).unwrap_or_default()
}

fn state(d: &Path) -> String {
    std::fs::read_to_string(d.join("root/var/lib/gh0st/setup.state")).unwrap()
}

#[test]
fn the_tour_passes_each_lesson_by_its_keys() {
    if !have_tz() {
        return;
    }
    let d = scratch("tour");
    run(&d, TOUR);
    // The notice, the welcome, eight lessons and the reward: step 11 is
    // Country.
    let s = state(&d);
    assert!(s.contains("step=11\n") && s.contains("tour=1"), "{s}");
    // The reward: Phantom earned and worn.
    let config = d.join("home/.config/cardputerzero/config.json");
    let config = std::fs::read_to_string(config).unwrap();
    assert!(config.contains("\"tour_done\": true"), "{config}");
    assert!(config.contains("\"palette\": \"Phantom\""), "{config}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn setup_walks_every_step_and_hands_over() {
    if !have_tz() {
        return;
    }
    let d = scratch("flow");
    let log = run(&d, KEYS);
    let helper: Vec<&str> = log
        .lines()
        .filter_map(|l| l.strip_prefix("argv: /usr/libexec/gh0st-helper "))
        .collect();
    let at = |verb: &str| helper.iter().position(|l| l.starts_with(verb));
    assert!(at("regdom UG").is_some(), "{log}");
    assert!(
        log.contains("timedatectl set-timezone Africa/Kampala"),
        "{log}"
    );
    assert!(at("rfkill wlan on").is_some(), "{log}");
    assert!(at("set-time ").is_some(), "{log}");
    let (made, finished) = (at("user-create alex"), at("setup-finish"));
    assert!(made.is_some() && made < finished, "{log}");
    // The password goes on stdin, never in the arguments or the state file.
    assert!(log.contains("stdin: hunter22"));
    assert!(
        !log.lines()
            .any(|l| l.starts_with("argv:") && l.contains("hunter22"))
    );
    let s = state(&d);
    assert!(s.contains("name=alex") && !s.contains("hunter22"), "{s}");
    let _ = std::fs::remove_dir_all(&d);
}
