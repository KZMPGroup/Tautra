use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use tautra_common::config::{self, Config};
use tautra_common::time::unix_ms;
use tautra_common::{autostart, build, paths, APP_VERSION};
use tautra_ipc::api::{
    app_error, notification, AgentStatus, ConnectionStatus, LocalDevice, NetworkStatus,
    NoticeLevel, PairingStatus,
};
use tautra_ipc::jsonrpc::RpcError;
use tautra_ipc::pipe::notification_text;
use tautra_ipc::PIPE_NAME;
use tautra_net::{NetError, NetEvent, NetHandle, NetOptions};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

use crate::kvm::Kvm;
use crate::notices::NoticeBoard;
use crate::rpc::Rpc;
use crate::share::Share;

pub struct Agent {
    pub started_unix_ms: u64,
    pub config: Mutex<Config>,
    pub config_path: PathBuf,
    pub net: Option<NetHandle>,
    pub kvm: Kvm,
    pub share: Share,
    net_error: Option<NetError>,
    notices: Mutex<NoticeBoard>,
    changed: Notify,
    pub notify_tx: broadcast::Sender<Arc<str>>,
    pub cancel: CancellationToken,
    pub logon: bool,
    pub logon_changed: Notify,
}

impl Agent {
    pub fn notice(&self, level: NoticeLevel, code: &str, message: impl Into<String>) {
        self.notices
            .lock()
            .expect("notice lock")
            .push(level, code, message);
        self.changed.notify_one();
    }

    pub fn touch(&self) {
        self.changed.notify_one();
    }

    pub fn local_device_id(&self) -> Option<uuid::Uuid> {
        let net = self.net.as_ref()?;
        uuid::Uuid::parse_str(&net.status().local.device_id).ok()
    }

    pub fn dismiss_notice(&self, id: u64) -> bool {
        let removed = self.notices.lock().expect("notice lock").dismiss(id);
        if removed {
            self.changed.notify_one();
        }
        removed
    }

    pub fn config(&self) -> Config {
        self.config.lock().expect("config lock").clone()
    }

    pub fn net(&self) -> Result<&NetHandle, RpcError> {
        self.net.as_ref().ok_or_else(|| {
            let err = self
                .net_error
                .clone()
                .unwrap_or_else(|| NetError::internal("jaringan tidak berjalan"));
            RpcError::app(
                err.code,
                format!("Jaringan Tautra tidak berjalan: {}", err.message),
            )
        })
    }

    pub fn status(&self) -> AgentStatus {
        let notices = self.notices.lock().expect("notice lock").list();
        match &self.net {
            Some(net) => {
                let s = net.status();
                AgentStatus {
                    agent_version: APP_VERSION.to_string(),
                    build: build::id(),
                    protocol_version: tautra_proto::PROTOCOL_VERSION,
                    started_unix_ms: self.started_unix_ms,
                    local_device: s.local,
                    connection: s.connection,
                    network: s.network,
                    pairing: s.pairing,
                    discovered: s.discovered,
                    trusted: s.trusted,
                    kvm: self.kvm.status(),
                    share: self.share.status(),
                    notices,
                }
            }
            None => {
                let err = self
                    .net_error
                    .clone()
                    .unwrap_or_else(|| NetError::internal("jaringan tidak berjalan"));
                AgentStatus {
                    agent_version: APP_VERSION.to_string(),
                    build: build::id(),
                    protocol_version: tautra_proto::PROTOCOL_VERSION,
                    started_unix_ms: self.started_unix_ms,
                    local_device: LocalDevice {
                        device_id: String::new(),
                        device_name: self.config().effective_device_name(),
                        fingerprint: String::new(),
                    },
                    connection: ConnectionStatus::Error {
                        code: err.code.to_string(),
                        message: err.message,
                    },
                    network: NetworkStatus::default(),
                    pairing: PairingStatus::Inactive,
                    discovered: Vec::new(),
                    trusted: Vec::new(),
                    kvm: self.kvm.status(),
                    share: self.share.status(),
                    notices,
                }
            }
        }
    }

