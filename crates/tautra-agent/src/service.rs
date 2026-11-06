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

pub fn set_assist(on: bool) -> bool {
    static LAST_OK: Mutex<Option<bool>> = Mutex::new(None);
    let result = tautra_service::client::set_assist(on);
    let ok = result.is_ok();
    if let Ok(mut last) = LAST_OK.lock() {
        if *last != Some(ok) {
            *last = Some(ok);
            match &result {
                Ok(()) => tracing::info!(aktif = on, "hook bantuan layanan untuk jendela admin diatur"),
                Err(e) => tracing::warn!(
                    "hook bantuan layanan tidak bisa diatur, kursor tidak akan menyeberang saat jendela admin fokus: {e}"
                ),
            }
        }
    }
    ok
}

pub fn serve_input_assist() {
    use tautra_service::protocol::{AssistInput, AssistReply, ASSIST_SDDL, INPUT_ASSIST_PIPE};
    std::thread::Builder::new()
        .name("tautra-assist-pipe".into())
        .spawn(|| {
            static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            let _ = tautra_service::pipe::serve(INPUT_ASSIST_PIPE, ASSIST_SDDL, &STOP, |request| {
                let reply = match serde_json::from_slice::<AssistInput>(request) {
                    Ok(AssistInput::Mouse { msg, x, y, mouse_data }) => {
                        let o = tautra_input::feed_external_mouse(msg, x, y, mouse_data);
                        AssistReply { swallow: o.swallow, remote: o.remote, park: o.park, place: o.place }
                    }
                    Ok(AssistInput::Key { msg, vk, scan, extended }) => {
                        let o = tautra_input::feed_external_key(msg, vk, scan, extended);
                        AssistReply { swallow: o.swallow, remote: o.remote, park: o.park, place: o.place }
                    }
                    Err(e) => {
                        tracing::warn!("permintaan hook bantuan dari layanan tidak dimengerti: {e}");
                        AssistReply::default()
                    }
                };
                serde_json::to_vec(&reply).unwrap_or_default()
            });
        })
        .ok();
}
