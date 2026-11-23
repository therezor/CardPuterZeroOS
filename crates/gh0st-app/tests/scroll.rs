//! A long text scrolls on Up and Down, stops at its last screen, and an
//! idle page with one doesn't wake.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::Cell;
use std::rc::Rc;

struct Long {
    scroll: Scroll,
    moved: Rc<Cell<u32>>,
}

impl Page for Long {
    fn title(&self) -> &str {
        "Long"
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        if self.scroll.key(key) {
            self.moved.set(self.moved.get() + 1);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let text = (1..=20)
            .map(|i| format!("Line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        ui.scroll_text(&self.scroll, &text);
    }
}

#[test]
fn scrolls_to_the_last_screen_and_no_further() {
    let moved = Rc::new(Cell::new(0));
    let mut d = Driver::new(Long {
        scroll: Scroll::new(),
        moved: moved.clone(),
    });
    d.frame();
    d.keys("f");
    assert_eq!(moved.get(), 0, "already at the top");
    for _ in 0..40 {
        d.keys("x").wait(20);
    }
    let down = moved.get();
    assert!(down > 0 && down < 20, "{down}: stops before the end");
    d.keys("f");
    assert_eq!(moved.get(), down + 1);
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    // The clock's minute and nothing else: 10 or 11 in ten minutes.
    let wakes = d.idle_wakes(600_000);
    assert!(wakes <= 11, "{wakes}");
}
