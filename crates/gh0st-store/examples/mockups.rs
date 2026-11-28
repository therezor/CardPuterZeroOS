//! The Store's real page over fixture files, as the engine writes them,
//! into mockups/store. Run from the workspace root:
//!
//!     GH0ST_STORE_ART=<dir> cargo run -p gh0st-store --example mockups
//!
//! `GH0ST_STORE_ART` is optional: a folder of `<package>.png` icons (48 px,
//! as CI makes them) and `<package>.shot0.png` screenshots. Without it the
//! apps show letter badges and the screenshot scene shows its error.

use gh0st_app::testing::Driver;
use gh0st_core::store::{About, App, Catalog, Flags, Icons, Outcome, Pinned, State};
use gh0st_core::update::{Phase, Progress};
use gh0st_store::app::Store;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Entries from M5's catalog: package, title, category, version, size,
/// author, flags, summary. Some flags are made up, to show each look.
type CatalogRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u64,
    &'static str,
    &'static str,
    &'static str,
);

const APPS: &[CatalogRow] = &[
    (
        "acidzero",
        "AcidZero",
        "Creative & Office",
        "0.2.0",
        58_940,
        "kurogedelic",
        "hardware",
        "Acid bass synth with a 16-step sequencer and MIDI CC control",
    ),
    (
        "airfiles",
        "Airfiles",
        "System Tools",
        "0.4.0",
        24_292,
        "JackyC",
        "service,network",
        "Network files transfer tool from smartphones and PCs",
    ),
    (
        "battleship",
        "Battleship",
        "Games",
        "1.0.0-2",
        969_402,
        "yutacar",
        "network",
        "Classic naval combat: 1P vs AI or 2P over BLE/WiFi.",
    ),
    (
        "beamhound",
        "Beamhound",
        "System Tools",
        "0.1.0",
        651_780,
        "bfeller",
        "network",
        "Sniffs out your IR remote from one button press and becomes it.",
    ),
    (
        "bitmap16dx",
        "BitMap16 DX",
        "Creative & Office",
        "0.9.1-1",
        229_796,
        "beepbot",
        "imu,network",
        "Draw tiny pixel art, collect palettes, and export your sketches.",
    ),
    (
        "cz-battery-saver",
        "Battery Saver",
        "System Tools",
        "1.1.1",
        17_348,
        "InstantComet",
        "scripts,service,paths,keys",
        "Longer battery life: USB autosuspend, screen-off idle, low-battery shutdown",
    ),
    (
        "cwtrainerzero",
        "CWTrainerZero",
        "Radio & Comms",
        "0.3.0-1",
        710_696,
        "Greg Weir VA1GW",
        "",
        "Offline Morse code practice for Cardputer Zero.",
    ),
    (
        "czmail",
        "Email",
        "Creative & Office",
        "0.3.4",
        53_630,
        "H4W9",
        "microphone,network",
        "Read and send email from your Cardputer Zero.",
    ),
    (
        "doom-zero-dev0",
        "DOOM Zero",
        "Games",
        "0.0.1~dev0",
        14_642_544,
        "KiloGramowy",
        "",
        "Pocket-sized Hell, custom controls and a bloody secret!",
    ),
    (
        "frotz-zero",
        "Frotz Zero",
        "Games",
        "0.8.1-1",
        433_068,
        "Ahazi",
        "network",
        "Play Z-machine adventures with saves, readable text and an online library",
    ),
    (
        "kanstl",
        "KanSTL",
        "System Tools",
        "0.1.0",
        101_736,
        "Weiming Feng",
        "",
        "STL 3D viewer for the Cardputer Zero",
    ),
    (
        "hamlog",
        "HamLog",
        "Radio & Comms",
        "0.1.1",
        2_225_820,
        "Gurkenwerfer",
        "network",
        "Ham Radio Log for Cardputer Zero",
    ),
];

const ABOUT: &[(&str, &str)] = &[
    (
        "acidzero",
        "AcidZero is an acid bass synthesizer for the Cardputer Zero. Enter notes on the \
         keyboard, set slide, accent and rests per step, and shape the sound with cutoff, \
         resonance, envelope mod, decay and accent.\nPress P for settings: MIDI input with \
         channel selection, CC learn for every knob, MIDI notes and start/stop, and output \
         volume.",
    ),
    (
        "cz-battery-saver",
        "Battery Saver cuts the battery drain by about a quarter with the screen on and \
         about 40% with it off.\nIt lets the internal USB Ethernet chip autosuspend, and can \
         silence key sounds so the sound card sleeps.",
    ),
];

