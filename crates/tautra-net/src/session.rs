use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tautra_common::build;
use tautra_proto::close_code;
use tautra_proto::control::{Control, GoodbyeReason, HandshakeHeader, Hello};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::framing::{read_msg, write_msg, StreamError};
use crate::tls;
use crate::trust::TrustStore;

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(3);
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct LocalInfo {
    pub device_id: Uuid,
    pub device_name: String,
    pub app_version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("perangkat tidak ada di daftar trusted")]
    Untrusted,
    #[error("perangkat lain menolak: perangkat ini tidak ada di daftar trusted-nya")]
    RejectedUntrusted,
    #[error("perangkat lain menolak koneksi (kode {0})")]
    Rejected(u32),
    #[error(
        "versi aplikasi berbeda (perangkat lain: {peer_app_version} build {peer_build}, protokol {peer_protocol})"
    )]
    VersionMismatch {
        peer_app_version: String,
        peer_protocol: u32,
        peer_build: u64,
    },
    #[error("identitas perangkat tidak cocok dengan yang di-pair")]
    WrongDevice,
    #[error("pesan handshake tidak sesuai: {0}")]
    Protocol(String),
    #[error("handshake melebihi batas waktu")]
    Timeout,
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error("koneksi gagal: {0}")]
    Connection(#[from] quinn::ConnectionError),
}

impl HandshakeError {
    fn classify(self) -> Self {
        let code = match &self {
            HandshakeError::Connection(quinn::ConnectionError::ApplicationClosed(c)) => {
                Some(c.error_code.into_inner() as u32)
            }
            HandshakeError::Stream(StreamError::Read(quinn::ReadError::ConnectionLost(
                quinn::ConnectionError::ApplicationClosed(c),
            )))
            | HandshakeError::Stream(StreamError::Write(quinn::WriteError::ConnectionLost(
                quinn::ConnectionError::ApplicationClosed(c),
            ))) => Some(c.error_code.into_inner() as u32),
            _ => None,
        };
        match code {
            Some(close_code::UNTRUSTED) => HandshakeError::RejectedUntrusted,
            Some(code) => HandshakeError::Rejected(code),
            None => self,
        }
    }
}

pub struct Established {
    pub conn: quinn::Connection,
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    pub peer_id: Uuid,
    pub peer_name: String,
    pub dialer: bool,
    pub remote: SocketAddr,
}

fn check_header(peer: &HandshakeHeader) -> Result<(), HandshakeError> {
    if !peer.is_valid_magic() {
        return Err(HandshakeError::Protocol("magic handshake salah".into()));
    }
    if !peer.is_compatible(build::id()) {
        return Err(HandshakeError::VersionMismatch {
            peer_app_version: peer.app_version.clone(),
            peer_protocol: peer.protocol_version,
            peer_build: peer.build,
        });
    }
    Ok(())
}

fn clamp_name(name: String) -> String {
    name.chars().take(tautra_proto::MAX_DEVICE_NAME).collect()
}

pub async fn handshake_dialer(
    conn: quinn::Connection,
    local: &LocalInfo,
    expected_id: Uuid,
) -> Result<Established, HandshakeError> {
    let result: Result<Established, HandshakeError> = async {
        let (mut send, mut recv) = conn.open_bi().await?;
        write_msg(&mut send, &HandshakeHeader::current(&local.app_version, build::id())).await?;
        write_msg(
            &mut send,
            &Hello {
                device_id: *local.device_id.as_bytes(),
                device_name: local.device_name.clone(),
            },
        )
        .await?;
        let header: HandshakeHeader = read_msg(&mut recv).await?;
        check_header(&header)?;
        let hello: Hello = read_msg(&mut recv).await?;
        if Uuid::from_bytes(hello.device_id) != expected_id {
            return Err(HandshakeError::WrongDevice);
        }
        Ok(Established {
            remote: conn.remote_address(),
            conn: conn.clone(),
            send,
            recv,
            peer_id: expected_id,
            peer_name: clamp_name(hello.device_name),
            dialer: true,
        })
    }
    .await;
    result.map_err(|e| {
        let e = e.classify();
        let code = match e {
            HandshakeError::VersionMismatch { .. } => close_code::VERSION_MISMATCH,
            HandshakeError::WrongDevice => close_code::UNTRUSTED,
            _ => close_code::PROTOCOL_ERROR,
        };
        conn.close(code.into(), b"handshake");
        e
    })
}

pub async fn handshake_acceptor(
    conn: quinn::Connection,
    local: &LocalInfo,
    trust: &Arc<RwLock<TrustStore>>,
) -> Result<Established, HandshakeError> {
    let entry = tls::peer_fingerprint(&conn).and_then(|fp| {
        trust
            .read()
            .ok()
            .and_then(|store| store.by_fingerprint(&fp).cloned())
    });
    let Some(entry) = entry else {
        conn.close(close_code::UNTRUSTED.into(), b"untrusted");
        return Err(HandshakeError::Untrusted);
    };

    let result: Result<Established, HandshakeError> = async {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let header: HandshakeHeader = read_msg(&mut recv).await?;
        write_msg(&mut send, &HandshakeHeader::current(&local.app_version, build::id())).await?;
        check_header(&header)?;
        let hello: Hello = read_msg(&mut recv).await?;
        if Uuid::from_bytes(hello.device_id) != entry.device_id {
            return Err(HandshakeError::WrongDevice);
        }
        write_msg(
            &mut send,
            &Hello {
                device_id: *local.device_id.as_bytes(),
                device_name: local.device_name.clone(),
            },
        )
        .await?;
        Ok(Established {
            remote: conn.remote_address(),
            conn: conn.clone(),
            send,
            recv,
            peer_id: entry.device_id,
            peer_name: clamp_name(hello.device_name),
            dialer: false,
        })
    }
    .await;
    result.map_err(|e| {
        let code = match e {
            HandshakeError::VersionMismatch { .. } => close_code::VERSION_MISMATCH,
            HandshakeError::WrongDevice => close_code::UNTRUSTED,
            _ => close_code::PROTOCOL_ERROR,
        };
        conn.close(code.into(), b"handshake");
        e.classify()
    })
}

