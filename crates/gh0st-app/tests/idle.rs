//! `Driver::idle_wakes` sees a timer that runs while nothing happens.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;

struct Ticker(bool);

impl Page for Ticker {
    fn title(&self) -> &str {
        "Ticker"
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.0 {
            cx.every(1_000);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text("tick");
    }
}

#[test]
fn a_timer_at_idle_shows_up() {
    let quiet = Driver::new(Ticker(false)).idle_wakes(600_000);
    assert_eq!(quiet, 11, "the first frame, then the clock's minute");
    let busy = Driver::new(Ticker(true)).idle_wakes(600_000);
    assert!(busy >= 600, "{busy}");
}
