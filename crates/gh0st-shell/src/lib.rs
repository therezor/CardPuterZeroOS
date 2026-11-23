//! The shell: the home screen and everything it owns while no app is open.

// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod hold;
pub mod home;
pub mod lock;
pub mod motion;
pub mod notices;
pub mod overlay;
pub mod run;
pub mod search;
pub mod switcher;
pub mod tasks;
