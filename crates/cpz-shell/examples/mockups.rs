//! Renders the review mockups at panel size and 3x. Home comes from the real
//! `Home` view; the other screens are laid out here with the real widgets
//! until their apps exist. Data is made up. Run from the workspace root:
//!
//!     CPZ_STOCK=<launcher>/projects/APPLaunch/APPLaunch \
//!     CPZ_CJK_FONT=$CPZ_STOCK/share/font/NotoSansCJK-Regular.ttc \
//!     cargo run -p cpz-shell --example mockups
//!
//! Both env vars are optional.

use cpz_shell::home::{Entry, Home, Icon, Layout};
use cpz_ui::Canvas;
use cpz_ui::color::{Color, hex};
use cpz_ui::geom::Rect;
use cpz_ui::icon::{self, IconSize};
use cpz_ui::snapshot::write_png;
use cpz_ui::theme::{self, Theme};
use cpz_ui::widgets::{self, Battery, Hint, Net, Row, Status, Trailing};
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

fn status_at(c: &mut Canvas, t: &Theme, title: &str, percent: u8) {
    let s = Status {
        title,
        clock: "14:32",
        battery: Some(Battery {
            percent,
            charging: false,
        }),
        net: Net::Wifi(3),
    };
    widgets::status_bar(c, t, &s);
}

fn status(c: &mut Canvas, t: &Theme, title: &str) {
    status_at(c, t, title, 87);
}

