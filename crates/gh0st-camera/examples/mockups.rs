//! The camera's real page over the fake helper, into mockups/camera. Run
//! from the workspace root:
//!
//!     cargo run -p gh0st-camera --example mockups
//!
//! As a test it draws every scene under the design lint.

use gh0st_app::testing::Driver;
use gh0st_camera::app::Camera;
use gh0st_camera::helper::Source;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// A folder of earlier shots, a few minutes apart.
fn fixtures(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gh0st-camera-mockups-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let base = 1_795_430_000;
    // Fourteen shots, every fourth a video. A photo starts with a real
    // frame header, so the full view knows it is 3280x2464.
    let sof: &[u8] = &[
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x09, 0xa0, 0x0c, 0xd0, 0x03,
    ];
    for i in 0..14u64 {
        let t = base + i * 420;
        let video = i % 4 == 1;
        let stem = gh0st_camera::files::stem(if video { "VID" } else { "CAM" }, t as i64);
        let p = dir.join(format!("{stem}.{}", if video { "mp4" } else { "jpg" }));
        let mut f = std::fs::File::create(&p).unwrap();
        if !video {
            std::io::Write::write_all(&mut f, sof).unwrap();
        }
        f.set_len(if video {
            48_201_000
        } else {
            3_712_000 + i * 97_000
        })
        .unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(t)).unwrap();
    }
    dir
}

fn camera(name: &str, camera: bool) -> Driver {
    let mut d = Driver::new(Camera::with(Source::Fake { camera }, fixtures(name)));
    d.pump(150);
    d
}

/// Runs each scene and hands the driver over at the moment to show.
fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let mut d = camera("photo", true);
    each("01-photo", &mut d);
    d.keys("c\nc\nc\nf\nf\ng\nt").pump(100);
    each("02-photo-zoom-light-grid-timer", &mut d);
    d.keys("esc\nenter").pump(100);
    each("03-photo-timer-counting", &mut d);
    d.wait(3000).pump(200);
    each("04-photo-just-taken", &mut d);

    let mut d = camera("video", true);
    d.keys("tab").pump(150);
    each("05-video", &mut d);
    d.keys("enter").pump(1200);
    each("06-video-recording", &mut d);
    d.keys("enter").pump(150);

    let mut d = camera("menu", true);
    d.keys("8").pump(50);
    each("07-menu", &mut d);
    d.keys("esc\nhelp").pump(50);
    each("08-help", &mut d);

    let mut d = camera("menu-video", true);
    d.keys("tab").pump(150);
    d.keys("8").pump(50);
    d.keys("x\nx\nx\nx\nx\nx\nx\nx\nx").pump(50);
    each("08-menu-video", &mut d);

    let mut d = camera("review", true);
    d.keys("p").pump(300);
    each("09-gallery", &mut d);
    d.keys("c\nc\nx").pump(300);
    each("10-gallery-moved", &mut d);
    d.keys("enter").pump(300);
    each("11-photo-full", &mut d);
    d.keys("7\n7\nc\nenter").pump(300);
    each("12-photo-zoomed", &mut d);

    // The newest shot is a video.
    let mut d = camera("play", true);
    d.keys("p").pump(300);
    d.keys("enter").pump(300);
    each("13-video-full", &mut d);
    d.keys("enter").pump(2300);
    each("14-video-playing", &mut d);
    d.keys("enter").pump(200);
    each("15-video-paused", &mut d);

    let mut d = camera("lite", false);
    each("16-no-camera", &mut d);
}

fn main() {
    let out = Path::new("mockups/camera");
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
