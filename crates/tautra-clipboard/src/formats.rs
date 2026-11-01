use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;

#[derive(Debug, Clone, Copy)]
pub struct Formats {
    pub html: u16,
    pub rtf: u16,
    pub png: u16,
    pub group_descriptor: u16,
    pub contents: u16,
    pub drop_effect: u16,
    pub history: u16,
    pub cloud: u16,
    pub exclude: u16,
    pub viewer_ignore: u16,
}

fn register(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

pub fn formats() -> &'static Formats {
    static FORMATS: OnceLock<Formats> = OnceLock::new();
    FORMATS.get_or_init(|| Formats {
        html: register(w!("HTML Format")),
        rtf: register(w!("Rich Text Format")),
        png: register(w!("PNG")),
        group_descriptor: register(w!("FileGroupDescriptorW")),
        contents: register(w!("FileContents")),
        drop_effect: register(w!("Preferred DropEffect")),
        history: register(w!("CanIncludeInClipboardHistory")),
        cloud: register(w!("CanUploadToCloudClipboard")),
        exclude: register(w!("ExcludeClipboardContentFromMonitorProcessing")),
        viewer_ignore: register(w!("Clipboard Viewer Ignore")),
    })
}
