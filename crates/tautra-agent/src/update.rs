use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use tautra_common::build;
use tautra_ipc::api::{app_error, notification, NoticeLevel, UpdateAppliedParams};
use tautra_ipc::pipe::notification_text;

use crate::app::Agent;

pub const WAIT_PID_ARG: &str = "--wait-pid";
const WAIT_PREVIOUS: Duration = Duration::from_secs(20);
const EXIT_DELAY: Duration = Duration::from_millis(700);
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn wait_for_previous() {
    let args: Vec<String> = std::env::args().collect();
    let Some(index) = args.iter().position(|a| a == WAIT_PID_ARG) else {
        return;
    };
    let Some(pid) = args.get(index + 1).and_then(|p| p.parse::<u32>().ok()) else {
        return;
    };
    tracing::info!(pid, "menunggu agent lama berhenti");
    build::wait_for_pid(pid, WAIT_PREVIOUS);
}

pub fn apply(agent: &Agent, device_name: &str, build_id: u64, dir: PathBuf, files: Vec<PathBuf>) {
    let result = (|| -> Result<bool, String> {
        let exe_dir = build::exe_dir().map_err(|e| e.to_string())?;
        let mut ui = false;
        for file in &files {
            let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !build::UPDATE_FILES.contains(&name) {
                continue;
            }
            build::swap_in(&exe_dir, name, file)
                .map_err(|e| format!("{name} gagal diganti: {e}"))?;
            tracing::info!(name, "file diganti dengan versi baru");
            if name == build::UI_EXE {
                ui = true;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(ui)
    })();
    let ui = match result {
        Ok(ui) => ui,
        Err(e) => {
            tracing::error!("pembaruan gagal diterapkan: {e}");
            agent.notice(
                NoticeLevel::Error,
                app_error::UPDATE_FAILED,
                format!("Pembaruan dari {device_name} gagal diterapkan: {e}"),
            );
            return;
        }
    };
    if let Some(text) = notification_text(
        notification::UPDATE_APPLIED,
        &UpdateAppliedParams { build: build_id, ui },
    ) {
        let _ = agent.notify_tx.send(text);
    }
    let exe = match build::exe_dir() {
        Ok(dir) => dir.join(build::AGENT_EXE),
        Err(e) => {
            tracing::error!("folder exe tidak diketahui: {e}");
            return;
        }
    };
    let spawned = std::process::Command::new(&exe)
        .arg(WAIT_PID_ARG)
        .arg(std::process::id().to_string())
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn();
    match spawned {
        Ok(_) => {
            tracing::info!(build = build_id, "agent versi baru dijalankan; agent ini berhenti");
            let cancel = agent.cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(EXIT_DELAY).await;
                cancel.cancel();
            });
        }
        Err(e) => {
            tracing::error!("agent baru gagal dijalankan: {e}");
            agent.notice(
                NoticeLevel::Error,
                app_error::UPDATE_FAILED,
                format!("Agent versi baru gagal dijalankan: {e}. Jalankan ulang Tautra secara manual."),
            );
        }
    }
}
