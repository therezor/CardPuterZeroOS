//! The first page: the USB cable and every paired computer or phone.

use gh0st_app::prelude::*;

use crate::words;

pub struct Hosts {
    list: List,
}

impl Hosts {
    pub fn new() -> Hosts {
        Hosts { list: List::new() }
    }
}

impl Default for Hosts {
    fn default() -> Self {
        Hosts::new()
    }
}

impl Page for Hosts {
    fn title(&self) -> &str {
        words::APP
    }

    fn icon(&self) -> &str {
        "keyboard"
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        self.list.key(key);
    }

    fn draw(&self, ui: &mut Ui) {
        ui.notice("keyboard", None, words::NO_HOSTS, words::NO_HOSTS_NOTE, &[]);
    }
}
