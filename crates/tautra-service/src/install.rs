use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST,
    ERROR_SERVICE_MARKED_FOR_DELETE, ERROR_SERVICE_NOT_ACTIVE, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
use windows::Win32::System::Services::{
    ChangeServiceConfig2W, CloseServiceHandle, ControlService, CreateServiceW, DeleteService,
    OpenSCManagerW, OpenServiceW, QueryServiceStatus, SetServiceObjectSecurity, StartServiceW,
    SC_ACTION, SC_ACTION_RESTART, SC_HANDLE, SC_MANAGER_ALL_ACCESS, SC_MANAGER_CONNECT,
    SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_CHANGE_CONFIG, SERVICE_CONFIG_DESCRIPTION,
    SERVICE_CONFIG_FAILURE_ACTIONS, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, SERVICE_CONTROL_STOP,
    SERVICE_DESCRIPTIONW, SERVICE_ERROR_NORMAL, SERVICE_FAILURE_ACTIONSW,
    SERVICE_FAILURE_ACTIONS_FLAG, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START,
    SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE, SERVICE_STOPPED, SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Storage::FileSystem::WRITE_DAC;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::protocol::{
    RESTART_ARG, SERVICE_ARG, SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME, SOURCE_FILE,
};
use crate::{wide, ServiceError};

const ELEVATION_TIMEOUT_MS: u32 = 2 * 60 * 1000;
const ERROR_CANCELLED: i32 = 1223;
const SERVICE_EXE: &str = "tautra-agent.exe";
const SAS_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System";
const FIREWALL_RULE: &str = "Tautra Agent";
const LOGON_FIREWALL_RULE: &str = "Tautra Login";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SERVICE_SDDL: &str = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPLOCRRC;;;AU)";

pub fn service_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("Tautra").join("service")
}

pub fn service_exe_path() -> PathBuf {
    service_dir().join(SERVICE_EXE)
}

pub fn source_path() -> Option<PathBuf> {
    let text = std::fs::read_to_string(service_dir().join(SOURCE_FILE)).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn remember_source() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::write(
            service_dir().join(SOURCE_FILE),
            exe.to_string_lossy().as_bytes(),
        );
    }
}

pub fn log_dir() -> PathBuf {
    service_dir()
        .parent()
        .map(|p| p.join("logs"))
        .unwrap_or_else(|| service_dir().join("logs"))
}

pub fn run_elevated(args: &[String]) -> Result<(), ServiceError> {
    let exe = std::env::current_exe()
        .map_err(|e| ServiceError::Other(format!("lokasi agent tidak diketahui: {e}")))?;
    let file = wide(&exe.to_string_lossy());
    let joined = args
        .iter()
        .map(|a| format!("\"{}\"", a.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ");
    let params = wide(&joined);
    let verb = wide("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        if e.code().0 & 0xFFFF == ERROR_CANCELLED {
            ServiceError::Other("izin administrator dibatalkan".into())
        } else {
            ServiceError::Other(format!("proses administrator gagal dimulai: {}", e.message()))
        }
    })?;
    if info.hProcess.is_invalid() {
        return Err(ServiceError::Other("proses administrator tidak terbentuk".into()));
    }
    let waited = unsafe { WaitForSingleObject(info.hProcess, ELEVATION_TIMEOUT_MS) };
    let mut code = 1u32;
    if waited == WAIT_OBJECT_0 {
        let _ = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    }
    let _ = unsafe { CloseHandle(info.hProcess) };
    if waited != WAIT_OBJECT_0 {
        return Err(ServiceError::Other("proses pemasangan layanan tidak selesai".into()));
    }
    match code {
        0 => Ok(()),
        other => Err(ServiceError::Other(format!(
            "pemasangan layanan gagal (kode {other}); lihat log agent"
        ))),
    }
}

struct ScHandle(SC_HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}

fn open_manager() -> Result<ScHandle, ServiceError> {
    open_manager_with(SC_MANAGER_ALL_ACCESS)
}

fn open_manager_with(access: u32) -> Result<ScHandle, ServiceError> {
    unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
        .map(ScHandle)
        .map_err(|e| ServiceError::Scm(format!("manajer layanan gagal dibuka: {}", e.message())))
}

fn open_service(manager: &ScHandle) -> Result<Option<ScHandle>, ServiceError> {
    open_service_with(manager, SERVICE_ALL_ACCESS)
}

