use tautra_common::autostart;

pub fn sync(enabled: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    let command = match std::env::current_exe() {
        Ok(exe) => enabled.then(|| autostart::command_for(&exe, &["--tray"])),
        Err(e) => {
            tracing::warn!("lokasi UI tidak diketahui: {e}");
            return;
        }
    };
    if let Err(e) = autostart::set_entry(autostart::UI_ENTRY, command.as_deref()) {
        tracing::warn!("auto-start UI gagal diatur: {e}");
    }
}
