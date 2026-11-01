use tautra_ipc::api::CompatReport;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    IMAGE_FILE_MACHINE_UNKNOWN,
};

const MIN_BUILD: u32 = 19041;

fn windows_build() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    if unsafe { RtlGetVersion(&mut info) }.is_ok() {
        info.dwBuildNumber
    } else {
        0
    }
}

fn native_machine() -> IMAGE_FILE_MACHINE {
    let mut process = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut native = IMAGE_FILE_MACHINE_UNKNOWN;
    if unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) }.is_ok() {
        native
    } else {
        IMAGE_FILE_MACHINE_UNKNOWN
    }
}

pub fn report() -> CompatReport {
    let build = windows_build();
    let native = native_machine();
    let mut warnings = Vec::new();

    if build != 0 && build < MIN_BUILD {
        warnings.push(format!(
            "Windows build {build} lebih lama dari yang didukung (minimal {MIN_BUILD}, yaitu Windows 10 versi 2004). Sebagian fitur bisa gagal."
        ));
    }

    let arch = match native {
        m if m == IMAGE_FILE_MACHINE_AMD64 => "x64".to_string(),
        m if m == IMAGE_FILE_MACHINE_ARM64 => {
            warnings.push(
                "Tautra berjalan sebagai aplikasi x64 di Windows ARM64 (emulasi). Hook dan injeksi input bisa tidak stabil.".to_string(),
            );
            "ARM64 (emulasi)".to_string()
        }
        _ => format!("tidak dikenal (0x{:04X})", native.0),
    };

    let os_version = if build == 0 {
        "Windows (versi tidak terbaca)".to_string()
    } else {
        format!("Windows build {build}")
    };

    CompatReport {
        os_version,
        arch,
        supported: warnings.is_empty(),
        warnings,
    }
}
