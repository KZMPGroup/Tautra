mod image;

#[cfg(windows)]
mod data_object;
#[cfg(windows)]
mod formats;
#[cfg(windows)]
mod global;
#[cfg(windows)]
mod read;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod stream;
#[cfg(windows)]
mod target;

pub use image::{dib_to_png, png_to_dib, ImageError};

#[cfg(windows)]
pub use data_object::{create_data_object, create_drag_object, read_dword, DropFiles};
#[cfg(windows)]
pub use target::shell_target;

#[cfg(windows)]
pub use service::{
    ClipboardError, ClipboardHandler, ClipboardOptions, ClipboardService, LocalContent,
    LocalImage, RemoteContent, RemoteFile, RemoteFiles, RemoteItem,
};
