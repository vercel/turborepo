//! Per-run backoff for sustained artifact service outages. This is independent
//! of the permanent 403 disable policy in `HTTPCache`.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::CacheError;

const FAILURE_THRESHOLD: u8 = 3;
const COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Default)]
struct State {
    failures: u8,
    open_until: Option<Instant>,
    probing: bool,
    generation: u64,
    warned: bool,
}

#[derive(Default)]
pub(crate) struct OutageBreaker(Mutex<State>);

pub(crate) struct Permit<'a> {
    breaker: &'a OutageBreaker,
    generation: u64,
    probe: bool,
    completed: bool,
}

impl OutageBreaker {
    #[cfg(test)]
    pub(crate) fn expire_for_test(&self) {
        self.0.lock().unwrap().open_until = Some(Instant::now() - Duration::from_secs(1));
    }

    pub(crate) fn enter(&self) -> Option<Permit<'_>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let probe = if let Some(until) = state.open_until {
            if Instant::now() < until || state.probing {
                return None;
            }
            state.probing = true;
            true
        } else {
            false
        };
        Some(Permit {
            breaker: self,
            generation: state.generation,
            probe,
            completed: false,
        })
    }
}

impl Permit<'_> {
    pub(crate) fn finish(mut self, result: &Result<(), &CacheError>) {
        let mut state = self.breaker.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == self.generation {
            let failure = result.as_ref().err().is_some_and(|err| is_outage(err));
            if failure {
                state.failures = state.failures.saturating_add(1);
                if self.probe || state.failures >= FAILURE_THRESHOLD {
                    state.open_until = Some(Instant::now() + COOLDOWN);
                    state.probing = false;
                    state.generation = state.generation.wrapping_add(1);
                    if !state.warned {
                        state.warned = true;
                        turborepo_log::warn(
                            turborepo_log::Source::turbo(turborepo_log::Subsystem::Cache),
                            "Remote artifact cache is temporarily unavailable; retrying after a \
                             30-second cooldown",
                        )
                        .emit();
                    }
                    tracing::debug!("remote artifact cache outage; backing off for 30 seconds");
                }
            } else {
                state.failures = 0;
                if self.probe {
                    state.open_until = None;
                    state.probing = false;
                    state.generation = state.generation.wrapping_add(1);
                    tracing::debug!("remote artifact cache recovered");
                }
            }
        }
        self.completed = true;
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        // If a half-open request is cancelled, do not leave the circuit stuck
        // with its only probe slot occupied. Retry after another cooldown.
        if self.probe && !self.completed {
            let mut state = self.breaker.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.generation == self.generation {
                state.probing = false;
                state.open_until = Some(Instant::now() + COOLDOWN);
            }
        }
    }
}

fn is_outage(error: &CacheError) -> bool {
    use turborepo_api_client::Error;
    match error {
        CacheError::ConnectError | CacheError::TimeoutError(_) => true,
        CacheError::ApiClientError(err, _) => match err.as_ref() {
            Error::ReqwestError(err) => {
                err.is_connect()
                    || err.is_timeout()
                    || err.status().is_some_and(|status| status.is_server_error())
            }
            Error::TooManyFailures(err) => err.is_connect() || err.is_timeout(),
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::backtrace::Backtrace;

    use super::*;

    fn outage() -> CacheError {
        CacheError::ConnectError
    }

    fn trip(breaker: &OutageBreaker) {
        for _ in 0..FAILURE_THRESHOLD {
            breaker.enter().unwrap().finish(&Err(&outage()));
        }
    }

    fn expire(breaker: &OutageBreaker) {
        breaker.0.lock().unwrap().open_until = Some(Instant::now() - Duration::from_secs(1));
    }

    #[test]
    fn sustained_failures_probe_and_recover() {
        let breaker = OutageBreaker::default();
        breaker.enter().unwrap().finish(&Err(&outage()));
        breaker.enter().unwrap().finish(&Ok(()));
        assert_eq!(breaker.0.lock().unwrap().failures, 0);
        trip(&breaker);
        assert!(breaker.enter().is_none());
        expire(&breaker);
        let probe = breaker.enter().unwrap();
        assert!(breaker.enter().is_none());
        probe.finish(&Err(&outage()));
        assert!(breaker.enter().is_none());
        expire(&breaker);
        breaker.enter().unwrap().finish(&Ok(()));
        assert!(breaker.enter().is_some());
    }

    #[test]
    fn non_outages_do_not_trip_and_close_a_probe() {
        let breaker = OutageBreaker::default();
        let non_outages = [
            CacheError::InvalidTag(Backtrace::capture()),
            CacheError::ForbiddenRemoteCacheWrite,
            CacheError::InvalidDuration(Backtrace::capture()),
        ];
        for error in &non_outages {
            for _ in 0..10 {
                breaker.enter().unwrap().finish(&Err(error));
            }
        }
        assert!(breaker.0.lock().unwrap().open_until.is_none());
        trip(&breaker);
        expire(&breaker);
        breaker.enter().unwrap().finish(&Err(&non_outages[0]));
        assert!(breaker.enter().is_some());
    }

    #[test]
    fn timeout_and_connectivity_failures_trip_but_signature_errors_do_not() {
        let breaker = OutageBreaker::default();
        breaker
            .enter()
            .unwrap()
            .finish(&Err(&CacheError::TimeoutError("hash".into())));
        breaker
            .enter()
            .unwrap()
            .finish(&Err(&CacheError::InvalidTag(Backtrace::capture())));
        assert!(breaker.enter().is_some());
        trip(&breaker);
        assert!(breaker.enter().is_none());
    }

    #[test]
    fn only_one_worker_probes_after_cooldown() {
        let breaker = std::sync::Arc::new(OutageBreaker::default());
        trip(&breaker);
        expire(&breaker);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(17));
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let breaker = breaker.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let permit = breaker.enter();
                    // Keep the winning probe in flight until every thread has tried.
                    barrier.wait();
                    permit.is_some()
                })
            })
            .collect();
        barrier.wait();
        // The second barrier has 17 participants too.
        barrier.wait();
        assert_eq!(
            workers
                .into_iter()
                .map(|w| w.join().unwrap())
                .filter(|won| *won)
                .count(),
            1
        );
    }

    #[test]
    fn stale_workers_and_cancelled_probe_cannot_change_recovery() {
        let breaker = OutageBreaker::default();
        let stale = breaker.enter().unwrap();
        trip(&breaker);
        stale.finish(&Ok(()));
        assert!(breaker.enter().is_none());
        expire(&breaker);
        drop(breaker.enter().unwrap());
        assert!(breaker.enter().is_none());
        expire(&breaker);
        let stale = breaker.enter().unwrap();
        stale.finish(&Ok(()));
        assert!(breaker.enter().is_some());
    }
}
