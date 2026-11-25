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
    assert!(
        (140..180).all(|x| f.get(x, 2) != 0),
        "the picture reaches the top"
    );
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

#[test]
fn esc_on_a_file_another_app_opened_goes_back_to_that_app() {
    let h = home("back");
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/a.jpg")),
    ));
    d.pump(300);
    d.keys("esc");
    assert!(d.quit(), "Esc leaves for the app that opened the file");

    // Browsed to by hand, the same picture goes back to its folder.
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.keys("x\nenter");
    d.pump(300);
    d.keys("enter");
    d.pump(300);
    d.keys("esc");
    assert!(!d.quit());
}

#[test]
fn leaving_a_song_another_app_opened_keeps_it_playing() {
    let h = home("song");
    std::fs::create_dir_all(h.join("Music")).unwrap();
    std::fs::write(h.join("Music/song.mp3"), b"x").unwrap();
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Music/song.mp3")),
    ));
    d.pump(300);
    d.keys("esc");
    assert!(d.left(), "steps back to the opener instead of closing");
}

fn songs(name: &str) -> PathBuf {
    let h = home(name);
    for d in [
        "Music/Album",
        "Music/Other",
        "Pictures/DCIM/Camera",
        "Videos",
    ] {
        std::fs::create_dir_all(h.join(d)).unwrap();
    }
    for f in [
        "Music/Album/1.mp3",
        "Music/Album/2.mp3",
        "Music/Other/x.mp3",
    ] {
        std::fs::write(h.join(f), b"x").unwrap();
    }
    h
}

/// Each key, then time and a frame, as on the device: a list learns its
/// rows when drawn.
fn step(d: &mut Driver, keys: &str) {
    for k in keys.lines() {
        d.keys(k);
        d.pump(300);
        d.frame();
    }
}

#[test]
fn music_plays_on_through_other_folders() {
    let h = songs("folders");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    // Music place, Album, the first song.
    step(&mut d, "x\nx\nx\nenter\nenter\nenter");
    assert_eq!(d.status(), None, "on its own page, no word");
    step(&mut d, "esc");
    assert_eq!(d.status().as_deref(), Some("Playing"), "behind the grid");
    step(&mut d, "esc");
    assert_eq!(d.status().as_deref(), Some("Playing"), "in another folder");
}

#[test]
fn behind_another_app_the_music_has_a_notice_with_pause_and_next() {
    let h = songs("notice");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nx\nx\nenter\nenter\nenter");
    assert!(d.notices().is_empty(), "none while in front");
    d.pause().pump(100);
    let n = d.notices();
    assert_eq!(n.len(), 1);
    let ids: Vec<&str> = n[0].actions.iter().map(|(i, _)| i.as_str()).collect();
    assert_eq!(ids, ["pause", "next"]);
    d.notice_action("playing", "pause").pump(100);
    assert_eq!(
        d.notices()[0].actions[0].0,
        "play",
        "the chip turns to Play"
    );
    let title = d.notices()[0].title.clone();
    d.notice_action("playing", "next").pump(300);
    assert_ne!(d.notices()[0].title, title, "the next song");
    d.resume().pump(100);
    assert!(d.notices().is_empty(), "gone in front");
}

#[test]
fn low_memory_keeps_the_picture_on_screen() {
    let h = home("lowmem");
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/a.jpg")),
    ));
    d.pump(300);
    let before = d.frame().pixels().to_vec();
    d.low_memory(255).pump(300);
    assert_eq!(d.frame().pixels(), &before[..]);
}

