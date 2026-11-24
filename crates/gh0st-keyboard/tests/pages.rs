//! The app's screens on fake links: what reaches the host, the Esc rules,
//! keys let go whenever the page loses them, and no wakes while idle.

use std::path::PathBuf;

use gh0st_app::testing::Driver;
use gh0st_keyboard::app::{App, Open, Target, address_of};
use gh0st_keyboard::hid::report::Report;
use gh0st_keyboard::hid::usage::{key, mods};
use gh0st_keyboard::host::System;
use gh0st_keyboard::link::fake::{FakeBt, FakeCable, SharedLog};
use gh0st_keyboard::link::{Ask, Event, UsbState};
use gh0st_keyboard::pages::session::{MOUSE, SNIPPETS, TYPE};
use gh0st_keyboard::store::{self, Host, Snippet};

const MAC: &str = "/org/bluez/hci0/dev_F0_18_98_00_00_01";
const PC: &str = "/org/bluez/hci0/dev_00_1A_7D_00_00_03";

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-kb-pages-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mac = Host::new(&address_of(MAC).unwrap(), "MacBook", System::Mac);
    let pc = Host::new(&address_of(PC).unwrap(), "Office PC", System::Windows);
    store::save_hosts(&d, &[mac, pc]).unwrap();
    store::save_snippets(
        &d,
        &[Snippet {
            name: "Hi".into(),
            text: "hi".into(),
        }],
    )
    .unwrap();
    d
}

/// The app typing on `host`, connected, in `mode`.
fn session(name: &str, host: &str, mode: usize) -> (Driver, SharedLog) {
    let bt = FakeBt::new().host(host, "Host", "computer", true);
    let log = bt.log.clone();
    let app = App::with(
        Box::new(bt),
        Box::new(FakeCable::new(UsbState::Off)),
        dir(name),
    )
    .opening(
        Some(Target::Bt(address_of(host).unwrap())),
        Open::Session(mode),
    );
    let mut d = Driver::new(app);
    d.wait(50);
    (d, log)
}

fn keyboards(log: &SharedLog) -> Vec<[u8; 8]> {
    log.borrow()
        .sent
        .iter()
        .filter_map(|r| match r {
            Report::Keyboard(k) => Some(*k),
            _ => None,
        })
        .collect()
}

fn pressed(log: &SharedLog) -> Vec<(u8, u8)> {
    keyboards(log)
        .into_iter()
        .filter(|k| k[2] != 0)
        .map(|k| (k[0], k[2]))
        .collect()
}

#[test]
fn typing_reaches_the_host() {
    let (mut d, log) = session("typing", PC, TYPE);
    d.keys("type hi");
    assert_eq!(pressed(&log), [(0, 0x0b), (0, 0x0c)]);
    assert_eq!(keyboards(&log).last(), Some(&[0; 8]), "each key let go");
}

#[test]
fn alt_is_command_on_a_mac() {
    let (mut d, log) = session("mac", MAC, TYPE);
    d.keys("alt+c");
    assert_eq!(pressed(&log), [(mods::LGUI, key::C)]);
}

#[test]
fn fn_esc_sends_esc_and_esc_goes_back() {
    let (mut d, log) = session("esc", PC, TYPE);
    d.keys("fn+esc");
    assert_eq!(pressed(&log), [(0, key::ESC)]);
    d.keys("esc");
    log.borrow_mut().sent.clear();
    // Back on the hosts: keys no longer reach the host.
    d.keys("type x");
    assert!(pressed(&log).is_empty());
    assert!(!d.quit());
}

#[test]
fn help_lets_go_of_held_keys() {
    let (mut d, log) = session("help", PC, TYPE);
    d.keys("down shift\ndown a");
    assert_eq!(keyboards(&log).last().map(|k| k[2]), Some(key::A));
    d.keys("help");
    assert_eq!(keyboards(&log).last(), Some(&[0; 8]));
}

#[test]
fn going_behind_lets_go_of_held_keys() {
    let (mut d, log) = session("pause", PC, TYPE);
    d.keys("down a");
    d.pause();
    assert_eq!(keyboards(&log).last(), Some(&[0; 8]));
}

#[test]
fn a_phone_is_let_go_when_the_app_goes_behind() {
    let phone = "/org/bluez/hci0/dev_3C_28_6D_00_00_02";
    let d = dir("phone");
    store::save_hosts(
        &d,
        &[Host::new(
            &address_of(phone).unwrap(),
            "Pixel",
            System::Android,
        )],
    )
    .unwrap();
    let bt = FakeBt::new().host(phone, "Pixel", "phone", true);
    let log = bt.log.clone();
    let app = App::with(Box::new(bt), Box::new(FakeCable::new(UsbState::Off)), d).opening(
        Some(Target::Bt(address_of(phone).unwrap())),
        Open::Session(TYPE),
    );
    let mut d = Driver::new(app);
    d.wait(50).pause();
    assert_eq!(log.borrow().disconnects, [phone]);
}

