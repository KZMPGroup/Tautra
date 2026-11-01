use serde::de::DeserializeOwned;
use serde::Serialize;
use tautra_proto::frame::{self, FrameError, HEADER_LEN};

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("stream ditutup peer")]
    Closed,
    #[error("gagal menulis stream: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("gagal membaca stream: {0}")]
    Read(#[from] quinn::ReadError),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

impl From<quinn::ReadExactError> for StreamError {
    fn from(err: quinn::ReadExactError) -> Self {
        match err {
            quinn::ReadExactError::FinishedEarly(_) => StreamError::Closed,
            quinn::ReadExactError::ReadError(e) => StreamError::Read(e),
        }
    }
}

pub async fn write_msg<T: Serialize>(
    send: &mut quinn::SendStream,
    msg: &T,
) -> Result<(), StreamError> {
    let bytes = frame::encode(msg)?;
    send.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_msg<T: DeserializeOwned>(recv: &mut quinn::RecvStream) -> Result<T, StreamError> {
    let mut header = [0u8; HEADER_LEN];
    recv.read_exact(&mut header).await?;
    let len = frame::payload_len(header)?;
    let mut payload = vec![0u8; len];
    recv.read_exact(&mut payload).await?;
    Ok(frame::decode(&payload)?)
}
