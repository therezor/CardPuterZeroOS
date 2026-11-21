//! Every screen in docs/design.md, drawn with the real gh0st-ui widgets at
//! panel size and 3x into mockups/design, a captioned contact sheet per
//! palette, and the doc's pictures at 2x into docs/design. Run from the
//! workspace root:
//!
//!     cargo run -p gh0st-ui --example design

use gh0st_ui::color::{hex, mix};
use gh0st_ui::font::Font;
use gh0st_ui::icon::{self, IconSize};
use gh0st_ui::snapshot::write_png;
use gh0st_ui::theme::{self, Theme, Tone};
use gh0st_ui::widgets::{
    self, AppItem, Battery, Card, Chip, Hint, Look, Message, Net, Row, Status, Trailing, Visual,
    soft,
    text_in,
};
use gh0st_ui::{Canvas, Color, Rect};
use std::path::PathBuf;

const CLOCK: &str = "14:32";

// ---------------------------------------------------------------- bars

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

const HELP: Hint<'static> = Hint {
    key: "Fn h",
    label: "help",
};

fn toast(c: &mut Canvas, t: &Theme, tone: Tone, text: &str, held: Option<u8>) {
    widgets::toast(c, t, t.m.height - t.m.hint_h, tone, text, held);
}

// ---------------------------------------------------------------- content

fn frame(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    c
}

fn rows_from(t: &Theme, y: i32, row_h: i32, i: usize) -> Rect {
    Rect::new(t.m.margin, y + i as i32 * row_h, t.m.width - 2 * t.m.margin, row_h)
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

fn para(c: &mut Canvas, t: &Theme, r: Rect, text: &str) {
    for (i, line) in wrap(t.f.body, text, r.w).iter().enumerate() {
        let lr = Rect::new(r.x, r.y + i as i32 * 18, r.w, 18);
        text_in(c, t.f.body, lr, line, soft(t));
    }
}

fn field_label(c: &mut Canvas, t: &Theme, y: i32, s: &str, col: Color) {
    text_in(c, t.f.small, Rect::new(15, y, t.m.width - 30, 13), s, col);
}

fn field(c: &mut Canvas, t: &Theme, y: i32, text: &str, masked: bool, focused: bool, error: bool) {
    let r = Rect::new(12, y, t.m.width - 24, 22);
    widgets::text_field(c, t, r, text, masked, focused, "");
    if error {
        c.stroke_round_rect(r, t.m.radius, t.c.critical);
    }
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

// ---------------------------------------------------------------- screens

fn settings_rows(t: &Theme) -> Vec<Row<'static>> {
    let a = t.c.accent;
    vec![
        row("wifi", a, "Wi-Fi", Trailing::ValueChevron("HomeNet")),
        row("bluetooth", a, "Bluetooth", Trailing::Toggle(true)),
        row("palette", a, "Palette", Trailing::Choice("Mint")),
        row("sun", a, "Brightness", Trailing::Level(80)),
        row("volume-2", a, "Sound", Trailing::ValueChevron("60%")),
    ]
}

fn settings(t: &Theme, sel: usize, slots: Slots) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("settings", "Settings"));
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, rw) in settings_rows(t).iter().enumerate() {
        widgets::list_row(&mut c, t, rows_from(t, area.y, t.m.row_h, i), rw, i == sel);
    }
    widgets::scrollbar(&mut c, t, area, 0, 5, 9);
    widgets::action_bar(&mut c, t, &slots);
    c
}

/// Settings' own keys: Enter opens, Esc goes back, n finds.
fn link_slots() -> Slots<'static> {
    [NONE, n("search", "Find"), NONE, NONE, menu()]
}

fn roles(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("palette", "Colour roles", t.c.name));
    let k = &t.c;
    let cells: [(&str, Color); 14] = [
        ("bg", k.bg),
        ("surface", k.surface),
        ("border", k.border),
        ("fg", k.fg),
        ("soft", k.soft()),
        ("muted", k.muted),
        ("disabled", k.disabled()),
        ("accent", k.accent),
        ("accent_fg", k.accent_fg),
        ("info", k.info),
        ("ok", k.ok),
        ("warning", k.warning),
        ("critical", k.critical),
        ("tints", k.tints[0]),
    ];
    let (w, h) = (74, 31);
    for (i, &(name, col)) in cells.iter().enumerate() {
        let (x, y) = (6 + (i as i32 % 4) * (w + 4), 22 + (i as i32 / 4) * (h + 2));
        if name == "tints" {
            for (j, tint) in k.tints.iter().enumerate() {
                c.round_rect(Rect::new(x + j as i32 * 12, y, 11, 16), 2, *tint);
            }
        } else {
            let sw = Rect::new(x, y, w, 16);
            c.round_rect(sw, t.m.radius, col);
            c.stroke_round_rect(sw, t.m.radius, k.border);
        }
        text_in(&mut c, t.f.small, Rect::new(x + 1, y + 17, w, 13), name, k.muted);
    }
    c
}

