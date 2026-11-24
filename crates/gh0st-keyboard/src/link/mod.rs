//! The ways reports reach a host: Bluetooth LE and a USB cable. Each joins
//! the app's poll loop through its fds and tells the pages what happened
//! as events.

pub mod ble;
pub mod gatt;

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
}
