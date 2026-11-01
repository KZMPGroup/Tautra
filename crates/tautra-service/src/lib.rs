#![cfg(windows)]

pub mod client;
pub mod install;
pub mod pipe;
pub mod protocol;
pub mod service;
pub mod worker;

pub use protocol::{
    ControlReply, ControlRequest, ServiceInput, ServiceState, INSTALL_ARG, RESTART_ARG,
    SERVICE_ARG, UNINSTALL_ARG, WORKER_ARG,
};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Pipe(String),
    #[error("{0}")]
    Scm(String),
    #[error("layanan Tautra belum berjalan")]
    NotRunning,
    #[error("{0}")]
    Other(String),
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
