use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tautra_common::config::Config;
use tautra_ipc::api::{
    app_error, method, notification, AgentStatus, ConnectionStatus, Notice, PairingStatus,
    TransferDirection, TransferState, TransferView, UpdateAppliedParams,
};
use tautra_ipc::pipe::{IpcClient, IpcError};
use tautra_ipc::PIPE_NAME;

use crate::i18n::{t, with, with_name, Key};
use crate::{autostart, tray};

pub const SNAPSHOT_EVENT: &str = "agent://snapshot";
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const SPAWN_COOLDOWN: Duration = Duration::from_secs(10);
const ERROR_FILE_NOT_FOUND: i32 = 2;
const TRANSFER_TOAST_AFTER_MS: u64 = 2000;
const RESTART_DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Serialize)]
pub struct LinkSnapshot {
    pub connected: bool,
    pub status: Option<AgentStatus>,
    pub config: Option<Config>,
    pub error: Option<String>,
}

#[derive(Default)]
struct LinkState {
    client: Option<IpcClient>,
    status: Option<AgentStatus>,
    config: Option<Config>,
    error: Option<String>,
    announced: HashSet<u64>,
    settled: HashSet<u64>,
    seen_notices: HashSet<u64>,
}

#[derive(Clone)]
pub struct AgentLink {
    app: AppHandle,
    state: Arc<Mutex<LinkState>>,
    quitting: Arc<AtomicBool>,
}

impl AgentLink {
    pub fn start(app: AppHandle) -> Self {
        let link = Self {
            app,
            state: Arc::default(),
            quitting: Arc::default(),
        };
        let worker = link.clone();
        tauri::async_runtime::spawn(async move { worker.run().await });
        link
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state.lock().expect("link lock")
    }

    pub fn snapshot(&self) -> LinkSnapshot {
        let s = self.lock();
        LinkSnapshot {
            connected: s.client.is_some(),
            status: s.status.clone(),
            config: s.config.clone(),
            error: s.error.clone(),
        }
    }

    pub async fn call(&self, name: &str, params: Value) -> Result<Value, IpcError> {
        let client = self.lock().client.clone().ok_or(IpcError::Disconnected)?;
        let result = client.call_value(name, params, CALL_TIMEOUT).await;
        if name == method::CONFIG_SET {
            if let Ok(value) = &result {
                if let Ok(config) = serde_json::from_value::<Config>(value.clone()) {
                    self.set_config(config);
                }
            }
        }
        result
    }

    pub fn begin_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    fn quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    pub fn ensure_agent(&self) -> Result<(), String> {
        if self.quitting() {
            return Ok(());
        }
        if self.lock().client.is_some() {
            return Ok(());
        }
        spawn_agent()
    }

    async fn run(self) {
        let mut last_spawn: Option<Instant> = None;
        loop {
            match IpcClient::connect(PIPE_NAME).await {
                Ok((client, mut notes)) => {
                    tracing::info!("tersambung ke agent");
                    {
                        let mut s = self.lock();
                        s.client = Some(client.clone());
                        s.error = None;
                    }
                    self.publish();
                    match client
                        .call::<_, Config>(method::CONFIG_GET, Value::Null, CALL_TIMEOUT)
                        .await
                    {
                        Ok(config) => self.set_config(config),
                        Err(e) => tracing::warn!("gagal membaca konfigurasi dari agent: {e}"),
                    }
                    match client
                        .call::<_, AgentStatus>(method::STATUS_GET, Value::Null, CALL_TIMEOUT)
                        .await
                    {
                        Ok(status) => self.set_status(status),
                        Err(e) => tracing::warn!("gagal membaca status agent: {e}"),
                    }
                    while let Some(note) = notes.recv().await {
                        if note.method == notification::UPDATE_APPLIED {
                            let params = note
                                .params
                                .clone()
                                .and_then(|p| serde_json::from_value::<UpdateAppliedParams>(p).ok());
                            if params.is_some_and(|p| p.ui) {
                                self.restart_later();
                            }
                            continue;
                        }
                        if note.method != notification::STATUS_CHANGED {
                            continue;
                        }
                        match note.params.map(serde_json::from_value::<AgentStatus>) {
                            Some(Ok(status)) => self.set_status(status),
                            Some(Err(e)) => tracing::warn!("status agent tidak valid: {e}"),
                            None => {}
                        }
                    }
                    tracing::warn!("koneksi ke agent terputus");
                    {
                        let mut s = self.lock();
                        s.client = None;
                        s.status = None;
                        s.error = Some("Koneksi ke agent terputus".into());
                    }
                    self.publish();
                }
                Err(e) => {
                    let not_running = e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND);
                    if not_running && !self.quitting() {
                        if last_spawn.is_none_or(|t| t.elapsed() > SPAWN_COOLDOWN) {
                            last_spawn = Some(Instant::now());
                            if let Err(err) = spawn_agent() {
                                tracing::error!("agent tidak bisa dijalankan: {err}");
                                self.set_error(err);
                            }
                        }
                    } else if !not_running {
                        self.set_error(format!("Pipe agent tidak bisa dibuka: {e}"));
                    }
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }

    fn restart_later(&self) {
        let app = self.app.clone();
        self.toast(&t(Key::ToastUpdatedTitle), &t(Key::ToastUpdatedBody));
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(RESTART_DELAY).await;
            tracing::info!("UI dijalankan ulang setelah pembaruan");
            app.restart();
        });
    }

    fn set_error(&self, error: String) {
        let changed = {
            let mut s = self.lock();
            let changed = s.error.as_deref() != Some(error.as_str());
            s.error = Some(error);
            changed
        };
        if changed {
            self.publish();
        }
    }

