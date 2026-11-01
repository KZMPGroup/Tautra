use serde::{Deserialize, Serialize};

pub use tautra_proto::kvm::ServiceInput;

pub const SERVICE_NAME: &str = "TautraService";
pub const SERVICE_DISPLAY_NAME: &str = "Tautra Service";
pub const SERVICE_DESCRIPTION: &str =
    "Meneruskan input Tautra ke layar UAC, layar kunci, dan jendela administrator.";

pub const SERVICE_ARG: &str = "--service";
pub const INSTALL_ARG: &str = "--service-install";
pub const UNINSTALL_ARG: &str = "--service-uninstall";
pub const WORKER_ARG: &str = "--input-worker";
pub const RESTART_ARG: &str = "--service-restart";
pub const SOURCE_FILE: &str = "source.txt";

pub const CONTROL_PIPE: &str = r"\\.\pipe\tautra-service";

pub const CONTROL_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)";
pub const WORKER_SDDL: &str = "D:(A;;GA;;;SY)";

pub const BUFFER_SIZE: u32 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRequest {
    Status,
    Inject(Vec<ServiceInput>),
    SendSas,
    CheckUpdate,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerReply {
    Ok,
    Desktop(String),
    Error(String),
}
