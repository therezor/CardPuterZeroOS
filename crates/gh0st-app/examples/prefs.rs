//! A settings list: a toggle, a choice, a link to a second page. The values
//! live in the shared device settings.
//!
//!     printf 'x\nc\nx\nenter\n' | cargo run -p gh0st-app --example prefs -- --sim /tmp/prefs

use gh0st_app::prelude::*;

const SPEEDS: [&str; 3] = ["Slow", "Normal", "Fast"];

struct Prefs {
    list: List,
    sounds: bool,
    speed: usize,
}

impl Page for Prefs {
    fn title(&self) -> &str {
        "Preferences"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints([("Enter", "open"), ("Esc", "back")])
    }

    fn start(&mut self, cx: &mut Cx) {
        self.sounds = cx.settings().get_bool("prefs.sounds").unwrap_or(true);
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.list.key(key) {
            Some(Action::Open(0) | Action::Change(0, _)) => {
                self.sounds = !self.sounds;
                cx.settings().set_bool("prefs.sounds", self.sounds);
            }
            Some(Action::Change(1, step)) => {
                self.speed = (self.speed as i32 + step).clamp(0, 2) as usize;
            }
            Some(Action::Open(2)) => cx.push(About),
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.list(
            &self.list,
            &[
                Row::toggle("Key sounds", self.sounds).icon("volume-2"),
                Row::choice("Speed", SPEEDS[self.speed]).icon("timer"),
                Row::link("About").icon("info"),
            ],
        );
    }
}

struct About;

impl Page for About {
    fn title(&self) -> &str {
        "About"
    }

    fn draw(&self, ui: &mut Ui) {
        ui.section("This device");
        ui.readout("Kernel", &gh0st_app::system::kernel().unwrap_or_default());
        ui.readout("SDK", gh0st_app::system::version());
    }
}

fn main() {
    gh0st_app::run(Prefs {
        list: List::new(),
        sounds: true,
        speed: 1,
    });
}
