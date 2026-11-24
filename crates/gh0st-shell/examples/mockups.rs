//! Renders the review mockups at panel size and 3x. Home, the switcher and
//! Settings come from their real views; data is made up. Run from the
//! workspace root:
//!
//!     GH0ST_STOCK=<launcher>/projects/APPLaunch/APPLaunch \
//!     GH0ST_CJK_FONT=$GH0ST_STOCK/share/font/NotoSansCJK-Regular.ttc \
//!     cargo run -p gh0st-shell --example mockups
//!
//! Both env vars are optional.

use gh0st_core::usage::{AppUsage, SystemUsage};
use gh0st_shell::home::{self, Entry, Home, Icon, Layout, Size};
use gh0st_shell::prompt::{self, Ask, Kind, Origin};
use gh0st_shell::switcher::{self, Card, Switcher};
use gh0st_ui::Canvas;
use gh0st_ui::color::{Color, hex};
use gh0st_ui::geom::Rect;
use gh0st_ui::icon::{self, IconSize};
use gh0st_ui::snapshot::write_png;
use gh0st_ui::theme::{self, Theme, Tone};
use gh0st_ui::wallpaper;
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
        count: 0,
    };
    vec![
        e(
            "Terminal",
            "Shell on this device",
            Icon::System {
                name: "square-terminal",
            },
            true,
        ),
        e(
            "Settings",
            "Wi-Fi, display, sound",
            Icon::System { name: "settings" },
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
        icon: "",
        title,
        context: "",
        tabs: &[],
        front_tab: 0,
        item: None,
        clock: "14:32",
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
        bluetooth: true,
        notices: 0,
    }
}

/// Which picture the scenes stand on: `GH0ST_WALLPAPER` as the shell
/// reads it, aurora when unset.
fn picture() -> String {
    std::env::var("GH0ST_WALLPAPER").unwrap_or_else(|_| "aurora".into())
}

/// A canvas on home's picture, or the lock's.
fn grounded(t: &Theme, lock: bool) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    let (w, h) = (t.m.width as usize, t.m.height as usize);
    if let Some(p) = wallpaper::builtin(&picture(), &t.c, w, h) {
        let b = wallpaper::bake(&p, &t.c, t.m.bar_h as usize, t.m.hint_h as usize);
        c.set_backdrop(Some(if lock { b.lock } else { b.home }));
    }
    c
}

