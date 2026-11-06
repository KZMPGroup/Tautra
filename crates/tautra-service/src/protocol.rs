use serde::{Deserialize, Serialize};

pub use tautra_proto::kvm::ServiceInput;

pub const SERVICE_NAME: &str = "TautraService";
pub const SERVICE_DISPLAY_NAME: &str = "Tautra Service";
pub const SERVICE_DESCRIPTION: &str =
    "Meneruskan input Tautra ke layar UAC, layar kunci, jendela administrator, dan layar masuk Windows.";

pub const SERVICE_ARG: &str = "--service";
pub const INSTALL_ARG: &str = "--service-install";
pub const UNINSTALL_ARG: &str = "--service-uninstall";
pub const WORKER_ARG: &str = "--input-worker";
pub const RESTART_ARG: &str = "--service-restart";
pub const LOGON_ARG: &str = "--logon";
pub const SOURCE_FILE: &str = "source.txt";

pub const CONTROL_PIPE: &str = r"\\.\pipe\tautra-service";
pub const INPUT_ASSIST_PIPE: &str = r"\\.\pipe\tautra-input-assist";

pub const CONTROL_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)";
pub const WORKER_SDDL: &str = "D:(A;;GA;;;SY)";
pub const ASSIST_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;AU)";

pub const BUFFER_SIZE: u32 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRequest {
    Status,
    Inject(Vec<ServiceInput>),
    SendSas,
    CheckUpdate,
    SyncLogon(LogonPayload),
    ClearLogon,
    SetAssist(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssistInput {
    Mouse { msg: u32, x: i32, y: i32, mouse_data: u32 },
    Key { msg: u32, vk: u32, scan: u32, extended: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AssistReply {
    pub swallow: bool,
    pub remote: bool,
    pub park: Option<(i32, i32)>,
    #[serde(default)]
    pub place: Option<(i32, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogonFile {
    pub path: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LogonPayload {
    pub files: Vec<LogonFile>,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlReply {
    Ok,
    Status(ServiceState),
    Error(String),
    Updating(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceState {
    pub worker_desktop: Option<String>,
    pub secure_desktop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerRequest {
    Inject(Vec<ServiceInput>),
    SendSas,
    Ping,
    SetAssist(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerReply {
    Ok,
    Desktop(String),
    Error(String),
}
