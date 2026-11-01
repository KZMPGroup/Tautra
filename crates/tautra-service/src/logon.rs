use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tautra_common::paths;

use crate::protocol::LogonPayload;

pub const FILES: [&str; 6] = [
    "config.toml",
    "layouts.toml",
    "trusted_devices.toml",
    "network_paths.toml",
    "known_networks.toml",
    "identity/identity.toml",
];
const KEY_FILE: &str = "identity/identity.key";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn dir() -> PathBuf {
    paths::logon_dir()
}

pub fn ready() -> bool {
    let root = dir();
    root.join(KEY_FILE).is_file() && root.join("identity/identity.toml").is_file()
}

fn lock_down(root: &Path) {
    let _ = Command::new("icacls")
        .arg(root)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub fn store(payload: &LogonPayload) -> Result<(), String> {
    if payload.key.is_empty() {
        return Err("kunci identitas kosong".into());
    }
    if !payload.files.iter().any(|f| f.path == "identity/identity.toml") {
        return Err("identitas perangkat tidak ikut terkirim".into());
    }
    let root = dir();
    std::fs::create_dir_all(root.join("identity")).map_err(|e| format!("folder login gagal dibuat: {e}"))?;
    lock_down(&root);
    for name in FILES {
        let target = root.join(name);
        match payload.files.iter().find(|f| f.path == name) {
            Some(file) => paths::write_atomic(&target, &file.data)
                .map_err(|e| format!("{name} gagal ditulis: {e}"))?,
            None => {
                let _ = std::fs::remove_file(&target);
            }
        }
    }
    let sealed = tautra_common::dpapi::protect(&payload.key).map_err(|e| e.to_string())?;
    paths::write_atomic(&root.join(KEY_FILE), &sealed).map_err(|e| format!("kunci gagal ditulis: {e}"))?;
    tracing::info!("data Tautra untuk layar login diperbarui");
    Ok(())
}

pub fn clear() -> Result<(), String> {
    let root = dir();
    if !root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&root).map_err(|e| format!("data layar login gagal dihapus: {e}"))?;
    tracing::info!("data Tautra untuk layar login dihapus");
    Ok(())
}