    fn set_status(&self, status: AgentStatus) {
        let previous = self.lock().status.replace(status.clone());
        self.notify_transitions(previous.as_ref(), &status);
        self.notify_transfers(previous.is_none(), &status.share.transfers);
        self.notify_failures(previous.is_none(), &status.notices);
        self.publish();
    }

    fn notify_failures(&self, initial: bool, notices: &[Notice]) {
        let fresh: Vec<Notice> = {
            let mut s = self.lock();
            let live: HashSet<u64> = notices.iter().map(|n| n.id).collect();
            s.seen_notices.retain(|id| live.contains(id));
            notices
                .iter()
                .filter(|n| s.seen_notices.insert(n.id))
                .filter(|_| !initial)
                .filter(|n| {
                    matches!(
                        n.code.as_str(),
                        app_error::CLIPBOARD_FAILED
                            | app_error::DRAGDROP_FAILED
                            | app_error::TRANSFER_FAILED
                    )
                })
                .cloned()
                .collect()
        };
        let Some(first) = fresh.first() else {
            return;
        };
        let mut body = first.message.clone();
        if fresh.len() > 1 {
            body.push_str(&with(Key::ToastMoreFailures, "count", &(fresh.len() - 1).to_string()));
        }
        self.toast(&t(Key::ToastFailedTitle), &body);
    }

    fn set_config(&self, config: Config) {
        let autostart_changed = {
            let mut s = self.lock();
            let changed =
                s.config.as_ref().map(|c| c.general.autostart) != Some(config.general.autostart);
            s.config = Some(config.clone());
            changed
        };
        if autostart_changed {
            autostart::sync(config.general.autostart);
        }
        self.publish();
    }

    fn publish(&self) {
        let snapshot = self.snapshot();
        tray::update(&self.app, &snapshot);
        let _ = self.app.emit(SNAPSHOT_EVENT, &snapshot);
    }

    fn notify_transitions(&self, previous: Option<&AgentStatus>, next: &AgentStatus) {
        let Some(previous) = previous else {
            return;
        };
        match (&previous.connection, &next.connection) {
            (ConnectionStatus::Connected(a), ConnectionStatus::Connected(b))
                if a.device_id == b.device_id => {}
            (_, ConnectionStatus::Connected(peer)) => self.toast(
                &t(Key::ToastConnectedTitle),
                &with_name(Key::ToastConnectedBody, &peer.device_name),
            ),
            (ConnectionStatus::Connected(peer), _) => self.toast(
                &t(Key::ToastDisconnectedTitle),
                &with_name(Key::ToastDisconnectedBody, &peer.device_name),
            ),
            _ => {}
        }
        if let PairingStatus::Succeeded { device_name, .. } = &next.pairing {
            if !matches!(previous.pairing, PairingStatus::Succeeded { .. }) {
                self.toast(
                    &t(Key::ToastPairedTitle),
                    &with_name(Key::ToastPairedBody, device_name),
                );
            }
        }
    }

    fn notify_transfers(&self, initial: bool, transfers: &[TransferView]) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default();
        let mut toasts = Vec::new();
        {
            let mut s = self.lock();
            let live: HashSet<u64> = transfers.iter().map(|t| t.id).collect();
            s.announced.retain(|id| live.contains(id));
            s.settled.retain(|id| live.contains(id));
            for transfer in transfers {
                let id = transfer.id;
                let active = matches!(transfer.state, TransferState::Running | TransferState::Waiting);
                if initial {
                    if active {
                        s.announced.insert(id);
                    } else {
                        s.settled.insert(id);
                    }
                    continue;
                }
                if active {
                    let elapsed = now.saturating_sub(transfer.started_unix_ms);
                    if elapsed >= TRANSFER_TOAST_AFTER_MS && s.announced.insert(id) {
                        toasts.push((transfer.clone(), true));
                    }
                    continue;
                }
                if s.settled.contains(&id) {
                    continue;
                }
                s.settled.insert(id);
                let long = transfer
                    .finished_unix_ms
                    .unwrap_or(now)
                    .saturating_sub(transfer.started_unix_ms)
                    >= TRANSFER_TOAST_AFTER_MS;
                let show = match transfer.state {
                    TransferState::Failed => true,
                    TransferState::Done => long || s.announced.contains(&id),
                    _ => false,
                };
                if show {
                    toasts.push((transfer.clone(), false));
                }
            }
        }
        for (transfer, started) in toasts {
            let (title, body) = if started {
                let title = match transfer.direction {
                    TransferDirection::Incoming => Key::ToastIncomingTitle,
                    TransferDirection::Outgoing => Key::ToastOutgoingTitle,
                };
                (title, Key::ToastTransferRunning)
            } else if transfer.state == TransferState::Failed {
                (Key::ToastTransferFailedTitle, Key::ToastTransferFailed)
            } else {
                (Key::ToastTransferDoneTitle, Key::ToastTransferDone)
            };
            let mut text = with_name(body, &transfer.name);
            if let Some(error) = transfer.error.as_deref().filter(|_| !started) {
                text.push_str(": ");
                text.push_str(error);
            }
            self.toast(&t(title), &text);
        }
    }

    fn toast(&self, title: &str, body: &str) {
        if let Err(e) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!("notifikasi gagal ditampilkan: {e}");
        }
    }
}

fn spawn_agent() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe()
        .map_err(|e| format!("lokasi UI tidak diketahui: {e}"))?
        .with_file_name("tautra-agent.exe");
    if !exe.is_file() {
        return Err(format!(
            "{} tidak ditemukan. Build agent dulu (cargo build -p tautra-agent).",
            exe.display()
        ));
    }
    std::process::Command::new(&exe)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .map(|_| tracing::info!(exe = %exe.display(), "agent dijalankan"))
        .map_err(|e| format!("gagal menjalankan {}: {e}", exe.display()))
}
