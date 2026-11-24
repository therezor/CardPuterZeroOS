//! Media: pictures, video and music. This app is the screen and the keys;
//! `gh0st-mediad`, a C++ helper it starts, decodes every file, so one that
//! crashes a decoder never takes the app down. They talk over a socket pair
//! and share pictures through a frame ring. The design is in docs/media.md.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
pub mod fake;
pub mod files;
pub mod helper;
pub mod mpris;
pub mod proto;
pub mod resume;
