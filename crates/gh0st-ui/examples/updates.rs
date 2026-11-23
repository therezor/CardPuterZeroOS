//! Mockups for updates and notifications, drawn with the real gh0st-ui
//! widgets: the Updates app, Settings > Updates, and the shell's
//! notifications. Writes mockups/updates. Run from the workspace root:
//!
//!     cargo run -p gh0st-ui --example updates

use gh0st_ui::font::Font;
use gh0st_ui::snapshot::write_png;
use gh0st_ui::theme::{self, Theme, Tone};
use gh0st_ui::widgets::{
    self, AppItem, Battery, Card, Chip, Hint, Look, Message, Net, Row, Status, Trailing, Visual,
    soft, text_in,
};
use gh0st_ui::{Canvas, Color, Rect};
use std::path::PathBuf;

const CLOCK: &str = "14:32";

// ---------------------------------------------------------------- helpers

fn top<'a>(icon: &'a str, title: &'a str) -> Status<'a> {
    Status {
        dock: &[],
        front: None,
        icon,
        title,
        context: "",
        tabs: &[],
        front_tab: 0,
        item: None,
        clock: CLOCK,
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net: Net::Wifi(3),
        bluetooth: false,
        notices: 0,
    }
}

fn sub<'a>(icon: &'a str, title: &'a str, parent: &'a str) -> Status<'a> {
    Status {
        context: parent,
        ..top(icon, title)
    }
}

type Slots<'a> = [Chip<'a>; 5];

const NONE: Chip<'static> = Chip {
    icon: "",
    word: "",
    look: Look::Normal,
};

fn l<'a>(icon: &'a str, word: &'a str, look: Look) -> Chip<'a> {
    Chip { icon, word, look }
}

fn n<'a>(icon: &'a str, word: &'a str) -> Chip<'a> {
    l(icon, word, Look::Normal)
}

fn menu() -> Chip<'static> {
    n("menu", "")
}

fn hints<'a>(h: &[(&'a str, &'a str)]) -> Vec<Hint<'a>> {
    h.iter().map(|&(key, label)| Hint { key, label }).collect()
}

fn frame(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    c
}

fn rows_from(t: &Theme, y: i32, row_h: i32, i: usize) -> Rect {
    Rect::new(
        t.m.margin,
        y + i as i32 * row_h,
        t.m.width - 2 * t.m.margin,
        row_h,
    )
}

fn row<'a>(icon: &'a str, tint: Color, label: &'a str, trailing: Trailing<'a>) -> Row<'a> {
    Row {
        icon: Some(icon),
        tint,
        label,
        trailing,
    }
}

