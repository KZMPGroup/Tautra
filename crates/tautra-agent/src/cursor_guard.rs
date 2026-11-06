use std::os::windows::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};

use tautra_input::cursor;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    OpenEventW, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    SYNCHRONIZATION_SYNCHRONIZE,
};

pub const ARG: &str = "--cursor-guard";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

pub fn requested() -> bool {
    std::env::args().any(|a| a == ARG)
}

fn watched_pid() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    let index = args.iter().position(|a| a == ARG)?;
    args.get(index + 1)?.parse().ok()
}

pub fn run() -> ExitCode {
    let Some(pid) = watched_pid() else {
        return ExitCode::FAILURE;
    };
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.ok();
    let marker = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &cursor::marker_name(pid)) }.ok();
    if let Some(process) = process {
        unsafe {
            WaitForSingleObject(process, INFINITE);
            let _ = CloseHandle(process);
        }
    }
    let hidden = match marker {
        Some(marker) => {
            let signaled = unsafe { WaitForSingleObject(marker, 0) } == WAIT_OBJECT_0;
            unsafe {
                let _ = CloseHandle(marker);
            }
            signaled
        }
        None => true,
    };
    if !hidden {
        return ExitCode::SUCCESS;
    }
    let _log = tautra_common::logging::init("cursor", false).ok();
    match cursor::restore() {
        Ok(()) => {
            tracing::warn!(pid, "agent berhenti saat kursor tersembunyi; kursor dimunculkan kembali");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(pid, "kursor gagal dimunculkan kembali setelah agent berhenti: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn spawn(logon: bool) {
    if let Err(e) = cursor::create_marker() {
        tracing::warn!("penanda kursor gagal dibuat; penjaga kursor tidak dijalankan: {e}");
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            tracing::warn!("lokasi exe agent tidak diketahui; penjaga kursor tidak dijalankan: {e}");
            return;
        }
    };
    let mut command = Command::new(&exe);
    command
        .arg(ARG)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if logon {
        command.arg(tautra_service::LOGON_ARG);
    }
    let spawned = command
        .creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .or_else(|_| command.creation_flags(CREATE_NO_WINDOW).spawn());
    match spawned {
        Ok(child) => tracing::info!(pid = child.id(), "penjaga kursor berjalan"),
        Err(e) => tracing::warn!("penjaga kursor gagal dijalankan: {e}"),
    }
}
