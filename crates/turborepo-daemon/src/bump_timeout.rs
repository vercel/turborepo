use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use tokio::time::Instant;

/// A timeout that can be bumped forward in time by calling reset.
///
/// Calling reset with a new duration will change the deadline
/// to the current time plus the new duration. It is non-mutating
/// and can be called from multiple threads.
#[derive(Debug)]
pub struct BumpTimeout {
    start: Instant,
    increment: Duration,
    deadline: AtomicU64,
}

impl BumpTimeout {
    #[allow(dead_code)]
    pub fn new(increment: Duration) -> Self {
        let start = Instant::now();
        let millis = increment.as_millis();
        Self {
            start,
            deadline: AtomicU64::new(millis as u64),
            increment,
        }
    }

    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.deadline.load(Ordering::Relaxed))
    }

    #[allow(dead_code)]
    pub fn deadline(&self) -> Instant {
        self.start + self.duration()
    }

    #[allow(dead_code)]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Resets the deadline to the current time plus the given duration.
    pub fn reset(&self) {
        let duration = self.start.elapsed() + self.increment;
        self.deadline
            .store(duration.as_millis() as u64, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn as_instant(&self) -> Instant {
        self.start + self.duration()
    }

    /// Waits until the deadline is reached, but if the deadline is
    /// changed while waiting, it will wait until the new deadline is reached.
    #[allow(dead_code)]
    pub async fn wait(&self) {
        let mut deadline = self.as_instant();
        loop {
            tokio::time::sleep_until(deadline).await;
            let new_deadline = self.as_instant();

            if new_deadline > deadline {
                deadline = new_deadline;
            } else {
                break;
            }
        }
    }
}
