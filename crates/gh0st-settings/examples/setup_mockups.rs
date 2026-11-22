//! First-boot setup (docs/setup.md), drawn on the design system
//! (docs/design.md) with the real widgets: every step and lesson at panel
//! size and 3x, and a contact sheet per palette, into mockups/setup. Run from
//! the workspace root:
//!
//!     cargo run -p gh0st-settings --example setup_mockups

use gh0st_app::Page;
use gh0st_app::gfx::font::Font;
use gh0st_app::gfx::icon::{self, IconSize};
use gh0st_app::gfx::snapshot::write_png;
use gh0st_app::gfx::theme::{self, Theme};
use gh0st_app::gfx::widgets::{
    self, Battery, Chip, Field, Hint, Look, Net, Row, Status, Trailing, soft, text_in,
};
use gh0st_app::gfx::{Canvas, Rect};
use gh0st_app::testing::Driver;
use gh0st_core::sysconf;
use gh0st_settings::model::pages::{self, Network, State};
use gh0st_settings::model::{self, Cursor, Dialog, PageId};
use gh0st_settings::setup::{self, tour};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The settings part's steps.
const STEPS: usize = 8;

fn setup(n: usize) -> String {
    format!("Setup {n} of {STEPS}")
}

// ---------------------------------------------------------------- bars

/// The top bar: the step's name, then its count in its part. No network
/// until the user joins one.
fn top(c: &mut Canvas, t: &Theme, icon: &str, title: &str, context: &str, net: Net) {
    let s = Status {
        dock: &[],
        front: None,
        icon,
        title,
        context,
        tabs: &[],
        front_tab: 0,
        item: None,
        clock: "14:32",
        battery: Some(Battery {
            percent: 87,
            charging: false,
        }),
        net,
        bluetooth: false,
        notices: 0,
    };
    widgets::status_bar(c, t, &s);
}

const NONE: Chip<'static> = Chip {
    icon: "",
    word: "",
    look: Look::Normal,
};

fn n<'a>(icon: &'a str, word: &'a str) -> Chip<'a> {
    Chip {
        icon,
        word,
        look: Look::Normal,
    }
}

fn l<'a>(icon: &'a str, word: &'a str, look: Look) -> Chip<'a> {
    Chip { icon, word, look }
}

const MENU: Chip<'static> = Chip {
    icon: "menu",
    word: "",
    look: Look::Normal,
};

fn skip() -> Chip<'static> {
    n("skip-forward", "Skip")
}

/// The main chip where Enter works the rows.
fn done() -> Chip<'static> {
    l("check", "Done", Look::Main)
}

/// Chips over 4 to 8; 4 stays empty (it is Esc) and 8 is the menu.
fn chips(c: &mut Canvas, t: &Theme, five: Chip, six: Chip, seven: Chip) {
    widgets::action_bar(c, t, &[NONE, five, six, seven, MENU]);
}

const HELP: Hint<'static> = Hint {
    key: "Fn h",
    label: "help",
};

fn hint_bar(c: &mut Canvas, t: &Theme, h: &[(&str, &str)]) {
    let hints: Vec<Hint> = h.iter().map(|&(key, label)| Hint { key, label }).collect();
    widgets::hint_bar(c, t, &hints, Some(HELP));
}

// ---------------------------------------------------------------- content

fn frame(t: &Theme) -> Canvas {
    let mut c = Canvas::new(t.m.width, t.m.height);
    c.fill(c.bounds(), t.c.bg);
    c
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

/// Wrapped text from `y`; returns the y under it.
fn para(c: &mut Canvas, t: &Theme, y: i32, s: &str, f: Font, lh: i32) -> i32 {
    let r = widgets::content(t);
    let mut y = y;
    for line in wrap(f, s, r.w - 6) {
        text_in(c, f, Rect::new(r.x + 3, y, r.w - 6, lh), &line, soft(t));
        y += lh;
    }
    y
}

fn row<'a>(t: &Theme, icon: &'a str, label: &'a str, trailing: Trailing<'a>) -> Row<'a> {
    Row {
        icon: (!icon.is_empty()).then_some(icon),
        tint: t.c.accent,
        label,
        trailing,
    }
}

