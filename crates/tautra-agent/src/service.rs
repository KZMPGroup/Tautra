use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tautra_input::inject::InjectFallback;
use tautra_proto::kvm::ServiceInput;
use tautra_service::{INSTALL_ARG, RESTART_ARG, SERVICE_ARG, UNINSTALL_ARG, WORKER_ARG};

pub fn maybe_handle() -> Option<ExitCode> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == SERVICE_ARG) {
        let code = match tautra_service::service::run_service() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("layanan Tautra gagal: {e}");
                ExitCode::FAILURE
            }
        };
        return Some(code);
    }
    if let Some(index) = args.iter().position(|a| a == WORKER_ARG) {
        let pipe = args.get(index + 1).cloned().unwrap_or_default();
        tautra_service::service::run_worker(&pipe);
        return Some(ExitCode::SUCCESS);
    }
    if args.iter().any(|a| a == INSTALL_ARG) {
        let code = match tautra_service::install::install() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("pemasangan layanan gagal: {e}");
                ExitCode::FAILURE
            }
        };
        return Some(code);
    }
    if args.iter().any(|a| a == RESTART_ARG) {
        let code = match tautra_service::install::restart_after_stop() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("layanan gagal dijalankan ulang: {e}");
                ExitCode::FAILURE
            }
        };
        return Some(code);
    }
    if args.iter().any(|a| a == UNINSTALL_ARG) {
        let code = match tautra_service::install::uninstall() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!("pencopotan layanan gagal: {e}");
                ExitCode::FAILURE
            }
        };
        return Some(code);
    }
    None
}

#[derive(Default)]
pub struct ServiceFallback;

impl InjectFallback for ServiceFallback {
    fn forward(&self, events: Vec<ServiceInput>) -> bool {
        match tautra_service::client::forward(events) {
            Ok(()) => true,
            Err(e) => {
                static LAST: Mutex<Option<Instant>> = Mutex::new(None);
                if let Ok(mut last) = LAST.lock() {
                    if last.is_none_or(|at| at.elapsed() >= Duration::from_secs(5)) {
                        *last = Some(Instant::now());
                        tracing::warn!("input ke jendela administrator gagal diteruskan ke layanan: {e}");
                    }
                }
                false
            }
        }
    }
}

pub fn fallback() -> Arc<dyn InjectFallback> {
    Arc::new(ServiceFallback)
}

pub fn install() -> Result<(), String> {
    tautra_service::install::run_elevated(&[INSTALL_ARG.to_string()]).map_err(|e| e.to_string())
}

pub fn uninstall() -> Result<(), String> {
    tautra_service::install::run_elevated(&[UNINSTALL_ARG.to_string()]).map_err(|e| e.to_string())
}

pub fn check_update() {
    if !tautra_service::install::is_installed() {
        return;
    }
    match tautra_service::client::check_update() {
        Ok(true) => tracing::info!("layanan Tautra memperbarui diri ke build ini"),
        Ok(false) => tracing::debug!("layanan Tautra sudah build yang sama"),
        Err(e) => tracing::warn!("pemeriksaan pembaruan layanan gagal: {e}"),
    }
}

pub fn status() -> tautra_ipc::api::ServiceStatus {
    let (installed, running) = tautra_service::install::query();
    tautra_ipc::api::ServiceStatus { installed, running }
}
