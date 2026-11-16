//! Registry access. Writes go to HKEY_CURRENT_USER only (no admin rights needed); HKLM is read-only.

use std::io;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Win2Console";

/// Reads a `REG_SZ` / `REG_EXPAND_SZ` value from HKCU. `None` if missing or unreadable.
#[cfg(windows)]
pub fn hkcu_string(subkey: &str, value: &str) -> Option<String> {
    read_string(windows::Win32::System::Registry::HKEY_CURRENT_USER, subkey, value)
}

/// Reads a `REG_SZ` / `REG_EXPAND_SZ` value from HKLM (read-only). `None` if missing or unreadable.
#[cfg(windows)]
pub fn hklm_string(subkey: &str, value: &str) -> Option<String> {
    read_string(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, subkey, value)
}

#[cfg(windows)]
fn read_string(root: windows::Win32::System::Registry::HKEY, subkey: &str, value: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::PCWSTR;

    let (subkey, value) = (crate::wide(subkey), crate::wide(value));
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut len: u32 = 0;
    // SAFETY: pointers are valid null-terminated UTF-16 strings / out-params for the duration of the calls.
    unsafe {
        if RegGetValueW(root, PCWSTR(subkey.as_ptr()), PCWSTR(value.as_ptr()), flags, None, None, Some(&mut len))
            != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2)];
        if RegGetValueW(
            root,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

#[cfg(not(windows))]
pub fn hkcu_string(_subkey: &str, _value: &str) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn hklm_string(_subkey: &str, _value: &str) -> Option<String> {
    None
}

/// Adds (`Some(command)`) or removes (`None`) Win2Console from `HKCU\...\Run`.
#[cfg(windows)]
pub fn set_run_at_login(command: Option<&str>) -> io::Result<()> {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
    use windows::core::PCWSTR;

    let (key, name) = (crate::wide(RUN_KEY), crate::wide(RUN_VALUE));
    // SAFETY: valid null-terminated UTF-16 buffers; data length is in bytes including the terminator.
    let err = unsafe {
        match command {
            Some(cmd) => {
                let data = crate::wide(cmd);
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            }
            None => match RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr())) {
                ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                e => e,
            },
        }
    };
    if err == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(err.0 as i32)) }
}

#[cfg(not(windows))]
pub fn set_run_at_login(_command: Option<&str>) -> io::Result<()> {
    Err(crate::unsupported())
}

/// Whether Win2Console is registered to start at login.
pub fn run_at_login_enabled() -> bool {
    hkcu_string(RUN_KEY, RUN_VALUE).is_some()
}

/// Writes an HKCU value (`REG_SZ` for `Ok(text)`, `REG_DWORD` for `Err(number)`).
#[cfg(windows)]
pub fn set_hkcu(subkey: &str, name: &str, value: Result<&str, u32>) -> io::Result<()> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RegSetKeyValueW};
    use windows::core::PCWSTR;

    let (key, name) = (crate::wide(subkey), crate::wide(name));
    let text = value.ok().map(crate::wide);
    let number = value.err();
    // SAFETY: valid null-terminated UTF-16 buffers; data lengths are in bytes.
    let err = unsafe {
        match (&text, &number) {
            (Some(t), _) => RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_SZ.0,
                Some(t.as_ptr().cast()),
                (t.len() * 2) as u32,
            ),
            (_, Some(n)) => RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_DWORD.0,
                Some((n as *const u32).cast()),
                4,
            ),
            _ => ERROR_SUCCESS,
        }
    };
    if err == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(err.0 as i32)) }
}

#[cfg(not(windows))]
pub fn set_hkcu(_subkey: &str, _name: &str, _value: Result<&str, u32>) -> io::Result<()> {
    Err(crate::unsupported())
}
