use std::time::{Duration, Instant};

use crate::tui::event::Direction;

const MAX_VELOCITY: f32 = 12.0; // max lines per event
const MIN_VELOCITY: f32 = 1.0; // min lines per event
const ACCELERATION: f32 = 2.0; // lines per event per fast scroll
const DECAY_TIME: Duration = Duration::from_millis(200); // ms to reset momentum

/// Core struct to track and compute momentum-based scrolling.
pub struct ScrollMomentum {
    velocity: f32,
    last_event: Option<Instant>,
    phase: Phase,
    last_direction: Option<Direction>,
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
        }
    }

    /// Call this on every scroll event (mouse wheel, key, etc).
    /// Returns the number of lines to scroll for this event.
    pub fn on_scroll_event(&mut self, direction: Direction) -> usize {
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
    }
}
