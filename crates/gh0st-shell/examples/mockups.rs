//! Renders the review mockups at panel size and 3x. Home, the switcher and
//! Settings come from their real views; data is made up. Run from the
//! workspace root:
//!
//!     GH0ST_STOCK=<launcher>/projects/APPLaunch/APPLaunch \
//!     GH0ST_CJK_FONT=$GH0ST_STOCK/share/font/NotoSansCJK-Regular.ttc \
//!     cargo run -p gh0st-shell --example mockups
//!
//! Both env vars are optional.

use gh0st_shell::home::{self, Entry, Home, Icon, Layout, Size};
use gh0st_shell::settings::pages::{self, AppRow, Conn, Device, Iface, Logs, Network, State};
use gh0st_shell::settings::{self, Cursor, Dialog, PageId};
use gh0st_shell::switcher::{self, Card, Switcher};
use gh0st_ui::Canvas;
use gh0st_ui::color::{Color, hex};
use gh0st_ui::geom::Rect;
use gh0st_ui::icon::{self, IconSize};
use gh0st_ui::snapshot::write_png;
use gh0st_ui::theme::{self, Theme};
use gh0st_ui::widgets::{self, Battery, Net, Status, Visual};
use std::path::{Path, PathBuf};

fn entries(t: &Theme, stock: Option<&Path>) -> Vec<Entry> {
    let png = |file: &str, tint: usize| match stock
        .map(|s| Icon::picture(&s.join("share/images").join(file), t))
    {
        Some(Ok(icon)) => icon,
        _ => Icon::Letter { tint },
    };
    let e = |name: &str, note: &str, icon: Icon, pinned: bool| Entry {
        id: name.to_lowercase(),
        name: name.to_string(),
        note: note.to_string(),
        icon,
        pinned,
    };
    vec![
        e(
            "Terminal",
            "Shell on this device",
            Icon::Symbol {
                name: "square-terminal",
                tint: 0,
            },
            true,
        ),
        e(
            "Settings",
            "Wi-Fi, display, sound",
            Icon::Symbol {
                name: "settings",
                tint: 1,
            },
            true,
        ),
        e(
            "Store",
            "Find and install apps",
            png("store_100.png", 2),
            false,
        ),
        e(
            "Camera",
            "Photos and video",
            png("camera_100.png", 3),
            false,
        ),
        e("Files", "Browse the SD card", png("file_100.png", 2), false),
        e("Music", "Play local audio", png("music_100.png", 5), false),
        e(
            "Recorder",
            "Record from the mic",
            png("rec_100.png", 3),
            false,
        ),
        e("备忘录", "Notes", Icon::Letter { tint: 4 }, false),
        e(
            "Keyboard Guide",
            "Learn the Fn and Sym keys",
            png("Keyboard.png", 1),
            false,
        ),
    ]
}

fn status<'a>(dock: &'a [Visual<'a>], front: Option<usize>, title: &'a str) -> Status<'a> {
    Status {
        dock,
        front,
        title,
        clock: "14:32",
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
    }
}

fn new_canvas(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    c
}

/// Two apps left open: Music and the terminal.
fn open_apps(t: &Theme, stock: Option<&Path>, n: usize) -> Vec<Card> {
    let all = entries(t, stock);
    let pick = ["Music", "Terminal", "Files", "Camera", "Recorder", "Store"];
    pick.iter()
        .take(n)
        .filter_map(|name| all.iter().find(|e| e.name == *name))
        .map(|e| Card {
            id: e.id.clone(),
            name: e.name.clone(),
            icon: e.icon.clone(),
            state: switcher::State::Paused,
        })
        .collect()
}

fn dock<'a>(t: &Theme, apps: &'a [Card]) -> Vec<Visual<'a>> {
    apps.iter()
        .map(|c| home::visual(t, &c.icon, &c.name, Size::Tiny))
        .collect()
}

fn home(t: &Theme, stock: Option<&Path>, layout: Layout, search: Option<&str>) -> Canvas {
    let mut h = Home::new(entries(t, stock), layout, t);
    match search {
        Some(q) => {
            h.start_search();
            q.chars().for_each(|ch| h.type_char(ch));
        }
        None => h.select(1),
    }
    let apps = open_apps(t, stock, 2);
    let d = dock(t, &apps);
    let mut c = new_canvas(t);
    h.draw(&mut c, t, &status(&d, Some(0), ""), 0);
    c
}

fn switcher_scene(t: &Theme, stock: Option<&Path>, n: usize, at: usize) -> Canvas {
    let mut s = Switcher::new(t);
    let mut apps = open_apps(t, stock, n);
    if n > 3 {
        apps[2].state = switcher::State::Closing;
    }
    s.set_apps(apps);
    s.select(at);
    let d = s.dock(t);
    let mut c = new_canvas(t);
    s.draw(&mut c, t, &status(&d, Some(s.selected()), ""), 0);
    c
}

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

fn settings_scene(
    t: &Theme,
    stock: Option<&Path>,
    id: PageId,
    selected: usize,
    dialog: Option<Dialog>,
) -> Canvas {
    let st = sample_state();
    let page = pages::build(&id, &st);
    let apps = open_apps(t, stock, 2);
    let d = dock(t, &apps);
    let title = page.title.clone();
    let mut cur = Cursor {
        selected,
        top: 0,
    };
    let per = settings::rows(t).1;
    cur.fit(page.len(), per);
    let mut c = new_canvas(t);
    settings::draw(&mut c, t, &status(&d, Some(0), &title), &page, cur, dialog.as_ref());
    c
}

