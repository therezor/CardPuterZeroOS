//! USB HID: what the host is told the device is, and the reports it sends.
//! Bluetooth LE (HID over GATT) and a USB cable carry the same bytes.

pub mod desc;
pub mod layout;
pub mod report;
pub mod usage;