#[test]
fn typing_wakes_nothing_while_idle() {
    let (mut d, _) = session("idle", PC, TYPE);
    d.keys("type hello");
    let wakes = d.idle_wakes(120_000);
    assert!(wakes <= 3, "the clock's minute, nothing else: {wakes}");
}

#[test]
fn the_mouse_moves_only_while_held() {
    let (mut d, log) = session("mouse", PC, MOUSE);
    d.keys("down c").wait(200).keys("up c");
    let moves: Vec<i8> = log
        .borrow()
        .sent
        .iter()
        .filter_map(|r| match r {
            Report::Mouse(m) => Some(m.dx),
            _ => None,
        })
        .collect();
    assert!(
        moves.len() >= 5 && moves.iter().all(|&dx| dx > 0),
        "{moves:?}"
    );
    // And faster the longer it's held.
    assert!(moves.last() > moves.first());
    let wakes = d.idle_wakes(120_000);
    assert!(wakes <= 3, "the timer stopped with the key: {wakes}");
}

#[test]
fn a_macro_types_its_text_at_a_pace() {
    let (mut d, log) = session("macro", PC, SNIPPETS);
    // The Windows shortcuts come first, then "Hi".
    let cuts = gh0st_keyboard::shortcuts::of(System::Windows).len();
    for _ in 0..cuts {
        d.keys("x");
    }
    d.keys("enter").wait(200);
    assert_eq!(pressed(&log), [(0, 0x0b), (0, 0x0c)]);
    assert_eq!(keyboards(&log).len(), 4, "a press and a release for each");
    let wakes = d.idle_wakes(120_000);
    assert!(wakes <= 3, "the pace timer stopped: {wakes}");
}

#[test]
fn a_shortcut_is_one_combination() {
    let (mut d, log) = session("cut", PC, SNIPPETS);
    d.keys("enter");
    assert_eq!(pressed(&log), [(mods::LCTRL, key::C)]);
}

#[test]
fn pairing_says_yes_only_on_enter() {
    let mut bt = FakeBt::new();
    bt.asking = Some(Ask::Confirm(123_456));
    bt.queue.push(Event::Pairing {
        device: MAC.into(),
        ask: Ask::Confirm(123_456),
    });
    let log = bt.log.clone();
    let app = App::with(
        Box::new(bt),
        Box::new(FakeCable::new(UsbState::Off)),
        dir("pair"),
    )
    .opening(None, Open::Pair);
    let mut d = Driver::new(app);
    d.wait(50);
    assert!(log.borrow().answers.is_empty());
    d.keys("enter");
    assert_eq!(log.borrow().answers, [true]);
}

#[test]
fn a_phone_that_paired_on_a_private_address_is_followed() {
    // An iPhone pairs from a private address; BlueZ then gives the device
    // its own address, on the same object.
    let private = "58:68:08:74:3B:84";
    let path = "/org/bluez/hci0/dev_4F_A2_48_D0_4F_AD";
    let d = dir("moved");
    store::save_hosts(&d, &[Host::new(private, "iPhone", System::Ios)]).unwrap();
    let mut bt = FakeBt::new().host(path, "iPhone", "phone", true);
    bt.devices[0].address = "F8:2A:E2:48:8E:99".into();
    bt.queue
        .push(Event::Moved(private.into(), "F8:2A:E2:48:8E:99".into()));
    let log = bt.log.clone();
    let app = App::with(
        Box::new(bt),
        Box::new(FakeCable::new(UsbState::Off)),
        d.clone(),
    )
    .opening(Some(Target::Bt(private.into())), Open::Session(TYPE));
    let mut drv = Driver::new(app);
    drv.wait(50).keys("type a");
    assert_eq!(
        pressed(&log),
        [(0, key::A)],
        "typing reaches the moved host"
    );
    assert_eq!(
        store::hosts(&d)[0].address,
        "F8:2A:E2:48:8E:99",
        "saved under its own address"
    );
}

#[test]
fn bksp_is_the_right_button_and_p_the_middle() {
    let (mut d, log) = session("buttons", PC, MOUSE);
    d.keys("backspace\np");
    let buttons: Vec<u8> = log
        .borrow()
        .sent
        .iter()
        .filter_map(|r| match r {
            Report::Mouse(m) => Some(m.buttons),
            _ => None,
        })
        .collect();
    assert_eq!(buttons, [2, 0, 4, 0]);
}