fn wrap(f: Font, s: &str, w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if f.measure(&next) > w && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn para(c: &mut Canvas, t: &Theme, r: Rect, text: &str, col: Color) {
    for (i, line) in wrap(t.f.body, text, r.w).iter().enumerate() {
        let lr = Rect::new(r.x, r.y + i as i32 * 18, r.w, 18);
        text_in(c, t.f.body, lr, line, col);
    }
}

/// A card with a question, centred over the content, keys at its foot.
fn ask_card(
    c: &mut Canvas,
    t: &Theme,
    title: &str,
    edge: Color,
    text: &str,
    keys: &[(&str, &str)],
) -> Rect {
    let w = t.m.width - 44;
    let lines = wrap(t.f.body, text, w - 24).len() as i32;
    let h = widgets::card_h(t, lines * 18 + t.m.card_gap + t.m.pill_h);
    let b = widgets::body(t);
    let body = widgets::card(c, t, Rect::new(22, b.y + (b.h - h) / 2, w, h), title, edge);
    para(c, t, body, text, soft(t));
    widgets::key_row(c, t, body.x, body.bottom() - t.m.pill_h, &hints(keys));
    body
}

fn scrim(c: &mut Canvas, t: &Theme) {
    widgets::scrim(c, t, widgets::body(t));
}

fn blit(dst: &mut Canvas, src: &Canvas, ox: i32, oy: i32) {
    for y in 0..src.height() {
        for x in 0..src.width() {
            dst.set(ox + x, oy + y, src.get(x, y));
        }
    }
}

// ---------------------------------------------------------------- Updates app

const APP_ICON: &str = "refresh-cw";

fn app_bar(c: &mut Canvas, t: &Theme) {
    widgets::status_bar(c, t, &top(APP_ICON, "Updates"));
}

fn app_message(t: &Theme, m: &Message, slots: Slots) -> Canvas {
    let mut c = frame(t);
    app_bar(&mut c, t);
    widgets::message(&mut c, t, widgets::body(t), m);
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn u_never(t: &Theme) -> Canvas {
    let keys = hints(&[("Enter", "check now")]);
    let m = Message {
        icon: APP_ICON,
        tone: None,
        text: "Not checked yet",
        note: "Checks daily on Wi-Fi. Stable channel.",
        keys: &keys,
    };
    app_message(t, &m, [NONE, NONE, NONE, NONE, menu()])
}

fn u_current(t: &Theme) -> Canvas {
    let keys = hints(&[("Enter", "check now")]);
    let m = Message {
        icon: "circle-check",
        tone: Some(Tone::Ok),
        text: "Up to date",
        note: "Gh0st 0.5 · Stable\nChecked today at 09:12",
        keys: &keys,
    };
    app_message(t, &m, [NONE, NONE, NONE, NONE, menu()])
}

fn u_available(t: &Theme) -> Canvas {
    let keys = hints(&[("Enter", "install")]);
    let m = Message {
        icon: "download",
        tone: Some(Tone::Info),
        text: "12 updates · 34 MB",
        note: "Gh0st 0.5, 3 security fixes, new kernel\nRestarts when done",
        keys: &keys,
    };
    app_message(
        t,
        &m,
        [
            NONE,
            n("refresh-cw", "Check"),
            n("list", "List"),
            NONE,
            menu(),
        ],
    )
}

/// The same state as a list: one row per group, the summary above.
fn u_available_list(t: &Theme) -> Canvas {
    let mut c = frame(t);
    app_bar(&mut c, t);
    let b = widgets::body(t);
    let head = Rect::new(15, b.y + 4, b.w - 30, 18);
    text_in(&mut c, t.f.body_bold, head, "12 updates · 34 MB", t.c.fg);
    let keys = hints(&[("Enter", "install")]);
    let kw = widgets::key_row_w(t, &keys);
    widgets::key_row(&mut c, t, head.right() - kw, head.y + 1, &keys);
    let a = t.c.accent;
    let y = head.bottom() + 3;
    let rows = [
        (
            row(
                "gh0st-components",
                a,
                "Gh0st OS",
                Trailing::Tag("2", Tone::Info),
            ),
            "Gh0st 0.4.1 → 0.5.0, kernel 6.12.50",
        ),
        (
            row("shield", t.c.ok, "Security", Trailing::Tag("3", Tone::Ok)),
            "openssl, libc6, curl",
        ),
        (
            row(
                "package",
                t.c.icon_tint("package"),
                "System",
                Trailing::Tag("6", Tone::Info),
            ),
            "Debian 13.3 fixes",
        ),
    ];
    for (i, (rw, note)) in rows.iter().enumerate() {
        let r = rows_from(t, y, t.m.app_row_h, i);
        widgets::note_row(&mut c, t, r, rw, note, false);
    }
    widgets::action_bar(
        &mut c,
        t,
        &[
            NONE,
            n("refresh-cw", "Check"),
            n("list", "List"),
            NONE,
            menu(),
        ],
    );
    c
}

/// Check, Download, Install, Restart, with the current one lit.
fn stepper(c: &mut Canvas, t: &Theme, y: i32, current: usize) {
    let r = Rect::new(15, y, t.m.width - 30, 26);
    widgets::stepper(
        c,
        t,
        r,
        &["Check", "Download", "Install", "Restart"],
        current,
    );
}

fn busy(
    t: &Theme,
    title: &str,
    label: &str,
    value: Option<u8>,
    note: &str,
    step: Option<usize>,
    slots: Slots,
) -> Canvas {
    let mut c = frame(t);
    app_bar(&mut c, t);
    let b = widgets::body(t);
    let w = b.w - 30;
    text_in(
        &mut c,
        t.f.body_bold,
        Rect::new(15, b.y + 8, w, 18),
        title,
        t.c.fg,
    );
    widgets::progress(&mut c, t, Rect::new(15, b.y + 30, w, 24), label, value, 90);
    text_in(
        &mut c,
        t.f.small,
        Rect::new(15, b.y + 58, w, 14),
        note,
        t.c.muted,
    );
    if let Some(s) = step {
        stepper(&mut c, t, b.bottom() - 34, s);
    }
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn u_checking(t: &Theme) -> Canvas {
    busy(
        t,
        "Checking for updates",
        "Debian security · 3 of 5",
        None,
        "You can leave. It carries on.",
        Some(0),
        [NONE, l("square", "Stop", Look::Danger), NONE, NONE, menu()],
    )
}

fn u_download(t: &Theme) -> Canvas {
    busy(
        t,
        "Downloading 12 updates",
        "12.1 of 34 MB",
        Some(35),
        "You can leave. It carries on.",
        Some(1),
        [NONE, l("square", "Stop", Look::Danger), NONE, NONE, menu()],
    )
}

fn u_install(t: &Theme) -> Canvas {
    busy(
        t,
        "Installing 12 updates",
        "libc6 · 5 of 12",
        Some(48),
        "Keep it on until it is done.",
        Some(2),
        [NONE, NONE, NONE, NONE, menu()],
    )
}

fn u_done(t: &Theme) -> Canvas {
    let keys = hints(&[("Enter", "restart"), ("Esc", "later")]);
    let m = Message {
        icon: "circle-check",
        tone: Some(Tone::Ok),
        text: "12 updates installed",
        note: "Restart to start the new kernel",
        keys: &keys,
    };
    app_message(t, &m, [NONE, NONE, NONE, NONE, menu()])
}

fn u_done_shell(t: &Theme) -> Canvas {
    let keys = hints(&[("Enter", "restart Gh0st"), ("Esc", "later")]);
    let m = Message {
        icon: "circle-check",
        tone: Some(Tone::Ok),
        text: "Gh0st 0.5 installed",
        note: "Restarting Gh0st closes open apps\nand locks the screen",
        keys: &keys,
    };
    app_message(t, &m, [NONE, NONE, NONE, NONE, menu()])
}

fn u_confirm(t: &Theme) -> Canvas {
    let mut c = u_available(t);
    scrim(&mut c, t);
    ask_card(
        &mut c,
        t,
        "Install 12 updates?",
        t.c.accent,
        "34 MB to download. The device restarts at the end.",
        &[("Enter", "install"), ("Esc", "cancel")],
    );
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

fn u_kernel_battery(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        battery: Some(Battery {
            percent: 42,
            charging: false,
        }),
        ..top(APP_ICON, "Updates")
    };
    widgets::status_bar(&mut c, t, &s);
    let m = Message {
        icon: "triangle-alert",
        tone: Some(Tone::Warning),
        text: "Kernel update needs over 50%",
        note: "The battery is at 42%. Charge it, then install.\nA power cut here can stop the device booting.",
        keys: &[],
    };
    widgets::message(&mut c, t, widgets::body(t), &m);
    widgets::action_bar(
        &mut c,
        t,
        &[
            NONE,
            n("refresh-cw", "Check"),
            n("list", "List"),
            NONE,
            menu(),
        ],
    );
    c
}

fn u_offline(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        net: Net::None,
        ..top(APP_ICON, "Updates")
    };
    widgets::status_bar(&mut c, t, &s);
    let keys = hints(&[("Enter", "try again")]);
    let m = Message {
        icon: "wifi-off",
        tone: Some(Tone::Critical),
        text: "No network",
        note: "Join Wi-Fi in Settings, then try again.\nLast checked yesterday at 09:12",
        keys: &keys,
    };
    widgets::message(&mut c, t, widgets::body(t), &m);
    widgets::action_bar(&mut c, t, &[NONE, NONE, NONE, NONE, menu()]);
    c
}

fn u_list(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("list", "Packages", "Updates"));
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Gh0st OS");
    y += 15;
    let g = [
        (
            row(
                "gh0st-components",
                t.c.icon_tint("gh0st-components"),
                "gh0st",
                Trailing::Value("4.1 MB"),
            ),
            "0.4.1 → 0.5.0 · restarts Gh0st",
        ),
        (
            row(
                "cpu",
                t.c.icon_tint("cpu"),
                "gh0st-kernel",
                Trailing::Value("21 MB"),
            ),
            "6.12.47 → 6.12.50 · restarts the device",
        ),
    ];
    for (i, (rw, note)) in g.iter().enumerate() {
        widgets::note_row(
            &mut c,
            t,
            rows_from(t, y, t.m.app_row_h, i),
            rw,
            note,
            i == 0,
        );
    }
    y += 2 * t.m.app_row_h + 2;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Security");
    y += 15;
    let s = row("shield", t.c.ok, "openssl", Trailing::Value("2.3 MB"));
    widgets::note_row(
        &mut c,
        t,
        rows_from(t, y, t.m.app_row_h, 0),
        &s,
        "3.5.1-1 → 3.5.1-1+deb13u1",
        false,
    );
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    widgets::scrollbar(&mut c, t, area, 0, 4, 12);
    widgets::action_bar(&mut c, t, &[NONE, NONE, NONE, NONE, menu()]);
    c
}

fn u_detail(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("gh0st-components", "gh0st", "Packages"));
    let b = widgets::body(t);
    para(
        &mut c,
        t,
        Rect::new(15, b.y + 4, b.w - 30, 36),
        "The Gh0st OS shell and Settings.",
        t.c.fg,
    );
    let rows = [
        ("Version", "0.4.1 → 0.5.0"),
        ("Download", "4.1 MB"),
        ("From", "Gh0st OS stable"),
        ("After", "Gh0st restarts"),
    ];
    for (i, (k, v)) in rows.iter().enumerate() {
        widgets::readout(
            &mut c,
            t,
            Rect::new(6, b.y + 44 + i as i32 * 20, b.w - 12, 20),
            k,
            v,
        );
    }
    widgets::action_bar(&mut c, t, &[NONE, NONE, NONE, NONE, menu()]);
    c
}

