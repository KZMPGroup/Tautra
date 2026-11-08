use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tautra_common::hotkey::Hotkey;
use tautra_proto::kvm::{KvmMessage, MouseButton};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetMessageW, PeekMessageW, PostThreadMessageW, SetCursorPos,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    KBDLLHOOKSTRUCT_FLAGS, LLKHF_EXTENDED,
    LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};

use crate::cursor;
use crate::geometry::{Crossing, Edge, Geometry, Located, Side};
use crate::InputError;

#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalOutcome {
    pub swallow: bool,
    pub remote: bool,
    pub park: Option<(i32, i32)>,
    pub place: Option<(i32, i32)>,
}

enum ExternalEvent {
    Mouse { msg: u32, x: i32, y: i32, mouse_data: u32 },
    Key { msg: u32, vk: u32, scan: u32, extended: bool },
}

struct ExternalBridge {
    thread_id: u32,
    tx: Sender<(ExternalEvent, SyncSender<ExternalOutcome>)>,
}

static EXTERNAL: Mutex<Option<ExternalBridge>> = Mutex::new(None);
static EXTERNAL_PARK: Mutex<Option<(i32, i32)>> = Mutex::new(None);

thread_local! {
    static PENDING_PLACE: std::cell::Cell<Option<(i32, i32)>> = const { std::cell::Cell::new(None) };
}

fn external_park() -> Option<(i32, i32)> {
    EXTERNAL_PARK.lock().ok().and_then(|p| *p)
}

fn set_external_park(park: Option<(i32, i32)>) {
    if let Ok(mut guard) = EXTERNAL_PARK.lock() {
        *guard = park;
    }
}

fn external_fallback(why: &str) -> ExternalOutcome {
    match external_park() {
        Some(park) => {
            tracing::warn!(?park, "{why}, event dari layanan ditelan agar kursor tetap terparkir");
            ExternalOutcome {
                swallow: true,
                remote: true,
                park: Some(park),
                place: None,
            }
        }
        None => {
            tracing::trace!("{why}, mode lokal, event dari layanan diteruskan");
            ExternalOutcome::default()
        }
    }
}

fn feed(event: ExternalEvent) -> ExternalOutcome {
    let started = Instant::now();
    let (thread_id, tx) = match EXTERNAL.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(bridge) => (bridge.thread_id, bridge.tx.clone()),
            None => return external_fallback("hook input agent belum aktif"),
        },
        Err(_) => return external_fallback("kunci jembatan hook rusak"),
    };
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    if tx.send((event, reply_tx)).is_err() {
        return external_fallback("antrean ke thread hook tertutup");
    }
    if let Err(e) = unsafe { PostThreadMessageW(thread_id, WM_APP, WPARAM(0), LPARAM(0)) } {
        return external_fallback(&format!("thread hook tidak bisa dibangunkan: {}", e.message()));
    }
    match reply_rx.recv_timeout(Duration::from_millis(150)) {
        Ok(outcome) => {
            let took = started.elapsed();
            if took > Duration::from_millis(20) {
                tracing::warn!(?took, ?outcome, "thread hook lambat memproses event dari layanan");
            }
            outcome
        }
        Err(_) => external_fallback("thread hook tidak menjawab dalam 150 ms"),
    }
}

pub fn feed_external_mouse(msg: u32, x: i32, y: i32, mouse_data: u32) -> ExternalOutcome {
    feed(ExternalEvent::Mouse { msg, x, y, mouse_data })
}

pub fn feed_external_key(msg: u32, vk: u32, scan: u32, extended: bool) -> ExternalOutcome {
    feed(ExternalEvent::Key { msg, vk, scan, extended })
}

