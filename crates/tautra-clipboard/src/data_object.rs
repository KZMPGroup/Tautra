use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tautra_proto::share::{ClipFormat, FileEntry, MAX_PATH_CHARS};
use windows::core::{implement, Error, Result as WinResult, BOOL, HRESULT};
use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DV_E_FORMATETC, DV_E_LINDEX, E_FAIL, E_INVALIDARG, E_NOTIMPL, FILETIME,
    OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumFORMATETC_Impl, IEnumSTATDATA,
    IStream, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
    TYMED_ISTREAM,
};
use windows::Win32::System::Ole::{ReleaseStgMedium, CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{
    FD_ATTRIBUTES, FD_FILESIZE, FD_PROGRESSUI, FD_UNICODE, FD_WRITESTIME, FILEDESCRIPTORW,
};

use crate::formats::formats;
use crate::global;
use crate::service::{ClipboardHandler, RemoteContent};
use crate::stream::RemoteStream;
use crate::png_to_dib;
use crate::target::shell_target;

const DROPEFFECT_COPY: u32 = 1;
const MAX_STORED_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Item(ClipFormat),
    Dib,
    Descriptor,
    Contents,
    DropEffect,
    Dword(u32),
    Hdrop,
}

pub trait DropFiles: Send + Sync + 'static {
    fn paths(&self) -> Result<Vec<PathBuf>, String>;
    fn dropped_on_shell(&self) -> Option<bool>;
}

fn dropfiles(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [20i32, 0, 0, 0, 1] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for path in paths {
        for unit in path.as_os_str().encode_wide() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

#[derive(Debug, Clone, Copy)]
struct Offered {
    format: u16,
    tymed: u32,
    source: Source,
}

struct FileTable {
    offer_id: u64,
    entries: Vec<FileEntry>,
    map: Vec<u32>,
}

pub fn filetime(unix_ms: u64) -> FILETIME {
    let ticks = unix_ms
        .saturating_mul(10_000)
        .saturating_add(116_444_736_000_000_000);
    FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    }
}

fn hglobal(format: u16, source: Source) -> Offered {
    Offered {
        format,
        tymed: TYMED_HGLOBAL.0 as u32,
        source,
    }
}

fn medium_hglobal(data: &[u8]) -> WinResult<STGMEDIUM> {
    let handle = global::alloc(data)?;
    Ok(STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: handle },
        pUnkForRelease: ManuallyDrop::new(None),
    })
}

#[implement(IDataObject)]
pub struct RemoteData {
    offer_id: u64,
    handler: Arc<dyn ClipboardHandler>,
    offered: Vec<Offered>,
    cache: Mutex<HashMap<ClipFormat, Arc<Vec<u8>>>>,
    dib: Mutex<Option<Arc<Vec<u8>>>>,
    files: Option<FileTable>,
    drop_files: Option<Arc<dyn DropFiles>>,
    stored: Mutex<Vec<(u16, Vec<u8>)>>,
}

pub fn create_data_object(content: RemoteContent, handler: Arc<dyn ClipboardHandler>) -> IDataObject {
    RemoteData::new(content, handler).into()
}

pub fn create_drag_object(
    content: RemoteContent,
    handler: Arc<dyn ClipboardHandler>,
    files: Arc<dyn DropFiles>,
) -> IDataObject {
    RemoteData::with_files(content, handler, Some(files)).into()
}