#[derive(Debug)]
pub enum SessionEvent {
    Rtt {
        session_id: u64,
        rtt: Duration,
    },
    Renamed {
        session_id: u64,
        device_name: String,
    },
    Ended {
        session_id: u64,
        reason: EndReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    Local,
    PeerClosed(u32),
    Goodbye(GoodbyeReason),
    Timeout,
    Lost(String),
}

impl EndReason {
    fn from_connection_error(err: &quinn::ConnectionError) -> Self {
        match err {
            quinn::ConnectionError::ApplicationClosed(c) => {
                EndReason::PeerClosed(c.error_code.into_inner() as u32)
            }
            quinn::ConnectionError::LocallyClosed => EndReason::Local,
            quinn::ConnectionError::TimedOut => EndReason::Timeout,
            other => EndReason::Lost(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub id: u64,
    pub device_id: Uuid,
    pub device_name: String,
    pub conn: quinn::Connection,
    control_tx: mpsc::Sender<Control>,
}

impl SessionHandle {
    pub async fn send_control(&self, msg: Control) -> bool {
        self.control_tx.send(msg).await.is_ok()
    }

    pub fn try_send_control(&self, msg: Control) -> bool {
        self.control_tx.try_send(msg).is_ok()
    }

    pub fn close(&self, code: u32, reason: &[u8]) {
        self.conn.close(code.into(), reason);
    }
}

pub fn spawn(
    est: Established,
    session_id: u64,
    events: mpsc::Sender<SessionEvent>,
) -> SessionHandle {
    let (control_tx, control_rx) = mpsc::channel(64);
    let handle = SessionHandle {
        id: session_id,
        device_id: est.peer_id,
        device_name: est.peer_name.clone(),
        conn: est.conn.clone(),
        control_tx,
    };
    tokio::spawn(run(est, session_id, control_rx, events));
    handle
}

async fn run(
    est: Established,
    session_id: u64,
    mut control_rx: mpsc::Receiver<Control>,
    events: mpsc::Sender<SessionEvent>,
) {
    let Established {
        conn,
        mut send,
        mut recv,
        ..
    } = est;
    let epoch = Instant::now();

    let (in_tx, mut in_rx) = mpsc::channel::<Result<Control, StreamError>>(32);
    let reader = tokio::spawn(async move {
        loop {
            let msg = read_msg::<Control>(&mut recv).await;
            let stop = msg.is_err();
            if in_tx.send(msg).await.is_err() || stop {
                break;
            }
        }
    });

    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut seq: u32 = 0;
    let mut last_pong = Instant::now();

    let reason = loop {
        tokio::select! {
            _ = ticker.tick() => {
                if last_pong.elapsed() > HEARTBEAT_TIMEOUT {
                    conn.close(close_code::HEARTBEAT_TIMEOUT.into(), b"heartbeat");
                    break EndReason::Timeout;
                }
                seq = seq.wrapping_add(1);
                let ping = Control::Ping { seq, sent_us: epoch.elapsed().as_micros() as u64 };
                if let Err(e) = write_msg(&mut send, &ping).await {
                    break end_from_stream(&conn, e);
                }
            }
            msg = in_rx.recv() => match msg {
                Some(Ok(Control::Ping { seq, sent_us })) => {
                    if let Err(e) = write_msg(&mut send, &Control::Pong { seq, sent_us }).await {
                        break end_from_stream(&conn, e);
                    }
                }
                Some(Ok(Control::Pong { sent_us, .. })) => {
                    last_pong = Instant::now();
                    let now_us = epoch.elapsed().as_micros() as u64;
                    let rtt = Duration::from_micros(now_us.saturating_sub(sent_us));
                    let _ = events.try_send(SessionEvent::Rtt { session_id, rtt });
                }
                Some(Ok(Control::Goodbye { reason })) => {
                    conn.close(close_code::NORMAL.into(), b"goodbye");
                    break EndReason::Goodbye(reason);
                }
                Some(Ok(Control::Rename { device_name })) => {
                    let _ = events
                        .send(SessionEvent::Renamed { session_id, device_name: clamp_name(device_name) })
                        .await;
                }
                Some(Err(e)) => break end_from_stream(&conn, e),
                None => break end_from_conn(&conn),
            },
            Some(msg) = control_rx.recv() => {
                if let Err(e) = write_msg(&mut send, &msg).await {
                    break end_from_stream(&conn, e);
                }
            }
            err = conn.closed() => break EndReason::from_connection_error(&err),
        }
    };
    reader.abort();
    tracing::info!(session_id, ?reason, "sesi berakhir");
    let _ = events
        .send(SessionEvent::Ended { session_id, reason })
        .await;
}

fn end_from_conn(conn: &quinn::Connection) -> EndReason {
    conn.close_reason()
        .map(|e| EndReason::from_connection_error(&e))
        .unwrap_or(EndReason::Lost("stream control ditutup".into()))
}

fn end_from_stream(conn: &quinn::Connection, err: StreamError) -> EndReason {
    match conn.close_reason() {
        Some(e) => EndReason::from_connection_error(&e),
        None => {
            conn.close(close_code::PROTOCOL_ERROR.into(), b"control stream");
            EndReason::Lost(err.to_string())
        }
    }
}
