use windows::core::HRESULT;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub const AGENT_ENTRY: &str = "TautraAgent";
pub const UI_ENTRY: &str = "Tautra";

#[derive(Debug, thiserror::Error)]
#[error("gagal mengubah auto-start '{entry}': {source}")]
pub struct AutostartError {
    entry: String,
    #[source]
    source: windows::core::Error,
}

pub fn set_entry(entry: &str, command: Option<&str>) -> Result<(), AutostartError> {
    let result = (|| {
        let key = windows_registry::CURRENT_USER.create(RUN_KEY)?;
        match command {
            Some(cmd) => key.set_string(entry, cmd),
            None => match key.remove_value(entry) {
                Err(e) if e.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => Ok(()),
                other => other,
            },
        }
    })();
    result.map_err(|source| AutostartError {
        entry: entry.to_string(),
        source,
    })
}

pub fn get_entry(entry: &str) -> Option<String> {
    windows_registry::CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(entry))
        .ok()
}

pub fn command_for(exe: &std::path::Path, args: &[&str]) -> String {
    let mut cmd = format!("\"{}\"", exe.display());
    for arg in args {
        cmd.push(' ');
        cmd.push_str(arg);
    }
    cmd
}
