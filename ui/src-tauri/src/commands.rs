use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tautra_ipc::api::method;
use tautra_ipc::pipe::IpcError;

use crate::agent_link::{AgentLink, LinkSnapshot};
use crate::window;

const SHOW_FALLBACK: Duration = Duration::from_secs(3);

pub struct UiState {
    mica: bool,
    start_hidden: bool,
    shown: AtomicBool,
}

impl UiState {
    pub fn new(mica: bool, start_hidden: bool) -> Self {
        Self {
            mica,
            start_hidden,
            shown: AtomicBool::new(false),
        }
    }

    fn show_once(&self, app: &AppHandle) {
        if !self.start_hidden && !self.shown.swap(true, Ordering::SeqCst) {
            window::show_main(app);
        }
    }
}

pub fn schedule_show_fallback(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_FALLBACK).await;
        if let Some(state) = app.try_state::<UiState>() {
            state.show_once(&app);
        }
    });
}

#[derive(Debug, Serialize)]
pub struct UiError {
    pub code: String,
    pub message: String,
}

impl From<IpcError> for UiError {
    fn from(err: IpcError) -> Self {
        let code = match &err {
            IpcError::Rpc(rpc) => rpc.app_code().unwrap_or("rpc").to_string(),
            IpcError::Disconnected => "agent.offline".into(),
            IpcError::Timeout(_) => "agent.timeout".into(),
            IpcError::Decode(_) | IpcError::Encode(_) => "internal".into(),
        };
        let message = match err {
            IpcError::Rpc(rpc) => rpc.message,
            other => other.to_string(),
        };
        Self { code, message }
    }
}

#[derive(Debug, Serialize)]
pub struct UiInfo {
    pub mica: bool,
    pub version: String,
}

#[tauri::command]
pub async fn agent_call(
    link: State<'_, AgentLink>,
    method: String,
    params: Option<Value>,
) -> Result<Value, UiError> {
    link.call(&method, params.unwrap_or(Value::Null))
        .await
        .map_err(UiError::from)
}

#[tauri::command]
pub fn agent_snapshot(link: State<'_, AgentLink>) -> LinkSnapshot {
    link.snapshot()
}

#[tauri::command]
pub fn start_agent(link: State<'_, AgentLink>) -> Result<(), UiError> {
    link.ensure_agent().map_err(|message| UiError {
        code: "agent.spawn_failed".into(),
        message,
    })
}

#[tauri::command]
pub fn ui_info(state: State<'_, UiState>) -> UiInfo {
    UiInfo {
        mica: state.mica,
        version: tautra_common::build::version_string(),
    }
}

#[tauri::command]
pub fn ui_ready(app: AppHandle, state: State<'_, UiState>) {
    state.show_once(&app);
}

#[tauri::command]
pub fn open_folder(path: String, create: Option<bool>) -> Result<(), UiError> {
    let dir = PathBuf::from(&path);
    if create.unwrap_or(false) && !dir.is_dir() && dir.is_absolute() {
        std::fs::create_dir_all(&dir).map_err(|e| UiError {
            code: "folder.open_failed".into(),
            message: format!("folder {path} tidak bisa dibuat: {e}"),
        })?;
    }
    if !dir.is_dir() {
        return Err(UiError {
            code: "folder.not_found".into(),
            message: format!("folder tidak ditemukan: {path}"),
        });
    }
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| UiError {
            code: "folder.open_failed".into(),
            message: e.to_string(),
        })
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    quit(app);
}

pub fn quit(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let link = app.state::<AgentLink>().inner().clone();
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            link.call(method::AGENT_SHUTDOWN, Value::Null),
        )
        .await;
        app.exit(0);
    });
}
