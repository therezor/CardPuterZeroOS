//! The Settings model: pages built from a snapshot of the system
//! (`pages::State`) each time it changes, and drawn with the shared rows.
//! The SDK draws the OS bars; this draws the body and dialogs.

pub mod bt;
pub mod net;
pub mod pages;

use gh0st_app::gfx::Canvas;
use gh0st_app::gfx::geom::Rect;
use gh0st_app::gfx::theme::Theme;
use gh0st_app::gfx::widgets::{self, Row, Trailing};
use gh0st_app::{Chip, Key, Tone};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageId {
    Root,
    Wifi,
    Network,
    Bluetooth,
    Display,
    Sound,
    Appearance,
    Apps,
    /// One app's settings, by `.desktop` file name.
    App(String),
    System,
    /// Regions, then the cities of one region.
    TimeZone(Option<String>),
    /// A saved connection's settings, by profile name.
    Conn(String),
    /// Boot, memory, CPU and identity settings that need the password.
    Advanced,
    /// The zram swap: size, compression and swappiness.
    Swap,
    /// Initials, then the countries under one.
    Country(Option<char>),
    Choose(Choice),
    Privacy,
    /// System > Updates: the last check, daily checks and the channel.
    Updates,
    About,
    /// Setup's device name and look: name, layout, palette, clock.
    Look,
    /// The time zones of the Wi-Fi country, then Other for every region.
    CountryZones,
    /// An access point in place of the Wi-Fi connection.
    Hotspot,
    /// The hotspot's QR code, name and password, for joining it.
    HotspotQr,
    /// Pictures in ~/Pictures, to make one the wallpaper.
    Pictures,
    /// Which app opens each kind of file.
    DefaultApps,
    /// The apps for one kind, by its index in `handlers::CATEGORIES`.
    DefaultFor(usize),
    /// Every interface's addresses, MAC and link, to read.
    NetInfo,
}

/// A value a row changes in place: Enter flips a toggle, Z and C step the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setting {
    Brightness,
    DarkTime,
    Volume,
    Mute,
    Layout,
    Palette,
    /// The picture behind home and the lock.
    Wallpaper,
    Animations,
    /// System sounds on or off.
    Sounds,
    Clock,
    /// Shown on home, by `.desktop` file name.
    AppShown(String),
    AppIcon(String),
    AppBackground(String),
    AppPinned(String),
    WifiRadio,
    Bluetooth,
    NetworkTime,
    ConnAuto,
    ConnMethod,
    ConnMac,
    Ssh,
    SendHostname,
    Ip6Privacy,
    /// What was copied stays through the lock.
    ClipboardKeep,
    /// The lock screen asks for the password.
    LockPassword,
    /// How long dark before the lock asks for it.
    LockAfter,
    /// How long dark before Wi-Fi goes off.
    DarkWifi,
    /// The built-in Ethernet chip may suspend.
    UsbPower,
    /// The daily update check's timer.
    UpdateDaily,
    /// Stable or Edge.
    UpdateChannel,
    /// Wi-Fi power saving, on the Wi-Fi page.
    WifiPowersave,
    Hotspot,
    HotspotSecurity,
    /// DNS for every connection: Automatic, or the servers typed below.
    Dns,
}

/// A value picked on its own page, each option with a note. The gated
/// settings work this way, so the password comes after the pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    GpuMem,
    CpuMode,
    Logs,
    Compression,
    Swappiness,
    Palette,
}

