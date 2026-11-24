//! The Store from the keys: open an app, install it with the password, and
//! confirm a risky one first. A fake pkexec first on PATH answers as
//! gh0st-helper does and logs what it is asked. M5's old cache goes in
//! child processes, and the idle Store doesn't wake. Alone in its own test
//! binary: it sets PATH and the Store's folders.

use gh0st_app::testing::Driver;
use gh0st_core::store::{About, App, Catalog, Flags, Icons, Pinned};
use gh0st_store::app::Store;
use std::os::unix::fs::PermissionsExt;

/// Installs need the password; the first call has none.
const PKEXEC: &str = r#"#!/bin/sh
read -r pw
echo "$* [$pw]" >> "$HELPER_LOG"
case "$2" in
  store-install|store-remove|store-upgrade) [ "$pw" = secret ] || exit 10 ;;
esac
exit 0
"#;

fn app(package: &str, title: &str, flags: &str) -> App {
    App {
        package: package.into(),
        version: "1.0".into(),
        size: 58_940,
        sha256: "e".repeat(64),
        category: "Games".into(),
        flags: Flags::parse(flags),
        title: title.into(),
        summary: "A small app".into(),
        author: "someone".into(),
        licence: "MIT".into(),
        url: format!("https://github.com/x/{package}.deb"),
        cn_url: String::new(),
        share: String::new(),
        desktop: package.into(),
    }
}

#[test]
fn install_with_the_password_and_confirm_a_risky_one() {
    let dir = std::env::temp_dir().join(format!("gh0st-store-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (state, run, urun, bin, cache) = (
        dir.join("state"),
        dir.join("run"),
        dir.join("urun"),
        dir.join("bin"),
        dir.join("cache"),
    );
    let old = cache.join("cardputerzero-appstore");
    for d in [&state, &run, &urun, &bin, &old] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(old.join("old.deb"), vec![0u8; 2_000_000]).unwrap();
    std::fs::write(bin.join("pkexec"), PKEXEC).unwrap();
    std::fs::set_permissions(bin.join("pkexec"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let about = About {
        description: "A synth.".into(),
        shots: Vec::new(),
    }
    .line("acidzero");
    let catalog = Catalog {
        date: 1,
        expires: u64::MAX,
        about: Pinned {
            sha256: "a".repeat(64),
            size: 1,
        },
        icons: Pinned {
            sha256: "b".repeat(64),
            size: 1,
        },
        apps: vec![
            app("acidzero", "AcidZero", "hardware"),
            // CJK the device can't draw falls back to the package name.
            app("rootsetup", "\u{6839}\u{8bbe}\u{7f6e}", "scripts,service"),
        ],
    };
    std::fs::write(state.join("catalog"), catalog.text()).unwrap();
    std::fs::write(state.join("about"), about).unwrap();
    std::fs::write(state.join("icons"), Icons::build(&[])).unwrap();
    std::fs::write(dir.join("status"), "").unwrap();
    let log = dir.join("log");
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        std::env::set_var("HELPER_LOG", &log);
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("GH0ST_STORE_STATE", &state);
        std::env::set_var("GH0ST_STORE_RUN", &run);
        std::env::set_var("GH0ST_UPDATE_RUN", &urun);
        std::env::set_var("GH0ST_DPKG_STATUS", dir.join("status"));
    }

    let mut d = Driver::new(Store::new());
    d.wait(50);
    d.jobs(5000);
    assert!(!old.exists(), "M5's old cache is gone");
    assert!(
        d.toast().is_some_and(|t| t.starts_with("Freed 2")),
        "{:?}",
        d.toast()
    );

    // Home opens on its tiles: Enter on All apps, Enter on AcidZero, Enter
    // installs it. The helper wants the password, so the dialog asks.
    d.keys("enter")
        .wait(50)
        .keys("enter")
        .wait(50)
        .keys("enter");
    d.jobs(5000);
    d.keys("type secret\nenter");
    d.jobs(5000);
    d.wait(50);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls,
        "/usr/libexec/gh0st-helper store-install --mirror auto acidzero=1.0 []\n\
         /usr/libexec/gh0st-helper store-install --mirror auto acidzero=1.0 [secret]\n"
    );

    // The engine ran: it wrote a state that ended. Back to the list, then
    // the risky app asks before it installs.
    std::fs::write(
        state.join("state"),
        "ended=2\nop=install\noutcome=ok\npackages=acidzero\n",
    )
    .unwrap();
    // x is Down on the Cardputer.
    d.keys("esc")
        .wait(50)
        .keys("x")
        .wait(50)
        .keys("enter")
        .wait(50);
    d.keys("enter").wait(50);
    let (title, _) = d.dialog().expect("the risky app asks first");
    assert_eq!(title, "Install rootsetup?");
    d.keys("enter").wait(50);
    // On Linux inotify sees AcidZero's run end; elsewhere the start timer
    // reads the state. Either way rootsetup goes next.
    d.wait(16_000);
    d.jobs(5000);
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("store-install --mirror auto rootsetup=1.0"),
        "{calls}"
    );
    assert!(d.lint().is_empty(), "{:?}", d.lint());

    // Idle on the list: the clock's minute and nothing else.
    d.keys("esc").wait(50);
    std::fs::remove_file(state.join("state")).ok();
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "{wakes}");
    let _ = std::fs::remove_dir_all(&dir);
}
