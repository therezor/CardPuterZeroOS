//! The Bluetooth link against a fake BlueZ: registration, the advert,
//! pairing, the host listening, reports and the keyboard lights.

mod common;

use common::fakebluez::{self, ADAPTER, PHONE, pump};
use gh0st_dbus::Value;
use gh0st_keyboard::hid::report::Report;
use gh0st_keyboard::link::ble::Advert;
use gh0st_keyboard::link::gatt::{self, CHAR};
use gh0st_keyboard::link::{Ask, Event};

const AGENT: &str = "org.bluez.Agent1";

fn ready() -> (gh0st_keyboard::link::ble::Ble, fakebluez::FakeBluez) {
    let (mut ble, mut fake) = fakebluez::start(true);
    ble.set_hosts(vec![PHONE.into()]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Ready), "{events:?}");
    (ble, fake)
}

#[test]
fn registers_the_agent_then_the_services() {
    let (ble, fake) = ready();
    assert_eq!(fake.calls("AddMatch").len(), 3);
    let order: Vec<&str> = fake
        .log
        .iter()
        .map(|l| l.split(' ').next().unwrap())
        .filter(|m| *m != "AddMatch")
        .collect();
    assert_eq!(
        order,
        [
            "GetManagedObjects",
            "RegisterAgent",
            "RequestDefaultAgent",
            "RegisterApplication"
        ]
    );
    assert!(ble.powered());
    assert_eq!(ble.name(), "zero");
    assert_eq!(ble.devices().len(), 1);
    // BlueZ fetched the HID service and its report map from our tree.
    let app = fake.app.as_ref().unwrap();
    let hid = app
        .get(gatt::HID)
        .unwrap()
        .get("org.bluez.GattService1")
        .unwrap();
    assert_eq!(
        hid.get("UUID").map(Value::inner),
        Some(&Value::str(gatt::uuid(0x1812)))
    );
    let map = app.get(gatt::MAP).unwrap().get(CHAR).unwrap();
    assert!(map.get("Value").unwrap().inner().as_bytes().unwrap().len() > 100);
}

#[test]
fn no_advert_until_asked() {
    let (mut ble, mut fake) = ready();
    assert!(fake.calls("RegisterAdvertisement").is_empty());
    ble.pair(true);
    pump(&mut ble, &mut fake);
    assert_eq!(fake.calls("RegisterAdvertisement"), [ADAPTER]);
    let adv = fake.advert.as_ref().unwrap();
    assert_eq!(
        adv.get("Discoverable").map(Value::inner),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        adv.get("LocalName").map(Value::inner),
        Some(&Value::str("zero"))
    );
    assert_eq!(
        adv.get("Appearance").map(Value::inner),
        Some(&Value::U16(0x03c1))
    );
    // Leaving pairing takes the advert down and the adapter's pairable off.
    ble.pair(false);
    pump(&mut ble, &mut fake);
    assert_eq!(fake.calls("UnregisterAdvertisement"), [ADAPTER]);
    assert_eq!(ble.wanted(), Advert::Off);
}

#[test]
fn a_reconnect_advert_has_no_name() {
    let (mut ble, mut fake) = ready();
    ble.reconnect(true);
    pump(&mut ble, &mut fake);
    let adv = fake.advert.as_ref().unwrap();
    assert_eq!(
        adv.get("Discoverable").map(Value::inner),
        Some(&Value::Bool(false))
    );
    assert!(adv.get("LocalName").is_none());
}

#[test]
fn an_unpowered_adapter_advertises_once_on() {
    let (mut ble, mut fake) = fakebluez::start(false);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Power(false)));
    ble.reconnect(true);
    pump(&mut ble, &mut fake);
    assert!(fake.calls("RegisterAdvertisement").is_empty());
    ble.power(true);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Power(true)));
    assert_eq!(fake.calls("RegisterAdvertisement").len(), 1);
}

#[test]
fn pairing_waits_for_the_user() {
    let (mut ble, mut fake) = ready();
    ble.pair(true);
    pump(&mut ble, &mut fake);
    let q = fake.call(
        gatt::AGENT,
        AGENT,
        "RequestConfirmation",
        vec![Value::path(PHONE), Value::U32(123_456)],
    );
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Pairing {
        device: PHONE.into(),
        ask: Ask::Confirm(123_456)
    }));
    assert!(fake.reply_to(q).is_none(), "waits for Enter");
    ble.answer(true);
    pump(&mut ble, &mut fake);
    assert_eq!(fake.reply_to(q).unwrap().error_name, None);
    // The bond shows as Paired.
    fake.device(PHONE, vec![("Paired", Value::Bool(true))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Paired(PHONE.into())) || ble.devices()[0].paired);
}