/// What dpkg has: two apps current, one behind the catalog, and python3,
/// one of Airfiles' extras.
const INSTALLED: &[(&str, &str)] = &[
    ("acidzero", "0.1.0"),
    ("czmail", "0.3.4"),
    ("frotz-zero", "0.7.0-1"),
    ("python3", "3.13.5-1"),
];

fn sha(c: u8) -> String {
    format!("{:064x}", c)
}

fn catalog() -> Catalog {
    let apps = APPS
        .iter()
        .enumerate()
        .map(|(i, &(p, title, cat, v, size, author, flags, summary))| App {
            package: p.into(),
            version: v.into(),
            size,
            sha256: sha(i as u8 + 1),
            category: cat.into(),
            flags: Flags::parse(flags),
            title: title.into(),
            summary: summary.into(),
            author: author.into(),
            licence: "MIT".into(),
            url: format!("https://github.com/CardputerZero/packages/releases/download/apt-pool/{p}_{v}_arm64.deb"),
            cn_url: String::new(),
            share: String::new(),
            desktop: p.into(),
            pin: false,
            // What the Store installs with it: one the device has, one it lacks.
            extras: if p == "airfiles" { vec!["python3".into(), "zstd".into()] } else { Vec::new() },
        })
        .collect();
    Catalog {
        date: 1_763_870_000,
        expires: 1_766_462_000,
        about: Pinned {
            sha256: sha(0xaa),
            size: 0,
        },
        icons: Pinned {
            sha256: sha(0xbb),
            size: 0,
        },
        apps,
    }
}

