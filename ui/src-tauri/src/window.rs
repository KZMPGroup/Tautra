use tauri::{AppHandle, Emitter, Manager};

pub const MAIN: &str = "main";
pub const HIDDEN_EVENT: &str = "window://hidden";
pub const SHOWN_EVENT: &str = "window://shown";

pub fn apply_backdrop(app: &AppHandle) -> bool {
    let Some(window) = app.get_webview_window(MAIN) else {
        return false;
    };
    match window_vibrancy::apply_mica(&window, None) {
        Ok(()) => true,
        Err(e) => {
            tracing::info!("Mica tidak tersedia, memakai latar solid: {e}");
            false
        }
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(SHOWN_EVENT, ());
    }
}
