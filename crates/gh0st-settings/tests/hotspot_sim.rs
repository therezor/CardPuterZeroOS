//! The hotspot page end to end: the real binary under `--sim`, keys from a
//! script, fake pkexec, nmcli and iw first on PATH that log what they're
//! asked. Nothing on the host changes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PKEXEC: &str = r#"#!/bin/sh
echo "argv: $*" >> "$SIM_LOG"
case "$2 $3" in
  "hotspot password") printf 'stdin: %s\n' "$(/bin/cat)" >> "$SIM_LOG" ;;
esac
exit 0
"#;

/// The saved profile, as nmcli prints it with secrets.
const NMCLI: &str = r#"#!/bin/sh
echo "nmcli $*" >> "$SIM_LOG"
case "$*" in
  *"connection show gh0st-hotspot")
    printf '802-11-wireless.ssid:zero-kfwm\n802-11-wireless-security.psk:kfwmtqzrapdh\n802-11-wireless-security.key-mgmt:sae\n' ;;
esac
exit 0
"#;

/// Tap an arrow: a bare `down` line in a script means hold.
fn arrow(name: &str) -> String {
    format!("down {name}\nup {name}\n")
}

fn keys() -> String {
    let down = arrow("down");
    [
        "wait 500\n",
        // Root: Wi-Fi, then Hotspot.
        &down,
        "enter\nwait 500\n",
        // The switch.
        "enter\nwait 600\n",
        // Name: clear the old one, type a new one.
        &down,
        "enter\nwait 200\n",
        &"backspace\n".repeat(12),
        "type myhotspot\nenter\nwait 600\n",
        // Password: a short one is refused before any helper runs.
        &down,
        "enter\nwait 200\ntype abc1234\nenter\nwait 400\n",
        "enter\nwait 200\ntype goodpass99\nenter\nwait 600\n",
        // Security: the next choice.
        &down,
        &arrow("right"),
        "wait 600\n",
    ]
    .concat()
}

fn write_exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-hotspot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["bin", "home", "frames"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    write_exe(&d.join("bin/pkexec"), PKEXEC);
    write_exe(&d.join("bin/nmcli"), NMCLI);
    for tool in [
        "iw",
        "timedatectl",
        "bluetoothctl",
        "vcgencmd",
        "wpctl",
        "systemctl",
    ] {
        let text = format!("#!/bin/sh\necho \"{tool} $*\" >> \"$SIM_LOG\"\nexit 0\n");
        write_exe(&d.join("bin").join(tool), &text);
    }
    std::os::unix::fs::symlink("/bin/sh", d.join("bin/sh")).unwrap();
    d
}

#[test]
fn the_hotspot_page_drives_the_helper_and_keeps_the_password_off_argv() {
    let d = scratch();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st-settings"))
        .arg("--sim")
        .arg(d.join("frames"))
        .env_clear()
        .env("PATH", d.join("bin"))
        .env("HOME", d.join("home"))
        .env("SIM_LOG", d.join("log"))
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    let mut input = child.stdin.take().unwrap();
    input.write_all(keys().as_bytes()).unwrap();
    drop(input);
    assert!(child.wait().unwrap().success());
    let log = std::fs::read_to_string(d.join("log")).unwrap_or_default();

    let helper: Vec<&str> = log
        .lines()
        .filter_map(|l| l.strip_prefix("argv: /usr/libexec/gh0st-helper "))
        .collect();
    assert_eq!(
        helper,
        [
            "hotspot on",
            "hotspot name myhotspot",
            "hotspot password",
            "hotspot security mixed"
        ],
        "{log}"
    );
    // The profile is read with its secrets, from nmcli.
    assert!(
        log.lines()
            .any(|l| l.starts_with("nmcli -s") && l.ends_with("connection show gh0st-hotspot")),
        "{log}"
    );
    // The new password goes on stdin only; the short one never leaves.
    assert!(log.contains("stdin: goodpass99"), "{log}");
    assert!(
        !log.lines()
            .any(|l| !l.starts_with("stdin:") && l.contains("goodpass99")),
        "{log}"
    );
    assert!(!log.contains("abc1234"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}
