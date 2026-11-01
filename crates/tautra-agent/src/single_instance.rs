use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

pub struct InstanceGuard(HANDLE);

impl InstanceGuard {
    pub fn acquire(logon: bool) -> windows::core::Result<Option<Self>> {
        let name = if logon {
            w!("Local\\Tautra.LogonAgent")
        } else {
            w!("Local\\Tautra.Agent")
        };
        unsafe {
            let handle = CreateMutexW(None, false, name)?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
