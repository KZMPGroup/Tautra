use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tautra_common::paths;
use tautra_ipc::api::TrustedDevice;
use uuid::Uuid;

use crate::fingerprint::Fingerprint;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedEntry {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: String,
    pub paired_unix_ms: u64,
    #[serde(default)]
    pub last_connected_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

impl TrustedEntry {
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        Fingerprint::from_hex(&self.fingerprint)
    }

    pub fn manual_address(&self) -> Option<std::net::SocketAddrV4> {
        self.address.as_deref().and_then(|a| a.parse().ok())
    }

    pub fn to_api(&self) -> TrustedDevice {
        TrustedDevice {
            device_id: self.device_id.to_string(),
            device_name: self.device_name.clone(),
            fingerprint: self
                .fingerprint()
                .map(|f| f.display())
                .unwrap_or_else(|| self.fingerprint.clone()),
            paired_unix_ms: self.paired_unix_ms,
            last_connected_unix_ms: self.last_connected_unix_ms,
            address: self.address.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default, rename = "device")]
    devices: Vec<TrustedEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("gagal menulis {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("gagal menyusun daftar perangkat: {0}")]
    Serialize(#[from] toml::ser::Error),
}

pub struct TrustStore {
    path: PathBuf,
    entries: Vec<TrustedEntry>,
}

impl TrustStore {
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let empty = |path: &Path| Self {
            path: path.to_path_buf(),
            entries: Vec::new(),
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (empty(path), None),
            Err(e) => {
                return (
                    empty(path),
                    Some(format!("Daftar perangkat trusted tidak bisa dibaca: {e}")),
                )
            }
        };
        match toml::from_str::<TrustFile>(&text) {
            Ok(file) => {
                let entries = file
                    .devices
                    .into_iter()
                    .filter(|e| e.fingerprint().is_some())
                    .collect();
                (
                    Self {
                        path: path.to_path_buf(),
                        entries,
                    },
                    None,
                )
            }
            Err(e) => {
                let backup =
                    path.with_extension(format!("broken-{}.toml", tautra_common::time::unix_ms()));
                let _ = std::fs::rename(path, &backup);
                (
                    empty(path),
                    Some(format!(
                        "Daftar perangkat trusted rusak ({}); salinan disimpan di {}. Pair ulang perangkat Anda.",
                        e.message(),
                        backup.display()
                    )),
                )
            }
        }
    }

    pub fn save(&self) -> Result<(), TrustError> {
        let file = TrustFile {
            devices: self.entries.clone(),
        };
        let text = toml::to_string_pretty(&file)?;
        paths::write_atomic(&self.path, text.as_bytes()).map_err(|source| TrustError::Write {
            path: self.path.clone(),
            source,
        })
    }

    pub fn entries(&self) -> &[TrustedEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn by_id(&self, id: &Uuid) -> Option<&TrustedEntry> {
        self.entries.iter().find(|e| e.device_id == *id)
    }

    pub fn by_fingerprint(&self, fp: &Fingerprint) -> Option<&TrustedEntry> {
        self.entries
            .iter()
            .find(|e| e.fingerprint().as_ref() == Some(fp))
    }

    pub fn upsert(&mut self, entry: TrustedEntry) {
        self.entries.retain(|e| e.device_id != entry.device_id);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, id: &Uuid) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.device_id != *id);
        before != self.entries.len()
    }

    pub fn mark_connected(&mut self, id: &Uuid, name: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.device_id == *id) {
            entry.last_connected_unix_ms = Some(tautra_common::time::unix_ms());
            if !name.is_empty() {
                entry.device_name = name.to_string();
            }
        }
    }

    pub fn rename(&mut self, id: &Uuid, name: &str) -> bool {
        match self.entries.iter_mut().find(|e| e.device_id == *id) {
            Some(entry) if entry.device_name != name && !name.is_empty() => {
                entry.device_name = name.to_string();
                true
            }
            _ => false,
        }
    }
}

pub fn default_path() -> Result<PathBuf, paths::PathError> {
    paths::trusted_devices_file()
}
