use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tautra_common::paths;
use tautra_common::time::unix_ms;
use uuid::Uuid;

const MAX_RECORDS: usize = 64;
const RECORD_TTL_MS: u64 = 30 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    peer: Uuid,
    addr: Ipv4Addr,
    score_us: u64,
    measured_unix_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    paths: Vec<Record>,
}

pub struct PathMemory {
    path: Option<PathBuf>,
    records: HashMap<(Uuid, Ipv4Addr), Record>,
}

impl PathMemory {
    pub fn load() -> Self {
        let path = paths::network_paths_file().ok();
        let mut records = HashMap::new();
        if let Some(p) = &path {
            if let Ok(text) = std::fs::read_to_string(p) {
                match toml::from_str::<File>(&text) {
                    Ok(file) => {
                        let now = unix_ms();
                        for r in file.paths {
                            if now.saturating_sub(r.measured_unix_ms) <= RECORD_TTL_MS {
                                records.insert((r.peer, r.addr), r);
                            }
                        }
                    }
                    Err(e) => tracing::warn!("network_paths.toml rusak, diabaikan: {e}"),
                }
            }
        }
        Self { path, records }
    }

    pub fn score(&self, peer: Uuid, addr: Ipv4Addr) -> Option<Duration> {
        self.records
            .get(&(peer, addr))
            .map(|r| Duration::from_micros(r.score_us))
    }

    pub fn record(&mut self, peer: Uuid, addr: Ipv4Addr, score: Option<Duration>) {
        match score {
            Some(score) => {
                self.records.insert(
                    (peer, addr),
                    Record {
                        peer,
                        addr,
                        score_us: score.as_micros() as u64,
                        measured_unix_ms: unix_ms(),
                    },
                );
            }
            None => {
                self.records.remove(&(peer, addr));
            }
        }
        while self.records.len() > MAX_RECORDS {
            let oldest = self
                .records
                .values()
                .min_by_key(|r| r.measured_unix_ms)
                .map(|r| (r.peer, r.addr));
            match oldest {
                Some(key) => {
                    self.records.remove(&key);
                }
                None => break,
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut list: Vec<Record> = self.records.values().cloned().collect();
        list.sort_by_key(|r| (r.peer, r.addr));
        let file = File { paths: list };
        match toml::to_string_pretty(&file) {
            Ok(text) => {
                if let Err(e) = paths::write_atomic(path, text.as_bytes()) {
                    tracing::warn!("riwayat jalur jaringan gagal disimpan: {e}");
                }
            }
            Err(e) => tracing::warn!("riwayat jalur jaringan gagal dibentuk: {e}"),
        }
    }
}
