//! The USB link: its queues against a node that refuses writes, and the
//! helper's protocol against a stand-in script.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

use gh0st_keyboard::hid::report::Report;
use gh0st_keyboard::link::usb::{Node, Usb};
use gh0st_keyboard::link::{Cable, Event, UsbState};

/// A node that takes `room` reports, then says it is full until `open`.
#[derive(Clone, Default)]
struct Gate {
    got: Rc<RefCell<Vec<Vec<u8>>>>,
    room: Rc<RefCell<usize>>,
}

impl Node for Gate {
    fn fd(&self) -> RawFd {
        -1
    }
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut room = self.room.borrow_mut();
        if *room == 0 {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        *room -= 1;
        self.got.borrow_mut().push(data.to_vec());
        Ok(data.len())
    }
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::WouldBlock.into())
    }
}

fn usb(kbd: &Gate) -> Usb {
    Usb::connected([
        Box::new(kbd.clone()),
        Box::new(Gate::default()),
        Box::new(Gate::default()),
    ])
}

#[test]
fn a_release_behind_a_busy_press_still_arrives() {
    let kbd = Gate::default();
    *kbd.room.borrow_mut() = 1;
    let mut u = usb(&kbd);
    let press = Report::Keyboard([0, 0, 4, 0, 0, 0, 0, 0]);
    let release = Report::Keyboard([0; 8]);
    assert!(u.send(&press));
    assert!(u.send(&release));
    assert_eq!(kbd.got.borrow().len(), 1, "the node took only the press");
    assert!(u.waiting());
    // The host collects the press: the next tick sends the release.
    *kbd.room.borrow_mut() = 1;
    u.tick();
    assert_eq!(kbd.got.borrow()[1], [0; 8]);
    assert!(!u.waiting());
}

#[test]
fn a_host_that_never_reads_is_not_queued_forever() {
    let kbd = Gate::default();
    let mut u = usb(&kbd);
    let r = Report::Keyboard([0; 8]);
    let taken = (0..200).filter(|_| u.send(&r)).count();
    assert_eq!(taken, 64);
}

#[test]
fn reports_go_to_their_own_node() {
    let kbd = Gate::default();
    *kbd.room.borrow_mut() = 10;
    let mut u = usb(&kbd);
    u.send(&Report::Consumer(0xcd));
    assert!(
        kbd.got.borrow().is_empty(),
        "media keys are not the keyboard's"
    );
}

/// A stand-in helper: prints a node and the states, then waits for stdin
/// to close, as the real one holds the gadget.
fn script(dir: &std::path::Path, states: &[&str]) -> Vec<String> {
    let node = dir.join("hidg0");
    std::fs::write(&node, b"").unwrap();
    let mut s = format!("echo node keyboard {}; ", node.display());
    for st in states {
        s.push_str(&format!("echo state {st}; "));
    }
    s.push_str("cat >/dev/null");
    vec!["sh".into(), "-c".into(), s]
}

fn pump(u: &mut Usb) -> Vec<Event> {
    let mut out = Vec::new();
    for _ in 0..50 {
        for fd in u.fds() {
            u.ready(fd, &mut out);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    out
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("gh0st-kb-usb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_helper_says_configured_and_typing_goes_through() {
    let d = dir("ok");
    let mut u = Usb::with(
        script(&d, &["not attached", "configured"]),
        d.join("no-hub"),
    );
    u.enable(true);
    assert_eq!(u.state(), UsbState::Starting);
    let pid = u.spawned().expect("a child to reap");
    let events = pump(&mut u);
    assert!(
        events.contains(&Event::Usb(UsbState::Connected)),
        "{events:?}"
    );
    assert!(u.send(&Report::Keyboard([0, 0, 4, 0, 0, 0, 0, 0])));
    assert_eq!(
        std::fs::read(d.join("hidg0")).unwrap(),
        [0, 0, 4, 0, 0, 0, 0, 0]
    );
    // Off closes stdin: the helper ends on its own.
    u.enable(false);
    let mut status = 0;
    // SAFETY: waits for the child this test started.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    assert!(libc::WIFEXITED(status));
    let mut out = Vec::new();
    u.exited(pid, Some(0), &mut out);
    assert_eq!(u.state(), UsbState::Off);
}

#[test]
fn not_attached_with_the_hub_there_means_the_switch_is_on_host() {
    let d = dir("host");
    std::fs::create_dir_all(d.join("hub")).unwrap();
    let mut u = Usb::with(script(&d, &["not attached"]), d.join("hub"));
    u.enable(true);
    let events = pump(&mut u);
    assert!(
        events.contains(&Event::Usb(UsbState::HostMode)),
        "{events:?}"
    );
    std::fs::remove_dir_all(d.join("hub")).unwrap();
    // Read again once anything comes, as a uevent would on the device.
    u.tick();
    assert!(
        !u.send(&Report::Keyboard([0; 8])),
        "no host: nothing is queued"
    );
    u.stop();
}

#[test]
fn a_helper_that_fails_says_so() {
    let mut u = Usb::with(
        vec!["sh".into(), "-c".into(), "exit 1".into()],
        "/nonexistent".into(),
    );
    u.enable(true);
    let pid = u.spawned().unwrap();
    let mut status = 0;
    // SAFETY: waits for the child this test started.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let mut out = Vec::new();
    u.exited(pid, Some(1), &mut out);
    assert_eq!(out, [Event::Usb(UsbState::Failed)]);
    assert!(u.why.is_some());
}
