use std::cell::Cell;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostQuitMessage, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    PM_NOREMOVE, WH_MOUSE_LL, WM_QUIT,
};

use crate::cursor;
use crate::InputError;

thread_local! {
    static REVEALED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn reveal_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 && !REVEALED.get() {
            REVEALED.set(true);
            if let Err(e) = cursor::restore() {
                tracing::warn!("kursor gagal dimunculkan kembali: {e}");
            }
            unsafe { PostQuitMessage(0) };
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn run(ready: mpsc::SyncSender<Result<u32, InputError>>) {
    let mut msg = MSG::default();
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    let hook = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(reveal_proc), module, 0) } {
        Ok(hook) => hook,
        Err(e) => {
            let _ = ready.send(Err(InputError::Hook(format!("mouse: {}", e.message()))));
            return;
        }
    };
    if let Err(e) = cursor::hide() {
        tracing::warn!("kursor perangkat ini gagal disembunyikan: {e}");
    }
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
    }
    if !REVEALED.get() {
        if let Err(e) = cursor::restore() {
            tracing::warn!("kursor gagal dimunculkan kembali: {e}");
        }
    }
}

pub struct CursorHider {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl CursorHider {
    pub fn start() -> Result<Self, InputError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("tautra-cursor".into())
            .spawn(move || run(ready_tx))
            .map_err(|e| InputError::Thread(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                join: Some(join),
            }),
            Ok(Err(e)) => {
                let _ = join.join();
                Err(e)
            }
            Err(_) => {
                let _ = join.join();
                Err(InputError::Thread(
                    "thread kursor berhenti sebelum siap".into(),
                ))
            }
        }
    }
}

impl Drop for CursorHider {
    fn drop(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
