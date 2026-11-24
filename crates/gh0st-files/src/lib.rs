//! Files: the card, archives and servers on two sides of one pane
//! (`docs/files.md`).

// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
pub mod engine;
pub mod fake;
pub mod model;
pub mod servers;
pub mod words;
