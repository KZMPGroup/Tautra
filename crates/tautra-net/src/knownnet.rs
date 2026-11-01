use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tautra_common::paths;
use tautra_common::time::unix_ms;

const MAX_KNOWN: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    last_seen_unix_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    networks: Vec<Entry>,
}

pub struct KnownNetworks {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

impl KnownNetworks {
    pub fn load() -> Self {
        let path = paths::known_networks_file().ok();
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| match toml::from_str::<File>(&text) {
                Ok(file) => Some(file.networks),
                Err(e) => {
                    tracing::warn!("known_networks.toml rusak, diabaikan: {e}");
                    None
                }
            })
            .unwrap_or_default();
        Self { path, entries }
    }

    pub fn names(&self) -> Vec<String> {
        let mut list: Vec<&Entry> = self.entries.iter().collect();
        list.sort_by_key(|e| std::cmp::Reverse(e.last_seen_unix_ms));
        list.into_iter().map(|e| e.name.clone()).collect()
    }

    pub fn see(&mut self, names: &[String]) {
        let now = unix_ms();
        let mut fresh = Vec::new();
        for name in names {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            match self.entries.iter_mut().find(|e| same(&e.name, name)) {
                Some(entry) => entry.last_seen_unix_ms = now,
                None => {
                    self.entries.push(Entry {
                        name: name.to_string(),
                        last_seen_unix_ms: now,
                    });
                    fresh.push(name.to_string());
                }
            }
        }
        if !fresh.is_empty() {
            while self.entries.len() > MAX_KNOWN {
                let oldest = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, e)| e.last_seen_unix_ms)
                    .map(|(i, _)| i);
                match oldest {
                    Some(i) => {
                        self.entries.remove(i);
                    }
                    None => break,
                }
            }
            self.save();
        }
    }

    pub fn forget(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !same(&e.name, name));
        let changed = self.entries.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let file = File {
            networks: self.entries.clone(),
        };
        match toml::to_string_pretty(&file) {
            Ok(text) => {
                if let Err(e) = paths::write_atomic(path, text.as_bytes()) {
                    tracing::warn!("daftar jaringan gagal disimpan: {e}");
                }
            }
            Err(e) => tracing::warn!("daftar jaringan gagal dibentuk: {e}"),
        }
    }
}
