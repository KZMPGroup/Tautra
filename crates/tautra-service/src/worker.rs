use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tautra_input::Injector;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS,
    DESKTOP_CREATEWINDOW, DESKTOP_JOURNALPLAYBACK, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS,
    DF_ALLOWOTHERACCOUNTHOOK, HDESK, UOI_NAME,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetCursorPos, GetMessageW, PeekMessageW, SetCursorPos, SetWindowsHookExW,
    UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED,
    MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_MOUSEMOVE,
};

use crate::protocol::{
    AssistInput, AssistReply, ServiceInput, WorkerReply, WorkerRequest, INPUT_ASSIST_PIPE,
};

const IDLE_EXIT: Duration = Duration::from_secs(30);

static ASSIST_ENABLED: AtomicBool = AtomicBool::new(false);
static ASSIST_REMOTE: AtomicBool = AtomicBool::new(false);

static ASSIST_SEEN: AtomicBool = AtomicBool::new(false);

fn assist_call(input: AssistInput) -> AssistReply {
    let Ok(bytes) = serde_json::to_vec(&input) else {
        return AssistReply::default();
    };
    let started = Instant::now();
    let reply = match crate::pipe::call(INPUT_ASSIST_PIPE, &bytes) {
        Ok(reply) => {
            if !ASSIST_SEEN.swap(true, Ordering::Relaxed) {
                tracing::info!("input di atas jendela admin diteruskan ke agent");
            }
            serde_json::from_slice::<AssistReply>(&reply).unwrap_or_else(|e| {
                tracing::warn!("jawaban agent untuk hook bantuan tidak dimengerti: {e}");
                AssistReply::default()
            })
        }
        Err(e) => {
            if ASSIST_SEEN.swap(false, Ordering::Relaxed) {
                tracing::warn!("agent tidak bisa dihubungi dari hook bantuan: {e}");
            }
            ASSIST_REMOTE.store(false, Ordering::Relaxed);
            AssistReply::default()
        }
    };
    let took = started.elapsed();
    if took > Duration::from_millis(30) {
        tracing::warn!(?took, ?input, ?reply, "hook bantuan lambat, Windows bisa melepas hook");
    } else {
        tracing::trace!(?took, ?input, ?reply, "hook bantuan diteruskan");
    }
    if let Some(place) = reply.place {
        match unsafe { SetCursorPos(place.0, place.1) } {
            Ok(()) => {
                let mut now = windows::Win32::Foundation::POINT::default();
                let _ = unsafe { GetCursorPos(&mut now) };
                tracing::info!(?place, now = ?(now.x, now.y), "worker meletakkan kursor sesuai permintaan agent");
            }
            Err(e) => tracing::warn!(?place, "worker gagal meletakkan kursor: {}", e.message()),
        }
    } else if let (true, Some(park)) = (reply.swallow, reply.park) {
        let mut now = windows::Win32::Foundation::POINT::default();
        if unsafe { GetCursorPos(&mut now) }.is_ok() && (now.x, now.y) != park {
            match unsafe { SetCursorPos(park.0, park.1) } {
                Ok(()) => tracing::debug!(now = ?(now.x, now.y), ?park, "kursor fisik dikembalikan ke parkir oleh worker"),
                Err(e) => tracing::warn!(?park, "worker gagal memarkir kursor: {}", e.message()),
            }
        }
    }
    reply
}

static ASSIST_ABOVE: AtomicBool = AtomicBool::new(false);

fn assist_active() -> bool {
    if !ASSIST_ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    let above = tautra_input::elevation::foreground_above_medium();
    if ASSIST_ABOVE.swap(above, Ordering::Relaxed) != above {
        tracing::info!(
            above,
            remote = ASSIST_REMOTE.load(Ordering::Relaxed),
            "jendela depan {}",
            if above { "di atas medium, hook bantuan mengambil alih" } else { "kembali medium, hook agent mengambil alih" }
        );
    }
    above
}

unsafe extern "system" fn assist_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 && assist_active() {
            let msg = wparam.0 as u32;
            let is_move = msg == WM_MOUSEMOVE;
            if ASSIST_REMOTE.load(Ordering::Relaxed) || is_move {
                let reply = assist_call(AssistInput::Mouse {
                    msg,
                    x: info.pt.x,
                    y: info.pt.y,
                    mouse_data: info.mouseData,
                });
                if ASSIST_REMOTE.swap(reply.remote, Ordering::Relaxed) != reply.remote {
                    tracing::info!(remote = reply.remote, "status jarak jauh dari agent berubah di hook bantuan");
                }
                if reply.swallow {
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn assist_keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if (info.flags & LLKHF_INJECTED).0 == 0
            && assist_active()
            && ASSIST_REMOTE.load(Ordering::Relaxed)
        {
            let reply = assist_call(AssistInput::Key {
                msg: wparam.0 as u32,
                vk: info.vkCode,
                scan: info.scanCode,
                extended: (info.flags & LLKHF_EXTENDED).0 != 0,
            });
            if ASSIST_REMOTE.swap(reply.remote, Ordering::Relaxed) != reply.remote {
                tracing::info!(remote = reply.remote, "status jarak jauh dari agent berubah di hook bantuan");
            }
            if reply.swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn run_assist_hooks() {
    let mut msg = MSG::default();
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .ok()
        .map(|m| HINSTANCE(m.0));
    let mouse = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(assist_mouse), module, 0) } {
        Ok(hook) => hook,
        Err(e) => {
            tracing::warn!("hook bantuan mouse gagal dipasang: {}", e.message());
            return;
        }
    };
    let keyboard = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(assist_keyboard), module, 0) }
    {
        Ok(hook) => hook,
        Err(e) => {
            tracing::warn!("hook bantuan keyboard gagal dipasang: {}", e.message());
            let _ = unsafe { UnhookWindowsHookEx(mouse) };
            return;
        }
    };
    tracing::info!("hook bantuan input layanan aktif");
    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            break;
        }
    }
    tracing::warn!("pompa pesan hook bantuan berakhir, hook dilepas");
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard);
        let _ = UnhookWindowsHookEx(mouse);
    }
}

