//! Bluetooth off blocks the radio. The image boots every radio blocked and
//! systemd-rfkill restores the saved state, so a power off alone came back
//! on at the next boot. A fake BlueZ takes the power off; the radio switch
//! is the test's, so the host's radios never change.

use gh0st_app::testing::Driver;
use gh0st_core::rfkill::Radio;
use gh0st_settings::app::Settings;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

mod common;
use common::fakebluez;

const PKEXEC: &str = "#!/bin/sh\necho \"pkexec $*\" >> \"$SIM_LOG\"\nexit 0\n";

static SWITCHED: Mutex<Vec<(Radio, bool)>> = Mutex::new(Vec::new());

fn works(r: Radio, blocked: bool) -> std::io::Result<()> {
    SWITCHED.lock().unwrap().push((r, blocked));
    Ok(())
}

fn denied(_: Radio, _: bool) -> std::io::Result<()> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

/// Opens Bluetooth, which reads as on, and turns it off. What pkexec ran,
/// and whether BlueZ was told to power off.
fn turn_off(
    radios: fn(Radio, bool) -> std::io::Result<()>,
    log: &std::path::Path,
) -> (String, bool) {
    let _ = std::fs::remove_file(log);
    let (conn, bluez) = fakebluez::start(true, vec![]);
    let mut d = Driver::new(Settings::new().with_bluez(conn).with_radios(radios));
    d.jobs(2000).pump(200);
    // Root: Wi-Fi, Hotspot, Network, then Bluetooth. Its first row is the
    // switch.
    d.keys("x\nx\nx\nenter").pump(200);
    assert_eq!(d.title(), "Bluetooth");
    d.keys("enter").pump(200).jobs(3000);
    let off = bluez.called("Set /org/bluez/hci0 Powered=false");
    (std::fs::read_to_string(log).unwrap_or_default(), off)
}

#[test]
fn bluetooth_off_blocks_the_radio_directly_or_through_the_helper() {
    common::stub_tools();
    let dir = std::env::temp_dir().join(format!("gh0st-settings-bt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pkexec = dir.join("pkexec");
    std::fs::write(&pkexec, PKEXEC).unwrap();
    std::fs::set_permissions(&pkexec, std::fs::Permissions::from_mode(0o755)).unwrap();
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

    let (out, off) = turn_off(works, &log);
    assert!(off, "BlueZ powers off");
    assert!(!out.contains("rfkill"), "blocked without the helper: {out}");
    assert_eq!(*SWITCHED.lock().unwrap(), [(Radio::Bluetooth, true)]);

    let (out, off) = turn_off(denied, &log);
    assert!(off, "BlueZ powers off");
    assert!(
        out.contains("pkexec /usr/libexec/gh0st-helper rfkill bluetooth off"),
        "the helper blocks it: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
