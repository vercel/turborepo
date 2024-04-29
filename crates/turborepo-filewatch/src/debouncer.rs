use std::{fmt::Debug, sync::Mutex, time::Duration};

use tokio::{select, sync, time::Instant};
use tracing::trace;

pub(crate) struct Debouncer {
    bump: sync::Notify,
    serial: Mutex<Option<usize>>,
    timeout: Duration,
}

const DEFAULT_DEBOUNCE_TIMEOUT: Duration = Duration::from_millis(10);

impl Default for Debouncer {
    fn default() -> Self {
        Self::new(DEFAULT_DEBOUNCE_TIMEOUT)
    }
}

impl Debug for Debouncer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let serial = { self.serial.lock().expect("lock is valid") };
        f.debug_struct("Debouncer")
            .field("is_pending", &serial.is_some())
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl Debouncer {
    pub(crate) fn new(timeout: Duration) -> Self {
        let bump = sync::Notify::new();
        let serial = Mutex::new(Some(0));
        Self {
            bump,
            serial,
            timeout,
        }
    }

    pub(crate) fn bump(&self) -> bool {
        let mut serial = self.serial.lock().expect("lock is valid");
        match *serial {
            None => false,
            Some(previous) => {
                *serial = Some(previous + 1);
                self.bump.notify_one();
                true
            }
        }
    }

    pub(crate) async fn debounce(&self) {
        let mut serial = {
            self.serial
                .lock()
                .expect("lock is valid")
                .expect("only this thread sets the value to None")
        };
        let mut deadline = Instant::now() + self.timeout;
        loop {
            let timeout = tokio::time::sleep_until(deadline);
            select! {
                _ = self.bump.notified() => {
                    trace!("debouncer notified");
                    // reset timeout
                    let current_serial = self.serial.lock().expect("lock is valid").expect("only this thread sets the value to None");
                    if current_serial == serial {
                        // we timed out between the serial update and the notification.
                        // ignore this notification, we've already bumped the timer
                        continue;
                    } else {
                        serial = current_serial;
                        deadline = Instant::now() + self.timeout;
                    }
                }
                _ = timeout => {
                    // check if serial is still valid. It's possible a bump came in before the timeout,
                    // but we haven't been notified yet.
                    let mut current_serial_opt = self.serial.lock().expect("lock is valid");
                    let current_serial = current_serial_opt.expect("only this thread sets the value to None");
                    if current_serial == serial {
                        // if the serial is what we last observed, and the timer expired, we timed out.
                        // we're done. Mark that we won't accept any more bumps and return
                        *current_serial_opt = None;
                        return;
                    } else {
                        serial = current_serial;
                        deadline = Instant::now() + self.timeout;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use crate::debouncer::Debouncer;

    #[tokio::test]
    async fn test_debouncer() {
        let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
        let debouncer_copy = debouncer.clone();
        let handle = tokio::task::spawn(async move {
            debouncer_copy.debounce().await;
        });
        for _ in 0..10 {
            // assert that we can continue bumping it past the original timeout
            tokio::time::sleep(Duration::from_millis(2)).await;
            assert!(debouncer.bump());
        }
        let start = Instant::now();
        handle.await.unwrap();
        let end = Instant::now();
        // give some wiggle room to account for race conditions, but assert that we
        // didn't immediately complete after the last bump
        assert!(end - start > Duration::from_millis(5));
        // we shouldn't be able to bump it after it's run out it's timeout
        assert!(!debouncer.bump());
    }
}
