use tautra_ipc::api::app_error;
use tautra_ipc::jsonrpc::RpcError;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct NetError {
    pub code: &'static str,
    pub message: String,
}

impl NetError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(app_error::INTERNAL, message)
    }
}

impl From<NetError> for RpcError {
    fn from(err: NetError) -> Self {
        RpcError::app(err.code, err.message)
    }
}
