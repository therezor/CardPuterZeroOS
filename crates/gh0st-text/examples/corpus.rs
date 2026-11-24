//! Writes the files the editor is measured and tested on. They are large,
//! so they never go in the repo:
//!
//!     cargo run --release -p gh0st-text --example corpus -- DIR [LOG_MB]
//!
//! - `big.log`: a log of LOG_MB megabytes (1024 by default), real lines.
//! - `one-line.json`: 5 MB of JSON on a single line.
//! - `crlf.txt`: lines ending in CR LF.
//! - `bad-utf8.txt`: bytes that are not UTF-8 among text.
//! - `no-newline.txt`: no newline after the last line.
//! - `nul.bin`: a binary file, NULs in the first block.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("usage: corpus DIR [LOG_MB]");
        std::process::exit(2);
    };
    let log_mb: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir)?;
    log(&dir.join("big.log"), log_mb << 20)?;
    json(&dir.join("one-line.json"), 5 << 20)?;
    small(dir)?;
    println!("wrote {}", dir.display());
    Ok(())
}

/// A log like a service writes: a time, a level, a source and a message of
/// varying length, so lines are not all alike.
fn log(path: &Path, size: u64) -> std::io::Result<()> {
    const LEVELS: [&str; 4] = ["INFO ", "DEBUG", "WARN ", "ERROR"];
    const WORDS: [&str; 12] = [
        "opened", "closed", "read", "wrote", "synced", "retried", "dropped", "queued", "mapped",
        "skipped", "renamed", "flushed",
    ];
    let mut w = BufWriter::with_capacity(4 << 20, File::create(path)?);
    let mut line = String::with_capacity(256);
    let mut n: u64 = 0;
    let mut written: u64 = 0;
    let mut seed: u32 = 0x9e37_79b9;
    while written < size {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        line.clear();
        let s = n % 86_400;
        use std::fmt::Write as _;
        let _ = write!(
            line,
            "2026-11-24T{:02}:{:02}:{:02}.{:03}Z {} gh0st-text[{}]: line {n} {} /home/pi/file-{}.txt in {} ms",
            s / 3600,
            s / 60 % 60,
            s % 60,
            seed % 1000,
            LEVELS[(seed >> 3) as usize % 4],
            1000 + seed % 9000,
            WORDS[(seed >> 7) as usize % WORDS.len()],
            seed % 977,
            seed % 250,
        );
        for _ in 0..(seed >> 11) % 6 {
            line.push_str(" and more words");
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
        written += line.len() as u64;
        n += 1;
    }
    w.flush()
}

fn json(path: &Path, size: usize) -> std::io::Result<()> {
    let mut s = String::with_capacity(size + 256);
    s.push('[');
    let mut i = 0;
    while s.len() < size {
        use std::fmt::Write as _;
        let _ = write!(
            s,
            "{{\"id\":{i},\"name\":\"item {i}\",\"tags\":[\"a\",\"b\"],\"ok\":true,\"size\":{}.5}},",
            i * 7
        );
        i += 1;
    }
    s.pop();
    s.push(']');
    std::fs::write(path, s)
}

fn small(dir: &Path) -> std::io::Result<()> {
    std::fs::write(
        dir.join("crlf.txt"),
        "first line\r\nsecond line\r\n\r\nlast line\r\n",
    )?;
    std::fs::write(
        dir.join("bad-utf8.txt"),
        b"good text\nbad \xff\xfe bytes\nhalf \xe2\x82 a euro\nend\n",
    )?;
    std::fs::write(dir.join("no-newline.txt"), "one\ntwo\nthree")?;
    let mut bin = vec![0u8; 4096];
    for (i, b) in bin.iter_mut().enumerate() {
        *b = (i * 31 % 256) as u8;
    }
    std::fs::write(dir.join("nul.bin"), bin)
}
