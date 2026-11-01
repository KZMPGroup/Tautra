use std::path::PathBuf;

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn safe_component(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." || part.chars().count() > 255 {
        return false;
    }
    if part
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*'))
    {
        return false;
    }
    if part.ends_with(' ') || part.ends_with('.') {
        return false;
    }
    let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
    !RESERVED.contains(&stem.trim_end())
}

pub fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split('/') {
        if !safe_component(part) {
            return None;
        }
        out.push(part);
    }
    (!out.as_os_str().is_empty()).then_some(out)
}
