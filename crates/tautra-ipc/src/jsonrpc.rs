use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: &str = "2.0";

pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const APP_ERROR: i64 = -32000;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message} (kode {code})")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub app_code: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn app(app_code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code::APP_ERROR,
            message: message.into(),
            data: Some(ErrorData {
                app_code: app_code.to_string(),
            }),
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            code::METHOD_NOT_FOUND,
            format!("method tidak dikenal: {method}"),
        )
    }

    pub fn invalid_params(err: impl std::fmt::Display) -> Self {
        Self::new(
            code::INVALID_PARAMS,
            format!("parameter tidak valid: {err}"),
        )
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::new(code::INTERNAL_ERROR, err.to_string())
    }

    pub fn app_code(&self) -> Option<&str> {
        self.data.as_ref().map(|d| d.app_code.as_str())
    }
}

#[derive(Debug, Clone)]
pub enum Incoming {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

pub fn parse(line: &str) -> Result<Incoming, RpcError> {
    let raw: RawMessage = serde_json::from_str(line)
        .map_err(|e| RpcError::new(code::PARSE_ERROR, format!("JSON tidak valid: {e}")))?;
    match (raw.method, raw.id) {
        (Some(method), Some(id)) => Ok(Incoming::Request(Request {
            jsonrpc: VERSION.into(),
            id,
            method,
            params: raw.params,
        })),
        (Some(method), None) => Ok(Incoming::Notification(Notification {
            jsonrpc: VERSION.into(),
            method,
            params: raw.params,
        })),
        (None, Some(id)) => Ok(Incoming::Response(Response {
            jsonrpc: VERSION.into(),
            id: Some(id),
            result: raw.result,
            error: raw.error,
        })),
        (None, None) => Err(RpcError::new(
            code::INVALID_REQUEST,
            "pesan tanpa method dan id",
        )),
    }
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: VERSION.into(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<u64>, error: RpcError) -> Self {
        Self {
            jsonrpc: VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: VERSION.into(),
            method: method.to_string(),
            params: Some(params),
        }
    }
}
