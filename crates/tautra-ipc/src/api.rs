use serde::{Deserialize, Serialize};

pub use tautra_common::config::Config;

pub mod method {
    pub const STATUS_GET: &str = "status.get";
    pub const CONFIG_GET: &str = "config.get";
    pub const CONFIG_SET: &str = "config.set";
    pub const PAIRING_HOST: &str = "pairing.host";
    pub const PAIRING_BROWSE: &str = "pairing.browse";
    pub const PAIRING_JOIN: &str = "pairing.join";
    pub const PAIRING_CANCEL: &str = "pairing.cancel";
    pub const DEVICE_REMOVE: &str = "device.remove";
    pub const CONNECTION_DISCONNECT: &str = "connection.disconnect";
    pub const CONNECTION_RESUME: &str = "connection.resume";
    pub const NOTICE_DISMISS: &str = "notice.dismiss";
    pub const NETWORK_FORGET: &str = "network.forget";
    pub const DIAGNOSTICS_GET: &str = "diagnostics.get";
    pub const COMPAT_GET: &str = "compat.get";
    pub const AGENT_SHUTDOWN: &str = "agent.shutdown";
    pub const LAYOUT_SET: &str = "layout.set";
    pub const LAYOUT_GUIDE: &str = "layout.guide";
    pub const KVM_SET_LOCK: &str = "kvm.set_lock";
    pub const KVM_SET_GAMING: &str = "kvm.set_gaming";
    pub const SERVICE_STATUS: &str = "service.status";
    pub const SERVICE_INSTALL: &str = "service.install";
    pub const SERVICE_UNINSTALL: &str = "service.uninstall";
    pub const TRANSFER_REVEAL: &str = "transfer.reveal";
    pub const TRANSFER_CANCEL: &str = "transfer.cancel";
    pub const TRANSFER_CLEAR: &str = "transfer.clear";
}

pub mod notification {
    pub const STATUS_CHANGED: &str = "status.changed";
    pub const UPDATE_APPLIED: &str = "update.applied";
}

