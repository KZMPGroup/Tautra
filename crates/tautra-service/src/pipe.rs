use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
    HLOCAL,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState,
    NAMED_PIPE_MODE, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use crate::protocol::BUFFER_SIZE;
use crate::{wide, ServiceError};

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0 .0))) };
        }
    }
}

fn security(sddl: &str) -> Result<Descriptor, ServiceError> {
    let text = wide(sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(text.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| ServiceError::Pipe(format!("hak akses pipe gagal dibuat: {}", e.message())))?;
    Ok(Descriptor(descriptor))
}

struct Connected(HANDLE);

impl Drop for Connected {
    fn drop(&mut self) {
        let _ = unsafe { DisconnectNamedPipe(self.0) };
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn read_message(pipe: HANDLE) -> Result<Vec<u8>, ServiceError> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; BUFFER_SIZE as usize];
    loop {
        let mut read = 0u32;
        match unsafe { ReadFile(pipe, Some(&mut chunk), Some(&mut read), None) } {
            Ok(()) => {
                out.extend_from_slice(&chunk[..read as usize]);
                if out.last() == Some(&b'\n') {
                    out.pop();
                    return Ok(out);
                }
                if read == 0 {
                    return Ok(out);
                }
            }
            Err(e) => return Err(ServiceError::Pipe(format!("baca pipe: {}", e.message()))),
        }
    }
}

fn write_message(pipe: HANDLE, mut bytes: Vec<u8>) -> Result<(), ServiceError> {
    bytes.push(b'\n');
    let mut written = 0u32;
    unsafe { WriteFile(pipe, Some(&bytes), Some(&mut written), None) }
        .map_err(|e| ServiceError::Pipe(format!("tulis pipe: {}", e.message())))?;
    let _ = unsafe { FlushFileBuffers(pipe) };
    Ok(())
}

pub fn serve<F>(
    pipe_path: &str,
    sddl: &str,
    stop: &AtomicBool,
    mut handler: F,
) -> Result<(), ServiceError>
where
    F: FnMut(&[u8]) -> Vec<u8>,
{
    let descriptor = security(sddl)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0 .0,
        bInheritHandle: windows::core::BOOL(0),
    };
    let name = wide(pipe_path);
    let create = || -> Option<Connected> {
        let pipe = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_SIZE,
                BUFFER_SIZE,
                0,
                Some(&attributes),
            )
        };
        if pipe.is_invalid() {
            let e = windows::core::Error::from_thread();
            tracing::error!("pipe {pipe_path} gagal dibuat: {}", e.message());
            return None;
        }
        Some(Connected(pipe))
    };
    let mut listening: Option<Connected> = None;
    while !stop.load(Ordering::SeqCst) {
        let pipe = match listening.take().or_else(create) {
            Some(pipe) => pipe,
            None => {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let connected = match unsafe { ConnectNamedPipe(pipe.0, None) } {
            Ok(()) => true,
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => true,
            Err(e) => {
                tracing::debug!("koneksi pipe {pipe_path} gagal: {}", e.message());
                false
            }
        };
        if !connected || stop.load(Ordering::SeqCst) {
            continue;
        }
        listening = create();
        match read_message(pipe.0) {
            Ok(request) => {
                let reply = handler(&request);
                if let Err(e) = write_message(pipe.0, reply) {
                    tracing::debug!("balasan pipe {pipe_path} gagal: {e}");
                }
            }
            Err(e) => tracing::debug!("permintaan pipe {pipe_path} gagal dibaca: {e}"),
        }
    }
    Ok(())
}

pub fn poke(pipe_path: &str) {
    let name = wide(pipe_path);
    let _ = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    }
    .map(|h| unsafe {
        let _ = CloseHandle(h);
    });
}

struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(pipe_path: &str) -> Result<Client, ServiceError> {
    let name = wide(pipe_path);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        } {
            Ok(handle) => {
                let client = Client(handle);
                let mode = NAMED_PIPE_MODE(PIPE_READMODE_MESSAGE.0);
                let _ = unsafe { SetNamedPipeHandleState(client.0, Some(&mode), None, None) };
                return Ok(client);
            }
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
                return Err(ServiceError::NotRunning);
            }
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(ServiceError::Pipe(format!("buka pipe: {}", e.message()))),
        }
    }
}

pub fn call(pipe_path: &str, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
    let client = open(pipe_path)?;
    write_message(client.0, request.to_vec())?;
    read_message(client.0)
}
