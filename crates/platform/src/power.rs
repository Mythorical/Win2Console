//! Sleep / restart / shut down.

use std::io;

#[cfg(windows)]
pub fn sleep() -> io::Result<()> {
    use windows::Win32::System::Power::SetSuspendState;
    // SAFETY: plain FFI call with no pointers. Arguments: hibernate=false, force=false, wake events enabled.
    let ok = unsafe { SetSuspendState(false, false, false) };
    if ok.into() { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(not(windows))]
pub fn sleep() -> io::Result<()> {
    Err(crate::unsupported())
}

/// Uses `shutdown.exe`, which handles the shutdown privilege itself.
pub fn restart() -> io::Result<()> {
    shutdown_exe("/r")
}

pub fn shut_down() -> io::Result<()> {
    shutdown_exe("/s")
}

fn shutdown_exe(mode: &str) -> io::Result<()> {
    if !cfg!(windows) {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "only supported on Windows"));
    }
    let status = std::process::Command::new("shutdown.exe").args([mode, "/t", "0"]).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("shutdown.exe exited with {status}"))) }
}
