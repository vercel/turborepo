//! Wall-clock marks for profiling phases (`TURBO_TRACE_DEBUG_TIMING=1`): each
//! [`mark`] prints the milliseconds since the first mark and since the previous
//! one to stderr. Free when unset.
//!
//! Marks say where the wall time of a sequential phase goes. For work spread
//! over threads (per-file analysis, per-package assembly) a [`Span`] sums the
//! time every thread spent in a region and counts the calls; [`report_spans`]
//! prints them. A span costs one atomic load when timing is off.

use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

/// Thread time and call count of one code region, summed over all threads.
///
/// ```
/// use turborepo_trace_core::timing::Span;
/// static READ: Span = Span::new("trace: read");
/// let _t = READ.enter();
/// ```
pub struct Span {
    name: &'static str,
    ns: AtomicU64,
    calls: AtomicU64,
    listed: std::sync::Once,
}

/// Adds the time since [`Span::enter`] to the span when dropped.
pub struct SpanGuard(Option<(&'static Span, Instant)>);

fn spans() -> &'static Mutex<Vec<&'static Span>> {
    static S: OnceLock<Mutex<Vec<&'static Span>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

impl Span {
    /// A span named `name` (`"<phase>: <what>"`).
    pub const fn new(name: &'static str) -> Span {
        Span {
            name,
            ns: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            listed: std::sync::Once::new(),
        }
    }

    /// Starts timing; the guard stops it. Does nothing unless timing is
    /// [`enabled`].
    #[inline]
    pub fn enter(&'static self) -> SpanGuard {
        if !enabled() {
            return SpanGuard(None);
        }
        SpanGuard(Some((self, Instant::now())))
    }

    /// Adds `ns` nanoseconds and one call (for regions timed by the caller).
    pub fn add(&'static self, ns: u64) {
        self.listed.call_once(|| {
            spans().lock().unwrap_or_else(|e| e.into_inner()).push(self);
        });
        self.ns.fetch_add(ns, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        if let Some((s, t)) = self.0.take() {
            s.add(t.elapsed().as_nanos() as u64);
        }
    }
}

/// Prints every span entered since the last report as `timing span <thread ms>
/// ms <calls> call(s) <name>`, by name, and resets them.
pub fn report_spans() {
    if !enabled() {
        return;
    }
    let mut v: Vec<&'static Span> = spans().lock().unwrap_or_else(|e| e.into_inner()).clone();
    v.sort_by_key(|s| s.name);
    for s in v {
        let (ns, n) = (
            s.ns.swap(0, Ordering::Relaxed),
            s.calls.swap(0, Ordering::Relaxed),
        );
        if n > 0 {
            eprintln!(
                "timing span {:10.1} thread-ms {n:9} call(s)  {}",
                ns as f64 / 1e6,
                s.name
            );
        }
    }
}

fn state() -> Option<&'static Mutex<(Instant, Instant)>> {
    static S: OnceLock<Option<Mutex<(Instant, Instant)>>> = OnceLock::new();
    S.get_or_init(|| {
        crate::debug::var_os("TIMING").map(|_| {
            let now = Instant::now();
            Mutex::new((now, now))
        })
    })
    .as_ref()
}

/// Whether marks are printed.
pub fn enabled() -> bool {
    state().is_some()
}

/// Prints `label` with the time since the first mark and since the previous
/// mark.
pub fn mark(label: &str) {
    let Some(s) = state() else {
        return;
    };
    let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    eprintln!(
        "timing {:8.1} ms (+{:7.1}) {label}",
        (now - s.0).as_secs_f64() * 1e3,
        (now - s.1).as_secs_f64() * 1e3
    );
    s.1 = now;
}
