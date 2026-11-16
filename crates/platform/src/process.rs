//! Launching games and inspecting / ending processes.

use std::io;
use std::path::Path;

use cs_core::LaunchSpec;

/// Starts a game. Returns the PID for direct exe launches; `None` for URI launches, where the owning
/// launcher (Steam, Epic, ...) starts the game and the tracker finds it by install dir.
pub fn launch(spec: &LaunchSpec) -> io::Result<Option<u32>> {
    match spec {
        LaunchSpec::Uri { uri } => open_uri(uri).map(|()| None),
        LaunchSpec::Exe { path, args, working_dir } => {
            let dir = working_dir.as_deref().or_else(|| path.parent());
            let mut cmd = std::process::Command::new(path);
            // Arguments come from Playnite as a ready-made command line; pass them through verbatim.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                if !args.is_empty() {
                    cmd.raw_arg(args.join(" "));
                }
            }
            #[cfg(not(windows))]
            cmd.args(args);
            if let Some(dir) = dir {
                cmd.current_dir(dir);
            }
            // The child is intentionally not waited on; dropping `Child` doesn't kill it.
            Ok(Some(cmd.spawn()?.id()))
        }
    }
}

#[cfg(windows)]
fn open_uri(uri: &str) -> io::Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{PCWSTR, w};

    let uri_w = crate::wide(uri);
    // SAFETY: valid null-terminated UTF-16 strings; null HWND/params/dir are allowed.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(uri_w.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // ShellExecute returns a value > 32 on success.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!("ShellExecute failed for {uri} (code {})", result.0 as isize)))
    }
}

#[cfg(not(windows))]
fn open_uri(_uri: &str) -> io::Result<()> {
    Err(crate::unsupported())
}

/// What the game tracker needs to know about processes. Abstracted so the tracker can be unit-tested.
pub trait ProcessSource: Send + Sync {
    /// All running processes as `(pid, image path)`, with paths in `normalize_path` form.
    /// Processes whose path can't be read (e.g. protected system processes) are omitted.
    fn processes(&self) -> Vec<(u32, String)>;
    fn is_alive(&self, pid: u32) -> bool;
}

/// The real process table.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcesses;

/// Lower-cased, backslash-separated path, for comparisons (Windows paths are case-insensitive).
pub fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// `normalize_path` plus a trailing backslash, for "is inside this folder" prefix checks.
pub fn normalize_dir(dir: &Path) -> String {
    let mut s = normalize_path(dir);
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;

    /// Closes the handle on drop.
    pub struct Owned(pub HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: we own this handle and close it exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Owned> {
        // SAFETY: plain FFI call; the returned handle is owned by `Owned`.
        unsafe { OpenProcess(access, false, pid).ok().map(Owned) }
    }

    pub fn all_pids() -> Vec<u32> {
        let mut out = Vec::new();
        // SAFETY: the snapshot handle is owned by `Owned`; `entry.dwSize` is set as the API requires.
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let snap = Owned(snap);
            let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            if Process32FirstW(snap.0, &mut entry).is_ok() {
                loop {
                    out.push(entry.th32ProcessID);
                    if Process32NextW(snap.0, &mut entry).is_err() {
                        break;
                    }
                }
            }
        }
        out
    }

    pub fn image_path(pid: u32) -> Option<String> {
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` and `len` describe a valid writable buffer.
        unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()? };
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn is_alive(pid: u32) -> bool {
        const STILL_ACTIVE: u32 = 259;
        let Some(h) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return false };
        let mut code = 0u32;
        // SAFETY: valid handle and out-param.
        unsafe { GetExitCodeProcess(h.0, &mut code).is_ok() && code == STILL_ACTIVE }
    }
}

#[cfg(windows)]
impl ProcessSource for SystemProcesses {
    fn processes(&self) -> Vec<(u32, String)> {
        let own = std::process::id();
        win::all_pids()
            .into_iter()
            .filter(|&pid| pid != 0 && pid != own)
            .filter_map(|pid| Some((pid, normalize_path(Path::new(&win::image_path(pid)?)))))
            .collect()
    }

    fn is_alive(&self, pid: u32) -> bool {
        win::is_alive(pid)
    }
}

#[cfg(not(windows))]
impl ProcessSource for SystemProcesses {
    fn processes(&self) -> Vec<(u32, String)> {
        Vec::new()
    }

    fn is_alive(&self, _pid: u32) -> bool {
        false
    }
}

/// Forcefully ends a process (used by "Close game" after the user confirms).
#[cfg(windows)]
pub fn terminate(pid: u32) -> io::Result<()> {
    use windows::Win32::System::Threading::{PROCESS_TERMINATE, TerminateProcess};
    let h = win::open(pid, PROCESS_TERMINATE).ok_or_else(io::Error::last_os_error)?;
    // SAFETY: valid handle opened with PROCESS_TERMINATE.
    unsafe { TerminateProcess(h.0, 1) }.map_err(|e| io::Error::other(e.to_string()))
}

#[cfg(not(windows))]
pub fn terminate(_pid: u32) -> io::Result<()> {
    Err(crate::unsupported())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize() {
        assert_eq!(normalize_dir(Path::new("C:/Games/Foo")), r"c:\games\foo\");
        assert_eq!(normalize_dir(Path::new(r"C:\Games\Foo\")), r"c:\games\foo\");
        assert_eq!(normalize_path(Path::new("C:/Games/Foo.EXE")), r"c:\games\foo.exe");
    }
}
