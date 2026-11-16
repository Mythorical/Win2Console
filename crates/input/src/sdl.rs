//! SDL3 gamepad backend. Runs on its own thread (SDL treats the first thread that initialises it as its
//! "main" thread; Slint never touches SDL, so there is no conflict).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sdl3::GamepadSubsystem;
use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button, Gamepad};
use sdl3::joystick::{JoystickId, PowerLevel};

use crate::repeat::{Repeater, StickState};
use crate::{ControllerInfo, NavAction};

/// What the input thread reports to the UI.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Nav(NavAction),
    /// The set of connected controllers (or their battery levels) changed.
    Controllers(Vec<ControllerInfo>),
    /// Mouse mode only (see [`set_mouse_mode`]).
    Mouse(MouseEvent),
}

/// The controller as a mouse: left stick moves, right stick scrolls, A clicks, X right-clicks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Move(i32, i32),
    Button { right: bool, down: bool },
    /// Wheel units (120 = one notch); vertical, horizontal.
    Wheel(i32, i32),
}

static MOUSE_MODE: AtomicBool = AtomicBool::new(false);

/// Turns the controller into a mouse (for the desktop). Other buttons, like the Guide, still report as usual.
pub fn set_mouse_mode(on: bool) {
    MOUSE_MODE.store(on, Ordering::Relaxed);
}

fn mouse_mode() -> bool {
    MOUSE_MODE.load(Ordering::Relaxed)
}

/// How often the pointer moves in mouse mode.
const MOUSE_TICK: Duration = Duration::from_millis(8);

/// How often battery levels are re-read.
const BATTERY_POLL: Duration = Duration::from_secs(30);
/// Longest wait between loop iterations when nothing is happening.
const IDLE_WAIT: Duration = Duration::from_millis(250);

/// Starts the controller thread. `sink` is called from that thread for every event.
/// If SDL can't start, the error is logged and the app keeps working with keyboard and mouse.
pub fn spawn(sink: impl Fn(InputEvent) + Send + 'static) {
    std::thread::Builder::new()
        .name("controller-input".into())
        .spawn(move || {
            if let Err(e) = run(&sink) {
                tracing::error!("controller input unavailable: {e}");
            }
        })
        .expect("spawn controller thread");
}

fn map_button(b: Button) -> Option<NavAction> {
    Some(match b {
        Button::DPadUp => NavAction::Up,
        Button::DPadDown => NavAction::Down,
        Button::DPadLeft => NavAction::Left,
        Button::DPadRight => NavAction::Right,
        // SDL names buttons by position, so PlayStation/Nintendo pads map by layout, not label.
        Button::South => NavAction::Accept,
        Button::East => NavAction::Back,
        Button::West => NavAction::X,
        Button::North => NavAction::Y,
        Button::Start => NavAction::Menu,
        Button::Back => NavAction::View,
        Button::LeftShoulder => NavAction::PrevTab,
        Button::RightShoulder => NavAction::NextTab,
        Button::Guide => NavAction::Guide,
        _ => return None,
    })
}

fn run(sink: &dyn Fn(InputEvent)) -> Result<(), Box<dyn std::error::Error>> {
    // We have no SDL window, so SDL would otherwise drop events whenever it thinks the app is unfocused.
    sdl3::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
    let sdl = sdl3::init()?;
    let gamepads: GamepadSubsystem = sdl.gamepad()?;
    let mut pump = sdl.event_pump()?;
    tracing::info!("controller input started");

    let mut open: HashMap<JoystickId, Gamepad> = HashMap::new();
    let mut sticks: HashMap<JoystickId, StickState> = HashMap::new();
    let mut repeater = Repeater::default();
    let mut last_battery = Instant::now();
    let mut last_reported: Vec<ControllerInfo> = Vec::new();

    let mut report = |open: &HashMap<JoystickId, Gamepad>, force: bool| {
        let mut list: Vec<ControllerInfo> = open.values().map(info).collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        if force || list != last_reported {
            last_reported = list.clone();
            sink(InputEvent::Controllers(list));
        }
    };
    report(&open, true);

    let mut pointer = Pointer::default();
    loop {
        let now = Instant::now();
        let mut wait = repeater.next_due(now).unwrap_or(IDLE_WAIT).min(IDLE_WAIT);
        if mouse_mode() {
            wait = wait.min(MOUSE_TICK);
        }
        if let Some(event) = pump.wait_event_timeout(wait) {
            handle(event, &gamepads, &mut open, &mut sticks, &mut repeater, sink, &mut report);
            // Drain anything else that's queued before checking repeats.
            while let Some(event) = pump.poll_event() {
                handle(event, &gamepads, &mut open, &mut sticks, &mut repeater, sink, &mut report);
            }
        }
        if let Some(action) = repeater.tick(Instant::now()) {
            sink(InputEvent::Nav(action));
        }
        if mouse_mode() {
            pointer.tick(&open, sink);
        }
        if last_battery.elapsed() >= BATTERY_POLL {
            last_battery = Instant::now();
            report(&open, false);
        }
    }
}

