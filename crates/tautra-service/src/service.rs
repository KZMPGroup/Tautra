use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CALL_NOT_IMPLEMENTED, HANDLE, NO_ERROR, WAIT_TIMEOUT};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityIdentification, SetTokenInformation, TokenPrimary, TokenSessionId,
    TOKEN_ALL_ACCESS,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQuerySessionInformationW, WTSUserName,
    WTS_CURRENT_SERVER_HANDLE,
};
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
    SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_STATUS,
    SERVICE_STATUS_CURRENT_STATE, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, TerminateProcess,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, NORMAL_PRIORITY_CLASS,
    PROCESS_INFORMATION, STARTUPINFOW,
};

use crate::protocol::{
    ControlReply, ControlRequest, ServiceState, WorkerReply, WorkerRequest, CONTROL_PIPE,
    CONTROL_SDDL, LOGON_ARG, SERVICE_DISPLAY_NAME, WORKER_ARG,
};
use crate::{wide, ServiceError};
use tautra_common::build;

const WORKER_PIPE: &str = r"\\.\pipe\tautra-service-worker";
const AGENT_EXE: &str = "tautra-agent.exe";
const PING_INTERVAL: Duration = Duration::from_secs(2);
const LOGON_BACKOFF: Duration = Duration::from_secs(5);

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicUsize = AtomicUsize::new(0);

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

struct Child {
    handle: HANDLE,
}

impl Child {
    fn alive(&self) -> bool {
        (unsafe { WaitForSingleObject(self.handle, 0) }) == WAIT_TIMEOUT
    }

    fn terminate(&self) {
        let _ = unsafe { TerminateProcess(self.handle, 1) };
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }
}

fn active_session() -> Option<u32> {
    let id = unsafe { WTSGetActiveConsoleSessionId() };
    (id != 0xFFFF_FFFF).then_some(id)
}

fn user_signed_in(session: u32) -> bool {
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    let queried = unsafe {
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            session,
            WTSUserName,
            &mut buffer,
            &mut bytes,
        )
    };
    if queried.is_err() || buffer.is_null() {
        return true;
    }
    let name = unsafe { buffer.to_string() }.unwrap_or_default();
    unsafe { WTSFreeMemory(buffer.0 as *mut c_void) };
    !name.trim().is_empty()
}

fn service_token(session: u32) -> Option<Token> {
    let mut raw = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut raw) }.ok()?;
    let source = Token(raw);
    let mut dup = HANDLE::default();
    unsafe {
        DuplicateTokenEx(
            source.0,
            TOKEN_ALL_ACCESS,
            None,
            SecurityIdentification,
            TokenPrimary,
            &mut dup,
        )
    }
    .ok()?;
    let token = Token(dup);
    let sid = session;
    unsafe {
        SetTokenInformation(
            token.0,
            TokenSessionId,
            &sid as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        )
    }
    .ok()?;
    Some(token)
}

fn spawn(token: &Token, command: &str, desktop: Option<&str>) -> Option<Child> {
    let mut cmdline = wide(command);
    let mut desktop_buf = desktop.map(|d| wide(&format!("WinSta0\\{d}")));
    let mut env: *mut c_void = std::ptr::null_mut();
    let has_env = unsafe { CreateEnvironmentBlock(&mut env, Some(token.0), false) }.is_ok();
    let mut startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    if let Some(buf) = desktop_buf.as_mut() {
        startup.lpDesktop = PWSTR(buf.as_mut_ptr());
    }
    let mut info = PROCESS_INFORMATION::default();
    let flags = CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | NORMAL_PRIORITY_CLASS;
    let result = unsafe {
        CreateProcessAsUserW(
            Some(token.0),
            PCWSTR::null(),
            Some(PWSTR(cmdline.as_mut_ptr())),
            None,
            None,
            false,
            flags,
            if env.is_null() { None } else { Some(env) },
            PCWSTR::null(),
            &startup,
            &mut info,
        )
    };
    if has_env && !env.is_null() {
        let _ = unsafe { DestroyEnvironmentBlock(env) };
    }
    match result {
        Ok(()) => {
            if !info.hThread.is_invalid() {
                let _ = unsafe { CloseHandle(info.hThread) };
            }
            Some(Child { handle: info.hProcess })
        }
        Err(e) => {
            tracing::warn!("proses '{command}' gagal dijalankan: {}", e.message());
            None
        }
    }
}

fn agent_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(AGENT_EXE)))
        .unwrap_or_else(|| PathBuf::from(AGENT_EXE))
}

