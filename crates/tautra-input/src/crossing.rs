use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tautra_common::config::CrossingStyle;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, MonitorFromPoint,
    ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW, PeekMessageW, PostThreadMessageW,
    RegisterClassExW, SetWindowPos, ShowWindow, TranslateMessage, UpdateLayeredWindow, HWND_TOPMOST, MSG, PM_NOREMOVE, PM_REMOVE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::geometry::Edge;

const ACCENT: (f32, f32, f32) = (155.0, 216.0, 245.0);
const MAX_ALPHA: f32 = 200.0;
const WM_CROSSING_PLAY: u32 = WM_APP + 31;
const FALLBACK_FRAME: Duration = Duration::from_millis(7);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossKind {
    Exit,
    Enter,
}

#[derive(Debug, Clone, Copy)]
struct Request {
    x: i32,
    y: i32,
    kind: CrossKind,
    edge: Option<Edge>,
    style: CrossingStyle,
}

struct Player {
    thread_id: u32,
    pending: Mutex<Option<Request>>,
}

static PLAYER: OnceLock<Option<Player>> = OnceLock::new();
static TRAIL: Mutex<Option<(i32, i32)>> = Mutex::new(None);

pub fn trail(dx: i32, dy: i32) {
    if let Ok(mut t) = TRAIL.lock() {
        *t = Some((dx, dy));
    }
}

fn take_trail() -> Option<(i32, i32)> {
    TRAIL.lock().ok().and_then(|t| *t)
}

fn player() -> Option<&'static Player> {
    PLAYER
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel(1);
            let spawned = std::thread::Builder::new()
                .name("tautra-crossing".into())
                .spawn(move || run(tx));
            if let Err(e) = spawned {
                tracing::warn!("thread animasi perpindahan kursor gagal dibuat: {e}");
                return None;
            }
            match rx.recv() {
                Ok(Some(thread_id)) => {
                    tracing::info!("thread animasi perpindahan kursor siap");
                    Some(Player {
                        thread_id,
                        pending: Mutex::new(None),
                    })
                }
                _ => {
                    tracing::warn!("thread animasi perpindahan kursor gagal disiapkan");
                    None
                }
            }
        })
        .as_ref()
}

pub fn play(x: i32, y: i32, kind: CrossKind, edge: Option<Edge>, style: CrossingStyle) {
    let Some(player) = player() else {
        return;
    };
    if let Ok(mut t) = TRAIL.lock() {
        *t = None;
    }
    if let Ok(mut pending) = player.pending.lock() {
        *pending = Some(Request {
            x,
            y,
            kind,
            edge,
            style,
        });
    }
    match unsafe { PostThreadMessageW(player.thread_id, WM_CROSSING_PLAY, WPARAM(0), LPARAM(0)) } {
        Ok(()) => tracing::debug!(x, y, ?kind, ?edge, ?style, "animasi perpindahan kursor diminta"),
        Err(e) => tracing::warn!("animasi perpindahan kursor gagal dikirim: {}", e.message()),
    }
}

fn monitor_at(x: i32, y: i32) -> Option<(RECT, f32)> {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
        dpi_x = 96;
    }
    Some((info.rcMonitor, dpi_x as f32 / 96.0))
}

fn nearest_edge(rect: &RECT, x: i32, y: i32) -> Edge {
    let left = x - rect.left;
    let right = rect.right - 1 - x;
    let top = y - rect.top;
    let bottom = rect.bottom - 1 - y;
    let nearest = left.min(right).min(top).min(bottom);
    if nearest == left {
        Edge::Left
    } else if nearest == right {
        Edge::Right
    } else if nearest == top {
        Edge::Top
    } else {
        Edge::Bottom
    }
}

struct Spec {
    duration: f32,
    depth: f32,
    length: f32,
    reach: f32,
    margin: f32,
}

