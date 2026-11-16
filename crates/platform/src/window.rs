//! Bringing Win2Console back to the foreground (e.g. when a game exits), like a console returning to
//! its dashboard.

/// Restores and focuses this process's main window.
///
/// Windows only lets the process that owns the foreground window move focus elsewhere, so we briefly
/// attach our input queue to the foreground thread (the standard workaround) before `SetForegroundWindow`.
#[cfg(windows)]
pub fn bring_own_window_to_front() {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, SW_RESTORE,
        SetForegroundWindow, ShowWindow,
    };
    use windows::core::BOOL;

    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the `&mut Search` passed to EnumWindows below, alive for the whole call.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut pid = 0u32;
        // SAFETY: valid window handle from EnumWindows and a valid out-param.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // SAFETY: valid window handle.
        if pid == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            search.found = Some(hwnd);
            return BOOL(0); // stop
        }
        BOOL(1)
    }

    let mut search = Search { pid: std::process::id(), found: None };
    // SAFETY: the callback only dereferences `search`, which outlives the call. EnumWindows reports
    // an "error" when the callback stops early, which is expected here.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    let Some(hwnd) = search.found else {
        tracing::warn!("bring_own_window_to_front: window not found");
        return;
    };

    // SAFETY: plain Win32 calls on valid handles/thread ids; attach is always paired with detach.
    unsafe {
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let our_thread = GetCurrentThreadId();
        let attached = fg_thread != 0
            && fg_thread != our_thread
            && windows::Win32::System::Threading::AttachThreadInput(our_thread, fg_thread, true).as_bool();
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = windows::Win32::System::Threading::AttachThreadInput(our_thread, fg_thread, false);
        }
    }
}

#[cfg(not(windows))]
pub fn bring_own_window_to_front() {}

/// Whether one of this process's windows is in the foreground.
#[cfg(windows)]
pub fn own_window_is_foreground() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: plain Win32 queries.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid == std::process::id()
    }
}

#[cfg(not(windows))]
pub fn own_window_is_foreground() -> bool {
    true
}

/// Minimises this process's main window (the in-game Guide's "Resume").
#[cfg(windows)]
pub fn minimize_own_window() {
    use windows::Win32::UI::WindowsAndMessaging::{SW_MINIMIZE, ShowWindow};
    if let Some(hwnd) = find_window(std::process::id()) {
        // SAFETY: valid window handle.
        unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
    }
}

#[cfg(not(windows))]
pub fn minimize_own_window() {}

/// Brings the main window of any of `pids` (the game) to the foreground.
#[cfg(windows)]
pub fn focus_process(pids: &[u32]) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, SetForegroundWindow, ShowWindow};
    for &pid in pids {
        if let Some(hwnd) = find_window(pid) {
            // SAFETY: valid window handle.
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
pub fn focus_process(_pids: &[u32]) -> bool {
    false
}

/// First visible top-level window of a process.
#[cfg(windows)]
fn find_window(target: u32) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};
    use windows::core::BOOL;

    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam points at the `Search` below for the duration of EnumWindows.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut pid = 0u32;
        // SAFETY: valid handles from EnumWindows.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == search.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            search.found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search { pid: target, found: None };
    // SAFETY: the callback only touches `search`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}

/// Global Guide hotkeys: Ctrl+Shift+G always, and Home while a game is running (`set_home`). `on_press` runs on
/// the hotkey thread.
pub struct GuideHotkey {
    #[cfg_attr(not(windows), allow(dead_code))]
    thread_id: u32,
}

#[cfg(windows)]
const WM_HOME_OFF: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP;
#[cfg(windows)]
const WM_HOME_ON: u32 = WM_HOME_OFF + 1;

#[cfg(windows)]
pub fn spawn_guide_hotkey(on_press: impl Fn() + Send + 'static) -> GuideHotkey {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_HOME,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY};

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("guide-hotkey".into())
        .spawn(move || {
            // SAFETY: hotkeys belong to this thread's message queue (created by PeekMessage), drained below.
            unsafe {
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                let _ = tx.send(GetCurrentThreadId());
                if let Err(e) = RegisterHotKey(None, 1, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, u32::from(b'G')) {
                    tracing::warn!("couldn't register the Guide hotkey (Ctrl+Shift+G): {e}");
                }
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    match msg.message {
                        WM_HOTKEY => on_press(),
                        WM_HOME_ON => {
                            if let Err(e) = RegisterHotKey(None, 2, MOD_NOREPEAT, u32::from(VK_HOME.0)) {
                                tracing::warn!("couldn't register Home as the Guide key: {e}");
                            }
                        }
                        WM_HOME_OFF => {
                            let _ = UnregisterHotKey(None, 2);
                        }
                        _ => {}
                    }
                }
            }
        })
        .expect("spawn hotkey thread");
    GuideHotkey { thread_id: rx.recv().unwrap_or(0) }
}

#[cfg(windows)]
impl GuideHotkey {
    pub fn set_home(&self, on: bool) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        // SAFETY: posting a plain message to our hotkey thread.
        let _ = unsafe {
            PostThreadMessageW(self.thread_id, if on { WM_HOME_ON } else { WM_HOME_OFF }, WPARAM(0), LPARAM(0))
        };
    }
}

#[cfg(not(windows))]
pub fn spawn_guide_hotkey(_on_press: impl Fn() + Send + 'static) -> GuideHotkey {
    GuideHotkey { thread_id: 0 }
}

#[cfg(not(windows))]
impl GuideHotkey {
    pub fn set_home(&self, _on: bool) {}
}

/// Primary screen size in physical pixels.
#[cfg(windows)]
pub fn screen_size() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    // SAFETY: plain queries.
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

#[cfg(not(windows))]
pub fn screen_size() -> (i32, i32) {
    (1920, 1080)
}

/// Turns a window into a notification overlay: always on top, never takes focus, ignores the mouse, and has
/// no taskbar button. Shows it without activating it.
#[cfg(windows)]
pub fn make_overlay(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        SetWindowLongPtrW, SetWindowPos, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: valid top-level window handle owned by this process.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_TRANSPARENT.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | add);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

#[cfg(not(windows))]
pub fn make_overlay(_hwnd: isize) {}
