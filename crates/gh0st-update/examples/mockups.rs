//! The Updates app's real pages over fixture state files, as the engine
//! writes them, into mockups/update-app. Run from the workspace root:
//!
//!     cargo run -p gh0st-update --example mockups

use gh0st_app::testing::Driver;
use gh0st_core::update::{Group, Outcome, Package, Phase, Progress, Restart, State};
use gh0st_update::app::Updates;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

fn pkg(name: &str, from: &str, to: &str, size: u64, group: Group, summary: &str) -> Package {
    Package {
        name: name.into(),
        from: from.into(),
        to: to.into(),
        size,
        group,
        restart: Restart::of(name),
        summary: summary.into(),
    }
}

fn ready() -> State {
    State {
        checked: gh0st_core::notify::now_secs() - 600,
        outcome: Outcome::Ok,
        channel: "stable".into(),
        packages: vec![
            pkg(
                "gh0st",
                "0.4.1+202611201200.1a2b3c4",
                "0.5.0+202611221733.35a14dd",
                4_100_000,
                Group::Gh0st,
                "The Gh0st OS shell and Settings",
            ),
            pkg(
                "gh0st-kernel",
                "6.18.47-1+rpt1+m5.a3fd3e8+gh0st1",
                "6.18.60-1+rpt1+m5.a3fd3e8+gh0st1",
                21_000_000,
                Group::Gh0st,
                "the CardputerZero kernel, drivers and firmware as one pinned set",
            ),
            pkg(
                "openssl",
                "3.5.1-1",
                "3.5.1-1+deb13u1",
                1_500_000,
                Group::Security,
                "Secure Sockets Layer toolkit",
            ),
            pkg(
                "libssl3t64",
                "3.5.1-1",
                "3.5.1-1+deb13u1",
                2_300_000,
                Group::Security,
                "Secure Sockets Layer toolkit, shared libraries",
            ),
            pkg(
                "curl",
                "8.14.1-2",
                "8.14.1-2+deb13u1",
                300_000,
                Group::Security,
                "command line tool for transferring data with URL syntax",
            ),
            pkg(
                "libc6",
                "2.41-12+deb13u4",
                "2.41-12+deb13u5",
                2_488_872,
                Group::System,
                "GNU C Library: Shared libraries",
            ),
            pkg(
                "tzdata",
                "2025b-4",
                "2025b-4+deb13u1",
                300_000,
                Group::System,
                "time zone and daylight-saving time data",
            ),
            pkg(
                "base-files",
                "13.8+deb13u7",
                "13.8+deb13u8",
                88_320,
                Group::System,
                "Debian base system miscellaneous files",
            ),
        ],
        ..State::default()
    }
}

struct Scene {
    name: &'static str,
    state: State,
    progress: Option<Progress>,
    keys: &'static str,
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/update-app");
    std::fs::create_dir_all(&out).unwrap();
    each_scene(|name, d| save(d, &out, name));
    println!("wrote {}", out.display());
}

/// The mockups are the fixtures: every scene, in every palette, under the
/// design lint.
#[test]
fn every_scene_keeps_the_design_rules() {
    let mut found = Vec::new();
    each_scene(|name, d| {
        for p in gh0st_app::gfx::theme::PALETTES {
            d.palette(p.name);
            found.extend(
                d.lint()
                    .into_iter()
                    .map(|f| format!("{name} {}: {f}", p.name)),
            );
        }
    });
    assert!(found.is_empty(), "{found:#?}");
}

/// Runs the app on each scene's state and keys, then hands it over.
fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let base = std::env::temp_dir().join(format!("gh0st-update-mockups-{}", std::process::id()));
    let busy = |install: bool, phase: Phase, percent: Option<u8>, n: u32, item: &str| Progress {
        install,
        phase,
        percent,
        done: 12_100_000,
        total: 34_000_000,
        n,
        m: if install { 8 } else { 12 },
        item: item.into(),
    };
    let scenes = [
        Scene {
            name: "01-never",
            state: State::default(),
            progress: None,
            keys: "",
        },
        Scene {
            name: "02-checking",
            state: State::default(),
            progress: Some(busy(false, Phase::Lists, Some(25), 3, "")),
            keys: "",
        },
        Scene {
            name: "03-current",
            state: State {
                checked: gh0st_core::notify::now_secs() - 3000,
                outcome: Outcome::Ok,
                channel: "stable".into(),
                ..State::default()
            },
            progress: None,
            keys: "",
        },
        Scene {
            name: "04-ready",
            state: ready(),
            progress: None,
            keys: "",
        },
        Scene {
            name: "05-confirm",
            state: ready(),
            progress: None,
            keys: "enter",
        },
        Scene {
            name: "06-download",
            state: ready(),
            progress: Some(busy(true, Phase::Download, Some(35), 0, "")),
            keys: "",
        },
        Scene {
            name: "07-install",
            state: ready(),
            progress: Some(busy(true, Phase::Install, Some(48), 4, "libc6")),
            keys: "",
        },
        Scene {
            name: "08-done",
            state: State {
                installed: 1,
                installed_count: 8,
                restart: Restart::Device,
                outcome: Outcome::Ok,
                checked: 1,
                ..State::default()
            },
            progress: None,
            keys: "",
        },
        Scene {
            name: "09-offline",
            state: State {
                checked: 1,
                outcome: Outcome::Offline,
                why: "No network".into(),
                ..State::default()
            },
            progress: None,
            keys: "",
        },
        Scene {
            name: "10-packages",
            state: ready(),
            progress: None,
            keys: "7",
        },
        Scene {
            name: "11-detail",
            state: ready(),
            progress: None,
            keys: "7\nwait\nenter",
        },
        Scene {
            name: "12-menu",
            state: ready(),
            progress: None,
            keys: "8",
        },
        Scene {
            name: "13-edge",
            state: State {
                channel: "edge".into(),
                ..ready()
            },
            progress: None,
            keys: "",
        },
    ];
    for s in scenes {
        let dir = base.join(s.name);
        let (state_dir, run_dir) = (dir.join("state"), dir.join("run"));
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(state_dir.join("state"), s.state.text()).unwrap();
        // The engine holds its lock while it runs; a progress file without
        // it is stale.
        let _lock = s.progress.as_ref().map(|p| {
            std::fs::write(run_dir.join("progress"), p.text()).unwrap();
            let f = std::fs::File::create(run_dir.join("lock")).unwrap();
            // SAFETY: flock on a file this function owns.
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
            f
        });
        // SAFETY: one thread; the page reads these as it starts.
        unsafe {
            std::env::set_var("GH0ST_UPDATE_STATE", &state_dir);
            std::env::set_var("GH0ST_UPDATE_RUN", &run_dir);
        }
        let mut d = Driver::new(Updates::new());
        // A list learns its rows when drawn, so a key after a push waits
        // for a frame, as on the device.
        for step in s.keys.split('\n').filter(|k| !k.is_empty()) {
            if step == "wait" {
                d.wait(100);
            } else {
                d.keys(step);
            }
        }
        d.wait(100);
        each(s.name, &mut d);
    }
    let _ = std::fs::remove_dir_all(&base);
}

fn save(d: &mut Driver, out: &Path, name: &str) {
    d.save(&out.join(format!("{name}.png"))).unwrap();
}
