use std::sync::Arc;
use std::time::Duration;

use tautra_net::{read_msg, write_msg};
use tautra_proto::kvm::{StreamHeader, StreamKind};
use tautra_proto::share::{
    ClipFetchReply, ClipFetchRequest, ClipFormat, FileReply, FileRequest, ShareMessage,
};
use tautra_transfer::TransferError;
use tokio::sync::{mpsc, watch};

use crate::app::Agent;

pub async fn write_loop(conn: quinn::Connection, mut rx: mpsc::Receiver<ShareMessage>) {
    let mut send = match conn.open_uni().await {
        Ok(send) => send,
        Err(e) => {
            tracing::debug!("stream clipboard/file tidak bisa dibuka: {e}");
            return;
        }
    };
    if let Err(e) = write_msg(
        &mut send,
        &StreamHeader {
            kind: StreamKind::Share,
        },
    )
    .await
    {
        tracing::debug!("header stream clipboard/file gagal dikirim: {e}");
        return;
    }
    while let Some(msg) = rx.recv().await {
        if let Err(e) = write_msg(&mut send, &msg).await {
            tracing::warn!("stream clipboard/file gagal ditulis: {e}");
            return;
        }
    }
}

const CLIP_RESUME_WAIT: Duration = Duration::from_secs(10);
const CLIP_RETRIES: u32 = 3;

async fn live_connection(
    conns: &mut watch::Receiver<Option<quinn::Connection>>,
) -> Option<quinn::Connection> {
    let ready = conns.wait_for(|c| c.as_ref().is_some_and(|c| c.close_reason().is_none()));
    match tokio::time::timeout(CLIP_RESUME_WAIT, ready).await {
        Ok(Ok(conn)) => conn.clone(),
        _ => None,
    }
}

pub async fn fetch_clip(
    mut conns: watch::Receiver<Option<quinn::Connection>>,
    offer_id: u64,
    format: ClipFormat,
    max: u64,
) -> Result<Vec<u8>, String> {
    let mut attempt = 0;
    loop {
        let Some(conn) = live_connection(&mut conns).await else {
            return Err("perangkat lain tidak tersambung".into());
        };
        match fetch_clip_once(&conn, offer_id, format, max).await {
            Ok(data) => return Ok(data),
            Err(e) if e.retry && attempt < CLIP_RETRIES => {
                attempt += 1;
                tracing::info!(offer_id, "pengambilan clipboard terputus, diulang: {}", e.message);
            }
            Err(e) => return Err(e.message),
        }
    }
}

struct ClipError {
    message: String,
    retry: bool,
}

fn transient(e: impl std::fmt::Display) -> ClipError {
    ClipError {
        message: e.to_string(),
        retry: true,
    }
}

fn permanent(message: String) -> ClipError {
    ClipError {
        message,
        retry: false,
    }
}

async fn fetch_clip_once(
    conn: &quinn::Connection,
    offer_id: u64,
    format: ClipFormat,
    max: u64,
) -> Result<Vec<u8>, ClipError> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(transient)?;
    write_msg(
        &mut send,
        &StreamHeader {
            kind: StreamKind::ClipFetch,
        },
    )
    .await
    .map_err(transient)?;
    write_msg(&mut send, &ClipFetchRequest { offer_id, format })
        .await
        .map_err(transient)?;
    let _ = send.finish();
    match read_msg::<ClipFetchReply>(&mut recv)
        .await
        .map_err(transient)?
    {
        ClipFetchReply::Data { size } if size <= max => {
            let mut data = vec![0u8; size as usize];
            recv.read_exact(&mut data)
                .await
                .map_err(|e| transient(format!("isi clipboard terputus: {e}")))?;
            Ok(data)
        }
        ClipFetchReply::Data { size } => Err(permanent(format!(
            "isi clipboard {} MB melebihi batas di perangkat ini",
            size / (1024 * 1024)
        ))),
        ClipFetchReply::Gone => Err(permanent(
            "isi clipboard sudah diganti di perangkat lain; salin ulang".into(),
        )),
    }
}

pub async fn serve_clip(agent: Arc<Agent>, mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
    let request: ClipFetchRequest = match read_msg(&mut recv).await {
        Ok(request) => request,
        Err(e) => {
            tracing::debug!("permintaan clipboard tidak valid: {e}");
            return;
        }
    };
    let result = match agent.share.clip_data(request.offer_id, request.format) {
        Some(data) => {
            let header = write_msg(
                &mut send,
                &ClipFetchReply::Data {
                    size: data.len() as u64,
                },
            )
            .await;
            match header {
                Ok(()) => send.write_all(&data).await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
        None => write_msg(&mut send, &ClipFetchReply::Gone)
            .await
            .map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => {
            let _ = send.finish();
        }
        Err(e) => tracing::debug!("isi clipboard gagal dikirim: {e}"),
    }
}

pub async fn serve_file(agent: Arc<Agent>, mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
    let request: FileRequest = match read_msg(&mut recv).await {
        Ok(request) => request,
        Err(e) => {
            tracing::debug!("permintaan file tidak valid: {e}");
            return;
        }
    };
    let Some(found) = agent.share.file_lookup(request.offer_id, request.index) else {
        if write_msg(&mut send, &FileReply::Gone).await.is_ok() {
            let _ = send.finish();
        }
        return;
    };
    tracing::debug!(
        offer = request.offer_id,
        index = request.index,
        offset = request.offset,
        path = %found.path.display(),
        "mengirim file"
    );
    let key = (request.offer_id, request.index);
    match tautra_transfer::send_file(
        &mut send,
        &found.path,
        &found.entry,
        request.offset,
        &found.sent,
        key,
    )
    .await
    {
        Ok(()) => {
            agent.share.file_served(request.offer_id, request.index);
            agent.touch();
        }
        Err(TransferError::Network(e)) => {
            tracing::debug!(offer = request.offer_id, "pengiriman file terputus: {e}")
        }
        Err(e) => tracing::warn!(offer = request.offer_id, "file gagal dikirim: {e}"),
    }
}