#[test]
fn the_chooser_opens_a_folder_or_a_file_and_nothing_picked_changes_nothing() {
    let h = songs("pick");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    // The start page's last row: Other folder.
    step(&mut d, "x\nx\nx\nx\nx\nenter");
    let asked = d.picks();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].mode, gh0st_app::PickMode::Folder);
    d.answer_pick(Some(vec![h.join("Music/Album")]));
    d.frame();
    assert_eq!(d.title(), "Album");

    // The menu's Open file, from the grid: several types, several files.
    step(&mut d, "o");
    let asked = d.picks();
    assert_eq!(asked[0].mode, gh0st_app::PickMode::Open);
    assert!(asked[0].multiple);
    assert_eq!(asked[0].filters.len(), 4);
    assert_eq!(
        asked[0].folder.as_deref(),
        Some(h.join("Music/Album").as_path())
    );
    d.answer_pick(None);
    assert_eq!(d.title(), "Album", "nothing picked, nothing changes");
    step(&mut d, "o");
    d.answer_pick(Some(vec![h.join("Pictures/b.png")]));
    d.pump(300);
    step(&mut d, "esc");
    assert!(
        !d.quit() && !d.left(),
        "picked here: Esc goes back to the folder"
    );
    assert_eq!(d.title(), "Pictures");
}

#[test]
fn a_long_clip_goes_on_from_where_it_stopped() {
    let h = home("resume");
    std::fs::create_dir_all(h.join("Videos")).unwrap();
    std::fs::write(h.join("Videos/movie night.mkv"), b"x").unwrap();
    let film = h.join("Videos/movie night.mkv");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), Some(film.clone())));
    d.pump(300);
    d.frame();
    // Play, then 30 s on; leaving keeps the place.
    step(&mut d, "enter\n7\n7\n7");
    step(&mut d, "esc");
    assert!(d.quit());
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), Some(film)));
    d.pump(300);
    d.frame();
    step(&mut d, "enter");
    assert!(
        d.toast().is_some_and(|t| t.starts_with("From 00:3")),
        "{:?}",
        d.toast()
    );
}

#[test]
fn the_grid_and_a_paused_video_rest_and_a_slideshow_keeps_awake_only_while_it_runs() {
    let h = home("rest");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nenter");
    d.pump(1000);
    assert!(d.idle_wakes(600_000) <= 11, "the grid");
    step(&mut d, "p");
    assert!(d.awake(), "a slideshow keeps the screen on");
    step(&mut d, "p");
    assert!(!d.awake(), "and lets it go");
    step(&mut d, "esc");
    // The video, played and paused.
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/d.mp4")),
    ));
    d.pump(300);
    d.frame();
    step(&mut d, "enter");
    d.pump(1000);
    step(&mut d, "enter");
    d.pump(1000);
    assert!(!d.awake(), "paused, the screen may go dark");
    assert!(d.idle_wakes(600_000) <= 11, "a paused video");
}

#[test]
fn the_helper_lets_go_of_a_picture_once_nothing_shows_it() {
    use gh0st_media::fake::forgotten;
    use std::time::{Duration, SystemTime};
    let h = home("forget");
    let trip = h.join("Pictures/Trip");
    std::fs::create_dir_all(&trip).unwrap();
    // Newest first: the clip, then the photo.
    for (n, age) in [("x.jpg", 60), ("y.mp4", 0)] {
        let f = std::fs::File::create(trip.join(n)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age))
            .unwrap();
    }
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "o");
    d.answer_pick(Some(vec![trip.join("x.jpg")]));
    d.pump(300);
    d.frame();
    assert_eq!(forgotten(&trip), 0, "the photo shows");
    step(&mut d, "left");
    assert_eq!(forgotten(&trip), 1, "the clip took the screen");
    step(&mut d, "esc");
    let before = forgotten(&trip);
    step(&mut d, "right\nenter");
    step(&mut d, "esc");
    assert_eq!(forgotten(&trip), before + 1, "back to the grid");
}

/// Seeks `song` to its end and presses `then` with the last keys: the end
/// is heard after it. A key at a time to 25 s short, so the link keeps up,
/// then 60 s at once, for a helper slow to say where it is.
fn to_the_end(d: &mut Driver, song: &std::path::Path, then: &str) {
    let dur = gh0st_media::fake::info_of(&song.to_string_lossy())
        .unwrap()
        .dur_ms;
    for _ in 0..(dur - 25_000) / 10_000 {
        d.keys("f").pump(30);
    }
    d.keys(&format!("f\nf\nf\nf\nf\nf{then}"));
    d.pump(500);
}

