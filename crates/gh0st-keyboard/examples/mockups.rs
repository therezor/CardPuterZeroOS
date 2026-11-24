//! Keyboard's real screens over fake links, into mockups/keyboard. Run
//! from the workspace root:
//!
//!     cargo run -p gh0st-keyboard --example mockups

use std::path::{Path, PathBuf};

use gh0st_app::testing::Driver;
use gh0st_keyboard::app::{App, Open, Target};
use gh0st_keyboard::host::System;
use gh0st_keyboard::link::fake::{FakeBt, FakeCable};
use gh0st_keyboard::link::{Advert, Ask, Bluetooth, Event, UsbState};
use gh0st_keyboard::pages::session::{MEDIA, MOUSE, SLIDES, SNIPPETS, TYPE};
use gh0st_keyboard::store::{self, Host, Snippet};

const MAC: &str = "/org/bluez/hci0/dev_F0_18_98_00_00_01";
const PIXEL: &str = "/org/bluez/hci0/dev_3C_28_6D_00_00_02";
const PC: &str = "/org/bluez/hci0/dev_00_1A_7D_00_00_03";

fn addr(path: &str) -> String {
    gh0st_keyboard::app::address_of(path).unwrap_or_default()
}

fn hosts() -> Vec<Host> {
    let mut mac = Host::new(&addr(MAC), "MacBook", System::Mac);
    mac.used = 30;
    let mut pixel = Host::new(&addr(PIXEL), "Pixel 9", System::Android);
    pixel.used = 20;
    let mut pc = Host::new(&addr(PC), "Office PC", System::Windows);
    pc.used = 10;
    vec![mac, pixel, pc]
}

fn bt_connected() -> FakeBt {
    FakeBt::new()
        .host(MAC, "MacBook", "computer", true)
        .host(PIXEL, "Pixel 9", "phone", false)
        .host(PC, "Office PC", "computer", false)
}

struct Scene {
    name: &'static str,
    keys: &'static str,
    make: fn(&Path) -> App,
}

fn dir_with(base: &Path, hosts: &[Host]) -> PathBuf {
    std::fs::create_dir_all(base).unwrap();
    store::save_hosts(base, hosts).unwrap();
    store::save_snippets(
        base,
        &[
            Snippet {
                name: "Email".into(),
                text: "me@example.com".into(),
            },
            Snippet {
                name: "Sign-off".into(),
                text: "Thanks,\nAlex".into(),
            },
        ],
    )
    .unwrap();
    base.to_path_buf()
}

fn mac_session(d: &Path, mode: usize) -> App {
    App::with(
        Box::new(bt_connected()),
        Box::new(FakeCable::new(UsbState::NoCable)),
        dir_with(d, &hosts()),
    )
    .opening(Some(Target::Bt(addr(MAC))), Open::Session(mode))
}

const SCENES: &[Scene] = &[
    Scene {
        name: "01-first-run",
        keys: "",
        make: |d| {
            App::with(
                Box::new(FakeBt::new()),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &[]),
            )
        },
    },
    Scene {
        name: "02-hosts",
        keys: "",
        make: |d| {
            App::with(
                Box::new(bt_connected()),
                Box::new(FakeCable::new(UsbState::HostMode)),
                dir_with(d, &hosts()),
            )
            .opening(Some(Target::Bt(addr(MAC))), Open::Hosts)
        },
    },
    Scene {
        name: "03-hosts-menu",
        keys: "8",
        make: |d| {
            App::with(
                Box::new(bt_connected()),
                Box::new(FakeCable::new(UsbState::NoCable)),
                dir_with(d, &hosts()),
            )
        },
    },
    Scene {
        name: "04-pair-waiting",
        keys: "",
        make: |d| {
            App::with(
                Box::new(FakeBt::new()),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &[]),
            )
            .opening(None, Open::Pair)
        },
    },
    Scene {
        name: "05-pair-code",
        keys: "",
        make: |d| {
            let mut bt = FakeBt::new().host(PIXEL, "Pixel 9", "phone", false);
            bt.devices[0].paired = false;
            bt.asking = Some(Ask::Confirm(482_913));
            bt.wanted = Some(Advert::Pair);
            bt.queue.push(Event::Pairing {
                device: PIXEL.into(),
                ask: Ask::Confirm(482_913),
            });
            App::with(
                Box::new(bt),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &[]),
            )
            .opening(None, Open::Pair)
        },
    },
    Scene {
        name: "06-paired",
        keys: "",
        make: |d| {
            App::with(
                Box::new(bt_connected()),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &hosts()),
            )
            .opening(None, Open::Profile(addr(PIXEL)))
        },
    },
    Scene {
        name: "07-type",
        keys: "type Hello there\nctrl+c\nenter\ntype ok",
        make: |d| mac_session(d, TYPE),
    },
    Scene {
        name: "08-type-latch",
        keys: "type echo hi\nfn+alt",
        make: |d| {
            App::with(
                Box::new(FakeBt::new().host(PC, "Office PC", "computer", true)),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &hosts()),
            )
            .opening(Some(Target::Bt(addr(PC))), Open::Session(TYPE))
        },
    },
    Scene {
        name: "09-type-waiting",
        keys: "",
        make: |d| {
            let mut bt = bt_connected();
            bt.disconnect(MAC);
            bt.wanted = Some(Advert::Reconnect);
            App::with(
                Box::new(bt),
                Box::new(FakeCable::new(UsbState::Off)),
                dir_with(d, &hosts()),
            )
            .opening(Some(Target::Bt(addr(MAC))), Open::Session(TYPE))
        },
    },
    Scene {
        name: "10-modes",
        keys: "fn+space\nc",
        make: |d| mac_session(d, TYPE),
    },
    Scene {
        name: "11-mouse",
        keys: "down f\nwait 200",
        make: |d| mac_session(d, MOUSE),
    },
    Scene {
        name: "12-media",
        keys: "",
        make: |d| mac_session(d, MEDIA),
    },
    Scene {
        name: "13-slides",
        keys: "6\nwait 100\nspace\nspace\nspace",
        make: |d| mac_session(d, SLIDES),
    },
    Scene {
        name: "14-snippets",
        keys: "",
        make: |d| mac_session(d, SNIPPETS),
    },
    Scene {
        name: "15-usb-switch",
        keys: "",
        make: |d| {
            App::with(
                Box::new(FakeBt::new()),
                Box::new(FakeCable::new(UsbState::HostMode)),
                dir_with(d, &[]),
            )
            .opening(Some(Target::Usb), Open::Session(TYPE))
        },
    },
    Scene {
        name: "16-help",
        keys: "help",
        make: |d| mac_session(d, TYPE),
    },
];

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
    let base = std::env::temp_dir().join(format!("gh0st-keyboard-mockups-{}", std::process::id()));
    for s in SCENES {
        let dir = base.join(s.name);
        let mut d = Driver::new((s.make)(&dir));
        d.wait(50);
        for step in s.keys.split('\n').filter(|k| !k.is_empty()) {
            if let Some(ms) = step.strip_prefix("wait ") {
                d.wait(ms.parse().unwrap());
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
