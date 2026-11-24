//! `gh0st session`: the freedesktop session services apps expect, mapped
//! onto what gh0st already has. Notifications become notice files the
//! shell toasts; a ScreenSaver inhibit becomes the app's `awake`.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod inhibit;
pub mod memory;
pub mod monitor;
pub mod notify;
pub mod portal;
pub mod run;
pub mod settings;
