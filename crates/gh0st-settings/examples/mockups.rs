//! Renders every Settings page at panel size and 3x, from made-up state, into
//! mockups/settings. Run from the workspace root:
//!
//!     cargo run -p gh0st-settings --example mockups

use gh0st_app::gfx::Canvas;
use gh0st_app::gfx::snapshot::write_png;
use gh0st_app::gfx::theme::{self, Theme};
use gh0st_app::gfx::widgets::{self, Battery, Chip, Hint, Look, Net, Status};
use gh0st_core::apps::Layout;
use gh0st_core::hotspot::Security;
use gh0st_core::sysconf::{self, GpuMem};
use gh0st_settings::model::pages::{
    self, AppRow, Conn, Device, Hotspot, Iface, Logs, Network, State,
};
use gh0st_settings::model::{self, Choice, Cursor, Dialog, PageId};
use std::path::PathBuf;

fn sample_state() -> State {
    let net = |ssid: &str, bars, secure, known, active| Network {
        ssid: ssid.into(),
        bars,
        secure,
        known,
        active,
        enterprise: false,
    };
    let app = |file: &str, name: &str, shown| AppRow {
        file: file.into(),
        name: name.into(),
        shown,
        icon: String::new(),
        background: file == "music.desktop",
        pinned: matches!(file, "files.desktop" | "camera.desktop"),
    };
    State {
        brightness: Some(80),
        dark_secs: 30,
        volume: Some(60),
        muted: false,
        layout: Layout::Grid,
        palette: "Mint",
        animations: true,
        clock_24h: true,
        apps: vec![
            app("files.desktop", "Files", true),
            app("camera.desktop", "Camera", true),
            app("music.desktop", "Music", true),
            app("piano.desktop", "Piano", true),
            app("store.desktop", "Store", true),
            app("zclaw.desktop", "ZClaw", false),
        ],
        wifi_on: true,
        wifi_busy: None,
        profiles: vec!["HomeNet".into()],
        conns: vec![
            ("HomeNet".into(), "802-11-wireless".into()),
            ("Wired connection 1".into(), "802-3-ethernet".into()),
        ],
        conn: Some(Conn {
            name: "HomeNet".into(),
            wifi: true,
            autoconnect: true,
            manual: false,
            address: String::new(),
            gateway: String::new(),
            dns: "1.1.1.1,9.9.9.9".into(),
            mac: "stable".into(),
            active: true,
            current: "192.168.1.42/24".into(),
        }),
        networks: vec![
            net("HomeNet", 3, true, true, true),
            net("Office-5G", 4, true, false, false),
            net("Cafe Guest", 2, false, false, false),
            net("\u{413}\u{430}\u{43b}\u{44f}_WiFi", 1, true, false, false),
        ],
        ifaces: vec![
            Iface {
                name: "wlan0".into(),
                state: "connected".into(),
                connection: "HomeNet".into(),
                ipv4: "192.168.1.42/24".into(),
                mac: "2c:cf:67:0a:41:9e".into(),
                gateway: "192.168.1.1".into(),
                dns: "1.1.1.1, 9.9.9.9".into(),
            },
            Iface {
                name: "eth0".into(),
                state: "unavailable".into(),
                ..Iface::default()
            },
        ],
        hotspot: Hotspot {
            on: false,
            busy: None,
            name: Some("zero-kfwm".into()),
            psk: Some("kfwmtqzrapdh".into()),
            security: Security::Wpa3,
            clients: None,
        },
        bt_on: true,
        bt_scanning: true,
        devices: vec![
            Device {
                mac: "41:42:A1:00:12:9F".into(),
                name: "Buds Pro".into(),
                paired: true,
                connected: true,
            },
            Device {
                mac: "F4:7B:09:22:10:01".into(),
                name: "MX Keys".into(),
                paired: true,
                connected: false,
            },
            Device {
                mac: "5C:EA:1D:8A:3B:70".into(),
                name: String::new(),
                paired: false,
                connected: false,
            },
        ],
        zone: "Europe/London".into(),
        zones: [
            "Africa/Cairo",
            "America/New_York",
            "Asia/Tokyo",
            "Europe/Kyiv",
            "Europe/Lisbon",
            "Europe/London",
            "Europe/Warsaw",
        ]
        .map(String::from)
        .to_vec(),
        network_time: true,
        ssh: false,
        logs: Logs::Off,
        nm: sysconf::NmPrivacy {
            mac_private: true,
            scan_random: true,
            send_hostname: false,
            ip6_privacy: true,
            wifi_powersave: true,
        },
        lock_password: true,
        lock_after: 300,
        dark_wifi: None,
        usb_power: Some(true),
        country_zones: Vec::new(),
        now: "2026-11-21 14:32".into(),
        gpu_mem: GpuMem::Set(128),
        gpu_running: Some(128),
        country: Some("AE".into()),
        country_running: Some("AE".into()),
        countries: sysconf::countries(
            "AE\tUnited Arab Emirates\nDE\tGermany\nGB\tBritain (UK)\nUA\tUkraine\n\
             UG\tUganda\nUS\tUnited States\nUY\tUruguay\nUZ\tUzbekistan\nAD\tAndorra\n",
        ),
        governor: "schedutil".into(),
        hostname: "RZR-ZERO".into(),
        swap_mb: None,
        swap_running: Some(207),
        swap_comp: None,
        swap_comp_running: Some("zstd".into()),
        swappiness: Some(100),
        moving: None,
        updates: updates_state(),
        update_daily: Some(true),
        about: [
            ("Version", "Gh0stdeck 0.1.0"),
            ("Build", "202611221438.2a760d9"),
            ("OS", "Debian 13 (trixie)"),
            ("Kernel", "6.18.39+rpt-rpi-v8"),
            ("Memory", "61 MB of 352 MB used"),
            ("Storage", "1.2 GB of 59 GB used"),
            ("System", "2.4 GB of 3.0 GB used"),
            ("Battery", "87%, 3.98 V"),
            ("Uptime", "2 h 14 min"),
            ("Host", "cardputer"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec(),
    }
}

/// The hotspot running, two phones on it.
fn hotspot_on() -> State {
    let mut st = sample_state();
    st.hotspot.on = true;
    st.hotspot.clients = Some(2);
    st.ifaces[0].connection = "gh0st-hotspot".into();
    st.ifaces[0].ipv4 = "10.42.0.1/24".into();
    st
}

/// Before the hotspot first turned on: no profile yet.
fn hotspot_new() -> State {
    let mut st = sample_state();
    st.hotspot.name = None;
    st.hotspot.psk = None;
    st
}

/// One page as the device shows it: the bars the SDK draws, the page body
/// and any dialog.
fn scene(t: &Theme, st: &State, id: PageId, selected: usize, dialog: Option<Dialog>) -> Canvas {
    let page = pages::build(&id, st);
    let mut cur = Cursor { selected, top: 0 };
    cur.fit(page.len(), model::rows(t).1);
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    let status = Status {
        dock: &[],
        front: None,
        icon: model::icon(&id),
        title: &page.title,
        context: if id == PageId::Root { "" } else { "Settings" },
        tabs: &[],
        front_tab: 0,
        item: None,
        clock: "14:32",
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net: if st.hotspot.on {
            Net::Hotspot
        } else {
            Net::Wifi(3)
        },
        bluetooth: st.bt_on,
        notices: 0,
    };
    widgets::status_bar(&mut c, t, &status);
    model::draw(&mut c, t, &page, cur, None);
    let body = widgets::body(t);
    let none = Chip::default();
    match dialog.as_ref().map(model::sdk_dialog) {
        Some(d) => {
            d.draw(&mut c, t, body);
            if d.typing() {
                let hints: Vec<Hint> = d
                    .hints()
                    .into_iter()
                    .map(|(key, label)| Hint { key, label })
                    .collect();
                widgets::hint_bar(&mut c, t, &hints, None);
            } else {
                widgets::action_bar(&mut c, t, &[none; 5]);
            }
        }
        None => {
            let found = model::chips(&id, &page, cur, false, false);
            let mut chips = [none; 5];
            let menu = Chip {
                icon: "menu",
                word: "",
                look: Look::Normal,
            };
            chips[4] = menu;
            let owned: Vec<(u8, gh0st_app::Chip)> =
                found.into_iter().map(|(n, ch, _)| (n, ch)).collect();
            for (n, ch) in &owned {
                chips[(*n - 4) as usize] = chip_of(ch);
            }
            widgets::action_bar(&mut c, t, &chips);
        }
    }
    c
}

/// An SDK chip as the widget draws it.
fn chip_of(ch: &gh0st_app::Chip) -> Chip<'_> {
    let (icon, word, look) = ch.parts();
    Chip { icon, word, look }
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/settings");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let tag = palette.name.to_lowercase();
        for (name, c) in &scenes(palette, false) {
            write_png(c, &out.join(format!("{tag}-{name}.png")), 1).unwrap();
            write_png(c, &out.join(format!("{tag}-{name}@3x.png")), 3).unwrap();
        }
    }
    println!("wrote {}", out.display());
}

