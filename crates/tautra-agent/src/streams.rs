use std::sync::Arc;

use tautra_net::{read_msg, SessionHandle, StreamError};
use tautra_proto::kvm::{KvmMessage, StreamHeader, StreamKind};
use tautra_proto::share::ShareMessage;
use tokio::sync::mpsc;

use crate::app::Agent;
use crate::share;

pub fn start(agent: Arc<Agent>, session: SessionHandle) {
    let kvm_tx = agent.kvm.session_started(agent.clone(), session.clone());
    agent.share.session_started(&agent, &session);
    tokio::spawn(accept_uni(agent.clone(), session.conn.clone(), kvm_tx));
    tokio::spawn(accept_bi(agent, session.conn));
}

async fn forward_kvm(mut recv: quinn::RecvStream, tx: mpsc::Sender<KvmMessage>) {
    loop {
        match read_msg::<KvmMessage>(&mut recv).await {
            Ok(msg) => {
                if tx.send(msg).await.is_err() {
                    return;
                }
            }
            Err(StreamError::Closed) => return,
            Err(e) => {
                tracing::warn!("stream KVM dari perangkat lain terputus: {e}");
                return;
            }
        }
    }
}

async fn forward_share(agent: Arc<Agent>, mut recv: quinn::RecvStream) {
    loop {
        match read_msg::<ShareMessage>(&mut recv).await {
            Ok(msg) => agent.share.on_message(&agent, msg),
            Err(StreamError::Closed) => return,
            Err(e) => {
                tracing::warn!("stream clipboard/file dari perangkat lain terputus: {e}");
                return;
            }
        }
    }
}

async fn accept_uni(agent: Arc<Agent>, conn: quinn::Connection, kvm_tx: mpsc::Sender<KvmMessage>) {
    loop {
        let mut recv = match conn.accept_uni().await {
            Ok(recv) => recv,
            Err(_) => return,
        };
        let header: StreamHeader = match read_msg(&mut recv).await {
            Ok(header) => header,
            Err(e) => {
                tracing::warn!("header stream dari perangkat lain tidak valid: {e}");
                continue;
            }
        };
        match header.kind {
            StreamKind::Kvm => {
                tokio::spawn(forward_kvm(recv, kvm_tx.clone()));
            }
            StreamKind::Share => {
                tokio::spawn(forward_share(agent.clone(), recv));
            }
            other => tracing::warn!(?other, "jenis stream satu arah tidak dikenal"),
        }
    }
}

async fn accept_bi(agent: Arc<Agent>, conn: quinn::Connection) {
    loop {
        let (send, mut recv) = match conn.accept_bi().await {
            Ok(pair) => pair,
            Err(_) => return,
        };
        let agent = agent.clone();
        tokio::spawn(async move {
            let header: StreamHeader = match read_msg(&mut recv).await {
                Ok(header) => header,
                Err(e) => {
                    tracing::warn!("header stream dari perangkat lain tidak valid: {e}");
                    return;
                }
            };
            match header.kind {
                StreamKind::ClipFetch => share::serve_clip(agent, send, recv).await,
                StreamKind::FileGet => share::serve_file(agent, send, recv).await,
                other => tracing::warn!(?other, "jenis stream dua arah tidak dikenal"),
            }
        });
    }
}