fn type_scale(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("tag", "Type and icons"));
    let mut y = 22;
    let lines: [(Font, &str, Color, i32); 7] = [
        (t.f.big, "Display 24", t.c.fg, 26),
        (t.f.title, "Title 17", t.c.fg, 20),
        (t.f.body_bold, "Body strong 15", t.c.fg, 18),
        (t.f.body, "Body 15", soft(t), 18),
        (t.f.small_bold, "Label 13", t.c.accent, 16),
        (t.f.small, "Caption 13", t.c.muted, 16),
        (t.f.mono, "Mono 10  ~/code $ ls", t.c.fg, 14),
    ];
    for (f, s, col, h) in lines {
        text_in(&mut c, f, Rect::new(12, y, 180, h), s, col);
        y += h;
    }
    let sizes = [
        (IconSize::Tiny, "12 bars"),
        (IconSize::Small, "14 rows"),
        (IconSize::Bar, "16"),
        (IconSize::Medium, "26 tiles"),
        (IconSize::Large, "36 large"),
    ];
    let mut y = 22;
    for (s, label) in sizes {
        let px = s as i32;
        icon::draw(&mut c, "settings", s, 206, y, t.c.accent);
        text_in(&mut c, t.f.small, Rect::new(248, y, 70, px.max(13)), label, t.c.muted);
        y += px + 4;
    }
    c
}

fn top_rows(t: &Theme, bars: &[(&str, Status)]) -> Canvas {
    let mut c = frame(t);
    let mut y = 0;
    for (caption, b) in bars {
        text_in(&mut c, t.f.small, Rect::new(8, y, 300, 13), caption, t.c.muted);
        y += 13;
        let mut bar = Canvas::new(t.m.width, t.m.bar_h);
        widgets::status_bar(&mut bar, t, b);
        blit(&mut c, &bar, 0, y);
        y += t.m.bar_h + 3;
    }
    c
}

fn top_a(t: &Theme) -> Canvas {
    let tabs = Status {
        tabs: &["1", "2", "3"],
        front_tab: 1,
        context: "~/code",
        ..top("square-terminal", "zsh")
    };
    let bt = Status {
        bluetooth: true,
        ..top("pencil", "Notes")
    };
    let cable = Status {
        net: Net::Ethernet,
        battery: Some(Battery {
            percent: 64,
            charging: true,
        }),
        ..top("pencil", "Notes")
    };
    top_rows(
        t,
        &[
            ("First page: app icon and name; Wi-Fi, battery, clock", top("settings", "Settings")),
            ("Deeper: the page in bold, its parent muted", sub("wifi", "Wi-Fi", "Settings")),
            ("Tabs for sessions, the front one underlined", tabs),
            ("Bluetooth on", bt),
            ("On a cable, charging", cable),
        ],
    )
}

fn top_b(t: &Theme) -> Canvas {
    let rec = Status {
        item: Some(("REC", Tone::Critical)),
        ..top("mic", "Recorder")
    };
    let over = Status {
        bluetooth: true,
        ..sub("pencil", "Holiday plans and packing list.txt", "Notes")
    };
    let low = Status {
        battery: Some(Battery {
            percent: 4,
            charging: false,
        }),
        net: Net::None,
        ..top("settings", "Settings")
    };
    let warn = Status {
        battery: Some(Battery {
            percent: 15,
            charging: false,
        }),
        ..top("settings", "Settings")
    };
    top_rows(
        t,
        &[
            ("App item: one word in a tone", rec),
            ("Overflow: context, then Bluetooth, then title", over),
            ("Battery at 4%, no network", low),
            ("Battery at 15%", warn),
        ],
    )
}