/// Every page in a palette: the full set in Mint, or in each with `all`.
fn scenes(palette: theme::Palette, all: bool) -> Vec<(&'static str, Canvas)> {
    {
        let t = Theme::new(palette);
        let st = sample_state();
        let pending = State {
            gpu_mem: GpuMem::Set(64),
            ..sample_state()
        };
        let swap_pending = State {
            swap_comp: Some("lz4".into()),
            ..sample_state()
        };
        let password = |title: &str| Dialog::Text {
            title: title.into(),
            placeholder: "Your password",
            text: "hunter22".into(),
            masked: true,
            secret: true,
        };
        let mut scenes: Vec<(&str, Canvas)> = vec![("root", scene(&t, &st, PageId::Root, 0, None))];
        if all || palette.name == "Mint" {
            let s = |id, sel| scene(&t, &st, id, sel, None);
            scenes.extend([
                ("root-scrolled", s(PageId::Root, 8)),
                ("display", s(PageId::Display, 0)),
                ("sound", s(PageId::Sound, 0)),
                ("appearance", s(PageId::Appearance, 1)),
                ("apps", s(PageId::Apps, 0)),
                (
                    "apps-move",
                    scene(
                        &t,
                        &State {
                            moving: Some("camera.desktop".into()),
                            ..sample_state()
                        },
                        PageId::Apps,
                        1,
                        None,
                    ),
                ),
                ("app", s(PageId::App("music.desktop".into()), 2)),
                ("wifi", s(PageId::Wifi, 2)),
                (
                    "wifi-password",
                    scene(
                        &t,
                        &st,
                        PageId::Wifi,
                        2,
                        Some(Dialog::Text {
                            title: "Join \u{201c}Office-5G\u{201d}".into(),
                            placeholder: "Password",
                            text: "hunter22".into(),
                            masked: true,
                            secret: true,
                        }),
                    ),
                ),
                ("network", s(PageId::Network, 0)),
                ("connection", s(PageId::Conn("HomeNet".into()), 1)),
                ("wifi-end", s(PageId::Wifi, 6)),
                ("hotspot", s(PageId::Hotspot, 0)),
                (
                    "hotspot-on",
                    scene(&t, &hotspot_on(), PageId::Hotspot, 0, None),
                ),
                (
                    "hotspot-new",
                    scene(&t, &hotspot_new(), PageId::Hotspot, 1, None),
                ),
                (
                    "hotspot-name",
                    scene(
                        &t,
                        &st,
                        PageId::Hotspot,
                        1,
                        Some(Dialog::Text {
                            title: "Hotspot name".into(),
                            placeholder: "1 to 32 characters",
                            text: "zero-kfwm".into(),
                            masked: false,
                            secret: false,
                        }),
                    ),
                ),
                (
                    "hotspot-password",
                    scene(
                        &t,
                        &st,
                        PageId::Hotspot,
                        2,
                        Some(Dialog::Text {
                            title: "Hotspot password".into(),
                            placeholder: "8 or more, no spaces",
                            text: "hunter22".into(),
                            masked: true,
                            secret: true,
                        }),
                    ),
                ),
                (
                    "hotspot-qr",
                    scene(&t, &hotspot_on(), PageId::HotspotQr, 0, None),
                ),
                ("hotspot-busy", {
                    let mut c = scene(&t, &st, PageId::Hotspot, 0, None);
                    let text = "Starting the hotspot\u{2026}";
                    widgets::busy(&mut c, &t, widgets::body(&t), text, 90);
                    c
                }),
                (
                    "hotspot-root",
                    scene(&t, &hotspot_on(), PageId::Root, 1, None),
                ),
                (
                    "hotspot-wifi",
                    scene(&t, &hotspot_on(), PageId::Wifi, 1, None),
                ),
                (
                    "hotspot-network",
                    scene(&t, &hotspot_on(), PageId::Network, 0, None),
                ),
                ("bluetooth", s(PageId::Bluetooth, 1)),
                ("system", s(PageId::System, 4)),
                ("advanced", s(PageId::Advanced, 0)),
                ("gpu-memory", s(PageId::Choose(Choice::GpuMem), 4)),
                ("cpu-mode", s(PageId::Choose(Choice::CpuMode), 0)),
                ("logs", s(PageId::Choose(Choice::Logs), 0)),
                ("country", s(PageId::Country(None), 1)),
                ("country-u", s(PageId::Country(Some('U')), 1)),
                (
                    "auth",
                    scene(&t, &st, PageId::Advanced, 0, Some(password("Password"))),
                ),
                (
                    "auth-wrong",
                    scene(
                        &t,
                        &st,
                        PageId::Advanced,
                        0,
                        Some(password("Wrong password")),
                    ),
                ),
                ("swap", s(PageId::Swap, 0)),
                ("compression", s(PageId::Choose(Choice::Compression), 0)),
                ("swappiness", s(PageId::Choose(Choice::Swappiness), 2)),
                (
                    "swap-size",
                    scene(
                        &t,
                        &st,
                        PageId::Swap,
                        0,
                        Some(Dialog::Text {
                            title: "Swap size".into(),
                            placeholder: "MB, 0 for off",
                            text: "207".into(),
                            masked: false,
                            secret: false,
                        }),
                    ),
                ),
                (
                    "device-name",
                    scene(
                        &t,
                        &st,
                        PageId::Advanced,
                        4,
                        Some(Dialog::Text {
                            title: "Device name".into(),
                            placeholder: "Letters, digits and hyphens",
                            text: "RZR-ZERO".into(),
                            masked: false,
                            secret: false,
                        }),
                    ),
                ),
                (
                    "restart-now",
                    scene(
                        &t,
                        &pending,
                        PageId::Advanced,
                        0,
                        Some(Dialog::Confirm {
                            title: "Restart to apply?".into(),
                            line: "Open apps close first.".into(),
                            yes: "restart",
                            no: "later",
                        }),
                    ),
                ),
                (
                    "advanced-restart",
                    scene(&t, &pending, PageId::Advanced, 5, None),
                ),
                (
                    "swap-restart",
                    scene(&t, &swap_pending, PageId::Swap, 3, None),
                ),
                (
                    "system-restart",
                    scene(&t, &pending, PageId::System, 4, None),
                ),
                ("timezone", s(PageId::TimeZone(Some("Europe".into())), 2)),
                ("privacy", s(PageId::Privacy, 6)),
                ("updates", s(PageId::Updates, 0)),
                (
                    "updates-edge",
                    scene(
                        &t,
                        &st,
                        PageId::Updates,
                        2,
                        Some(Dialog::Confirm {
                            title: "Switch to Edge?".into(),
                            line: "Edge gets every build of main. Things may break.".into(),
                            yes: "switch",
                            no: "cancel",
                        }),
                    ),
                ),
                ("about", s(PageId::About, 0)),
                (
                    "power-off",
                    scene(
                        &t,
                        &st,
                        PageId::System,
                        6,
                        Some(Dialog::Confirm {
                            title: "Power off?".into(),
                            line: "Open apps close first.".into(),
                            yes: "power off",
                            no: "cancel",
                        }),
                    ),
                ),
            ]);
        }
        scenes
    }
}

