use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const AGENT_EXE: &str = "tautra-agent.exe";
pub const UI_EXE: &str = "tautra-ui.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
pub const UPDATE_FILES: [&str; 3] = [AGENT_EXE, UI_EXE, UNINSTALL_EXE];
pub const OLD_SUFFIX: &str = ".old";

static BUILD: OnceLock<u64> = OnceLock::new();

#[macro_export]
macro_rules! declare_build {
    () => {
        #[used]
        static TAUTRA_BUILD_MARKER: &str =
            concat!("TAUTRA-BUILD-MARK:", env!("TAUTRA_BUILD"), ":END");

        #[inline(never)]
        fn tautra_build_id() -> u64 {
            let marker: &str = std::hint::black_box(TAUTRA_BUILD_MARKER);
            marker
                .split(':')
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        }
    };
}

pub fn set(id: u64) {
    let _ = BUILD.set(id);
}

pub fn id() -> u64 {
    BUILD.get().copied().unwrap_or(0)
}

pub fn version_string() -> String {
    format!("{}+{}", crate::APP_VERSION, id())
}

fn needle() -> Vec<u8> {
    let mut n = b"TAUTRA-BU".to_vec();
    n.extend_from_slice(b"ILD-MARK:");
    n
}

pub fn marker_in_bytes(bytes: &[u8]) -> Option<u64> {
    let needle = needle();
    let mut from = 0;
    while from + needle.len() <= bytes.len() {
        let pos = bytes[from..]
            .windows(needle.len())
            .position(|w| w == needle.as_slice())?;
        let start = from + pos + needle.len();
        let digits: Vec<u8> = bytes[start..]
            .iter()
            .take(20)
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        let end = start + digits.len();
        if !digits.is_empty() && bytes[end..].starts_with(b":END") {
            if let Some(id) = std::str::from_utf8(&digits)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
            {
                return Some(id);
            }
        }
        from = start;
    }
    None
}

pub fn marker_in_file(path: &Path) -> std::io::Result<Option<u64>> {
    let bytes = std::fs::read(path)?;
    Ok(marker_in_bytes(&bytes))
}

pub fn disk_build(dir: &Path) -> std::io::Result<Option<u64>> {
    let Some(agent) = marker_in_file(&dir.join(AGENT_EXE))? else {
        return Ok(None);
    };
    let mut best = agent;
    for name in UPDATE_FILES {
        if name == AGENT_EXE {
            continue;
        }
        let path = dir.join(name);
        if path.is_file() {
            if let Some(id) = marker_in_file(&path)? {
                best = best.max(id);
            }
        }
    }
    Ok(Some(best))
}

pub fn release_id(own: u64) -> u64 {
    let Ok(dir) = exe_dir() else {
        return own;
    };
    let mut best = own;
    for name in UPDATE_FILES {
        let path = dir.join(name);
        if path.is_file() {
            if let Ok(Some(id)) = marker_in_file(&path) {
                best = best.max(id);
            }
        }
    }
    best
}

pub fn exe_dir() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "folder exe tidak diketahui")
    })
}

pub fn old_name(name: &str) -> String {
    format!("{name}{OLD_SUFFIX}")
}

pub fn swap_in(dir: &Path, name: &str, fresh: &Path) -> std::io::Result<()> {
    let target = dir.join(name);
    let old = dir.join(old_name(name));
    if old.exists() && std::fs::remove_file(&old).is_err() {
        let parked = dir.join(format!(
            "{name}{OLD_SUFFIX}-{}",
            crate::time::unix_ms()
        ));
        std::fs::rename(&old, parked)?;
    }
    if target.exists() {
        std::fs::rename(&target, &old)?;
    }
    if let Err(e) = std::fs::rename(fresh, &target) {
        if old.exists() {
            let _ = std::fs::rename(&old, &target);
        }
        return Err(e);
    }
    Ok(())
}

pub fn clean_old(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    let mut clean = true;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(OLD_SUFFIX)
            && UPDATE_FILES.iter().any(|f| name.starts_with(f))
            && std::fs::remove_file(entry.path()).is_err()
        {
            clean = false;
        }
    }
    clean
}

#[cfg(windows)]
pub fn wait_for_pid(pid: u32, timeout: std::time::Duration) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return;
    };
    let _ = unsafe { WaitForSingleObject(handle, timeout.as_millis() as u32) };
    let _ = unsafe { CloseHandle(handle) };
}

#[cfg(windows)]
pub fn stop_pid(pid: u32, timeout: std::time::Duration) {
    use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }) else {
        return;
    };
    if unsafe { WaitForSingleObject(handle, timeout.as_millis() as u32) } == WAIT_TIMEOUT {
        let _ = unsafe { TerminateProcess(handle, 0) };
        let _ = unsafe { WaitForSingleObject(handle, 2000) };
    }
    let _ = unsafe { CloseHandle(handle) };
}
