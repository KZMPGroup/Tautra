use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use quinn::Endpoint;
use tautra_common::build;
use tautra_proto::update::{UpdateFile, UpdateManifest, UpdateReply, UpdateRequest, MAX_UPDATE_FILE};
use tautra_proto::{close_code, ALPN_UPDATE};
use tokio::io::AsyncWriteExt;

use crate::fingerprint::Fingerprint;
use crate::framing::{read_msg, write_msg};
use crate::tls::{TlsContext, SERVER_NAME};

const CHUNK: usize = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);
const FILE_TIMEOUT: Duration = Duration::from_secs(300);
pub const UPDATE_DIR: &str = "update";

fn hash_file(path: &Path) -> std::io::Result<(u64, [u8; 32])> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let size = std::io::copy(&mut file, &mut hasher)?;
    Ok((size, *hasher.finalize().as_bytes()))
}

fn local_manifest() -> Result<(PathBuf, UpdateManifest), String> {
    let dir = build::exe_dir().map_err(|e| e.to_string())?;
    let marker = build::disk_build(&dir)
        .map_err(|e| format!("exe di {} tidak bisa dibaca: {e}", dir.display()))?;
    if marker != Some(build::id()) {
        return Err("exe di disk bukan versi yang sedang berjalan; jalankan ulang Tautra".into());
    }
    let mut files = Vec::new();
    for name in build::UPDATE_FILES {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let (size, hash) = hash_file(&path).map_err(|e| format!("{name}: {e}"))?;
        files.push(UpdateFile {
            name: name.to_string(),
            size,
            hash,
        });
    }
    Ok((
        dir,
        UpdateManifest {
            build: build::id(),
            files,
        },
    ))
}

