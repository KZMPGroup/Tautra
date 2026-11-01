use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{implement, w, BOOL, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, POINT, SIZE, S_OK,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    CoCreateInstance, IBindCtx, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC,
    TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, ReleaseStgMedium, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardState, SetKeyboardState, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    Common::ITEMIDLIST, ILFree, SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
    BHID_DataObject, CLSID_DragDropHelper, IDragSourceHelper, IDragSourceHelper2, SHGetFileInfoW,
    SHGetImageList,
    DSH_ALLOWDROPDESCRIPTIONTEXT, SHDRAGIMAGE, SHFILEINFOW, SHGFI_SYSICONINDEX,
    SHGFI_USEFILEATTRIBUTES, SHIL_JUMBO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, LoadCursorW, SetCursor, DI_NORMAL, IDC_ARROW,
};
use windows_core::Interface;

const IMAGE_SIZE: i32 = 96;
const CLR_NONE: u32 = 0xFFFF_FFFF;
const HIDDEN_KEYS: [VIRTUAL_KEY; 9] = [
    VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU,
    VK_RMENU,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragEffect {
    None,
    Copy,
}

pub struct DragLabel {
    pub name: String,
    pub dir: bool,
}

fn register(name: PCWSTR) -> u16 {
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

fn read_dword(data: &IDataObject, format: u16) -> Option<u32> {
    let query = FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut medium = unsafe { data.GetData(&query) }.ok()?;
    let mut value = None;
    if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        unsafe {
            let handle = medium.u.hGlobal;
            if GlobalSize(handle) >= 4 {
                let ptr = GlobalLock(handle) as *const u8;
                if !ptr.is_null() {
                    let bytes = std::slice::from_raw_parts(ptr, 4);
                    value = Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                    let _ = GlobalUnlock(handle);
                }
            }
        }
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    value
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetAsyncKeyState(key.0 as i32) };
    state < 0
}

fn hide_modifiers() {
    let mut state = [0u8; 256];
    if unsafe { GetKeyboardState(&mut state) }.is_err() {
        return;
    }
    let mut changed = false;
    for key in HIDDEN_KEYS {
        let slot = &mut state[key.0 as usize];
        if *slot & 0x80 != 0 {
            *slot &= !0x80;
            changed = true;
        }
    }
    if changed {
        let _ = unsafe { SetKeyboardState(&state) };
    }
}

#[implement(IDropSource)]
struct DragSource {
    cancel: Arc<AtomicBool>,
    data: IDataObject,
    layered: u16,
    native: bool,
}

impl IDropSource_Impl for DragSource_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if !self.native {
            hide_modifiers();
        }
        if fescapepressed.as_bool() || key_down(VK_ESCAPE) || self.cancel.load(Ordering::Acquire) {
            return DRAGDROP_S_CANCEL;
        }
        if grfkeystate.0 & MK_LBUTTON.0 == 0 {
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        if read_dword(&self.data, self.layered).unwrap_or(0) != 0 {
            if let Ok(arrow) = unsafe { LoadCursorW(None, IDC_ARROW) } {
                unsafe { SetCursor(Some(arrow)) };
            }
            return S_OK;
        }
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn icon_bitmap(label: &DragLabel) -> Option<HBITMAP> {
    let name: Vec<u16> = label.name.encode_utf16().chain(std::iter::once(0)).collect();
    let attributes = if label.dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    let mut info = SHFILEINFOW::default();
    let found = unsafe {
        SHGetFileInfoW(
            PCWSTR(name.as_ptr()),
            attributes,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES,
        )
    };
    if found == 0 {
        return None;
    }
    let list: IImageList = unsafe { SHGetImageList(SHIL_JUMBO as i32) }.ok()?;
    let icon = unsafe { list.GetIcon(info.iIcon, ILD_TRANSPARENT.0) }.ok()?;
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: IMAGE_SIZE,
        biHeight: -IMAGE_SIZE,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let bitmap_info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap = unsafe {
        let dc = CreateCompatibleDC(None);
        let bitmap = CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0);
        let result = bitmap.and_then(|bitmap| {
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            let drawn = DrawIconEx(dc, 0, 0, icon, IMAGE_SIZE, IMAGE_SIZE, 0, None, DI_NORMAL);
            SelectObject(dc, old);
            match drawn {
                Ok(()) => Ok(bitmap),
                Err(e) => {
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                    Err(e)
                }
            }
        });
        let _ = DeleteDC(dc);
        let _ = DestroyIcon(icon);
        result
    };
    bitmap.ok()
}

fn attach_image(data: &IDataObject, label: &DragLabel) {
    let helper: IDragSourceHelper =
        match unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) } {
            Ok(helper) => helper,
            Err(e) => {
                tracing::debug!("drag image tidak tersedia: {}", e.message());
                return;
            }
        };
    if let Ok(helper2) = helper.cast::<IDragSourceHelper2>() {
        let _ = unsafe { helper2.SetFlags(DSH_ALLOWDROPDESCRIPTIONTEXT.0 as u32) };
    }
    let Some(bitmap) = icon_bitmap(label) else {
        return;
    };
    let image = SHDRAGIMAGE {
        sizeDragImage: SIZE {
            cx: IMAGE_SIZE,
            cy: IMAGE_SIZE,
        },
        ptOffset: POINT {
            x: IMAGE_SIZE / 2,
            y: IMAGE_SIZE * 3 / 4,
        },
        hbmpDragImage: bitmap,
        crColorKey: COLORREF(CLR_NONE),
    };
    if let Err(e) = unsafe { helper.InitializeFromBitmap(&image, data) } {
        tracing::debug!("drag image gagal dipasang: {}", e.message());
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
    }
}

