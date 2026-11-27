//! Bluetooth from the real keys against a fake BlueZ: a scan finds a
//! speaker, Enter pairs it with the adapter pairable for that pairing
//! only, trusts and connects it, and Forget held on Del drops a device.

use gh0st_app::testing::Driver;
use gh0st_settings::app::Settings;

mod common;
use common::fakebluez::{self, ADAPTER, Dev};

#[test]
fn scan_pair_and_forget() {
    common::stub_tools();
    let mut buds = Dev::new("AA:BB:CC:DD:EE:01", "Buds");
    buds.paired = true;
    let mut jbl = Dev::new("D8:37:3B:35:E0:81", "JBL Xtreme 3");
    jbl.hidden = true;
    let (buds_path, jbl_path) = (buds.path.clone(), jbl.path.clone());
    let (conn, bluez) = fakebluez::start(true, vec![buds, jbl]);

    let mut d = Driver::new(Settings::new().with_bluez(conn));
    d.jobs(2000).pump(200);
    d.keys("x\nx\nx\nenter").pump(200);
    assert_eq!(d.title(), "Bluetooth");

    // Scan, for its time, then stop.
    d.keys("u").pump(200);
    assert!(bluez.called(&format!("StartDiscovery {ADAPTER}")));
    d.wait(20_100).pump(200);
    assert!(bluez.called(&format!("StopDiscovery {ADAPTER}")));

    // The switch, Buds (paired), then JBL (found).
    d.keys("x\nx\nenter").pump(300);
    let calls = bluez.calls();
    let order = [
        format!("Set {ADAPTER} PairableTimeout=180"),
        format!("Set {ADAPTER} Pairable=true"),
        format!("Pair {jbl_path}"),
        format!("Set {ADAPTER} Pairable=false"),
        format!("Set {jbl_path} Trusted=true"),
        format!("Connect {jbl_path}"),
    ];
    let at: Vec<usize> = order
        .iter()
        .map(|c| calls.iter().position(|x| x == c).expect(c))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "{calls:?}");
    assert_eq!(d.toast(), Some("Paired with JBL Xtreme 3"));

    // JBL, connected, is first now: the third row is Buds. A tap asks for
    // a hold; the hold forgets.
    d.keys("backspace").pump(200);
    assert_eq!(d.toast(), Some("Hold to forget"));
    assert!(!bluez.called(&format!("RemoveDevice {buds_path}")));
    d.keys("down backspace")
        .wait(1000)
        .keys("up backspace")
        .pump(300);
    assert!(bluez.called(&format!("RemoveDevice {buds_path}")));
    assert_eq!(d.toast(), Some("Forgot Buds"));
}

#[test]
fn a_refusal_says_so() {
    common::stub_tools();
    let mut jbl = Dev::new("D8:37:3B:35:E0:81", "JBL Xtreme 3");
    jbl.hidden = true;
    jbl.refuse = Some("org.bluez.Error.AuthenticationRejected");
    let (conn, bluez) = fakebluez::start(true, vec![jbl]);
    let mut d = Driver::new(Settings::new().with_bluez(conn));
    d.jobs(2000).pump(200);
    d.keys("x\nx\nx\nenter").pump(200);
    d.keys("u").pump(200);
    d.wait(20_100).pump(200);
    d.keys("x\nenter").pump(300);
    assert_eq!(d.toast(), Some("JBL Xtreme 3 refused to pair"));
    assert!(bluez.called(&format!("Set {ADAPTER} Pairable=false")));
}

#[test]
fn a_second_scan_still_lists_what_bluez_saw_lately() {
    common::stub_tools();
    let mut jbl = Dev::new("D8:37:3B:35:E0:81", "JBL Xtreme 3");
    jbl.hidden = true;
    let jbl_path = jbl.path.clone();
    let (conn, bluez) = fakebluez::start(true, vec![jbl]);
    let mut d = Driver::new(Settings::new().with_bluez(conn));
    d.jobs(2000).pump(200);
    d.keys("x\nx\nx\nenter").pump(200);
    // The first scan finds it; the second hears nothing new of it.
    d.keys("u").pump(200).wait(20_100).pump(200);
    d.keys("u").pump(200).wait(20_100).pump(200);
    assert_ne!(d.toast(), Some("Nothing found. Is it in pairing mode?"));
    d.keys("x\nenter").pump(300);
    assert!(bluez.called(&format!("Pair {jbl_path}")));
}
