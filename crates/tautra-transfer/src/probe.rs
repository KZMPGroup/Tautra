use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::core::{implement, w, Ref, Result as WinResult, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, POINTL, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{
    CoTaskMemFree, IDataObject, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{GetClipboardFormatNameW, RegisterClipboardFormatW};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, OleInitialize, OleUninitialize, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop, CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
    MOUSEINPUT, MOUSE_EVENT_FLAGS, VK_LBUTTON,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use crate::drag::{self, DragEffect, DragLabel, Prepare};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW, GetCursorPos,
    GetForegroundWindow, GetMessageW, WindowFromPoint,
    GetSystemMetrics, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
    HWND_TOPMOST, LWA_ALPHA, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_LBUTTONDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};

const WM_PROBE: u32 = WM_APP + 1;
const WM_DROP_NOW: u32 = WM_APP + 2;
const WM_CANCEL: u32 = WM_APP + 3;
const WM_START_DRAG: u32 = WM_APP + 4;
const DROP_TIMER: usize = 1;
const DROP_TIMEOUT_MS: u32 = 1500;
const START_TIMER: usize = 2;
const START_TIMEOUT_MS: u32 = 400;
const PROBE_SIZE: i32 = 64;
const PROBE_BAND: i32 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProbeState {
    Idle = 0,
    Probing = 1,
    Files = 2,
    NoFiles = 3,
    Dropping = 4,
    Dropped = 5,
    Failed = 6,
}

impl ProbeState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => ProbeState::Probing,
            2 => ProbeState::Files,
            3 => ProbeState::NoFiles,
            4 => ProbeState::Dropping,
            5 => ProbeState::Dropped,
            6 => ProbeState::Failed,
            _ => ProbeState::Idle,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("jendela drag-drop gagal dibuat: {0}")]
    Window(String),
    #[error("thread drag-drop gagal dibuat: {0}")]
    Thread(String),
}

type DropHandler = Arc<dyn Fn(Vec<PathBuf>) + Send + Sync>;

pub enum DragOutcome {
    Dropped(DragEffect),
    Failed(String),
}

pub struct DragRequest {
    pub data: Box<dyn FnOnce() -> Result<IDataObject, String> + Send>,
    pub label: DragLabel,
    pub native: bool,
    pub prepare: Option<Prepare>,
    pub done: Box<dyn FnOnce(DragOutcome) + Send>,
}

struct Armed {
    data: IDataObject,
    label: DragLabel,
    native: bool,
    prepare: Option<Prepare>,
    done: Box<dyn FnOnce(DragOutcome) + Send>,
}

