//! Soft keys: the number keys 4 to 8 under the screen, labelled in the
//! bottom bar. No top bar, so the page gets those rows too.
//!
//!     printf '7\n7\n5\n' | cargo run -p gh0st-app --example counter -- --sim /tmp/counter

use gh0st_app::prelude::*;

struct Counter {
    n: i32,
}

impl Page for Counter {
    fn title(&self) -> &str {
        "Counter"
    }

    fn bars(&self) -> Bars {
        Bars::none().softkeys([
            SoftKey::new("arrow-left", "Back"),
            SoftKey::label("Less"),
            SoftKey::label("Reset"),
            SoftKey::label("More"),
            SoftKey::none(),
        ])
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Soft(4) => cx.back(),
            Key::Soft(5) | Key::Down => self.n -= 1,
            Key::Soft(6) => self.n = 0,
            Key::Soft(7) | Key::Up => self.n += 1,
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.big(&self.n.to_string());
    }
}

fn main() {
    gh0st_app::run(Counter { n: 0 });
}