fn u_menu(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("menu", "Menu", "Updates"));
    let (x, w, kw) = (6, t.m.width - 12, 40);
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(x, y, w, 15), "Updates");
    y += 15;
    let rows: [(Row, &str); 4] = [
        (
            row(
                "download",
                t.c.icon_tint("download"),
                "Install all",
                Trailing::None,
            ),
            "Enter",
        ),
        (
            row(
                "refresh-cw",
                t.c.icon_tint("refresh-cw"),
                "Check now",
                Trailing::None,
            ),
            "5",
        ),
        (
            row("list", t.c.icon_tint("list"), "Packages", Trailing::None),
            "6",
        ),
        (
            row(
                "trash-2",
                t.c.icon_tint("trash-2"),
                "Clear downloads",
                Trailing::Value("120 MB"),
            ),
            "",
        ),
    ];
    for (i, (rw, k)) in rows.iter().enumerate() {
        widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), rw, k, kw, i == 0, false);
        y += 22;
    }
    let _ = y;
    widgets::scrollbar(&mut c, t, Rect::new(x, 20, w, 128), 0, 4, 7);
    let slots = [NONE, NONE, NONE, NONE, l("menu", "", Look::On)];
    widgets::action_bar(&mut c, t, &slots);
    c
}

// ---------------------------------------------------------------- Settings

