//! Updates: shows what the update engine (`gh0st update`, root, run by
//! systemd) found and how far it is, and asks it to check, install or stop
//! through gh0st-helper. It never runs apt itself, so closing it never
//! stops an install. The design is in docs/updates.md.

pub mod app;
pub mod packages;
pub mod words;
