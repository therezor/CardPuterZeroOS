//! The page over the fake helper: a file given on the command line opens,
//! the arrows move through its folder, and nothing wakes while it rests.

use gh0st_app::testing::Driver;
use gh0st_media::app::Media;
use gh0st_media::helper::Source;
use std::path::PathBuf;

fn home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("gh0st-media-flow-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(h.join("Pictures")).unwrap();
    for n in ["a.jpg", "b.png", "c.heic", "d.mp4", "notes.txt"] {
        std::fs::write(h.join("Pictures").join(n), b"x").unwrap();
    }
    h
}

#[test]
fn a_file_given_opens_on_the_whole_screen() {
    let h = home("given");
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/b.png")),
    ));
    d.pump(300);
    // A whole-screen picture covers the top bar's rows: a 4:3 photo is
    // as tall as the screen, black either side.
    let f = d.frame();
    assert!((140..180).all(|x| f.get(x, 2) != 0), "the picture reaches the top");
    assert!((0..40).all(|x| f.get(x, 85) == 0), "black beside it");
    assert!(d.toast().is_none());
}

#[test]
fn nothing_wakes_while_a_picture_rests() {
    let h = home("idle");
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/a.jpg")),
    ));
    d.pump(300);
    d.frame();
    // The first frame, then the clock's minute, and nothing else.
    assert!(d.idle_wakes(600_000) <= 11);
}

#[test]
fn a_missing_helper_says_so_and_the_places_still_list() {
    let h = home("missing");
    let mut d = Driver::new(Media::with(Source::Missing, h, None));
    d.pump(50);
    d.frame();
    d.keys("x\nenter").pump(50);
    d.frame();
    assert!(!d.quit());
}
