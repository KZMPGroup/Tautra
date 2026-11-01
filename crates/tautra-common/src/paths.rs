use std::path::PathBuf;

use crate::APP_NAME;

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("folder {0} tidak ditemukan di sistem")]
    MissingKnownFolder(&'static str),
    #[error("gagal membuat folder {path}: {source}")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn ensure(path: PathBuf) -> Result<PathBuf, PathError> {
    std::fs::create_dir_all(&path).map_err(|source| PathError::Create {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn config_dir() -> Result<PathBuf, PathError> {
    let base = dirs::config_dir().ok_or(PathError::MissingKnownFolder("APPDATA"))?;
    ensure(base.join(APP_NAME))
}

pub fn local_dir() -> Result<PathBuf, PathError> {
    let base = dirs::data_local_dir().ok_or(PathError::MissingKnownFolder("LOCALAPPDATA"))?;
    ensure(base.join(APP_NAME))
}

pub fn log_dir() -> Result<PathBuf, PathError> {
    ensure(local_dir()?.join("logs"))
}

pub fn identity_dir() -> Result<PathBuf, PathError> {
    ensure(local_dir()?.join("identity"))
}

pub fn config_file() -> Result<PathBuf, PathError> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn layouts_file() -> Result<PathBuf, PathError> {
    Ok(config_dir()?.join("layouts.toml"))
}

pub fn trusted_devices_file() -> Result<PathBuf, PathError> {
    Ok(local_dir()?.join("trusted_devices.toml"))
}

pub fn network_paths_file() -> Result<PathBuf, PathError> {
    Ok(local_dir()?.join("network_paths.toml"))
}

pub fn known_networks_file() -> Result<PathBuf, PathError> {
    Ok(local_dir()?.join("known_networks.toml"))
}

pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}
