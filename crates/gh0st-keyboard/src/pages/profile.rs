//! One host's settings: its name, its system and whether it stays
//! connected behind other apps. It opens after pairing too, with "Start
//! typing" first, so Enter goes straight to typing.

use gh0st_app::prelude::*;

use crate::app::{Nav, Screen, Shared, Target};
use crate::host::System;
use crate::pages::{hosts::icon_of, session::Session};
use crate::words;

const RENAME: u32 = 1;

pub struct Profile {
    address: String,
    title: String,
    icon: &'static str,
    list: List,
    /// Just paired: says so above the rows.
    fresh: bool,
}

impl Profile {
    pub fn new(s: &Shared, address: &str, fresh: bool) -> Profile {
        let h = s.host(address);
        Profile {
            address: address.to_string(),
            title: h.map(|h| h.name.clone()).unwrap_or_default(),
            icon: icon_of(h.map(|h| h.system).unwrap_or_default()),
            list: List::new(),
            fresh,
        }
    }

    fn step_system(&mut self, s: &mut Shared, d: i32) {
        let Some(h) = s.host_mut(&self.address) else {
            return;
        };
        let all = System::ALL;
        let i = all.iter().position(|x| *x == h.system).unwrap_or(0);
        h.system = all[gh0st_app::cycle(i, d, all.len())];
        h.stay = !h.system.handheld();
        self.icon = icon_of(h.system);
        s.save_hosts();
        if s.target == Some(Target::Bt(self.address.clone())) {
            let o = s.engine.options();
            s.release();
            s.engine.set_options(crate::engine::Options {
                system: s.system(),
                ..o
            });
        }
    }
}

impl Screen for Profile {
    fn title(&self) -> &str {
        &self.title
    }

    fn about(&self) -> &str {
        words::PROFILE_ABOUT
    }

    fn icon(&self) -> &str {
        self.icon
    }

    fn bars(&self, _: &Shared) -> Bars {
        Bars::top().chip(6, Chip::new("trash-2", words::FORGET).danger().hold())
    }

    fn draw(&self, s: &Shared, ui: &mut Ui) {
        let Some(h) = s.host(&self.address) else {
            return;
        };
        if self.fresh {
            ui.note(&words::paired(&h.name));
        }
        let rows = [
            Row::link(words::START_TYPING).icon("keyboard"),
            Row::link(words::NAME).value(&h.name).icon("pencil"),
            Row::choice(words::SYSTEM, h.system.name()).icon(icon_of(h.system)),
            Row::toggle(words::STAY, h.stay).icon("link"),
        ];
        ui.list(&self.list, &rows);
    }

    fn key(&mut self, s: &mut Shared, key: Key, cx: &mut Cx) -> Nav {
        if key == Key::Soft(6) {
            let path = s.path_for(&self.address);
            if let Some(bt) = &mut s.bt {
                bt.forget(&path);
            }
            if s.target == Some(Target::Bt(self.address.clone())) {
                s.release();
                s.target = None;
            }
            s.hosts.retain(|h| h.address != self.address);
            s.save_hosts();
            s.sync_hosts();
            cx.toast(words::FORGOT);
            return Nav::Pop;
        }
        match self.list.key(key) {
            Some(Action::Open(0)) => {
                super::use_host(s, &self.address, cx);
                return Nav::Replace(Box::new(Session::new(s)));
            }
            Some(Action::Open(1)) => {
                let name = s
                    .host(&self.address)
                    .map(|h| h.name.clone())
                    .unwrap_or_default();
                cx.ask(
                    RENAME,
                    Dialog::text(words::NAME, "name").value(&name).yes("save"),
                );
            }
            Some(Action::Open(2)) => self.step_system(s, 1),
            Some(Action::Change(2, d)) => self.step_system(s, d),
            Some(Action::Open(3) | Action::Change(3, _)) => {
                if let Some(h) = s.host_mut(&self.address) {
                    h.stay = !h.stay;
                }
                s.save_hosts();
            }
            _ => {}
        }
        Nav::Stay
    }

    fn event(&mut self, s: &mut Shared, _: &crate::link::Event, _: &mut Cx) -> Nav {
        if let Some(new) = s.moved_to(&self.address) {
            self.address = new;
        }
        Nav::Stay
    }

    fn answer(&mut self, s: &mut Shared, id: u32, a: Answer, _: &mut Cx) -> Nav {
        if let (RENAME, Answer::Yes(name)) = (id, a) {
            let name = name.trim().to_string();
            if !name.is_empty()
                && let Some(h) = s.host_mut(&self.address)
            {
                h.name = name.clone();
                self.title = name;
                s.save_hosts();
            }
        }
        Nav::Stay
    }
}