/// Something Enter starts that is not a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Join(String),
    JoinHidden,
    Device(String),
    BtScan,
    SetZone(String),
    /// Type a new value for a connection field.
    ConnEdit(ConnField),
    ConnUp,
    ConnForget,
    Password,
    Restart,
    PowerOff,
    /// Wi-Fi country, by ISO code.
    SetCountry(String),
    /// An option of a choice, in the helper's word: "64", "power-saver".
    Choose(Choice, String),
    /// Kind `0` opens with app `1` (a `.desktop` file name) from now on.
    SetDefault(usize, String),
    /// Kind `0` opens with the built-in pick again.
    ResetDefault(usize),
    DeviceName,
    /// Type the zram swap size in MB.
    SwapSize,
    /// Type the date and time, for a clock network time can't set.
    SetTime,
    /// Brings up the Updates app over Settings.
    OpenUpdates,
    HotspotName,
    HotspotPassword,
    /// Type the DNS servers for every connection.
    DnsServers,
    /// Crop a picture in ~/Pictures, by file name, to the wallpaper.
    SetPicture(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnField {
    Address,
    Gateway,
    Dns,
    /// A fixed MAC address.
    Mac,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Open(PageId),
    Set(Setting),
    Run(Command),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Chevron,
    Text(String),
    TextChevron(String),
    Toggle(bool),
    Choice(String),
    Level(u8),
    Signal {
        level: u8,
        secure: bool,
    },
    /// A row that can't be used now, drawn faint with why.
    Off(&'static str),
}

pub struct Item {
    /// `Some("")` keeps the icon column empty, so marked and unmarked rows
    /// line up.
    pub icon: Option<&'static str>,
    pub label: String,
    pub value: Value,
    pub act: Act,
    /// Drawn in the critical colour: power off.
    pub danger: bool,
}

impl Item {
    pub fn new(icon: &'static str, label: impl Into<String>, value: Value, act: Act) -> Item {
        Item {
            icon: Some(icon),
            label: label.into(),
            value,
            act,
            danger: false,
        }
    }
}

pub enum Line {
    Readout(String, String),
    /// A heading over the readouts under it.
    Head(String),
    /// A value too long to share its line with a label, as an IPv6
    /// address: mono, the whole width.
    Wide(String),
}

pub enum Body {
    List(Vec<Item>),
    /// Readouts with no cursor: Up and Down scroll.
    Info(Vec<Line>),
    /// A QR code and labelled values beside it.
    Qr {
        text: String,
        lines: Vec<(String, String)>,
    },
}

pub struct Page {
    pub title: String,
    pub body: Body,
    /// Extra keys this page takes, shown before Esc.
    pub keys: Vec<(&'static str, &'static str)>,
}

impl Page {
    pub fn len(&self) -> usize {
        match &self.body {
            Body::List(items) => items.len(),
            Body::Info(lines) => lines.len(),
            Body::Qr { .. } => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn item(&self, i: usize) -> Option<&Item> {
        match &self.body {
            Body::List(items) => items.get(i),
            Body::Info(_) | Body::Qr { .. } => None,
        }
    }
}

/// Where the cursor sits on one page of the stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub selected: usize,
    pub top: usize,
}

impl Cursor {
    /// Moves by `d` rows over `len`, keeping `per` rows on screen. An info
    /// page has no cursor, so it scrolls instead.
    pub fn step(&mut self, d: i32, len: usize, per: usize, info: bool) {
        if len == 0 {
            return;
        }
        if info {
            let max = len.saturating_sub(per) as i64;
            self.top = (self.top as i64 + d as i64).clamp(0, max) as usize;
            return;
        }
        self.selected = (self.selected as i64 + d as i64).clamp(0, len as i64 - 1) as usize;
        self.fit(len, per);
    }

    pub fn fit(&mut self, len: usize, per: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + per {
            self.top = self.selected + 1 - per;
        }
        self.top = self.top.min(len.saturating_sub(per));
    }
}

/// A dialog Settings wants, before the SDK draws it.
pub enum Dialog {
    /// One line of text: a password, a network name.
    Text {
        title: String,
        placeholder: &'static str,
        text: String,
        masked: bool,
        /// A password: Tab shows or hides it.
        secret: bool,
    },
    Confirm {
        title: String,
        line: String,
        yes: &'static str,
        /// What Esc says: "cancel", or "later" for a restart.
        no: &'static str,
    },
}

/// Rows of a list page that fit the body.
pub fn rows(t: &Theme) -> (Rect, usize) {
    widgets::rows_area(t, t.m.row_h)
}

/// Lines of an info page that fit the body.
pub fn lines(t: &Theme) -> (Rect, usize) {
    widgets::rows_area(t, t.m.line_h)
}

/// The page body and any dialog. The SDK has cleared the body and draws
/// the bars.
/// `armed`: the row waiting for a second press says so.
pub fn draw(c: &mut Canvas, t: &Theme, page: &Page, cur: Cursor, armed: Option<&Act>) {
    match &page.body {
        Body::List(items) => {
            let (area, n) = rows(t);
            let w = area.w - t.m.scrollbar_w;
            for (slot, i) in (cur.top..items.len()).take(n).enumerate() {
                let it = &items[i];
                let r = Rect::new(area.x, area.y + slot as i32 * t.m.row_h, w, t.m.row_h);
                if let Value::Off(why) = it.value {
                    let icon = it.icon.unwrap_or("");
                    widgets::disabled_row(c, t, r, icon, &it.label, why);
                    continue;
                }
                let mut row = row(t, it);
                if armed == Some(&it.act) {
                    row.trailing = Trailing::Alert("Press again");
                }
                widgets::list_row(c, t, r, &row, i == cur.selected);
            }
            let track = Rect::new(area.x, area.y, w, area.h);
            widgets::scrollbar(c, t, track, cur.top, n, items.len());
            if items.is_empty() {
                let m = widgets::Message {
                    icon: "list",
                    tone: None,
                    text: "Nothing here",
                    note: "",
                    keys: &[],
                };
                widgets::message(c, t, widgets::content(t), &m);
            }
        }
        Body::Info(lines_) => {
            let (area, n) = lines(t);
            let w = area.w - t.m.scrollbar_w;
            for (slot, line) in lines_.iter().skip(cur.top).take(n).enumerate() {
                let r = Rect::new(area.x, area.y + slot as i32 * t.m.line_h, w, t.m.line_h);
                match line {
                    Line::Readout(k, v) => widgets::readout(c, t, r, k, v),
                    Line::Head(h) => widgets::section(c, t, r, h),
                    Line::Wide(v) => {
                        let r = Rect::new(r.x + t.m.pad, r.y, r.w - 2 * t.m.pad, r.h);
                        widgets::text_in(c, t.f.mono, r, v, t.c.fg);
                    }
                }
            }
            let track = Rect::new(area.x, area.y, w, area.h);
            widgets::scrollbar(c, t, track, cur.top, n, lines_.len());
        }
        Body::Qr { text, lines } => {
            let lines: Vec<(&str, &str)> = lines
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            if !widgets::qr_card(c, t, widgets::body(t), text, &lines) {
                let m = widgets::Message {
                    icon: "qr-code",
                    tone: Some(Tone::Warning),
                    text: "Too long for a QR code",
                    note: "",
                    keys: &[],
                };
                widgets::message(c, t, widgets::content(t), &m);
            }
        }
    }
}

fn row<'a>(t: &Theme, it: &'a Item) -> Row<'a> {
    let trailing = match &it.value {
        Value::None | Value::Off(_) => Trailing::None,
        Value::Chevron => Trailing::Chevron,
        Value::Text(s) => Trailing::Value(s),
        Value::TextChevron(s) => Trailing::ValueChevron(s),
        Value::Toggle(on) => Trailing::Toggle(*on),
        Value::Choice(s) => Trailing::Choice(s),
        Value::Level(p) => Trailing::Level(*p),
        Value::Signal { level, secure } => Trailing::Signal {
            level: *level,
            secure: *secure,
        },
    };
    Row {
        icon: it.icon,
        tint: if it.danger {
            t.c.critical
        } else {
            t.c.icon_tint(it.icon.unwrap_or(""))
        },
        label: &it.label,
        trailing,
    }
}

/// The chips over keys 5 to 7, and the key each stands for: the page's own,
/// then the selected row's. Enter and Esc get none. One chip sits in the
/// middle on 6; two go apart on 5 and 7, a pair's back one on the left.
pub fn chips(
    id: &PageId,
    page: &Page,
    cur: Cursor,
    moving: bool,
    armed: bool,
) -> Vec<(u8, Chip, Key)> {
    let mut v = Vec::new();
    // While the hotspot runs, the Wi-Fi page has nothing to scan or forget.
    let hotspot = page
        .item(1)
        .is_some_and(|i| i.act == Act::Open(PageId::Hotspot));
    match id {
        PageId::Wifi if hotspot => {}
        PageId::Wifi | PageId::Bluetooth => {
            v.push((5, Chip::new("refresh-cw", "Scan"), Key::Char('u')));
            let forget = Chip::new("trash-2", "Forget").danger().armed(armed);
            v.push((7, forget, Key::Backspace));
        }
        PageId::Hotspot => {
            v.push((6, Chip::new("refresh-cw", "Reload"), Key::Char('u')));
        }
        PageId::Apps if !moving => {
            v.push((6, Chip::new("arrow-up-down", "Move"), Key::Char('m')));
        }
        _ => {}
    }
    let pair = match page.item(cur.selected).map(|i| &i.value) {
        Some(Value::Choice(_)) => Some((("chevron-left", "Prev"), ("chevron-right", "Next"))),
        Some(Value::Level(_)) => Some((("minus", "Less"), ("plus", "More"))),
        _ => None,
    };
    if let Some(((i1, w1), (i2, w2))) = pair {
        v.clear();
        v.push((5, Chip::new(i1, w1), Key::Left));
        v.push((7, Chip::new(i2, w2), Key::Right));
    }
    v
}

/// Each page's icon in the top bar.
pub fn icon(id: &PageId) -> &'static str {
    match id {
        PageId::Wifi | PageId::Conn(_) => "wifi",
        PageId::Network => "network",
        PageId::Bluetooth => "bluetooth",
        PageId::Display => "sun",
        PageId::Sound => "volume-2",
        PageId::Appearance => "palette",
        PageId::Pictures => "images",
        PageId::Apps | PageId::App(_) | PageId::DefaultApps | PageId::DefaultFor(_) => {
            "layout-grid"
        }
        PageId::System | PageId::Advanced | PageId::Swap => "cpu",
        PageId::TimeZone(_) | PageId::CountryZones => "clock",
        PageId::Country(_) => "globe",
        PageId::Privacy => "shield",
        PageId::Updates => "download",
        PageId::Hotspot | PageId::HotspotQr => "radio-tower",
        PageId::Look => "palette",
        PageId::About => "info",
        _ => "settings",
    }
}

