use serde::{Deserialize, Serialize};

use crate::DeviceId;

pub const MAC_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingMsg {
    Request {
        device_id: DeviceId,
        device_name: String,
        spake: Vec<u8>,
    },
    Response {
        device_id: DeviceId,
        device_name: String,
        spake: Vec<u8>,
    },
    Confirm {
        mac: [u8; MAC_LEN],
    },
    ConfirmAck {
        mac: [u8; MAC_LEN],
    },
    Rejected {
        reason: PairingRejectReason,
    },
    Identify,
    Identity {
        device_id: DeviceId,
        device_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingRejectReason {
    NotHosting,
    WrongPin,
    TooManyAttempts,
    Busy,
}

pub const CONFIRM_LABEL_CLIENT: &[u8] = b"tautra-pair-confirm-client";
pub const CONFIRM_LABEL_SERVER: &[u8] = b"tautra-pair-confirm-server";
