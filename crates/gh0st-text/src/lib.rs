//! gh0st-text: a code and text editor for the CardputerZero. It opens files
//! of any size: the text stays on the card and only what is on screen is
//! read, lexed and drawn.

// Release builds abort on a panic, and here a panic loses the user's edits.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
pub mod buf;
pub mod find;
pub mod lex;
pub mod view;
pub mod watch;
