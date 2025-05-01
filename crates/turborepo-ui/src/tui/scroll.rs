use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::tui::event::Direction;

const MAX_VELOCITY: f32 = 12.0; // max lines per event
const MIN_VELOCITY: f32 = 1.0; // min lines per event
const ACCELERATION: f32 = 2.0; // lines per event per fast scroll
const DECAY_TIME: Duration = Duration::from_millis(200); // ms to reset momentum
const THROTTLE_FACTOR: u8 = 3; // Only process 1 out of every 3 events
const WINDOW_SIZE: usize = 3; // Number of scrolls to trigger acceleration
const WINDOW_TIME: Duration = Duration::from_millis(250); // Time window for acceleration

/// Core struct to track and compute momentum-based scrolling.
pub struct ScrollMomentum {
    velocity: f32,
    last_event: Option<Instant>,
    phase: Phase,
    last_direction: Option<Direction>,
    throttle_counter: u8,
    recent_events: VecDeque<Instant>,
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
            recent_events: VecDeque::with_capacity(WINDOW_SIZE),
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
            self.recent_events.clear();
        }

        // Update moving window of recent scrolls
        self.recent_events.push_back(now);
        if self.recent_events.len() > WINDOW_SIZE {
            self.recent_events.pop_front();
        }

        let reached_window_size = self.recent_events.len() == WINDOW_SIZE;
        let within_window_time =
            now.duration_since(self.recent_events.front().copied().unwrap()) <= WINDOW_TIME;
        let should_accelerate = reached_window_size && within_window_time;

        if !should_accelerate {
            self.phase = Phase::Idle;
            self.velocity = MIN_VELOCITY;
            return 1;
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
        self.recent_events.clear();
    }
}
