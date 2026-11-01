use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tautra_common::{dpapi, paths};
use tautra_service::{LogonFile, LogonPayload};

use crate::app::Agent;

const INTERVAL: Duration = Duration::from_secs(30);
const CLEARED: u64 = 0;

fn source(name: &str) -> Option<PathBuf> {
    match name {
        "config.toml" => paths::config_file().ok(),
        "layouts.toml" => paths::layouts_file().ok(),
        "trusted_devices.toml" => paths::trusted_devices_file().ok(),
        "network_paths.toml" => paths::network_paths_file().ok(),
        "known_networks.toml" => paths::known_networks_file().ok(),
        "identity/identity.toml" => paths::identity_dir().ok().map(|d| d.join("identity.toml")),
        _ => None,
    }
}

fn payload() -> Option<LogonPayload> {
    let sealed = std::fs::read(paths::identity_dir().ok()?.join("identity.key")).ok()?;
    let key = dpapi::unprotect(&sealed).ok()?;
    let files = tautra_service::logon::FILES
        .iter()
        .filter_map(|name| {
            let data = std::fs::read(source(name)?).ok()?;
            Some(LogonFile {
                path: (*name).to_string(),
                data,
            })
        })
        .collect::<Vec<_>>();
    files
        .iter()
        .any(|f| f.path == "identity/identity.toml")
        .then_some(LogonPayload { files, key })
}

fn fingerprint(payload: &LogonPayload) -> u64 {
    let mut hasher = DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish().max(1)
}

fn sync_once(enabled: bool, last: Option<u64>) -> Option<u64> {
    if !tautra_service::install::is_installed() {
        return last;
    }
    if !tautra_service::install::ensure_running() {
        return last;
    }
    if !enabled {
        if last == Some(CLEARED) {
            return last;
        }
        return match tautra_service::client::clear_logon() {
            Ok(()) => Some(CLEARED),
            Err(e) => {
                tracing::debug!("data layar login belum bisa dihapus: {e}");
                last
            }
        };
    }
    let Some(payload) = payload() else {
        return last;
    };
    let current = fingerprint(&payload);
    if last == Some(current) {
        return last;
    }
    match tautra_service::client::sync_logon(payload) {
        Ok(()) => {
            tracing::info!("data untuk layar login dikirim ke layanan Tautra");
            Some(current)
        }
        Err(e) => {
            tracing::debug!("data layar login belum bisa dikirim: {e}");
            last
        }
    }
}

pub async fn run(agent: Arc<Agent>) {
    let mut last: Option<u64> = None;
    tokio::time::sleep(Duration::from_secs(5)).await;
    loop {
        let enabled = agent.config().general.autostart;
        last = tokio::task::spawn_blocking(move || sync_once(enabled, last))
            .await
            .unwrap_or(last);
        tokio::select! {
            _ = tokio::time::sleep(INTERVAL) => {}
            _ = agent.logon_changed.notified() => {}
            _ = agent.cancel.cancelled() => break,
        }
    }
}
