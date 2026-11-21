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

fn write_exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-setup-sim-{}", std::process::id()));
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

#[test]
fn setup_walks_every_step_and_hands_over() {
    let tz = Path::new("/usr/share/zoneinfo");
    if !tz.join("zone.tab").exists() || !tz.join("iso3166.tab").exists() {
        eprintln!("skipped: no tz database on this host");
        return;
    }
    let d = scratch();
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(KEYS.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());

    let log = std::fs::read_to_string(d.join("log")).unwrap();
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
    let state = std::fs::read_to_string(d.join("root/var/lib/cpzeroos/setup.state")).unwrap();
    assert!(
        state.contains("name=alex") && !state.contains("hunter22"),
        "{state}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
