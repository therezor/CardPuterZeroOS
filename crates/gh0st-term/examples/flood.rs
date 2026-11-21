//! Feeds 50 MB of shell-like output through the terminal, as `cat` of a big
//! file would, and reports the parse rate and what history holds.
//! `cargo run --release -p gh0st-term --example flood`

use gh0st_term::vt::Vt;
use std::time::Instant;

fn main() {
    let mut chunk = Vec::new();
    let mut i = 0u32;
    while chunk.len() < 16 * 1024 {
        let plain = std::env::var_os("PLAIN").is_some();
        let line = match if plain { 2 } else { i % 4 } {
            0 => format!("drwxr-xr-x  2 rezor staff  {i:>6} Nov 21 10:00 dir{i}\r\n"),
            1 => format!("\x1b[1;34mblue{i}\x1b[0m  \x1b[32mgreen\x1b[0m plain text here\r\n"),
            2 => format!("{}\r\n", "y".repeat(120)),
            _ => format!("\x1b[38;5;{}mcolour {i}\x1b[0m ünïcödé 漢字\r\n", i % 256),
        };
        chunk.extend_from_slice(line.as_bytes());
        i += 1;
    }
    let total = std::env::var("MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        << 20;
    let mut vt = Vt::new(53, 12, 1 << 20);
    let t0 = Instant::now();
    let mut fed = 0;
    while fed < total {
        vt.feed(&chunk);
        fed += chunk.len();
    }
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "{} MB in {secs:.2} s: {:.0} MB/s; history {} lines, {} KB held",
        fed >> 20,
        fed as f64 / secs / 1e6,
        vt.t.history.len(),
        vt.t.history.bytes() >> 10
    );
}