fn chip_states(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("tag", "Chip states"));
    let rows: [[(Chip, &str); 5]; 3] = [
        [
            (n("search", "Find"), "normal"),
            (l("arrow-right", "Next", Look::Main), "main"),
            (l("sun", "Flash", Look::On), "on"),
            (l("trash-2", "Forget", Look::Danger), "danger"),
            (l("trash-2", "Erase", Look::Hold(40)), "hold red"),
        ],
        [
            (l("check", "Join", Look::Disabled), "disabled"),
            (l("search", "Find", Look::Pressed), "pressed"),
            (l("square", "Stop", Look::Danger), "danger"),
            (l("check", "Join", Look::Busy), "busy"),
            (menu(), "icon only"),
        ],
        [
            (n("chevron-left", "Prev"), "z"),
            (n("chevron-right", "Next"), "c"),
            (n("search", "Find"), "s"),
            (n("eye", "Show"), "Tab"),
            (menu(), "Fn 8"),
        ],
    ];
    let captions = ["", "", "Other keys that run the same chip"];
    for (ri, r) in rows.iter().enumerate() {
        let y = 24 + ri as i32 * 38;
        if !captions[ri].is_empty() {
            let cr = Rect::new(8, y - 2, 300, 12);
            text_in(&mut c, t.f.small, cr, captions[ri], t.c.muted);
        }
        let cy = y + if captions[ri].is_empty() { 0 } else { 11 };
        let chips: Slots = r.map(|(ch, _)| ch);
        let w = widgets::bar_widths(t, &chips);
        for (i, (ch, cap)) in r.iter().enumerate() {
            let cx = t.m.softkey_x[i];
            widgets::chip(&mut c, t, cx, cy, ch, w[i], None);
            let cr = Rect::new(cx - 28, cy + t.m.chip_h + 1, 56, 13);
            widgets::text_centre(&mut c, t.f.small, cr, cap, t.c.muted);
        }
    }
    widgets::action_bar(&mut c, t, &link_slots());
    c
}

fn rows_a(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("cpu", "System", "Settings"));
    let a = t.c.accent;
    let (y, h) = (20, t.m.row_h);
    let rows = [
        row("info", a, "Version", Trailing::Value("0.4.1")),
        row("globe", a, "English (UK)", Trailing::Check),
        row("download", a, "Updates", Trailing::Tag("3", Tone::Info)),
    ];
    for (i, rw) in rows.iter().enumerate() {
        widgets::list_row(&mut c, t, rows_from(t, y, h, i), rw, false);
    }
    widgets::disabled_row(&mut c, t, rows_from(t, y, h, 3), "usb", "USB gadget", "needs a cable");
    let forget = row("trash-2", t.c.critical, "Forget network", Trailing::Alert("Press again"));
    widgets::list_row(&mut c, t, rows_from(t, y, h, 4), &forget, true);
    let slots = [NONE, NONE, l("trash-2", "Forget", Look::DangerMain), NONE, menu()];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn rows_b(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("wifi", "Wi-Fi", "Settings"));
    let a = t.c.accent;
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Connected");
    y += 15;
    let home = row("wifi", a, "HomeNet", Trailing::Signal { level: 3, secure: false });
    let note = "Connected, 5 GHz, 192.168.1.42";
    widgets::note_row(&mut c, t, rows_from(t, y, t.m.app_row_h, 0), &home, note, true);
    y += t.m.app_row_h + 2;
    widgets::section(&mut c, t, Rect::new(6, y, 300, 15), "Nearby");
    y += 15;
    let office = row("wifi", a, "Office-5G", Trailing::Signal { level: 4, secure: true });
    let cafe = row("wifi", a, "Cafe Guest", Trailing::Signal { level: 2, secure: false });
    widgets::list_row(&mut c, t, rows_from(t, y, t.m.row_h, 0), &office, false);
    widgets::list_row(&mut c, t, rows_from(t, y, t.m.row_h, 1), &cafe, false);
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    widgets::scrollbar(&mut c, t, area, 0, 4, 7);
    let slots = [
        NONE,
        n("refresh-cw", "Scan"),
        l("trash-2", "Forget", Look::Danger),
        NONE,
        menu(),
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn picker(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("palette", "Palette", "Settings"));
    let a = t.c.accent;
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, name) in ["Mint", "Nord", "Paper"].iter().enumerate() {
        let tr = if i == 0 { Trailing::Check } else { Trailing::None };
        let r = rows_from(t, area.y, t.m.row_h, i);
        widgets::list_row(&mut c, t, r, &row("palette", a, name, tr), i == 1);
    }
    widgets::action_bar(&mut c, t, &[NONE, NONE, NONE, NONE, menu()]);
    c
}

fn form(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("user", "Account", "Setup 7 of 10"));
    field_label(&mut c, t, 21, "Your name on this device", t.c.muted);
    field(&mut c, t, 35, "alex", false, false, false);
    field_label(&mut c, t, 62, "Password, 4 or more", t.c.muted);
    field(&mut c, t, 76, "abc", true, true, true);
    icon::draw(&mut c, "triangle-alert", IconSize::Tiny, 15, 103, t.c.critical);
    let er = Rect::new(31, 101, 260, 16);
    text_in(&mut c, t.f.small, er, "Too short: 4 or more", t.c.critical);
    let h = hints(&[("Esc", "back"), ("Tab", "next field"), ("Enter", "next")]);
    widgets::hint_bar(&mut c, t, &h, Some(HELP));
    c
}

