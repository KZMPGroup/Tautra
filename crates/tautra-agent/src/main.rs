#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod compat;
#[cfg(windows)]
mod kvm;
#[cfg(windows)]
mod metrics;
#[cfg(windows)]
mod notices;
#[cfg(windows)]
mod rpc;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod share;
#[cfg(windows)]
mod single_instance;
#[cfg(windows)]
mod streams;
#[cfg(windows)]
mod update;

use std::process::ExitCode;

tautra_common::declare_build!();

#[cfg(not(windows))]
fn main() -> ExitCode {
    let _ = tautra_build_id();
    eprintln!("Tautra hanya berjalan di Windows.");
    ExitCode::FAILURE
}

#[cfg(windows)]
fn main() -> ExitCode {
    tautra_common::build::set(tautra_common::build::release_id(tautra_build_id()));
    let console = cfg!(debug_assertions) || std::env::args().any(|a| a == "--console");
    let _log = match tautra_common::logging::init("agent", console) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Tautra: logging gagal dipasang: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = tautra_input::display::set_dpi_awareness() {
        tracing::warn!("mode DPI per-monitor gagal diaktifkan: {e}");
    }

    if let Some(code) = service::maybe_handle() {
        return code;
    }

    update::wait_for_previous();

    let _instance = match single_instance::InstanceGuard::acquire() {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            tracing::info!("agent lain sudah berjalan di sesi ini; keluar");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            tracing::error!("gagal memeriksa instance agent: {e}");
            return ExitCode::FAILURE;
        }
    };

    raise_priority();
    sync_installed_version();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("tautra-rt")
        .on_thread_start(raise_thread_priority)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("runtime async gagal dibuat: {e}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(app::run()) {
        Ok(()) => {
            tracing::info!("agent berhenti dengan normal");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("agent berhenti karena error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn raise_priority() {
    use windows::Win32::System::Threading::{GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS};
    if let Err(e) = unsafe { SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) } {
        tracing::warn!("prioritas proses agent gagal dinaikkan: {e}");
    }
}

#[cfg(windows)]
fn raise_thread_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
}

fn sync_installed_version() {
    let Ok(dir) = tautra_common::build::exe_dir() else {
        return;
    };
    if !tautra_common::installed::is_installed(&dir) {
        return;
    }
    match tautra_common::installed::sync_version(&dir, tautra_common::APP_VERSION) {
        Ok(true) => tracing::info!(version = tautra_common::APP_VERSION, "versi di daftar aplikasi Windows diperbarui"),
        Ok(false) => {}
        Err(e) => tracing::warn!("versi di daftar aplikasi Windows gagal diperbarui: {e}"),
    }
}
