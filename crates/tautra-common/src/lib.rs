#[cfg(windows)]
pub mod autostart;
pub mod build;
pub mod config;
#[cfg(windows)]
pub mod dpapi;
pub mod hotkey;
#[cfg(windows)]
pub mod installed;
pub mod locale;
pub mod logging;
pub mod paths;
pub mod time;

pub const APP_NAME: &str = "Tautra";

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