fn wifi_behind(c: &mut Canvas, t: &Theme) {
    widgets::status_bar(c, t, &sub("wifi", "Wi-Fi", "Settings"));
    let a = t.c.accent;
    let names = ["HomeNet", "Office-5G", "Cafe Guest", "Neighbours", "Library"];
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, s) in names.iter().enumerate() {
        let rw = row("wifi", a, s, Trailing::Signal { level: 3, secure: true });
        widgets::list_row(c, t, rows_from(t, area.y, t.m.row_h, i), &rw, i == 1);
    }
}

/// Field, then a 6 px gap, a 15 px line, and 6 px again to the card's edge.
fn join_card(c: &mut Canvas, t: &Theme) -> Rect {
    scrim(c, t);
    let title = "Join \u{201c}Office-5G\u{201d}";
    let body = widgets::card(c, t, Rect::new(22, 34, 276, 80), title, t.c.accent);
    let fr = Rect::new(body.x, body.y + 2, body.w, 22);
    widgets::text_field(c, t, fr, "hunter22", true, true, "");
    fr
}

fn dialog_typing(t: &Theme) -> Canvas {
    let mut c = frame(t);
    wifi_behind(&mut c, t);
    let fr = join_card(&mut c, t);
    let note = "Password for a WPA2 network";
    let nr = Rect::new(fr.x, fr.bottom() + 6, fr.w, 15);
    text_in(&mut c, t.f.small, nr, note, t.c.muted);
    let h = hints(&[("Esc", "cancel"), ("Tab", "show"), ("Enter", "join")]);
    widgets::hint_bar(&mut c, t, &h, Some(HELP));
    c
}

fn dialog_busy(t: &Theme) -> Canvas {
    let mut c = frame(t);
    wifi_behind(&mut c, t);
    let fr = join_card(&mut c, t);
    let nr = Rect::new(fr.x, fr.bottom() + 6, 80, 15);
    text_in(&mut c, t.f.small, nr, "Joining\u{2026}", t.c.muted);
    let bar = Rect::new(100, fr.bottom() + 12, 186, 3);
    c.round_rect(bar, 1, t.c.border);
    c.round_rect(Rect::new(bar.x + 90, bar.y, 60, bar.h), 1, t.c.accent);
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

fn dialog_danger(t: &Theme) -> Canvas {
    let mut c = settings(t, 0, [NONE; 5]);
    scrim(&mut c, t);
    let r = Rect::new(22, 30, 276, 92);
    let body = widgets::card(&mut c, t, r, "Erase everything?", t.c.critical);
    let text = "Apps, files and settings are deleted. This can't be undone.";
    para(&mut c, t, body, text);
    widgets::key_row(&mut c, t, body.x, body.y + 40, &hints(&[("Esc", "cancel")]));
    let hint = "hold 7 to erase";
    let hx = body.right() - t.f.small.measure(hint);
    let hr = Rect::new(hx, body.y + 40, body.w, t.m.pill_h);
    text_in(&mut c, t.f.small, hr, hint, t.c.muted);
    let slots = [NONE, NONE, NONE, l("trash-2", "Erase", Look::Hold(40)), NONE];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn dialog_three(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    scrim(&mut c, t);
    let r = Rect::new(22, 30, 276, 92);
    let body = widgets::card(&mut c, t, r, "Save changes?", t.c.accent);
    para(&mut c, t, body, "Packing list has edits you haven't saved.");
    let h = hints(&[("Esc", "cancel"), ("Enter", "save")]);
    widgets::key_row(&mut c, t, body.x, body.y + 40, &h);
    let slots = [NONE, NONE, l("trash-2", "Discard", Look::Danger), NONE, NONE];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn app_menu(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("menu", "Menu", "Settings"));
    let a = t.c.accent;
    let (x, w, kw) = (6, t.m.width - 12, 40);
    let mut y = 20;
    widgets::section(&mut c, t, Rect::new(x, y, w, 15), "Settings");
    y += 15;
    let rows: [(Row, &str); 2] = [
        (row("search", a, "Search", Trailing::None), "n"),
        (row("layout-grid", a, "Top bar", Trailing::Toggle(true)), "b"),
    ];
    for (i, (rw, k)) in rows.iter().enumerate() {
        widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), rw, k, kw, i == 0, false);
        y += 22;
    }
    widgets::section(&mut c, t, Rect::new(x, y, w, 15), "App");
    y += 15;
    let help = row("keyboard", a, "Help", Trailing::Chevron);
    widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), &help, "Fn h", kw, false, false);
    y += 22;
    let close = row("circle-x", t.c.critical, "Close app", Trailing::None);
    widgets::menu_row(&mut c, t, Rect::new(x, y, w, 22), &close, "", kw, false, false);
    let slots = [NONE, NONE, NONE, NONE, l("menu", "", Look::On)];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn help(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("keyboard", "Help", "Settings"));
    let (x, w, kw) = (6, t.m.width - 12, 62);
    let mut y = 19;
    let lines: [(&str, &str); 7] = [
        ("#", "Under the screen"),
        ("5 n", "Find"),
        ("8", "Menu"),
        ("#", "Keys"),
        ("Esc", "Back; hold to exit"),
        ("Enter", "Open"),
        ("Fn k", "Switch apps"),
    ];
    for (k, label) in lines {
        if k == "#" {
            widgets::section(&mut c, t, Rect::new(x, y, w, 16), label);
            y += 16;
        } else {
            widgets::keys_line(&mut c, t, Rect::new(x, y, w, 19), k, kw, label, "");
            y += 19;
        }
    }
    widgets::scrollbar(&mut c, t, Rect::new(x, 20, w, 128), 0, 7, 12);
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

