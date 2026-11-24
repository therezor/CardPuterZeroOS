//! Camera: photos and video from the IMX219. This app is the screen and the
//! keys; `gh0st-camd`, a C++ helper it starts, owns libcamera and the
//! encoders. They talk over a socket pair and share preview frames through
//! a ring in shared memory. The design is in docs/camera.md.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
pub mod fake;
pub mod files;
pub mod helper;
pub mod proto;
