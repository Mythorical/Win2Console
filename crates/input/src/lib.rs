//! Controller input. An SDL3 thread turns gamepad events from any controller into [`NavAction`]s,
//! with key repeat for held directions. See `docs/ARCHITECTURE.md` for the button mapping.

mod repeat;
mod sdl;

pub use repeat::{Repeater, StickState};
pub use sdl::{InputEvent, spawn};

/// A navigation action, independent of the device that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavAction {
    Up,
    Down,
    Left,
    Right,
    /// A / Cross
    Accept,
    /// B / Circle
    Back,
    /// X / Square
    X,
    /// Y / Triangle
    Y,
    /// Menu / Options / Start
    Menu,
    /// View / Share / Back (Select)
    View,
    /// LB / L1
    PrevTab,
    /// RB / R1
    NextTab,
    /// Guide / Xbox / PS button
    Guide,
}

impl NavAction {
    pub fn is_direction(self) -> bool {
        matches!(self, NavAction::Up | NavAction::Down | NavAction::Left | NavAction::Right)
    }
}

/// Status of one connected controller, for the top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerInfo {
    pub name: String,
    /// Battery percentage when the controller reports one (wireless pads); `None` for wired.
    pub battery_percent: Option<u8>,
}
