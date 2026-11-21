//! Feeds a file of recorded output through a 53x12 terminal and prints the
//! screen, one row a line. `cargo run -p gh0st-term --example replay -- FILE`

use gh0st_term::vt::Vt;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let bytes = std::fs::read(path).unwrap();
    let mut vt = Vt::new(53, 12, 1 << 20);
    vt.feed(&bytes);
    let t = &vt.t;
    for (y, r) in t.grid().rows.iter().enumerate() {
        let s: String = r
            .cells
            .iter()
            .filter(|c| !c.is_spacer())
            .map(|c| {
                if c.is_cluster() {
                    t.clusters.get(c.code()).to_string()
                } else {
                    c.ch().to_string()
                }
            })
            .collect();
        println!(
            "{y:2} {}{}",
            s.trim_end(),
            if r.wrapped { " ⏎" } else { "" }
        );
    }
    // With a second argument, draws the screen there as a PNG.
    if let Some(png) = std::env::args().nth(2) {
        use gh0st_app::gfx::{Canvas, Rect, Theme, theme};
        use gh0st_term::render::{self, Colors, Look, Span};
        let th = Theme::default();
        let look = Look::of(&theme::TERM_TERMINUS[0]);
        let mut c = Canvas::new(320, 12 * 12);
        c.fill(c.bounds(), th.c.bg);
        let area = Rect::new(0, 0, 320, 144);
        for (y, r) in t.grid().rows.iter().enumerate() {
            let span = Span {
                x: 0,
                y: y as i32 * 12,
                first: 0,
                n: 53,
            };
            render::row(
                &mut c,
                area,
                &look,
                &Colors::of(&th),
                &span,
                &r.cells,
                &t.clusters,
                None,
                None,
            );
        }
        gh0st_app::gfx::snapshot::write_png(&c, std::path::Path::new(&png), 3).unwrap();
    }
}
