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

pub use image::{dib_to_png, png_to_dib, ImageError};

#[cfg(windows)]
pub use data_object::{create_data_object, read_dword};

#[cfg(windows)]
pub use service::{
    ClipboardError, ClipboardHandler, ClipboardOptions, ClipboardService, LocalContent,
    LocalImage, RemoteContent, RemoteFile, RemoteFiles, RemoteItem,
};
