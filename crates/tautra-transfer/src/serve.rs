use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, UNIX_EPOCH};

use tautra_net::write_msg;
use tautra_proto::share::{FileEntry, FileReply};
use tokio::sync::mpsc;

use crate::error::TransferError;
use crate::{CHUNK, RESET_SOURCE_FAILED};

enum Piece {
    Data(Vec<u8>),
    Hash([u8; 32]),
    Failed(String),
}

pub type FileKey = (u64, u32);

const CHECKPOINT_EVERY: u64 = 64 * 1024 * 1024;
const MAX_CHECKPOINT_FILES: usize = 16;
const CHECKPOINT_TTL_SECS: u64 = 3600;

struct Checkpoints {
    points: Vec<(u64, blake3::Hasher)>,
    touched: Instant,
}

static CHECKPOINTS: Mutex<Option<HashMap<FileKey, Checkpoints>>> = Mutex::new(None);

fn with_checkpoints<T>(f: impl FnOnce(&mut HashMap<FileKey, Checkpoints>) -> T) -> T {
    let mut guard = CHECKPOINTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, c| c.touched.elapsed().as_secs() < CHECKPOINT_TTL_SECS);
    f(map)
}

fn checkpoint_before(key: FileKey, offset: u64) -> Option<(u64, blake3::Hasher)> {
    with_checkpoints(|map| {
        let entry = map.get_mut(&key)?;
        entry.touched = Instant::now();
        entry
            .points
            .iter()
            .filter(|(pos, _)| *pos <= offset)
            .max_by_key(|(pos, _)| *pos)
            .cloned()
    })
}

fn save_checkpoint(key: FileKey, pos: u64, hasher: &blake3::Hasher) {
    with_checkpoints(|map| {
        if !map.contains_key(&key) && map.len() >= MAX_CHECKPOINT_FILES {
            let oldest = map
                .iter()
                .min_by_key(|(_, c)| c.touched)
                .map(|(k, _)| *k);
            if let Some(oldest) = oldest {
                map.remove(&oldest);
            }
        }
        let entry = map.entry(key).or_insert_with(|| Checkpoints {
            points: Vec::new(),
            touched: Instant::now(),
        });
        entry.touched = Instant::now();
        entry.points.retain(|(p, _)| *p != pos);
        entry.points.push((pos, hasher.clone()));
    });
}

fn forget_checkpoints(key: FileKey) {
    with_checkpoints(|map| {
        map.remove(&key);
    });
}

fn current_state(path: &Path) -> std::io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok((meta.len(), modified))
}

fn read_into(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn produce(path: PathBuf, size: u64, offset: u64, key: FileKey, tx: mpsc::Sender<Piece>) {
    let mut hasher = blake3::Hasher::new();
    let mut sent = 0u64;
    let result = (|| -> std::io::Result<[u8; 32]> {
        let mut file = std::fs::File::open(&path)?;
        let mut buf = vec![0u8; CHUNK];
        let mut hashed = 0u64;
        if let Some((pos, saved)) = checkpoint_before(key, offset) {
            if pos > 0 {
                file.seek(SeekFrom::Start(pos))?;
                hasher = saved;
                hashed = pos;
            }
        }
        while hashed < offset {
            let want = ((offset - hashed) as usize).min(CHUNK);
            let n = read_into(&mut file, &mut buf[..want])?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "file lebih pendek dari yang diumumkan",
                ));
            }
            hasher.update(&buf[..n]);
            hashed += n as u64;
        }
        file.seek(SeekFrom::Start(offset))?;
        sent = offset;
        let mut next_checkpoint = (offset / CHECKPOINT_EVERY + 1) * CHECKPOINT_EVERY;
        while sent < size {
            let want = ((size - sent) as usize).min(CHUNK);
            let mut chunk = vec![0u8; want];
            let n = read_into(&mut file, &mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "file terpotong saat dibaca",
                ));
            }
            chunk.truncate(n);
            hasher.update(&chunk);
            sent += n as u64;
            if sent >= next_checkpoint {
                save_checkpoint(key, sent, &hasher);
                next_checkpoint = (sent / CHECKPOINT_EVERY + 1) * CHECKPOINT_EVERY;
            }
            if tx.blocking_send(Piece::Data(chunk)).is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "pengiriman dihentikan",
                ));
            }
        }
        Ok(*hasher.finalize().as_bytes())
    })();
    let last = match result {
        Ok(hash) => {
            forget_checkpoints(key);
            Piece::Hash(hash)
        }
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            if sent > 0 {
                save_checkpoint(key, sent, &hasher);
            }
            return;
        }
        Err(e) => Piece::Failed(format!("{}: {e}", path.display())),
    };
    let _ = tx.blocking_send(last);
}

fn network(e: impl std::fmt::Display) -> TransferError {
    TransferError::Network(e.to_string())
}

pub async fn send_file(
    send: &mut quinn::SendStream,
    path: &Path,
    entry: &FileEntry,
    offset: u64,
    sent: &AtomicU64,
    key: FileKey,
) -> Result<(), TransferError> {
    let state = match current_state(path) {
        Ok(state) => state,
        Err(e) => {
            let message = format!("{}: {e}", path.display());
            write_msg(send, &FileReply::Failed(message.clone()))
                .await
                .map_err(network)?;
            return Err(TransferError::Remote(message));
        }
    };
    let edited = state.0 == entry.size && entry.modified_ms != 0 && state.1 != entry.modified_ms;
    if state.0 < entry.size || edited {
        write_msg(
            send,
            &FileReply::Failed(format!(
                "{} berubah sejak dibagikan; salin ulang file tersebut",
                path.display()
            )),
        )
        .await
        .map_err(network)?;
        return Err(TransferError::Changed);
    }
    if offset > entry.size {
        write_msg(
            send,
            &FileReply::Failed("posisi lanjutan melebihi ukuran file".into()),
        )
        .await
        .map_err(network)?;
        return Err(TransferError::Remote("offset tidak valid".into()));
    }
    write_msg(send, &FileReply::Ready { size: entry.size })
        .await
        .map_err(network)?;

    let (tx, mut rx) = mpsc::channel(4);
    let reader = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        let size = entry.size;
        move || produce(path, size, offset, key, tx)
    });
    let outcome = async {
        while let Some(piece) = rx.recv().await {
            match piece {
                Piece::Data(chunk) => {
                    send.write_all(&chunk).await.map_err(network)?;
                    sent.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Piece::Hash(hash) => {
                    send.write_all(&hash).await.map_err(network)?;
                    let _ = send.finish();
                    return Ok(());
                }
                Piece::Failed(message) => {
                    let _ = send.reset(quinn::VarInt::from_u32(RESET_SOURCE_FAILED));
                    return Err(TransferError::Remote(message));
                }
            }
        }
        Err(TransferError::Remote("pembaca file berhenti mendadak".into()))
    }
    .await;
    drop(rx);
    let _ = reader.await;
    outcome
}
