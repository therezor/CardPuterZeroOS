//! The smallest app: a title, two lines of text, Esc to leave.
//!
//!     cargo run -p gh0st-app --example hello -- --sim /tmp/hello < /dev/null

use gh0st_app::prelude::*;

struct Hello;

impl Page for Hello {
    fn title(&self) -> &str {
        "Hello"
    }

    fn draw(&self, ui: &mut Ui) {
        ui.heading("Hello, CardputerZero");
        ui.text("This page has the OS top bar. Press Esc to leave.");
    }
}

fn main() {
    gh0st_app::run(Hello);
}