fn worker_call(request: &WorkerRequest) -> ControlReply {
    let bytes = serde_json::to_vec(request).unwrap_or_default();
    match crate::pipe::call(WORKER_PIPE, &bytes) {
        Ok(reply) => match serde_json::from_slice::<WorkerReply>(&reply) {
            Ok(WorkerReply::Ok) | Ok(WorkerReply::Desktop(_)) => ControlReply::Ok,
            Ok(WorkerReply::Error(e)) => ControlReply::Error(e),
            Err(e) => ControlReply::Error(format!("balasan worker tidak dimengerti: {e}")),
        },
        Err(ServiceError::NotRunning) => {
            ControlReply::Error("worker injeksi belum siap".into())
        }
        Err(e) => ControlReply::Error(e.to_string()),
    }
}

fn control_handler(state: &Mutex<ServiceState>, request: &[u8]) -> Vec<u8> {
    let reply = match serde_json::from_slice::<ControlRequest>(request) {
        Ok(ControlRequest::Status) => {
            let snapshot = state.lock().map(|s| s.clone()).unwrap_or(ServiceState {
                worker_desktop: None,
                secure_desktop: false,
            });
            ControlReply::Status(snapshot)
        }
        Ok(ControlRequest::Inject(events)) => worker_call(&WorkerRequest::Inject(events)),
        Ok(ControlRequest::SendSas) => worker_call(&WorkerRequest::SendSas),
        Ok(ControlRequest::CheckUpdate) => match self_update() {
            Ok(restarting) => ControlReply::Updating(restarting),
            Err(e) => ControlReply::Error(e),
        },
        Ok(ControlRequest::SyncLogon(payload)) => match crate::logon::store(&payload) {
            Ok(()) => ControlReply::Ok,
            Err(e) => ControlReply::Error(e),
        },
        Ok(ControlRequest::ClearLogon) => match crate::logon::clear() {
            Ok(()) => ControlReply::Ok,
            Err(e) => ControlReply::Error(e),
        },
        Err(e) => ControlReply::Error(format!("permintaan tidak dimengerti: {e}")),
    };
    serde_json::to_vec(&reply).unwrap_or_default()
}

fn self_update() -> Result<bool, String> {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) {
        return Ok(true);
    }
    let result = (|| {
        let Some(source) = crate::install::source_path() else {
            return Err("sumber pembaruan layanan belum dicatat; pasang ulang layanan sekali".to_string());
        };
        let source_build = build::marker_in_file(&source)
            .map_err(|e| format!("{} tidak bisa dibaca: {e}", source.display()))?
            .ok_or_else(|| format!("{} bukan exe Tautra", source.display()))?;
        if source_build == build::id() {
            return Ok(false);
        }
        let dir = crate::install::service_dir();
        let fresh = dir.join("tautra-agent.new.exe");
        std::fs::copy(&source, &fresh).map_err(|e| format!("salin exe baru gagal: {e}"))?;
        let copied = build::marker_in_file(&fresh)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "salinan exe baru tidak valid".to_string())?;
        if copied != source_build {
            let _ = std::fs::remove_file(&fresh);
            return Err("salinan exe baru berbeda dari sumbernya".into());
        }
        build::swap_in(&dir, AGENT_EXE, &fresh).map_err(|e| format!("exe layanan gagal diganti: {e}"))?;
        tracing::info!(from = build::id(), to = source_build, "exe layanan diganti, menjalankan ulang layanan");
        crate::install::spawn_restart_helper().map_err(|e| e.to_string())?;
        STOP.store(true, Ordering::SeqCst);
        crate::pipe::poke(CONTROL_PIPE);
        Ok(true)
    })();
    if !matches!(result, Ok(true)) {
        BUSY.store(false, Ordering::SeqCst);
    }
    result
}

fn ping_worker() -> Option<String> {
    let bytes = serde_json::to_vec(&WorkerRequest::Ping).ok()?;
    let reply = crate::pipe::call(WORKER_PIPE, &bytes).ok()?;
    match serde_json::from_slice::<WorkerReply>(&reply).ok()? {
        WorkerReply::Desktop(name) => Some(name),
        _ => None,
    }
}

