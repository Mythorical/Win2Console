//! Key repeat for held directions and analog-stick-to-direction conversion.

use std::time::{Duration, Instant};

use crate::NavAction;

pub const REPEAT_DELAY: Duration = Duration::from_millis(400);
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(80);

/// Repeats the most recently pressed held direction.
#[derive(Debug, Default)]
pub struct Repeater {
    held: Option<(NavAction, Instant)>,
}

impl Repeater {
    /// Call on press. Directions start repeating after `REPEAT_DELAY`.
    pub fn press(&mut self, action: NavAction, now: Instant) {
        if action.is_direction() {
            self.held = Some((action, now + REPEAT_DELAY));
        }
    }

    /// Call on release.
    pub fn release(&mut self, action: NavAction) {
        if self.held.is_some_and(|(a, _)| a == action) {
            self.held = None;
        }
    }

    /// Returns a repeated action if one is due.
    pub fn tick(&mut self, now: Instant) -> Option<NavAction> {
        let (action, next) = self.held.as_mut()?;
        if now >= *next {
            *next = now + REPEAT_INTERVAL;
            Some(*action)
        } else {
            None
        }
    }

    /// How long the input thread may sleep before the next repeat is due.
    pub fn next_due(&self, now: Instant) -> Option<Duration> {
        self.held.map(|(_, next)| next.saturating_duration_since(now))
    }
}

/// Converts an analog stick into a single held direction, with hysteresis so it doesn't flicker.
#[derive(Debug, Default)]
pub struct StickState {
    x: f32,
    y: f32,
    current: Option<NavAction>,
}

/// Fraction of full deflection needed to register a direction, and to release it.
const PRESS_THRESHOLD: f32 = 0.5;
const RELEASE_THRESHOLD: f32 = 0.35;

impl StickState {
    /// Updates one axis (`value` in -1.0..=1.0, y positive = down, as SDL reports).
    /// Returns `(released, pressed)` direction changes.
    pub fn set_axis(&mut self, is_x: bool, value: f32) -> (Option<NavAction>, Option<NavAction>) {
        if is_x {
            self.x = value;
        } else {
            self.y = value;
        }
        let magnitude = self.x.abs().max(self.y.abs());
        let wanted = if (self.current.is_some() && magnitude >= RELEASE_THRESHOLD) || magnitude >= PRESS_THRESHOLD {
            Some(if self.x.abs() >= self.y.abs() {
                if self.x > 0.0 { NavAction::Right } else { NavAction::Left }
            } else if self.y > 0.0 {
                NavAction::Down
            } else {
                NavAction::Up
            })
        } else {
            None
        };
        if wanted == self.current {
            return (None, None);
        }
        let released = self.current;
        self.current = wanted;
        (released, wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_timing() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(NavAction::Down, t0);
        assert_eq!(r.tick(t0 + Duration::from_millis(399)), None);
        assert_eq!(r.tick(t0 + REPEAT_DELAY), Some(NavAction::Down));
        assert_eq!(r.tick(t0 + REPEAT_DELAY + Duration::from_millis(79)), None);
        assert_eq!(r.tick(t0 + REPEAT_DELAY + REPEAT_INTERVAL), Some(NavAction::Down));
        r.release(NavAction::Down);
        assert_eq!(r.tick(t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn buttons_do_not_repeat_and_release_only_matching() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(NavAction::Accept, t0);
        assert_eq!(r.tick(t0 + Duration::from_secs(1)), None);
        r.press(NavAction::Left, t0);
        r.release(NavAction::Right);
        assert_eq!(r.tick(t0 + REPEAT_DELAY), Some(NavAction::Left));
    }

    #[test]
    fn stick_hysteresis() {
        let mut s = StickState::default();
        assert_eq!(s.set_axis(true, 0.4), (None, None));
        assert_eq!(s.set_axis(true, 0.6), (None, Some(NavAction::Right)));
        assert_eq!(s.set_axis(true, 0.4), (None, None)); // still above release threshold
        assert_eq!(s.set_axis(true, 0.2), (Some(NavAction::Right), None));
        assert_eq!(s.set_axis(false, -0.9), (None, Some(NavAction::Up)));
        assert_eq!(s.set_axis(true, 0.95), (Some(NavAction::Up), Some(NavAction::Right)));
    }
}
