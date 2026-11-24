//! The media app's real page over the fake helper, into mockups/media. Run
//! from the workspace root:
//!
//!     cargo run -p gh0st-media --example mockups
//!
//! As a test it draws every scene under the design lint.

use gh0st_app::testing::Driver;
use gh0st_media::app::Media;
use gh0st_media::helper::Source;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// A home with pictures, videos and an album. The fake helper paints each
/// file by its name.
fn home(name: &str) -> PathBuf {
    let h = std::env::temp_dir().join(format!("gh0st-media-mockups-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&h);
    let base = 1_795_430_000u64;
    let mut n = 0u64;
    let mut file = |p: PathBuf, bytes: u64| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(bytes).unwrap();
        n += 1;
        f.set_modified(UNIX_EPOCH + Duration::from_secs(base + n * 3600))
            .unwrap();
    };
    let pics = h.join("Pictures");
    for i in 0..6 {
        file(
            pics.join(format!("Trip/IMG_{:04}.jpg", 4100 + i)),
            3_100_000,
        );
    }
    file(pics.join("DCIM/Camera/CAM_20261123_101500.jpg"), 3_700_000);
    for (name, bytes) in [
        ("harbour.heic", 2_400_000),
        ("lights.gif", 900_000),
        ("broken.png", 4_000),
        ("tall-tower.png", 6_200_000),
        ("ridge-pano.jpg", 9_800_000),
        ("sunset.jpg", 4_100_000),
        ("walk.mp4", 48_000_000),
        ("street.webp", 1_200_000),
        ("garden.avif", 800_000),
    ] {
        file(pics.join(name), bytes);
    }
    file(h.join("Videos/movie night.mkv"), 1_400_000_000);
    file(h.join("Videos/clip.mp4"), 22_000_000);
    let album = h.join("Music/Night Signals - Arcade");
    file(album.join("cover.jpg"), 120_000);
    for (i, t) in [
        "Neon Rain",
        "Night Drive",
        "Static Hearts",
        "Low Orbit",
        "Afterglow",
        "Tunnel Lights",
    ]
    .iter()
    .enumerate()
    {
        file(album.join(format!("{:02} {t}.flac", i + 1)), 31_000_000);
    }
    file(h.join("Music/Field Notes/01 Morning.mp3"), 7_000_000);
    file(h.join("Music/Field Notes/cover.jpg"), 90_000);
    file(h.join("Music/Loose/tape nocover.mp3"), 4_000_000);
    file(h.join("Music/Loose/voice memo nocover.m4a"), 900_000);
    h
}

fn media(name: &str, source: Source) -> Driver {
    let mut d = Driver::new(Media::with(source, home(name), None));
    d.pump(100);
    // A list learns its rows when it is drawn, as the first frame does.
    d.frame();
    d
}

/// Runs each scene and hands the driver over at the moment to show.
fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let mut d = media("start", Source::Fake);
    each("01-start", &mut d);
    d.keys("x\nenter").pump(400);
    each("02-grid-pictures", &mut d);

    let mut d = media("music-grid", Source::Fake);
    d.keys("x\nx\nx\nenter").pump(400);
    each("03-grid-music", &mut d);

    // Pictures, newest first: garden, street, walk, sunset, ridge, tall,
    // broken, lights, harbour, then the folders ahead of them.
    let mut d = media("viewer", Source::Fake);
    d.keys("x\nenter").pump(300);
    d.keys("c\nc\nc\nc\nc\nenter").pump(400);
    each("04-viewer-fit", &mut d);
    d.keys("c").pump(400);
    each("05-viewer-panorama", &mut d);
    d.keys("6").pump(400);
    each("06-viewer-panorama-filled", &mut d);
    d.keys("z\nz").pump(400);
    each("07-viewer-panorama-panned", &mut d);
    d.keys("6\nz").pump(400);
    d.keys("7\n7\nf").pump(400);
    each("08-viewer-zoomed", &mut d);
    d.keys("5\n5\nenter").pump(300);
    each("09-viewer-details", &mut d);

    let mut d = media("broken", Source::Fake);
    d.keys("x\nenter").pump(300);
    d.keys("c\nc\nc\nc\nc\nc\nc\nc\nenter").pump(400);
    each("10-viewer-broken", &mut d);

    let mut d = media("video", Source::Fake);
    d.keys("x\nx\nenter").pump(300);
    d.keys("c\nenter").pump(400);
    each("11-video-ready", &mut d);
    d.keys("enter").pump(2300);
    each("12-video-playing", &mut d);
    d.keys("8").pump(100);
    d.keys("x\nx\nx\nx\nx\nx").pump(100);
    each("13-video-menu", &mut d);

    let mut d = media("music", Source::Fake);
    d.keys("x\nx\nx\nenter").pump(300);
    d.keys("c\nenter").pump(300);
    d.keys("enter").pump(400);
    d.keys("c").pump(1500);
    each("14-music-playing", &mut d);
    d.keys("q").pump(100);
    d.frame();
    d.keys("x").pump(50);
    each("15-music-queue", &mut d);

    // The menu over the grid: the file chooser's two ways in.
    let mut d = media("menu", Source::Fake);
    d.keys("x\nenter").pump(400);
    d.keys("8").pump(100);
    d.frame();
    d.keys("x\nx\nx\nx\nx\nx").pump(100);
    each("17-menu", &mut d);

    // Songs with no cover of their own: a note on the tile.
    let mut d = media("nocover", Source::Fake);
    d.keys("x\nx\nx\nenter").pump(400);
    d.frame();
    d.keys("c\nenter").pump(400);
    each("18-grid-songs-no-cover", &mut d);

    // The start page's last row asks the file chooser for any folder.
    let mut d = media("other", Source::Fake);
    d.keys("x\nx\nx\nx\nx").pump(100);
    each("19-start-other-folder", &mut d);

    let mut d = media("missing", Source::Missing);
    d.keys("x\nenter").pump(100);
    each("16-no-helper", &mut d);
}

fn main() {
    let out = Path::new("mockups/media");
    std::fs::create_dir_all(out).unwrap();
    // GH0ST_PALETTE=<name> renders the scenes in another palette.
    let palette = std::env::var("GH0ST_PALETTE").ok();
    each_scene(|name, d| {
        if let Some(p) = &palette {
            d.palette(p);
        }
        d.save(&out.join(format!("{name}.png"))).unwrap()
    });
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
        d.palette(gh0st_app::gfx::theme::PALETTES[0].name);
    });
    assert!(found.is_empty(), "{found:#?}");
}