pub mod app_error {
    pub const CONFIG_INVALID: &str = "config.invalid";
    pub const CONFIG_SAVE_FAILED: &str = "config.save_failed";
    pub const CONFIG_LOAD_RESET: &str = "config.load_reset";
    pub const DEVICE_NOT_FOUND: &str = "device.not_found";
    pub const DEVICE_NOT_TRUSTED: &str = "device.not_trusted";
    pub const DEVICE_UNREACHABLE: &str = "device.unreachable";
    pub const PAIRING_WRONG_PIN: &str = "pairing.wrong_pin";
    pub const PAIRING_TOO_MANY_ATTEMPTS: &str = "pairing.too_many_attempts";
    pub const PAIRING_NOT_HOSTING: &str = "pairing.not_hosting";
    pub const PAIRING_BUSY: &str = "pairing.busy";
    pub const PAIRING_TIMEOUT: &str = "pairing.timeout";
    pub const PAIRING_FAILED: &str = "pairing.failed";
    pub const PAIRING_INVALID_PIN: &str = "pairing.invalid_pin";
    pub const PROTOCOL_VERSION_MISMATCH: &str = "protocol.version_mismatch";
    pub const NETWORK_BIND_FAILED: &str = "network.bind_failed";
    pub const NETWORK_PORT_FALLBACK: &str = "network.port_fallback";
    pub const DISCOVERY_FAILED: &str = "discovery.failed";
    pub const IDENTITY_FAILED: &str = "identity.failed";
    pub const TRUST_STORE_FAILED: &str = "trust_store.failed";
    pub const AUTOSTART_FAILED: &str = "autostart.failed";
    pub const CONNECTION_LOST: &str = "connection.lost";
    pub const INPUT_HOOK_FAILED: &str = "input.hook_failed";
    pub const INPUT_INJECT_BLOCKED: &str = "input.inject_blocked";
    pub const DISPLAY_FAILED: &str = "display.failed";
    pub const KVM_NOT_READY: &str = "kvm.not_ready";
    pub const LAYOUT_INVALID: &str = "layout.invalid";
    pub const LAYOUT_SAVE_FAILED: &str = "layout.save_failed";
    pub const CLIPBOARD_FAILED: &str = "clipboard.failed";
    pub const DRAGDROP_FAILED: &str = "dragdrop.failed";
    pub const TRANSFER_FAILED: &str = "transfer.failed";
    pub const TRANSFER_NOT_FOUND: &str = "transfer.not_found";
    pub const SHARE_NOT_READY: &str = "share.not_ready";
    pub const SERVICE_FAILED: &str = "service.failed";
    pub const COMPAT_UNSUPPORTED: &str = "compat.unsupported";
    pub const UPDATE_RUNNING: &str = "update.running";
    pub const UPDATE_FAILED: &str = "update.failed";
    pub const INTERNAL: &str = "internal";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub agent_version: String,
    pub build: u64,
    pub protocol_version: u32,
    pub started_unix_ms: u64,
    pub local_device: LocalDevice,
    pub connection: ConnectionStatus,
    pub network: NetworkStatus,
    pub pairing: PairingStatus,
    pub discovered: Vec<DiscoveredDevice>,
    pub trusted: Vec<TrustedDevice>,
    pub kvm: KvmStatus,
    pub share: ShareStatus,
    pub notices: Vec<Notice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDevice {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionStatus {
    Idle {
        reason: IdleReason,
    },
    Searching,
    Connecting {
        device_id: String,
        device_name: String,
        attempt: u32,
    },
    Connected(ConnectedPeer),
    Paused,
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleReason {
    Starting,
    NoTrustedDevices,
    NoNetwork,
    NetworkNotAllowed,
    AutoConnectDisabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectedPeer {
    pub device_id: String,
    pub device_name: String,
    pub address: String,
    pub rtt_ms: Option<f32>,
    pub since_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkStatus {
    pub allowed: bool,
    pub networks: Vec<NetworkInfo>,
    pub quic_port: u16,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub known: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub name: String,
    pub category: NetworkCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkCategory {
    Public,
    Private,
    Domain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairingStatus {
    #[default]
    Inactive,
    Hosting {
        pin: String,
        expires_unix_ms: u64,
        attempts_left: u32,
    },
    Joining {
        device_id: String,
        device_name: String,
    },
    Succeeded {
        device_id: String,
        device_name: String,
    },
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub device_name: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub protocol_version: u32,
    pub pairing_open: bool,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub paired_unix_ms: u64,
    pub last_connected_unix_ms: Option<u64>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub level: NoticeLevel,
    pub code: String,
    pub message: String,
    pub at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingHostResult {
    pub pin: String,
    pub expires_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingJoinParams {
    #[serde(default)]
    pub device_id: String,
    pub pin: String,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceParams {
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkNameParams {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeDismissParams {
    pub id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAppliedParams {
    pub build: u64,
    pub ui: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsInfo {
    pub agent_pid: u32,
    pub config_path: String,
    pub log_dir: String,
    pub data_dir: String,
    #[serde(default)]
    pub cpu_percent: Option<f32>,
    #[serde(default)]
    pub working_set_mb: Option<f32>,
    #[serde(default)]
    pub private_mb: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KvmState {
    #[default]
    Inactive,
    Waiting,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KvmRole {
    InputHost,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KvmActive {
    #[default]
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KvmStatus {
    pub state: KvmState,
    pub role: Option<KvmRole>,
    pub active: KvmActive,
    pub controlled_by_peer: bool,
    pub locked: bool,
    pub gaming: bool,
    pub layout: Option<LayoutView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutView {
    pub revision: u64,
    pub input_host: String,
    pub devices: Vec<LayoutDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutDevice {
    pub device_id: String,
    pub device_name: String,
    pub is_local: bool,
    pub portable: bool,
    pub x: i32,
    pub y: i32,
    pub monitors: Vec<LayoutMonitor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutMonitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale_percent: u32,
    pub diagonal_inch: Option<u32>,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutPosition {
    pub device_id: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutGuideParams {
    pub positions: Option<Vec<LayoutPosition>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSetParams {
    pub input_host: String,
    pub positions: Vec<LayoutPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToggleParams {
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatReport {
    pub os_version: String,
    pub arch: String,
    pub supported: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Text,
    Html,
    Rtf,
    Image,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipActivity {
    pub at_unix_ms: u64,
    pub kinds: Vec<ClipKind>,
    pub bytes: u64,
    pub files: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Running,
    Waiting,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferView {
    pub id: u64,
    pub direction: TransferDirection,
    pub peer_name: String,
    pub name: String,
    pub items: u32,
    pub files: u32,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub state: TransferState,
    pub error: Option<String>,
    pub started_unix_ms: u64,
    pub finished_unix_ms: Option<u64>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ShareStatus {
    pub clipboard_ready: bool,
    pub drag_ready: bool,
    pub last_sent: Option<ClipActivity>,
    pub last_received: Option<ClipActivity>,
    pub transfers: Vec<TransferView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferParams {
    pub id: u64,
}