fn home_status() -> Status<'static> {
    Status {
        title: "Zero",
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

fn home(t: &Theme, stock: Option<&Path>, layout: Layout, search: Option<&str>) -> Canvas {
    let mut h = Home::new(entries(t, stock), layout, t);
    match search {
        Some(q) => {
            h.start_search();
            q.chars().for_each(|ch| h.type_char(ch));
        }
        None => h.select(1),
    }
    let mut c = new_canvas(t);
    h.draw(&mut c, t, &home_status());
    c
}

fn rows(c: &mut Canvas, t: &Theme, items: &[Row], selected: usize, total: usize) {
    let (area, n) = widgets::rows_area(t, t.m.row_h);
    for (i, row) in items.iter().take(n).enumerate() {
        let r = Rect::new(area.x, area.y + i as i32 * t.m.row_h, area.w - 4, t.m.row_h);
        widgets::list_row(c, t, r, row, i == selected);
    }
    let track = Rect::new(area.x, area.y, area.w - 4, area.h);
    widgets::scrollbar(c, t, track, 0, n, total);
}

fn settings(t: &Theme) -> Canvas {
    let mut c = new_canvas(t);
    status(&mut c, t, "Settings");
    let tint = t.c.accent;
    let items = [
        Row {
            icon: Some("wifi"),
            tint,
            label: "Wi-Fi",
            trailing: Trailing::ValueChevron("HomeNet"),
        },
        Row {
            icon: Some("network"),
            tint,
            label: "Network",
            trailing: Trailing::ValueChevron("192.168.1.42"),
        },
        Row {
            icon: Some("bluetooth"),
            tint,
            label: "Bluetooth",
            trailing: Trailing::Toggle(false),
        },
        Row {
            icon: Some("sun"),
            tint,
            label: "Display",
            trailing: Trailing::ValueChevron("80%"),
        },
        Row {
            icon: Some("volume-2"),
            tint,
            label: "Sound",
            trailing: Trailing::ValueChevron("60%"),
        },
        Row {
            icon: Some("palette"),
            tint,
            label: "Appearance",
            trailing: Trailing::ValueChevron("Mint"),
        },
    ];
    rows(&mut c, t, &items, 0, 13);
    let hints = [
        Hint {
            key: "Enter",
            label: "open",
        },
        Hint {
            key: "Esc",
            label: "back",
        },
    ];
    widgets::hint_bar(&mut c, t, &hints, None);
    c
}

fn appearance(t: &Theme, layout: Layout) -> Canvas {
    let mut c = new_canvas(t);
    status(&mut c, t, "Appearance");
    let tint = t.c.accent;
    let items = [
        Row {
            icon: Some("layout-grid"),
            tint,
            label: "Home layout",
            trailing: Trailing::Choice(layout.label()),
        },
        Row {
            icon: Some("palette"),
            tint,
            label: "Palette",
            trailing: Trailing::ValueChevron(t.c.name),
        },
        Row {
            icon: Some("arrow-left-right"),
            tint,
            label: "Animations",
            trailing: Trailing::Toggle(true),
        },
        Row {
            icon: Some("clock"),
            tint,
            label: "Clock",
            trailing: Trailing::Choice("24 h"),
        },
    ];
    rows(&mut c, t, &items, 0, items.len());
    let hints = [
        Hint {
            key: "Z C",
            label: "change",
        },
        Hint {
            key: "Esc",
            label: "back",
        },
    ];
    widgets::hint_bar(&mut c, t, &hints, None);
    c
}

fn wifi_rows(t: &Theme) -> [Row<'static>; 6] {
    let tint = t.c.accent;
    [
        Row {
            icon: Some("wifi"),
            tint,
            label: "Wi-Fi",
            trailing: Trailing::Toggle(true),
        },
        Row {
            icon: Some("check"),
            tint,
            label: "HomeNet",
            trailing: Trailing::Signal {
                level: 3,
                secure: true,
            },
        },
        Row {
            icon: Some(""),
            tint,
            label: "Office-5G",
            trailing: Trailing::Signal {
                level: 4,
                secure: true,
            },
        },
        Row {
            icon: Some(""),
            tint,
            label: "Cafe Guest",
            trailing: Trailing::Signal {
                level: 2,
                secure: false,
            },
        },
        Row {
            icon: Some(""),
            tint,
            label: "Галя_WiFi",
            trailing: Trailing::Signal {
                level: 1,
                secure: true,
            },
        },
        Row {
            icon: Some("search"),
            tint: t.c.muted,
            label: "Join hidden network",
            trailing: Trailing::Chevron,
        },
    ]
}

fn wifi(t: &Theme) -> Canvas {
    let mut c = new_canvas(t);
    status(&mut c, t, "Wi-Fi");
    rows(&mut c, t, &wifi_rows(t), 2, 6);
    let hints = [
        Hint {
            key: "Enter",
            label: "join",
        },
        Hint {
            key: "R",
            label: "rescan",
        },
        Hint {
            key: "Esc",
            label: "back",
        },
    ];
    widgets::hint_bar(&mut c, t, &hints, None);
    c
}

fn wifi_password(t: &Theme) -> Canvas {
    let mut c = wifi(t);
    let r = Rect::new(28, 36, 264, 88);
    let inner = widgets::dialog(&mut c, t, r, "Join \u{201c}Office-5G\u{201d}", t.c.accent);
    let field = Rect::new(inner.x, inner.y + 2, inner.w, 24);
    widgets::text_field(&mut c, t, field, "hunter22", true, true, "Password");
    let line = Rect::new(inner.x, field.bottom() + 8, inner.w, 16);
    let mut x = line.x;
    for (key, label) in [("Tab", "show"), ("Enter", "join"), ("Esc", "cancel")] {
        x = widgets::key_pill(&mut c, t, x, line, key) + 4;
        x = widgets::text_in(
            &mut c,
            t.f.small,
            Rect::new(x, line.y + 1, 60, line.h - 1),
            label,
            t.c.muted,
        ) + 12;
    }
    c
}

fn network(t: &Theme) -> Canvas {
    let mut c = new_canvas(t);
    status(&mut c, t, "Network");
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    let w = area.w - 4;
    let mut y = area.y;
    let row = Row {
        icon: Some("ethernet-port"),
        tint: t.c.accent,
        label: "Ethernet",
        trailing: Trailing::Toggle(true),
    };
    widgets::list_row(&mut c, t, Rect::new(area.x, y, w, t.m.row_h), &row, true);
    y += t.m.row_h + 2;
    let line = 14;
    let sec = |c: &mut Canvas, y: &mut i32, s: &str| {
        widgets::section(c, t, Rect::new(area.x, *y, w, line + 2), s);
        *y += line + 2;
    };
    let ro = |c: &mut Canvas, y: &mut i32, k: &str, v: &str| {
        widgets::readout(c, t, Rect::new(area.x, *y, w, line), k, v);
        *y += line;
    };
    sec(&mut c, &mut y, "eth0 \u{00b7} connected");
    ro(&mut c, &mut y, "IPv4", "192.168.1.50/24");
    ro(&mut c, &mut y, "MAC", "2c:cf:67:0a:41:9e");
    ro(&mut c, &mut y, "Gateway", "192.168.1.1");
    sec(&mut c, &mut y, "wlan0 \u{00b7} HomeNet");
    ro(&mut c, &mut y, "IPv4", "192.168.1.42/24");
    ro(&mut c, &mut y, "DNS", "1.1.1.1, 9.9.9.9");
    let track = Rect::new(area.x, area.y, w, area.h);
    widgets::scrollbar(&mut c, t, track, 0, 8, 12);
    let hints = [
        Hint {
            key: "Enter",
            label: "toggle",
        },
        Hint {
            key: "Esc",
            label: "back",
        },
    ];
    widgets::hint_bar(&mut c, t, &hints, None);
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
    status_at(&mut c, t, "Zero", 3);
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
    let stock = std::env::var_os("CPZ_STOCK").map(PathBuf::from);
    let stock = stock.as_deref();
    let out = PathBuf::from("mockups");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let t = Theme::new(palette);
        let mut scenes: Vec<(&str, Canvas)> = vec![
            ("home-grid", home(&t, stock, Layout::Grid, None)),
            ("home-list", home(&t, stock, Layout::List, None)),
        ];
        if palette.name == "Mint" {
            scenes.extend([
                (
                    "home-grid-search",
                    home(&t, stock, Layout::Grid, Some("re")),
                ),
                (
                    "home-list-search",
                    home(&t, stock, Layout::List, Some("re")),
                ),
                ("appearance", appearance(&t, Layout::Grid)),
                ("settings", settings(&t)),
                ("wifi", wifi(&t)),
                ("wifi-password", wifi_password(&t)),
                ("network", network(&t)),
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