fn new_canvas(t: &Theme) -> Canvas {
    grounded(t, false)
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

fn charging(t: &Theme, stock: Option<&Path>, percent: u8) -> Canvas {
    let mut h = Home::new(entries(t, stock), Layout::Grid, t);
    h.select(1);
    let mut s = status(&[], Some(0), "");
    s.battery = Some(Battery {
        percent,
        charging: true,
    });
    let mut c = new_canvas(t);
    h.draw(&mut c, t, &s, 0);
    c
}

fn switcher_scene(t: &Theme, stock: Option<&Path>, n: usize, at: usize) -> Canvas {
    let mut s = Switcher::new(t);
    let mut apps = open_apps(t, stock, n);
    if n > 3 {
        apps[2].state = switcher::State::Closing;
    }
    // The terminal runs behind, so it has a CPU figure.
    apps[1].state = switcher::State::Running;
    let figures = [
        (38_400, None),
        (11_900, Some(2.4)),
        (0, None),
        (61_700, None),
        (6_100, None),
    ];
    let usage: Vec<(&str, AppUsage)> = apps
        .iter()
        .zip(figures)
        .map(|(a, (mem_kb, cpu))| (a.id.as_str(), AppUsage { mem_kb, cpu }))
        .collect();
    let system = SystemUsage {
        used_kb: 221_000,
        total_kb: 360_448,
        cpu: Some(if at == 0 { 100.0 } else { 6.8 }),
    };
    s.set_apps(apps.clone());
    s.set_usage(&usage, system);
    s.select(at);
    let d = s.dock(t);
    let mut c = new_canvas(t);
    s.draw(&mut c, t, &status(&d, Some(s.selected()), ""), 0);
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

/// The lock screen: the field with that many dots, or None at rest.
fn lock(t: &Theme, field: Option<usize>, ready: bool, note: Option<(Tone, &str)>) -> Canvas {
    let mut s = status(&[], None, "");
    s.clock = "";
    let l = widgets::Lock {
        clock: "14:32",
        date: "Sunday 22 November",
        field,
        ready,
        note,
        bell: note.is_some_and(|(_, n)| n.ends_with("notifications")),
    };
    let mut c = grounded(t, true);
    widgets::lock_screen(&mut c, t, &s, &l);
    c
}

/// A keyring question, as the shell asks it.
fn ask(t: &Theme, origin: Origin, kind: Kind, typed: usize, checking: bool, wrong: bool) -> Canvas {
    let (icon, title) = prompt::bar(&origin);
    let mut s = status(&[], None, title);
    s.icon = icon;
    let a = Ask { origin, kind };
    let v = prompt::View {
        ask: &a,
        typed,
        checking,
        wrong,
    };
    let mut c = grounded(t, true);
    // The wallpaper an app asks for shows behind its question, unfrosted.
    if let Origin::Portal(_, "wallpaper") = a.origin {
        let (w, h) = (t.m.width as usize, t.m.height as usize);
        if let Some(p) = wallpaper::builtin("contour", &t.c, w, h) {
            let b = wallpaper::bake(&p, &t.c, t.m.bar_h as usize, t.m.hint_h as usize);
            c.set_backdrop(Some(b.home));
        }
    }
    prompt::draw(&mut c, t, &s, &v);
    c
}

fn approve() -> Kind {
    Kind::Approve {
        app: "Chromium".into(),
        what: "read \u{201c}GitHub token\u{201d}".into(),
    }
}

fn password(reason: &str) -> Kind {
    Kind::Password {
        reason: reason.into(),
        retry: false,
    }
}

/// Home's menu with Lock selected.
fn home_menu(t: &Theme, stock: Option<&Path>) -> Canvas {
    let mut c = home(t, stock, Layout::Grid, None);
    let row = |label, choice, count, keys| home::MenuRow {
        label,
        choice,
        count,
        keys,
        dim: false,
    };
    let rows = [
        row("Pin Store", None, None, "p"),
        row("Layout", Some("Grid"), None, "l"),
        row("Find", None, None, "s"),
        row("Notifications", None, Some("2"), ""),
        row("Lock", None, None, "Tab"),
    ];
    home::draw_menu(&mut c, t, &rows, 4);
    c
}

fn low_battery(t: &Theme, stock: Option<&Path>) -> Canvas {
    let mut c = home(t, stock, Layout::Grid, None);
    let r = Rect::new(40, 30, 240, 110);
    let all = c.bounds();
    widgets::scrim(&mut c, t, all);
    let inner = widgets::card(&mut c, t, r, "Battery critically low", t.c.critical);
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

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let stock = std::env::var_os("GH0ST_STOCK").map(PathBuf::from);
    let stock = stock.as_deref();
    let out = PathBuf::from("mockups");
    // Only our own PNGs go: Settings renders its pages into mockups/settings.
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    for palette in [theme::MINT, theme::NORD, theme::PAPER] {
        let tag = palette.name.to_lowercase();
        for (name, c) in &scenes(palette, stock, false) {
            write_png(c, &out.join(format!("{tag}-{name}.png")), 1).unwrap();
            write_png(c, &out.join(format!("{tag}-{name}@3x.png")), 3).unwrap();
        }
    }
    println!("wrote {}", out.display());
}

/// The mockups are the fixtures: every screen, every palette, under the
/// design lint.
#[test]
fn every_screen_keeps_the_design_rules() {
    gh0st_ui::lint::start();
    for palette in theme::PALETTES {
        scenes(*palette, None, true);
    }
    let found = gh0st_ui::lint::take();
    gh0st_ui::lint::stop();
    assert!(found.is_empty(), "{found:#?}");
}

/// The shell's screens in a palette: the full set in Mint, or in each with
/// `all`.
fn scenes(palette: theme::Palette, stock: Option<&Path>, all: bool) -> Vec<(&'static str, Canvas)> {
    {
        let t = Theme::new(palette);
        let mut scenes: Vec<(&str, Canvas)> = vec![
            ("home-grid", home(&t, stock, Layout::Grid, None)),
            ("home-list", home(&t, stock, Layout::List, None)),
            ("switcher", switcher_scene(&t, stock, 2, 1)),
            ("lock", lock(&t, None, true, None)),
            (
                "prompt-approve",
                ask(&t, Origin::Daemon(1), approve(), 0, false, false),
            ),
            (
                "prompt-account",
                ask(
                    &t,
                    Origin::Portal(1, "account"),
                    Kind::Approve {
                        app: "Chat".into(),
                        what: "see your name".into(),
                    },
                    0,
                    false,
                    false,
                ),
            ),
            (
                "prompt-wallpaper",
                ask(
                    &t,
                    Origin::Portal(2, "wallpaper"),
                    Kind::Approve {
                        app: "Media".into(),
                        what: "make this your wallpaper".into(),
                    },
                    0,
                    false,
                    false,
                ),
            ),
            (
                "prompt-password",
                ask(
                    &t,
                    Origin::Daemon(2),
                    password("git-credential-libsecret wants the keyring"),
                    6,
                    false,
                    false,
                ),
            ),
        ];
        if all || palette.name == "Mint" {
            scenes.extend([
                (
                    "home-grid-search",
                    home(&t, stock, Layout::Grid, Some("re")),
                ),
                (
                    "home-list-search",
                    home(&t, stock, Layout::List, Some("re")),
                ),
                ("switcher-home", switcher_scene(&t, stock, 2, 0)),
                ("switcher-many", switcher_scene(&t, stock, 5, 4)),
                ("terminal", terminal(&t)),
                ("low-battery", low_battery(&t, stock)),
                ("charging-low", charging(&t, stock, 5)),
                ("charging-high", charging(&t, stock, 87)),
                ("lock-typing", lock(&t, Some(7), true, None)),
                (
                    "lock-checking",
                    lock(&t, Some(7), false, Some((Tone::Info, "Checking\u{2026}"))),
                ),
                (
                    "lock-wrong",
                    lock(&t, Some(0), true, Some((Tone::Critical, "Wrong password"))),
                ),
                (
                    "lock-wait",
                    lock(
                        &t,
                        Some(0),
                        false,
                        Some((Tone::Warning, "Try again in 12 s")),
                    ),
                ),
                (
                    "lock-no-password",
                    lock(&t, None, true, Some((Tone::Info, "No password set"))),
                ),
                ("lock-field", lock(&t, Some(0), true, None)),
                (
                    "lock-notices",
                    lock(&t, None, true, Some((Tone::Info, "2 notifications"))),
                ),
                ("home-menu-lock", home_menu(&t, stock)),
                (
                    "prompt-wrong",
                    ask(
                        &t,
                        Origin::Daemon(2),
                        password("Chromium wants the keyring"),
                        0,
                        false,
                        true,
                    ),
                ),
                (
                    "prompt-checking",
                    ask(
                        &t,
                        Origin::Daemon(2),
                        password("Chromium wants the keyring"),
                        9,
                        true,
                        false,
                    ),
                ),
                (
                    "prompt-open",
                    ask(
                        &t,
                        Origin::Open,
                        password("Your password opens it for your apps."),
                        0,
                        false,
                        false,
                    ),
                ),
                (
                    "prompt-stale",
                    ask(&t, Origin::Stale, password(""), 3, false, false),
                ),
            ]);
        }
        scenes
    }
}
