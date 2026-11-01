use serde::{Deserialize, Serialize};

pub const INLINE_LIMIT: u64 = 1024 * 1024;
pub const HASH_LEN: usize = 32;
pub const MAX_PATH_CHARS: usize = 259;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipFormat {
    Text,
    Html,
    Rtf,
    Png,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipPayload {
    Inline(Vec<u8>),
    Lazy { size: u64 },
}

impl ClipPayload {
    pub fn size(&self) -> u64 {
        match self {
            ClipPayload::Inline(data) => data.len() as u64,
            ClipPayload::Lazy { size } => *size,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipItem {
    pub format: ClipFormat,
    pub payload: ClipPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub dir: bool,
    pub modified_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOffer {
    pub id: u64,
    pub entries: Vec<FileEntry>,
}

impl FileOffer {
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter(|e| !e.dir).map(|e| e.size).sum()
    }

    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.dir).count()
    }

    pub fn top_level(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for entry in &self.entries {
            let top = entry.path.split('/').next().unwrap_or_default();
            if !top.is_empty() && !names.contains(&top) {
                names.push(top);
            }
        }
        names
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipOffer {
    pub id: u64,
    pub items: Vec<ClipItem>,
    pub files: Option<FileOffer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShareMessage {
    Clipboard(ClipOffer),
    Revoke { offer_id: u64 },
    Reject { offer_id: u64, reason: String },
    DragEnd { offer_id: u64, effect: DropEffect },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropEffect {
    None,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipFetchRequest {
    pub offer_id: u64,
    pub format: ClipFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipFetchReply {
    Data { size: u64 },
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest {
    pub offer_id: u64,
    pub index: u32,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileReply {
    Ready { size: u64 },
    Gone,
    Failed(String),
}
