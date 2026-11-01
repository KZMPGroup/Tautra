use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const ENTROPY: &[u8] = b"Tautra.DeviceIdentity.v1";

#[derive(Debug, thiserror::Error)]
pub enum DpapiError {
    #[error("DPAPI gagal mengenkripsi: {0}")]
    Protect(windows::core::Error),
    #[error("DPAPI gagal membuka data (akun Windows berbeda atau data rusak): {0}")]
    Unprotect(windows::core::Error),
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

unsafe fn take_blob(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    LocalFree(Some(HLOCAL(out.pbData.cast())));
    bytes
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let input = blob(data);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(DpapiError::Protect)?;
        Ok(take_blob(out))
    }
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let input = blob(data);
    let entropy = blob(ENTROPY);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            Some(&entropy),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(DpapiError::Unprotect)?;
        Ok(take_blob(out))
    }
}