    pub async fn set_config(&self, new: Config) -> Result<Config, RpcError> {
        new.validate()
            .map_err(|e| RpcError::app(app_error::CONFIG_INVALID, e.to_string()))?;
        config::save_to(&self.config_path, &new)
            .map_err(|e| RpcError::app(app_error::CONFIG_SAVE_FAILED, e.to_string()))?;
        let old = std::mem::replace(&mut *self.config.lock().expect("config lock"), new.clone());

        if old.general.autostart != new.general.autostart && !self.logon {
            if let Err(e) = sync_autostart(new.general.autostart) {
                self.notice(NoticeLevel::Error, app_error::AUTOSTART_FAILED, e);
            }
        }
        self.logon_changed.notify_one();
        if let Some(net) = &self.net {
            let name_changed = old.effective_device_name() != new.effective_device_name();
            if old.network != new.network || name_changed {
                net.update_config(new.network.clone(), new.effective_device_name())
                    .await;
            }
        }
        if old.input != new.input {
            self.kvm.options_changed(&new.input);
        }
        if old.share != new.share {
            self.share.options_changed(&new.share);
        }
        tracing::info!("konfigurasi diperbarui");
        self.changed.notify_one();
        Ok(new)
    }
}

fn sync_autostart(enabled: bool) -> Result<(), String> {
    let command = if enabled {
        let exe =
            std::env::current_exe().map_err(|e| format!("lokasi agent tidak diketahui: {e}"))?;
        Some(autostart::command_for(&exe, &[]))
    } else {
        None
    };
    autostart::set_entry(autostart::AGENT_ENTRY, command.as_deref()).map_err(|e| e.to_string())
}

pub async fn run(logon: bool) -> anyhow::Result<()> {
    let started_unix_ms = unix_ms();
    let mut notices = NoticeBoard::default();

    let (mut config, config_path) = match config::load() {
        Ok(loaded) => {
            if let Some(warning) = loaded.warning {
                tracing::warn!("{warning}");
                notices.push(NoticeLevel::Warning, app_error::CONFIG_LOAD_RESET, warning);
            }
            (loaded.config, loaded.path)
        }
        Err(e) => {
            tracing::error!("konfigurasi tidak bisa dibaca: {e}");
            notices.push(
                NoticeLevel::Error,
                app_error::CONFIG_LOAD_RESET,
                format!("Konfigurasi tidak bisa dibaca ({e}); memakai pengaturan default."),
            );
            (
                Config::default(),
                paths::config_file().context("lokasi config.toml")?,
            )
        }
    };

    if logon {
        config.share.clipboard = false;
        config.share.drag_drop = false;
        tracing::info!("agent berjalan di layar login Windows");
    }

    if let Err(e) = tautra_input::cursor::restore() {
        tracing::warn!("kursor sistem gagal dipulihkan saat start: {e}");
    }

    let (kvm, layout_warning) = Kvm::new();
    if let Some(warning) = layout_warning {
        tracing::warn!("{warning}");
        notices.push(NoticeLevel::Warning, app_error::LAYOUT_SAVE_FAILED, warning);
    }

    if !logon {
        if let Err(e) = sync_autostart(config.general.autostart) {
            tracing::error!("auto-start gagal diatur: {e}");
            notices.push(NoticeLevel::Error, app_error::AUTOSTART_FAILED, e);
        }
    }

    let compat = crate::compat::report();
    if !compat.supported {
        notices.push(
            NoticeLevel::Warning,
            app_error::COMPAT_UNSUPPORTED,
            compat.warnings.join(" "),
        );
    }

    let (net, net_events, net_error) = match tautra_net::start(NetOptions {
        network: config.network.clone(),
        device_name: config.effective_device_name(),
        app_version: APP_VERSION.to_string(),
        updates: !logon,
    })
    .await
    {
        Ok((handle, events)) => (Some(handle), Some(events), None),
        Err(e) => {
            tracing::error!(code = e.code, "jaringan gagal dimulai: {}", e.message);
            notices.push(NoticeLevel::Error, e.code, e.message.clone());
            (None, None, Some(e))
        }
    };

    let (notify_tx, _) = broadcast::channel::<Arc<str>>(32);
    let agent = Arc::new(Agent {
        started_unix_ms,
        config: Mutex::new(config),
        config_path,
        net,
        kvm,
        share: Share::new(),
        net_error,
        notices: Mutex::new(notices),
        changed: Notify::new(),
        notify_tx: notify_tx.clone(),
        cancel: CancellationToken::new(),
        logon,
        logon_changed: Notify::new(),
    });

    if !logon {
        agent.share.start(&agent);
        if let Ok(dir) = build::exe_dir() {
            tokio::spawn(clean_old_until_done(dir));
        }
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let _ = tokio::task::spawn_blocking(crate::service::check_update).await;
        });
        tokio::spawn(crate::logon_sync::run(agent.clone()));
    }

    tokio::spawn(status_pump(agent.clone(), notify_tx.clone()));
    if let Some(events) = net_events {
        tokio::spawn(event_pump(agent.clone(), events));
    }
    tokio::spawn({
        let cancel = agent.cancel.clone();
        async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                tracing::info!("Ctrl+C diterima");
                cancel.cancel();
            }
        }
    });

    let served = if logon {
        agent.cancel.cancelled().await;
        Ok(())
    } else {
        tautra_ipc::pipe::serve(
            PIPE_NAME,
            Arc::new(Rpc::new(agent.clone())),
            notify_tx,
            agent.cancel.clone(),
        )
        .await
    };

    agent.cancel.cancel();
    agent.share.shutdown();
    if let Some(net) = &agent.net {
        if tokio::time::timeout(std::time::Duration::from_secs(3), net.shutdown())
            .await
            .is_err()
        {
            tracing::warn!("jaringan tidak berhenti tepat waktu");
        }
    }
    served.context("server IPC named pipe gagal")
}

