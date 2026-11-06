mod error;
mod manifest;
mod path;

#[cfg(windows)]
mod drag;
#[cfg(windows)]
mod fetch;
#[cfg(windows)]
mod probe;
#[cfg(windows)]
mod reveal;
#[cfg(windows)]
mod serve;

pub use error::TransferError;
pub use manifest::{build_manifest, Manifest, MAX_ENTRIES};
pub use path::safe_relative;

#[cfg(windows)]
pub use drag::{shell_data, DragEffect, DragLabel, Prepare};
#[cfg(windows)]
pub use fetch::{stream_file, BlockingReader};
#[cfg(windows)]
pub use probe::{DragOutcome, DragRequest, DropProbe, ProbeError, ProbeState};
#[cfg(windows)]
pub use reveal::reveal;
#[cfg(windows)]
pub use serve::{send_file, FileKey};

pub const CHUNK: usize = 1024 * 1024;
pub const RESET_SOURCE_FAILED: u32 = 2;
pub const RESET_CANCELLED: u32 = 3;