fn open_service_with(manager: &ScHandle, access: u32) -> Result<Option<ScHandle>, ServiceError> {
    let name = wide(SERVICE_NAME);
    match unsafe { OpenServiceW(manager.0, PCWSTR(name.as_ptr()), access) } {
        Ok(handle) => Ok(Some(ScHandle(handle))),
        Err(e) if e.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() => Ok(None),
        Err(e) => Err(ServiceError::Scm(format!("layanan gagal dibuka: {}", e.message()))),
    }
}

fn service_state(service: &ScHandle) -> Option<SERVICE_STATUS_CURRENT_STATE> {
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service.0, &mut status) }
        .ok()
        .map(|_| status.dwCurrentState)
}

fn stop_service(service: &ScHandle) -> Result<(), ServiceError> {
    let mut status = SERVICE_STATUS::default();
    match unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) } {
        Ok(()) => {}
        Err(e) if e.code() == ERROR_SERVICE_NOT_ACTIVE.to_hresult() => return Ok(()),
        Err(e) => return Err(ServiceError::Scm(format!("layanan gagal dihentikan: {}", e.message()))),
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if service_state(service) == Some(SERVICE_STOPPED) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

fn copy_exe(dest: &Path) -> Result<(), ServiceError> {
    let source = std::env::current_exe()
        .map_err(|e| ServiceError::Other(format!("lokasi agent tidak diketahui: {e}")))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ServiceError::Other(format!("folder layanan gagal dibuat: {e}")))?;
    }
    let mut attempt = 0;
    loop {
        match std::fs::copy(&source, dest) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < 10 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(400));
                let _ = e;
            }
            Err(e) => {
                return Err(ServiceError::Other(format!(
                    "exe layanan gagal disalin ke {}: {e}",
                    dest.display()
                )))
            }
        }
    }
}

fn create_service(manager: &ScHandle, exe: &Path) -> Result<ScHandle, ServiceError> {
    let name = wide(SERVICE_NAME);
    let display = wide(SERVICE_DISPLAY_NAME);
    let path = wide(&format!("\"{}\" {}", exe.display(), SERVICE_ARG));
    let handle = unsafe {
        CreateServiceW(
            manager.0,
            PCWSTR(name.as_ptr()),
            PCWSTR(display.as_ptr()),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
    .map_err(|e| ServiceError::Scm(format!("layanan gagal didaftarkan: {}", e.message())))?;
    let service = ScHandle(handle);
    harden(&service);
    Ok(service)
}

fn harden(service: &ScHandle) {
    let mut description = wide(SERVICE_DESCRIPTION);
    let info = SERVICE_DESCRIPTIONW {
        lpDescription: PWSTR(description.as_mut_ptr()),
    };
    let _ = unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DESCRIPTION,
            Some(&info as *const _ as *const c_void),
        )
    };
    let mut actions = [
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 1_000 },
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 5_000 },
        SC_ACTION { Type: SC_ACTION_RESTART, Delay: 30_000 },
    ];
    let failure = SERVICE_FAILURE_ACTIONSW {
        dwResetPeriod: 86_400,
        lpRebootMsg: PWSTR::null(),
        lpCommand: PWSTR::null(),
        cActions: actions.len() as u32,
        lpsaActions: actions.as_mut_ptr(),
    };
    if let Err(e) = unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_FAILURE_ACTIONS,
            Some(&failure as *const _ as *const c_void),
        )
    } {
        tracing::warn!("pemulihan otomatis layanan gagal diatur: {}", e.message());
    }
    let flag = SERVICE_FAILURE_ACTIONS_FLAG {
        fFailureActionsOnNonCrashFailures: windows::core::BOOL(1),
    };
    let _ = unsafe {
        ChangeServiceConfig2W(
            service.0,
            SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
            Some(&flag as *const _ as *const c_void),
        )
    };
    let sddl = wide(SERVICE_SDDL);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .is_ok()
    {
        if let Err(e) = unsafe { SetServiceObjectSecurity(service.0, DACL_SECURITY_INFORMATION, descriptor) } {
            tracing::warn!("hak akses layanan gagal diatur: {}", e.message());
        }
        let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    }
}

pub fn harden_self() {
    let Ok(manager) = open_manager_with(SC_MANAGER_CONNECT) else {
        return;
    };
    if let Ok(Some(service)) = open_service_with(&manager, SERVICE_CHANGE_CONFIG | WRITE_DAC.0) {
        harden(&service);
    }
}