#[derive(Clone)]
struct Shared {
    state: Arc<AtomicU8>,
    pending: Arc<Mutex<Option<DragRequest>>>,
    dragging: Arc<AtomicBool>,
    arming: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl Shared {
    fn get(&self) -> ProbeState {
        ProbeState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set(&self, state: ProbeState) {
        self.state.store(state as u8, Ordering::Release);
    }
}

thread_local! {
    static WINDOW: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static ARMED: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

const TEXT_LIMIT: usize = 1 << 20;
const LINE_LIMIT: usize = 4096;

fn format_of(format: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn register(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

fn offers(data: &IDataObject, format: u16) -> bool {
    let query = format_of(format);
    unsafe { data.QueryGetData(&query) }.is_ok()
}

fn read_text(data: &IDataObject, format: u16) -> Option<String> {
    if !offers(data, format) {
        return None;
    }
    let query = format_of(format);
    let mut medium = unsafe { data.GetData(&query) }.ok()?;
    let mut text = None;
    if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        unsafe {
            let handle = medium.u.hGlobal;
            let size = GlobalSize(handle).min(TEXT_LIMIT * 2);
            let ptr = GlobalLock(handle) as *const u16;
            if !ptr.is_null() {
                let units = std::slice::from_raw_parts(ptr, size / 2);
                let len = units.iter().position(|&c| c == 0).unwrap_or(units.len());
                text = Some(String::from_utf16_lossy(&units[..len]));
                let _ = GlobalUnlock(handle);
            }
        }
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    text
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            out.push(hex(bytes[i + 1])? << 4 | hex(bytes[i + 2])?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_file_url(value: &str) -> bool {
    value.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
}

fn file_url_path(value: &str) -> Option<PathBuf> {
    let rest = value.get(5..)?.strip_prefix("//")?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(rest)?;
    let (host, path) = match decoded.find('/') {
        Some(i) => (&decoded[..i], &decoded[i..]),
        None => (decoded.as_str(), ""),
    };
    let path = if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        path.trim_start_matches('/').replace('/', "\\")
    } else {
        format!("\\\\{host}{}", path.replace('/', "\\"))
    };
    Some(PathBuf::from(path))
}

fn parse_path(line: &str) -> Option<PathBuf> {
    let line = line.trim().trim_matches('"');
    if line.is_empty() {
        return None;
    }
    let path = if is_file_url(line) {
        file_url_path(line)?
    } else {
        PathBuf::from(line)
    };
    (path.is_absolute() && std::fs::symlink_metadata(&path).is_ok()).then_some(path)
}

fn text_paths(data: &IDataObject) -> Vec<PathBuf> {
    let Some(text) = read_text(data, CF_UNICODETEXT.0) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() || lines.len() > LINE_LIMIT {
        return Vec::new();
    }
    let mut paths = Vec::with_capacity(lines.len());
    for line in lines {
        match parse_path(line) {
            Some(path) if !paths.contains(&path) => paths.push(path),
            Some(_) => {}
            None => return Vec::new(),
        }
    }
    paths
}

fn file_hint(data: &IDataObject) -> bool {
    if offers(data, register(w!("Chromium Web Custom MIME Data Format"))) {
        return true;
    }
    read_text(data, register(w!("UniformResourceLocatorW"))).is_some_and(|url| is_file_url(url.trim()))
}

fn has_files(data: &IDataObject) -> bool {
    offers(data, CF_HDROP.0) || (file_hint(data) && !text_paths(data).is_empty())
}

fn format_name(format: u16) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClipboardFormatNameW(format as u32, &mut buf) }.max(0) as usize;
    if len == 0 {
        format.to_string()
    } else {
        String::from_utf16_lossy(&buf[..len])
    }
}

fn describe_formats(data: &IDataObject) -> String {
    let Ok(list) = (unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }) else {
        return String::new();
    };
    let mut names = Vec::new();
    for _ in 0..64 {
        let mut item = [FORMATETC::default()];
        let mut fetched = 0u32;
        if unsafe { list.Next(&mut item, Some(&mut fetched as *mut u32)) } != S_OK || fetched == 0 {
            break;
        }
        if !item[0].ptd.is_null() {
            unsafe { CoTaskMemFree(Some(item[0].ptd as *const _)) };
        }
        let name = format_name(item[0].cfFormat);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join(", ")
}

fn hdrop_paths(data: &IDataObject) -> Vec<PathBuf> {
    let format = format_of(CF_HDROP.0);
    let Ok(mut medium) = (unsafe { data.GetData(&format) }) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    unsafe {
        let hdrop = HDROP(medium.u.hGlobal.0);
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
        ReleaseStgMedium(&mut medium);
    }
    paths
}

fn read_files(data: &IDataObject) -> Vec<PathBuf> {
    if file_hint(data) {
        let paths = text_paths(data);
        if !paths.is_empty() {
            return paths;
        }
    }
    hdrop_paths(data)
}

fn accepted(allowed: *mut DROPEFFECT) -> DROPEFFECT {
    let allowed = if allowed.is_null() {
        DROPEFFECT_COPY
    } else {
        unsafe { *allowed }
    };
    if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else if allowed.0 & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK
    } else {
        DROPEFFECT_COPY
    }
}

fn set_effect(effect: *mut DROPEFFECT, value: DROPEFFECT) {
    if !effect.is_null() {
        unsafe { *effect = value };
    }
}

#[implement(IDropTarget)]
struct ProbeTarget {
    shared: Shared,
    hwnd: isize,
    on_drop: DropHandler,
    rejected: Mutex<String>,
}

impl ProbeTarget {
    fn note_rejected(&self, data: &IDataObject) {
        let formats = describe_formats(data);
        let mut last = self.rejected.lock().unwrap_or_else(|e| e.into_inner());
        if *last != formats {
            tracing::info!(%formats, "drag ditolak: tidak ada file yang dikenali");
            *last = formats;
        }
    }

    fn hide(&self) {
        let hwnd = HWND(self.hwnd as *mut _);
        unsafe {
            let _ = KillTimer(Some(hwnd), DROP_TIMER);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

impl IDropTarget_Impl for ProbeTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Ref<IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        let data = pdataobj.ok().ok();
        let files = data.is_some_and(has_files);
        if !files {
            if let Some(data) = data {
                self.note_rejected(data);
            }
        }
        if matches!(
            self.shared.get(),
            ProbeState::Probing | ProbeState::Files | ProbeState::NoFiles
        ) {
            self.shared.set(if files {
                ProbeState::Files
            } else {
                ProbeState::NoFiles
            });
        }
        set_effect(
            pdweffect,
            if files {
                accepted(pdweffect)
            } else {
                DROPEFFECT_NONE
            },
        );
        Ok(())
    }

    fn DragOver(
        &self,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        let accept = matches!(
            self.shared.get(),
            ProbeState::Files | ProbeState::Dropping
        );
        set_effect(
            pdweffect,
            if accept {
                accepted(pdweffect)
            } else {
                DROPEFFECT_NONE
            },
        );
        Ok(())
    }

    fn DragLeave(&self) -> WinResult<()> {
        if matches!(self.shared.get(), ProbeState::Files | ProbeState::NoFiles) {
            self.shared.set(ProbeState::Probing);
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        if self.shared.get() != ProbeState::Dropping {
            set_effect(pdweffect, DROPEFFECT_NONE);
            self.shared.set(ProbeState::Idle);
            self.hide();
            return Ok(());
        }
        let paths = pdataobj.ok().map(read_files).unwrap_or_default();
        self.hide();
        if paths.is_empty() {
            set_effect(pdweffect, DROPEFFECT_NONE);
            self.shared.set(ProbeState::Failed);
            tracing::warn!("drop diterima tetapi daftar file kosong");
            return Ok(());
        }
        set_effect(pdweffect, accepted(pdweffect));
        self.shared.set(ProbeState::Dropped);
        tracing::info!(count = paths.len(), "file diterima dari drag lintas perangkat");
        (self.on_drop)(paths);
        Ok(())
    }
}

fn mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn normalize(value: i32, origin: i32, size: i32) -> i32 {
    let size = size.max(1) as i64;
    let offset = (value - origin).clamp(0, (size - 1) as i32) as i64;
    ((offset * 65536 + size - 1) / size).min(65535) as i32
}

fn nudge_cursor() {
    let mut point = Default::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return;
    }
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let input = mouse(
        normalize(point.x, vx, vw),
        normalize(point.y, vy, vh),
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    );
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn release_left() {
    let input = mouse(0, 0, MOUSEEVENTF_LEFTUP);
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn press_left_at(point: POINT) -> bool {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let side = if point.x > vx { point.x - 1 } else { point.x + 1 };
    let absolute = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let inputs = [
        mouse(normalize(side, vx, vw), normalize(point.y, vy, vh), absolute),
        mouse(
            normalize(point.x, vx, vw),
            normalize(point.y, vy, vh),
            absolute | MOUSEEVENTF_LEFTDOWN,
        ),
    ];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize == inputs.len() }
}

fn window_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..len])
}

fn left_down() -> bool {
    let state = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) };
    state < 0
}

