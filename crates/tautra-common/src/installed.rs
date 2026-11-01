use std::path::{Path, PathBuf};

const UNINSTALL_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

fn normalize(path: &str) -> PathBuf {
    let trimmed = path.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    PathBuf::from(trimmed.to_lowercase())
}

fn location(key: &windows_registry::Key) -> Option<PathBuf> {
    if let Ok(dir) = key.get_string("InstallLocation") {
        if !dir.trim().is_empty() {
            return Some(normalize(&dir));
        }
    }
    let uninstall = key.get_string("UninstallString").ok()?;
    let exe = uninstall.trim().trim_start_matches('"');
    let exe = exe.split('"').next().unwrap_or(exe);
    Path::new(exe).parent().map(|p| normalize(&p.to_string_lossy()))
}

pub fn sync_version(dir: &Path, version: &str) -> Result<bool, String> {
    let wanted = normalize(&dir.to_string_lossy());
    let root = windows_registry::CURRENT_USER
        .open(UNINSTALL_ROOT)
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = root.keys().map_err(|e| e.to_string())?.collect();
    let mut changed = false;
    for name in names {
        let Ok(entry) = root.open(&name) else {
            continue;
        };
        if location(&entry).as_deref() != Some(wanted.as_path()) {
            continue;
        }
        if entry.get_string("DisplayVersion").ok().as_deref() == Some(version) {
            continue;
        }
        let writable = windows_registry::CURRENT_USER
            .create(format!(r"{UNINSTALL_ROOT}\{name}"))
            .map_err(|e| e.to_string())?;
        writable
            .set_string("DisplayVersion", version)
            .map_err(|e| e.to_string())?;
        changed = true;
    }
    Ok(changed)
}

pub fn is_installed(dir: &Path) -> bool {
    dir.join(crate::build::UNINSTALL_EXE).is_file()
}
