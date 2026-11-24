//! `gh0st keyring`: the system keyring. It holds the vault, serves the
//! freedesktop Secret Service on the session bus, and takes unlock, lock
//! and prompt answers from the shell over a socket pair. `docs/keyring.md`
//! has the model and its limits.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod db;
pub mod secret;
pub mod secure;
pub mod vault;
