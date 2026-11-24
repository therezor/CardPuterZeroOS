//! Times the editor on a file, against its budgets (docs/text.md): the
//! engine, then the page as the app runs it, keys to frame.
//!
//!     cargo run --release -p gh0st-text --example bench -- FILE
//!
//! Run it on the corpus from `examples/corpus.rs`, on a host and on the
//! device.

use gh0st_app::testing::Driver;
use gh0st_text::app::{Editor, KeyModel, Options};
use gh0st_text::buf::Buffer;
use gh0st_text::find::{Finder, Found};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

/// Counts allocations, to see what a frame asks of the heap.
struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes on to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

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

    // Find as the app runs it: folded, in steps, a reader warming
    // the file ahead. A match near the end, then a miss whose first letter
    // is common in one case only.
    let mut finds = Vec::new();
    for needle in ["LINE 7700000 ", "lzzq"] {
        let t = Instant::now();
        let mut f = Finder::new(needle.as_bytes(), 0, true);
        let mut warm = (b.len() >= gh0st_text::warm::MIN)
            .then(|| gh0st_text::warm::spawn(path.as_ref(), 0).ok())
            .flatten();
        b.set_warming(warm.is_some());
        let mut worst: f64 = 0.0;
        let found = loop {
            if let Some(pid) = warm {
                let mut st = 0;
                // SAFETY: a non-blocking wait on our own child.
                if unsafe { libc::waitpid(pid, &mut st, libc::WNOHANG) } == pid {
                    warm = None;
                    b.set_warming(false);
                }
            }
            let s = Instant::now();
            let r = f.step(&mut b, gh0st_text::find::STEP)?;
            worst = worst.max(ms(s));
            match r {
                Found::Hit(at) => break Some(at),
                Found::Miss => break None,
                Found::More => {}
                Found::Wait => std::thread::sleep(std::time::Duration::from_millis(15)),
            }
        };
        if let Some(pid) = warm {
            gh0st_text::warm::stop(pid);
            let mut st = 0;
            // SAFETY: reaps the reader just stopped.
            unsafe { libc::waitpid(pid, &mut st, 0) };
            b.set_warming(false);
        }
        finds.push((needle, ms(t), worst, found.is_some()));
    }

    // The journal's write: a typing run, then fsync, as 2 s after typing.
    let jdir = std::env::temp_dir().join(format!("gh0st-text-bench-{}", std::process::id()));
    let mut j = gh0st_text::journal::Journal::new(&jdir, path.as_ref());
    for i in 0..40 {
        j.add(&gh0st_text::buf::Change::Replace {
            at: i,
            len: 0,
            bytes: b"x".to_vec(),
        });
    }
    let t = Instant::now();
    j.flush()?;
    let first_flush = ms(t);
    j.add(&gh0st_text::buf::Change::Replace {
        at: 40,
        len: 0,
        bytes: b"y".to_vec(),
    });
    let t = Instant::now();
    j.flush()?;
    let next_flush = ms(t);
    j.discard();

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
    for (needle, total, worst, hit) in finds {
        println!("find {needle:14} {total:.0} ms, worst step {worst:.2} ms ({hit})");
    }
    println!(
        "lex as Rust       {:.0} MB/s ({lex_ms:.1} ms for {} kB)",
        (lexed as f64 / 1048576.0) / (lex_ms / 1000.0),
        lexed >> 10
    );
    println!("journal fsync     first {first_flush:.2} ms, then {next_flush:.2} ms");
    println!("max RSS           {} kB", rss_kb());
    drop(b);
    page(path.as_ref(), &jdir, mid + 1)?;
    let _ = std::fs::remove_dir_all(&jdir);
    Ok(())
}

/// Median and worst.
fn spread(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (v[v.len() / 2], v[v.len() - 1])
}

/// Runs `keys` `n` times, each followed by a frame: the time from the key
/// to a drawn frame, the heap allocations, and the rows the panel would be
/// sent.
fn steps(d: &mut Driver, keys: &str, n: usize) -> String {
    let mut times = Vec::with_capacity(n);
    let mut allocs = Vec::with_capacity(n);
    let mut rows = Vec::with_capacity(n);
    let mut before = d.frame().pixels().to_vec();
    for _ in 0..n {
        let a = ALLOCS.load(Relaxed);
        let t = Instant::now();
        if !keys.is_empty() {
            d.keys(keys);
        }
        let f = d.frame();
        times.push(ms(t));
        allocs.push((ALLOCS.load(Relaxed) - a) as f64);
        let w = f.width() as usize;
        let changed = f
            .pixels()
            .chunks(w)
            .zip(before.chunks(w))
            .filter(|(a, b)| a != b)
            .count();
        rows.push(changed as f64);
        before.clear();
        before.extend_from_slice(f.pixels());
    }
    let (t50, tmax) = spread(&mut times);
    let (a50, amax) = spread(&mut allocs);
    let (r50, rmax) = spread(&mut rows);
    format!(
        "{t50:.3} ms, worst {tmax:.3}; allocs {a50:.0}, worst {amax:.0}; rows {r50:.0}, worst {rmax:.0}"
    )
}

/// The page: what a key costs from the press to a drawn frame. The panel's
/// send is not here: it needs the device's screen.
fn page(path: &Path, data: &Path, line: u64) -> std::io::Result<()> {
    let opts = Options {
        keys: Some(KeyModel::Modeless),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    let t = Instant::now();
    let mut d = Driver::new(Editor::open(Some(path), opts.clone()));
    d.frame();
    let first = ms(t);
    // Indexing runs on the app's timer; virtual time lets it run flat out.
    let t = Instant::now();
    d.wait(3_600_000);
    let indexed = ms(t);
    d.keys(&format!("ctrl+g\ntype {line}\nenter"));
    println!("page first frame  {first:.2} ms; indexed in {indexed:.0} ms");
    println!("page frame        {}", steps(&mut d, "", 200));
    println!("page type a key   {}", steps(&mut d, "type x", 200));
    println!("page cursor down  {}", steps(&mut d, "fn+x", 200));
    println!("page page down    {}", steps(&mut d, "fn+m", 100));
    d.keys("ctrl+z");
    drop(d);

    // Wrap at its worst: a line just under the 64 KB limit, at its tail.
    let long = data.join("long.rs");
    let line: String = (0..2400)
        .map(|i| format!("let v{i} = (a + {i}); "))
        .collect::<String>()
        .chars()
        .take(60 << 10)
        .collect();
    std::fs::write(&long, format!("fn main() {{\n{line}\n}}\n"))?;
    let wrap = Options {
        wrap: Some(true),
        ..opts.clone()
    };
    let mut d = Driver::new(Editor::open(Some(&long), wrap));
    d.wait(1000);
    d.keys("fn+x\nctrl+e");
    println!("wrap 60 KB line   {}", steps(&mut d, "", 100));
    println!("wrap type a key   {}", steps(&mut d, "type x", 100));
    println!("wrap cursor up    {}", steps(&mut d, "fn+f", 100));
    drop(d);

    // Hex: a binary file a row at a time.
    let bin = data.join("bin");
    let bytes: Vec<u8> = (0..1u32 << 16)
        .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    std::fs::write(&bin, [&[0u8; 16][..], &bytes].concat())?;
    let mut d = Driver::new(Editor::open(Some(&bin), opts));
    d.wait(1000);
    println!("hex frame         {}", steps(&mut d, "", 100));
    println!("hex row down      {}", steps(&mut d, "fn+x", 100));
    Ok(())
}
