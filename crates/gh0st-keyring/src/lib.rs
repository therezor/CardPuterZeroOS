//! The Keyring app: what the keyring holds, grouped by the app that stored
//! it, the Wi-Fi passwords, and which apps may read others' secrets. A
//! plain Secret Service client: a secret shows only after the shell asks
//! for the account password.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod app;
pub mod backend;
