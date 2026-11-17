//! Bringing Win2Console back to the foreground (e.g. when a game exits), like a console returning to
//! its dashboard.

/// The main window, once it exists. Other windows of ours (pop-ups, the keyboard) must not be mistaken for it.
static MAIN_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

pub fn set_main_window(hwnd: isize) {
    MAIN_WINDOW.store(hwnd, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(windows)]
fn own_window() -> Option<windows::Win32::Foundation::HWND> {
    match MAIN_WINDOW.load(std::sync::atomic::Ordering::Relaxed) {
        0 => find_window(std::process::id()),
        h => Some(windows::Win32::Foundation::HWND(h as *mut _)),
    }
}

/// Restores and focuses this process's main window.
///
/// Windows only lets the process that owns the foreground window move focus elsewhere, so we briefly
/// attach our input queue to the foreground thread (the standard workaround) before `SetForegroundWindow`.
/// Controller input doesn't count as user input to Windows, so if that's still refused we tap Alt, which
/// lifts the foreground lock, and try again.
#[cfg(windows)]
pub fn bring_own_window_to_front() {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    let Some(hwnd) = own_window() else {
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
        if GetForegroundWindow() != hwnd {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_MENU,
            };
            let key = |flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
            };
            SendInput(&[key(KEYBD_EVENT_FLAGS(0))], size_of::<INPUT>() as i32);
            let _ = SetForegroundWindow(hwnd);
            SendInput(&[key(KEYEVENTF_KEYUP)], size_of::<INPUT>() as i32);
            let _ = BringWindowToTop(hwnd);
        }
    }
}

#[cfg(not(windows))]
pub fn bring_own_window_to_front() {}

/// Hides the main window from the screen without minimising it (DWM cloaking), so it can be brought to the
/// front and repainted before anyone sees it: a restored window first shows its last frame (the dashboard).
static CLOAKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
pub fn cloak_own_window(cloak: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let Some(hwnd) = own_window() else { return };
    CLOAKED.store(cloak, std::sync::atomic::Ordering::Relaxed);
    let value = windows::core::BOOL::from(cloak);
    // SAFETY: valid window handle and a BOOL-sized attribute value.
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &value as *const _ as *const _, size_of_val(&value) as u32);
    }
}

#[cfg(not(windows))]
pub fn cloak_own_window(cloak: bool) {
    CLOAKED.store(cloak, std::sync::atomic::Ordering::Relaxed);
}

pub fn own_window_is_cloaked() -> bool {
    CLOAKED.load(std::sync::atomic::Ordering::Relaxed)
}

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
    if let Some(hwnd) = own_window() {
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

/// The window in front (to give focus back to it later).
#[cfg(windows)]
pub fn foreground_window() -> isize {
    // SAFETY: plain Win32 query.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as isize }
}

#[cfg(not(windows))]
pub fn foreground_window() -> isize {
    0
}

/// Puts a window (from [`foreground_window`]) back in front; false if it's gone or is one of ours.
#[cfg(windows)]
pub fn set_foreground_window(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, SetForegroundWindow};
    let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut _);
    // SAFETY: plain Win32 calls; they tolerate stale handles.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if hwnd.0.is_null() || !IsWindow(Some(hwnd)).as_bool() || pid == std::process::id() {
            return false;
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

#[cfg(not(windows))]
pub fn set_foreground_window(_hwnd: isize) -> bool {
    false
}

/// Clips a window to a rounded rectangle (e.g. the pill-shaped achievement pop-up).
#[cfg(windows)]
pub fn round_window(hwnd: isize, width: i32, height: i32, radius: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    // SAFETY: valid window handle; the window takes ownership of the region.
    unsafe {
        let rgn = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2);
        SetWindowRgn(HWND(hwnd as *mut _), Some(rgn), true);
    }
}

#[cfg(not(windows))]
pub fn round_window(_hwnd: isize, _width: i32, _height: i32, _radius: i32) {}