fn status() -> String {
    INSTALLED
        .iter()
        .map(|(p, v)| format!("Package: {p}\nStatus: install ok installed\nVersion: {v}\n\n"))
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Fixture {
    /// No catalog yet.
    Empty,
    /// No catalog, and the last refresh failed offline.
    Offline,
    Catalog,
}

struct Scene {
    name: &'static str,
    fixture: Fixture,
    progress: Option<Progress>,
    keys: &'static str,
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/store");
    std::fs::create_dir_all(&out).unwrap();
    each_scene(|name, d| {
        d.save(&out.join(format!("{name}.png"))).unwrap();
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
    });
    assert!(found.is_empty(), "{found:#?}");
}

fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let base = std::env::temp_dir().join(format!("gh0st-store-mockups-{}", std::process::id()));
    let art = std::env::var_os("GH0ST_STORE_ART").map(PathBuf::from);
    let downloading = Progress {
        install: true,
        phase: Phase::Download,
        percent: Some(45),
        done: 26_500,
        total: 58_940,
        n: 0,
        m: 1,
        item: "acidzero".into(),
    };
    let c = Fixture::Catalog;
    let scenes = [
        Scene {
            name: "01-home",
            fixture: c,
            progress: None,
            keys: "",
        },
        Scene {
            name: "02-home-list",
            fixture: c,
            progress: None,
            keys: "8\nx\nx\nx\nx\nc\nesc",
        },
        Scene {
            name: "03-category",
            fixture: c,
            progress: None,
            keys: "x\nwait\nenter",
        },
        Scene {
            name: "04-updates",
            fixture: c,
            progress: None,
            keys: "enter",
        },
        Scene {
            name: "05-search",
            fixture: c,
            progress: None,
            keys: "5\ntype ma",
        },
        Scene {
            name: "06-no-match",
            fixture: c,
            progress: None,
            keys: "5\ntype zzz",
        },
        Scene {
            name: "07-detail-update",
            fixture: c,
            progress: None,
            keys: "enter\nwait\nenter",
        },
        Scene {
            name: "08-detail-new",
            fixture: c,
            progress: None,
            keys: "c\nc\nenter\nwait\nx\nenter",
        },
        Scene {
            name: "09-confirm",
            fixture: c,
            progress: None,
            keys: "c\nc\nenter\nwait\nx\nenter\nwait\nenter",
        },
        Scene {
            name: "10-installing",
            fixture: c,
            progress: Some(downloading.clone()),
            keys: "enter\nwait\nenter",
        },
        Scene {
            name: "11-list-busy",
            fixture: c,
            progress: Some(downloading.clone()),
            keys: "enter",
        },
        Scene {
            name: "12-detail-installed",
            fixture: c,
            progress: None,
            keys: "c\nenter\nwait\nx\nenter",
        },
        Scene {
            name: "13-screenshot",
            fixture: c,
            progress: None,
            keys: "enter\nwait\nenter\nwait\n5",
        },
        Scene {
            name: "14-menu",
            fixture: c,
            progress: None,
            keys: "8",
        },
        Scene {
            name: "15-empty",
            fixture: Fixture::Empty,
            progress: None,
            keys: "",
        },
        Scene {
            name: "16-offline",
            fixture: Fixture::Offline,
            progress: None,
            keys: "",
        },
        Scene {
            name: "17-help",
            fixture: c,
            progress: None,
            keys: "help",
        },
    ];
    for s in scenes {
        let dir = base.join(s.name);
        let (state, run, upd, cache) = (
            dir.join("state"),
            dir.join("run"),
            dir.join("update"),
            dir.join("cache"),
        );
        for d in [&state, &run, &upd, &cache] {
            std::fs::create_dir_all(d).unwrap();
        }
        let status_path = dir.join("status");
        std::fs::write(&status_path, status()).unwrap();
        match s.fixture {
            Fixture::Catalog => write_catalog(&state, art.as_deref(), &cache),
            Fixture::Offline => {
                let st = State {
                    op: "refresh".into(),
                    outcome: Outcome::Offline,
                    ..State::default()
                };
                std::fs::write(state.join("state"), st.text()).unwrap();
            }
            Fixture::Empty => {}
        }
        // The engine holds the update lock while it runs; progress without
        // it is stale.
        let _lock = s.progress.as_ref().map(|p| {
            std::fs::write(run.join("progress"), p.text()).unwrap();
            let f = std::fs::File::create(upd.join("lock")).unwrap();
            // SAFETY: flock on a file this function owns.
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
            f
        });
        // SAFETY: one thread; the page reads these as it starts.
        unsafe {
            std::env::set_var("GH0ST_STORE_STATE", &state);
            std::env::set_var("GH0ST_STORE_RUN", &run);
            std::env::set_var("GH0ST_UPDATE_RUN", &upd);
            std::env::set_var("GH0ST_DPKG_STATUS", &status_path);
            std::env::set_var("XDG_CACHE_HOME", &cache);
            std::env::set_var("TZ", "Europe/London");
            // Settings the scene changes stay in its folder.
            std::env::set_var("HOME", &dir);
        }
        let mut d = Driver::new(Store::new());
        d.wait(100);
        // A list learns its rows when drawn, so a key after a view change
        // waits for a frame, as on the device.
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

/// The catalog, its about file, the icon pack from the art folder, and the
/// first screenshot of AcidZero in the cache as a fetched one would be.
fn write_catalog(state: &Path, art: Option<&Path>, cache: &Path) {
    let mut c = catalog();
    let mut about = String::new();
    let shot = art
        .map(|a| a.join("acidzero.shot0.png"))
        .filter(|p| p.is_file());
    let shot_sha = shot
        .as_deref()
        .and_then(gh0st_core::store::file_sha256)
        .unwrap_or_else(|| sha(0xee));
    for (p, text) in ABOUT {
        let shots = if *p == "acidzero" {
            vec![
                (shot_sha.clone(), "https://example.invalid/0.png".into()),
                (sha(0xef), "https://example.invalid/1.png".into()),
            ]
        } else {
            Vec::new()
        };
        about.push_str(
            &About {
                description: text.to_string(),
                shots,
            }
            .line(p),
        );
    }
    if let Some(src) = &shot {
        let dir = cache.join("gh0st-store/shots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(src, dir.join(format!("{shot_sha}.png"))).unwrap();
    }
    let pngs: Vec<(String, Vec<u8>)> = art
        .map(|a| {
            APPS.iter()
                .filter_map(|app| {
                    let bytes = std::fs::read(a.join(format!("{}.png", app.0))).ok()?;
                    Some((app.0.to_string(), bytes))
                })
                .collect()
        })
        .unwrap_or_default();
    let refs: Vec<(&str, &[u8])> = pngs
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let pack = Icons::build(&refs);
    c.icons.size = pack.len() as u64;
    std::fs::write(state.join("icons"), pack).unwrap();
    std::fs::write(state.join("about"), about).unwrap();
    std::fs::write(state.join("catalog"), c.text()).unwrap();
    let st = State {
        refreshed: gh0st_core::notify::now_secs() - 3600,
        op: "refresh".into(),
        outcome: Outcome::Ok,
        ..State::default()
    };
    std::fs::write(state.join("state"), st.text()).unwrap();
}