/// List rows from `y`, the selected one lit.
fn rows(c: &mut Canvas, t: &Theme, y: i32, items: &[Row], selected: usize) {
    let r = widgets::content(t);
    let h = t.m.row_h;
    let fit = ((t.m.height - t.m.hint_h - y) / h) as usize;
    for (i, it) in items.iter().take(fit).enumerate() {
        let rr = Rect::new(
            t.m.margin,
            y + i as i32 * h,
            r.w + 2 * (r.x - t.m.margin),
            h,
        );
        widgets::list_row(c, t, rr, it, i == selected);
    }
    if items.len() > fit {
        let area = Rect::new(t.m.margin, y, r.w, fit as i32 * h);
        widgets::scrollbar(c, t, area, 0, fit, items.len());
    }
}

fn list_top(t: &Theme) -> i32 {
    widgets::rows_area(t, t.m.row_h).0.y
}

/// A form field from `y`, as `Ui::form` draws one; returns the y where the
/// next one goes.
fn field(c: &mut Canvas, t: &Theme, y: i32, f: Field) -> i32 {
    let r = widgets::content(t);
    y + widgets::form_field(c, t, Rect::new(r.x, y, r.w, 0), &f) + t.m.field_gap
}

fn plain<'a>(label: &'a str, text: &'a str, focused: bool) -> Field<'a> {
    Field {
        label,
        text,
        placeholder: "",
        masked: false,
        focused,
        error: None,
    }
}

fn secret<'a>(label: &'a str, text: &'a str, error: Option<&'a str>) -> Field<'a> {
    Field {
        masked: true,
        error,
        ..plain(label, text, true)
    }
}

// ---------------------------------------------------------------- steps

// The notice, the welcome and the training are the real pages, drawn by
// the SDK as on the device: they can't drift from the code.

fn real(page: impl Page + 'static, keys: &str, hold_ms: u64, t: &Theme) -> Canvas {
    let mut d = Driver::new(page);
    d.palette(t.c.name);
    if !keys.is_empty() {
        d.keys(keys);
    }
    if hold_ms > 0 {
        d.wait(hold_ms);
    }
    d.frame().clone()
}

fn answers() -> setup::Shared {
    Rc::new(RefCell::new(setup::Answers::default()))
}

fn lesson(n: usize, keys: &str, t: &Theme) -> Canvas {
    let label = format!("Training {} of {}", n + 1, tour::LESSONS);
    real(tour::Lesson::new(n, &label, answers()), keys, 0, t)
}

// Settings' own pages, which setup reuses.

fn sample_state() -> State {
    let net = |ssid: &str, bars, secure| Network {
        ssid: ssid.into(),
        bars,
        secure,
        known: false,
        active: false,
        enterprise: false,
    };
    State {
        wifi_on: true,
        networks: vec![
            net("HomeNet", 3, true),
            net("Office-5G", 4, true),
            net("Cafe Guest", 2, false),
            net("Neighbours", 1, true),
        ],
        countries: sysconf::countries(
            "AE\tUnited Arab Emirates\nDE\tGermany\nGB\tBritain (UK)\nUA\tUkraine\n\
             UG\tUganda\nUS\tUnited States\nUY\tUruguay\nUZ\tUzbekistan\nAD\tAndorra\n",
        ),
        ..State::default()
    }
}

fn settings_page(c: &mut Canvas, t: &Theme, id: PageId, selected: usize) {
    let page = pages::build(&id, &sample_state());
    let mut cur = Cursor { selected, top: 0 };
    cur.fit(page.len(), model::rows(t).1);
    model::draw(c, t, &page, cur);
}

fn country(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "globe", "Country", &setup(1), Net::None);
    settings_page(&mut c, t, PageId::Country(Some('U')), 1);
    chips(&mut c, t, NONE, NONE, NONE);
    c
}

fn time_zone(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "clock", "Time zone", &setup(2), Net::None);
    let items = [
        row(t, "", "New York", Trailing::Value("UTC\u{2212}5")),
        row(t, "", "Chicago", Trailing::Value("UTC\u{2212}6")),
        row(t, "", "Denver", Trailing::Value("UTC\u{2212}7")),
        row(t, "", "Los Angeles", Trailing::Value("UTC\u{2212}8")),
        row(t, "", "Anchorage", Trailing::Value("UTC\u{2212}9")),
        row(t, "", "Other\u{2026}", Trailing::Chevron),
    ];
    rows(&mut c, t, list_top(t), &items, 0);
    chips(&mut c, t, NONE, NONE, NONE);
    c
}

