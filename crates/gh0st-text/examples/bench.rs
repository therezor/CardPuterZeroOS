//! Times the editor's engine on a file, against its budgets (docs/text.md):
//!
//!     cargo run --release -p gh0st-text --example bench -- FILE
//!
//! Run it on the corpus from `examples/corpus.rs`, on a host and on the
//! device.

use gh0st_text::buf::Buffer;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn rss_kb() -> i64 {
    // SAFETY: getrusage fills the struct it is given.
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    // kB on Linux, bytes on macOS.
    if cfg!(target_os = "macos") {
        u.ru_maxrss / 1024
    } else {
        u.ru_maxrss
    }
}

fn main() -> std::io::Result<()> {
    // The editor's reader runs as this binary, as it does as gh0st-text.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("--warm") {
        let from = raw.get(2).and_then(|f| f.parse().ok()).unwrap_or(0);
        std::process::exit(gh0st_text::warm::main(raw[1].as_ref(), from));
    }
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: bench FILE");
        std::process::exit(2);
    };
    let t = Instant::now();
    let mut b = Buffer::open(path.as_ref())?;
    let open = ms(t);

    // The first screen: 14 lines from the top.
    let t = Instant::now();
    let mut off = 0;
    for _ in 0..14 {
        let end = b.line_end(off)?;
        let _ = b.bytes(off..end.min(off + 4096))?;
        off = (end + 1).min(b.len());
    }
    let first = ms(t);

    // The end, before any index: the last line's start.
    let t = Instant::now();
    let len = b.len();
    let last = b.line_start(len)?;
    let to_end = ms(t);

    // Index slices of 2 MB, the slowest one, and the whole run. A slice
    // the disk is behind on waits 15 ms, as the app's timer does.
    let t = Instant::now();
    let mut slices = 0;
    let mut waits = 0;
    let mut worst: f64 = 0.0;
    // A reader brings the file into memory, as the editor starts one.
    let mut warm = (b.len() >= gh0st_text::warm::MIN)
        .then(|| gh0st_text::warm::spawn(path.as_ref(), 0).ok())
        .flatten();
    b.set_warming(warm.is_some());
    loop {
        if let Some(pid) = warm {
            let mut st = 0;
            // SAFETY: a non-blocking wait on our own child.
            if unsafe { libc::waitpid(pid, &mut st, libc::WNOHANG) } == pid {
                warm = None;
                b.set_warming(false);
            }
        }
        let s = Instant::now();
        let done = match b.index_step(2 << 20) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                worst = worst.max(ms(s));
                waits += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e),
        };
        worst = worst.max(ms(s));
        slices += 1;
        if done {
            break;
        }
    }
    let index = ms(t);
    let lines = b.lines();

    // Line lookups once indexed.
    let t = Instant::now();
    let mid = lines.count / 2;
    let at = b.line_offset(mid)?.unwrap_or(0);
    let lookup = ms(t);

    // Typing in the middle: 1000 one-byte inserts, then undo them all.
    let t = Instant::now();
    for i in 0..1000u64 {
        b.insert(at + i, b"x")?;
    }
    let typing = ms(t) / 1000.0;
    let t = Instant::now();
    b.seal();
    b.undo();
    let undo = ms(t);

    // Find: a string near the end.
    let t = Instant::now();
    let needle = b"line 7700000 ";
    let mut found = None;
    let mut pos = 0;
    let mut carry: Vec<u8> = Vec::new();
    let len = b.len();
    b.for_each(len.saturating_sub(64 << 20)..len, |c| {
        carry.extend_from_slice(c);
        if let Some(i) = carry.windows(needle.len()).position(|w| w == needle) {
            found = Some(pos + i);
            return false;
        }
        let keep = carry.len().saturating_sub(needle.len());
        pos += keep;
        carry.drain(..keep);
        true
    })?;
    let find = ms(t);

    // Lexing: the first 4 MB, line by line, as Rust, the busiest table.
    let t = Instant::now();
    let mut state = 0;
    let mut spans = Vec::new();
    let mut lexed = 0u64;
    let mut off = 0;
    while off < len.min(4 << 20) {
        let end = b.line_end(off)?;
        let line = b.bytes(off..end.min(off + gh0st_text::lex::LINE_MAX as u64))?;
        state = gh0st_text::lex::lex(gh0st_text::lex::Lang::Rust, state, &line, &mut spans);
        lexed += line.len() as u64 + 1;
        off = end + 1;
    }
    let lex_ms = ms(t);

    println!("file              {} MB", len >> 20);
    println!("open              {open:.2} ms");
    println!("first screen      {first:.2} ms");
    println!("last line start   {to_end:.2} ms (at {last})");
    println!(
        "index             {index:.0} ms in {slices} slices of 2 MB and {waits} waits for the disk, worst {worst:.2} ms, {:.0} MB/s",
        (len as f64 / 1048576.0) / (index / 1000.0)
    );
    println!("lines             {} (exact: {})", lines.count, lines.exact);
    println!("line {mid}        {lookup:.3} ms");
    println!(
        "insert            {typing:.4} ms each, {} pieces",
        b.pieces()
    );
    println!("undo 1000         {undo:.3} ms");
    println!("find in last 64MB {find:.1} ms ({})", found.is_some());
    println!(
        "lex as Rust       {:.0} MB/s ({lex_ms:.1} ms for {} kB)",
        (lexed as f64 / 1048576.0) / (lex_ms / 1000.0),
        lexed >> 10
    );
    println!("max RSS           {} kB", rss_kb());
    Ok(())
}