fn settings_list(t: &Theme, status: &Status, rows: &[Row], sel: usize, slots: Slots) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, status);
    let (area, fit) = widgets::rows_area(t, t.m.row_h);
    for (i, rw) in rows.iter().take(fit).enumerate() {
        widgets::list_row(&mut c, t, rows_from(t, area.y, t.m.row_h, i), rw, i == sel);
    }
    widgets::scrollbar(&mut c, t, area, 0, fit.min(rows.len()), rows.len());
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn s_system(t: &Theme) -> Canvas {
    let rows = [
        row(
            "lock",
            t.c.icon_tint("lock"),
            "Lock after",
            Trailing::Choice("5 min"),
        ),
        row(
            "sliders-horizontal",
            t.c.icon_tint("sliders-horizontal"),
            "Advanced",
            Trailing::Chevron,
        ),
        row(
            "download",
            t.c.icon_tint("download"),
            "Updates",
            Trailing::ValueChevron("3 ready"),
        ),
        row(
            "rotate-cw",
            t.c.icon_tint("rotate-cw"),
            "Restart",
            Trailing::None,
        ),
        row("power", t.c.critical, "Power off", Trailing::None),
    ];
    settings_list(
        t,
        &sub("cpu", "System", "Settings"),
        &rows,
        2,
        [NONE, NONE, NONE, NONE, menu()],
    )
}