pub async fn serve(conn: quinn::Connection) {
    let result: Result<(), String> = async {
        let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
        let mut served: Option<(PathBuf, UpdateManifest)> = None;
        loop {
            let request: UpdateRequest = match read_msg(&mut recv).await {
                Ok(r) => r,
                Err(crate::framing::StreamError::Closed) => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            match request {
                UpdateRequest::Manifest => {
                    let built = tokio::task::spawn_blocking(local_manifest)
                        .await
                        .map_err(|e| e.to_string())?;
                    match built {
                        Ok((dir, manifest)) => {
                            write_msg(&mut send, &UpdateReply::Manifest(manifest.clone()))
                                .await
                                .map_err(|e| e.to_string())?;
                            served = Some((dir, manifest));
                        }
                        Err(e) => {
                            write_msg(&mut send, &UpdateReply::Error(e.clone()))
                                .await
                                .map_err(|e| e.to_string())?;
                            let _ = send.finish();
                            let _ = tokio::time::timeout(REPLY_TIMEOUT, send.stopped()).await;
                            return Err(e);
                        }
                    }
                }
                UpdateRequest::File { name } => {
                    let Some((dir, manifest)) = &served else {
                        return Err("file diminta sebelum manifest".into());
                    };
                    let Some(entry) = manifest.files.iter().find(|f| f.name == name) else {
                        write_msg(&mut send, &UpdateReply::Error(format!("{name} tidak ada")))
                            .await
                            .map_err(|e| e.to_string())?;
                        continue;
                    };
                    let path = dir.join(&entry.name);
                    let mut file = tokio::fs::File::open(&path)
                        .await
                        .map_err(|e| format!("{name}: {e}"))?;
                    write_msg(&mut send, &UpdateReply::File { size: entry.size })
                        .await
                        .map_err(|e| e.to_string())?;
                    let mut remaining = entry.size;
                    let mut buf = vec![0u8; CHUNK];
                    while remaining > 0 {
                        let want = (remaining as usize).min(CHUNK);
                        let n = tokio::io::AsyncReadExt::read(&mut file, &mut buf[..want])
                            .await
                            .map_err(|e| format!("{name}: {e}"))?;
                        if n == 0 {
                            return Err(format!("{name} berubah saat dikirim"));
                        }
                        send.write_all(&buf[..n])
                            .await
                            .map_err(|e| e.to_string())?;
                        remaining -= n as u64;
                    }
                }
            }
        }
    }
    .await;
    match result {
        Ok(()) => {
            tracing::info!(remote = %conn.remote_address(), "pembaruan selesai dikirim");
            conn.close(close_code::NORMAL.into(), b"update done");
        }
        Err(e) => {
            tracing::warn!(remote = %conn.remote_address(), "pengiriman pembaruan gagal: {e}");
            conn.close(close_code::PROTOCOL_ERROR.into(), b"update failed");
        }
    }
}

pub struct Fetched {
    pub build: u64,
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

async fn connect(
    endpoint: &Endpoint,
    tls: &TlsContext,
    addrs: &[SocketAddr],
    expected: Fingerprint,
) -> Result<quinn::Connection, String> {
    let (config, _) = tls
        .client_config(ALPN_UPDATE, Some(expected))
        .map_err(|e| e.to_string())?;
    let mut last = String::from("tidak ada alamat");
    for addr in addrs {
        let connecting = match endpoint.connect_with(config.clone(), *addr, SERVER_NAME) {
            Ok(c) => c,
            Err(e) => {
                last = format!("{addr}: {e}");
                continue;
            }
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(Ok(conn)) => return Ok(conn),
            Ok(Err(e)) => last = format!("{addr}: {e}"),
            Err(_) => last = format!("{addr}: batas waktu"),
        }
    }
    Err(last)
}

async fn read_reply(recv: &mut quinn::RecvStream) -> Result<UpdateReply, String> {
    tokio::time::timeout(REPLY_TIMEOUT, read_msg::<UpdateReply>(recv))
        .await
        .map_err(|_| "perangkat lain tidak menjawab".to_string())?
        .map_err(|e| e.to_string())
}

async fn receive_file(
    recv: &mut quinn::RecvStream,
    dest: &Path,
    entry: &UpdateFile,
) -> Result<(), String> {
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut remaining = entry.size;
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = (remaining as usize).min(CHUNK);
        let n = recv
            .read(&mut buf[..want])
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "stream terputus sebelum file lengkap".to_string())?;
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .await
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        remaining -= n as u64;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if *hasher.finalize().as_bytes() != entry.hash {
        return Err(format!("hash {} tidak cocok", entry.name));
    }
    Ok(())
}

pub async fn fetch(
    endpoint: Endpoint,
    tls: std::sync::Arc<TlsContext>,
    addrs: Vec<SocketAddr>,
    expected: Fingerprint,
    expected_build: u64,
) -> Result<Fetched, String> {
    let exe_dir = build::exe_dir().map_err(|e| e.to_string())?;
    let dir = exe_dir.join(UPDATE_DIR);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("folder pembaruan gagal dibuat: {e}"))?;

    let conn = connect(&endpoint, &tls, &addrs, expected).await?;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        write_msg(&mut send, &UpdateRequest::Manifest)
            .await
            .map_err(|e| e.to_string())?;
        let manifest = match read_reply(&mut recv).await? {
            UpdateReply::Manifest(m) => m,
            UpdateReply::Error(e) => return Err(e),
            UpdateReply::File { .. } => return Err("balasan tidak sesuai".into()),
        };
        if manifest.build != expected_build {
            return Err(format!(
                "build yang dikirim ({}) berbeda dari yang diiklankan ({expected_build})",
                manifest.build
            ));
        }
        if !manifest.files.iter().any(|f| f.name == build::AGENT_EXE) {
            return Err("manifest tidak memuat exe agent".into());
        }
        let mut files = Vec::new();
        for entry in &manifest.files {
            if entry.size == 0 || entry.size > MAX_UPDATE_FILE {
                return Err(format!("ukuran {} tidak wajar", entry.name));
            }
            let dest = dir.join(&entry.name);
            let current = exe_dir.join(&entry.name);
            if entry.name == build::UNINSTALL_EXE && !current.is_file() {
                continue;
            }
            let same = tokio::task::spawn_blocking({
                let current = current.clone();
                let hash = entry.hash;
                move || hash_file(&current).map(|(_, h)| h == hash).unwrap_or(false)
            })
            .await
            .unwrap_or(false);
            if same {
                tracing::info!(name = %entry.name, "file lokal sudah sama, tidak diunduh");
                continue;
            }
            write_msg(
                &mut send,
                &UpdateRequest::File {
                    name: entry.name.clone(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            match read_reply(&mut recv).await? {
                UpdateReply::File { size } if size == entry.size => {}
                UpdateReply::File { .. } => return Err(format!("ukuran {} berubah", entry.name)),
                UpdateReply::Error(e) => return Err(e),
                UpdateReply::Manifest(_) => return Err("balasan tidak sesuai".into()),
            }
            tokio::time::timeout(FILE_TIMEOUT, receive_file(&mut recv, &dest, entry))
                .await
                .map_err(|_| format!("{} terlalu lama diunduh", entry.name))??;
            files.push(dest);
        }
        let _ = send.finish();
        let staged = dir.clone();
        let current = exe_dir.clone();
        let combined = tokio::task::spawn_blocking(move || -> std::io::Result<Option<u64>> {
            let mut best: Option<u64> = None;
            for name in build::UPDATE_FILES {
                let path = if staged.join(name).is_file() {
                    staged.join(name)
                } else {
                    current.join(name)
                };
                if !path.is_file() {
                    continue;
                }
                match build::marker_in_file(&path)? {
                    Some(id) => best = Some(best.map_or(id, |b| b.max(id))),
                    None if name == build::AGENT_EXE => return Ok(None),
                    None => {}
                }
            }
            Ok(best)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        if combined != Some(manifest.build) {
            return Err("file yang diunduh tidak memuat tanda build yang benar".into());
        }
        Ok(Fetched {
            build: manifest.build,
            dir: dir.clone(),
            files,
        })
    }
    .await;
    conn.close(close_code::NORMAL.into(), b"update");
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
    result
}
