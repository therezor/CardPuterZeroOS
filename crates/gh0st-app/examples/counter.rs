//! Chips: actions over the number keys 5 to 7, with the menu on 8.
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
        Bars::top()
            .chip(5, Chip::new("minus", "Less"))
            .chip(6, Chip::new("rotate-cw", "Reset").danger())
            .chip(7, Chip::new("plus", "More"))
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        match key {
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
