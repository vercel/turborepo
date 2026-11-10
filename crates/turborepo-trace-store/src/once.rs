//! Memo cells that are safe to fill from rayon workers.
//!
//! `OnceLock::get_or_init` blocks every other caller while the initializer
//! runs. If the initializer itself waits on rayon work (a `par_iter`,
//! `rayon::join`, or the tracer's `pool.scope`, also across pools), the waiting
//! worker keeps executing other jobs from its own pool, and one of those can
//! call `get_or_init` on the same cell: it then blocks on itself, and once
//! every worker is parked the process deadlocks. This happened with
//! directory-membership listings during `affected` on turborepo.
//!
//! [`init_once`] computes the value outside the cell and publishes it
//! first-writer-wins, so no thread ever blocks on a cell. Concurrent first
//! callers may compute the value twice; the loser's copy is dropped. Use it for
//! every memo whose initializer can reach rayon. The computations it memoizes
//! must be deterministic (they read through the build's memoized `DirCache`),
//! so which copy wins does not matter.

use std::sync::OnceLock;

/// The value of `cell`, computing it with `f` (outside any lock) if it is
/// empty.
// Invariant: the cell was set above.
#[allow(clippy::expect_used)]
pub(crate) fn init_once<V>(cell: &OnceLock<V>, f: impl FnOnce() -> V) -> &V {
    if let Some(v) = cell.get() {
        return v;
    }
    let v = f();
    // Another thread may have published first; keep its value and drop ours.
    let _ = cell.set(v);
    cell.get().expect("OnceLock set above")
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use rayon::prelude::*;

    use super::*;

    /// Many rayon jobs fill one cell whose initializer itself runs nested rayon
    /// work: with `get_or_init` this deadlocks once a waiting worker steals
    /// a job that hits the cell.
    #[test]
    fn nested_rayon_initializers_do_not_deadlock() {
        for threads in [2, 3, 8, 16] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            for _ in 0..20 {
                let cell: Arc<OnceLock<usize>> = Arc::new(OnceLock::new());
                let runs = AtomicUsize::new(0);
                let sums: Vec<usize> = pool.install(|| {
                    (0..256)
                        .into_par_iter()
                        .map(|_| {
                            *init_once(&cell, || {
                                runs.fetch_add(1, Ordering::Relaxed);
                                (0..64usize)
                                    .into_par_iter()
                                    .map(|i| {
                                        std::thread::yield_now();
                                        i
                                    })
                                    .sum()
                            })
                        })
                        .collect()
                });
                assert!(sums.iter().all(|&s| s == (0..64).sum::<usize>()));
                assert!(runs.load(Ordering::Relaxed) >= 1);
            }
        }
    }
}