async fn status_pump(agent: Arc<Agent>, tx: broadcast::Sender<Arc<str>>) {
    let mut net_rx = agent.net.as_ref().map(|n| n.watch_status());
    let mut last: Option<AgentStatus> = None;
    loop {
        let status = agent.status();
        if last.as_ref() != Some(&status) {
            if let Some(text) = notification_text(notification::STATUS_CHANGED, &status) {
                let _ = tx.send(text);
            }
            last = Some(status);
        }
        tokio::select! {
            changed = async {
                match net_rx.as_mut() {
                    Some(rx) => rx.changed().await.is_ok(),
                    None => std::future::pending().await,
                }
            } => {
                if !changed {
                    net_rx = None;
                }
            }
            _ = agent.changed.notified() => {}
            _ = agent.cancel.cancelled() => break,
        }
    }
}

async fn event_pump(agent: Arc<Agent>, mut events: broadcast::Receiver<NetEvent>) {
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(NetEvent::Notice { level, code, message }) => agent.notice(level, code, message),
                Ok(NetEvent::SessionStarted(session)) => {
                    tracing::info!(device = %session.device_id, name = %session.device_name, "sesi dimulai");
                    crate::streams::start(agent.clone(), session);
                }
                Ok(NetEvent::SessionEnded { device_id, device_name }) => {
                    tracing::info!(device = %device_id, name = %device_name, "sesi berakhir");
                }
                Ok(NetEvent::UpdateReady { device_name, build, dir, files }) => {
                    if !agent.logon {
                        crate::update::apply(&agent, &device_name, build, dir, files);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("{n} event jaringan terlewat");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = agent.cancel.cancelled() => break,
        }
    }
}

async fn clean_old_until_done(dir: std::path::PathBuf) {
    for _ in 0..90 {
        let target = dir.clone();
        let clean = tokio::task::spawn_blocking(move || build::clean_old(&target))
            .await
            .unwrap_or(false);
        if clean {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    tracing::warn!(dir = %dir.display(), "file .old lama masih terkunci, dihapus saat start berikutnya");
}
