use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tautra_input::Injector;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS,
    DESKTOP_CREATEWINDOW, DESKTOP_JOURNALPLAYBACK, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS,
    DF_ALLOWOTHERACCOUNTHOOK, HDESK, UOI_NAME,
};

use crate::protocol::{ServiceInput, WorkerReply, WorkerRequest};

const IDLE_EXIT: Duration = Duration::from_secs(30);

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
        ServiceInput::Move { x, y } => injector.move_to(x, y),
        ServiceInput::Button { button, down, x, y } => injector.button(button, down, x, y),
        ServiceInput::Wheel { delta, horizontal } => injector.wheel(delta, horizontal),
        ServiceInput::Key { scan, vk, extended, down } => injector.key(scan, vk, extended, down),
        ServiceInput::ReleaseAll => {
            injector.release_all();
            Ok(())
        }
    };
    if let Err(e) = result {
        tracing::debug!("injeksi worker gagal: {e}");
    }
}

pub fn run(pipe_name: &str) {
    let mut binding = DesktopBinding::default();
    binding.follow_input();
    let mut injector = Injector::new();
    let last = AtomicU64::new(now());
    let stop = AtomicBool::new(false);

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