fn privacy(t: &Theme, selected: usize) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "shield", "Privacy", &setup(3), Net::None);
    let items = [
        row(t, "wifi", "Private Wi-Fi address", Trailing::Toggle(true)),
        row(
            t,
            "search",
            "Random address in scans",
            Trailing::Toggle(true),
        ),
        row(t, "tag", "Send device name", Trailing::Toggle(false)),
        row(t, "globe", "IPv6 privacy address", Trailing::Toggle(true)),
        row(t, "clock", "Network time", Trailing::Toggle(true)),
        row(t, "hard-drive", "Keep logs", Trailing::ValueChevron("Off")),
        row(t, "square-terminal", "SSH", Trailing::Toggle(false)),
        row(t, "bluetooth", "Bluetooth", Trailing::Toggle(false)),
    ];
    let first = selected.saturating_sub(4);
    rows(&mut c, t, list_top(t), &items[first..], selected - first);
    if matches!(items[selected].trailing, Trailing::Choice(_)) {
        chips(
            &mut c,
            t,
            n("chevron-left", "Prev"),
            done(),
            n("chevron-right", "Next"),
        );
    } else {
        chips(&mut c, t, NONE, done(), NONE);
    }
    c
}

fn network(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "wifi", "Network", &setup(4), Net::None);
    settings_page(&mut c, t, PageId::Wifi, 1);
    chips(&mut c, t, n("refresh-cw", "Scan"), NONE, skip());
    c
}

fn network_cable(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "wifi", "Network", &setup(4), Net::None);
    let sig = |level| Trailing::Signal {
        level,
        secure: true,
    };
    let items = [
        row(
            t,
            "ethernet-port",
            "Use the cable",
            Trailing::Value("plugged in"),
        ),
        row(t, "wifi", "HomeNet", sig(3)),
        row(t, "wifi", "Office-5G", sig(4)),
        row(t, "wifi", "Neighbours", sig(1)),
    ];
    rows(&mut c, t, list_top(t), &items, 0);
    chips(&mut c, t, n("refresh-cw", "Scan"), NONE, skip());
    c
}

fn wifi_password(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "wifi", "Network", &setup(4), Net::None);
    settings_page(&mut c, t, PageId::Wifi, 1);
    let d = model::sdk_dialog(&Dialog::Text {
        title: "Join \u{201c}Office-5G\u{201d}".into(),
        placeholder: "Password",
        text: "hunter22".into(),
        masked: true,
        secret: true,
    });
    d.draw(&mut c, t, widgets::body(t));
    let hints: Vec<Hint> = d
        .hints()
        .into_iter()
        .map(|(key, label)| Hint { key, label })
        .collect();
    widgets::hint_bar(&mut c, t, &hints, Some(HELP));
    c
}

fn date_time(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "clock", "Date and time", &setup(5), Net::None);
    let r = widgets::content(t);
    let s = "No network time yet. Check these.";
    let y = para(&mut c, t, r.y, s, t.f.small, 15);
    let y = field(&mut c, t, y + 2, plain("Date", "2026-11-21", false));
    field(&mut c, t, y, plain("Time, 24 hour", "14:32", true));
    hint_bar(
        &mut c,
        t,
        &[("Esc", "back"), ("Tab", "next field"), ("Enter", "set")],
    );
    c
}

fn account(t: &Theme, error: bool) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "user", "Account", &setup(6), Net::Wifi(3));
    let r = widgets::content(t);
    let y = field(
        &mut c,
        t,
        r.y,
        plain("Your name on this device", "alex", false),
    );
    let (pw, why) = match error {
        true => ("abc", Some("Too short: 4 or more")),
        false => ("hunter22", None),
    };
    let y = field(&mut c, t, y, secret("Password, 4 or more", pw, why));
    if !error {
        let s = "The name is for SSH and the terminal: a to z, digits, - and _.";
        para(&mut c, t, y, s, t.f.small, 15);
    }
    hint_bar(
        &mut c,
        t,
        &[("Esc", "back"), ("Tab", "next field"), ("Enter", "next")],
    );
    c
}

