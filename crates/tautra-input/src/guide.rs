use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FW_BOLD, HGDIOBJ, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PeekMessageW, PostThreadMessageW, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    GWLP_USERDATA, HWND_TOPMOST, LWA_ALPHA, MSG, PM_NOREMOVE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_SHOWNOACTIVATE, WM_APP, WM_PAINT, WM_QUIT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::geometry::{gap, span, Edge, Geometry, RectI, ADJACENT_TOLERANCE};
use crate::InputError;

const BAND: f64 = 50.0;
const TICK: f64 = 10.0;
const DEPTH_RATIO: f64 = 0.09;
const MIN_DEPTH: i32 = 72;
const LINE: i32 = 2;
const ALPHA: u8 = 235;
const WM_GUIDE_UPDATE: u32 = WM_APP + 21;
const EDGES: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];
const PALETTE: [(u32, bool); 8] = [
    (0xFFFFFF, true),
    (0xFFE600, true),
    (0x00E5FF, true),
    (0x00C853, true),
    (0xE040FB, false),
    (0xFF1744, false),
    (0x2962FF, false),
    (0xFF9100, true),
];
const UNLINKED: u32 = 0x3C3C3C;

#[derive(Debug, Clone, PartialEq)]
pub struct GuideCell {
    pub rect: RectI,
    pub color: Option<usize>,
    pub label: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuideStrip {
    pub rect: RectI,
    pub cells: Vec<GuideCell>,
    pub lines: Vec<RectI>,
}

fn rgb(hex: u32) -> COLORREF {
    COLORREF(((hex & 0xFF) << 16) | (hex & 0xFF00) | ((hex >> 16) & 0xFF))
}

pub fn strips(geometry: &Geometry) -> Vec<GuideStrip> {
    let local = &geometry.local;
    let remote = &geometry.remote;
    let mut out = Vec::new();
    for screen in &local.screens {
        for edge in EDGES {
            let horizontal = edge.horizontal();
            let overlaps: Vec<(f64, f64)> = remote
                .screens
                .iter()
                .filter(|r| gap(&screen.layout, &r.layout, edge).abs() <= ADJACENT_TOLERANCE)
                .filter_map(|r| {
                    let (a, b) = span(&screen.layout, horizontal);
                    let (c, d) = span(&r.layout, horizontal);
                    let (lo, hi) = (a.max(c), b.min(d));
                    (hi > lo).then_some((lo, hi))
                })
                .collect();
            if overlaps.is_empty() {
                continue;
            }
            let p = screen.phys;
            let depth = if horizontal {
                (((p.right - p.left) as f64 * DEPTH_RATIO) as i32).max(MIN_DEPTH)
            } else {
                (((p.bottom - p.top) as f64 * DEPTH_RATIO) as i32).max(MIN_DEPTH)
            };
            let rect = match edge {
                Edge::Left => RectI { left: p.left, top: p.top, right: p.left + depth, bottom: p.bottom },
                Edge::Right => RectI { left: p.right - depth, top: p.top, right: p.right, bottom: p.bottom },
                Edge::Top => RectI { left: p.left, top: p.top, right: p.right, bottom: p.top + depth },
                Edge::Bottom => RectI { left: p.left, top: p.bottom - depth, right: p.right, bottom: p.bottom },
            };
            let to_phys = |l: f64| -> i32 {
                let (x, y) = local.from_layout(l, l);
                let v = if horizontal { y } else { x };
                let (lo, hi) = if horizontal { (p.top, p.bottom) } else { (p.left, p.right) };
                (v.round() as i32).clamp(lo, hi)
            };
            let piece = |a: i32, b: i32| -> RectI {
                if horizontal {
                    RectI { left: rect.left, top: a, right: rect.right, bottom: b }
                } else {
                    RectI { left: a, top: rect.top, right: b, bottom: rect.bottom }
                }
            };
            let linked = |l: f64| overlaps.iter().any(|(lo, hi)| l >= *lo && l < *hi);
            let (slo, shi) = span(&screen.layout, horizontal);
            let mut cells = Vec::new();
            let mut lines = Vec::new();
            let mut k = (slo / BAND).floor() as i64;
            while (k as f64) * BAND < shi {
                let lo = (k as f64 * BAND).max(slo);
                let hi = ((k + 1) as f64 * BAND).min(shi);
                let mut cuts = vec![lo, hi];
                for (olo, ohi) in &overlaps {
                    for c in [*olo, *ohi] {
                        if c > lo && c < hi {
                            cuts.push(c);
                        }
                    }
                }
                cuts.sort_by(f64::total_cmp);
                for pair in cuts.windows(2) {
                    let (a, b) = (to_phys(pair[0]), to_phys(pair[1]));
                    if b <= a {
                        continue;
                    }
                    let on = linked((pair[0] + pair[1]) / 2.0);
                    cells.push(GuideCell {
                        rect: piece(a, b),
                        color: on.then_some(k.rem_euclid(PALETTE.len() as i64) as usize),
                        label: on.then_some(k),
                    });
                }
                k += 1;
            }
            let mut t = (slo / TICK).ceil() * TICK;
            while t < shi {
                if linked(t) {
                    let at = to_phys(t);
                    let major = (t / BAND).round() * BAND == t;
                    let reach = if major { depth } else { depth / 3 };
                    let width = if major { LINE } else { 1 };
                    let line = match edge {
                        Edge::Left => RectI { left: rect.left, top: at, right: rect.left + reach, bottom: at + width },
                        Edge::Right => RectI { left: rect.right - reach, top: at, right: rect.right, bottom: at + width },
                        Edge::Top => RectI { left: at, top: rect.top, right: at + width, bottom: rect.top + reach },
                        Edge::Bottom => RectI { left: at, top: rect.bottom - reach, right: at + width, bottom: rect.bottom },
                    };
                    lines.push(line);
                }
                t += TICK;
            }
            out.push(GuideStrip { rect, cells, lines });
        }
    }
    out
}

thread_local! {
    static STRIPS: RefCell<Vec<GuideStrip>> = const { RefCell::new(Vec::new()) };
}

fn local_rect(r: &RectI, origin: &RectI) -> RECT {
    RECT {
        left: r.left - origin.left,
        top: r.top - origin.top,
        right: r.right - origin.left,
        bottom: r.bottom - origin.top,
    }
}

fn fill(hdc: windows::Win32::Graphics::Gdi::HDC, rect: &RECT, hex: u32) {
    unsafe {
        let brush = CreateSolidBrush(rgb(hex));
        FillRect(hdc, rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

fn paint(hwnd: HWND) {
    let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as usize;
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    STRIPS.with(|strips| {
        let strips = strips.borrow();
        let Some(strip) = strips.get(index) else {
            return;
        };
        let origin = strip.rect;
        let thickness = (origin.right - origin.left).min(origin.bottom - origin.top);
        let font = unsafe {
            CreateFontW(
                -(thickness * 2 / 5).max(12),
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        let old = unsafe { SelectObject(hdc, HGDIOBJ(font.0)) };
        unsafe { SetBkMode(hdc, TRANSPARENT) };
        for cell in &strip.cells {
            let mut rect = local_rect(&cell.rect, &origin);
            let (hex, light) = cell.color.map(|i| PALETTE[i]).unwrap_or((UNLINKED, false));
            fill(hdc, &rect, hex);
            if let Some(label) = cell.label {
                let length = (rect.right - rect.left).max(rect.bottom - rect.top);
                if length >= thickness / 2 {
                    unsafe { SetTextColor(hdc, rgb(if light { 0x111111 } else { 0xFFFFFF })) };
                    let mut text: Vec<u16> = label.to_string().encode_utf16().collect();
                    unsafe { DrawTextW(hdc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE) };
                }
            }
        }
        for line in &strip.lines {
            fill(hdc, &local_rect(line, &origin), 0x000000);
        }
        unsafe {
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
    });
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_PAINT {
        paint(hwnd);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn place(hwnd: HWND, rect: &RectI) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn create(instance: HINSTANCE, index: usize, rect: &RectI) -> Option<HWND> {
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("TautraLayoutGuide"),
            w!("Tautra"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance),
            None,
        )
    };
    let hwnd = match hwnd {
        Ok(hwnd) => hwnd,
        Err(e) => {
            tracing::warn!("jendela panduan layout gagal dibuat: {}", e.message());
            return None;
        }
    };
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), ALPHA, LWA_ALPHA);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, index as isize);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    place(hwnd, rect);
    Some(hwnd)
}

fn apply(instance: HINSTANCE, windows: &mut Vec<HWND>, strips: Vec<GuideStrip>) {
    let rects: Vec<RectI> = strips.iter().map(|s| s.rect).collect();
    STRIPS.with(|cell| *cell.borrow_mut() = strips);
    while windows.len() > rects.len() {
        if let Some(hwnd) = windows.pop() {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
    for (i, rect) in rects.iter().enumerate() {
        match windows.get(i) {
            Some(hwnd) => place(*hwnd, rect),
            None => {
                if let Some(hwnd) = create(instance, i, rect) {
                    windows.push(hwnd);
                } else {
                    break;
                }
            }
        }
    }
}

fn run(shared: Arc<Mutex<Vec<GuideStrip>>>, ready: mpsc::SyncSender<Result<u32, InputError>>) {
    let mut msg = MSG::default();
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    let instance = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(module) => HINSTANCE(module.0),
        Err(e) => {
            let _ = ready.send(Err(InputError::Thread(e.message())));
            return;
        }
    };
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: w!("TautraLayoutGuide"),
        ..Default::default()
    };
    unsafe { RegisterClassExW(&class) };
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    let mut windows: Vec<HWND> = Vec::new();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.hwnd.is_invalid() && msg.message == WM_GUIDE_UPDATE {
            let strips = std::mem::take(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()));
            apply(instance, &mut windows, strips);
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    for hwnd in windows {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

pub struct GuideOverlay {
    thread_id: u32,
    shared: Arc<Mutex<Vec<GuideStrip>>>,
    join: Option<JoinHandle<()>>,
}

impl GuideOverlay {
    pub fn start() -> Result<Self, InputError> {
        let shared = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("tautra-guide".into())
            .spawn({
                let shared = shared.clone();
                move || run(shared, tx)
            })
            .map_err(|e| InputError::Thread(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                shared,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(InputError::Thread("thread panduan layout berhenti sebelum siap".into()))
            }
        }
    }

    pub fn show(&self, strips: Vec<GuideStrip>) {
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = strips;
        if let Err(e) = unsafe { PostThreadMessageW(self.thread_id, WM_GUIDE_UPDATE, WPARAM(0), LPARAM(0)) } {
            tracing::warn!("panduan layout gagal diperbarui: {}", e.message());
        }
    }
}

impl Drop for GuideOverlay {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