fn spec(style: CrossingStyle) -> Spec {
    match style {
        CrossingStyle::Ripple => Spec {
            duration: 560.0,
            depth: 48.0,
            length: 96.0,
            reach: 14.0,
            margin: 14.0,
        },
        CrossingStyle::Glow => Spec {
            duration: 480.0,
            depth: 40.0,
            length: 120.0,
            reach: 24.0,
            margin: 12.0,
        },
        CrossingStyle::Streak => Spec {
            duration: 400.0,
            depth: 84.0,
            length: 40.0,
            reach: 62.0,
            margin: 16.0,
        },
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Option<Surface> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let screen = unsafe { GetDC(None) };
        let dc = unsafe { CreateCompatibleDC(Some(screen)) };
        unsafe { ReleaseDC(None, screen) };
        if dc.is_invalid() {
            tracing::warn!("DC animasi perpindahan gagal dibuat");
            return None;
        }
        let mut bits = std::ptr::null_mut();
        let bitmap = match unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) } {
            Ok(b) if !bits.is_null() => b,
            Ok(b) => {
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(b.0));
                    let _ = DeleteDC(dc);
                }
                tracing::warn!("bitmap animasi perpindahan tidak punya memori piksel");
                return None;
            }
            Err(e) => {
                unsafe {
                    let _ = DeleteDC(dc);
                }
                tracing::warn!("bitmap animasi perpindahan gagal dibuat: {}", e.message());
                return None;
            }
        };
        let old = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        Some(Surface {
            dc,
            bitmap,
            old,
            bits: bits as *mut u32,
            width,
            height,
        })
    }

    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

struct Anim {
    kind: CrossKind,
    style: CrossingStyle,
    edge: Edge,
    rect: RECT,
    scale: f32,
    duration: f32,
    origin: (i32, i32),
    anchor: (f32, f32),
    path: Path,
    fallback: (f32, f32),
    max_reach: f32,
    following: bool,
    surface: Surface,
    started: Instant,
    frames: u32,
}

fn place(rect: &RECT, edge: Edge, x: i32, y: i32, width: i32, height: i32) -> ((i32, i32), (f32, f32)) {
    let clamp_x = |v: i32| v.clamp(rect.left, (rect.right - width).max(rect.left));
    let clamp_y = |v: i32| v.clamp(rect.top, (rect.bottom - height).max(rect.top));
    match edge {
        Edge::Left => ((rect.left, clamp_y(y - height / 2)), (rect.left as f32, y as f32 + 0.5)),
        Edge::Right => ((rect.right - width, clamp_y(y - height / 2)), (rect.right as f32, y as f32 + 0.5)),
        Edge::Top => ((clamp_x(x - width / 2), rect.top), (x as f32 + 0.5, rect.top as f32)),
        Edge::Bottom => ((clamp_x(x - width / 2), rect.bottom - height), (x as f32 + 0.5, rect.bottom as f32)),
    }
}

fn inward(rect: &RECT, edge: Edge, x: i32, y: i32) -> f32 {
    match edge {
        Edge::Left => (x - rect.left) as f32,
        Edge::Right => (rect.right - 1 - x) as f32,
        Edge::Top => (y - rect.top) as f32,
        Edge::Bottom => (rect.bottom - 1 - y) as f32,
    }
}

impl Anim {
    fn new(request: Request) -> Option<Anim> {
        let Some((rect, scale)) = monitor_at(request.x, request.y) else {
            tracing::warn!(x = request.x, y = request.y, "monitor untuk animasi perpindahan tidak ditemukan");
            return None;
        };
        let x = request.x.clamp(rect.left, rect.right - 1);
        let y = request.y.clamp(rect.top, rect.bottom - 1);
        let edge = request.edge.unwrap_or_else(|| nearest_edge(&rect, x, y));
        let s = spec(request.style);
        let depth = (s.depth * scale).ceil() as i32;
        let length = (s.length.max(2.0 * s.depth) * scale).ceil() as i32;
        let (width, height) = if edge.horizontal() { (depth, length) } else { (length, depth) };
        let width = width.min(rect.right - rect.left).max(1);
        let height = height.min(rect.bottom - rect.top).max(1);
        let (origin, anchor) = place(&rect, edge, x, y, width, height);
        let max_reach = ((s.depth - s.margin) * scale).max(1.0);
        let following = true;
        let surface = Surface::new(width, height)?;
        tracing::trace!(
            x = request.x,
            y = request.y,
            ?edge,
            diminta = ?request.edge,
            kind = ?request.kind,
            style = ?request.style,
            width,
            height,
            ?origin,
            following,
            "animasi perpindahan kursor dimulai di tepi layar"
        );
        Some(Anim {
            kind: request.kind,
            style: request.style,
            edge,
            rect,
            scale,
            duration: s.duration,
            origin,
            anchor,
            path: Path::new(),
            fallback: (s.reach * scale, 0.0),
            max_reach,
            following,
            surface,
            started: Instant::now(),
            frames: 0,
        })
    }

