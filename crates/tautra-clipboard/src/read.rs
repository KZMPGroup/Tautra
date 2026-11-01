use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::{HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::formats::formats;
use crate::global;
use crate::service::{ClipboardOptions, LocalContent, LocalImage};

const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY: Duration = Duration::from_millis(20);
const DIB_FACTOR: u64 = 4;

struct Opened;

impl Drop for Opened {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn open(hwnd: HWND) -> Result<Opened, String> {
    for _ in 0..OPEN_ATTEMPTS {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return Ok(Opened);
        }
        std::thread::sleep(OPEN_RETRY);
    }
    Err("clipboard sedang dikunci aplikasi lain".into())
}

fn available(format: u16) -> bool {
    unsafe { IsClipboardFormatAvailable(format as u32) }.is_ok()
}

fn get(format: u16, limit: u64) -> Option<Vec<u8>> {
    let handle = unsafe { GetClipboardData(format as u32) }.ok()?;
    if handle.is_invalid() {
        return None;
    }
    let data = global::read(HGLOBAL(handle.0), limit);
    if data.is_none() {
        tracing::info!(format, "format clipboard dilewati karena kosong atau melebihi batas ukuran");
    }
    data
}

fn dropped_files() -> Vec<PathBuf> {
    let Ok(handle) = (unsafe { GetClipboardData(CF_HDROP.0 as u32) }) else {
        return Vec::new();
    };
    let hdrop = HDROP(handle.0);
    let mut paths = Vec::new();
    unsafe {
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; len + 1];
            let written = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..written])));
        }
    }
    paths
}

fn non_empty(data: Vec<u8>) -> Option<Vec<u8>> {
    (!data.is_empty()).then_some(data)
}

pub fn snapshot(hwnd: HWND, options: &ClipboardOptions) -> Result<Option<LocalContent>, String> {
    let _opened = open(hwnd)?;
    let f = formats();
    if available(f.exclude) || available(f.viewer_ignore) {
        tracing::debug!("isi clipboard ditandai privat oleh aplikasi sumber, tidak dikirim");
        return Ok(None);
    }
    let limit = options.max_bytes;
    let mut content = LocalContent::default();
    if options.text && available(CF_UNICODETEXT.0) {
        content.text = get(CF_UNICODETEXT.0, limit.saturating_mul(2))
            .map(|d| global::utf16_text(&d))
            .filter(|t| !t.is_empty() && t.len() as u64 <= limit);
    }
    if options.html && available(f.html) {
        content.html = get(f.html, limit).map(global::trim_nul).and_then(non_empty);
    }
    if options.rtf && available(f.rtf) {
        content.rtf = get(f.rtf, limit).map(global::trim_nul).and_then(non_empty);
    }
    if options.image {
        if available(f.png) {
            content.image = get(f.png, limit).and_then(non_empty).map(LocalImage::Png);
        }
        if content.image.is_none() && (available(CF_DIBV5.0) || available(CF_DIB.0)) {
            content.image = get(CF_DIB.0, limit.saturating_mul(DIB_FACTOR))
                .and_then(non_empty)
                .map(LocalImage::Dib);
        }
    }
    if options.files && available(CF_HDROP.0) {
        content.files = dropped_files();
    }
    Ok(Some(content))
}
