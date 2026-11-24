//! Links that need no radio and no cable: they hold whatever state a test
//! or a mockup gives them and keep every report sent.

use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

use super::{Advert, Ask, Bluetooth, Cable, Device, Event, UsbState};
use crate::hid::report::Report;

/// What a fake link was asked to do, kept where a test can read it after
/// the app took the link.
#[derive(Default, Debug)]
pub struct Log {
    pub sent: Vec<Report>,
    pub answers: Vec<bool>,
    pub disconnects: Vec<String>,
}

pub type SharedLog = Rc<RefCell<Log>>;

#[derive(Default)]
pub struct FakeBt {
    pub log: SharedLog,
    pub name: String,
    pub powered: bool,
    pub devices: Vec<Device>,
    pub connected: Option<String>,
    pub listening: bool,
    pub wanted: Option<Advert>,
    pub asking: Option<Ask>,
    pub pairing: bool,
    pub battery: Option<u8>,
    pub stopped: bool,
    /// Events the next `ready` hands over.
    pub queue: Vec<Event>,
}

impl FakeBt {
    pub fn new() -> FakeBt {
        FakeBt {
            name: "zero".into(),
            powered: true,
            ..FakeBt::default()
        }
    }

    /// A bonded host, connected or not.
    pub fn host(mut self, path: &str, name: &str, icon: &str, connected: bool) -> FakeBt {
        let address = path.rsplit("dev_").next().unwrap_or("").replace('_', ":");
        self.devices.push(Device {
            path: path.into(),
            address,
            name: name.into(),
            icon: icon.into(),
            modalias: String::new(),
            paired: true,
            connected,
        });
        if connected {
            self.connected = Some(path.into());
            self.listening = true;
        }
        self
    }
}

impl Bluetooth for FakeBt {
    fn fd(&self) -> Option<RawFd> {
        None
    }
    fn ready(&mut self, out: &mut Vec<Event>) {
        out.append(&mut self.queue);
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn powered(&self) -> bool {
        self.powered
    }
    fn devices(&self) -> &[Device] {
        &self.devices
    }
    fn connected(&self) -> Option<&str> {
        self.connected.as_deref()
    }
    fn listening(&self) -> bool {
        self.listening
    }
    fn known(&self) -> bool {
        true
    }
    fn set_hosts(&mut self, _: Vec<String>) {}
    fn wanted(&self) -> Advert {
        self.wanted.unwrap_or(Advert::Off)
    }
    fn asking(&self) -> Option<Ask> {
        self.asking
    }
    fn power(&mut self, on: bool) {
        self.powered = on;
    }
    fn pair(&mut self, on: bool) {
        self.pairing = on;
        self.wanted = Some(if on { Advert::Pair } else { Advert::Off });
    }
    fn reconnect(&mut self, on: bool) {
        self.wanted = Some(if on { Advert::Reconnect } else { Advert::Off });
    }
    fn answer(&mut self, yes: bool) {
        self.log.borrow_mut().answers.push(yes);
        self.asking = None;
    }
    fn passkey(&mut self, _: u32) {
        self.log.borrow_mut().answers.push(true);
        self.asking = None;
    }
    fn disconnect(&mut self, device: &str) {
        self.log.borrow_mut().disconnects.push(device.into());
        if self.connected.as_deref() == Some(device) {
            self.connected = None;
            self.listening = false;
        }
        if let Some(d) = self.devices.iter_mut().find(|d| d.path == device) {
            d.connected = false;
        }
    }
    fn forget(&mut self, device: &str) {
        self.devices.retain(|d| d.path != device);
    }
    fn send(&mut self, r: &Report) -> bool {
        if self.connected.is_none() {
            return false;
        }
        self.log.borrow_mut().sent.push(*r);
        true
    }
    fn battery(&mut self, percent: u8) {
        self.battery = Some(percent);
    }
    fn stop(&mut self) {
        self.stopped = true;
    }
}

pub struct FakeCable {
    pub state: UsbState,
    pub sent: Vec<Report>,
}

impl FakeCable {
    pub fn new(state: UsbState) -> FakeCable {
        FakeCable {
            state,
            sent: Vec::new(),
        }
    }
}

impl Cable for FakeCable {
    fn fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
    fn ready(&mut self, _: RawFd, _: &mut Vec<Event>) {}
    fn state(&self) -> UsbState {
        self.state
    }
    fn enable(&mut self, on: bool) {
        self.state = if on { UsbState::NoCable } else { UsbState::Off };
    }
    fn send(&mut self, r: &Report) -> bool {
        if self.state != UsbState::Connected {
            return false;
        }
        self.sent.push(*r);
        true
    }
    fn stop(&mut self) {
        self.state = UsbState::Off;
    }
}
