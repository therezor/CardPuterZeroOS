//! Settings > Apps > an app that runs as root > Root, no password: the
//! switch asks the helper, which wants the password (docs/isolation.md,
//! Root apps). A fake pkexec logs the call. Alone in its own process: it
//! sets PATH and the app folders.

use gh0st_app::testing::Driver;
use gh0st_settings::app::Settings;
use std::os::unix::fs::PermissionsExt;

mod common;

const PKEXEC: &str = "#!/bin/sh\necho \"pkexec $*\" >> \"$SIM_LOG\"\nexit 0\n";

fn exe(p: &std::path::Path, text: &str) {
    std::fs::write(p, text).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn the_root_switch_asks_the_helper() {
    common::stub_tools();
    let dir = std::env::temp_dir().join(format!("gh0st-settings-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["bin", "ours", "stock", "icons", "home"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    exe(&dir.join("bin/pkexec"), PKEXEC);
    // sudo in its Exec: it runs as root, so it may take a grant.
    exe(&dir.join("rooty"), "#!/bin/sh\n");
    std::fs::write(
        dir.join("stock/rooty.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Rooty\nExec={} sudo\n",
            dir.join("rooty").display()
        ),
    )
    .unwrap();
    let log = dir.join("log");
    // SAFETY: this test file is its own process, and this its only test.
    unsafe {
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                dir.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        std::env::set_var("SIM_LOG", &log);
        std::env::set_var("HOME", dir.join("home"));
        std::env::set_var("GH0ST_APPS_OURS", dir.join("ours"));
        std::env::set_var("GH0ST_APPS_STOCK", dir.join("stock"));
        std::env::set_var("GH0ST_ICON_ROOT", dir.join("icons"));
    }
    let mut d = Driver::new(Settings::new());
    d.jobs(2000).pump(200);
    // Root: Wi-Fi, Hotspot, Network, Bluetooth, Display, Sound,
    // Appearance, then Apps.
    d.keys(&"x\n".repeat(7)).keys("enter").pump(200);
    assert_eq!(d.title(), "Apps");
    d.keys("enter").pump(200);
    assert_eq!(d.title(), "Rooty");
    // Its last row: Root, no password.
    d.keys(&"x\n".repeat(7)).keys("enter").pump(200).jobs(3000);
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("pkexec /usr/libexec/gh0st-helper root-grant rooty on"),
        "{calls}"
    );
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    let _ = std::fs::remove_dir_all(&dir);
}