fn process_external(state: &mut State, event: ExternalEvent) -> ExternalOutcome {
    PENDING_PLACE.set(None);
    let swallow = match event {
        ExternalEvent::Mouse { msg, x, y, mouse_data } => {
            tracing::trace!(msg, x, y, mouse_data, "event mouse dari layanan");
            let info = MSLLHOOKSTRUCT {
                pt: POINT { x, y },
                mouseData: mouse_data,
                flags: 0,
                time: 0,
                dwExtraInfo: 0,
            };
            state.on_mouse(msg, &info)
        }
        ExternalEvent::Key { msg, vk, scan, extended } => {
            tracing::trace!(msg, vk, scan, extended, "event keyboard dari layanan");
            let flags = if extended {
                LLKHF_EXTENDED
            } else {
                KBDLLHOOKSTRUCT_FLAGS(0)
            };
            let info = KBDLLHOOKSTRUCT {
                vkCode: vk,
                scanCode: scan,
                flags,
                time: 0,
                dwExtraInfo: 0,
            };
            state.on_key(msg, &info)
        }
    };
    let outcome = ExternalOutcome {
        swallow,
        remote: state.is_remote(),
        park: match &state.mode {
            Mode::Remote(remote) => Some(remote.park),
            Mode::Local => None,
        },
        place: PENDING_PLACE.take(),
    };
    if let Some(place) = outcome.place {
        tracing::info!(?place, "posisi kursor diserahkan ke worker layanan");
    }
    tracing::trace!(?outcome, "event dari layanan diproses");
    outcome
}

const VK_SHIFT: usize = 0x10;
const VK_CONTROL: usize = 0x11;
const VK_MENU: usize = 0x12;
const VK_LWIN: usize = 0x5B;
const VK_RWIN: usize = 0x5C;
const VK_LSHIFT: usize = 0xA0;
const VK_RSHIFT: usize = 0xA1;
const VK_LCONTROL: usize = 0xA2;
const VK_RCONTROL: usize = 0xA3;
const VK_LMENU: usize = 0xA4;
const VK_RMENU: usize = 0xA5;
const LEFT_BIT: u8 = 1;
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const DROP_TIMEOUT: Duration = Duration::from_millis(1500);
const REMOTE_DRAG_TIMEOUT: Duration = Duration::from_millis(2500);
const REJECT_RETRY: Duration = Duration::from_millis(300);
const FULLSCREEN_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCheck {
    Pending,
    Files,
    NoFiles,
    Dropped,
    Failed,
}

