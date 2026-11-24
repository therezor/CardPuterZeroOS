//! Network > DNS end to end: the real binary under `--sim`, keys from a
//! script, a fake pkexec that logs what the helper is asked. Nothing on the
//! host changes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PKEXEC: &str = "#!/bin/sh\necho \"argv: $*\" >> \"$SIM_LOG\"\nexit 0\n";

fn tap(name: &str) -> String {
    format!("down {name}\nup {name}\n")
}

fn keys() -> String {
    let down = tap("down");
    [
        "wait 500\n",
        // Root: Wi-Fi, Hotspot, then Network. DNS is its first row.
        &down,
        &down,
        "enter\nwait 500\n",
        "enter\nwait 300\n",
        // Automatic, Cloudflare, then Quad9.
        &down,
        &down,
        "enter\nwait 600\n",
        // Custom: an address with a shell word after it is refused, then
        // a plain one goes.
        "enter\nwait 300\n",
        &down,
        &down,
        &down,
        "enter\nwait 300\n",
        "type 1.1.1.1;reboot\nenter\nwait 400\n",
        &"backspace\n".repeat(16),
        "type 192.168.1.1\nenter\nwait 600\n",
    ]
    .concat()
}

fn write_exe(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-dns-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for sub in ["bin", "home", "frames"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    write_exe(&d.join("bin/pkexec"), PKEXEC);
    for tool in [
        "nmcli",
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
fn a_dns_pick_runs_the_helper_and_a_bad_address_never_does() {
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
        ["dns 9.9.9.9,149.112.112.112,2620:fe::fe", "dns 192.168.1.1"],
        "{log}"
    );
    assert!(!log.contains("reboot"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}