fn desktop_name(desktop: HDESK) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut needed = 0u32;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(buffer.as_mut_ptr() as *mut c_void),
            (buffer.len() * 2) as u32,
            Some(&mut needed),
        )
    }
    .ok()?;
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

#[derive(Default)]
struct DesktopBinding {
    current: Option<HDESK>,
    name: String,
}

impl DesktopBinding {
    fn follow_input(&mut self) {
        let access = DESKTOP_ACCESS_FLAGS(
            DESKTOP_READOBJECTS.0
                | DESKTOP_WRITEOBJECTS.0
                | DESKTOP_CREATEWINDOW.0
                | DESKTOP_JOURNALPLAYBACK.0,
        );
        let desktop = match unsafe { OpenInputDesktop(DF_ALLOWOTHERACCOUNTHOOK, false, access) } {
            Ok(desktop) => desktop,
            Err(e) => {
                tracing::warn!("desktop input tidak bisa dibuka: {}", e.message());
                return;
            }
        };
        let name = desktop_name(desktop).unwrap_or_default();
        if self.current.is_some() && name == self.name {
            let _ = unsafe { CloseDesktop(desktop) };
            return;
        }
        match unsafe { SetThreadDesktop(desktop) } {
            Ok(()) => {
                if let Some(old) = self.current.replace(desktop) {
                    let _ = unsafe { CloseDesktop(old) };
                }
                tracing::info!(desktop = %name, "worker injeksi pindah ke desktop input");
                self.name = name;
            }
            Err(e) => {
                tracing::warn!("worker gagal pindah ke desktop {name}: {}", e.message());
                let _ = unsafe { CloseDesktop(desktop) };
            }
        }
    }
}

fn send_sas() {
    unsafe { windows::Win32::Security::Authentication::Identity::SendSAS(false) };
}

fn apply(injector: &mut Injector, event: ServiceInput) {
    let result = match event {
        ServiceInput::Move { x, y } => {
            tracing::trace!(x, y, "worker injeksi gerak");
            injector.move_to(x, y)
        }
        ServiceInput::Button { button, down, x, y } => injector.button(button, down, x, y),
        ServiceInput::Wheel { delta, horizontal } => injector.wheel(delta, horizontal),
        ServiceInput::Key { scan, vk, extended, down } => injector.key(scan, vk, extended, down),
        ServiceInput::ReleaseAll => {
            injector.release_all();
            Ok(())
        }
    };
    match result {
        Ok(()) => tracing::trace!("injeksi worker berhasil"),
        Err(e) => tracing::warn!("injeksi worker gagal: {e}"),
    }
}

pub fn run(pipe_name: &str) {
    let mut binding = DesktopBinding::default();
    binding.follow_input();
    let mut injector = Injector::new();
    let last = AtomicU64::new(now());
    let stop = AtomicBool::new(false);

    let _ = std::thread::Builder::new()
        .name("tautra-assist".into())
        .spawn(run_assist_hooks);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
                if now().saturating_sub(last.load(Ordering::SeqCst)) > IDLE_EXIT.as_secs() {
                    tracing::info!("worker injeksi berhenti karena tidak ada kontak dari layanan");
                    std::process::exit(0);
                }
            }
        });

        let _ = crate::pipe::serve(pipe_name, crate::protocol::WORKER_SDDL, &stop, |request| {
            last.store(now(), Ordering::SeqCst);
            let reply = match serde_json::from_slice::<WorkerRequest>(request) {
                Ok(WorkerRequest::Inject(events)) => {
                    binding.follow_input();
                    for event in events {
                        apply(&mut injector, event);
                    }
                    WorkerReply::Ok
                }
                Ok(WorkerRequest::SendSas) => {
                    send_sas();
                    WorkerReply::Ok
                }
                Ok(WorkerRequest::Ping) => {
                    binding.follow_input();
                    WorkerReply::Desktop(binding.name.clone())
                }
                Ok(WorkerRequest::SetAssist(on)) => {
                    if ASSIST_ENABLED.swap(on, Ordering::Relaxed) != on {
                        tracing::info!(aktif = on, "hook bantuan input worker diubah");
                    }
                    if !on {
                        ASSIST_REMOTE.store(false, Ordering::Relaxed);
                    }
                    WorkerReply::Ok
                }
                Err(e) => WorkerReply::Error(format!("permintaan tidak dimengerti: {e}")),
            };
            serde_json::to_vec(&reply).unwrap_or_default()
        });
    });
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