#[test]
fn pairing_off_the_pair_page_is_refused() {
    let (mut ble, mut fake) = ready();
    let q = fake.call(
        gatt::AGENT,
        AGENT,
        "RequestConfirmation",
        vec![Value::path(PHONE), Value::U32(1)],
    );
    let events = pump(&mut ble, &mut fake);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(
        fake.reply_to(q).unwrap().error_name.as_deref(),
        Some("org.bluez.Error.Rejected")
    );
}

#[test]
fn a_passkey_typed_here_goes_back() {
    let (mut ble, mut fake) = ready();
    ble.pair(true);
    let q = fake.call(
        gatt::AGENT,
        AGENT,
        "RequestPasskey",
        vec![Value::path(PHONE)],
    );
    pump(&mut ble, &mut fake);
    assert_eq!(ble.asking(), Some(Ask::Enter));
    ble.passkey(42);
    pump(&mut ble, &mut fake);
    assert_eq!(fake.reply_to(q).unwrap().body, [Value::U32(42)]);
}

#[test]
fn only_bluez_may_call() {
    let (mut ble, mut fake) = ready();
    let q = fake.call_as(
        ":1.66",
        gatt::MAP,
        CHAR,
        "ReadValue",
        vec![Value::props::<&str>([])],
    );
    pump(&mut ble, &mut fake);
    assert_eq!(
        fake.reply_to(q).unwrap().error_name.as_deref(),
        Some("org.freedesktop.DBus.Error.AccessDenied")
    );
}

#[test]
fn reads_honour_the_offset() {
    let (mut ble, mut fake) = ready();
    let q = fake.call(
        gatt::INFO,
        CHAR,
        "ReadValue",
        vec![Value::props([("offset", Value::U16(2))])],
    );
    pump(&mut ble, &mut fake);
    assert_eq!(
        fake.reply_to(q).unwrap().body,
        [Value::Bytes(vec![0x00, 0x03])]
    );
}

#[test]
fn a_host_connects_listens_and_gets_reports() {
    let (mut ble, mut fake) = ready();
    ble.reconnect(true);
    pump(&mut ble, &mut fake);
    assert!(!ble.send(&Report::Keyboard([0; 8])), "nobody to send to");
    fake.device(PHONE, vec![("Connected", Value::Bool(true))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Connected(PHONE.into())));
    // Connected: the reconnect advert stops.
    assert_eq!(fake.calls("UnregisterAdvertisement").len(), 1);
    fake.call(gatt::KEYBOARD, CHAR, "StartNotify", vec![]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Listening(true)));
    let r = [0, 0, 0x04, 0, 0, 0, 0, 0];
    assert!(ble.send(&Report::Keyboard(r)));
    assert!(ble.send(&Report::Keyboard(r)), "equal reports still go");
    pump(&mut ble, &mut fake);
    let sent: Vec<&gh0st_dbus::Message> = fake
        .signals
        .iter()
        .filter(|s| s.path.as_deref() == Some(gatt::KEYBOARD))
        .collect();
    assert_eq!(sent.len(), 2);
    let value = sent[0].body[1].get("Value").unwrap().inner();
    assert_eq!(value, &Value::Bytes(r.to_vec()));
    // To BlueZ alone: no other program on the bus hears the keys.
    assert_eq!(sent[0].destination.as_deref(), Some(":1.3"));
    // The host goes: the advert comes back for it.
    fake.device(PHONE, vec![("Connected", Value::Bool(false))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Disconnected(PHONE.into())));
    assert!(events.contains(&Event::Listening(false)));
    assert_eq!(fake.calls("RegisterAdvertisement").len(), 2);
}

