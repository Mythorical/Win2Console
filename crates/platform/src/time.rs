//! Local wall-clock time for the top bar (avoids pulling in a date/time crate).

/// Local time as (hour 0-23, minute).
#[cfg(windows)]
pub fn local_hour_minute() -> (u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain FFI call returning a struct by value.
    let t = unsafe { GetLocalTime() };
    (t.wHour as u32, t.wMinute as u32)
}

/// UTC fallback for non-Windows builds (development only).
#[cfg(not(windows))]
pub fn local_hour_minute() -> (u32, u32) {
    let secs = cs_core::now_unix().max(0) as u64;
    (((secs / 3600) % 24) as u32, ((secs / 60) % 60) as u32)
}

/// 12-hour clock text, e.g. `3:05 PM`.
pub fn clock_text() -> String {
    let (h, m) = local_hour_minute();
    let (h12, suffix) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {suffix}")
}

/// Local date and time for file names, e.g. `2026-11-15 21-04-33`.
#[cfg(windows)]
pub fn file_timestamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain FFI call returning a struct by value.
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
pub fn file_timestamp() -> String {
    cs_core::now_unix().to_string()
}
