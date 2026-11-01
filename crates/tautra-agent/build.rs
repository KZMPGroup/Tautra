use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=TAUTRA_BUILD={now}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../crates");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!("cargo:rerun-if-changed=../../ui/src-tauri/icons/icon.ico");

    #[cfg(windows)]
    {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/src-tauri/icons/icon.ico");
        let mut res = tauri_winres::WindowsResource::new();
        res.set_icon(&icon.to_string_lossy());
        res.compile()
            .expect("gagal menanam ikon Tautra ke tautra-agent.exe");
    }
}