fn supervise(state: Arc<Mutex<ServiceState>>) {
    let mut worker: Option<Child> = None;
    let mut worker_session: Option<u32> = None;
    let mut worker_desktop: Option<String> = None;
    let mut last_ping = Instant::now() - PING_INTERVAL;
    let worker_command = format!("\"{}\" {} \"{}\"", agent_exe().display(), WORKER_ARG, WORKER_PIPE);
    let logon_command = format!("\"{}\" {}", agent_exe().display(), LOGON_ARG);
    let mut logon: Option<Child> = None;
    let mut logon_session: Option<u32> = None;
    let mut logon_started = Instant::now() - LOGON_BACKOFF;

    while !STOP.load(Ordering::SeqCst) {
        match active_session() {
            Some(session) => {
                let dead = worker.as_ref().map(|w| !w.alive()).unwrap_or(true);
                if dead || worker_session != Some(session) {
                    if let Some(old) = worker.take() {
                        old.terminate();
                    }
                    worker = service_token(session)
                        .and_then(|token| spawn(&token, &worker_command, Some("Default")));
                    worker_session = worker.as_ref().map(|_| session);
                    worker_desktop = None;
                    if worker.is_some() {
                        tracing::info!(session, "worker injeksi dijalankan");
                    }
                }
                if last_ping.elapsed() >= PING_INTERVAL {
                    last_ping = Instant::now();
                    if worker.as_ref().is_some_and(|w| w.alive()) {
                        if let Some(name) = ping_worker() {
                            worker_desktop = Some(name);
                        }
                    }
                }
            }
            None => {
                if let Some(old) = worker.take() {
                    old.terminate();
                }
                worker_session = None;
                worker_desktop = None;
            }
        }

        let wanted = active_session().filter(|s| !user_signed_in(*s) && crate::logon::ready());
        match wanted {
            Some(session) => {
                let alive = logon.as_ref().is_some_and(|c| c.alive());
                if (!alive || logon_session != Some(session)) && logon_started.elapsed() >= LOGON_BACKOFF {
                    if let Some(old) = logon.take() {
                        old.terminate();
                    }
                    logon_started = Instant::now();
                    logon = service_token(session)
                        .and_then(|token| spawn(&token, &logon_command, Some("Winlogon")));
                    logon_session = logon.as_ref().map(|_| session);
                    if logon.is_some() {
                        tracing::info!(session, "agent layar login dijalankan");
                    }
                }
            }
            None => {
                if let Some(old) = logon.take() {
                    old.terminate();
                    tracing::info!("agent layar login dihentikan");
                }
                logon_session = None;
            }
        }

        if let Ok(mut guard) = state.lock() {
            guard.worker_desktop = worker_desktop.clone();
            guard.secure_desktop = worker_desktop
                .as_deref()
                .map(|d| !d.eq_ignore_ascii_case("Default"))
                .unwrap_or(false);
        }

        for _ in 0..10 {
            if STOP.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    if let Some(old) = worker.take() {
        old.terminate();
    }
    if let Some(old) = logon.take() {
        old.terminate();
    }
}

fn report(handle: SERVICE_STATUS_HANDLE, state: SERVICE_STATUS_CURRENT_STATE, accepted: u32) {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accepted,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 3000,
    };
    let _ = unsafe { SetServiceStatus(handle, &status) };
}

unsafe extern "system" fn control_callback(
    control: u32,
    _event_type: u32,
    _event_data: *mut c_void,
    _context: *mut c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            STOP.store(true, Ordering::SeqCst);
            let handle = SERVICE_STATUS_HANDLE(STATUS_HANDLE.load(Ordering::SeqCst) as *mut c_void);
            report(handle, SERVICE_STOP_PENDING, 0);
            crate::pipe::poke(CONTROL_PIPE);
            NO_ERROR.0
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR.0,
        _ => ERROR_CALL_NOT_IMPLEMENTED.0,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let name = wide(crate::protocol::SERVICE_NAME);
    let handle = match unsafe {
        RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(control_callback), None)
    } {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!("handler layanan gagal didaftarkan: {}", e.message());
            return;
        }
    };
    STATUS_HANDLE.store(handle.0 as usize, Ordering::SeqCst);
    report(handle, SERVICE_RUNNING, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN);
    tracing::info!("{SERVICE_DISPLAY_NAME} berjalan");
    std::thread::spawn(|| {
        crate::install::harden_self();
        crate::install::refresh_firewall_rule();
        std::thread::sleep(Duration::from_secs(2));
        match self_update() {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => tracing::debug!("pembaruan layanan saat mulai dilewati: {e}"),
        }
        let dir = crate::install::service_dir();
        for _ in 0..90 {
            if build::clean_old(&dir) {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    let state = Arc::new(Mutex::new(ServiceState {
        worker_desktop: None,
        secure_desktop: false,
    }));
    let control_state = state.clone();
    let control = std::thread::spawn(move || {
        let _ = crate::pipe::serve(CONTROL_PIPE, CONTROL_SDDL, &STOP, |request| {
            control_handler(&control_state, request)
        });
    });
    supervise(state);
    let _ = control.join();

    tracing::info!("{SERVICE_DISPLAY_NAME} berhenti");
    report(handle, SERVICE_STOPPED, 0);
}

pub fn run_service() -> Result<(), ServiceError> {
    let mut name = wide(crate::protocol::SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }
        .map_err(|e| ServiceError::Scm(format!("dispatcher layanan gagal: {}", e.message())))
}

pub fn run_worker(pipe: &str) {
    crate::worker::run(pipe);
}

