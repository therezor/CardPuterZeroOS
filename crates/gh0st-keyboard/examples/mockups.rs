//! Keyboard's real pages over a fake link, into mockups/keyboard. Run from
//! the workspace root:
//!
//!     cargo run -p gh0st-keyboard --example mockups

use gh0st_app::testing::Driver;
use gh0st_keyboard::pages::hosts::Hosts;
use std::path::{Path, PathBuf};

struct Scene {
    name: &'static str,
    keys: &'static str,
}

const SCENES: &[Scene] = &[Scene {
    name: "01-empty",
    keys: "",
}];

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/keyboard");
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

fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    for s in SCENES {
        let mut d = Driver::new(Hosts::new());
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
}

fn save(d: &mut Driver, out: &Path, name: &str) {
    d.save(&out.join(format!("{name}.png"))).unwrap();
}
