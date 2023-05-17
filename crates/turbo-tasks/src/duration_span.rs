use std::time::Instant;

pub struct DurationSpanGuard<F: FnOnce(u64)> {
    start: Instant,
    f: Option<F>,
}

impl<F: FnOnce(u64)> DurationSpanGuard<F> {
    pub fn new(f: F) -> Self {
        Self {
            start: Instant::now(),
            f: Some(f),
        }
    }
}

impl<F: FnOnce(u64)> Drop for DurationSpanGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            f(self.start.elapsed().as_micros() as u64);
        }
    }
}

#[macro_export]
macro_rules! duration_span {
    ($name:literal) => {
        turbo_tasks::duration_span::DurationSpanGuard::new(|duration| {
            turbo_tasks::macro_helpers::tracing::info!(name = $name, duration = duration);
        })
    };
    ($name:literal, $($arg:tt)+) => {
        turbo_tasks::duration_span::DurationSpanGuard::new(|duration| {
            turbo_tasks::macro_helpers::tracing::info!(name = $name, $($arg)+, duration = duration);
        })
    };
}