#[test]
fn behind_the_grid_the_next_song_is_the_one_named() {
    let h = songs("handover");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nx\nx\nenter\nenter\nenter");
    // The song ends while the grid shows its tile: the next one plays.
    to_the_end(&mut d, &h.join("Music/Album/1.mp3"), "\nesc");
    assert_eq!(d.status().as_deref(), Some("Playing"));
    d.pause().pump(300);
    assert_eq!(d.notices()[0].title, "2", "not the tile selected");
}

#[test]
fn repeat_turned_off_on_the_last_song_ends_the_music_there() {
    let h = songs("repeat");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    // The album's last song, repeat on and off again.
    step(&mut d, "x\nx\nx\nenter\nenter\nright\nenter\nr\nr");
    to_the_end(&mut d, &h.join("Music/Album/2.mp3"), "");
    d.pause().pump(300);
    assert!(d.notices().is_empty(), "nothing plays on");
    d.resume().pump(100);
    // Play again: the queue, as music.
    step(&mut d, "enter\nesc");
    assert_eq!(d.status().as_deref(), Some("Playing"));
}

#[test]
fn a_song_that_cant_play_leaves_no_word_that_it_plays() {
    let h = songs("playerr");
    std::fs::write(h.join("Music/Album/0 broken.mp3"), b"x").unwrap();
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nx\nx\nenter\nenter\nenter");
    assert!(d.toast().is_some_and(|t| t.starts_with("Can't play")));
    step(&mut d, "esc");
    assert_eq!(d.status(), None);
}

#[test]
fn under_music_the_arrows_still_step_through_pictures() {
    let h = songs("arrows");
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    step(&mut d, "x\nx\nx\nenter\nenter\nenter\no");
    d.answer_pick(Some(vec![h.join("Pictures/a.jpg")]));
    d.pump(300);
    let before = d.frame().pixels().to_vec();
    step(&mut d, "right");
    assert!(d.frame().pixels() != &before[..], "the next picture");
    assert_eq!(d.status().as_deref(), Some("Playing"));
}

#[test]
fn a_moving_picture_moves_and_the_arrows_pass_it() {
    use std::time::{Duration, SystemTime};
    let h = home("anim");
    let dir = h.join("Pictures/Anim");
    std::fs::create_dir_all(&dir).unwrap();
    // Newest first: the moving one, then a still one.
    for (n, age) in [("lights.gif", 0), ("still.jpg", 60)] {
        let f = std::fs::File::create(dir.join(n)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age))
            .unwrap();
    }
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(dir.join("lights.gif")),
    ));
    d.pump(300);
    let a = d.frame().pixels().to_vec();
    d.pump(300);
    assert!(d.frame().pixels() != &a[..], "it moves");
    assert!(!d.awake(), "a picture doesn't hold the screen on");
    step(&mut d, "right");
    let b = d.frame().pixels().to_vec();
    d.pump(300);
    assert!(d.frame().pixels() == &b[..], "the next one is still");
}

#[test]
fn a_file_the_folder_hides_opens_all_the_same() {
    let h = home("hidden");
    std::fs::write(h.join("Pictures/.secret.jpg"), b"x").unwrap();
    let mut d = Driver::new(Media::with(
        Source::Fake,
        h.clone(),
        Some(h.join("Pictures/./.secret.jpg")),
    ));
    d.pump(300);
    let f = d.frame();
    assert!((140..180).all(|x| f.get(x, 2) != 0), "on the whole screen");
    assert!(d.toast().is_none());
}

#[test]
fn a_slideshow_starts_at_a_picture_and_ends_with_its_folder() {
    let h = home("slides");
    std::fs::create_dir_all(h.join("Pictures/Trip")).unwrap();
    let mut d = Driver::new(Media::with(Source::Fake, h.clone(), None));
    d.pump(300);
    d.frame();
    // The first tile is the folder: the show starts at the picture after.
    step(&mut d, "x\nenter\np");
    assert_eq!(d.title(), "Media", "a picture, not the folder");
    assert!(d.awake());
    step(&mut d, "g");
    d.answer_pick(Some(vec![h.join("Pictures/Trip")]));
    d.frame();
    assert_eq!(d.title(), "Trip");
    assert!(!d.awake(), "the show ended");
}
