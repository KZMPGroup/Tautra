#![cfg(windows)]

mod discovery;
mod error;
mod fingerprint;
mod framing;
mod identity;
mod knownnet;
mod manager;
mod netwatch;
mod pairing;
mod pathmem;
mod qos;
mod session;
mod tls;
mod trust;
mod update;

pub use error::NetError;
pub use fingerprint::Fingerprint;
pub use framing::{read_msg, write_msg, StreamError};
pub use manager::{start, NetCommand, NetEvent, NetHandle, NetOptions, NetStatus};
pub use session::SessionHandle;