/// The mockups are the fixtures: every page, every palette, drawn under the
/// design lint (gh0st_ui::lint).
#[test]
fn every_page_keeps_the_design_rules() {
    gh0st_app::gfx::lint::start();
    for palette in theme::PALETTES {
        scenes(*palette, true);
    }
    let found = gh0st_app::gfx::lint::take();
    gh0st_app::gfx::lint::stop();
    assert!(found.is_empty(), "{found:#?}");
}

/// What the update engine wrote: 12 ready, checked ten minutes ago.
fn updates_state() -> gh0st_core::update::State {
    use gh0st_core::update::{Group, Outcome, Package, Restart, State};
    let pkg = |name: &str, size: u64, group: Group| Package {
        name: name.into(),
        from: "1".into(),
        to: "2".into(),
        size,
        group,
        restart: Restart::of(name),
        summary: String::new(),
    };
    let mut packages = vec![
        pkg("gh0st", 4_100_000, Group::Gh0st),
        pkg("openssl", 1_500_000, Group::Security),
        pkg("libssl3t64", 2_300_000, Group::Security),
        pkg("curl", 300_000, Group::Security),
    ];
    for i in 0..8 {
        packages.push(pkg(&format!("lib{i}"), 3_000_000, Group::System));
    }
    State {
        checked: gh0st_core::notify::now_secs() - 600,
        outcome: Outcome::Ok,
        channel: "stable".into(),
        packages,
        ..State::default()
    }
}