    fn progress(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() * 1000.0 / self.duration).min(1.0)
    }

    fn follow_cursor(&mut self) {
        if !self.following {
            return;
        }
        let (u, v) = match self.kind {
            CrossKind::Enter => {
                let mut point = POINT::default();
                if let Err(e) = unsafe { GetCursorPos(&mut point) } {
                    tracing::trace!("posisi kursor untuk animasi tidak terbaca: {}", e.message());
                    return;
                }
                let u = inward(&self.rect, self.edge, point.x, point.y);
                let cx = point.x as f32 + 0.5;
                let cy = point.y as f32 + 0.5;
                let v = if self.edge.horizontal() { cy - self.anchor.1 } else { cx - self.anchor.0 };
                (u, v)
            }
            CrossKind::Exit => {
                let Some((dx, dy)) = take_trail() else {
                    return;
                };
                let (dx, dy) = (dx as f32, dy as f32);
                match self.edge {
                    Edge::Right => (dx, -dy),
                    Edge::Left => (-dx, -dy),
                    Edge::Bottom => (dy, -dx),
                    Edge::Top => (-dy, -dx),
                }
            }
        };
        let (depth, length) = if self.edge.horizontal() {
            (self.surface.width as f32, self.surface.height as f32)
        } else {
            (self.surface.height as f32, self.surface.width as f32)
        };
        let step = 1.5 * self.scale;
        let d = (u * u + v * v).sqrt();
        if u < 0.0 || u > depth || v.abs() > length * 0.5 {
            self.following = false;
            if d >= step && u > 0.0 {
                let (eu, ev) = self.path.end();
                let (du, dv) = (u - eu, v - ev);
                let dd = (du * du + dv * dv).sqrt().max(0.001);
                let room = (self.max_reach - self.path.len()).max(step);
                self.path.push((eu + du / dd * room, ev + dv / dd * room));
            }
            tracing::trace!(
                kind = ?self.kind,
                u,
                v,
                depth,
                titik = self.path.pts.len(),
                panjang = self.path.len(),
                "kursor sudah jauh dari tepi, animasi berhenti mengikuti"
            );
            return;
        }
        let (eu, ev) = self.path.end();
        let (du, dv) = (u - eu, v - ev);
        let dd = (du * du + dv * dv).sqrt();
        if dd < step {
            return;
        }
        let room = self.max_reach - self.path.len();
        if room <= 0.0 || !self.path.push(if dd > room { (eu + du / dd * room, ev + dv / dd * room) } else { (u, v) }) {
            self.following = false;
            tracing::trace!(
                kind = ?self.kind,
                titik = self.path.pts.len(),
                panjang = self.path.len(),
                "jalur animasi penuh, berhenti mengikuti"
            );
        }
    }

    fn settle(&mut self, p: f32) {
        if self.path.pts.len() == 1 && p > 0.25 {
            self.path.push(self.fallback);
            self.following = false;
            tracing::trace!(kind = ?self.kind, "kursor tidak bergerak, animasi memakai arah bawaan");
        }
    }

    fn render(&mut self, p: f32) {
        let frame = Frame::new(self.style, self.kind, p, self.scale, &self.path);
        let (ox, oy) = self.origin;
        let (ax, ay) = self.anchor;
        let edge = self.edge;
        let width = self.surface.width as usize;
        let pixels = self.surface.pixels();
        for (row, line) in pixels.chunks_mut(width).enumerate() {
            let sy = (oy + row as i32) as f32 + 0.5;
            for (col, pixel) in line.iter_mut().enumerate() {
                let sx = (ox + col as i32) as f32 + 0.5;
                let (u, v) = match edge {
                    Edge::Left => (sx - ax, sy - ay),
                    Edge::Right => (ax - sx, sy - ay),
                    Edge::Top => (sy - ay, sx - ax),
                    Edge::Bottom => (ay - sy, sx - ax),
                };
                *pixel = premultiplied(frame.intensity(u, v, &self.path).clamp(0.0, 1.0) * MAX_ALPHA);
            }
        }
    }

    fn present(&mut self, hwnd: HWND) -> bool {
        let p = self.progress();
        self.follow_cursor();
        self.settle(p);
        self.render(p);
        let dst = POINT {
            x: self.origin.0,
            y: self.origin.1,
        };
        let size = SIZE {
            cx: self.surface.width,
            cy: self.surface.height,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if let Err(e) = unsafe { update_layered(hwnd, &dst, &size, self.surface.dc, &src, &blend) } {
            tracing::warn!("frame animasi perpindahan gagal ditampilkan: {}", e.message());
            return false;
        }
        if self.frames == 0 {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
        }
        self.frames += 1;
        p < 1.0
    }
}

