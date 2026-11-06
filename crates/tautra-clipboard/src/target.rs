use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, POINT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowThreadProcessId, WindowFromPoint};

const SHELL: &str = "explorer.exe";

fn process_name(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let found = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(process);
    }
    if !found {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().map(str::to_string)
}

pub fn shell_target() -> bool {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return false;
    }
    let hwnd = unsafe { WindowFromPoint(point) };
    if hwnd.is_invalid() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    process_name(pid).is_some_and(|name| name.eq_ignore_ascii_case(SHELL))
}
