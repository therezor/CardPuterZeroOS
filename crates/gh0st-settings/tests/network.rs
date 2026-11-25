//! A network change reads nmcli again only when the page in front shows
//! the network; under another page it waits for one that does. A fake
//! nmcli first on PATH logs each call.

use gh0st_app::testing::Driver;
use gh0st_settings::app::Settings;
use std::os::unix::fs::PermissionsExt;

mod common;

const NMCLI: &str = "#!/bin/sh\necho \"$*\" >> \"$SIM_LOG\"\nexit 0\n";

#[test]
fn a_change_under_a_page_without_the_network_waits_for_one_with_it() {
    common::stub_tools();
    let dir = std::env::temp_dir().join(format!("gh0st-settings-net-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let nmcli = dir.join("nmcli");
    std::fs::write(&nmcli, NMCLI).unwrap();
    std::fs::set_permissions(&nmcli, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    let reads = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("device show"))
            .count()
    };
    let mut d = Driver::new(Settings::new());
    d.jobs(5000);
    assert_eq!(reads(), 1, "the first page reads the network");
    // Display: Wi-Fi, Hotspot, Network, Bluetooth, then Display.
    d.keys("x\nx\nx\nx\nenter");
    assert_eq!(d.title(), "Display");
    d.network_changed().jobs(2000);
    d.network_changed().jobs(2000);
    assert_eq!(reads(), 1, "Display shows none of it");
    d.keys("esc").jobs(2000);
    assert_eq!(reads(), 2, "back on the first page, once");
    d.network_changed().jobs(2000);
    assert_eq!(reads(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}
