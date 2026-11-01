use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;
use crate::paths;

pub const DEFAULT_QUIC_PORT: u16 = 47800;

pub const MAX_DEVICE_NAME_LEN: usize = 63;

pub const MAX_EDGE_DELAY_MS: u32 = 2000;

pub const MIN_CLIPBOARD_MB: u32 = 1;

pub const MAX_CLIPBOARD_MB: u32 = 1024;

pub const LANGUAGE_AUTO: &str = "auto";

pub const MAX_LANGUAGE_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub network: NetworkConfig,
    pub input: InputConfig,
    pub share: ShareConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub device_name: String,
    pub language: String,
    pub autostart: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            language: LANGUAGE_AUTO.into(),
            autostart: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub port: u16,
    pub auto_connect: bool,
    pub restrict_networks: bool,
    pub allowed_networks: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_QUIC_PORT,
            auto_connect: true,
            restrict_networks: false,
            allowed_networks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputConfig {
    pub edge_delay_ms: u32,
    pub hotkey_switch_local: String,
    pub hotkey_switch_remote: String,
    pub hotkey_lock: String,
    pub hotkey_gaming: String,
    pub gaming_auto: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            edge_delay_ms: 0,
            hotkey_switch_local: "Ctrl+Alt+F1".into(),
            hotkey_switch_remote: "Ctrl+Alt+F2".into(),
            hotkey_lock: "ScrollLock".into(),
            hotkey_gaming: "Ctrl+Alt+G".into(),
            gaming_auto: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareConfig {
    pub clipboard: bool,
    pub text: bool,
    pub html: bool,
    pub rtf: bool,
    pub image: bool,
    pub files: bool,
    pub drag_drop: bool,
    pub max_clipboard_mb: u32,
}

impl Default for ShareConfig {
    fn default() -> Self {
        Self {
            clipboard: true,
            text: true,
            html: true,
            rtf: true,
            image: true,
            files: true,
            drag_drop: true,
            max_clipboard_mb: 64,
        }
    }
}

impl ShareConfig {
    pub fn max_clipboard_bytes(&self) -> u64 {
        self.max_clipboard_mb.clamp(MIN_CLIPBOARD_MB, MAX_CLIPBOARD_MB) as u64 * 1024 * 1024
    }
}

impl InputConfig {
    pub fn hotkeys(&self) -> [(&'static str, &str); 4] {
        [
            ("input.hotkey_switch_local", &self.hotkey_switch_local),
            ("input.hotkey_switch_remote", &self.hotkey_switch_remote),
            ("input.hotkey_lock", &self.hotkey_lock),
            ("input.hotkey_gaming", &self.hotkey_gaming),
        ]
    }
}

impl Config {
    pub fn effective_device_name(&self) -> String {
        let name = self.general.device_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "Windows PC".to_string())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.input.edge_delay_ms > MAX_EDGE_DELAY_MS {
            return Err(ConfigError::Invalid {
                field: "input.edge_delay_ms",
                reason: format!("maksimal {MAX_EDGE_DELAY_MS} ms"),
            });
        }
        let mut seen = Vec::new();
        for (field, text) in self.input.hotkeys() {
            if text.trim().is_empty() {
                continue;
            }
            let hotkey = Hotkey::parse(text).map_err(|e| ConfigError::Invalid {
                field,
                reason: e.to_string(),
            })?;
            if seen.contains(&hotkey) {
                return Err(ConfigError::Invalid {
                    field,
                    reason: format!("{hotkey} sudah dipakai hotkey lain"),
                });
            }
            seen.push(hotkey);
        }
        if !(MIN_CLIPBOARD_MB..=MAX_CLIPBOARD_MB).contains(&self.share.max_clipboard_mb) {
            return Err(ConfigError::Invalid {
                field: "share.max_clipboard_mb",
                reason: format!("harus {MIN_CLIPBOARD_MB}–{MAX_CLIPBOARD_MB} MB"),
            });
        }
        let language = &self.general.language;
        if language.is_empty()
            || language.len() > MAX_LANGUAGE_LEN
            || !language.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(ConfigError::Invalid {
                field: "general.language",
                reason: format!("kode bahasa tidak dikenal: {language}"),
            });
        }
        let name = self.general.device_name.trim();
        if name.chars().count() > MAX_DEVICE_NAME_LEN {
            return Err(ConfigError::Invalid {
                field: "general.device_name",
                reason: format!("maksimal {MAX_DEVICE_NAME_LEN} karakter"),
            });
        }
        if name.chars().any(char::is_control) {
            return Err(ConfigError::Invalid {
                field: "general.device_name",
                reason: "tidak boleh berisi karakter kontrol".into(),
            });
        }
        if self
            .network
            .allowed_networks
            .iter()
            .any(|n| n.trim().is_empty())
        {
            return Err(ConfigError::Invalid {
                field: "network.allowed_networks",
                reason: "nama jaringan tidak boleh kosong".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Path(#[from] paths::PathError),
    #[error("gagal membaca {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("gagal menulis {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("gagal menyusun TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("nilai {field} tidak valid: {reason}")]
    Invalid { field: &'static str, reason: String },
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
    pub warning: Option<String>,
}

pub fn load() -> Result<LoadedConfig, ConfigError> {
    let path = paths::config_file()?;
    load_from(&path)
}

pub fn load_from(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            save_to(path, &config)?;
            return Ok(LoadedConfig {
                config,
                path: path.to_path_buf(),
                warning: None,
            });
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    match toml::from_str::<Config>(&text) {
        Ok(config) => Ok(LoadedConfig {
            config,
            path: path.to_path_buf(),
            warning: None,
        }),
        Err(parse_err) => {
            let backup = path.with_extension(format!("broken-{}.toml", crate::time::unix_ms()));
            let _ = std::fs::rename(path, &backup);
            let config = Config::default();
            save_to(path, &config)?;
            Ok(LoadedConfig {
                config,
                path: path.to_path_buf(),
                warning: Some(format!(
                    "config.toml tidak valid ({}). Salinan disimpan di {} dan konfigurasi default dipakai.",
                    parse_err.message(),
                    backup.display()
                )),
            })
        }
    }
}

pub fn save(config: &Config) -> Result<(), ConfigError> {
    let path = paths::config_file()?;
    save_to(&path, config)
}

pub fn save_to(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let text = toml::to_string_pretty(config)?;
    paths::write_atomic(path, text.as_bytes()).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })
}
