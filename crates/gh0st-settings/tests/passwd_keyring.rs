//! A password change keeps the keyring in step: Settings asks it to
//! prepare, runs passwd, then commits. Fake gh0st and passwd first on PATH
//! log each call and what came on stdin.

use gh0st_app::testing::Driver;
use gh0st_settings::app::{Embed, Settings};
use gh0st_settings::model::PageId;
use std::os::unix::fs::PermissionsExt;

const FAKE: &str = r#"#!/bin/sh
printf '%s ' "$(basename "$0") $*" >> "$SIM_LOG"
if [ "$(basename "$0")" = passwd ] || [ "$2" = prepare ]; then
  tr '\n' '|' >> "$SIM_LOG"
fi
echo >> "$SIM_LOG"
[ "$(basename "$0")" = gh0st ] && [ -n "$NO_KEYRING" ] && exit 3
exit 0
"#;

fn run(no_keyring: bool) -> Vec<String> {
    let dir =
        std::env::temp_dir().join(format!("gh0st-passwd-{}-{no_keyring}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["gh0st", "passwd"] {
        let p = dir.join(name);
        std::fs::write(&p, FAKE).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
        if no_keyring {
            std::env::set_var("NO_KEYRING", "1");
        }
    }
    let mut d = Driver::new(Settings::embedded(Embed {
        page: PageId::System,
        label: "System".into(),
        skip: false,
        done: false,
    }));
    d.jobs(5000).wait(100);
    for _ in 0..4 {
        d.keys("down down\nup down");
    }
    d.keys("enter");
    d.keys("type old\nenter");
    d.keys("type new\nenter");
    d.keys("type new\nenter");
    d.jobs(5000);
    let log = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    log.lines()
        .filter(|l| l.starts_with("passwd") || l.starts_with("gh0st keyring"))
        .map(String::from)
        .collect()
}

/// One test: the env is the process's, and both cases set it.
#[test]
fn passwd_runs_between_prepare_and_commit_or_alone() {
    assert_eq!(
        run(false),
        [
            "gh0st keyring prepare old|new|",
            "passwd  old|new|new|",
            "gh0st keyring commit 1 ",
        ]
    );
    // No keyring running: passwd still goes, and nothing commits.
    assert_eq!(
        run(true),
        ["gh0st keyring prepare old|new|", "passwd  old|new|new|"]
    );
}