fn probe_rect(x: i32, y: i32) -> (i32, i32, i32, i32) {
    let half = PROBE_SIZE / 2;
    let fallback = (x - half, y - half, PROBE_SIZE, PROBE_SIZE);
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return fallback;
    }
    let r = info.rcMonitor;
    let (width, height) = (r.right - r.left, r.bottom - r.top);
    let edges = [
        x - r.left,
        r.right - 1 - x,
        y - r.top,
        r.bottom - 1 - y,
    ];
    let nearest = (0..edges.len()).min_by_key(|&i| edges[i]).unwrap_or(1);
    if edges[nearest] > PROBE_BAND / 2 {
        return fallback;
    }
    match nearest {
        0 => (r.left, r.top, PROBE_BAND, height),
        1 => (r.right - PROBE_BAND, r.top, PROBE_BAND, height),
        2 => (r.left, r.top, width, PROBE_BAND),
        _ => (r.left, r.bottom - PROBE_BAND, width, PROBE_BAND),
    }
}

fn show_probe(hwnd: HWND, x: i32, y: i32) -> windows::core::Result<()> {
    let (left, top, width, height) = probe_rect(x, y);
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            left,
            top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

fn cover_screen(hwnd: HWND) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

fn shared() -> Option<Shared> {
    WINDOW.with(|cell| cell.borrow().clone())
}

fn finish_drag(outcome: DragOutcome, done: Box<dyn FnOnce(DragOutcome) + Send>) {
    if let Some(shared) = shared() {
        shared.arming.store(false, Ordering::Release);
        shared.dragging.store(false, Ordering::Release);
    }
    done(outcome);
}

fn arm_drag(hwnd: HWND) {
    let Some(shared) = shared() else {
        return;
    };
    let Some(request) = shared.pending.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    if ARMED.with(|cell| cell.borrow().is_some()) {
        shared.arming.store(false, Ordering::Release);
        (request.done)(DragOutcome::Failed("drag lain masih berjalan".into()));
        return;
    }
    let data = match (request.data)() {
        Ok(data) => data,
        Err(reason) => {
            finish_drag(DragOutcome::Failed(reason), request.done);
            return;
        }
    };
    if let Err(e) = cover_screen(hwnd) {
        finish_drag(
            DragOutcome::Failed(format!("jendela drag gagal ditampilkan: {}", e.message())),
            request.done,
        );
        return;
    }
    let mut point = POINT::default();
    if let Err(e) = unsafe { GetCursorPos(&mut point) } {
        finish_drag(
            DragOutcome::Failed(format!("posisi kursor tidak terbaca: {}", e.message())),
            request.done,
        );
        return;
    }
    let under = unsafe { WindowFromPoint(point) };
    if under != hwnd {
        tracing::warn!(
            x = point.x,
            y = point.y,
            window = %window_name(under),
            "jendela drag tidak berada di bawah kursor saat drag dimulai"
        );
    }
    ARMED.with(|cell| {
        *cell.borrow_mut() = Some(Armed {
            data,
            label: request.label,
            native: request.native,
            prepare: request.prepare,
            done: request.done,
        })
    });
    if !press_left_at(point) {
        abort_armed(hwnd, "Windows menolak klik dari Tautra untuk memulai drag");
        return;
    }
    unsafe { SetTimer(Some(hwnd), START_TIMER, START_TIMEOUT_MS, None) };
}

fn abort_armed(hwnd: HWND, reason: &str) {
    unsafe {
        let _ = KillTimer(Some(hwnd), START_TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let Some(armed) = ARMED.with(|cell| cell.borrow_mut().take()) else {
        return;
    };
    if left_down() {
        release_left();
    }
    let mut point = POINT::default();
    let under = if unsafe { GetCursorPos(&mut point) }.is_ok() {
        window_name(unsafe { WindowFromPoint(point) })
    } else {
        String::new()
    };
    let foreground = window_name(unsafe { GetForegroundWindow() });
    tracing::warn!(
        x = point.x,
        y = point.y,
        %under,
        %foreground,
        "drag lintas perangkat tidak dimulai: {reason}"
    );
    finish_drag(DragOutcome::Failed(reason.to_string()), armed.done);
}

fn run_armed(hwnd: HWND) -> bool {
    let Some(armed) = ARMED.with(|cell| cell.borrow_mut().take()) else {
        return false;
    };
    unsafe {
        let _ = KillTimer(Some(hwnd), START_TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    if let Some(shared) = shared() {
        shared.arming.store(false, Ordering::Release);
    }
    let cancel = shared()
        .map(|s| s.cancel.clone())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(true)));
    let outcome = if cancel.load(Ordering::Acquire) {
        Ok(DragEffect::None)
    } else {
        drag::run(&armed.data, &armed.label, cancel, armed.native, armed.prepare)
    };
    if left_down() {
        release_left();
    }
    let outcome = match outcome {
        Ok(effect) => {
            tracing::info!(?effect, "drag lintas perangkat selesai");
            DragOutcome::Dropped(effect)
        }
        Err(e) => {
            tracing::warn!("drag lintas perangkat gagal: {e}");
            DragOutcome::Failed(e)
        }
    };
    finish_drag(outcome, armed.done);
    true
}

fn hide(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), DROP_TIMER);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

fn with_shared(f: impl FnOnce(&Shared)) {
    WINDOW.with(|cell| {
        if let Some(shared) = cell.borrow().as_ref() {
            f(shared);
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PROBE => {
            let x = wparam.0 as u32 as i32;
            let y = lparam.0 as i32;
            if let Err(e) = show_probe(hwnd, x, y) {
                tracing::warn!("jendela drag-drop gagal ditampilkan: {}", e.message());
                with_shared(|s| s.set(ProbeState::Failed));
                return LRESULT(0);
            }
            nudge_cursor();
            LRESULT(0)
        }
        WM_DROP_NOW => {
            let mut point = POINT::default();
            if unsafe { GetCursorPos(&mut point) }.is_ok() {
                let _ = show_probe(hwnd, point.x, point.y);
                nudge_cursor();
            }
            if wparam.0 != 0 {
                release_left();
            }
            unsafe { SetTimer(Some(hwnd), DROP_TIMER, DROP_TIMEOUT_MS, None) };
            LRESULT(0)
        }
        WM_CANCEL => {
            hide(hwnd);
            LRESULT(0)
        }
        WM_START_DRAG => {
            arm_drag(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if run_armed(hwnd) {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_TIMER if wparam.0 == START_TIMER => {
            abort_armed(hwnd, "klik untuk memulai drag tidak sampai ke Tautra");
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DROP_TIMER => {
            hide(hwnd);
            with_shared(|s| {
                if s.get() == ProbeState::Dropping {
                    tracing::warn!("drop file tidak terjadi dalam batas waktu");
                    s.set(ProbeState::Failed);
                }
            });
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = RevokeDragDrop(hwnd);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_window(shared: &Shared, on_drop: DropHandler) -> Result<HWND, ProbeError> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|e| ProbeError::Window(e.message()))?;
    let instance = HINSTANCE(module.0);
    let class = w!("TautraDropProbe");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(ProbeError::Window(
            windows::core::Error::from_thread().message(),
        ));
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Tautra"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|e| ProbeError::Window(e.message()))?;
    unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) }
        .map_err(|e| ProbeError::Window(e.message()))?;
    let target: IDropTarget = ProbeTarget {
        shared: shared.clone(),
        hwnd: hwnd.0 as isize,
        on_drop,
        rejected: Mutex::new(String::new()),
    }
    .into();
    unsafe { RegisterDragDrop(hwnd, &target) }
        .map_err(|e| ProbeError::Window(format!("RegisterDragDrop: {}", e.message())))?;
    Ok(hwnd)
}

fn run(shared: Shared, on_drop: DropHandler, ready: SyncSender<Result<isize, ProbeError>>) {
    if let Err(e) = unsafe { OleInitialize(None) } {
        let _ = ready.send(Err(ProbeError::Window(format!(
            "OleInitialize: {}",
            e.message()
        ))));
        return;
    }
    WINDOW.with(|cell| *cell.borrow_mut() = Some(shared.clone()));
    match create_window(&shared, on_drop) {
        Ok(hwnd) => {
            let _ = ready.send(Ok(hwnd.0 as isize));
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    }
    WINDOW.with(|cell| *cell.borrow_mut() = None);
    unsafe { OleUninitialize() };
}

pub struct DropProbe {
    hwnd: isize,
    shared: Shared,
    join: Option<JoinHandle<()>>,
}

impl DropProbe {
    pub fn start(on_drop: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) -> Result<Self, ProbeError> {
        let shared = Shared {
            state: Arc::new(AtomicU8::new(ProbeState::Idle as u8)),
            pending: Arc::new(Mutex::new(None)),
            dragging: Arc::new(AtomicBool::new(false)),
            arming: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let handler: DropHandler = Arc::new(on_drop);
        let (tx, rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("tautra-drop".into())
            .spawn({
                let shared = shared.clone();
                move || run(shared, handler, tx)
            })
            .map_err(|e| ProbeError::Thread(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                shared,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(ProbeError::Thread("thread drag-drop berhenti sebelum siap".into()))
            }
        }
    }

    fn post(&self, msg: u32, wparam: usize, lparam: isize) {
        let hwnd = HWND(self.hwnd as *mut _);
        if let Err(e) = unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(lparam)) } {
            tracing::warn!("perintah drag-drop gagal dikirim: {}", e.message());
        }
    }

    pub fn state(&self) -> ProbeState {
        self.shared.get()
    }

    pub fn probe(&self, x: i32, y: i32) {
        self.shared.set(ProbeState::Probing);
        self.post(WM_PROBE, x as u32 as usize, y as isize);
    }

    pub fn drop_now(&self, inject_release: bool) {
        self.shared.set(ProbeState::Dropping);
        self.post(WM_DROP_NOW, inject_release as usize, 0);
    }

    pub fn cancel(&self) {
        if self.shared.get() != ProbeState::Dropping {
            self.shared.set(ProbeState::Idle);
        }
        self.post(WM_CANCEL, 0, 0);
    }

    pub fn start_drag(&self, request: DragRequest) {
        self.shared.cancel.store(false, Ordering::Release);
        self.shared.dragging.store(true, Ordering::Release);
        self.shared.arming.store(true, Ordering::Release);
        let previous = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(request);
        if let Some(previous) = previous {
            (previous.done)(DragOutcome::Failed("digantikan drag yang lebih baru".into()));
        }
        self.post(WM_START_DRAG, 0, 0);
    }

    pub fn dragging(&self) -> bool {
        self.shared.dragging.load(Ordering::Acquire)
    }

    pub fn arming(&self) -> bool {
        self.shared.arming.load(Ordering::Acquire)
    }

    pub fn cancel_drag(&self) {
        if !self.dragging() {
            return;
        }
        self.shared.cancel.store(true, Ordering::Release);
        nudge_cursor();
        tracing::info!("drag lintas perangkat dibatalkan");
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.cancel_drag();
        self.post(WM_CLOSE, 0, 0);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
