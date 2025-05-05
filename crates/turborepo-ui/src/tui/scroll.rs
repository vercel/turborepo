use std::time::{Duration, Instant};

use crate::tui::event::Direction;

/// The maximum number of lines that can be scrolled per event.
/// Increase for a higher top speed; decrease for a lower top speed.
const MAX_VELOCITY: f32 = 12.0; // max lines per event

/// The minimum number of lines to scroll per event (when not accelerating).
/// Usually leave at 1.0 for single-line scrolls.
const MIN_VELOCITY: f32 = 1.0;

/// How much the scroll velocity increases per qualifying event.
/// Increase for faster acceleration (reaches top speed quicker, feels
/// snappier). Decrease for slower, smoother acceleration (takes longer to reach
/// top speed).
const ACCELERATION: f32 = 0.3;

/// How long (in ms) between scrolls before momentum resets.
/// Increase to allow longer pauses between scrolls while keeping momentum.
/// Decrease to require faster, more continuous scrolling to maintain momentum.
const DECAY_TIME: Duration = Duration::from_millis(350);

/// How long (in ms) between scrolls before events are ignored
/// Increase to allow longer pauses between scrolls to trigger throttling
/// Decrease to require faster, more continuous scrolling to trigger throttling
const THROTTLE_TIME: Duration = Duration::from_millis(50);

/// Only process 1 out of every N scroll events (throttling).
/// Increase to make scrolling less sensitive to high-frequency mouse wheels
/// (e.g. trackpads). Decrease to process more events (smoother, but may be
/// too fast on some input devices).
const THROTTLE_FACTOR: u8 = 3;

/// Tracks and computes momentum-based scrolling.
pub struct ScrollMomentum {
    velocity: f32,
    last_event: Option<Instant>,
    last_direction: Option<Direction>,
    throttle_counter: u8,
}

impl Default for ScrollMomentum {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollMomentum {
    /// Create a new ScrollMomentum tracker.
    pub fn new() -> Self {
        Self {
            velocity: 0.0,
            last_event: None,
            last_direction: None,
            throttle_counter: 0,
        }
    }

    /// Call this on every scroll event (mouse wheel, key, etc).
    /// Returns the number of lines to scroll for this event.
    pub fn on_scroll_event(&mut self, direction: Direction) -> usize {
        let now = Instant::now();
        let last_event = self.last_event.replace(now);
        let last_direction = self.last_direction.replace(direction);

        let has_direction_changed = last_direction.is_some_and(|last| last != direction);
        let is_scrolling_quickly =
            last_event.is_some_and(|last| now.duration_since(last) < DECAY_TIME);
        let is_throttling = last_event.is_some_and(|last| now.duration_since(last) < THROTTLE_TIME);

        if is_throttling {
            self.throttle_counter = (self.throttle_counter + 1) % THROTTLE_FACTOR;
            let should_throttle = self.throttle_counter != 0;
            if should_throttle {
                return 0;
            }
        } else {
            self.throttle_counter = 0;
        }

        if has_direction_changed {
            self.velocity = MIN_VELOCITY;
        } else if is_scrolling_quickly {
            self.velocity = (self.velocity + ACCELERATION).min(MAX_VELOCITY);
        } else {
            self.velocity = MIN_VELOCITY;
        }

        self.velocity.round().clamp(MIN_VELOCITY, MAX_VELOCITY) as usize
    }

    /// Reset the momentum (e.g. on focus loss or scroll stop)
    pub fn reset(&mut self) {
        self.velocity = 0.0;
        self.last_event = None;
        self.last_direction = None;
        self.throttle_counter = 0;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn first_event_scrolls() {
        let mut scroll = ScrollMomentum::new();
        assert_eq!(scroll.on_scroll_event(Direction::Up), 1);
    }
}