unsafe fn update_layered(
    hwnd: HWND,
    dst: &POINT,
    size: &SIZE,
    src_dc: HDC,
    src: &POINT,
    blend: &BLENDFUNCTION,
) -> windows::core::Result<()> {
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(dst),
            Some(size),
            Some(src_dc),
            Some(src),
            COLORREF(0),
            Some(blend),
            ULW_ALPHA,
        )
    }
}

fn premultiplied(a: f32) -> u32 {
    if a <= 0.5 {
        return 0;
    }
    let f = a / 255.0;
    let r = (ACCENT.0 * f) as u32;
    let g = (ACCENT.1 * f) as u32;
    let b = (ACCENT.2 * f) as u32;
    ((a as u32) << 24) | (r << 16) | (g << 8) | b
}

fn bump(d: f32) -> f32 {
    let t = 1.0 - d * d;
    if t <= 0.0 {
        0.0
    } else {
        t * t
    }
}

fn soft(d: f32) -> f32 {
    let t = 1.0 - d * d;
    if t <= 0.0 {
        0.0
    } else {
        t * t * t
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

fn ease_in(t: f32) -> f32 {
    t * t
}

fn swell(t: f32, rise: f32) -> f32 {
    if t <= 0.0 || t >= 1.0 {
        0.0
    } else if t < rise {
        t / rise
    } else {
        let r = 1.0 - (t - rise) / (1.0 - rise);
        r * r
    }
}

fn window(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0.0, 1.0)
}

fn blob(du: f32, dv: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    soft((du * du + dv * dv).sqrt() / radius)
}

fn ring(du: f32, dv: f32, radius: f32, width: f32) -> f32 {
    let d = (du * du + dv * dv).sqrt();
    bump((d - radius) / width)
}

fn sparkle(du: f32, dv: f32, inner: f32, outer: f32, spokes: f32) -> f32 {
    let r = (du * du + dv * dv).sqrt();
    if r < inner || r > outer {
        return 0.0;
    }
    let angle = dv.atan2(du);
    let beam = (angle * spokes * 0.5 + 0.4).cos().abs();
    let beam = beam * beam;
    let beam = beam * beam;
    let beam = beam * beam * beam;
    let along = 1.0 - (r - inner) / (outer - inner);
    beam * along * along
}

const PATH_MAX: usize = 48;

struct Path {
    pts: Vec<(f32, f32)>,
    cum: Vec<f32>,
}

impl Path {
    fn new() -> Path {
        let mut pts = Vec::with_capacity(PATH_MAX);
        let mut cum = Vec::with_capacity(PATH_MAX);
        pts.push((0.0, 0.0));
        cum.push(0.0);
        Path { pts, cum }
    }

    fn len(&self) -> f32 {
        *self.cum.last().unwrap_or(&0.0)
    }

    fn end(&self) -> (f32, f32) {
        *self.pts.last().unwrap_or(&(0.0, 0.0))
    }

    fn push(&mut self, p: (f32, f32)) -> bool {
        if self.pts.len() >= PATH_MAX {
            return false;
        }
        let (eu, ev) = self.end();
        let seg = ((p.0 - eu).powi(2) + (p.1 - ev).powi(2)).sqrt();
        let total = self.len() + seg;
        self.pts.push(p);
        self.cum.push(total);
        true
    }

    fn at(&self, s: f32) -> (f32, f32) {
        if s <= 0.0 {
            return self.pts[0];
        }
        for i in 1..self.pts.len() {
            if s <= self.cum[i] {
                let seg = self.cum[i] - self.cum[i - 1];
                if seg <= 0.0 {
                    return self.pts[i];
                }
                let f = (s - self.cum[i - 1]) / seg;
                let (a, b) = (self.pts[i - 1], self.pts[i]);
                return (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            }
        }
        self.end()
    }

    fn nearest(&self, u: f32, v: f32, from: f32, to: f32, reach: f32) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        for i in 1..self.pts.len() {
            let (c0, c1) = (self.cum[i - 1], self.cum[i]);
            if c1 < from || c0 > to {
                continue;
            }
            let (a, b) = (self.pts[i - 1], self.pts[i]);
            let seg = c1 - c0;
            if seg <= 0.0 {
                continue;
            }
            let (au, av) = (u - a.0, v - a.1);
            if au.abs() > seg + reach || av.abs() > seg + reach {
                continue;
            }
            let (bu, bv) = (b.0 - a.0, b.1 - a.1);
            let f = ((au * bu + av * bv) / (seg * seg)).clamp(0.0, 1.0);
            let s = (c0 + f * seg).clamp(from, to);
            let f = (s - c0) / seg;
            let (qu, qv) = (a.0 + bu * f, a.1 + bv * f);
            let dist = ((u - qu).powi(2) + (v - qv).powi(2)).sqrt();
            if dist <= reach && best.map_or(true, |(d, _)| dist < d) {
                best = Some((dist, s));
            }
        }
        best
    }
}

struct Glow {
    du: f32,
    dv: f32,
    radius: f32,
    level: f32,
}

struct Ring {
    radius: f32,
    width: f32,
    level: f32,
}

struct Spark {
    du: f32,
    dv: f32,
    inner: f32,
    outer: f32,
    level: f32,
}

struct Band {
    center: f32,
    dv: f32,
    half: f32,
    core: f32,
    halo: f32,
    level: f32,
}

struct Trail {
    head: f32,
    tail: f32,
    thick: f32,
    dot: f32,
    level: f32,
}

fn unit(target: (f32, f32)) -> ((f32, f32), f32) {
    let d = (target.0 * target.0 + target.1 * target.1).sqrt();
    if d < 0.001 {
        ((1.0, 0.0), 0.0)
    } else {
        ((target.0 / d, target.1 / d), d)
    }
}

fn no_spark() -> Spark {
    Spark {
        du: 0.0,
        dv: 0.0,
        inner: 0.0,
        outer: 0.0,
        level: 0.0,
    }
}

fn no_rings() -> [Ring; 3] {
    [
        Ring { radius: 0.0, width: 1.0, level: 0.0 },
        Ring { radius: 0.0, width: 1.0, level: 0.0 },
        Ring { radius: 0.0, width: 1.0, level: 0.0 },
    ]
}

enum Frame {
    Ripple {
        glow: Glow,
        rings: [Ring; 3],
        spark: Spark,
    },
    Glow {
        band: Band,
        glow: Glow,
    },
    Streak {
        trail: Trail,
        glow: Glow,
        spark: Spark,
    },
}

impl Frame {
    fn new(style: CrossingStyle, kind: CrossKind, p: f32, s: f32, path: &Path) -> Frame {
        let target = path.end();
        let (dir, _) = unit(target);
        let total = path.len();
        match style {
            CrossingStyle::Ripple => match kind {
                CrossKind::Exit => {
                    let glow = Glow {
                        du: 0.0,
                        dv: 0.0,
                        radius: 16.0 * s * (1.0 - ease_in(window(p, 0.0, 0.45))),
                        level: 0.32 * (1.0 - window(p, 0.0, 0.5)),
                    };
                    let mut rings = no_rings();
                    for (i, r) in rings.iter_mut().enumerate() {
                        let t = window(p, i as f32 * 0.14, 1.0);
                        r.radius = 5.0 * s + ease_out(t) * 38.0 * s;
                        r.width = (1.8 + t * 2.2) * s;
                        r.level = if t > 0.0 { 0.75 * (1.0 - t).powf(1.4) } else { 0.0 };
                    }
                    Frame::Ripple {
                        glow,
                        rings,
                        spark: no_spark(),
                    }
                }
                CrossKind::Enter => {
                    let t = window(p, 0.0, 0.7);
                    let k = 0.45 + ease_out(t) * 0.55;
                    let glow = Glow {
                        du: target.0 * k,
                        dv: target.1 * k,
                        radius: 34.0 * s - ease_in(t) * 26.0 * s,
                        level: 0.36 * swell(window(p, 0.0, 0.8), 0.12),
                    };
                    let st = window(p, 0.5, 1.0);
                    let spark = Spark {
                        du: target.0,
                        dv: target.1,
                        inner: 4.0 * s + st * 3.0 * s,
                        outer: 9.0 * s + st * 5.0 * s,
                        level: 0.9 * swell(st, 0.25),
                    };
                    Frame::Ripple {
                        glow,
                        rings: no_rings(),
                        spark,
                    }
                }
            },
            CrossingStyle::Glow => match kind {
                CrossKind::Exit => {
                    let band = Band {
                        center: 2.0 * s + (1.0 - ease_in(p)) * 14.0 * s,
                        dv: dir.1 * 12.0 * s,
                        half: 56.0 * s,
                        core: 1.6 * s,
                        halo: (14.0 - 6.0 * p) * s,
                        level: swell(p, 0.1),
                    };
                    let glow = Glow {
                        du: 2.0 * s,
                        dv: 0.0,
                        radius: 22.0 * s * (1.0 - window(p, 0.0, 0.6)),
                        level: 0.25 * (1.0 - window(p, 0.0, 0.6)),
                    };
                    Frame::Glow { band, glow }
                }
                CrossKind::Enter => {
                    let t = window(p, 0.0, 0.75);
                    let travel = (target.0 - 2.0 * s).max(0.0);
                    let band = Band {
                        center: 2.0 * s + ease_out(t) * travel,
                        dv: target.1 * ease_out(t),
                        half: 56.0 * s,
                        core: 1.6 * s,
                        halo: (8.0 + 12.0 * t) * s,
                        level: swell(p, 0.15),
                    };
                    let glow = Glow {
                        du: 2.0 * s + ease_out(t) * travel,
                        dv: target.1 * ease_out(t),
                        radius: (10.0 + 18.0 * t) * s,
                        level: 0.22 * swell(p, 0.2),
                    };
                    Frame::Glow { band, glow }
                }
            },
            CrossingStyle::Streak => match kind {
                CrossKind::Exit => {
                    let head = ((40.0 - ease_in(p) * 10.0) * s).min(total);
                    let trail = Trail {
                        head,
                        tail: 0.0,
                        thick: (2.0 - 0.6 * p) * s,
                        dot: 2.6 * s,
                        level: swell(p, 0.08),
                    };
                    let glow = Glow {
                        du: 0.0,
                        dv: 0.0,
                        radius: 14.0 * s * (1.0 - window(p, 0.0, 0.5)),
                        level: 0.3 * (1.0 - window(p, 0.0, 0.5)),
                    };
                    Frame::Streak {
                        trail,
                        glow,
                        spark: no_spark(),
                    }
                }
                CrossKind::Enter => {
                    let t = window(p, 0.0, 0.65);
                    let head = (ease_out(t) * 62.0 * s).min(total);
                    let (hu, hv) = path.at(head);
                    let trail = Trail {
                        head,
                        tail: (head - 44.0 * s * (1.0 - window(p, 0.5, 1.0))).max(0.0),
                        thick: 2.0 * s,
                        dot: 2.8 * s,
                        level: swell(p, 0.08).max(if p < 0.6 { 1.0 } else { 0.0 }) * (1.0 - window(p, 0.6, 1.0)),
                    };
                    let st = window(p, 0.4, 1.0);
                    let spark = Spark {
                        du: hu,
                        dv: hv,
                        inner: 4.5 * s + st * 2.0 * s,
                        outer: 10.0 * s + st * 5.0 * s,
                        level: swell(st, 0.3),
                    };
                    let glow = Glow {
                        du: hu,
                        dv: hv,
                        radius: 12.0 * s,
                        level: 0.2 * swell(window(p, 0.2, 1.0), 0.3),
                    };
                    Frame::Streak { trail, glow, spark }
                }
            },
        }
    }

    fn intensity(&self, u: f32, v: f32, path: &Path) -> f32 {
        match self {
            Frame::Ripple { glow, rings, spark } => {
                let mut a = glow.level * blob(u - glow.du, v - glow.dv, glow.radius);
                for r in rings {
                    if r.level > 0.0 {
                        a += r.level * ring(u, v, r.radius, r.width);
                    }
                }
                if spark.level > 0.0 {
                    a += spark.level * sparkle(u - spark.du, v - spark.dv, spark.inner, spark.outer, 6.0);
                }
                a
            }
            Frame::Glow { band, glow } => {
                let mut a = glow.level * blob(u - glow.du, v - glow.dv, glow.radius);
                if band.level > 0.0 {
                    let along = soft((v - band.dv) / band.half);
                    if along > 0.0 {
                        let d = u - band.center;
                        a += (0.85 * bump(d / band.core) + 0.3 * soft(d / band.halo)) * along * band.level;
                    }
                }
                a
            }
            Frame::Streak { trail, glow, spark } => {
                let mut a = glow.level * blob(u - glow.du, v - glow.dv, glow.radius);
                if trail.level > 0.0 {
                    let span = trail.head - trail.tail;
                    if span > 0.5 {
                        if let Some((dist, s)) = path.nearest(u, v, trail.tail, trail.head, trail.thick) {
                            let t = (s - trail.tail) / span;
                            a += 0.8 * t * t * bump(dist / trail.thick) * trail.level;
                        }
                    }
                    let (hu, hv) = path.at(trail.head);
                    a += trail.level * blob(u - hu, v - hv, trail.dot);
                }
                if spark.level > 0.0 {
                    a += spark.level * sparkle(u - spark.du, v - spark.dv, spark.inner, spark.outer, 6.0);
                }
                a
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn take_request() -> Option<Request> {
    PLAYER
        .get()
        .and_then(|p| p.as_ref())
        .and_then(|p| p.pending.lock().ok().and_then(|mut r| r.take()))
}

fn handle(msg: &MSG, current: &mut Option<Anim>) {
    if msg.hwnd.is_invalid() && msg.message == WM_CROSSING_PLAY {
        if let Some(request) = take_request() {
            if let Some(previous) = current.take() {
                tracing::trace!(frames = previous.frames, "animasi perpindahan sebelumnya diganti");
            }
            *current = Anim::new(request);
        }
        return;
    }
    unsafe {
        let _ = TranslateMessage(msg);
        DispatchMessageW(msg);
    }
}

fn finish(hwnd: HWND, anim: Anim) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let elapsed = anim.started.elapsed().as_secs_f32() * 1000.0;
    let fps = if elapsed > 0.0 { anim.frames as f32 * 1000.0 / elapsed } else { 0.0 };
    tracing::trace!(
        frames = anim.frames,
        elapsed_ms = elapsed as u32,
        fps = fps as u32,
        "animasi perpindahan kursor selesai"
    );
}

fn run(ready: mpsc::SyncSender<Option<u32>>) {
    let mut msg = MSG::default();
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    let instance = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(module) => HINSTANCE(module.0),
        Err(e) => {
            tracing::warn!("modul untuk animasi perpindahan tidak didapat: {}", e.message());
            let _ = ready.send(None);
            return;
        }
    };
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: w!("TautraCrossing"),
        ..Default::default()
    };
    unsafe { RegisterClassExW(&class) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("TautraCrossing"),
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
    };
    let hwnd = match hwnd {
        Ok(hwnd) => hwnd,
        Err(e) => {
            tracing::warn!("jendela animasi perpindahan gagal dibuat: {}", e.message());
            let _ = ready.send(None);
            return;
        }
    };
    let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
    let mut current: Option<Anim> = None;
    let mut dwm_ok = true;
    loop {
        if current.is_none() {
            let got = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if got <= 0 {
                break;
            }
            handle(&msg, &mut current);
            continue;
        }
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            handle(&msg, &mut current);
        }
        let Some(anim) = current.as_mut() else {
            continue;
        };
        if !anim.present(hwnd) {
            if let Some(anim) = current.take() {
                finish(hwnd, anim);
            }
            continue;
        }
        if dwm_ok {
            if let Err(e) = unsafe { DwmFlush() } {
                tracing::debug!("DwmFlush tidak tersedia, animasi memakai jeda tetap: {}", e.message());
                dwm_ok = false;
            }
        }
        if !dwm_ok {
            std::thread::sleep(FALLBACK_FRAME);
        }
    }
    tracing::warn!("pompa pesan animasi perpindahan berakhir");
}
