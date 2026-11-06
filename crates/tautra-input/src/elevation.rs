use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, WindowFromPoint, GA_ROOT,
};

const MEDIUM_RID: u32 = 0x2000;

fn token_integrity(token: HANDLE) -> Option<u32> {
    unsafe {
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        )
        .ok()?;
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = GetSidSubAuthorityCount(sid);
        if count.is_null() {
            return None;
        }
        let last = (*count).saturating_sub(1) as u32;
        let rid = GetSidSubAuthority(sid, last);
        if rid.is_null() {
            return None;
        }
        Some(*rid)
    }
}

fn process_integrity(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let rid = token_integrity(token);
        let _ = CloseHandle(token);
        rid
    }
}

fn own_integrity() -> u32 {
    static OWN: OnceLock<u32> = OnceLock::new();
    *OWN.get_or_init(|| process_integrity(unsafe { GetCurrentProcess() }).unwrap_or(MEDIUM_RID))
}

const CACHE_TTL: Duration = Duration::from_millis(1000);

fn pid_blocks(pid: u32) -> bool {
    if pid == 0 || pid == unsafe { GetCurrentProcessId() } {
        return false;
    }
    static CACHE: Mutex<Vec<(u32, bool, Instant)>> = Mutex::new(Vec::new());
    if let Ok(guard) = CACHE.lock() {
        if let Some((_, blocked, _)) = guard
            .iter()
            .find(|(cached, _, at)| *cached == pid && at.elapsed() < CACHE_TTL)
        {
            return *blocked;
        }
    }
    let blocked = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(process) => {
                let other = process_integrity(process);
                let _ = CloseHandle(process);
                match other {
                    Some(level) => level > own_integrity(),
                    None => true,
                }
            }
            Err(_) => true,
        }
    };
    if let Ok(mut guard) = CACHE.lock() {
        guard.retain(|(cached, _, at)| *cached != pid && at.elapsed() < CACHE_TTL);
        guard.push((pid, blocked, Instant::now()));
    }
    blocked
}

fn window_blocks(hwnd: HWND) -> bool {
    if hwnd.0.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid_blocks(pid)
}

pub fn foreground_blocks_injection() -> bool {
    window_blocks(unsafe { GetForegroundWindow() })
}

fn pid_above_medium(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    static CACHE: Mutex<Vec<(u32, bool, Instant)>> = Mutex::new(Vec::new());
    if let Ok(guard) = CACHE.lock() {
        if let Some((_, above, _)) = guard
            .iter()
            .find(|(cached, _, at)| *cached == pid && at.elapsed() < CACHE_TTL)
        {
            return *above;
        }
    }
    let above = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(process) => {
                let level = process_integrity(process);
                let _ = CloseHandle(process);
                level.map(|l| l > MEDIUM_RID).unwrap_or(true)
            }
            Err(_) => true,
        }
    };
    if let Ok(mut guard) = CACHE.lock() {
        guard.retain(|(cached, _, at)| *cached != pid && at.elapsed() < CACHE_TTL);
        guard.push((pid, above, Instant::now()));
    }
    above
}

pub fn foreground_above_medium() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid_above_medium(pid)
}

pub fn point_blocks_injection(x: i32, y: i32) -> bool {
    let hwnd = unsafe { WindowFromPoint(POINT { x, y }) };
    if hwnd.0.is_null() {
        return false;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    window_blocks(if root.0.is_null() { hwnd } else { root })
}
