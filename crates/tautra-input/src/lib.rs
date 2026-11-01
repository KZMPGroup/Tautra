#![cfg(windows)]

pub mod cursor;
pub mod display;
pub mod elevation;
pub mod fullscreen;
pub mod geometry;
pub mod guide;
pub mod hider;
pub mod hook;
pub mod inject;

pub use geometry::{DeviceGeometry, Geometry, Side};
pub use guide::GuideOverlay;
pub use hider::CursorHider;
pub use hook::{
    ActiveSide, DragCheck, HookController, HookOptions, HookStatus, Hotkeys, InputSink,
};
pub use inject::{InjectError, Injector};

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("hook input gagal dipasang: {0}")]
    Hook(String),
    #[error("thread input gagal dibuat: {0}")]
    Thread(String),
    #[error("daftar monitor gagal dibaca: {0}")]
    Display(String),
}
