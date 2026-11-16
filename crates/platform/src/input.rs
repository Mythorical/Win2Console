//! Synthetic keyboard and mouse input, sent to whatever window is in the foreground: the virtual keyboard
//! types into games and apps, and the controller drives the mouse on the desktop. (Windows doesn't let a
//! normal app send input to an app running as administrator.)

/// Virtual-key codes used by the virtual keyboard.
pub const VK_BACK: u16 = 0x08;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_LEFT: u16 = 0x25;
pub const VK_RIGHT: u16 = 0x27;

#[cfg(windows)]
mod imp {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        MOUSE_EVENT_FLAGS, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    };

    fn send(inputs: &[INPUT]) {
        // SAFETY: a valid slice of fully initialised INPUT structures.
        unsafe {
            SendInput(inputs, size_of::<INPUT>() as i32);
        }
    }

    fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    fn mouse_input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    pub fn type_text(text: &str) {
        let mut inputs = Vec::new();
        for unit in text.encode_utf16() {
            inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
            inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        send(&inputs);
    }

    pub fn press_key(vk: u16) {
        send(&[key_input(vk, 0, KEYBD_EVENT_FLAGS(0)), key_input(vk, 0, KEYEVENTF_KEYUP)]);
    }

    pub fn mouse_move(dx: i32, dy: i32) {
        send(&[mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE)]);
    }

    pub fn mouse_button(right: bool, down: bool) {
        let flags = match (right, down) {
            (false, true) => MOUSEEVENTF_LEFTDOWN,
            (false, false) => MOUSEEVENTF_LEFTUP,
            (true, true) => MOUSEEVENTF_RIGHTDOWN,
            (true, false) => MOUSEEVENTF_RIGHTUP,
        };
        send(&[mouse_input(0, 0, 0, flags)]);
    }

    pub fn wheel(vertical: i32, horizontal: i32) {
        let mut inputs = Vec::new();
        if vertical != 0 {
            inputs.push(mouse_input(0, 0, vertical, MOUSEEVENTF_WHEEL));
        }
        if horizontal != 0 {
            inputs.push(mouse_input(0, 0, horizontal, MOUSEEVENTF_HWHEEL));
        }
        send(&inputs);
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn type_text(_text: &str) {}
    pub fn press_key(_vk: u16) {}
    pub fn mouse_move(_dx: i32, _dy: i32) {}
    pub fn mouse_button(_right: bool, _down: bool) {}
    pub fn wheel(_vertical: i32, _horizontal: i32) {}
}

/// Types `text` as if from a keyboard (any characters, not just the keyboard layout's).
pub fn type_text(text: &str) {
    imp::type_text(text);
}

/// Presses and releases one key (`VK_*`).
pub fn press_key(vk: u16) {
    imp::press_key(vk);
}

/// Moves the pointer by (`dx`, `dy`) pixels.
pub fn mouse_move(dx: i32, dy: i32) {
    imp::mouse_move(dx, dy);
}

pub fn mouse_button(right: bool, down: bool) {
    imp::mouse_button(right, down);
}

/// Scrolls; 120 is one wheel notch (positive = up / right).
pub fn wheel(vertical: i32, horizontal: i32) {
    imp::wheel(vertical, horizontal);
}