fn account_again(t: &Theme, differ: bool) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "user", "Account", &setup(6), Net::Wifi(3));
    let r = widgets::content(t);
    let (text, why) = match differ {
        true => ("hunter2", Some("They differ. Type it again.")),
        false => ("hunt", None),
    };
    let y = field(&mut c, t, r.y, secret("The password again", text, why));
    if !differ {
        let s = "You will need it for the lock screen, SSH and changes in Settings.";
        para(&mut c, t, y, s, t.f.small, 15);
    }
    hint_bar(
        &mut c,
        t,
        &[("Esc", "back"), ("Tab", "show"), ("Enter", "next")],
    );
    c
}

fn look(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(
        &mut c,
        t,
        "palette",
        "Device and look",
        &setup(7),
        Net::Wifi(3),
    );
    let items = [
        row(t, "tag", "Device name", Trailing::ValueChevron("zero")),
        row(t, "layout-grid", "Home layout", Trailing::Choice("Grid")),
        row(t, "palette", "Palette", Trailing::Choice(t.c.name)),
        row(t, "clock", "24-hour clock", Trailing::Toggle(true)),
    ];
    rows(&mut c, t, list_top(t), &items, 2);
    chips(
        &mut c,
        t,
        n("chevron-left", "Prev"),
        done(),
        n("chevron-right", "Next"),
    );
    c
}

fn finished(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "circle-check", "Ready", &setup(8), Net::Wifi(3));
    let r = widgets::content(t);
    let lines = [
        ("Country", "Britain (UK)"),
        ("Time zone", "Europe/London"),
        ("Network", "HomeNet"),
        ("Account", "alex"),
    ];
    let h = 19;
    for (i, (k, v)) in lines.iter().enumerate() {
        let rr = Rect::new(r.x, r.y + i as i32 * h, r.w, h);
        widgets::readout(&mut c, t, rr, k, v);
    }
    let keys = [Hint {
        key: "Enter",
        label: "start",
    }];
    let y = r.y + 4 * h + 10;
    widgets::key_row(&mut c, t, r.x + 3, y, &keys);
    chips(&mut c, t, NONE, NONE, NONE);
    c
}

fn starting(t: &Theme) -> Canvas {
    let mut c = frame(t);
    let s = IconSize::Large as i32;
    let b = c.bounds();
    let ink = t.c.accent;
    icon::draw(
        &mut c,
        widgets::LOGO,
        IconSize::Large,
        (b.w - s) / 2,
        50,
        ink,
    );
    let tr = Rect::new(0, 50 + s + 12, b.w, 18);
    widgets::text_centre(&mut c, t.f.body, tr, "Starting\u{2026}", t.c.fg);
    c
}

/// Fn h on the network step: the chips, then the keys.
fn help(t: &Theme) -> Canvas {
    let mut c = frame(t);
    top(&mut c, t, "keyboard", "Help", &setup(4), Net::None);
    let (x, w, kw) = (6, t.m.width - 12, 62);
    let mut y = 19;
    let lines: [(&str, &str); 7] = [
        ("#", "Under the screen"),
        ("5 r", "Scan"),
        ("7", "Skip"),
        ("8", "Menu"),
        ("#", "Keys"),
        ("Enter", "Join"),
        ("Esc", "Back a step"),
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
    widgets::action_bar(&mut c, t, &[NONE; 5]);
    c
}

// ---------------------------------------------------------------- output

fn blit(dst: &mut Canvas, src: &Canvas, ox: i32, oy: i32) {
    for y in 0..src.height() {
        for x in 0..src.width() {
            dst.set(ox + x, oy + y, src.get(x, y));
        }
    }
}

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
        let lr = Rect::new(ox, oy, w, cap - 3);
        text_in(&mut s, t.f.small_bold, lr, &label, t.c.fg);
        blit(&mut s, c, ox, oy + cap);
        let edge = Rect::new(ox - 1, oy + cap - 1, w + 2, h + 2);
        s.stroke_round_rect(edge, 1, t.c.border);
    }
    s
}

