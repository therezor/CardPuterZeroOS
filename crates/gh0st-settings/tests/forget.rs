//! Forget asks for a second press in the row, from the real keys: Backspace
//! reaches Settings as a page command. A fake nmcli first on PATH answers
//! the queries and logs the delete.

use gh0st_app::testing::Driver;
use gh0st_settings::app::{Embed, Settings};
use gh0st_settings::model::PageId;
use std::os::unix::fs::PermissionsExt;

const NMCLI: &str = r#"#!/bin/sh
echo "nmcli $*" >> "$SIM_LOG"
case "$*" in
  *"WIFI radio"*) echo enabled ;;
  *"NAME,TYPE connection show"*) echo "home:802-11-wireless" ;;
  *"wifi list"*) echo "*:80:WPA2:home" ;;
esac
exit 0
"#;

#[test]
fn forget_needs_a_second_backspace_on_the_same_row() {
    let dir = std::env::temp_dir().join(format!("gh0st-forget-{}", std::process::id()));
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
    let deletes = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("connection delete"))
            .count()
    };
    let mut d = Driver::new(Settings::embedded(Embed {
        page: PageId::Wifi,
        label: "Setup 1 of 1".into(),
        skip: false,
        done: false,
    }));
    d.jobs(5000).wait(100);
    d.keys("backspace");
    d.jobs(2000);
    assert_eq!(deletes(), 0, "the first press only arms");
    d.keys("backspace");
    d.jobs(2000);
    assert_eq!(deletes(), 1, "the second forgets");
    let _ = std::fs::remove_dir_all(&dir);
}
