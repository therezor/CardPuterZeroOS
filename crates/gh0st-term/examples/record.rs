//! Records what a program writes to a 53x12 terminal, for golden tests.
//! `cargo run -p gh0st-term --example record -- OUT MS KEYS CMD ARGS...`:
//! runs CMD, waits MS, sends KEYS (with `\r` for Enter), waits again, and
//! saves every byte to OUT.

use gh0st_term::pty::Pty;
use std::time::{Duration, Instant};

fn pump(p: &Pty, ms: u64, out: &mut Vec<u8>) {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut buf = [0u8; 4096];
    while Instant::now() < end {
        match p.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return,
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (out, ms, keys) = (
        &a[0],
        a[1].parse().unwrap(),
        a[2].replace("\\r", "\r").replace("\\033", "\x1b"),
    );
    let p = Pty::spawn(&a[3..], None, None, 53, 12).unwrap();
    let mut bytes = Vec::new();
    pump(&p, ms, &mut bytes);
    // SAFETY: writes the keys to the PTY master.
    unsafe { libc::write(p.fd(), keys.as_ptr().cast(), keys.len()) };
    pump(&p, ms, &mut bytes);
    std::fs::write(out, &bytes).unwrap();
    println!("{} bytes", bytes.len());
}
