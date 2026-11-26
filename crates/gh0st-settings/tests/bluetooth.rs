//! Bluetooth off blocks the radio. The image boots every radio blocked and
//! systemd-rfkill restores the saved state, so a power off alone came back
//! on at the next boot. Fake tools first on PATH log each call; the radio
//! switch is the test's, so the host's radios never change.

use gh0st_app::testing::Driver;
use gh0st_core::rfkill::Radio;
use gh0st_settings::app::Settings;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

mod common;

const BLUETOOTHCTL: &str = "#!/bin/sh\necho \"bluetoothctl $*\" >> \"$SIM_LOG\"\n\
[ \"$1\" = show ] && echo 'Powered: yes'\nexit 0\n";
const PKEXEC: &str = "#!/bin/sh\necho \"pkexec $*\" >> \"$SIM_LOG\"\nexit 0\n";

static SWITCHED: Mutex<Vec<(Radio, bool)>> = Mutex::new(Vec::new());

fn works(r: Radio, blocked: bool) -> std::io::Result<()> {
    SWITCHED.lock().unwrap().push((r, blocked));
    Ok(())
}

fn denied(_: Radio, _: bool) -> std::io::Result<()> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

fn exe(path: &std::path::Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Opens Bluetooth, which reads as on, and turns it off.
fn turn_off(s: Settings, log: &std::path::Path) -> String {
    let _ = std::fs::remove_file(log);
    let mut d = Driver::new(s);
    d.jobs(2000);
    // Root: Wi-Fi, Hotspot, Network, then Bluetooth. Its first row is the
    // switch.
    d.keys("x\nx\nx\nenter").jobs(2000);
    assert_eq!(d.title(), "Bluetooth");
    d.keys("enter").jobs(3000);
    std::fs::read_to_string(log).unwrap_or_default()
}

#[test]
fn bluetooth_off_blocks_the_radio_directly_or_through_the_helper() {
    common::stub_tools();
    let dir = std::env::temp_dir().join(format!("gh0st-settings-bt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    exe(&dir.join("bluetoothctl"), BLUETOOTHCTL);
    exe(&dir.join("pkexec"), PKEXEC);
    let log = dir.join("log");
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // SAFETY: this test file is its own process, and this its only test.
    unsafe {
        std::env::set_var("PATH", path);
        std::env::set_var("SIM_LOG", &log);
    }

    let out = turn_off(Settings::new().with_radios(works), &log);
    assert!(out.contains("bluetoothctl power off"), "{out}");
    assert!(!out.contains("rfkill"), "blocked without the helper: {out}");
    assert_eq!(*SWITCHED.lock().unwrap(), [(Radio::Bluetooth, true)]);

    let out = turn_off(Settings::new().with_radios(denied), &log);
    let off = out.find("bluetoothctl power off").expect(&out);
    let block = out
        .find("pkexec /usr/libexec/gh0st-helper rfkill bluetooth off")
        .expect(&out);
    assert!(off < block, "power off first, then the block: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}
