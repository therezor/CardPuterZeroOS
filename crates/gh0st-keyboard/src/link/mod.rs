//! The ways reports reach a host: Bluetooth LE and a USB cable. Each joins
//! the app's poll loop through its fds and tells the pages what happened
//! as events.

pub mod ble;
pub mod fake;
pub mod gatt;

use std::os::fd::RawFd;

use crate::hid::report::Report;
pub use ble::Advert;

/// A Bluetooth device BlueZ knows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// BlueZ's object path, `/org/bluez/hci0/dev_..`.
    pub path: String,
    pub address: String,
    pub name: String,
    pub icon: String,
    pub modalias: String,
    pub paired: bool,
    pub connected: bool,
}

/// What a pairing host asks of the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Both show this code: Enter if they match.
    Confirm(u32),
    /// Type this code on the host.
    Show(u32),
    /// Type the code the host shows, here.
    Enter,
    /// No code: pair with this host?
    Allow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// BlueZ took the keyboard's services: the link can pair and connect.
    Ready,
    /// The adapter's power, on or off.
    Power(bool),
    /// The list of devices changed.
    Devices,
    Connected(String),
    Disconnected(String),
    /// The connected host listens for key reports, or stopped.
    Listening(bool),
    /// The host's keyboard lights: bit 0 Num Lock, 1 Caps Lock.
    Leds(u8),
    /// A host is pairing and asks this.
    Pairing {
        device: String,
        ask: Ask,
    },
    /// The host the user paired is bonded.
    Paired(String),
    /// The host gave up pairing.
    PairCancelled,
    /// The advert ran out: no host came.
    AdvertEnded,
    /// The link can't go on. Why, in words for the user.
    Failed(String),
    /// The USB keyboard's state changed.
    Usb(UsbState),
}

/// What the pages ask of Bluetooth. [`ble::Ble`] is BlueZ; [`fake::FakeBt`]
/// stands in for tests and mockups.
pub trait Bluetooth {
    /// The fd to watch, while the link is alive.
    fn fd(&self) -> Option<RawFd>;
    fn ready(&mut self, out: &mut Vec<Event>);
    /// The name hosts see while pairing.
    fn name(&self) -> &str;
    fn powered(&self) -> bool;
    fn devices(&self) -> &[Device];
    /// The host connected now, among those given to `set_hosts`.
    fn connected(&self) -> Option<&str>;
    fn listening(&self) -> bool;
    /// BlueZ has answered: the adapter and its power are known.
    fn known(&self) -> bool;
    fn set_hosts(&mut self, paths: Vec<String>);
    fn wanted(&self) -> Advert;
    fn asking(&self) -> Option<Ask>;
    fn power(&mut self, on: bool);
    fn pair(&mut self, on: bool);
    fn reconnect(&mut self, on: bool);
    fn answer(&mut self, yes: bool);
    fn passkey(&mut self, code: u32);
    fn disconnect(&mut self, device: &str);
    fn forget(&mut self, device: &str);
    fn send(&mut self, r: &Report) -> bool;
    fn battery(&mut self, percent: u8);
    fn stop(&mut self);
}

impl Bluetooth for ble::Ble {
    fn fd(&self) -> Option<RawFd> {
        Some(ble::Ble::fd(self))
    }
    fn ready(&mut self, out: &mut Vec<Event>) {
        ble::Ble::ready(self, out)
    }
    fn name(&self) -> &str {
        ble::Ble::name(self)
    }
    fn powered(&self) -> bool {
        ble::Ble::powered(self)
    }
    fn devices(&self) -> &[Device] {
        ble::Ble::devices(self)
    }
    fn connected(&self) -> Option<&str> {
        ble::Ble::connected(self)
    }
    fn listening(&self) -> bool {
        ble::Ble::listening(self)
    }
    fn known(&self) -> bool {
        ble::Ble::known(self)
    }
    fn set_hosts(&mut self, paths: Vec<String>) {
        ble::Ble::set_hosts(self, paths)
    }
    fn wanted(&self) -> Advert {
        ble::Ble::wanted(self)
    }
    fn asking(&self) -> Option<Ask> {
        ble::Ble::asking(self)
    }
    fn power(&mut self, on: bool) {
        ble::Ble::power(self, on)
    }
    fn pair(&mut self, on: bool) {
        ble::Ble::pair(self, on)
    }
    fn reconnect(&mut self, on: bool) {
        ble::Ble::reconnect(self, on)
    }
    fn answer(&mut self, yes: bool) {
        ble::Ble::answer(self, yes)
    }
    fn passkey(&mut self, code: u32) {
        ble::Ble::passkey(self, code)
    }
    fn disconnect(&mut self, device: &str) {
        ble::Ble::disconnect(self, device)
    }
    fn forget(&mut self, device: &str) {
        ble::Ble::forget(self, device)
    }
    fn send(&mut self, r: &Report) -> bool {
        ble::Ble::send(self, r)
    }
    fn battery(&mut self, percent: u8) {
        ble::Ble::battery(self, percent)
    }
    fn stop(&mut self) {
        ble::Ble::stop(self)
    }
}

/// Where the USB cable stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbState {
    /// The USB keyboard is off.
    Off,
    /// Turning on: the helper builds the gadget.
    Starting,
    /// The side switch is on Host: the port can't be a keyboard.
    HostMode,
    /// On, waiting for a cable to a computer or phone.
    NoCable,
    /// A host took the keyboard.
    Connected,
    /// The host is asleep.
    Asleep,
    /// It could not start. The helper said why.
    Failed,
}

/// What the pages ask of the USB keyboard.
pub trait Cable {
    fn fds(&self) -> Vec<RawFd>;
    fn ready(&mut self, fd: RawFd, out: &mut Vec<Event>);
    fn state(&self) -> UsbState;
    /// Turns the USB keyboard on or off.
    fn enable(&mut self, on: bool);
    fn send(&mut self, r: &Report) -> bool;
    fn stop(&mut self);
}

/// The USB keyboard where there is none yet: always off.
pub struct NoCable;

impl Cable for NoCable {
    fn fds(&self) -> Vec<RawFd> {
        Vec::new()
    }
    fn ready(&mut self, _: RawFd, _: &mut Vec<Event>) {}
    fn state(&self) -> UsbState {
        UsbState::Off
    }
    fn enable(&mut self, _: bool) {}
    fn send(&mut self, _: &Report) -> bool {
        false
    }
    fn stop(&mut self) {}
}
