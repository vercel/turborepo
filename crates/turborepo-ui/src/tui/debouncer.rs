use std::time::{Duration, Instant};

pub struct Debouncer<T> {
    value: Option<T>,
    duration: Duration,
    start: Option<Instant>,
}

impl<T> Debouncer<T> {
    /// Creates a new debouncer that will yield the latest value after the
    /// provided duration Duration is reset after the debouncer yields a
    /// value.
    pub fn new(duration: Duration) -> Self {
        Self {
            value: None,
            duration,
            start: None,
        }
    }

    /// Returns a value if debouncer duration has elapsed.
    #[must_use]
    pub fn query(&mut self) -> Option<T> {
        if self
            .start
            .map_or(false, |start| start.elapsed() >= self.duration)
        {
            self.start = None;
            self.value.take()
        } else {
            None
        }
    }

    /// Updates debouncer with given value. Returns a value if debouncer
    /// duration has elapsed.
    #[must_use]
    pub fn update(&mut self, value: T) -> Option<T> {
        self.insert_value(Some(value));
        self.query()
    }

    fn insert_value(&mut self, value: Option<T>) {
        // If there isn't a start set, bump it
        self.start.get_or_insert_with(Instant::now);
        if let Some(value) = value {
            self.value = Some(value);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DEFAULT_DURATION: Duration = Duration::from_millis(5);

    #[test]
    fn test_yields_after_duration() {
        let mut debouncer = Debouncer::new(DEFAULT_DURATION);
        assert!(debouncer.update(1).is_none());
        assert!(debouncer.query().is_none());
        std::thread::sleep(DEFAULT_DURATION);
        assert_eq!(debouncer.query(), Some(1));
        assert!(debouncer.query().is_none());
    }

    #[test]
    fn test_yields_latest() {
        let mut debouncer = Debouncer::new(DEFAULT_DURATION);
        assert!(debouncer.update(1).is_none());
        assert!(debouncer.update(2).is_none());
        assert!(debouncer.update(3).is_none());
        std::thread::sleep(DEFAULT_DURATION);
        assert_eq!(debouncer.update(4), Some(4));
        assert!(debouncer.query().is_none());
    }
}
