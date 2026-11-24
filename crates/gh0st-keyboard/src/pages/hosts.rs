//! The first screen: the USB cable and every paired computer or phone.
//! Enter types on one; 5 pairs a new one.

use gh0st_app::prelude::*;

use crate::app::{Nav, Screen, Shared, Target};
use crate::host::System;
use crate::link::{Advert, Event, UsbState};
use crate::pages::{pair::Pair, profile::Profile, session::Session};
use crate::store;
use crate::words;

const PAIR: u32 = 1;
const EDIT: u32 = 2;
const UNLINK: u32 = 3;
const USB_ON: u32 = 4;
const FORGET: u32 = 5;

pub fn icon_of(system: System) -> &'static str {
    match system {
        System::Mac | System::ChromeOs => "laptop",
        System::Windows | System::Linux => "monitor",
        System::Android | System::Ios => "smartphone",
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Item {
    Usb,
    Host(String),
}

pub struct Hosts {
    list: List,
    /// Opens the session when the host used last comes back by itself,
    /// once, as the app starts.
    auto: bool,
}

impl Hosts {
    pub fn new() -> Hosts {
        Hosts {
            list: List::new(),
            auto: true,
        }
    }

    fn items(s: &Shared) -> Vec<Item> {
        let mut v = vec![Item::Usb];
        v.extend(s.hosts.iter().map(|h| Item::Host(h.address.clone())));
        v
    }

    fn selected(&self, s: &Shared) -> Item {
        Hosts::items(s)
            .into_iter()
            .nth(self.list.selected())
            .unwrap_or(Item::Usb)
    }

    fn selected_host(&self, s: &Shared) -> Option<String> {
        match self.selected(s) {
            Item::Host(a) => Some(a),
            Item::Usb => None,
        }
    }

    /// The host's state for its row: a word and a tone, if any.
    fn state(s: &Shared, address: &str) -> Option<(&'static str, Tone)> {
        let bt = s.bt.as_ref()?;
        let ours = s.target == Some(Target::Bt(address.to_string()));
        if s.device(address).is_some_and(|d| d.connected) {
            return Some((words::CONNECTED, Tone::Ok));
        }
        (ours && bt.wanted() == Advert::Reconnect).then_some((words::WAITING, Tone::Info))
    }

    fn open(&mut self, s: &mut Shared, item: Item, cx: &mut Cx) -> Nav {
        match item {
            Item::Usb => {
                if matches!(s.usb.state(), UsbState::Off | UsbState::Failed) {
                    s.usb.enable(true);
                }
                s.set_target(Target::Usb, cx);
            }
            Item::Host(a) => super::use_host(s, &a, cx),
        }
        self.auto = false;
        Nav::Push(Box::new(Session::new(s)))
    }

    fn unlink(s: &mut Shared, address: &str) {
        s.release();
        if let Some(bt) = &mut s.bt {
            bt.reconnect(false);
            bt.disconnect(&store::path_of(address));
        }
    }

    fn forget(&mut self, s: &mut Shared, address: &str, cx: &mut Cx) {
        if s.target == Some(Target::Bt(address.to_string())) {
            s.release();
            s.target = None;
        }
        if let Some(bt) = &mut s.bt {
            bt.forget(&store::path_of(address));
        }
        s.hosts.retain(|h| h.address != address);
        s.save_hosts();
        s.sync_hosts();
        self.list.select(0);
        cx.toast(words::FORGOT);
    }
}

impl Default for Hosts {
    fn default() -> Self {
        Hosts::new()
    }
}

impl Screen for Hosts {
    fn title(&self) -> &str {
        words::APP
    }

    fn context(&self) -> Option<&str> {
        None
    }

    fn bars(&self, s: &Shared) -> Bars {
        let pair = Chip::new("bluetooth", words::PAIR).runs(PAIR);
        let Some(a) = self.selected_host(s) else {
            return Bars::top().chip(6, pair);
        };
        let forget = Chip::new("trash-2", words::FORGET)
            .danger()
            .hold()
            .runs(FORGET);
        let b = Bars::top().chip(5, pair).chip(7, forget);
        if s.device(&a).is_some_and(|d| d.connected) {
            b.chip(6, Chip::new("link", words::UNLINK).runs(UNLINK))
        } else {
            b
        }
    }

    fn draw(&self, s: &Shared, ui: &mut Ui) {
        let usb = s.usb.state();
        let usb_row = if usb == UsbState::Connected {
            Row::tag(words::USB, words::CONNECTED, Tone::Ok)
        } else {
            Row::link(words::USB)
        };
        let mut rows = vec![usb_row.icon("usb").note(words::usb_note(usb))];
        let powered = s.bt.as_ref().is_some_and(|b| b.powered());
        for h in &s.hosts {
            let note = if s.bt.is_some() && !powered {
                words::BT_OFF
            } else {
                h.system.name()
            };
            let row = match Hosts::state(s, &h.address) {
                Some((word, tone)) => Row::tag(&h.name, word, tone),
                None => Row::link(&h.name),
            };
            rows.push(row.icon(icon_of(h.system)).note(note));
        }
        ui.list(&self.list, &rows);
        if let Some(why) = &s.bt_error {
            ui.note(&format!("{}: {why}", words::NO_BT));
        } else if s.hosts.is_empty() {
            ui.note(words::PAIR_NOTE);
        }
    }

    fn key(&mut self, s: &mut Shared, key: Key, cx: &mut Cx) -> Nav {
        match self.list.key(key) {
            Some(Action::Open(i)) => match Hosts::items(s).into_iter().nth(i) {
                Some(item) => self.open(s, item, cx),
                None => Nav::Stay,
            },
            _ => Nav::Stay,
        }
    }

    fn commands(&self, s: &Shared, c: &mut Commands) {
        let host = self.selected_host(s);
        let linked = host
            .as_deref()
            .is_some_and(|a| s.device(a).is_some_and(|d| d.connected));
        c.add(
            Command::action(PAIR, "Pair a computer or phone")
                .keys("p")
                .opens(),
        );
        c.add(
            Command::action(EDIT, "Host settings")
                .keys("e")
                .opens()
                .enabled(host.is_some()),
        );
        c.add(Command::action(UNLINK, words::UNLINK).enabled(linked));
        c.add(
            Command::action(FORGET, words::FORGET)
                .keys_only()
                .enabled(host.is_some()),
        );
        c.add(Command::toggle(
            USB_ON,
            "USB keyboard",
            s.usb.state() != UsbState::Off,
        ));
    }

    fn command(&mut self, s: &mut Shared, id: u32, _: Step, cx: &mut Cx) -> Nav {
        match id {
            PAIR => Nav::Push(Box::new(Pair::new())),
            EDIT => match self.selected_host(s) {
                Some(a) => Nav::Push(Box::new(Profile::new(s, &a, false))),
                None => Nav::Stay,
            },
            UNLINK => {
                if let Some(a) = self.selected_host(s) {
                    Hosts::unlink(s, &a);
                }
                Nav::Stay
            }
            FORGET => {
                if let Some(a) = self.selected_host(s) {
                    self.forget(s, &a, cx);
                }
                Nav::Stay
            }
            USB_ON => {
                let on = s.usb.state() == UsbState::Off;
                if !on && s.target == Some(Target::Usb) {
                    s.release();
                }
                s.usb.enable(on);
                let _ = cx;
                Nav::Stay
            }
            _ => Nav::Stay,
        }
    }

    fn event(&mut self, s: &mut Shared, e: &Event, _: &mut Cx) -> Nav {
        // The host used last came back on its own: straight to typing.
        let back = match e {
            Event::Listening(true) => matches!(s.target, Some(Target::Bt(_))),
            Event::Usb(UsbState::Connected) => s.target == Some(Target::Usb),
            _ => false,
        };
        if self.auto && back && s.live() {
            self.auto = false;
            return Nav::Push(Box::new(Session::new(s)));
        }
        Nav::Stay
    }
}
