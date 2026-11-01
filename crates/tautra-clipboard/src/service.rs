use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;

use tautra_proto::share::{ClipFormat, FileEntry};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardOwner, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    OleFlushClipboard, OleInitialize, OleIsCurrentClipboard, OleSetClipboard, OleUninitialize,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowThreadProcessId, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetTimer, TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CLIPBOARDUPDATE, WM_CLOSE, WM_DESTROY, WM_TIMER, WNDCLASSEXW,
};

use crate::data_object::RemoteData;
use crate::read;

const WM_COMMAND_READY: u32 = WM_APP + 1;
const DEBOUNCE_TIMER: usize = 1;
const DEBOUNCE_MS: u32 = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardOptions {
    pub enabled: bool,
    pub text: bool,
    pub html: bool,
    pub rtf: bool,
    pub image: bool,
    pub files: bool,
    pub max_bytes: u64,
}

#[derive(Debug, Clone)]
pub enum LocalImage {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

#[derive(Debug, Clone, Default)]
pub struct LocalContent {
    pub text: Option<String>,
    pub html: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
    pub image: Option<LocalImage>,
    pub files: Vec<PathBuf>,
}

impl LocalContent {
    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.html.is_none()
            && self.rtf.is_none()
            && self.image.is_none()
            && self.files.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct RemoteItem {
    pub format: ClipFormat,
    pub inline: Option<Vec<u8>>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct RemoteFiles {
    pub offer_id: u64,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Clone)]
pub struct RemoteContent {
    pub offer_id: u64,
    pub items: Vec<RemoteItem>,
    pub files: Option<RemoteFiles>,
}

impl RemoteContent {
    fn flushable(&self) -> bool {
        self.files.is_none() && self.items.iter().all(|i| i.inline.is_some())
    }
}

pub trait RemoteFile: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String>;
}

pub trait ClipboardHandler: Send + Sync + 'static {
    fn local_changed(&self, content: LocalContent);
    fn fetch(&self, offer_id: u64, format: ClipFormat) -> Result<Vec<u8>, String>;
    fn open_file(&self, offer_id: u64, index: u32, size: u64)
        -> Result<Box<dyn RemoteFile>, String>;
    fn error(&self, message: String);
}

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("clipboard gagal disiapkan: {0}")]
    Setup(String),
}

enum Command {
    Remote(RemoteContent),
    Options(ClipboardOptions),
}

struct State {
    handler: Arc<dyn ClipboardHandler>,
    options: ClipboardOptions,
    rx: Receiver<Command>,
    current: Option<IDataObject>,
    flushable: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn owned_by_us() -> bool {
    let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
        return false;
    };
    if owner.is_invalid() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

fn on_debounce(hwnd: HWND) {
    let Some((handler, options)) = STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| (s.handler.clone(), s.options))
    }) else {
        return;
    };
    if !options.enabled || owned_by_us() {
        return;
    }
    match read::snapshot(hwnd, &options) {
        Ok(Some(content)) if !content.is_empty() => {
            STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.current = None;
                }
            });
            handler.local_changed(content);
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("clipboard lokal gagal dibaca: {e}"),
    }
}

fn apply_remote(content: RemoteContent) {
    let Some(handler) = STATE.with(|cell| cell.borrow().as_ref().map(|s| s.handler.clone())) else {
        return;
    };
    let flushable = content.flushable();
    let object: IDataObject = RemoteData::new(content, handler.clone()).into();
    match unsafe { OleSetClipboard(&object) } {
        Ok(()) => STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.current = Some(object);
                state.flushable = flushable;
            }
        }),
        Err(e) => {
            tracing::error!("clipboard dari perangkat lain gagal dipasang: {}", e.message());
            handler.error(format!(
                "Clipboard dari perangkat lain gagal dipasang: {}",
                e.message()
            ));
        }
    }
}

fn drain_commands() {
    loop {
        let cmd = STATE.with(|cell| cell.borrow().as_ref().and_then(|s| s.rx.try_recv().ok()));
        match cmd {
            Some(Command::Remote(content)) => apply_remote(content),
            Some(Command::Options(options)) => STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.options = options;
                }
            }),
            None => break,
        }
    }
}

