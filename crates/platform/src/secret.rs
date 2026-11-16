//! Encrypting small secrets (account tokens) with the current Windows user's key (DPAPI).

use std::io;

#[cfg(windows)]
fn blob_to_vec(blob: &windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with LocalAlloc; we copy then free once.
    unsafe {
        let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(blob.pbData.cast())));
        bytes
    }
}

#[cfg(windows)]
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    use windows::core::PCWSTR;
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `data` for the call's duration; DPAPI only reads it.
    unsafe { CryptProtectData(&input, PCWSTR::null(), None, None, None, 0, &mut out) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(blob_to_vec(&out))
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`.
    unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut out) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(blob_to_vec(&out))
}

/// Development builds on other OSes store secrets unencrypted.
#[cfg(not(windows))]
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}
