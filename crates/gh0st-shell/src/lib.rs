//! The shell: the home screen and everything it owns while no app is open.

// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod askpass;
pub mod hold;
pub mod home;
pub mod lock;
pub mod motion;
pub mod notices;
pub mod overlay;
pub mod power;
pub mod prompt;
pub mod run;
pub mod services;
pub mod sound;
pub mod switcher;
pub mod tasks;