pub fn read_dword(data: &IDataObject, format: u16) -> Option<u32> {
    let query = FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut medium = unsafe { data.GetData(&query) }.ok()?;
    let bytes = if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        global::read(unsafe { medium.u.hGlobal }, 64)
    } else {
        None
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    let bytes = bytes?;
    (bytes.len() >= 4).then(|| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl RemoteData {
    pub fn new(content: RemoteContent, handler: Arc<dyn ClipboardHandler>) -> Self {
        Self::with_files(content, handler, None)
    }

    fn with_files(
        content: RemoteContent,
        handler: Arc<dyn ClipboardHandler>,
        drop_files: Option<Arc<dyn DropFiles>>,
    ) -> Self {
        let f = formats();
        let mut offered = Vec::new();
        let mut cache = HashMap::new();
        let mut lazy = false;

        let files = content.files.and_then(|files| {
            let map: Vec<u32> = files
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.path.encode_utf16().count() <= MAX_PATH_CHARS)
                .map(|(i, _)| i as u32)
                .collect();
            let skipped = files.entries.len() - map.len();
            if skipped > 0 {
                handler.error(format!(
                    "{skipped} file/folder tidak bisa ditempel karena path-nya lebih dari {MAX_PATH_CHARS} karakter"
                ));
            }
            (!map.is_empty()).then_some(FileTable {
                offer_id: files.offer_id,
                entries: files.entries,
                map,
            })
        });
        let drop_files = drop_files.filter(|_| files.is_some());
        if drop_files.is_some() {
            offered.push(hglobal(CF_HDROP.0, Source::Hdrop));
        }
        if files.is_some() {
            lazy = true;
            offered.push(hglobal(f.group_descriptor, Source::Descriptor));
            offered.push(Offered {
                format: f.contents,
                tymed: TYMED_ISTREAM.0 as u32,
                source: Source::Contents,
            });
            offered.push(hglobal(f.drop_effect, Source::DropEffect));
        }

        let mut order: Vec<_> = content.items.into_iter().collect();
        order.sort_by_key(|item| match item.format {
            ClipFormat::Png => 0,
            ClipFormat::Html => 1,
            ClipFormat::Rtf => 2,
            ClipFormat::Text => 3,
        });
        for item in order {
            match item.inline {
                Some(data) => {
                    cache.insert(item.format, Arc::new(data));
                }
                None => lazy = true,
            }
            match item.format {
                ClipFormat::Text => offered.push(hglobal(CF_UNICODETEXT.0, Source::Item(item.format))),
                ClipFormat::Html => offered.push(hglobal(f.html, Source::Item(item.format))),
                ClipFormat::Rtf => offered.push(hglobal(f.rtf, Source::Item(item.format))),
                ClipFormat::Png => {
                    offered.push(hglobal(f.png, Source::Item(item.format)));
                    offered.push(hglobal(CF_DIB.0, Source::Dib));
                }
            }
        }
        if lazy {
            offered.push(hglobal(f.history, Source::Dword(0)));
            offered.push(hglobal(f.cloud, Source::Dword(0)));
        }
        Self {
            offer_id: content.offer_id,
            handler,
            offered,
            cache: Mutex::new(cache),
            dib: Mutex::new(None),
            files,
            drop_files,
            stored: Mutex::new(Vec::new()),
        }
    }

    fn stored(&self, format: &FORMATETC) -> Option<Vec<u8>> {
        if format.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return None;
        }
        self.stored
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(f, _)| *f == format.cfFormat)
            .map(|(_, data)| data.clone())
    }

    fn find(&self, format: &FORMATETC) -> Option<Offered> {
        self.offered
            .iter()
            .find(|o| {
                o.format == format.cfFormat
                    && format.tymed & o.tymed != 0
                    && self.visible(o.source, shell_target)
            })
            .copied()
    }

    fn visible(&self, source: Source, shell: impl FnOnce() -> bool) -> bool {
        let Some(files) = self.drop_files.as_ref() else {
            return true;
        };
        let shell = || files.dropped_on_shell().unwrap_or_else(shell);
        match source {
            Source::Hdrop => !shell(),
            Source::Descriptor | Source::Contents => shell(),
            _ => true,
        }
    }

    fn fail(&self, message: String) -> Error {
        tracing::warn!("{message}");
        self.handler.error(message.clone());
        Error::new(E_FAIL, message)
    }

    fn data(&self, format: ClipFormat) -> WinResult<Arc<Vec<u8>>> {
        if let Some(data) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&format)
        {
            return Ok(data.clone());
        }
        let data = self
            .handler
            .fetch(self.offer_id, format)
            .map(Arc::new)
            .map_err(|e| self.fail(format!("Isi clipboard dari perangkat lain gagal diambil: {e}")))?;
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(format, data.clone());
        Ok(data)
    }

    fn dib(&self) -> WinResult<Arc<Vec<u8>>> {
        if let Some(dib) = self.dib.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(dib.clone());
        }
        let png = self.data(ClipFormat::Png)?;
        let dib = png_to_dib(&png)
            .map(Arc::new)
            .map_err(|e| self.fail(format!("Gambar dari perangkat lain gagal dikonversi: {e}")))?;
        *self.dib.lock().unwrap_or_else(|e| e.into_inner()) = Some(dib.clone());
        Ok(dib)
    }

    fn descriptor(&self, table: &FileTable) -> Vec<u8> {
        let size = std::mem::size_of::<FILEDESCRIPTORW>();
        let count = table.map.len();
        let mut out = vec![0u8; 4 + size * count.max(1)];
        out[..4].copy_from_slice(&(count as u32).to_le_bytes());
        let flags = [FD_ATTRIBUTES, FD_FILESIZE, FD_WRITESTIME, FD_PROGRESSUI, FD_UNICODE]
            .iter()
            .fold(0u32, |acc, f| acc | f.0 as u32);
        for (slot, &index) in table.map.iter().enumerate() {
            let entry = &table.entries[index as usize];
            let mut name = [0u16; 260];
            for (dst, unit) in name
                .iter_mut()
                .zip(entry.path.replace('/', "\\").encode_utf16())
            {
                *dst = unit;
            }
            let descriptor = FILEDESCRIPTORW {
                dwFlags: flags,
                dwFileAttributes: if entry.dir {
                    FILE_ATTRIBUTE_DIRECTORY.0
                } else {
                    FILE_ATTRIBUTE_NORMAL.0
                },
                ftLastWriteTime: filetime(entry.modified_ms),
                nFileSizeHigh: (entry.size >> 32) as u32,
                nFileSizeLow: entry.size as u32,
                cFileName: name,
                ..Default::default()
            };
            unsafe {
                std::ptr::write_unaligned(
                    out.as_mut_ptr().add(4 + slot * size) as *mut FILEDESCRIPTORW,
                    descriptor,
                );
            }
        }
        out
    }

    fn bytes(&self, source: Source) -> WinResult<Vec<u8>> {
        Ok(match source {
            Source::Item(ClipFormat::Text) => {
                let data = self.data(ClipFormat::Text)?;
                let text = String::from_utf8_lossy(&data);
                text.encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect()
            }
            Source::Item(format @ (ClipFormat::Html | ClipFormat::Rtf)) => {
                let data = self.data(format)?;
                let mut out = Vec::with_capacity(data.len() + 1);
                out.extend_from_slice(&data);
                out.push(0);
                out
            }
            Source::Item(ClipFormat::Png) => self.data(ClipFormat::Png)?.to_vec(),
            Source::Dib => self.dib()?.to_vec(),
            Source::Descriptor => match &self.files {
                Some(table) => self.descriptor(table),
                None => return Err(DV_E_FORMATETC.into()),
            },
            Source::DropEffect => DROPEFFECT_COPY.to_le_bytes().to_vec(),
            Source::Hdrop => match &self.drop_files {
                Some(files) => {
                    let paths = files
                        .paths()
                        .map_err(|e| self.fail(format!("File dari perangkat lain gagal disiapkan: {e}")))?;
                    dropfiles(&paths)
                }
                None => return Err(DV_E_FORMATETC.into()),
            },
            Source::Dword(value) => value.to_le_bytes().to_vec(),
            Source::Contents => return Err(DV_E_FORMATETC.into()),
        })
    }

    fn contents(&self, lindex: i32) -> WinResult<STGMEDIUM> {
        let table = self.files.as_ref().ok_or(Error::from(DV_E_FORMATETC))?;
        let slot = usize::try_from(lindex).map_err(|_| Error::from(DV_E_LINDEX))?;
        let index = *table.map.get(slot).ok_or(Error::from(DV_E_LINDEX))?;
        let entry = &table.entries[index as usize];
        if entry.dir {
            return Err(DV_E_LINDEX.into());
        }
        let reader = self
            .handler
            .open_file(table.offer_id, index, entry.size)
            .map_err(|e| self.fail(format!("File dari perangkat lain gagal dibuka: {e}")))?;
        let name = entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string();
        let stream: IStream = RemoteStream::new(
            reader,
            entry.size,
            &name,
            filetime(entry.modified_ms),
            self.handler.clone(),
        )
        .into();
        Ok(STGMEDIUM {
            tymed: TYMED_ISTREAM.0 as u32,
            u: STGMEDIUM_0 {
                pstm: ManuallyDrop::new(Some(stream)),
            },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

impl IDataObject_Impl for RemoteData_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> WinResult<STGMEDIUM> {
        let format = unsafe { pformatetcin.as_ref() }.ok_or(Error::from(E_INVALIDARG))?;
        if let Some(bytes) = self.stored(format) {
            return medium_hglobal(&bytes);
        }
        let offered = self.find(format).ok_or(Error::from(DV_E_FORMATETC))?;
        if offered.source == Source::Contents {
            return self.contents(format.lindex);
        }
        let bytes = self.bytes(offered.source)?;
        medium_hglobal(&bytes)
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        match unsafe { pformatetc.as_ref() } {
            Some(format) if self.find(format).is_some() || self.stored(format).is_some() => S_OK,
            Some(_) => DV_E_FORMATETC,
            None => E_INVALIDARG,
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        pformatectin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> HRESULT {
        if let (Some(input), false) = (unsafe { pformatectin.as_ref() }, pformatetcout.is_null()) {
            unsafe {
                *pformatetcout = FORMATETC {
                    ptd: std::ptr::null_mut(),
                    ..*input
                };
            }
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(
        &self,
        pformatetc: *const FORMATETC,
        pmedium: *const STGMEDIUM,
        frelease: BOOL,
    ) -> WinResult<()> {
        let (Some(format), Some(medium)) = (unsafe { pformatetc.as_ref() }, unsafe { pmedium.as_ref() }) else {
            return Err(E_INVALIDARG.into());
        };
        let bytes = if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            global::read(unsafe { medium.u.hGlobal }, MAX_STORED_BYTES)
        } else {
            None
        };
        if frelease.as_bool() {
            unsafe { ReleaseStgMedium(pmedium as *mut STGMEDIUM) };
        }
        let Some(bytes) = bytes else {
            return if medium.tymed == TYMED_HGLOBAL.0 as u32 {
                Ok(())
            } else {
                Err(E_NOTIMPL.into())
            };
        };
        let mut stored = self.stored.lock().unwrap_or_else(|e| e.into_inner());
        match stored.iter_mut().find(|(f, _)| *f == format.cfFormat) {
            Some((_, data)) => *data = bytes,
            None => stored.push((format.cfFormat, bytes)),
        }
        Ok(())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> WinResult<IEnumFORMATETC> {
        if dwdirection != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        let stored: Vec<u16> = self
            .stored
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(f, _)| *f)
            .filter(|f| !self.offered.iter().any(|o| o.format == *f))
            .collect();
        let shell = self
            .drop_files
            .as_ref()
            .is_some_and(|files| files.dropped_on_shell().unwrap_or_else(shell_target));
        let list = self
            .offered
            .iter()
            .filter(|o| self.visible(o.source, || shell))
            .map(|o| (o.format, o.tymed))
            .chain(stored.into_iter().map(|f| (f, TYMED_HGLOBAL.0 as u32)))
            .map(|(format, tymed)| FORMATETC {
                cfFormat: format,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed,
            })
            .collect();
        Ok(FormatEnum::new(list, 0).into())
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: windows::core::Ref<IAdviseSink>,
    ) -> WinResult<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _dwconnection: u32) -> WinResult<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> WinResult<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IEnumFORMATETC)]
struct FormatEnum {
    list: Vec<FORMATETC>,
    pos: AtomicUsize,
}

impl FormatEnum {
    fn new(list: Vec<FORMATETC>, pos: usize) -> Self {
        Self {
            list,
            pos: AtomicUsize::new(pos),
        }
    }
}

impl IEnumFORMATETC_Impl for FormatEnum_Impl {
    fn Next(&self, celt: u32, rgelt: *mut FORMATETC, pceltfetched: *mut u32) -> HRESULT {
        if rgelt.is_null() {
            return E_INVALIDARG;
        }
        let pos = self.pos.load(Ordering::Acquire).min(self.list.len());
        let count = (celt as usize).min(self.list.len() - pos);
        for i in 0..count {
            unsafe { *rgelt.add(i) = self.list[pos + i] };
        }
        self.pos.store(pos + count, Ordering::Release);
        if !pceltfetched.is_null() {
            unsafe { *pceltfetched = count as u32 };
        }
        if count == celt as usize {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, celt: u32) -> WinResult<()> {
        let pos = self.pos.load(Ordering::Acquire);
        self.pos
            .store((pos + celt as usize).min(self.list.len()), Ordering::Release);
        Ok(())
    }

    fn Reset(&self) -> WinResult<()> {
        self.pos.store(0, Ordering::Release);
        Ok(())
    }

    fn Clone(&self) -> WinResult<IEnumFORMATETC> {
        Ok(FormatEnum::new(self.list.clone(), self.pos.load(Ordering::Acquire)).into())
    }
}
