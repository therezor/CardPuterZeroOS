//! The Bluetooth link on the device, without the app: turns Bluetooth on,
//! makes the keyboard discoverable, says yes to pairing, and types a line
//! once the host listens. For checking a host before the pages exist:
//!
//!     gh0st-keyboard-probe           # pair a new host
//!     gh0st-keyboard-probe again     # wait for a bonded host
//!
//! Ctrl+C stops it. Every event is printed.

use std::time::{Duration, Instant};

use gh0st_keyboard::engine;
use gh0st_keyboard::hid::report::Report;
use gh0st_keyboard::link::ble::Ble;
use gh0st_keyboard::link::{Ask, Event};

const LINE: &str = "hello from gh0st\n";
/// A report every this often: slow enough for any host.
const PACE: Duration = Duration::from_millis(15);

static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn main() {
    let again = std::env::args().nth(1).as_deref() == Some("again");
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
    if gh0st_core::rfkill::set(gh0st_core::rfkill::Radio::Bluetooth, false).is_err() {
        println!("rfkill: asking the helper");
        let _ = std::process::Command::new("pkexec")
            .args(["/usr/libexec/gh0st-helper", "rfkill", "bluetooth", "on"])
            .status();
    }
    let mut ble = match Ble::system() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mut queue: std::collections::VecDeque<Report> = Default::default();
    let mut next = Instant::now();
    let mut typed = false;
    let start = Instant::now();
    let end = start + Duration::from_secs(600);
    while !STOP.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < end {
        let mut pfd = libc::pollfd {
            fd: ble.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = if queue.is_empty() {
            500
        } else {
            PACE.as_millis() as i32
        };
        // SAFETY: one pollfd on our stack.
        unsafe { libc::poll(&mut pfd, 1, wait) };
        let mut events = Vec::new();
        ble.ready(&mut events);
        for e in events {
            println!("{:>7} ms  {e:?}", start.elapsed().as_millis());
            match e {
                Event::Power(false) => ble.power(true),
                Event::Ready if again => ble.reconnect(true),
                Event::Ready => ble.pair(true),
                Event::Power(true) => {}
                Event::Pairing { ask, .. } => match ask {
                    Ask::Confirm(code) => {
                        println!("  code {code:06}: saying yes; check the host shows it");
                        ble.answer(true);
                    }
                    Ask::Show(code) => println!("  type {code:06} on the host"),
                    Ask::Allow => ble.answer(true),
                    Ask::Enter => {
                        println!("  the host wants a code typed here: not in the probe");
                        ble.answer(false);
                    }
                },
                Event::Paired(_) => ble.pair(false),
                Event::Listening(true) if !typed => {
                    typed = true;
                    let mut out = Vec::new();
                    engine::text(LINE, &mut out);
                    queue.extend(out);
                    next = Instant::now() + Duration::from_secs(1);
                }
                Event::Failed(_) => STOP.store(true, std::sync::atomic::Ordering::SeqCst),
                _ => {}
            }
        }
        if Instant::now() >= next
            && let Some(r) = queue.pop_front()
        {
            ble.send(&r);
            next = Instant::now() + PACE;
        }
    }
    println!("stopping");
    ble.stop();
}
