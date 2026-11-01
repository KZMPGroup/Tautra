use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{implement, Error, Result as WinResult, GUID, HRESULT, PWSTR};
use windows::Win32::Foundation::{
    E_INVALIDARG, E_NOTIMPL, FILETIME, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION, STG_E_READFAULT,
    S_OK,
};
use windows::Win32::System::Com::{
    CoTaskMemAlloc, ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE, STATFLAG,
    STATFLAG_NONAME, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR,
    STREAM_SEEK_END, STREAM_SEEK_SET,
};

use crate::service::{ClipboardHandler, RemoteFile};

#[implement(IStream)]
pub struct RemoteStream {
    reader: Mutex<Box<dyn RemoteFile>>,
    size: u64,
    pos: AtomicU64,
    read_pos: AtomicU64,
    name: Vec<u16>,
    modified: FILETIME,
    handler: Arc<dyn ClipboardHandler>,
}

impl RemoteStream {
    pub fn new(
        reader: Box<dyn RemoteFile>,
        size: u64,
        name: &str,
        modified: FILETIME,
        handler: Arc<dyn ClipboardHandler>,
    ) -> Self {
        Self {
            reader: Mutex::new(reader),
            size,
            pos: AtomicU64::new(0),
            read_pos: AtomicU64::new(0),
            name: name.encode_utf16().chain(std::iter::once(0)).collect(),
            modified,
            handler,
        }
    }
}

impl ISequentialStream_Impl for RemoteStream_Impl {
    fn Read(&self, pv: *mut core::ffi::c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        let report = |n: usize| {
            if !pcbread.is_null() {
                unsafe { *pcbread = n as u32 };
            }
        };
        if pv.is_null() {
            return E_INVALIDARG;
        }
        let pos = self.pos.load(Ordering::Acquire);
        if pos >= self.size || cb == 0 {
            report(0);
            return S_OK;
        }
        if pos != self.read_pos.load(Ordering::Acquire) {
            report(0);
            return STG_E_INVALIDFUNCTION;
        }
        let want = (cb as u64).min(self.size - pos) as usize;
        let buf = unsafe { std::slice::from_raw_parts_mut(pv as *mut u8, want) };
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        let mut filled = 0;
        while filled < buf.len() {
            let chunk = &mut buf[filled..];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(chunk)))
                .unwrap_or_else(|_| Err("terjadi kesalahan internal saat membaca file".into()));
            match result {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => {
                    tracing::warn!("file dari perangkat lain gagal dibaca: {e}");
                    self.handler
                        .error(format!("File dari perangkat lain gagal ditempel: {e}"));
                    report(filled);
                    return STG_E_READFAULT;
                }
            }
        }
        self.pos.store(pos + filled as u64, Ordering::Release);
        self.read_pos.store(pos + filled as u64, Ordering::Release);
        report(filled);
        S_OK
    }

    fn Write(&self, _pv: *const core::ffi::c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for RemoteStream_Impl {
    fn Seek(&self, dlibmove: i64, dworigin: STREAM_SEEK, plibnewposition: *mut u64) -> WinResult<()> {
        let base = match dworigin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => self.pos.load(Ordering::Acquire) as i128,
            STREAM_SEEK_END => self.size as i128,
            _ => return Err(STG_E_INVALIDFUNCTION.into()),
        };
        let target = base + dlibmove as i128;
        if target < 0 || target > self.size as i128 {
            return Err(STG_E_INVALIDFUNCTION.into());
        }
        self.pos.store(target as u64, Ordering::Release);
        if !plibnewposition.is_null() {
            unsafe { *plibnewposition = target as u64 };
        }
        Ok(())
    }

    fn SetSize(&self, _libnewsize: u64) -> WinResult<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn CopyTo(
        &self,
        _pstm: windows::core::Ref<IStream>,
        _cb: u64,
        _pcbread: *mut u64,
        _pcbwritten: *mut u64,
    ) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn Commit(&self, _grfcommitflags: &STGC) -> WinResult<()> {
        Ok(())
    }

    fn Revert(&self) -> WinResult<()> {
        Ok(())
    }

    fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> WinResult<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> WinResult<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn Stat(&self, pstatstg: *mut STATSTG, grfstatflag: &STATFLAG) -> WinResult<()> {
        if pstatstg.is_null() {
            return Err(Error::from(E_INVALIDARG));
        }
        let name = if grfstatflag.0 & STATFLAG_NONAME.0 != 0 {
            PWSTR::null()
        } else {
            let bytes = self.name.len() * 2;
            let ptr = unsafe { CoTaskMemAlloc(bytes) } as *mut u16;
            if !ptr.is_null() {
                unsafe { std::ptr::copy_nonoverlapping(self.name.as_ptr(), ptr, self.name.len()) };
            }
            PWSTR(ptr)
        };
        unsafe {
            *pstatstg = STATSTG {
                pwcsName: name,
                r#type: STGTY_STREAM.0 as u32,
                cbSize: self.size,
                mtime: self.modified,
                ctime: self.modified,
                atime: self.modified,
                grfMode: STGM_READ,
                grfLocksSupported: 0,
                clsid: GUID::zeroed(),
                grfStateBits: 0,
                reserved: 0,
            };
        }
        Ok(())
    }

    fn Clone(&self) -> WinResult<IStream> {
        Err(E_NOTIMPL.into())
    }
}