fn release_clipboard() {
    let current = STATE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .and_then(|s| s.current.take().map(|c| (c, s.flushable)))
    });
    let Some((object, flushable)) = current else {
        return;
    };
    if unsafe { OleIsCurrentClipboard(&object) }.is_err() {
        return;
    }
    let result = if flushable {
        unsafe { OleFlushClipboard() }
    } else {
        unsafe { OleSetClipboard(None::<&IDataObject>) }
    };
    if let Err(e) = result {
        tracing::warn!("clipboard Tautra gagal dilepas: {}", e.message());
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if !owned_by_us() {
                unsafe { SetTimer(Some(hwnd), DEBOUNCE_TIMER, DEBOUNCE_MS, None) };
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DEBOUNCE_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), DEBOUNCE_TIMER) };
            on_debounce(hwnd);
            LRESULT(0)
        }
        WM_COMMAND_READY => {
            drain_commands();
            LRESULT(0)
        }
        WM_CLOSE => {
            release_clipboard();
            unsafe {
                let _ = RemoveClipboardFormatListener(hwnd);
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

fn create_window() -> Result<HWND, ClipboardError> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|e| ClipboardError::Setup(e.message()))?;
    let instance = HINSTANCE(module.0);
    let class = w!("TautraClipboard");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        lpszClassName: class,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&wc) } == 0 {
        return Err(ClipboardError::Setup(
            windows::core::Error::from_thread().message(),
        ));
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Tautra Clipboard"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|e| ClipboardError::Setup(e.message()))?;
    if let Err(e) = unsafe { AddClipboardFormatListener(hwnd) } {
        let _ = unsafe { DestroyWindow(hwnd) };
        return Err(ClipboardError::Setup(format!(
            "AddClipboardFormatListener: {}",
            e.message()
        )));
    }
    Ok(hwnd)
}

fn run(
    handler: Arc<dyn ClipboardHandler>,
    options: ClipboardOptions,
    rx: Receiver<Command>,
    ready: SyncSender<Result<isize, ClipboardError>>,
) {
    if let Err(e) = unsafe { OleInitialize(None) } {
        let _ = ready.send(Err(ClipboardError::Setup(format!(
            "OleInitialize: {}",
            e.message()
        ))));
        return;
    }
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(State {
            handler,
            options,
            rx,
            current: None,
            flushable: false,
        })
    });
    match create_window() {
        Ok(hwnd) => {
            let _ = ready.send(Ok(hwnd.0 as isize));
            tracing::info!("pemantau clipboard aktif");
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
    STATE.with(|cell| *cell.borrow_mut() = None);
    unsafe { OleUninitialize() };
    tracing::info!("pemantau clipboard berhenti");
}

pub struct ClipboardService {
    hwnd: isize,
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl ClipboardService {
    pub fn start(
        handler: Arc<dyn ClipboardHandler>,
        options: ClipboardOptions,
    ) -> Result<Self, ClipboardError> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("tautra-clipboard".into())
            .spawn(move || run(handler, options, rx, ready_tx))
            .map_err(|e| ClipboardError::Setup(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                tx,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(ClipboardError::Setup(
                    "thread clipboard berhenti sebelum siap".into(),
                ))
            }
        }
    }

    fn post(&self, msg: u32) {
        let hwnd = HWND(self.hwnd as *mut _);
        if let Err(e) = unsafe { PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0)) } {
            tracing::warn!("perintah clipboard gagal dikirim: {}", e.message());
        }
    }

    fn send(&self, cmd: Command) {
        if self.tx.send(cmd).is_ok() {
            self.post(WM_COMMAND_READY);
        }
    }

    pub fn set_remote(&self, content: RemoteContent) {
        self.send(Command::Remote(content));
    }

    pub fn set_options(&self, options: ClipboardOptions) {
        self.send(Command::Options(options));
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.post(WM_CLOSE);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