fn s_updates(t: &Theme, sel: usize) -> Canvas {
    let a = t.c.accent;
    let rows = [
        row(
            "download",
            a,
            "12 updates ready",
            Trailing::ValueChevron("34 MB"),
        ),
        row(
            "refresh-cw",
            t.c.icon_tint("refresh-cw"),
            "Check daily",
            Trailing::Toggle(true),
        ),
        row(
            "flag",
            t.c.icon_tint("flag"),
            "Channel",
            Trailing::Choice("Stable"),
        ),
        row(
            "clock",
            t.c.icon_tint("clock"),
            "Last check",
            Trailing::Value("Today 09:12"),
        ),
        row(
            "trash-2",
            t.c.icon_tint("trash-2"),
            "Clear downloads",
            Trailing::Value("120 MB"),
        ),
    ];
    let slots = if sel == 2 {
        [
            NONE,
            n("chevron-left", "Prev"),
            NONE,
            n("chevron-right", "Next"),
            menu(),
        ]
    } else {
        [NONE, n("refresh-cw", "Check"), NONE, NONE, menu()]
    };
    settings_list(t, &sub("download", "Updates", "System"), &rows, sel, slots)
}

fn s_edge(t: &Theme) -> Canvas {
    let mut c = s_updates(t, 2);
    scrim(&mut c, t);
    ask_card(
        &mut c,
        t,
        "Switch to Edge?",
        t.c.warning,
        "Edge gets every build of main. Things may break.",
        &[("Enter", "switch"), ("Esc", "cancel")],
    );
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

fn s_privacy(t: &Theme) -> Canvas {
    let rows = [
        row(
            "eye",
            t.c.icon_tint("eye"),
            "Send device name",
            Trailing::Toggle(false),
        ),
        row(
            "globe",
            t.c.icon_tint("globe"),
            "IPv6 privacy address",
            Trailing::Toggle(true),
        ),
        row(
            "hard-drive",
            t.c.icon_tint("hard-drive"),
            "Keep logs",
            Trailing::Choice("Off"),
        ),
    ];
    settings_list(
        t,
        &sub("shield", "Privacy", "Settings"),
        &rows,
        3,
        [NONE, NONE, NONE, NONE, menu()],
    )
}

// ---------------------------------------------------------------- notifications

fn home_items(t: &Theme) -> Vec<(&'static str, Visual<'static>)> {
    let tints = t.c.tints;
    vec![
        (
            "Terminal",
            Visual::System {
                icon: "square-terminal",
            },
        ),
        ("Settings", Visual::System { icon: "settings" }),
        ("Updates", Visual::System { icon: APP_ICON }),
        (
            "Store",
            Visual::Badge {
                icon: "shopping-bag",
                tint: tints[2],
            },
        ),
        (
            "Camera",
            Visual::Badge {
                icon: "camera",
                tint: tints[3],
            },
        ),
        (
            "Files",
            Visual::Badge {
                icon: "folder",
                tint: tints[2],
            },
        ),
        (
            "Music",
            Visual::Letter {
                ch: 'm',
                tint: tints[5],
            },
        ),
        (
            "Recorder",
            Visual::Badge {
                icon: "mic",
                tint: tints[3],
            },
        ),
    ]
}

fn home_slots() -> Slots<'static> {
    [
        n("arrow-left-right", "Apps"),
        NONE,
        n("search", "Find"),
        NONE,
        menu(),
    ]
}

