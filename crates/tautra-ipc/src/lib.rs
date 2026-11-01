pub mod api;
pub mod jsonrpc;
#[cfg(windows)]
pub mod pipe;

pub const PIPE_NAME: &str = r"\\.\pipe\tautra-agent";

pub const MAX_MESSAGE_LEN: usize = 4 * 1024 * 1024;
