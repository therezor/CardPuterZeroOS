//! Renders the Keyring app's screens into `mockups/keyring`, from the real
//! app on the real Secret Service with made-up secrets:
//!
//!     cargo run -p gh0st-keyring --example mockups

#[path = "../tests/common/fake.rs"]
#[allow(dead_code)]
mod fake;

use gh0st_app::testing::Driver;
use gh0st_keyring::app::Home;
use std::path::PathBuf;

/// A keyring with a few apps' secrets in it.
fn filled(name: &str) -> fake::Fake {
    let mut f = fake::Fake::new(name);
    let gh = f.seed(
        "exe:/usr/bin/git-credential-libsecret",
        "GitHub token",
        &[("service", "github.com")],
        b"ghp_x",
    );
    f.seed(
        "chromium",
        "Chromium Safe Storage",
        &[("application", "chromium")],
        b"k",
    );
    f.seed(
        "exe:/usr/bin/python3",
        "Mastodon",
        &[("service", "mastodon")],
        b"m",
    );
    f.seed(
        "networkmanager",
        "Home Wi-Fi",
        &[("xdg:schema", "org.freedesktop.NetworkManager.Connection")],
        b"wifi-pass",
    );
    f.approve("exe:/usr/bin/python3", &gh);
    f
}

struct Scene {
    name: &'static str,
    locked: bool,
    keys: &'static str,
}

const SCENES: &[Scene] = &[
    Scene {
        name: "01-home",
        locked: false,
        keys: "",
    },
    Scene {
        name: "02-locked",
        locked: true,
        keys: "",
    },
    Scene {
        name: "03-passwords",
        locked: false,
        keys: "enter",
    },
    Scene {
        name: "04-item",
        locked: false,
        keys: "enter|enter",
    },
    Scene {
        name: "05-item-shown",
        locked: false,
        keys: "enter|enter|5",
    },
    Scene {
        name: "06-wifi",
        locked: false,
        keys: "x|enter",
    },
    Scene {
        name: "07-access",
        locked: false,
        keys: "x|x|enter",
    },
    Scene {
        name: "08-add",
        locked: false,
        keys: "7|type GitLab",
    },
];

fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    for s in SCENES {
        let mut f = filled(s.name);
        if s.locked {
            f.svc.lock();
        }
        let mut d = Driver::new(Home::new(f.shared()));
        d.wait(50);
        // A key script runs its keys 30 ms apart: a page pushed by one
        // needs a moment before the next.
        for chunk in s.keys.split('|').filter(|c| !c.is_empty()) {
            d.keys(chunk);
            d.wait(100);
        }
        d.wait(50);
        each(s.name, &mut d);
    }
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/keyring");
    std::fs::create_dir_all(&out).unwrap();
    each_scene(|name, d| d.save(&out.join(format!("{name}.png"))).unwrap());
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
