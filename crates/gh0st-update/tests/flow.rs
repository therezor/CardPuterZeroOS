//! Installing from the Updates app: Enter on the ready list, the card says
//! yes, the helper asks for the password, and the install unit starts. A
//! fake pkexec first on PATH answers as gh0st-helper does and logs what it
//! is asked. Alone in its own test binary: it sets PATH and the update
//! folders, and the SDK reaps every child of the process.

use gh0st_app::testing::Driver;
use gh0st_core::update::{Group, Outcome, Package, Restart, State};
use gh0st_update::app::Updates;
use std::os::unix::fs::PermissionsExt;

/// Install needs the password; checking doesn't.
const PKEXEC: &str = r#"#!/bin/sh
read -r pw
echo "$* [$pw]" >> "$HELPER_LOG"
case "$2" in
  update-install) [ "$pw" = secret ] || exit 10 ;;
esac
exit 0
"#;

#[test]
fn enter_confirm_password_then_the_install_unit_starts() {
    let dir = std::env::temp_dir().join(format!("gh0st-update-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (state_dir, run_dir, bin) = (dir.join("state"), dir.join("run"), dir.join("bin"));
    for d in [&state_dir, &run_dir, &bin] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(bin.join("pkexec"), PKEXEC).unwrap();
    std::fs::set_permissions(bin.join("pkexec"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let st = State {
        checked: gh0st_core::notify::now_secs(),
        outcome: Outcome::Ok,
        channel: "stable".into(),
        packages: vec![Package {
            name: "tzdata".into(),
            from: "2025b-4".into(),
            to: "2025b-4+deb13u1".into(),
            size: 300_000,
            group: Group::System,
            restart: Restart::None,
            summary: "time zone data".into(),
        }],
        ..State::default()
    };
    std::fs::write(state_dir.join("state"), st.text()).unwrap();
    let log = dir.join("log");
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        std::env::set_var("HELPER_LOG", &log);
        std::env::set_var("GH0ST_UPDATE_STATE", &state_dir);
        std::env::set_var("GH0ST_UPDATE_RUN", &run_dir);
    }

    let mut d = Driver::new(Updates::new());
    d.wait(50);
    // Enter on the ready list opens the card; Enter on the card says yes.
    d.keys("enter").wait(50).keys("enter");
    d.jobs(5000);
    // The helper wanted the password: type it into the dialog.
    d.keys("type secret\nenter");
    d.jobs(5000);
    d.wait(50);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls,
        "/usr/libexec/gh0st-helper update-install []\n\
         /usr/libexec/gh0st-helper update-install [secret]\n"
    );
    // Stop is the one chip, on 6: the page waits for the engine.
    d.keys("6");
    d.jobs(5000);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.ends_with("/usr/libexec/gh0st-helper update-stop []\n"),
        "{calls}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
