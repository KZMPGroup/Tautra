use hmac::{Hmac, Mac};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tautra_ipc::api::app_error;
use tautra_common::build;
use tautra_proto::control::HandshakeHeader;
use tautra_proto::pairing::{
    PairingMsg, PairingRejectReason, CONFIRM_LABEL_CLIENT, CONFIRM_LABEL_SERVER, MAC_LEN,
};
use tautra_proto::{close_code, MAX_DEVICE_NAME};
use uuid::Uuid;

use crate::error::NetError;
use crate::fingerprint::Fingerprint;
use crate::framing::{read_msg, write_msg, StreamError};
use crate::session::LocalInfo;
use crate::tls;

pub const PIN_LEN: usize = 6;

#[derive(Debug, Clone)]
pub struct PairedPeer {
    pub device_id: Uuid,
    pub device_name: String,
    pub fingerprint: Fingerprint,
    pub remote: std::net::SocketAddr,
}

#[derive(Debug, thiserror::Error)]
pub enum PairingFailure {
    #[error("PIN salah")]
    WrongPin,
    #[error("terlalu banyak percobaan PIN")]
    TooManyAttempts,
    #[error("perangkat tujuan tidak sedang menampilkan PIN")]
    NotHosting,
    #[error("perangkat tujuan sedang melayani pairing lain")]
    Busy,
    #[error("konfirmasi kunci gagal (kemungkinan PIN salah atau ada pihak ketiga di jaringan)")]
    Mismatch,
    #[error(
        "versi aplikasi berbeda (perangkat lain: {peer_app_version} build {peer_build}, protokol {peer_protocol})"
    )]
    VersionMismatch {
        peer_app_version: String,
        peer_protocol: u32,
        peer_build: u64,
    },
    #[error("pesan pairing tidak sesuai: {0}")]
    Protocol(String),
    #[error("tidak bisa menghubungi perangkat: {0}")]
    Unreachable(String),
    #[error("pairing melebihi batas waktu")]
    Timeout,
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error("koneksi pairing gagal: {0}")]
    Connection(#[from] quinn::ConnectionError),
}

impl PairingFailure {
    pub fn to_net_error(&self) -> NetError {
        let code = match self {
            PairingFailure::WrongPin | PairingFailure::Mismatch => app_error::PAIRING_WRONG_PIN,
            PairingFailure::TooManyAttempts => app_error::PAIRING_TOO_MANY_ATTEMPTS,
            PairingFailure::NotHosting => app_error::PAIRING_NOT_HOSTING,
            PairingFailure::Busy => app_error::PAIRING_BUSY,
            PairingFailure::VersionMismatch { .. } => app_error::PROTOCOL_VERSION_MISMATCH,
            PairingFailure::Unreachable(_) => app_error::DEVICE_UNREACHABLE,
            PairingFailure::Timeout => app_error::PAIRING_TIMEOUT,
            _ => app_error::PAIRING_FAILED,
        };
        NetError::new(code, self.to_string())
    }
}

pub fn is_valid_pin(pin: &str) -> bool {
    pin.len() == PIN_LEN && pin.bytes().all(|b| b.is_ascii_digit())
}

pub fn generate_pin() -> String {
    use rand::Rng;
    let n: u32 = rand::rngs::OsRng.gen_range(0..1_000_000);
    format!("{n:06}")
}

fn confirm_mac(
    key: &[u8],
    label: &[u8],
    first: &Fingerprint,
    second: &Fingerprint,
) -> Hmac<Sha256> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC menerima kunci panjang berapa pun");
    mac.update(label);
    mac.update(&first.0);
    mac.update(&second.0);
    mac
}

fn mac_bytes(mac: Hmac<Sha256>) -> [u8; MAC_LEN] {
    mac.finalize().into_bytes().into()
}

fn reject_to_failure(reason: PairingRejectReason) -> PairingFailure {
    match reason {
        PairingRejectReason::NotHosting => PairingFailure::NotHosting,
        PairingRejectReason::WrongPin => PairingFailure::WrongPin,
        PairingRejectReason::TooManyAttempts => PairingFailure::TooManyAttempts,
        PairingRejectReason::Busy => PairingFailure::Busy,
    }
}