/// Home with a bell in the top bar and a count on the Updates tile.
fn n_home(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let status = Status {
        front: Some(0),
        notices: 2,
        ..top("", "")
    };
    widgets::status_bar(&mut c, t, &status);
    let area = widgets::grid(t);
    let tw = (area.w - 3 * t.m.gap) / 4;
    let th = (area.h - t.m.gap) / 2;
    for (i, (label, v)) in home_items(t).into_iter().enumerate() {
        let r = Rect::new(
            area.x + (i as i32 % 4) * (tw + t.m.gap),
            area.y + (i as i32 / 4) * (th + t.m.gap),
            tw,
            th,
        );
        let item = AppItem {
            label,
            note: "",
            visual: v,
            pinned: i < 2,
            count: if label == "Updates" { 12 } else { 0 },
            tag: None,
        };
        widgets::tile(&mut c, t, r, &item, i == 2);
    }
    widgets::scrollbar(&mut c, t, area, 0, 2, 3);
    widgets::action_bar(&mut c, t, &home_slots());
    c
}

/// A notice arriving over an app: the shell's toast, the app's own icon.
fn n_toast(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        tabs: &["1", "2"],
        front_tab: 0,
        context: "~/code",
        ..top("square-terminal", "zsh")
    };
    widgets::status_bar(&mut c, t, &s);
    let lines = [
        "rezor@zero ~/code $ cargo test",
        "   Compiling gh0st-core v0.5.0",
        "    Finished test in 38.4s",
        "     Running unittests src/lib.rs",
        "test result: ok. 214 passed",
        "rezor@zero ~/code $ ",
    ];
    for (i, line) in lines.iter().enumerate() {
        t.f.mono.draw(&mut c, 6, 32 + i as i32 * 14, line, t.c.fg);
    }
    let bottom = t.m.height - t.m.hint_h;
    let v = Visual::System { icon: APP_ICON };
    widgets::notice_toast(&mut c, t, bottom, &v, "Updates", "12 updates ready");
    c
}

