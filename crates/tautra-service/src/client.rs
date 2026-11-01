use tautra_proto::kvm::ServiceInput;

use crate::protocol::{ControlReply, ControlRequest, ServiceState, CONTROL_PIPE};
use crate::ServiceError;

fn call(request: &ControlRequest) -> Result<ControlReply, ServiceError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|e| ServiceError::Other(format!("permintaan gagal dibentuk: {e}")))?;
    let reply = crate::pipe::call(CONTROL_PIPE, &bytes)?;
    serde_json::from_slice(&reply)
        .map_err(|e| ServiceError::Other(format!("balasan layanan tidak dimengerti: {e}")))
}

pub fn forward(events: Vec<ServiceInput>) -> Result<(), ServiceError> {
    match call(&ControlRequest::Inject(events))? {
        ControlReply::Ok => Ok(()),
        ControlReply::Error(e) => Err(ServiceError::Other(e)),
        ControlReply::Status(_) | ControlReply::Updating(_) => Ok(()),
    }
}

pub fn send_sas() -> Result<(), ServiceError> {
    match call(&ControlRequest::SendSas)? {
        ControlReply::Ok => Ok(()),
        ControlReply::Error(e) => Err(ServiceError::Other(e)),
        ControlReply::Status(_) | ControlReply::Updating(_) => Ok(()),
    }
}

pub fn check_update() -> Result<bool, ServiceError> {
    match call(&ControlRequest::CheckUpdate)? {
        ControlReply::Updating(restarting) => Ok(restarting),
        ControlReply::Error(e) => Err(ServiceError::Other(e)),
        ControlReply::Ok | ControlReply::Status(_) => Ok(false),
    }
}

pub fn status() -> Result<ServiceState, ServiceError> {
    match call(&ControlRequest::Status)? {
        ControlReply::Status(state) => Ok(state),
        ControlReply::Error(e) => Err(ServiceError::Other(e)),
        ControlReply::Ok | ControlReply::Updating(_) => {
            Err(ServiceError::Other("balasan status kosong".into()))
        }
    }
}

pub fn running() -> bool {
    !matches!(status(), Err(ServiceError::NotRunning))
}
