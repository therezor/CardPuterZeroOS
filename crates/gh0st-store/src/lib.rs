//! The Store: CardputerZero apps from M5's catalog, mirrored and signed by
//! our CI. The app shows and asks; `gh0st store`, as root, downloads,
//! checks and installs. docs/store.md has the whole design.

// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
mod pics;
mod shots;
pub mod words;