fn clamp_name(name: String) -> String {
    name.chars().take(MAX_DEVICE_NAME).collect()
}

fn check_header(peer: &HandshakeHeader) -> Result<(), PairingFailure> {
    if !peer.is_valid_magic() {
        return Err(PairingFailure::Protocol("magic handshake salah".into()));
    }
    if !peer.is_pairing_compatible() {
        return Err(PairingFailure::VersionMismatch {
            peer_app_version: peer.app_version.clone(),
            peer_protocol: peer.protocol_version,
            peer_build: peer.build,
        });
    }
    Ok(())
}

pub async fn join(
    conn: &quinn::Connection,
    local: &LocalInfo,
    local_fp: Fingerprint,
    host_id: Option<Uuid>,
    pin: &str,
) -> Result<PairedPeer, PairingFailure> {
    let peer_fp = tls::peer_fingerprint(conn)
        .ok_or_else(|| PairingFailure::Protocol("sertifikat peer tidak ada".into()))?;
    let (mut send, mut recv) = conn.open_bi().await?;

    write_msg(&mut send, &HandshakeHeader::current(&local.app_version, build::id())).await?;
    let header: HandshakeHeader = read_msg(&mut recv).await?;
    check_header(&header)?;

    let host_id = match host_id {
        Some(id) => id,
        None => {
            write_msg(&mut send, &PairingMsg::Identify).await?;
            match read_msg::<PairingMsg>(&mut recv).await? {
                PairingMsg::Identity { device_id, .. } => {
                    let id = Uuid::from_bytes(device_id);
                    if id == local.device_id {
                        return Err(PairingFailure::Protocol(
                            "alamat itu adalah perangkat ini sendiri".into(),
                        ));
                    }
                    id
                }
                PairingMsg::Rejected { reason } => return Err(reject_to_failure(reason)),
                other => {
                    return Err(PairingFailure::Protocol(format!(
                        "mengharapkan Identity, dapat {other:?}"
                    )))
                }
            }
        }
    };

    let (spake, msg_a) = Spake2::<Ed25519Group>::start_a(
        &Password::new(pin.as_bytes()),
        &Identity::new(local.device_id.as_bytes()),
        &Identity::new(host_id.as_bytes()),
    );
    write_msg(
        &mut send,
        &PairingMsg::Request {
            device_id: *local.device_id.as_bytes(),
            device_name: local.device_name.clone(),
            spake: msg_a,
        },
    )
    .await?;

    let (host_name, msg_b) = match read_msg::<PairingMsg>(&mut recv).await? {
        PairingMsg::Response {
            device_id,
            device_name,
            spake,
        } => {
            if Uuid::from_bytes(device_id) != host_id {
                return Err(PairingFailure::Protocol(
                    "device_id perangkat tujuan berbeda".into(),
                ));
            }
            (device_name, spake)
        }
        PairingMsg::Rejected { reason } => return Err(reject_to_failure(reason)),
        other => {
            return Err(PairingFailure::Protocol(format!(
                "mengharapkan Response, dapat {other:?}"
            )))
        }
    };
    let key = spake
        .finish(&msg_b)
        .map_err(|e| PairingFailure::Protocol(format!("SPAKE2: {e:?}")))?;

    let mac = mac_bytes(confirm_mac(&key, CONFIRM_LABEL_CLIENT, &local_fp, &peer_fp));
    write_msg(&mut send, &PairingMsg::Confirm { mac }).await?;

    match read_msg::<PairingMsg>(&mut recv).await? {
        PairingMsg::ConfirmAck { mac } => {
            confirm_mac(&key, CONFIRM_LABEL_SERVER, &peer_fp, &local_fp)
                .verify_slice(&mac)
                .map_err(|_| PairingFailure::Mismatch)?;
        }
        PairingMsg::Rejected { reason } => return Err(reject_to_failure(reason)),
        other => {
            return Err(PairingFailure::Protocol(format!(
                "mengharapkan ConfirmAck, dapat {other:?}"
            )))
        }
    }
    let _ = send.finish();
    conn.close(close_code::PAIRING_DONE.into(), b"paired");

    Ok(PairedPeer {
        device_id: host_id,
        device_name: clamp_name(host_name),
        fingerprint: peer_fp,
        remote: conn.remote_address(),
    })
}