pub trait InputSink: Send + Sync + 'static {
    fn mouse_move(&self, x: i32, y: i32);
    fn message(&self, msg: KvmMessage);
    fn status(&self, status: HookStatus);
    fn drag_probe(&self, x: i32, y: i32);
    fn drag_state(&self) -> DragCheck;
    fn drag_drop(&self);
    fn drag_cancel(&self);
    fn drag_return(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveSide {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookStatus {
    pub active: ActiveSide,
    pub locked: bool,
    pub gaming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hotkeys {
    pub switch_local: Option<Hotkey>,
    pub switch_remote: Option<Hotkey>,
    pub lock: Option<Hotkey>,
    pub gaming: Option<Hotkey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HookOptions {
    pub edge_delay: Duration,
    pub hotkeys: Hotkeys,
    pub gaming_auto: bool,
    pub crossing_animation: bool,
    pub crossing_style: tautra_common::config::CrossingStyle,
}

enum Command {
    Geometry(Option<Geometry>),
    Options(HookOptions),
    Locked(bool),
    Gaming(bool),
    SwitchLocal,
    RemoteDrag(bool),
    Stop,
}

#[derive(Debug, Clone, Copy)]
enum Action {
    SwitchLocal,
    SwitchRemote,
    Lock,
    Gaming,
}

struct RemoteCursor {
    park: (i32, i32),
    x: f64,
    y: f64,
    screen: usize,
    sent: (i32, i32),
}

enum Mode {
    Local,
    Remote(RemoteCursor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragPhase {
    None,
    LocalProbe(Instant),
    LocalDropping(Instant),
    RemoteProbe(Instant),
    RemoteFiles,
    Rejected(Instant),
}

struct State {
    sink: Arc<dyn InputSink>,
    options: HookOptions,
    geometry: Option<Geometry>,
    locked: bool,
    gaming: bool,
    mode: Mode,
    keys_down: [bool; 256],
    local_held: [bool; 256],
    swallow_up: [bool; 256],
    local_buttons: u8,
    remote_buttons: u8,
    local_screen: usize,
    edge_since: Option<(Edge, Instant)>,
    cursor_hidden: bool,
    drag: DragPhase,
    held: Arc<AtomicBool>,
    fullscreen: bool,
    fs_checked: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn button_from(msg: u32, mouse_data: u32) -> Option<(MouseButton, bool)> {
    let x = (mouse_data >> 16) as u16;
    match msg {
        WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
        WM_LBUTTONUP => Some((MouseButton::Left, false)),
        WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
        WM_RBUTTONUP => Some((MouseButton::Right, false)),
        WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
        WM_MBUTTONUP => Some((MouseButton::Middle, false)),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let button = if x == 2 {
                MouseButton::X2
            } else {
                MouseButton::X1
            };
            Some((button, msg == WM_XBUTTONDOWN))
        }
        _ => None,
    }
}

fn button_bit(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
        MouseButton::X1 => 8,
        MouseButton::X2 => 16,
    }
}

fn set_bit(mask: &mut u8, bit: u8, on: bool) {
    if on {
        *mask |= bit;
    } else {
        *mask &= !bit;
    }
}

fn place_cursor(x: i32, y: i32) {
    PENDING_PLACE.set(Some((x, y)));
    let result = unsafe { SetCursorPos(x, y) };
    let now = cursor::position();
    match result {
        Ok(()) => tracing::trace!(x, y, ?now, "posisi kursor diatur"),
        Err(e) if now == Some((x, y)) => {
            tracing::debug!(x, y, "SetCursorPos melapor gagal tetapi kursor sudah di posisi: {}", e.message())
        }
        Err(e) => tracing::warn!(
            x,
            y,
            ?now,
            "posisi kursor gagal diatur dari agent (jendela admin di depan?), worker layanan akan mengatur: {}",
            e.message()
        ),
    }
}

fn repark(park: (i32, i32)) {
    let Some(now) = cursor::position() else {
        return;
    };
    if now != park {
        tracing::debug!(?now, ?park, "kursor fisik bergeser dari parkir, dikembalikan");
        place_cursor(park.0, park.1);
    }
}

impl State {
    fn status(&self) -> HookStatus {
        HookStatus {
            active: match self.mode {
                Mode::Local => ActiveSide::Local,
                Mode::Remote(_) => ActiveSide::Remote,
            },
            locked: self.locked,
            gaming: self.effective_gaming(),
        }
    }

    fn report(&self) {
        set_external_park(match &self.mode {
            Mode::Remote(remote) => Some(remote.park),
            Mode::Local => None,
        });
        let status = self.status();
        tracing::debug!(?status, "status hook dilaporkan");
        self.sink.status(status);
    }

    fn is_remote(&self) -> bool {
        matches!(self.mode, Mode::Remote(_))
    }

    fn effective_gaming(&self) -> bool {
        self.gaming || (self.options.gaming_auto && self.fullscreen)
    }

    fn refresh_fullscreen(&mut self) {
        if !self.options.gaming_auto {
            if self.fullscreen {
                self.fullscreen = false;
                self.report();
            }
            return;
        }
        let due = self
            .fs_checked
            .is_none_or(|t| t.elapsed() >= FULLSCREEN_POLL);
        if !due {
            return;
        }
        self.fs_checked = Some(Instant::now());
        let now = crate::fullscreen::foreground_is_fullscreen();
        if now == self.fullscreen {
            return;
        }
        self.fullscreen = now;
        if now && matches!(self.mode, Mode::Remote(_)) {
            self.return_local(None, true);
        }
        self.report();
    }

    fn usable_geometry(&self) -> Option<&Geometry> {
        self.geometry.as_ref().filter(|g| g.is_usable())
    }

    fn edge_ready(&mut self, edge: Edge) -> bool {
        let delay = self.options.edge_delay;
        if delay.is_zero() {
            return true;
        }
        match self.edge_since {
            Some((e, since)) if e == edge => since.elapsed() >= delay,
            _ => {
                self.edge_since = Some((edge, Instant::now()));
                false
            }
        }
    }

    fn hide_cursor(&mut self) {
        match cursor::hide() {
            Ok(()) => {
                tracing::info!("kursor lokal disembunyikan");
                self.cursor_hidden = true
            }
            Err(e) => {
                tracing::warn!("kursor lokal gagal disembunyikan: {e}");
                self.cursor_hidden = true;
            }
        }
    }

    fn show_cursor(&mut self) {
        if self.cursor_hidden {
            match cursor::restore() {
                Ok(()) => tracing::info!("kursor lokal dipulihkan"),
                Err(e) => tracing::error!("kursor lokal gagal dipulihkan: {e}"),
            }
            self.cursor_hidden = false;
        }
    }

    fn enter_remote(&mut self, target: Crossing) {
        let Some(g) = self.usable_geometry() else {
            return;
        };
        let local = g
            .local
            .screens
            .get(self.local_screen)
            .or_else(|| g.local.screens.first())
            .map(|s| s.phys.center())
            .unwrap_or((0, 0));
        let exit = cursor::position();
        let exit_edge = target.edge.map(Edge::opposite);
        place_cursor(local.0, local.1);
        self.hide_cursor();
        if self.options.crossing_animation {
            if let Some((ex, ey)) = exit {
                crate::crossing::play(ex, ey, crate::crossing::CrossKind::Exit, exit_edge, self.options.crossing_style);
            }
        }
        self.local_held = self.keys_down;
        self.remote_buttons = 0;
        self.local_buttons = 0;
        self.edge_since = None;
        self.drag = DragPhase::None;
        self.held.store(false, Ordering::Release);
        self.mode = Mode::Remote(RemoteCursor {
            park: local,
            x: target.x as f64,
            y: target.y as f64,
            screen: target.screen,
            sent: (target.x, target.y),
        });
        self.sink.message(KvmMessage::Enter {
            x: target.x,
            y: target.y,
            edge: target.edge.map(Into::into),
        });
        tracing::info!(
            x = target.x,
            y = target.y,
            screen = target.screen,
            ?local,
            cursor = ?cursor::position(),
            "input pindah ke perangkat lain"
        );
        self.report();
    }

    fn return_local(&mut self, target: Option<Crossing>, notify_peer: bool) {
        let Mode::Remote(remote) = std::mem::replace(&mut self.mode, Mode::Local) else {
            return;
        };
        if notify_peer {
            self.sink.message(KvmMessage::Leave {
                x: remote.sent.0,
                y: remote.sent.1,
                edge: target.as_ref().and_then(|c| c.edge).map(|e| e.opposite().into()),
            });
        }
        self.show_cursor();
        match target {
            Some(c) => {
                self.local_screen = c.screen;
                place_cursor(c.x, c.y);
                if self.options.crossing_animation {
                    crate::crossing::play(
                        c.x,
                        c.y,
                        crate::crossing::CrossKind::Enter,
                        c.edge,
                        self.options.crossing_style,
                    );
                }
            }
            None => place_cursor(remote.park.0, remote.park.1),
        }
        self.remote_buttons = 0;
        self.edge_since = None;
        self.drag = DragPhase::None;
        self.local_held = [false; 256];
        tracing::info!(
            ?target,
            notify_peer,
            cursor = ?cursor::position(),
            "input kembali ke perangkat ini"
        );
        self.report();
    }

    fn switch_remote(&mut self) {
        if self.effective_gaming() || matches!(self.mode, Mode::Remote(_)) {
            return;
        }
        let Some(g) = self.usable_geometry() else {
            return;
        };
        let Some(screen) = g.remote.primary_screen() else {
            return;
        };
        let (x, y) = g.remote.screens[screen].phys.center();
        let mut point = Default::default();
        if unsafe { GetCursorPos(&mut point) }.is_ok() {
            if let Some(i) = g.local.screen_at(point.x, point.y) {
                self.local_screen = i;
            }
        }
        self.enter_remote(Crossing { screen, x, y, edge: None });
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::SwitchLocal => self.return_local(None, true),
            Action::SwitchRemote => self.switch_remote(),
            Action::Lock => {
                self.locked = !self.locked;
                tracing::info!(locked = self.locked, "kunci kursor diubah lewat hotkey");
                self.report();
            }
            Action::Gaming => self.set_gaming(!self.gaming),
        }
    }

    fn set_gaming(&mut self, on: bool) {
        self.gaming = on;
        if on {
            self.return_local(None, true);
        }
        tracing::info!(gaming = on, "gaming mode diubah");
        self.report();
    }

    fn modifiers(&self) -> (bool, bool, bool, bool) {
        let k = &self.keys_down;
        (
            k[VK_CONTROL] || k[VK_LCONTROL] || k[VK_RCONTROL],
            k[VK_MENU] || k[VK_LMENU] || k[VK_RMENU],
            k[VK_SHIFT] || k[VK_LSHIFT] || k[VK_RSHIFT],
            k[VK_LWIN] || k[VK_RWIN],
        )
    }

    fn match_hotkey(&self, vk: u16) -> Option<Action> {
        let (ctrl, alt, shift, win) = self.modifiers();
        let h = &self.options.hotkeys;
        [
            (h.switch_local, Action::SwitchLocal),
            (h.switch_remote, Action::SwitchRemote),
            (h.lock, Action::Lock),
            (h.gaming, Action::Gaming),
        ]
        .into_iter()
        .find_map(|(key, action)| {
            key.filter(|k| {
                k.vk == vk && k.ctrl == ctrl && k.alt == alt && k.shift == shift && k.win == win
            })
            .map(|_| action)
        })
    }

    fn on_key(&mut self, msg: u32, info: &KBDLLHOOKSTRUCT) -> bool {
        let vk = (info.vkCode & 0xFF) as usize;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let repeat = down && self.keys_down[vk];
        self.keys_down[vk] = down;

        if !down && self.swallow_up[vk] {
            self.swallow_up[vk] = false;
            return true;
        }
        if down {
            if let Some(action) = self.match_hotkey(vk as u16) {
                self.swallow_up[vk] = true;
                if !repeat {
                    self.run_action(action);
                }
                return true;
            }
        }
        if matches!(self.mode, Mode::Local) {
            return false;
        }
        if self.local_held[vk] {
            if !down {
                self.local_held[vk] = false;
            }
            return false;
        }
        self.sink.message(KvmMessage::Key {
            scan: (info.scanCode & 0xFFFF) as u16,
            vk: vk as u16,
            extended: (info.flags & LLKHF_EXTENDED).0 != 0,
            down,
        });
        true
    }

    fn on_mouse(&mut self, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
        if matches!(self.mode, Mode::Local) {
            self.local_mouse(msg, info)
        } else {
            self.remote_mouse(msg, info);
            true
        }
    }

    fn end_local_drag(&mut self) {
        match self.drag {
            DragPhase::LocalProbe(_) | DragPhase::Rejected(_) => {
                self.sink.drag_cancel();
                self.drag = DragPhase::None;
            }
            DragPhase::LocalDropping(_) => self.drag = DragPhase::None,
            _ => {}
        }
    }

    fn local_drag(&mut self, target: Crossing, x: i32, y: i32) -> bool {
        if matches!(self.drag, DragPhase::None | DragPhase::Rejected(_)) && self.sink.drag_return() {
            self.held.store(false, Ordering::Release);
            self.enter_remote(target);
            self.remote_buttons = LEFT_BIT;
            self.sink.message(KvmMessage::DragHold);
            tracing::info!("drag dikembalikan ke perangkat asal");
            return true;
        }
        match self.drag {
            DragPhase::None => {
                self.sink.drag_probe(x, y);
                self.drag = DragPhase::LocalProbe(Instant::now());
                false
            }
            DragPhase::LocalProbe(since) => match self.sink.drag_state() {
                DragCheck::Files => {
                    self.sink.drag_drop();
                    self.drag = DragPhase::LocalDropping(Instant::now());
                    true
                }
                DragCheck::Pending if since.elapsed() < PROBE_TIMEOUT => false,
                _ => {
                    self.sink.drag_cancel();
                    self.drag = DragPhase::Rejected(Instant::now());
                    false
                }
            },
            DragPhase::LocalDropping(since) => match self.sink.drag_state() {
                DragCheck::Dropped => {
                    self.enter_remote(target);
                    self.remote_buttons = LEFT_BIT;
                    self.sink.message(KvmMessage::DragHold);
                    tracing::info!("drag file diteruskan ke perangkat lain");
                    true
                }
                DragCheck::Failed => {
                    self.drag = DragPhase::Rejected(Instant::now());
                    false
                }
                _ if since.elapsed() > DROP_TIMEOUT => {
                    self.sink.drag_cancel();
                    self.drag = DragPhase::Rejected(Instant::now());
                    false
                }
                _ => true,
            },
            DragPhase::Rejected(since) if since.elapsed() > REJECT_RETRY => {
                self.sink.drag_probe(x, y);
                self.drag = DragPhase::LocalProbe(Instant::now());
                false
            }
            _ => false,
        }
    }

    fn remote_drag_ready(&mut self) -> bool {
        match self.drag {
            DragPhase::None => {
                self.sink.message(KvmMessage::DragProbe);
                self.drag = DragPhase::RemoteProbe(Instant::now());
                false
            }
            DragPhase::RemoteProbe(since) => {
                if since.elapsed() > REMOTE_DRAG_TIMEOUT {
                    self.drag = DragPhase::Rejected(Instant::now());
                }
                false
            }
            DragPhase::RemoteFiles => true,
            DragPhase::Rejected(since) if since.elapsed() > REJECT_RETRY => {
                self.sink.message(KvmMessage::DragProbe);
                self.drag = DragPhase::RemoteProbe(Instant::now());
                false
            }
            _ => false,
        }
    }

    fn local_mouse(&mut self, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
        if let Some((button, down)) = button_from(msg, info.mouseData) {
            let bit = button_bit(button);
            if bit == LEFT_BIT && !down {
                self.held.store(false, Ordering::Release);
            }
            set_bit(&mut self.local_buttons, bit, down);
            if self.local_buttons & LEFT_BIT == 0 {
                self.end_local_drag();
            }
            return false;
        }
        if msg != WM_MOUSEMOVE {
            return false;
        }
        let (x, y) = (info.pt.x, info.pt.y);
        self.refresh_fullscreen();
        let dragging = self.local_buttons == LEFT_BIT;
        let blocked = self.locked || self.effective_gaming() || (self.local_buttons != 0 && !dragging);
        let Some(g) = self.usable_geometry() else {
            return false;
        };
        let screen = g.local.screen_at(x, y).unwrap_or(self.local_screen);
        let edge = if blocked {
            None
        } else {
            g.local.exit_edge(screen, x, y)
        };
        self.local_screen = screen;
        let Some(edge) = edge else {
            self.edge_since = None;
            if !matches!(self.drag, DragPhase::LocalDropping(_)) {
                self.end_local_drag();
            }
            return false;
        };
        if !self.edge_ready(edge) {
            return false;
        }
        let target = self
            .usable_geometry()
            .and_then(|g| g.cross(Side::Local, screen, edge, x as f64, y as f64));
        match target {
            Some(target) if dragging => self.local_drag(target, x, y),
            Some(target) => {
                self.enter_remote(target);
                true
            }
            None => false,
        }
    }

    fn remote_mouse(&mut self, msg: u32, info: &MSLLHOOKSTRUCT) {
        let Mode::Remote(remote) = &mut self.mode else {
            return;
        };
        if let Some((button, down)) = button_from(msg, info.mouseData) {
            let bit = button_bit(button);
            let (x, y) = remote.sent;
            set_bit(&mut self.remote_buttons, bit, down);
            if self.remote_buttons & LEFT_BIT == 0 {
                self.drag = DragPhase::None;
            }
            self.sink.message(KvmMessage::Button { button, down, x, y });
            return;
        }
        match msg {
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                self.sink.message(KvmMessage::Wheel {
                    delta: (info.mouseData >> 16) as u16 as i16 as i32,
                    horizontal: msg == WM_MOUSEHWHEEL,
                });
            }
            WM_MOUSEMOVE => self.remote_move(info.pt.x, info.pt.y),
            _ => {}
        }
    }

    fn remote_move(&mut self, px: i32, py: i32) {
        let Mode::Remote(remote) = &self.mode else {
            return;
        };
        let (park, rx, ry, current) = (remote.park, remote.x, remote.y, remote.screen);
        let dx = px - park.0;
        let dy = py - park.1;
        if dx == 0 && dy == 0 {
            return;
        }
        if dx.abs() > 400 || dy.abs() > 400 {
            tracing::warn!(px, py, ?park, dx, dy, "lompatan mouse besar dari titik parkir");
        }
        let Some(g) = self.usable_geometry() else {
            tracing::warn!("geometri tidak bisa dipakai, input dikembalikan ke perangkat ini");
            self.return_local(None, true);
            return;
        };
        let factor = g.speed_factor();
        let nx = rx + dx as f64 * factor;
        let ny = ry + dy as f64 * factor;
        let (x, y, screen, edge) = match g.remote.locate(current, nx, ny) {
            Located::Inside(i) => (nx, ny, i, None),
            Located::Outside(edge) => {
                let screen = current.min(g.remote.screens.len() - 1);
                let (cx, cy) = g.remote.screens[screen].phys.clamp(nx, ny);
                (cx, cy, screen, Some(edge))
            }
        };
        match edge {
            Some(edge) => {
                let dragging = self.remote_buttons == LEFT_BIT;
                if !self.locked && (self.remote_buttons == 0 || dragging) && self.edge_ready(edge) {
                    let target = self
                        .usable_geometry()
                        .and_then(|g| g.cross(Side::Remote, screen, edge, x, y));
                    if let Some(target) = target {
                        if !dragging {
                            self.return_local(Some(target), true);
                            return;
                        }
                        if self.remote_drag_ready() {
                            self.return_local(Some(target), true);
                            self.local_buttons = LEFT_BIT;
                            self.held.store(true, Ordering::Release);
                            tracing::info!("drag file dari perangkat lain diterima di sini");
                            return;
                        }
                    }
                }
            }
            None => {
                self.edge_since = None;
                if matches!(self.drag, DragPhase::Rejected(_)) {
                    self.drag = DragPhase::None;
                }
            }
        }
        let Mode::Remote(remote) = &mut self.mode else {
            return;
        };
        remote.x = x;
        remote.y = y;
        remote.screen = screen;
        let point = (x.floor() as i32, y.floor() as i32);
        tracing::trace!(px, py, dx, dy, ?point, screen, ?edge, "gerak mouse jarak jauh");
        if point != remote.sent {
            remote.sent = point;
            self.sink.mouse_move(point.0, point.1);
        }
        repark(park);
    }

    fn apply(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Geometry(geometry) => {
                self.geometry = geometry;
                let fallback = match (&self.mode, self.usable_geometry()) {
                    (Mode::Remote(_), None) => Some(None),
                    (Mode::Remote(r), Some(g)) if r.screen >= g.remote.screens.len() => {
                        g.remote.primary_screen().map(|i| {
                            let (x, y) = g.remote.screens[i].phys.center();
                            Some((i, x, y))
                        })
                    }
                    _ => None,
                };
                match fallback {
                    Some(None) => self.return_local(None, true),
                    Some(Some((screen, x, y))) => {
                        if let Mode::Remote(r) = &mut self.mode {
                            r.screen = screen;
                            r.x = x as f64;
                            r.y = y as f64;
                            r.sent = (x, y);
                            self.sink.mouse_move(x, y);
                        }
                    }
                    None => {}
                }
                if let Some(g) = self.usable_geometry() {
                    if self.local_screen >= g.local.screens.len() {
                        self.local_screen = 0;
                    }
                }
            }
            Command::Options(options) => self.options = options,
            Command::Locked(on) => {
                self.locked = on;
                self.report();
            }
            Command::Gaming(on) => self.set_gaming(on),
            Command::SwitchLocal => self.return_local(None, true),
            Command::RemoteDrag(files) => {
                if matches!(self.drag, DragPhase::RemoteProbe(_)) {
                    self.drag = if files {
                        DragPhase::RemoteFiles
                    } else {
                        DragPhase::Rejected(Instant::now())
                    };
                }
            }
            Command::Stop => return false,
        }
        true
    }

    fn shutdown(&mut self) {
        self.return_local(None, true);
        self.show_cursor();
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 {
            let swallow = STATE.with(|cell| match cell.try_borrow_mut() {
                Ok(mut state) => state
                    .as_mut()
                    .is_some_and(|s| s.on_mouse(wparam.0 as u32, info)),
                Err(_) => false,
            });
            if swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (info.flags & LLKHF_INJECTED).0 == 0 {
            let swallow = STATE.with(|cell| match cell.try_borrow_mut() {
                Ok(mut state) => state
                    .as_mut()
                    .is_some_and(|s| s.on_key(wparam.0 as u32, info)),
                Err(_) => false,
            });
            if swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

struct Init {
    sink: Arc<dyn InputSink>,
    options: HookOptions,
    geometry: Option<Geometry>,
    locked: bool,
    gaming: bool,
    held: Arc<AtomicBool>,
}

fn install() -> Result<(HHOOK, HHOOK), InputError> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) }
        .map_err(|e| InputError::Hook(format!("mouse: {}", e.message())))?;
    match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0) } {
        Ok(keyboard) => Ok((mouse, keyboard)),
        Err(e) => {
            let _ = unsafe { UnhookWindowsHookEx(mouse) };
            Err(InputError::Hook(format!("keyboard: {}", e.message())))
        }
    }
}

fn run(init: Init, rx: Receiver<Command>, ready: SyncSender<Result<u32, InputError>>) {
    let mut msg = MSG::default();
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    if let Err(e) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) }
    {
        tracing::warn!("prioritas thread input gagal dinaikkan: {}", e.message());
    }
    let (mouse, keyboard) = match install() {
        Ok(hooks) => hooks,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(State {
            sink: init.sink,
            options: init.options,
            geometry: init.geometry,
            locked: init.locked,
            gaming: init.gaming,
            mode: Mode::Local,
            keys_down: [false; 256],
            local_held: [false; 256],
            swallow_up: [false; 256],
            local_buttons: 0,
            remote_buttons: 0,
            local_screen: 0,
            edge_since: None,
            cursor_hidden: false,
            drag: DragPhase::None,
            held: init.held,
            fullscreen: false,
            fs_checked: None,
        });
    });
    let thread_id = unsafe { GetCurrentThreadId() };
    let (ext_tx, ext_rx) = mpsc::channel::<(ExternalEvent, SyncSender<ExternalOutcome>)>();
    if let Ok(mut guard) = EXTERNAL.lock() {
        *guard = Some(ExternalBridge {
            thread_id,
            tx: ext_tx,
        });
    }
    let _ = ready.send(Ok(thread_id));
    tracing::info!("hook input aktif");

    'pump: loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            break;
        }
        if msg.message != WM_APP {
            continue;
        }
        while let Ok(cmd) = rx.try_recv() {
            let keep = STATE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .is_none_or(|state| state.apply(cmd))
            });
            if !keep {
                break 'pump;
            }
        }
        while let Ok((event, reply)) = ext_rx.try_recv() {
            let outcome = STATE.with(|cell| {
                cell.borrow_mut()
                    .as_mut()
                    .map(|state| process_external(state, event))
                    .unwrap_or_default()
            });
            let _ = reply.send(outcome);
        }
    }

    if let Ok(mut guard) = EXTERNAL.lock() {
        *guard = None;
    }
    STATE.with(|cell| {
        if let Some(mut state) = cell.borrow_mut().take() {
            state.shutdown();
        }
    });
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard);
        let _ = UnhookWindowsHookEx(mouse);
    }
    tracing::info!("hook input dilepas");
}

