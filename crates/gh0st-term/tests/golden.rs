//! Real programs' output, recorded at 53x12 with `examples/record.rs`, fed
//! back through the emulator. The screen must come out as the program drew
//! it.

use gh0st_term::vt::Vt;

fn rows(vt: &Vt) -> Vec<String> {
    let t = &vt.t;
    t.grid()
        .rows
        .iter()
        .map(|r| {
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
            s.trim_end().to_string()
        })
        .collect()
}

/// Feeds `bytes` a byte at a time and returns the last screen seen on the
/// alternate screen, where full-screen programs draw.
fn last_alt_screen(bytes: &[u8]) -> (Vec<String>, Vt) {
    let mut vt = Vt::new(53, 12, 1 << 16);
    let mut seen = Vec::new();
    for b in bytes {
        vt.feed(std::slice::from_ref(b));
        if vt.t.alt_active {
            seen = rows(&vt);
        }
    }
    (seen, vt)
}

#[test]
fn vi_draws_its_file_tildes_and_status() {
    let (screen, vt) = last_alt_screen(include_bytes!("data/vi.bin"));
    assert!(vt.t.alt_active, "vi still runs");
    assert_eq!(screen[0], "helloline one", "{screen:#?}");
    assert_eq!(screen[1], "line two");
    for r in &screen[2..11] {
        assert_eq!(r, "~", "{screen:#?}");
    }
}

#[test]
fn top_draws_its_header_and_gives_the_screen_back() {
    let (screen, vt) = last_alt_screen(include_bytes!("data/top.bin"));
    assert!(screen[0].starts_with("Processes:"), "{screen:#?}");
    assert!(screen.iter().any(|r| r.starts_with("PID")), "{screen:#?}");
    assert!(!vt.t.alt_active, "q left the alternate screen");
}

/// Yazi 26.9.1 on a PNG, with this terminal answering its queries: it finds
/// sixel in the DA1 reply and shows the file in the preview pane.
#[test]
fn yazi_previews_an_image_in_sixel_and_erases_it_on_moving_away() {
    let (screen, vt) = last_alt_screen(include_bytes!("data/yazi-sixel.bin"));
    assert!(screen[1].contains("a-home.png"), "{screen:#?}");
    let imgs = &vt.t.grid().images;
    assert_eq!(imgs.len(), 1);
    // In the preview pane, on the right, under the title row.
    assert!(
        imgs[0].col > 26 && imgs[0].row == 1,
        "{} {}",
        imgs[0].col,
        imgs[0].row
    );
    let (_, vt) = last_alt_screen(include_bytes!("data/yazi-sixel-move.bin"));
    assert!(vt.t.grid().images.is_empty());
}