pub async fn host(
    conn: &quinn::Connection,
    local: &LocalInfo,
    local_fp: Fingerprint,
    pin: &str,
) -> Result<PairedPeer, PairingFailure> {
    let peer_fp = tls::peer_fingerprint(conn)
        .ok_or_else(|| PairingFailure::Protocol("sertifikat peer tidak ada".into()))?;
    let (mut send, mut recv) = conn.accept_bi().await?;

    let header: HandshakeHeader = read_msg(&mut recv).await?;
    write_msg(&mut send, &HandshakeHeader::current(&local.app_version, build::id())).await?;
    check_header(&header)?;

    let mut first = read_msg::<PairingMsg>(&mut recv).await?;
    if first == PairingMsg::Identify {
        write_msg(
            &mut send,
            &PairingMsg::Identity {
                device_id: *local.device_id.as_bytes(),
                device_name: local.device_name.clone(),
            },
        )
        .await?;
        first = read_msg::<PairingMsg>(&mut recv).await?;
    }
    let (joiner_id, joiner_name, msg_a) = match first {
        PairingMsg::Request {
            device_id,
            device_name,
            spake,
        } => (Uuid::from_bytes(device_id), device_name, spake),
        other => {
            return Err(PairingFailure::Protocol(format!(
                "mengharapkan Request, dapat {other:?}"
            )))
        }
    };
    if joiner_id == local.device_id {
        return Err(PairingFailure::Protocol(
            "perangkat tidak bisa pair dengan dirinya sendiri".into(),
        ));
    }

    let (spake, msg_b) = Spake2::<Ed25519Group>::start_b(
        &Password::new(pin.as_bytes()),
        &Identity::new(joiner_id.as_bytes()),
        &Identity::new(local.device_id.as_bytes()),
    );
    write_msg(
        &mut send,
        &PairingMsg::Response {
            device_id: *local.device_id.as_bytes(),
            device_name: local.device_name.clone(),
            spake: msg_b,
        },
    )
    .await?;
    let key = spake
        .finish(&msg_a)
        .map_err(|e| PairingFailure::Protocol(format!("SPAKE2: {e:?}")))?;

    let mac = match read_msg::<PairingMsg>(&mut recv).await? {
        PairingMsg::Confirm { mac } => mac,
        other => {
            return Err(PairingFailure::Protocol(format!(
                "mengharapkan Confirm, dapat {other:?}"
            )))
        }
    };
    if confirm_mac(&key, CONFIRM_LABEL_CLIENT, &peer_fp, &local_fp)
        .verify_slice(&mac)
        .is_err()
    {
        let _ = write_msg(
            &mut send,
            &PairingMsg::Rejected {
                reason: PairingRejectReason::WrongPin,
            },
        )
        .await;
        let _ = send.finish();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), conn.closed()).await;
        return Err(PairingFailure::WrongPin);
    }

    let ack = mac_bytes(confirm_mac(&key, CONFIRM_LABEL_SERVER, &local_fp, &peer_fp));
    write_msg(&mut send, &PairingMsg::ConfirmAck { mac: ack }).await?;
    let _ = send.finish();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), conn.closed()).await;

    Ok(PairedPeer {
        device_id: joiner_id,
        device_name: clamp_name(joiner_name),
        fingerprint: peer_fp,
        remote: conn.remote_address(),
    })
}

pub async fn reject(conn: quinn::Connection, local: LocalInfo, reason: PairingRejectReason) {
    let result: Result<(), PairingFailure> = async {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let _header: HandshakeHeader = read_msg(&mut recv).await?;
        write_msg(&mut send, &HandshakeHeader::current(&local.app_version, build::id())).await?;
        let _request: PairingMsg = read_msg(&mut recv).await?;
        write_msg(&mut send, &PairingMsg::Rejected { reason }).await?;
        let _ = send.finish();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), conn.closed()).await;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("penolakan pairing tidak terkirim: {e}");
    }
    conn.close(close_code::PAIRING_FAILED.into(), b"rejected");
}
