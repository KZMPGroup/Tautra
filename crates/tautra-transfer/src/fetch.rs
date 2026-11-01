use std::time::Duration;

use bytes::{Buf, Bytes};
use tautra_net::{read_msg, write_msg};
use tautra_proto::kvm::{StreamHeader, StreamKind};
use tautra_proto::share::{FileReply, FileRequest, HASH_LEN};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::error::TransferError;
use crate::{CHUNK, RESET_CANCELLED, RESET_SOURCE_FAILED};

const READ_TIMEOUT: Duration = Duration::from_secs(60);
const RESUME_WAIT: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_RETRIES: u32 = 5;

fn network(e: impl std::fmt::Display) -> TransferError {
    TransferError::Network(e.to_string())
}

fn read_error(e: quinn::ReadError) -> TransferError {
    match e {
        quinn::ReadError::Reset(code) if code.into_inner() == RESET_SOURCE_FAILED as u64 => {
            TransferError::Remote("pengirim gagal membaca file di tengah transfer".into())
        }
        other => network(other),
    }
}

fn read_exact_error(e: quinn::ReadExactError) -> TransferError {
    match e {
        quinn::ReadExactError::FinishedEarly(_) => {
            network("stream berakhir sebelum hash diterima")
        }
        quinn::ReadExactError::ReadError(e) => read_error(e),
    }
}

async fn request(
    conn: &quinn::Connection,
    offer_id: u64,
    index: u32,
    offset: u64,
    size: u64,
    cancel: &CancellationToken,
) -> Result<quinn::RecvStream, TransferError> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(network)?;
    write_msg(
        &mut send,
        &StreamHeader {
            kind: StreamKind::FileGet,
        },
    )
    .await
    .map_err(network)?;
    write_msg(
        &mut send,
        &FileRequest {
            offer_id,
            index,
            offset,
        },
    )
    .await
    .map_err(network)?;
    let _ = send.finish();
    let reply: FileReply = tokio::select! {
        reply = read_msg(&mut recv) => reply.map_err(network)?,
        _ = cancel.cancelled() => return Err(TransferError::Cancelled),
    };
    match reply {
        FileReply::Ready { size: s } if s == size => Ok(recv),
        FileReply::Ready { .. } => Err(TransferError::Changed),
        FileReply::Gone => Err(TransferError::Gone),
        FileReply::Failed(message) => Err(TransferError::Remote(message)),
    }
}

pub struct BlockingReader {
    rx: mpsc::Receiver<Result<Bytes, String>>,
    pending: Bytes,
    handle: Handle,
    cancel: CancellationToken,
    done: bool,
}

impl BlockingReader {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        while self.pending.is_empty() {
            if self.done {
                return Ok(0);
            }
            let rx = &mut self.rx;
            let next = self
                .handle
                .block_on(async { tokio::time::timeout(READ_TIMEOUT, rx.recv()).await });
            match next {
                Err(_) => {
                    return Err(format!(
                        "tidak ada data dari perangkat lain selama {} detik",
                        READ_TIMEOUT.as_secs()
                    ))
                }
                Ok(None) => self.done = true,
                Ok(Some(Ok(bytes))) => self.pending = bytes,
                Ok(Some(Err(e))) => {
                    self.done = true;
                    return Err(e);
                }
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.advance(n);
        Ok(n)
    }
}

impl Drop for BlockingReader {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub fn stream_file(
    handle: &Handle,
    conns: watch::Receiver<Option<quinn::Connection>>,
    offer_id: u64,
    index: u32,
    size: u64,
) -> BlockingReader {
    let (tx, rx) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    handle.spawn(produce_stream(
        conns,
        offer_id,
        index,
        size,
        tx,
        cancel.clone(),
    ));
    BlockingReader {
        rx,
        pending: Bytes::new(),
        handle: handle.clone(),
        cancel,
        done: false,
    }
}

async fn live_connection(
    conns: &mut watch::Receiver<Option<quinn::Connection>>,
) -> Option<quinn::Connection> {
    let ready = conns.wait_for(|c| c.as_ref().is_some_and(|c| c.close_reason().is_none()));
    match tokio::time::timeout(RESUME_WAIT, ready).await {
        Ok(Ok(conn)) => conn.clone(),
        _ => None,
    }
}

struct StreamState {
    pos: u64,
    hasher: blake3::Hasher,
}

async fn stream_once(
    conn: &quinn::Connection,
    offer_id: u64,
    index: u32,
    size: u64,
    state: &mut StreamState,
    tx: &mpsc::Sender<Result<Bytes, String>>,
    cancel: &CancellationToken,
) -> Result<(), TransferError> {
    let mut recv = request(conn, offer_id, index, state.pos, size, cancel).await?;
    while state.pos < size {
        let chunk = tokio::select! {
            chunk = recv.read_chunk((size - state.pos).min(CHUNK as u64) as usize, true) => chunk.map_err(read_error)?,
            _ = cancel.cancelled() => {
                let _ = recv.stop(quinn::VarInt::from_u32(RESET_CANCELLED));
                return Err(TransferError::Cancelled);
            }
        };
        let Some(chunk) = chunk else {
            return Err(network("stream berakhir sebelum file selesai"));
        };
        state.hasher.update(&chunk.bytes);
        state.pos += chunk.bytes.len() as u64;
        if tx.send(Ok(chunk.bytes)).await.is_err() {
            return Err(TransferError::Cancelled);
        }
    }
    let mut hash = [0u8; HASH_LEN];
    recv.read_exact(&mut hash).await.map_err(read_exact_error)?;
    if state.hasher.finalize().as_bytes() != &hash {
        return Err(TransferError::HashMismatch);
    }
    Ok(())
}

async fn produce_stream(
    mut conns: watch::Receiver<Option<quinn::Connection>>,
    offer_id: u64,
    index: u32,
    size: u64,
    tx: mpsc::Sender<Result<Bytes, String>>,
    cancel: CancellationToken,
) {
    let mut state = StreamState {
        pos: 0,
        hasher: blake3::Hasher::new(),
    };
    let mut failures = 0;
    loop {
        let conn = tokio::select! {
            conn = live_connection(&mut conns) => conn,
            _ = cancel.cancelled() => return,
        };
        let Some(conn) = conn else {
            let _ = tx
                .send(Err(format!(
                    "perangkat lain tidak tersambung kembali dalam {} detik",
                    RESUME_WAIT.as_secs()
                )))
                .await;
            return;
        };
        let before = state.pos;
        match stream_once(&conn, offer_id, index, size, &mut state, &tx, &cancel).await {
            Ok(()) | Err(TransferError::Cancelled) => return,
            Err(e) if e.retryable() => {
                failures = if state.pos > before { 1 } else { failures + 1 };
                if failures > MAX_RETRIES {
                    let _ = tx.send(Err(e.to_string())).await;
                    return;
                }
                tracing::warn!(offer_id, index, pos = state.pos, "stream file terputus, dilanjutkan: {e}");
                tokio::time::sleep(RETRY_DELAY).await;
            }
            Err(e) => {
                tracing::warn!(offer_id, index, "stream file gagal: {e}");
                let _ = tx.send(Err(e.to_string())).await;
                return;
            }
        }
    }
}
