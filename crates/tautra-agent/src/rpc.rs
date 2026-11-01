use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tautra_common::paths;
use tautra_ipc::api::{
    app_error, method, Config, DeviceParams, DiagnosticsInfo, LayoutGuideParams, LayoutSetParams, NetworkNameParams,
    NoticeDismissParams,
    PairingJoinParams, ToggleParams, TransferParams,
};
use tautra_ipc::jsonrpc::RpcError;
use tautra_ipc::pipe::{HandlerFuture, RpcHandler};
use uuid::Uuid;

use crate::app::Agent;

pub struct Rpc {
    agent: Arc<Agent>,
}

impl Rpc {
    pub fn new(agent: Arc<Agent>) -> Self {
        Self { agent }
    }
}

impl RpcHandler for Rpc {
    fn handle(&self, method: String, params: Value) -> HandlerFuture {
        let agent = self.agent.clone();
        Box::pin(async move { dispatch(&agent, &method, params).await })
    }
}

fn params<T: DeserializeOwned>(value: Value) -> Result<T, RpcError> {
    serde_json::from_value(value).map_err(RpcError::invalid_params)
}

fn ok<T: Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(RpcError::internal)
}

fn device_id(raw: &str) -> Result<Uuid, RpcError> {
    Uuid::parse_str(raw.trim())
        .map_err(|_| RpcError::app(app_error::DEVICE_NOT_FOUND, "device_id tidak valid"))
}

async fn resolve_address(raw: &str, default_port: u16) -> Result<Vec<std::net::SocketAddr>, RpcError> {
    let invalid = |msg: String| RpcError::app(app_error::DEVICE_UNREACHABLE, msg);
    let target = match raw.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            let port: u16 = port
                .parse()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| invalid(format!("port {port} tidak valid")))?;
            (host.to_string(), port)
        }
        _ => (raw.to_string(), default_port),
    };
    let resolved = tokio::net::lookup_host((target.0.as_str(), target.1))
        .await
        .map_err(|e| invalid(format!("alamat {raw} tidak ditemukan: {e}")))?;
    let addrs: Vec<std::net::SocketAddr> = resolved.filter(|a| a.is_ipv4()).collect();
    if addrs.is_empty() {
        return Err(invalid(format!("alamat {raw} tidak punya IPv4")));
    }
    Ok(addrs)
}

async fn dispatch(agent: &Agent, name: &str, value: Value) -> Result<Value, RpcError> {
    match name {
        method::STATUS_GET => ok(agent.status()),
        method::CONFIG_GET => ok(agent.config()),
        method::CONFIG_SET => {
            let config: Config = params(value)?;
            ok(agent.set_config(config).await?)
        }
        method::PAIRING_HOST => ok(agent.net()?.host_pairing().await?),
        method::PAIRING_BROWSE => {
            agent.net()?.browse_for_pairing().await;
            ok(())
        }
        method::PAIRING_JOIN => {
            let p: PairingJoinParams = params(value)?;
            match p.address.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                Some(address) => {
                    let addrs = resolve_address(address, agent.config().network.port).await?;
                    agent
                        .net()?
                        .join_pairing_at(addrs, address.to_string(), p.pin)
                        .await?;
                }
                None => {
                    agent
                        .net()?
                        .join_pairing(device_id(&p.device_id)?, p.pin)
                        .await?;
                }
            }
            ok(())
        }
        method::PAIRING_CANCEL => {
            agent.net()?.cancel_pairing().await;
            ok(())
        }
        method::DEVICE_REMOVE => {
            let p: DeviceParams = params(value)?;
            agent.net()?.remove_device(device_id(&p.device_id)?).await?;
            ok(())
        }
        method::CONNECTION_DISCONNECT => {
            agent.net()?.disconnect().await;
            ok(())
        }
        method::CONNECTION_RESUME => {
            agent.net()?.resume().await;
            ok(())
        }
        method::NOTICE_DISMISS => {
            let p: NoticeDismissParams = params(value)?;
            agent.dismiss_notice(p.id);
            ok(())
        }
        method::NETWORK_FORGET => {
            let p: NetworkNameParams = params(value)?;
            agent.net()?.forget_network(p.name).await;
            ok(())
        }
        method::DIAGNOSTICS_GET => {
            let usage = crate::metrics::sample().await;
            let display = |r: Result<std::path::PathBuf, paths::PathError>| {
                r.map(|p| p.display().to_string()).unwrap_or_default()
            };
            ok(DiagnosticsInfo {
                agent_pid: std::process::id(),
                config_path: agent.config_path.display().to_string(),
                log_dir: display(paths::log_dir()),
                data_dir: display(paths::local_dir()),
                cpu_percent: usage.cpu_percent,
                working_set_mb: usage.working_set_mb,
                private_mb: usage.private_mb,
            })
        }
        method::LAYOUT_SET => {
            let p: LayoutSetParams = params(value)?;
            agent.kvm.set_layout(p).await?;
            ok(())
        }
        method::LAYOUT_GUIDE => {
            let p: LayoutGuideParams = params(value)?;
            agent.kvm.set_guide(p)?;
            ok(())
        }
        method::KVM_SET_LOCK => {
            let p: ToggleParams = params(value)?;
            agent.kvm.set_locked(p.enabled);
            agent.touch();
            ok(())
        }
        method::KVM_SET_GAMING => {
            let p: ToggleParams = params(value)?;
            agent.kvm.set_gaming(p.enabled);
            agent.touch();
            ok(())
        }
        method::COMPAT_GET => ok(crate::compat::report()),
        method::SERVICE_STATUS => {
            ok(tokio::task::spawn_blocking(crate::service::status)
                .await
                .map_err(RpcError::internal)?)
        }
        method::SERVICE_INSTALL => {
            tokio::task::spawn_blocking(crate::service::install)
                .await
                .map_err(RpcError::internal)?
                .map_err(|e| RpcError::app(app_error::SERVICE_FAILED, e))?;
            ok(tokio::task::spawn_blocking(crate::service::status)
                .await
                .map_err(RpcError::internal)?)
        }
        method::SERVICE_UNINSTALL => {
            tokio::task::spawn_blocking(crate::service::uninstall)
                .await
                .map_err(RpcError::internal)?
                .map_err(|e| RpcError::app(app_error::SERVICE_FAILED, e))?;
            ok(tokio::task::spawn_blocking(crate::service::status)
                .await
                .map_err(RpcError::internal)?)
        }
        method::TRANSFER_CANCEL => {
            let p: TransferParams = params(value)?;
            agent.share.cancel_transfer(p.id)?;
            agent.touch();
            ok(())
        }
        method::TRANSFER_REVEAL => {
            let p: TransferParams = params(value)?;
            agent.share.reveal_transfer(p.id)?;
            ok(())
        }
        method::TRANSFER_CLEAR => {
            agent.share.clear_transfers();
            agent.touch();
            ok(())
        }
        method::AGENT_SHUTDOWN => {
            tracing::info!("agent diminta berhenti oleh UI");
            agent.cancel.cancel();
            ok(())
        }
        other => Err(RpcError::method_not_found(other)),
    }
}