fn toasts(t: &Theme) -> Canvas {
    let mut c = settings(t, 0, link_slots());
    scrim(&mut c, t);
    let items = [
        (Tone::Ok, "Saved to Screenshots"),
        (Tone::Info, "Hold to exit"),
        (Tone::Warning, "Battery at 15%"),
        (Tone::Critical, "Couldn't join Office-5G"),
    ];
    for (i, (k, s)) in items.into_iter().enumerate() {
        widgets::toast(&mut c, t, 48 + i as i32 * 26, k, s, None);
    }
    c
}

fn toast_float(t: &Theme) -> Canvas {
    let mut c = settings(t, 1, link_slots());
    toast(&mut c, t, Tone::Ok, "Bluetooth on", None);
    c
}

const NOTES: [(&str, &str); 4] = [
    ("Shopping", "Today"),
    ("Packing list", "Mon"),
    ("Ideas", "Nov 12"),
    ("Wi-Fi codes", "Oct 3"),
];

fn notes_first(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("pencil", "Notes"));
    let a = t.c.accent;
    let (area, _) = widgets::rows_area(t, t.m.row_h);
    for (i, (s, v)) in NOTES.iter().enumerate() {
        let rw = row("pencil", a, s, Trailing::Value(v));
        widgets::list_row(&mut c, t, rows_from(t, area.y, t.m.row_h, i), &rw, i == 0);
    }
    widgets::action_bar(&mut c, t, &[NONE, n("plus", "New"), NONE, NONE, menu()]);
    c
}

fn esc_tap(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    toast(&mut c, t, Tone::Info, "Hold to exit", Some(0));
    c
}

fn esc_hold(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    toast(&mut c, t, Tone::Info, "Hold to exit", Some(60));
    c
}

fn esc_hung(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    widgets::banner(&mut c, t, "Not responding. Hold Esc to close it", Tone::Warning);
    c
}

fn low_battery(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    widgets::banner(&mut c, t, "Battery 3%: powering off in 30 s", Tone::Critical);
    c
}

fn volume(t: &Theme) -> Canvas {
    let mut c = notes_first(t);
    widgets::level_bar(&mut c, t, "volume-2", "Volume", 60);
    c
}

fn progress_screen(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("download", "Updates", "Settings"));
    let w = t.m.width - 30;
    widgets::progress(&mut c, t, Rect::new(15, 26, w, 24), "Downloading gh0st 0.5", Some(42), 0);
    let note = "18 of 43 MB, about a minute left";
    text_in(&mut c, t.f.small, Rect::new(15, 50, 290, 13), note, t.c.muted);
    widgets::progress(&mut c, t, Rect::new(15, 76, w, 24), "Checking the download", None, 76);
    let note = "Moves only while it is on screen";
    text_in(&mut c, t.f.small, Rect::new(15, 100, 290, 13), note, t.c.muted);
    widgets::action_bar(&mut c, t, &[NONE, NONE, l("square", "Stop", Look::Danger), NONE, menu()]);
    c
}

fn empty(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        net: Net::None,
        ..sub("wifi", "Wi-Fi", "Settings")
    };
    widgets::status_bar(&mut c, t, &s);
    let note = "Move closer to the router, then scan again.";
    let keys = hints(&[("Enter", "scan")]);
    let b = widgets::body(t);
    let m = Message {
        icon: "wifi-off",
        tone: None,
        text: "No networks found",
        note,
        keys: &keys,
    };
    widgets::message(&mut c, t, b, &m);
    widgets::action_bar(&mut c, t, &[NONE, NONE, NONE, NONE, menu()]);
    c
}