fn write(dir: &Path, name: &str, c: &Canvas) {
    write_png(c, &dir.join(format!("{name}.png")), 1).unwrap();
    write_png(c, &dir.join(format!("{name}@3x.png")), 3).unwrap();
}

type Screen = (&'static str, &'static str, fn(&Theme) -> Canvas);

fn main() {
    let out = PathBuf::from("mockups/setup");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&out).unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let all: &[Screen] = &[
        ("00-notice", "pre-alpha, first of all", |t| {
            real(tour::Notice, "", 0, t)
        }),
        ("01-welcome", "Enter trains, 6 skips to settings", |t| {
            real(tour::Welcome::new(answers()), "", 0, t)
        }),
        ("02-opsec", "swear in: type opsec", |t| {
            lesson(0, "type ops", t)
        }),
        ("03-sworn", "passed", |t| lesson(0, "type opsec", t)),
        ("04-caps", "now mean it: caps lock", |t| {
            lesson(1, "type OPS", t)
        }),
        ("05-typo", "a wrong letter", |t| lesson(1, "type OPX", t)),
        ("06-sobriety", "walk the dot to the ring", |t| {
            lesson(2, "c", t)
        }),
        ("07-sober", "three moves: sober", |t| {
            lesson(2, "c\nc\nx\nenter", t)
        }),
        ("08-email", "a fake email, Sym for @", |t| {
            lesson(3, "type no", t)
        }),
        ("09-dont", "the chip on 7 says Don't", |t| lesson(4, "", t)),
        ("10-pressed", "everyone presses it", |t| lesson(4, "7", t)),
        ("11-hat", "a switch and a choice", |t| lesson(5, "", t)),
        ("12-unreadable", "hat on, paranoia max", |t| {
            lesson(5, "7\nf\nenter", t)
        }),
        ("13-menu", "secret menu on 8", |t| lesson(6, "", t)),
        ("14-menu-open", "setup's menu, no Close app", |t| {
            lesson(6, "8", t)
        }),
        ("15-boom", "hold 7 to self-destruct", |t| {
            let label = format!("Training 8 of {}", tour::LESSONS);
            real(tour::Lesson::new(7, &label, answers()), "down 7", 400, t)
        }),
        ("16-graduation", "the reward: Phantom", |t| lesson(8, "", t)),
        ("17-country", "a letter jumps", country),
        ("18-time-zone", "Enter picks", time_zone),
        ("19-privacy", "Enter flips, 6 Done goes on", |t| {
            privacy(t, 0)
        }),
        ("20-privacy-logs", "Keep logs opens its list", |t| {
            privacy(t, 5)
        }),
        ("21-network", "Enter joins, 5 Scan, 7 Skip", network),
        ("22-network-cable", "with a cable in", network_cable),
        (
            "23-wifi-password",
            "the join dialog: key hints",
            wifi_password,
        ),
        (
            "24-date-time",
            "only when the clock has not synced",
            date_time,
        ),
        ("25-account", "name and password, Tab between", |t| {
            account(t, false)
        }),
        ("26-account-short", "an error under the field", |t| {
            account(t, true)
        }),
        ("27-again", "once more, Tab shows it", |t| {
            account_again(t, false)
        }),
        ("28-again-differ", "they differ", |t| account_again(t, true)),
        ("29-look", "5 and 7 change, 6 Done", look),
        ("30-done", "what was set", finished),
        ("31-starting", "the shell comes up", starting),
        ("32-help", "Fn h on a step", help),
    ];
    for palette in theme::PALETTES.iter().chain(theme::EARNED) {
        let t = Theme::new(*palette);
        let tag = palette.name.to_lowercase();
        let screens: Vec<(&str, &str, Canvas)> = all
            .iter()
            .map(|&(name, cap, draw)| (name, cap, draw(&t)))
            .collect();
        if palette.name == "Mint" {
            for (name, _, c) in &screens {
                write(&out, name, c);
            }
        }
        let s = sheet(&t, &screens);
        write_png(&s, &out.join(format!("{tag}-sheet.png")), 1).unwrap();
        write_png(&s, &out.join(format!("{tag}-sheet@2x.png")), 2).unwrap();
    }
    println!("wrote {}", out.display());
}
