use std::path::PathBuf;

use crate::manifest::MAX_ENTRIES;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("koneksi ke perangkat lain terputus: {0}")]
    Network(String),
    #[error("file sudah tidak dibagikan lagi oleh perangkat pengirim")]
    Gone,
    #[error("perangkat pengirim gagal membaca file: {0}")]
    Remote(String),
    #[error("file berubah di perangkat pengirim selama transfer")]
    Changed,
    #[error("hash BLAKE3 tidak cocok, data rusak di perjalanan")]
    HashMismatch,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("nama file tidak aman: {0}")]
    UnsafePath(String),
    #[error("transfer dibatalkan")]
    Cancelled,
    #[error("terlalu banyak file ({0}); maksimal {MAX_ENTRIES} per transfer")]
    TooMany(usize),
    #[error("tidak ada file yang bisa dikirim")]
    Empty,
}

impl TransferError {
    pub fn retryable(&self) -> bool {
        matches!(self, TransferError::Network(_))
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        TransferError::Io {
            path: path.into(),
            source,
        }
    }
}
