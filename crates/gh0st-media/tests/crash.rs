//! The helper dying: the file it was reading is marked and not asked for
//! again, the music goes on from the next song, and a helper that dies
//! whatever it is asked is started only a few times.

use gh0st_app::testing::Driver;
use gh0st_media::app::Media;
use gh0st_media::fake::crashes;
use gh0st_media::helper::Source;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Every Driver reaps any child with waitpid(-1): one test at a time, so
/// none takes another's helper.
static ONE: Mutex<()> = Mutex::new(());

fn one() -> std::sync::MutexGuard<'static, ()> {
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

fn home(name: &str, files: &[&str]) -> PathBuf {
    let h = std::env::temp_dir().join(format!("gh0st-media-crash-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).unwrap();
    for f in files {
        let p = h.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    h
}

/// Each key, then time and a frame, as on the device.
fn step(d: &mut Driver, keys: &str) {
    for k in keys.lines() {
        d.keys(k);
        d.pump(300);
        d.frame();
    }
}

#[test]
fn a_tile_that_kills_the_helper_is_asked_for_once() {
    let _one = one();
    let h = home(
        "tile",
        &["Pictures/a.jpg", "Pictures/crash.jpg", "Pictures/b.jpg"],
    );
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nenter");
    d.pump(1000);
    assert_eq!(crashes(&h.join("Pictures/crash.jpg")), 1);
    // Still there, and still answering.
    step(&mut d, "right\nenter");
    assert!(d.toast().is_none());
}

#[test]
fn a_song_that_kills_the_helper_is_passed_over() {
    let _one = one();
    let h = home(
        "song",
        &["Music/Album/1 crash.mp3", "Music/Album/2 fine.mp3"],
    );
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    // Music, Album, the first song: its tile, then its tags, end the fake.
    step(&mut d, "x\nx\nx\nenter\nenter\nenter");
    d.pump(1000);
    assert_eq!(crashes(&h.join("Music/Album/1 crash.mp3")), 2);
    d.pause().pump(300);
    let n = d.notices();
    assert_eq!(n.len(), 1, "the next song plays");
    assert_eq!(n[0].title, "fine");
}

fn lines(log: &Path) -> Vec<i32> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn kill_all(log: &Path) {
    for pid in lines(log) {
        // SAFETY: a signal to a helper this test started.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// A helper that says its pid in `log` as it starts.
fn sh(script: String) -> Source {
    Source::Exec(vec!["/bin/sh".into(), "-c".into(), script])
}

#[test]
fn a_helper_that_dies_as_it_starts_is_tried_four_times() {
    let _one = one();
    let h = home("dies", &[]);
    let log = h.join("starts");
    let mut d = Driver::new(Media::with(
        sh(format!("echo $$ >> '{}'", log.display())),
        h.clone(),
        None,
    ));
    d.pump(1500);
    assert_eq!(lines(&log).len(), 4, "the first and three more");
    d.frame();
}

#[test]
fn a_helper_that_died_is_started_again_once() {
    let _one = one();
    let h = home("once", &[]);
    let log = h.join("starts");
    let flag = h.join("started");
    // The first hangs up, then exits a moment later, so the app hears
    // both. The next lives.
    let script = format!(
        "echo $$ >> '{log}'; [ -e '{flag}' ] && exec sleep 5; touch '{flag}'; \
         eval \"exec ${{{fd}}}>&-\"; sleep 0.3; exit 3",
        log = log.display(),
        flag = flag.display(),
        fd = gh0st_media::helper::FD_ENV,
    );
    let mut d = Driver::new(Media::with(sh(script), h.clone(), None));
    d.pump(1000);
    let started = lines(&log).len();
    drop(d);
    kill_all(&log);
    assert_eq!(started, 2, "its hangup and its exit start one helper");
}