fn handle(
    event: Event,
    gamepads: &GamepadSubsystem,
    open: &mut HashMap<JoystickId, Gamepad>,
    sticks: &mut HashMap<JoystickId, StickState>,
    repeater: &mut Repeater,
    sink: &dyn Fn(InputEvent),
    report: &mut dyn FnMut(&HashMap<JoystickId, Gamepad>, bool),
) {
    match event {
        Event::GamepadAdded { which, .. } => match gamepads.open(which) {
            Ok(pad) => {
                tracing::info!("controller connected: {}", pad.name().unwrap_or_default());
                open.insert(which, pad);
                report(open, false);
            }
            Err(e) => tracing::warn!("can't open controller {which}: {e}"),
        },
        Event::GamepadRemoved { which, .. } => {
            if let Some(pad) = open.remove(&which) {
                tracing::info!("controller disconnected: {}", pad.name().unwrap_or_default());
            }
            sticks.remove(&which);
            report(open, false);
        }
        Event::GamepadButtonDown { button: b @ (Button::South | Button::West), .. } if mouse_mode() => {
            sink(InputEvent::Mouse(MouseEvent::Button { right: b == Button::West, down: true }));
        }
        Event::GamepadButtonUp { button: b @ (Button::South | Button::West), .. } if mouse_mode() => {
            sink(InputEvent::Mouse(MouseEvent::Button { right: b == Button::West, down: false }));
        }
        Event::GamepadButtonDown { button, .. } => {
            if let Some(action) = map_button(button) {
                repeater.press(action, Instant::now());
                sink(InputEvent::Nav(action));
            }
        }
        Event::GamepadButtonUp { button, .. } => {
            if let Some(action) = map_button(button) {
                repeater.release(action);
            }
        }
        Event::GamepadAxisMotion { .. } if mouse_mode() => {}
        Event::GamepadAxisMotion { which, axis, value, .. } => {
            let is_x = match axis {
                Axis::LeftX => true,
                Axis::LeftY => false,
                _ => return,
            };
            let stick = sticks.entry(which).or_default();
            let (released, pressed) = stick.set_axis(is_x, value as f32 / i16::MAX as f32);
            if let Some(a) = released {
                repeater.release(a);
            }
            if let Some(a) = pressed {
                repeater.press(a, Instant::now());
                sink(InputEvent::Nav(a));
            }
        }
        _ => {}
    }
}

/// Mouse mode's pointer: stick deflection (past a dead zone, on a curve) becomes speed; fractions carry over.
#[derive(Default)]
struct Pointer {
    last: Option<Instant>,
    rest: (f32, f32, f32, f32),
}

impl Pointer {
    fn tick(&mut self, open: &HashMap<JoystickId, Gamepad>, sink: &dyn Fn(InputEvent)) {
        let now = Instant::now();
        if self.last.is_some_and(|t| now - t < MOUSE_TICK) {
            return;
        }
        let dt = self.last.map_or(0.0, |t| (now - t).as_secs_f32().min(0.05));
        self.last = Some(now);
        // The stick pushed furthest across all controllers.
        let pick = |axis: Axis| {
            open.values().map(|p| p.axis(axis) as f32 / i16::MAX as f32).fold(0.0f32, |m, v| if v.abs() > m.abs() { v } else { m })
        };
        let curve = |v: f32| {
            const DEAD: f32 = 0.15;
            if v.abs() < DEAD { 0.0 } else { v.signum() * ((v.abs() - DEAD) / (1.0 - DEAD)).powf(2.2) }
        };
        // Pixels per second at full tilt; scroll in wheel units per second.
        let (dx, dy) = (curve(pick(Axis::LeftX)) * 1800.0 * dt, curve(pick(Axis::LeftY)) * 1800.0 * dt);
        let (sx, sy) = (curve(pick(Axis::RightX)) * 900.0 * dt, -curve(pick(Axis::RightY)) * 900.0 * dt);
        let r = &mut self.rest;
        let (mx, my, wx, wy) = (r.0 + dx, r.1 + dy, r.2 + sx, r.3 + sy);
        let (ix, iy, iwx, iwy) = (mx.trunc(), my.trunc(), wx.trunc(), wy.trunc());
        *r = (mx - ix, my - iy, wx - iwx, wy - iwy);
        if ix != 0.0 || iy != 0.0 {
            sink(InputEvent::Mouse(MouseEvent::Move(ix as i32, iy as i32)));
        }
        if iwx != 0.0 || iwy != 0.0 {
            sink(InputEvent::Mouse(MouseEvent::Wheel(iwy as i32, iwx as i32)));
        }
    }
}

fn info(pad: &Gamepad) -> ControllerInfo {
    let power = pad.power_info();
    tracing::debug!("controller {:?}: power {:?} {}%", pad.name(), power.state, power.percentage);
    let battery_percent = match power.state {
        PowerLevel::OnBattery | PowerLevel::Charging if (0..=100).contains(&power.percentage) => {
            Some(power.percentage as u8)
        }
        _ => None,
    };
    ControllerInfo { name: pad.name().unwrap_or_else(|| "Controller".into()), battery_percent }
}
