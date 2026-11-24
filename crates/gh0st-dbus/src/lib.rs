//! D-Bus for gh0st: the wire format, SASL EXTERNAL, and a connection that
//! fits one `poll` loop. No threads, no async runtime.
//!
//! Every length and depth the spec bounds is checked on the way in, so a
//! hostile peer gets an error, never a panic or a huge allocation. Byte
//! buffers are zeroed as they are consumed: secrets pass through here.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod address;
pub mod auth;
pub mod bus;
pub mod conn;
pub mod message;
pub mod names;
pub mod objects;
pub mod sig;
pub mod value;
mod wire;

pub use conn::Conn;
pub use message::{Kind, Message};
pub use sig::Ty;
pub use value::Value;

use std::fmt;

/// The largest message we take or make. The spec allows 128 MiB; nothing
/// we serve needs more than a few KiB.
pub const MAX_MESSAGE: usize = 1 << 20;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The peer broke the protocol, or a message we built is not valid.
    Invalid(&'static str),
    /// An error reply.
    Remote {
        name: String,
        text: String,
    },
    Timeout,
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Invalid(why) => write!(f, "invalid: {why}"),
            Error::Remote { name, text } => write!(f, "{name}: {text}"),
            Error::Timeout => f.write_str("timed out"),
            Error::Closed => f.write_str("connection closed"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Overwrites a buffer in a way the optimiser keeps.
pub fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: a volatile write through a valid &mut u8.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