/// Settings' dialog as the SDK draws it.
pub fn sdk_dialog(d: &Dialog) -> gh0st_app::Dialog {
    match d {
        Dialog::Text {
            title,
            placeholder,
            text,
            secret: true,
            ..
        } => {
            let d = gh0st_app::Dialog::password(title)
                .placeholder(placeholder)
                .value(text);
            // A network asks to be joined; the rest just take a password.
            if title.starts_with("Join") {
                d.yes("join")
            } else {
                d
            }
        }
        Dialog::Text {
            title,
            placeholder,
            text,
            ..
        } => gh0st_app::Dialog::text(title, placeholder).value(text),
        Dialog::Confirm {
            title,
            line,
            yes,
            no,
        } => {
            let d = gh0st_app::Dialog::confirm(title, line).yes(yes).no(no);
            if *yes == "forget" { d.danger() } else { d }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_keeps_its_row_on_screen() {
        let mut c = Cursor::default();
        c.step(6, 10, 5, false);
        assert_eq!((c.selected, c.top), (6, 2));
        c.step(-9, 10, 5, false);
        assert_eq!((c.selected, c.top), (0, 0));
    }

    #[test]
    fn an_info_page_scrolls_without_a_cursor() {
        let mut c = Cursor::default();
        c.step(3, 12, 8, true);
        assert_eq!(c.top, 3);
        c.step(9, 12, 8, true);
        assert_eq!(c.top, 4, "never past the last screen");
    }
}
