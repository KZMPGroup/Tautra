#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_link;
mod autostart;
mod commands;
mod i18n;
mod tray;
mod window;

use tauri::{Emitter, Manager, WindowEvent};

use crate::agent_link::AgentLink;
use crate::commands::UiState;

tautra_common::declare_build!();

fn main() {
    tautra_common::build::set(tautra_build_id());
    let start_hidden = std::env::args().any(|a| a == "--tray");
    let _log = match tautra_common::logging::init("ui", cfg!(debug_assertions)) {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!("Tautra UI: logging gagal dipasang: {e}");
            None
        }
    };

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::agent_call,
            commands::agent_snapshot,
            commands::start_agent,
            commands::ui_info,
            commands::set_tray_strings,
            commands::ui_ready,
            commands::open_folder,
            commands::quit_app,
        ])
        .setup(move |app| {
            let mica = window::apply_backdrop(app.handle());
            app.manage(UiState::new(mica, start_hidden));
            tray::create(app.handle())?;
            app.manage(AgentLink::start(app.handle().clone()));
            commands::schedule_show_fallback(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == window::MAIN {
                    api.prevent_close();
                    let _ = window.hide();
                    let _ = window.emit(window::HIDDEN_EVENT, ());
                }
            }
        })
        .run(tauri::generate_context!());

    if let Err(e) = result {
        tracing::error!("UI berhenti karena error: {e}");
    }
}
