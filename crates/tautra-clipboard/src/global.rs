use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};

pub fn read(handle: HGLOBAL, limit: u64) -> Option<Vec<u8>> {
    unsafe {
        let size = GlobalSize(handle);
        if size == 0 || size as u64 > limit {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(handle);
        Some(data)
    }
}

pub fn alloc(data: &[u8]) -> windows::core::Result<HGLOBAL> {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1))?;
        let ptr = GlobalLock(handle) as *mut u8;
        if ptr.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        let _ = GlobalUnlock(handle);
        Ok(handle)
    }
}

pub fn trim_nul(mut data: Vec<u8>) -> Vec<u8> {
    if let Some(end) = data.iter().position(|&b| b == 0) {
        data.truncate(end);
    }
    data
}

pub fn utf16_text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}
