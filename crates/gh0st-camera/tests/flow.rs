//! The camera over the fake helper: what a press does on the card, and what
//! a pause lets go of.

use gh0st_app::testing::Driver;
use gh0st_camera::app::Camera;
use gh0st_camera::files::{self, Kind};
use gh0st_camera::helper::Source;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// One test at a time: each app reaps with `waitpid(-1)`, so in one process
/// a test could take another's helper, even one std is still spawning.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-camera-flow-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn start(source: Source, name: &str) -> (Driver, PathBuf) {
    let d = dir(name);
    let mut drv = Driver::new(Camera::with(source, d.clone()));
    drv.pump(150);
    (drv, d)
}

fn count(d: &std::path::Path, kind: Kind) -> usize {
    files::list(d).iter().filter(|s| s.kind == kind).count()
}

#[test]
fn enter_saves_a_photo() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "photo");
    drv.keys("enter").pump(150);
    assert_eq!(count(&d, Kind::Photo), 1);
    assert!(drv.lint().is_empty());
}

#[test]
fn the_helper_runs_as_a_process_too() {
    let _one = serial();
    let exe = env!("CARGO_BIN_EXE_gh0st-camera").to_string();
    let (mut drv, d) = start(Source::Exec(vec![exe, "--fake-helper".into()]), "process");
    drv.keys("enter").pump(300);
    assert_eq!(count(&d, Kind::Photo), 1);
}

#[test]
fn the_timer_waits_before_the_shot() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "timer");
    drv.keys("t\nenter").pump(100);
    assert_eq!(count(&d, Kind::Photo), 0);
    drv.wait(3000).pump(150);
    assert_eq!(count(&d, Kind::Photo), 1);
}

#[test]
fn esc_cancels_the_timer_without_leaving() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "cancel");
    drv.keys("t\nenter\nesc").wait(4000).pump(150);
    assert_eq!(count(&d, Kind::Photo), 0);
    assert!(!drv.quit());
}

#[test]
fn the_screen_stays_on_only_while_recording() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "awake");
    drv.keys("tab").pump(100);
    assert!(!drv.awake());
    drv.keys("enter").pump(100);
    assert!(drv.awake());
    drv.keys("enter").pump(150);
    // A board is slower to hear the helper stop.
    for _ in 0..20 {
        if !drv.awake() {
            break;
        }
        drv.pump(50);
    }
    assert!(!drv.awake());
    assert_eq!(count(&d, Kind::Video), 1);
}

#[test]
fn a_pause_ends_the_recording_and_rests() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "pause");
    drv.keys("tab\nenter").pump(150);
    assert!(drv.awake());
    drv.pause().pump(150);
    assert!(!drv.awake());
    assert_eq!(count(&d, Kind::Video), 1);
    // Stopped, the camera wakes for nothing.
    assert_eq!(drv.idle_wakes(30_000), 0);
    drv.resume().pump(150);
    drv.keys("enter").pump(150);
    assert!(drv.awake(), "recording again after the return");
}

#[test]
fn a_held_six_in_review_deletes_the_shot() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: true }, "delete");
    drv.keys("enter").pump(150);
    drv.keys("enter").pump(150);
    assert_eq!(count(&d, Kind::Photo), 2);
    drv.keys("p").pump(100);
    // A tap is not enough.
    drv.keys("6").pump(50);
    assert_eq!(count(&d, Kind::Photo), 2);
    drv.keys("down 6").wait(1000).keys("up 6").pump(50);
    assert_eq!(count(&d, Kind::Photo), 1);
    assert!(drv.lint().is_empty());
}

#[test]
fn without_a_camera_enter_does_nothing() {
    let _one = serial();
    let (mut drv, d) = start(Source::Fake { camera: false }, "lite");
    drv.keys("enter").pump(100);
    assert_eq!(count(&d, Kind::Photo), 0);
    assert!(drv.lint().is_empty());
}

#[test]
fn a_missing_helper_is_said_not_crashed() {
    let _one = serial();
    let (mut drv, d) = start(
        Source::Exec(vec!["/nonexistent/gh0st-camd".into()]),
        "missing",
    );
    drv.keys("enter").pump(50);
    assert_eq!(count(&d, Kind::Photo), 0);
    assert!(drv.lint().is_empty());
}

/// The C++ helper over the same wire, when `GH0ST_CAMD_TEST` names a
/// build of it: `make -C crates/gh0st-camera/camd stub O=$PWD/target/camd`
/// gives one with a test-pattern sensor.
#[test]
fn the_cpp_helper_speaks_the_same_wire() {
    let _one = serial();
    let Ok(exe) = std::env::var("GH0ST_CAMD_TEST") else {
        eprintln!("skipped: GH0ST_CAMD_TEST is not set");
        return;
    };
    let (mut drv, d) = start(Source::Exec(vec![exe]), "cpp");
    drv.keys("enter").pump(1500);
    let shots = files::list(&d);
    assert_eq!(shots.len(), 1, "one photo");
    let jpeg = std::fs::read(&shots[0].path).unwrap();
    assert_eq!(
        &jpeg[..4],
        b"\xff\xd8\xff\xe1",
        "a JPEG that opens with EXIF"
    );
    assert_eq!(&jpeg[6..12], b"Exif\0\0");
    assert!(std::fs::read_dir(&d).unwrap().count() == 1, "no .part left");
    assert!(drv.lint().is_empty());
    drv.pause().pump(300);
    assert_eq!(drv.idle_wakes(30_000), 0);
    drv.resume().pump(300);
    drv.keys("enter").pump(1500);
    assert_eq!(count(&d, Kind::Photo), 2, "shoots again after the return");
}
