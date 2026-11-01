use tautra_proto::kvm::{DisplaySet, MonitorInfo};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, EnumDisplayMonitors, GetDeviceCaps, GetMonitorInfoW, HDC, HMONITOR,
    HORZSIZE, MONITORINFO, MONITORINFOEXW, VERTSIZE,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

use crate::InputError;

const NO_SYSTEM_BATTERY: u8 = 128;
const UNKNOWN_BATTERY: u8 = 255;

pub fn set_dpi_awareness() -> Result<(), String> {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(|e| e.message())
}

unsafe extern "system" fn collect(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    list.push(monitor);
    BOOL(1)
}

fn physical_size(device: &[u16]) -> (u32, u32) {
    let hdc = unsafe { CreateDCW(PCWSTR(device.as_ptr()), PCWSTR(device.as_ptr()), None, None) };
    if hdc.is_invalid() {
        return (0, 0);
    }
    let w = unsafe { GetDeviceCaps(Some(hdc), HORZSIZE) };
    let h = unsafe { GetDeviceCaps(Some(hdc), VERTSIZE) };
    let _ = unsafe { DeleteDC(hdc) };
    (w.max(0) as u32, h.max(0) as u32)
}

fn monitor_info(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    if !ok.as_bool() {
        return None;
    }
    let r = info.monitorInfo.rcMonitor;
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        dpi_x = 96;
    }
    let len = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..len])
        .trim_start_matches(r"\\.\")
        .to_string();
    let (width_mm, height_mm) = physical_size(&info.szDevice);
    let monitor = MonitorInfo {
        name,
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
        scale_percent: (dpi_x * 100 + 48) / 96,
        width_mm,
        height_mm,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    };
    match monitor.physical_scale() {
        Some(scale) => tracing::debug!(
            name = %monitor.name,
            width_mm,
            height_mm,
            scale,
            "ukuran fisik monitor dipakai untuk skala layout"
        ),
        None => tracing::debug!(
            name = %monitor.name,
            width_mm,
            height_mm,
            "ukuran fisik monitor tidak tersedia/tidak masuk akal, memakai skala DPI"
        ),
    }
    Some(monitor)
}

pub fn enumerate() -> Result<DisplaySet, InputError> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    if !ok.as_bool() {
        return Err(InputError::Display(
            windows::core::Error::from_thread().message(),
        ));
    }
    let mut monitors: Vec<MonitorInfo> = handles.into_iter().filter_map(monitor_info).collect();
    if monitors.is_empty() {
        return Err(InputError::Display("tidak ada monitor aktif".into()));
    }
    monitors.sort_by_key(|m| (!m.primary, m.x, m.y));
    Ok(DisplaySet { monitors })
}

pub fn is_portable() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return false;
    }
    status.BatteryFlag != NO_SYSTEM_BATTERY && status.BatteryFlag != UNKNOWN_BATTERY
}
