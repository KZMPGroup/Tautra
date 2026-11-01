use serde::{Deserialize, Serialize};

use crate::DeviceId;

pub const HANDSHAKE_MAGIC: [u8; 4] = *b"TTRA";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandshakeHeader {
    pub magic: [u8; 4],
    pub protocol_version: u32,
    pub app_version: String,
    pub build: u64,
}

impl HandshakeHeader {
    pub fn current(app_version: &str, build: u64) -> Self {
        Self {
            magic: HANDSHAKE_MAGIC,
            protocol_version: crate::PROTOCOL_VERSION,
            app_version: app_version.to_string(),
            build,
        }
    }

    pub fn is_valid_magic(&self) -> bool {
        self.magic == HANDSHAKE_MAGIC
    }

    pub fn is_pairing_compatible(&self) -> bool {
        self.is_valid_magic() && self.protocol_version == crate::PROTOCOL_VERSION
    }

    pub fn is_compatible(&self, local_build: u64) -> bool {
        self.is_valid_magic()
            && self.protocol_version == crate::PROTOCOL_VERSION
            && self.build == local_build
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub device_id: DeviceId,
    pub device_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoodbyeReason {
    UserDisconnect,
    Shutdown,
    Sleep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    Ping { seq: u32, sent_us: u64 },
    Pong { seq: u32, sent_us: u64 },
    Goodbye { reason: GoodbyeReason },
    Rename { device_name: String },
}
