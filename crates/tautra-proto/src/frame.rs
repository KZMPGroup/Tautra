use serde::de::DeserializeOwned;
use serde::Serialize;

pub const MAX_FRAME_LEN: usize = 4 * 1024 * 1024;

pub const HEADER_LEN: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame terlalu besar: {0} byte (maks {MAX_FRAME_LEN})")]
    TooLarge(usize),
    #[error("gagal encode pesan: {0}")]
    Encode(postcard::Error),
    #[error("gagal decode pesan: {0}")]
    Decode(postcard::Error),
}

pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let mut buf = postcard::to_extend(msg, vec![0u8; HEADER_LEN]).map_err(FrameError::Encode)?;
    let len = buf.len() - HEADER_LEN;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    buf[..HEADER_LEN].copy_from_slice(&(len as u32).to_le_bytes());
    Ok(buf)
}

pub fn payload_len(header: [u8; HEADER_LEN]) -> Result<usize, FrameError> {
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

pub fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, FrameError> {
    postcard::from_bytes(payload).map_err(FrameError::Decode)
}