fn error(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        net: Net::None,
        ..sub("wifi", "Wi-Fi", "Settings")
    };
    widgets::status_bar(&mut c, t, &s);
    let note = "The password was not accepted.";
    let keys = hints(&[("Enter", "try again")]);
    let b = widgets::body(t);
    let m = Message {
        icon: "circle-x",
        tone: Some(Tone::Critical),
        text: "Couldn't join Office-5G",
        note,
        keys: &keys,
    };
    widgets::message(&mut c, t, b, &m);
    widgets::action_bar(&mut c, t, &[NONE, NONE, n("pencil", "Edit"), NONE, menu()]);
    c
}

const READER: &str = "The panel sits right above the number row, so an action on screen is drawn above the key that runs it. Press the key under a chip and that chip runs. Enter and Esc need no chip: Enter opens or does the main thing, Esc goes back one step, and holding Esc on the first screen closes the app. Key 8 opens the menu, and Fn and H list every key.";

fn reader(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = sub("info", "Keys under the screen", "Help 2 of 5");
    widgets::status_bar(&mut c, t, &s);
    let b = widgets::body(t);
    for (i, line) in wrap(t.f.body, READER, b.w - 30).iter().take(7).enumerate() {
        let r = Rect::new(15, b.y + 3 + i as i32 * 18, b.w - 30, 18);
        text_in(&mut c, t.f.body, r, line, t.c.fg);
    }
    widgets::scrollbar(&mut c, t, Rect::new(6, b.y + 2, 306, 128), 1, 7, 30);
    let slots = [
        NONE,
        n("chevron-up", "PgUp"),
        n("chevron-down", "PgDn"),
        NONE,
        menu(),
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn find(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &sub("search", "Find", "Home"));
    let fr = Rect::new(12, 21, t.m.width - 24, 22);
    widgets::text_field(&mut c, t, fr, "re", false, true, "");
    let a = t.c.accent;
    let hits = [
        ("mic", "Recorder", "Sound and voice notes"),
        ("settings", "Settings", "Wi-Fi, display, about"),
    ];
    for (i, (ic, s, v)) in hits.iter().enumerate() {
        let r = rows_from(t, 46, t.m.app_row_h, i);
        widgets::note_row(&mut c, t, r, &row(ic, a, s, Trailing::None), v, i == 0);
    }
    let r = rows_from(t, 46 + 2 * t.m.app_row_h, t.m.row_h, 0);
    widgets::list_row(&mut c, t, r, &row("download", a, "Updates", Trailing::None), false);
    let h = hints(&[("Esc", "cancel"), ("Enter", "open")]);
    widgets::hint_bar(&mut c, t, &h, Some(HELP));
    c
}

fn level_row(t: &Theme) -> Canvas {
    let slots = [NONE, n("minus", "Less"), n("plus", "More"), NONE, menu()];
    settings(t, 3, slots)
}

fn choice_row(t: &Theme) -> Canvas {
    let slots = [
        NONE,
        n("chevron-left", "Prev"),
        n("chevron-right", "Next"),
        NONE,
        menu(),
    ];
    settings(t, 2, slots)
}

fn home(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let dock = [
        Visual::System {
            icon: "square-terminal",
        },
        Visual::Letter {
            ch: 'm',
            tint: t.c.tints[5],
        },
    ];
    let status = Status {
        dock: &dock,
        front: Some(0),
        bluetooth: true,
        ..top("", "")
    };
    widgets::status_bar(&mut c, t, &status);
    let tints = t.c.tints;
    let items = [
        ("Terminal", Visual::System { icon: "square-terminal" }),
        ("Settings", Visual::System { icon: "settings" }),
        ("Store", Visual::Badge { icon: "shopping-bag", tint: tints[2] }),
        ("Camera", Visual::Badge { icon: "camera", tint: tints[3] }),
        ("Files", Visual::Badge { icon: "folder", tint: tints[2] }),
        ("Music", Visual::Letter { ch: 'm', tint: tints[5] }),
        ("Recorder", Visual::Badge { icon: "mic", tint: tints[3] }),
        ("Notes", Visual::Badge { icon: "pencil", tint: tints[4] }),
    ];
    let area = widgets::grid(t);
    let tw = (area.w - 3 * t.m.gap) / 4;
    let th = (area.h - t.m.gap) / 2;
    for (i, (label, v)) in items.into_iter().enumerate() {
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
        };
        widgets::tile(&mut c, t, r, &item, i == 1);
    }
    widgets::scrollbar(&mut c, t, area, 0, 2, 3);
    let slots = [
        NONE,
        n("search", "Find"),
        n("arrow-left-right", "Apps"),
        NONE,
        menu(),
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn switcher_base(t: &Theme) -> Canvas {
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
        ..top("", "")
    };
    widgets::status_bar(&mut c, t, &status);
    let b = widgets::grid(t);
    let w = (b.w - 2 * t.m.gap) / 3;
    let cards = [
        ("Home", None, "layout-grid", false),
        ("Terminal", Some("12M 1%"), "square-terminal", false),
        ("Settings", Some("6M 0%"), "settings", true),
    ];
    for (i, (label, stats, ic, paused)) in cards.into_iter().enumerate() {
        let r = Rect::new(b.x + i as i32 * (w + t.m.gap), b.y, w, b.h);
        let cd = Card {
            label,
            note: "",
            stats,
            visual: Visual::System { icon: ic },
            paused,
        };
        if i == 1 {
            widgets::selection(&mut c, t, r);
            widgets::selection_ring(&mut c, t, r);
        }
        widgets::card_content(&mut c, t, r, &cd, i == 1);
    }
    c
}

fn switcher(t: &Theme) -> Canvas {
    let mut c = switcher_base(t);
    let slots = [
        NONE,
        l("circle-x", "Close", Look::Danger),
        l("copy-x", "All", Look::Danger),
        NONE,
        NONE,
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

fn close_all(t: &Theme) -> Canvas {
    let mut c = switcher_base(t);
    scrim(&mut c, t);
    let r = Rect::new(22, 30, 276, 92);
    let body = widgets::card(&mut c, t, r, "Close all apps?", t.c.critical);
    para(&mut c, t, body, "Terminal and Settings close. Work you haven't saved is lost.");
    let h = hints(&[("Esc", "cancel"), ("Enter", "close all")]);
    widgets::key_row(&mut c, t, body.x, body.y + 40, &h);
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

const TERM: [&str; 9] = [
    "rezor@zero ~/code $ ls",
    "assets  crates  docs  packaging  scripts",
    "rezor@zero ~/code $ cargo build --release",
    "   Compiling gh0st-ui v0.4.1",
    "   Compiling gh0st-app v0.4.1",
    "    Finished release in 41.2s",
    "rezor@zero ~/code $ git status --short",
    " M docs/design.md",
    "rezor@zero ~/code $ ",
];

fn terminal(t: &Theme, scroll: bool) -> Canvas {
    let mut c = frame(t);
    let s = Status {
        tabs: &["1", "2"],
        context: if scroll { "scrolling" } else { "~/code" },
        ..top("square-terminal", "zsh")
    };
    widgets::status_bar(&mut c, t, &s);
    let lh = 12;
    let bottom = if scroll { t.m.height - t.m.hint_h } else { t.m.height };
    let n_lines = ((bottom - 20) / lh) as usize;
    let first = TERM.len().saturating_sub(n_lines);
    for (i, line) in TERM[first..].iter().enumerate() {
        let y = 20 + i as i32 * lh;
        let r = Rect::new(4, y, 312, lh);
        if scroll && i == 3 {
            c.fill(r, mix(t.c.bg, t.c.accent, 60));
        }
        let col = if line.starts_with("rezor") { t.c.accent } else { t.c.fg };
        text_in(&mut c, t.f.mono, r, line, col);
        if !scroll && i + first == TERM.len() - 1 {
            let x = 4 + t.f.mono.measure(line);
            c.fill(Rect::new(x, y + 1, 6, lh - 2), t.c.fg);
        }
    }
    if scroll {
        let slots = [NONE, n("search", "Find"), n("copy", "Copy"), NONE, menu()];
        widgets::action_bar(&mut c, t, &slots);
    } else {
        let text = "Fn Esc sends Esc. Fn h: help";
        widgets::toast(&mut c, t, t.m.height, Tone::Info, text, None);
    }
    c
}

fn camera(t: &Theme) -> Canvas {
    let mut c = frame(t);
    widgets::status_bar(&mut c, t, &top("camera", "Camera"));
    let b = widgets::body(t);
    for y in b.y..b.bottom() {
        let k = ((y - b.y) * 255 / b.h) as u8;
        c.hline(0, t.m.width, y, mix(hex("#2a4a6a"), hex("#c08a50"), k));
    }
    let f = Rect::new(130, 55, 60, 60);
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let x = if dx == 0 { f.x } else { f.right() - 10 };
        let y = if dy == 0 { f.y } else { f.bottom() - 2 };
        c.fill(Rect::new(x, y, 10, 2), t.c.fg);
        let x = if dx == 0 { f.x } else { f.right() - 2 };
        let y = if dy == 0 { f.y } else { f.bottom() - 10 };
        c.fill(Rect::new(x, y, 2, 10), t.c.fg);
    }
    let slots = [
        NONE,
        n("zoom-out", "Out"),
        n("zoom-in", "In"),
        n("images", "Album"),
        menu(),
    ];
    widgets::action_bar(&mut c, t, &slots);
    c
}

// ---------------------------------------------------------------- output

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
        text_in(&mut s, t.f.small_bold, Rect::new(ox, oy, w, cap - 3), &label, t.c.fg);
        blit(&mut s, c, ox, oy + cap);
        s.stroke_round_rect(Rect::new(ox - 1, oy + cap - 1, w + 2, h + 2), 1, t.c.border);
    }
    s
}

/// Screens docs/design.md shows.
const DOC: [&str; 14] = [
    "01-roles",
    "03-top-a",
    "05-chips",
    "06-list",
    "08-list-choice",
    "14-typing",
    "16-dialog-typing",
    "19-dialog-danger",
    "21-menu",
    "22-help",
    "23-toasts",
    "27-esc-hold",
    "36-home",
    "37-switcher",
];

type Screen = (&'static str, &'static str, fn(&Theme) -> Canvas, bool);

fn main() {
    let out = PathBuf::from("mockups/design");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    // (name, caption, draw, in every palette)
    let all: &[Screen] = &[
        ("01-roles", "colour roles", roles, true),
        ("02-type", "type and icon scale", type_scale, false),
        ("03-top-a", "top bar: what can show", top_a, false),
        ("04-top-b", "top bar: app item, overflow, battery", top_b, false),
        ("05-chips", "chip states", chip_states, true),
        ("06-list", "Enter opens, Esc back: no chips for them", |t| settings(t, 0, link_slots()), true),
        ("08-list-choice", "choice row: 5 Prev, 6 Next", choice_row, false),
        ("09-picker", "Enter on a choice opens the full list", picker, false),
        ("10-list-level", "level row: 5 Less, 6 More", level_row, false),
        ("12-rows-a", "rows: value, check, tag, disabled, armed", rows_a, true),
        ("13-rows-b", "rows: sections, two lines, signal", rows_b, false),
        ("14-typing", "typing: a key-hint bar replaces the chips", form, true),
        ("16-dialog-typing", "dialog with a field: hint bar", dialog_typing, true),
        ("18-dialog-busy", "after Enter: busy in the card", dialog_busy, false),
        ("19-dialog-danger", "data loss: hold 7, never Enter", dialog_danger, true),
        ("20-dialog-three", "Enter save, 6 Discard, Esc cancel", dialog_three, false),
        ("21-menu", "app menu (8): Esc goes back one level", app_menu, false),
        ("22-help", "Help (Fn h): every key", help, false),
        ("23-toasts", "toast tones", toasts, true),
        ("24-toast", "a toast floats above the chips", toast_float, false),
        ("26-esc-tap", "Esc tapped on the first page", esc_tap, false),
        ("27-esc-hold", "Esc held: the toast fills over 0.7 s", esc_hold, false),
        ("28-esc-hung", "app hung: banner takes the top bar", esc_hung, false),
        ("29-low-battery", "system banner", low_battery, false),
        ("30-volume", "volume keys: level takes the top bar", volume, false),
        ("31-progress", "progress", progress_screen, false),
        ("32-empty", "empty: Enter does the one action", empty, false),
        ("33-error", "error state", error, false),
        ("34-reader", "long text: PgUp, PgDn", reader, false),
        ("35-find", "find while typing", find, false),
        ("36-home", "home: Find, Apps, Menu", home, true),
        ("37-switcher", "switcher: 5 closes one, 6 all", switcher, false),
        ("37b-close-all", "Close all asks first", close_all, false),
        ("38-term-typing", "terminal: Esc as everywhere, Fn Esc sends Esc", |t| terminal(t, false), false),
        ("39-term-scroll", "terminal scroll mode: chips", |t| terminal(t, true), false),
        ("40-camera", "camera: Enter shoots", camera, false),
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
        // The doc's pictures, at 2x so they read on a desktop screen.
        if mint {
            let docs = PathBuf::from("docs/design");
            std::fs::create_dir_all(&docs).unwrap();
            for (name, _, c) in screens.iter().filter(|s| DOC.contains(&s.0)) {
                write_png(c, &docs.join(format!("{name}.png")), 2).unwrap();
            }
        }
        let s = sheet(&t, &screens);
        write_png(&s, &out.join(format!("{tag}-sheet.png")), 1).unwrap();
        write_png(&s, &out.join(format!("{tag}-sheet@2x.png")), 2).unwrap();
    }
    println!("wrote {}", out.display());
}