pub struct HookController {
    thread_id: u32,
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
    held: Arc<AtomicBool>,
}

impl HookController {
    pub fn start(
        sink: Arc<dyn InputSink>,
        options: HookOptions,
        geometry: Option<Geometry>,
        locked: bool,
        gaming: bool,
    ) -> Result<Self, InputError> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let held = Arc::new(AtomicBool::new(false));
        let init = Init {
            sink,
            options,
            geometry,
            locked,
            gaming,
            held: held.clone(),
        };
        let join = std::thread::Builder::new()
            .name("tautra-input".into())
            .spawn(move || run(init, rx, ready_tx))
            .map_err(|e| InputError::Thread(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                tx,
                join: Some(join),
                held,
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(InputError::Thread(
                    "thread input berhenti sebelum siap".into(),
                ))
            }
        }
    }

    fn send(&self, cmd: Command) {
        if self.tx.send(cmd).is_ok() {
            if let Err(e) =
                unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) }
            {
                tracing::warn!("perintah ke thread input gagal dikirim: {}", e.message());
            }
        }
    }

    pub fn set_geometry(&self, geometry: Option<Geometry>) {
        self.send(Command::Geometry(geometry));
    }

    pub fn set_options(&self, options: HookOptions) {
        self.send(Command::Options(options));
    }

    pub fn set_locked(&self, on: bool) {
        self.send(Command::Locked(on));
    }

    pub fn set_gaming(&self, on: bool) {
        self.send(Command::Gaming(on));
    }

    pub fn switch_local(&self) {
        self.send(Command::SwitchLocal);
    }

    pub fn remote_drag(&self, files: bool) {
        self.send(Command::RemoteDrag(files));
    }

    pub fn drag_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }
}

impl Drop for HookController {
    fn drop(&mut self) {
        self.send(Command::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
