use serde::{Deserialize, Serialize};

pub const MAX_UPDATE_FILE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateRequest {
    Manifest,
    File { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateFile {
    pub name: String,
    pub size: u64,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub build: u64,
    pub files: Vec<UpdateFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateReply {
    Manifest(UpdateManifest),
    File { size: u64 },
    Error(String),
}
