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
const ACCELERATION: f32 = 0.2;

/// How long (in ms) between scrolls before momentum resets.
/// Increase to allow longer pauses between scrolls while keeping momentum.
/// Decrease to require faster, more continuous scrolling to maintain momentum.
const DECAY_TIME: Duration = Duration::from_millis(200);

/// Only process 1 out of every N scroll events (throttling).
/// Increase to make scrolling less sensitive to high-frequency mouse wheels
/// (e.g. trackpads). Decrease to process more events (smoother, but may be
/// too fast on some input devices).
const THROTTLE_FACTOR: u8 = 3;

/// Core struct to track and compute momentum-based scrolling.
pub struct ScrollMomentum {
    velocity: f32,
    last_event: Option<Instant>,
    phase: Phase,
    last_direction: Option<Direction>,
    throttle_counter: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Accelerating,
}

impl ScrollMomentum {
    pub fn new() -> Self {
        Self {
            velocity: 0.0,
            last_event: None,
            phase: Phase::Idle,
            last_direction: None,
            throttle_counter: 0,
        }
    }

    /// Call this on every scroll event (mouse wheel, key, etc).
    /// Returns the number of lines to scroll for this event.
    pub fn on_scroll_event(&mut self, direction: Direction) -> usize {
        // Throttle: only process 1 out of every THROTTLE_FACTOR events
        self.throttle_counter = (self.throttle_counter + 1) % THROTTLE_FACTOR;
        if self.throttle_counter != 0 {
            return 0;
        }

        let now = Instant::now();

        // Reset momentum if direction changes
        let direction_changed = self.last_direction.map_or(false, |last| last != direction);
        if direction_changed {
            self.velocity = MIN_VELOCITY;
            self.phase = Phase::Idle;
            self.last_event = Some(now);
        }

        let lines_to_scroll = if let Some(last) = self.last_event {
            let dt = now.duration_since(last);
            if dt < DECAY_TIME && !direction_changed {
                // User is scrolling fast, accelerate
                self.phase = Phase::Accelerating;
                self.velocity = (self.velocity + ACCELERATION).min(MAX_VELOCITY);
                self.velocity.round() as usize
            } else {
                // Too slow, reset
                self.phase = Phase::Idle;
                self.velocity = MIN_VELOCITY;
                MIN_VELOCITY as usize
            }
        } else {
            self.phase = Phase::Accelerating;
            self.velocity = MIN_VELOCITY;
            MIN_VELOCITY as usize
        };

        self.last_event = Some(now);
        self.last_direction = Some(direction);
        lines_to_scroll
    }

    /// Call this to reset the momentum (e.g. on focus loss or scroll stop)
    pub fn reset(&mut self) {
        self.velocity = 0.0;
        self.last_event = None;
        self.phase = Phase::Idle;
        self.last_direction = None;
        self.throttle_counter = 0;
    }
}
