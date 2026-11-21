//! Where parse time goes: vte alone, the terminal without history, and the
//! whole terminal, over the same 20 MB of plain lines.

use gh0st_term::vt::Vt;
use std::time::Instant;

struct Null(usize);

impl vte::Perform for Null {
    fn print(&mut self, _: char) {
        self.0 += 1;
    }
    fn execute(&mut self, _: u8) {
        self.0 += 1;
    }
}

fn main() {
    let line = format!("{}\r\n", "y".repeat(120));
    let chunk: Vec<u8> = line
        .as_bytes()
        .iter()
        .copied()
        .cycle()
        .take(16 * 1024)
        .collect();
    let total = 20 << 20;
    let rate = |t: Instant| total as f64 / t.elapsed().as_secs_f64() / 1e6;

    let mut p = vte::Parser::<4096>::new_with_size();
    let mut n = Null(0);
    let t = Instant::now();
    for _ in 0..total / chunk.len() {
        p.advance(&mut n, &chunk);
    }
    println!("vte alone:       {:.1} MB/s", rate(t));

    // Our side alone: the same calls vte would make, without vte.
    {
        use vte::Perform;
        let mut vt = Vt::new(53, 12, 1 << 20);
        let t = Instant::now();
        for _ in 0..total / 122 {
            for _ in 0..120 {
                vt.t.print('y');
            }
            vt.t.execute(b'\r');
            vt.t.execute(b'\n');
        }
        println!("terminal alone:  {:.1} MB/s", rate(t));
    }
    {
        use vte::Perform;
        let mut vt = Vt::new(53, 12, 1 << 20);
        let n = 20_000_000u64;
        let t = Instant::now();
        for i in 0..n {
            if i % 50 == 0 {
                vt.t.execute(b'\r');
            }
            vt.t.print('y');
        }
        let ns = t.elapsed().as_nanos() as f64 / n as f64;
        println!("print alone:     {ns:.1} ns per char");
        let t = Instant::now();
        let mut x = 0u64;
        for i in 0..n {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
        }
        std::hint::black_box(x);
        println!(
            "a mul-add loop:  {:.1} ns per step",
            t.elapsed().as_nanos() as f64 / n as f64
        );
    }

    {
        use gh0st_term::vt::cell::{Cell, Clusters, Pen};
        use gh0st_term::vt::grid::{Grid, Row};
        use gh0st_term::vt::history::History;
        let mut row = Row::new(53, Cell::BLANK);
        for c in &mut row.cells {
            *c = Cell::new('y', &Pen::DEFAULT);
        }
        let cl = Clusters::default();
        let mut h = History::new(1 << 20);
        let n = 1_000_000;
        let t = Instant::now();
        for _ in 0..n {
            h.push(&row, &cl);
        }
        println!(
            "history push:    {:.0} ns per row",
            t.elapsed().as_nanos() as f64 / n as f64
        );
        let mut g = Grid::new(53, 12);
        let t = Instant::now();
        for _ in 0..n {
            g.scroll_up(0, 11, 1, &Pen::DEFAULT);
        }
        println!(
            "grid scroll:     {:.0} ns per row",
            t.elapsed().as_nanos() as f64 / n as f64
        );
    }
    let short = format!("{}\r\n", "y".repeat(40));
    let short: Vec<u8> = short
        .as_bytes()
        .iter()
        .copied()
        .cycle()
        .take(16 * 1024)
        .collect();
    let mut nolf = Vec::new();
    while nolf.len() < 16 * 1024 {
        nolf.extend_from_slice(b"\x1b[Hyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy");
    }
    for (name, cols, data) in [
        ("200 cols, no wrap:", 200, &chunk),
        ("40-char lines:    ", 53, &short),
        ("no scroll at all: ", 53, &nolf),
    ] {
        let mut vt = Vt::new(cols, 12, 1 << 20);
        let t = Instant::now();
        for _ in 0..total / data.len() {
            vt.feed(data);
        }
        println!("{name} {:.1} MB/s", rate(t));
    }
    for (name, hist) in [("no history:     ", 0), ("1 MB history:   ", 1 << 20)] {
        let mut vt = Vt::new(53, 12, hist);
        let t = Instant::now();
        for _ in 0..total / chunk.len() {
            vt.feed(&chunk);
        }
        println!("{name} {:.1} MB/s", rate(t));
    }
}