#[test]
fn the_host_sets_the_lights_and_the_protocol() {
    let (mut ble, mut fake) = ready();
    let q = fake.call(
        gatt::LEDS,
        CHAR,
        "WriteValue",
        vec![Value::Bytes(vec![0x02]), Value::props::<&str>([])],
    );
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Leds(2)));
    assert!(fake.reply_to(q).is_some());
    // Boot protocol: key reports go to the boot characteristic.
    fake.call(
        gatt::MODE,
        CHAR,
        "WriteValue",
        vec![Value::Bytes(vec![0]), Value::props::<&str>([])],
    );
    fake.device(PHONE, vec![("Connected", Value::Bool(true))]);
    pump(&mut ble, &mut fake);
    ble.send(&Report::Keyboard([0; 8]));
    pump(&mut ble, &mut fake);
    assert!(
        fake.signals
            .iter()
            .any(|s| s.path.as_deref() == Some(gatt::BOOT_IN))
    );
}

#[test]
fn the_advert_ending_is_told() {
    let (mut ble, mut fake) = ready();
    ble.reconnect(true);
    pump(&mut ble, &mut fake);
    fake.call(
        gatt::ADVERT,
        "org.bluez.LEAdvertisement1",
        "Release",
        vec![],
    );
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::AdvertEnded));
    assert_eq!(ble.wanted(), Advert::Off);
}

#[test]
fn bluez_going_away_fails_the_link() {
    let (mut ble, mut fake) = ready();
    let gone = gh0st_dbus::Message::signal(
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
    )
    .arg(Value::str("org.bluez"))
    .arg(Value::str(":1.3"))
    .arg(Value::str(""));
    fake.emit(gone);
    let events = pump(&mut ble, &mut fake);
    assert!(
        matches!(events.last(), Some(Event::Failed(_))),
        "{events:?}"
    );
}

#[test]
fn stop_unregisters_everything() {
    let (mut ble, mut fake) = ready();
    ble.pair(true);
    pump(&mut ble, &mut fake);
    ble.stop();
    pump(&mut ble, &mut fake);
    assert_eq!(fake.calls("UnregisterApplication"), [ADAPTER]);
    assert_eq!(fake.calls("UnregisterAgent"), ["/org/bluez"]);
    assert_eq!(fake.calls("UnregisterAdvertisement").len(), 1);
}

const HEADSET: &str = "/org/bluez/hci0/dev_11_22_33_44_55_66";
const LAPTOP: &str = "/org/bluez/hci0/dev_AA_00_00_00_00_02";

#[test]
fn a_headset_is_not_a_host() {
    let (mut ble, mut fake) = ready();
    ble.reconnect(true);
    pump(&mut ble, &mut fake);
    fake.device(HEADSET, vec![("Connected", Value::Bool(true))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Connected(HEADSET.into())));
    assert_eq!(ble.connected(), None, "the headset types nothing");
    assert!(!ble.send(&Report::Keyboard([0; 8])));
    assert!(
        fake.calls("UnregisterAdvertisement").is_empty(),
        "still waiting for a host"
    );
}

#[test]
fn a_second_host_stays_when_the_first_goes() {
    let (mut ble, mut fake) = ready();
    ble.set_hosts(vec![PHONE.into(), LAPTOP.into()]);
    fake.device(PHONE, vec![("Connected", Value::Bool(true))]);
    fake.device(LAPTOP, vec![("Connected", Value::Bool(true))]);
    fake.call(gatt::KEYBOARD, CHAR, "StartNotify", vec![]);
    pump(&mut ble, &mut fake);
    fake.device(PHONE, vec![("Connected", Value::Bool(false))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Disconnected(PHONE.into())));
    assert_eq!(ble.connected(), Some(LAPTOP));
    assert!(ble.listening(), "the laptop still listens");
    assert!(ble.send(&Report::Keyboard([0; 8])));
}

#[test]
fn an_address_that_changes_is_told() {
    let (mut ble, mut fake) = ready();
    fake.device(PHONE, vec![("Address", Value::str("F8:2A:E2:48:8E:99"))]);
    let events = pump(&mut ble, &mut fake);
    assert!(events.contains(&Event::Moved(
        "AA:BB:CC:DD:EE:FF".into(),
        "F8:2A:E2:48:8E:99".into()
    )));
}

#[test]
fn the_advert_comes_back_after_the_radio_restarts() {
    let (mut ble, mut fake) = ready();
    ble.pair(true);
    pump(&mut ble, &mut fake);
    ble.power(false);
    pump(&mut ble, &mut fake);
    ble.power(true);
    pump(&mut ble, &mut fake);
    assert_eq!(fake.calls("RegisterAdvertisement").len(), 2);
    assert_eq!(ble.wanted(), Advert::Pair);
}
