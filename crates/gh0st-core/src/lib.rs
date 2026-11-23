//! Everything below the pixels: config, apps, input, policies, the event
//! loop and child processes. The pure parts build and test on any host.

// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod apps;
pub mod backlight;
pub mod battery;
pub mod child;
pub mod config;
pub mod desktop;
pub mod esc;
pub mod evdev;
pub mod hotspot;
pub mod job;
pub mod keys;
pub mod link;
pub mod notify;
pub mod password;
pub mod procfs;
pub mod rfkill;
pub mod rt;
pub mod script;
pub mod search;
pub mod status;
pub mod sysconf;
pub mod update;
pub mod usage;