fn terminal(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    let bg = hex("#0b0f0d");
    c.fill(c.bounds(), bg);
    let green = hex("#3ddc97");
    let blue = hex("#6cb6ff");
    let fg = hex("#d7e3dc");
    let dim = hex("#7d8a84");
    let yellow = hex("#e8c468");
    type Span<'a> = (&'a str, Color, bool);
    let prompt = |cmd: &'static str| -> Vec<Span<'static>> {
        vec![
            ("pi@zero", green, true),
            (":", fg, false),
            ("~", blue, true),
            (cmd, fg, false),
        ]
    };
    let lines: Vec<Vec<Span>> = vec![
        prompt(" $ ls"),
        vec![
            ("Documents  Downloads  ", blue, true),
            ("notes.md  ", fg, false),
            ("projects", blue, true),
        ],
        prompt(" $ free -h | head -2"),
        vec![("       total   used   free  shared  buff/cache", fg, false)],
        vec![("Mem:   427Mi   58Mi  271Mi   1.2Mi       104Mi", fg, false)],
        prompt(" $ python3"),
        vec![("Python 3.13.5 on linux", fg, false)],
        vec![("Type \"help\" for more information.", dim, false)],
        vec![
            (">>> ", yellow, false),
            ("[n * n for n in range(8)]", fg, false),
        ],
        vec![("[0, 1, 4, 9, 16, 25, 36, 49]", fg, false)],
        vec![
            (">>> ", yellow, false),
            ("print(\"Привіт, 世界\")", fg, false),
        ],
        vec![("Привіт, 世界", fg, false)],
        vec![(">>> ", yellow, false)],
    ];
    let (cw, ch) = (6, 12);
    let ox = (t.m.width - 53 * cw) / 2;
    for (row, spans) in lines.iter().enumerate() {
        let y = row as i32 * ch + 10;
        let mut col = 0;
        for (s, color, bold) in spans {
            let f = if *bold { t.f.mono_bold } else { t.f.mono };
            for chr in s.chars() {
                let mut buf = [0u8; 4];
                f.draw(&mut c, ox + col * cw, y, chr.encode_utf8(&mut buf), *color);
                col += if (chr as u32) >= 0x2e80 { 2 } else { 1 };
            }
        }
        if row == lines.len() - 1 {
            c.fill(Rect::new(ox + col * cw, y - 9, cw, ch), fg);
        }
    }
    c
}

fn low_battery(t: &Theme, stock: Option<&Path>) -> Canvas {
    let mut c = home(t, stock, Layout::Grid, None);
    let r = Rect::new(40, 30, 240, 110);
    let inner = widgets::dialog(&mut c, t, r, "Battery critically low", t.c.critical);
    icon::draw(
        &mut c,
        "plug-zap",
        IconSize::Large,
        inner.x + 2,
        inner.y + 6,
        t.c.critical,
    );
    let x = inner.x + 50;
    t.f.big.draw(&mut c, x, inner.y + 24, "3%", t.c.critical);
    t.f.body
        .draw(&mut c, x, inner.y + 43, "Powering off in 12 s", t.c.fg);
    t.f.small.draw(
        &mut c,
        inner.x,
        inner.bottom() - 2,
        "Plug in the charger to cancel.",
        t.c.muted,
    );
    c
}

fn main() {
    let stock = std::env::var_os("GH0ST_STOCK").map(PathBuf::from);
    let stock = stock.as_deref();
    let out = PathBuf::from("mockups");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let t = Theme::new(palette);
        let mut scenes: Vec<(&str, Canvas)> = vec![
            ("home-grid", home(&t, stock, Layout::Grid, None)),
            ("home-list", home(&t, stock, Layout::List, None)),
            ("switcher", switcher_scene(&t, stock, 2, 1)),
            ("settings", settings_scene(&t, stock, PageId::Root, 0, None)),
        ];
        if palette.name == "Mint" {
            let s = |id, sel| settings_scene(&t, stock, id, sel, None);
            scenes.extend([
                ("home-grid-search", home(&t, stock, Layout::Grid, Some("re"))),
                ("home-list-search", home(&t, stock, Layout::List, Some("re"))),
                ("switcher-home", switcher_scene(&t, stock, 2, 0)),
                ("switcher-many", switcher_scene(&t, stock, 5, 4)),
                ("settings-scrolled", s(PageId::Root, 8)),
                ("display", s(PageId::Display, 0)),
                ("sound", s(PageId::Sound, 0)),
                ("appearance", s(PageId::Appearance, 1)),
                ("apps", s(PageId::Apps, 4)),
                ("app", s(PageId::App("music.desktop".into()), 2)),
                ("wifi", s(PageId::Wifi, 2)),
                (
                    "wifi-password",
                    settings_scene(
                        &t,
                        stock,
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
                    settings_scene(
                        &t,
                        stock,
                        PageId::System,
                        5,
                        Some(Dialog::Confirm {
                            title: "Power off?".into(),
                            line: "Open apps close first.".into(),
                            yes: "power off",
                        }),
                    ),
                ),
                ("terminal", terminal(&t)),
                ("low-battery", low_battery(&t, stock)),
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
