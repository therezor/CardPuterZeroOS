//! Everything below the pixels: config, apps, input, policies, the event
//! loop and child processes. The pure parts build and test on any host.

pub mod apps;
pub mod backlight;
pub mod battery;
pub mod child;
pub mod config;
pub mod desktop;
pub mod esc;
pub mod evdev;
pub mod keys;
pub mod rt;
pub mod script;
pub mod status;