pub fn ensure_running() -> bool {
    let Ok(manager) = open_manager_with(SC_MANAGER_CONNECT) else {
        return false;
    };
    let Ok(Some(service)) = open_service_with(&manager, SERVICE_START | SERVICE_QUERY_STATUS) else {
        return false;
    };
    if service_state(&service) == Some(SERVICE_RUNNING) {
        return true;
    }
    match start_service(&service) {
        Ok(()) => {
            tracing::info!("layanan Tautra dijalankan kembali");
            true
        }
        Err(e) => {
            tracing::debug!("layanan Tautra belum bisa dijalankan: {e}");
            false
        }
    }
}

fn start_service(service: &ScHandle) -> Result<(), ServiceError> {
    match unsafe { StartServiceW(service.0, None) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERROR_SERVICE_ALREADY_RUNNING.to_hresult() => Ok(()),
        Err(e) => Err(ServiceError::Scm(format!("layanan gagal dimulai: {}", e.message()))),
    }
}

fn enable_software_sas() {
    if let Ok(key) = windows_registry::LOCAL_MACHINE.create(SAS_KEY) {
        let _ = key.set_u32("SoftwareSASGeneration", 1);
    }
}

fn netsh(args: &[&str]) {
    let _ = Command::new("netsh")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

fn delete_rule(name: &str) {
    netsh(&[
        "advfirewall",
        "firewall",
        "delete",
        "rule",
        &format!("name={name}"),
    ]);
}

fn remove_firewall_rule() {
    delete_rule(FIREWALL_RULE);
    delete_rule(LOGON_FIREWALL_RULE);
}

fn add_rule(name: &str, exe: &Path) {
    delete_rule(name);
    netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={name}"),
        "dir=in",
        "action=allow",
        &format!("program={}", exe.display()),
        "enable=yes",
        "profile=any",
    ]);
}

fn add_firewall_rule() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    firewall_rule_for(&exe);
}

pub fn refresh_firewall_rule() {
    if let Some(source) = source_path() {
        firewall_rule_for(&source);
    }
}

fn firewall_rule_for(exe: &Path) {
    add_rule(FIREWALL_RULE, exe);
    add_rule(LOGON_FIREWALL_RULE, &service_exe_path());
}

pub fn install() -> Result<(), ServiceError> {
    let manager = open_manager()?;
    if let Some(existing) = open_service(&manager)? {
        stop_service(&existing)?;
    }
    let dest = service_exe_path();
    copy_exe(&dest)?;
    remember_source();
    let service = match open_service(&manager)? {
        Some(service) => {
            harden(&service);
            service
        }
        None => create_service(&manager, &dest)?,
    };
    enable_software_sas();
    add_firewall_rule();
    start_service(&service)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if service_state(&service) == Some(SERVICE_RUNNING) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(ServiceError::Scm("layanan Tautra tidak kunjung berjalan".into()))
}

pub fn uninstall() -> Result<(), ServiceError> {
    let manager = open_manager()?;
    remove_firewall_rule();
    let Some(service) = open_service(&manager)? else {
        let _ = std::fs::remove_dir_all(service_dir());
        let _ = crate::logon::clear();
        return Ok(());
    };
    stop_service(&service)?;
    match unsafe { DeleteService(service.0) } {
        Ok(()) => {}
        Err(e) if e.code() == ERROR_SERVICE_MARKED_FOR_DELETE.to_hresult() => {}
        Err(e) => return Err(ServiceError::Scm(format!("layanan gagal dihapus: {}", e.message()))),
    }
    drop(service);
    let _ = std::fs::remove_dir_all(service_dir());
    let _ = crate::logon::clear();
    Ok(())
}

pub fn spawn_restart_helper() -> Result<(), ServiceError> {
    Command::new(service_exe_path())
        .arg(RESTART_ARG)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| ServiceError::Other(format!("pembantu restart layanan gagal dijalankan: {e}")))
}

pub fn restart_after_stop() -> Result<(), ServiceError> {
    let manager = open_manager()?;
    let Some(service) = open_service(&manager)? else {
        return Err(ServiceError::Scm("layanan Tautra tidak terdaftar".into()));
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if service_state(&service) == Some(SERVICE_STOPPED) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    start_service(&service)?;
    tracing::info!("layanan Tautra dijalankan ulang setelah pembaruan");
    Ok(())
}

pub fn query() -> (bool, bool) {
    let Ok(manager) = open_manager_with(SC_MANAGER_CONNECT) else {
        return (false, false);
    };
    match open_service_with(&manager, SERVICE_QUERY_STATUS) {
        Ok(Some(service)) => (true, service_state(&service) == Some(SERVICE_RUNNING)),
        _ => (false, false),
    }
}

pub fn is_installed() -> bool {
    query().0
}