fn n_switcher(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let dock = [
        Visual::System {
            icon: "square-terminal",
        },
        Visual::System { icon: "settings" },
    ];
    let status = Status {
        dock: &dock,
        front: Some(1),
        notices: 2,
        ..top("", "")
    };
    widgets::status_bar(&mut c, t, &status);
    let b = widgets::grid(t);
    let w = (b.w - 2 * t.m.gap) / 3;
    let cards = [
        ("Home", None, "layout-grid"),
        ("Terminal", Some("12M 1%"), "square-terminal"),
        ("Settings", Some("6M 0%"), "settings"),
    ];
    for (i, (label, stats, ic)) in cards.into_iter().enumerate() {
        let r = Rect::new(b.x + i as i32 * (w + t.m.gap), b.y, w, b.h);
        let cd = Card {
            label,
            note: "",
            stats,
            visual: Visual::System { icon: ic },
            paused: false,
        };
        if i == 1 {
            widgets::selection(&mut c, t, r);
            widgets::selection_ring(&mut c, t, r);
        }
        widgets::card_content(&mut c, t, r, &cd, i == 1);
    }
    let slots = [
        l("copy-x", "All", Look::Danger),
        n("bell", "2"),
        n("lock", "Lock"),
        NONE,
        l("circle-x", "Close", Look::Danger),
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

struct Notice {
    icon: &'static str,
    title: &'static str,
    note: &'static str,
    age: &'static str,
}

const NOTICES: [Notice; 3] = [
    Notice {
        icon: APP_ICON,
        title: "12 updates ready",
        note: "Gh0st 0.5, 3 security fixes · 34 MB",
        age: "5 min",
    },
    Notice {
        icon: "mic",
        title: "Recording saved",
        note: "Memo 12 · 4:31",
        age: "1 h",
    },
    Notice {
        icon: "battery-full",
        title: "Charged",
        note: "Unplug to save the battery",
        age: "3 h",
    },
];

fn n_panel(t: &Theme, sel: usize, held: Option<u8>) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("bell", "Notifications"));
    let y = 20;
    for (i, nt) in NOTICES.iter().enumerate() {
        let r = rows_from(t, y, t.m.app_row_h + 2, i);
        let r = Rect::new(r.x, r.y, r.w, t.m.app_row_h);
        let tint = if i == 1 { t.c.tints[3] } else { t.c.accent };
        let unread = i == 0;
        let rw = row(nt.icon, tint, nt.title, Trailing::Value(nt.age));
        widgets::note_row(&mut c, t, r, &rw, nt.note, i == sel);
        if unread {
            c.circle((r.right() - 4) as f32, (r.y + 6) as f32, 2.5, t.c.accent);
        }
    }
    let all = match held {
        Some(p) => l("copy-x", "All", Look::Hold(p)),
        None => l("copy-x", "All", Look::Danger),
    };
    let slots = [all, NONE, NONE, NONE, n("check", "Clear")];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn n_empty(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("bell", "Notifications"));
    let m = Message {
        icon: "bell",
        tone: None,
        text: "No notifications",
        note: "Apps and updates leave them here",
        keys: &[],
    };
    widgets::message(&mut c, t, widgets::body(t), &m);
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

fn n_home_menu(t: &Theme) -> Canvas {
    let mut c = n_home(t);
    let (x, w, kw) = (6, t.m.width - 12, 30);
    let b = widgets::body(t);
    let h = 15 + 3 * 22 + 4;
    let r = Rect::new(x, b.bottom() - h - 2, w, h);
    c.round_rect(r, t.m.radius_large, t.c.bg);
    c.stroke_round_rect(r, t.m.radius_large, t.c.border);
    let mut y = r.y + 2;
    widgets::section(&mut c, t, Rect::new(x + 2, y, w - 4, 15), "Home");
    y += 15;
    let rows: [(Row, &str); 3] = [
        (
            row(
                "bell",
                t.c.icon_tint("bell"),
                "Notifications",
                Trailing::Tag("2", Tone::Info),
            ),
            "",
        ),
        (
            row("pin", t.c.icon_tint("pin"), "Unpin Updates", Trailing::None),
            "p",
        ),
        (
            row(
                "layout-grid",
                t.c.icon_tint("layout-grid"),
                "List layout",
                Trailing::None,
            ),
            "l",
        ),
    ];
    for (i, (rw, k)) in rows.iter().enumerate() {
        widgets::menu_row(
            &mut c,
            t,
            Rect::new(x + 2, y, w - 4, 22),
            rw,
            k,
            kw,
            i == 0,
            false,
        );
        y += 22;
    }
    let mut slots = home_slots();
    slots[4] = l("menu", "", Look::On);
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn n_lock(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        clock: "",
        ..top("", "")
    };
    let l = widgets::Lock {
        clock: "14:32",
        date: "Sunday 22 November",
        field: None,
        ready: false,
        note: Some((Tone::Info, "2 notifications")),
        bell: true,
    };
    widgets::lock_screen(&mut c, t, &s, &l);
    c
}

// ---------------------------------------------------------------- sheet

fn sheet(t: &Theme, screens: &[(&str, &str, Canvas)]) -> Canvas {
    let (w, h, gap, cap, cols) = (t.m.width, t.m.height, 10, 18, 3);
    let n = screens.len() as i32;
    let rows_n = (n + cols - 1) / cols;
    let mut s = Canvas::new(cols * (w + gap) + gap, rows_n * (h + gap + cap) + gap);
    s.fill(s.bounds(), t.c.surface);
    for (i, (name, caption, c)) in screens.iter().enumerate() {
        let ox = gap + (i as i32 % cols) * (w + gap);
        let oy = gap + (i as i32 / cols) * (h + gap + cap);
        let label = format!("{name}  {caption}");
        text_in(
            &mut s,
            t.f.small_bold,
            Rect::new(ox, oy, w, cap - 3),
            &label,
            t.c.fg,
        );
        blit(&mut s, c, ox, oy + cap);
        s.stroke_round_rect(Rect::new(ox - 1, oy + cap - 1, w + 2, h + 2), 1, t.c.border);
    }
    s
}

type Screen = (&'static str, &'static str, fn(&Theme) -> Canvas, bool);

fn main() {
    let out = PathBuf::from("mockups/updates");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    // (name, caption, draw, in every palette)
    let all: &[Screen] = &[
        ("u01-never", "Updates: never checked", u_never, false),
        ("u02-checking", "checking: Stop on 5", u_checking, false),
        ("u03-current", "up to date", u_current, false),
        (
            "u04a-available",
            "A: ready, one message, Enter installs",
            u_available,
            true,
        ),
        (
            "u04b-available",
            "B: ready, grouped rows, Enter installs",
            u_available_list,
            true,
        ),
        (
            "u05-confirm",
            "after the password: confirm",
            u_confirm,
            false,
        ),
        ("u06-download", "downloading: Stop on 5", u_download, false),
        ("u07-install", "installing: no Stop", u_install, true),
        ("u08-done", "done: Enter restarts the device", u_done, false),
        (
            "u09-done-shell",
            "done, only Gh0st changed",
            u_done_shell,
            false,
        ),
        (
            "u10-kernel-battery",
            "kernel set at 50% or less",
            u_kernel_battery,
            false,
        ),
        ("u11-offline", "error: no network", u_offline, false),
        ("u12-packages", "6 List: by group", u_list, false),
        ("u13-detail", "Enter on a package", u_detail, false),
        ("u14-menu", "menu (8)", u_menu, false),
        (
            "s01-system",
            "Settings > System: Updates row",
            s_system,
            false,
        ),
        (
            "s02-updates",
            "Settings > System > Updates",
            |t| s_updates(t, 0),
            true,
        ),
        ("s03-edge", "Channel to Edge asks first", s_edge, false),
        (
            "s04-privacy",
            "Privacy and setup: the same row",
            s_privacy,
            false,
        ),
        ("n01-toast", "a notice over an app", n_toast, true),
        (
            "n02-home",
            "home: bell in the bar, count on the tile",
            n_home,
            true,
        ),
        (
            "n03-home-menu",
            "home menu (8): Notifications",
            n_home_menu,
            false,
        ),
        (
            "n04-switcher",
            "switcher: 5 opens notifications",
            n_switcher,
            false,
        ),
        (
            "n05-panel",
            "notifications: Enter opens, 8 clears one",
            |t| n_panel(t, 0, None),
            true,
        ),
        (
            "n06-panel-all",
            "hold 4: clears all",
            |t| n_panel(t, 1, Some(60)),
            false,
        ),
        ("n07-empty", "no notifications", n_empty, false),
        ("n08-lock", "lock screen: only a count", n_lock, false),
    ];
    for palette in theme::PALETTES {
        let t = Theme::new(*palette);
        let mint = palette.name == "Mint";
        let screens: Vec<(&str, &str, Canvas)> = all
            .iter()
            .filter(|s| mint || s.3)
            .map(|&(name, cap, draw, _)| (name, cap, draw(&t)))
            .collect();
        let tag = palette.name.to_lowercase();
        for (name, _, c) in &screens {
            write_png(c, &out.join(format!("{tag}-{name}.png")), 1).unwrap();
            write_png(c, &out.join(format!("{tag}-{name}@3x.png")), 3).unwrap();
        }
        let s = sheet(&t, &screens);
        write_png(&s, &out.join(format!("{tag}-sheet.png")), 1).unwrap();
        write_png(&s, &out.join(format!("{tag}-sheet@2x.png")), 2).unwrap();
    }
    println!("wrote {}", out.display());
}