pub fn shell_data(paths: &[PathBuf]) -> Result<IDataObject, String> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    let mut failure = None;
    for path in paths {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut pidl = std::ptr::null_mut();
        match unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None) } {
            Ok(()) => pidls.push(pidl),
            Err(e) => {
                failure = Some(format!("{} tidak bisa dibuka: {}", path.display(), e.message()));
                break;
            }
        }
    }
    let result = match failure {
        Some(reason) => Err(reason),
        None if pidls.is_empty() => Err("tidak ada file untuk diseret".into()),
        None => {
            let list: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
            unsafe { SHCreateShellItemArrayFromIDLists(&list) }
                .and_then(|items| unsafe { items.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject) })
                .map_err(|e| format!("data drag gagal dibuat: {}", e.message()))
        }
    };
    for pidl in pidls {
        unsafe { ILFree(Some(pidl as *const ITEMIDLIST)) };
    }
    result
}

fn effect_of(value: u32) -> DragEffect {
    if value & (DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0) != 0 {
        DragEffect::Copy
    } else {
        DragEffect::None
    }
}

pub fn run(
    data: &IDataObject,
    label: &DragLabel,
    cancel: Arc<AtomicBool>,
    native: bool,
) -> Result<DragEffect, String> {
    let layered = register(w!("IsShowingLayered"));
    let logical = register(w!("Logical Performed DropEffect"));
    let performed = register(w!("Performed DropEffect"));
    let allowed = if native {
        DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK
    } else {
        attach_image(data, label);
        hide_modifiers();
        DROPEFFECT_COPY
    };
    let source: IDropSource = DragSource {
        cancel,
        data: data.clone(),
        layered,
        native,
    }
    .into();
    let mut effect = DROPEFFECT_NONE;
    let hr = unsafe { DoDragDrop(data, &source, allowed, &mut effect) };
    if hr == DRAGDROP_S_CANCEL {
        return Ok(DragEffect::None);
    }
    if hr != DRAGDROP_S_DROP {
        return Err(format!("DoDragDrop gagal: {}", windows::core::Error::from_hresult(hr).message()));
    }
    let direct = effect_of(effect.0);
    if direct != DragEffect::None {
        return Ok(direct);
    }
    let reported = read_dword(data, logical)
        .or_else(|| read_dword(data, performed))
        .unwrap_or(0);
    Ok(effect_of(reported))
}
