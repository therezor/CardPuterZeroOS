//! Renders every Settings page at panel size and 3x, from made-up state, into
//! mockups/settings. Run from the workspace root:
//!
//!     cargo run -p gh0st-settings --example mockups

use gh0st_app::gfx::Canvas;
use gh0st_app::gfx::snapshot::write_png;
use gh0st_app::gfx::theme::{self, Theme};
use gh0st_app::gfx::widgets::{self, Battery, Hint, Net, Status};
use gh0st_core::apps::Layout;
use gh0st_settings::model::pages::{self, AppRow, Conn, Device, Iface, Logs, Network, State};
use gh0st_settings::model::{self, Cursor, Dialog, PageId};
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
        pinned: false,
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
            app("store.desktop", "Store", true),
            app("files.desktop", "Files", true),
            app("camera.desktop", "Camera", true),
            app("music.desktop", "Music", true),
            app("zclaw.desktop", "ZClaw", false),
            app("piano.desktop", "Piano", true),
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
        about: [
            ("Version", "Gh0stdeck 0.1.0"),
            ("OS", "Debian 13 (trixie)"),
            ("Kernel", "6.18.39+rpt-rpi-v8"),
            ("Memory", "61 MB of 352 MB used"),
            ("Storage", "3.1 GB of 29 GB used"),
            ("Battery", "87%, 3.98 V"),
            ("Uptime", "2 h 14 min"),
            ("Host", "cardputer"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec(),
    }
}

/// One page as the device shows it: the OS top bar and hints the SDK draws,
/// the page body and any dialog.
fn scene(t: &Theme, id: PageId, selected: usize, dialog: Option<Dialog>) -> Canvas {
    let st = sample_state();
    let page = pages::build(&id, &st);
    let mut cur = Cursor { selected, top: 0 };
    cur.fit(page.len(), model::rows(t).1);
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    let status = Status {
        dock: &[],
        front: None,
        title: &page.title,
        clock: "14:32",
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
    };
    widgets::status_bar(&mut c, t, &status);
    let hints: Vec<Hint> = model::hints(&page, page.item(cur.selected))
        .into_iter()
        .map(|(key, label)| Hint { key, label })
        .collect();
    widgets::hint_bar(&mut c, t, &hints, None);
    model::draw(&mut c, t, &page, cur, dialog.as_ref());
    c
}

fn main() {
    let out = PathBuf::from("mockups/settings");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let t = Theme::new(palette);
        let mut scenes: Vec<(&str, Canvas)> = vec![("root", scene(&t, PageId::Root, 0, None))];
        if palette.name == "Mint" {
            let s = |id, sel| scene(&t, id, sel, None);
            scenes.extend([
                ("root-scrolled", s(PageId::Root, 8)),
                ("display", s(PageId::Display, 0)),
                ("sound", s(PageId::Sound, 0)),
                ("appearance", s(PageId::Appearance, 1)),
                ("apps", s(PageId::Apps, 4)),
                ("app", s(PageId::App("music.desktop".into()), 2)),
                ("wifi", s(PageId::Wifi, 2)),
                (
                    "wifi-password",
                    scene(
                        &t,
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
                ("bluetooth", s(PageId::Bluetooth, 1)),
                ("system", s(PageId::System, 2)),
                ("timezone", s(PageId::TimeZone(Some("Europe".into())), 2)),
                ("privacy", s(PageId::Privacy, 0)),
                ("about", s(PageId::About, 0)),
                (
                    "power-off",
                    scene(
                        &t,
                        PageId::System,
                        5,
                        Some(Dialog::Confirm {
                            title: "Power off?".into(),
                            line: "Open apps close first.".into(),
                            yes: "power off",
                        }),
                    ),
                ),
            ]);
        }
        let tag = palette.name.to_lowercase();
        for (name, c) in &scenes {
            write_png(c, &out.join(format!("{tag}-{name}.png")), 1).unwrap();
            write_png(c, &out.join(format!("{tag}-{name}@3x.png")), 3).unwrap();
        }
    }
    println!("wrote {}", out.display());
}
