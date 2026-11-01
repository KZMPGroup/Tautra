use std::os::windows::process::CommandExt;
use std::path::Path;

pub fn reveal(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(|_| ())
}
