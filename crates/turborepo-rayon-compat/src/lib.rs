/// Safe ceiling for rayon's global thread pool.
///
/// Rayon has a known initialization race condition that can deadlock
/// above ~90 threads on Linux. In [testing][issue], 96 cores always
/// deadlocked while 60 cores never did. 72 is a conservative cap — the
/// highest multiple of 8 below the observed failure threshold — giving
/// headroom against variance across kernel versions and schedulers.
///
/// [issue]: https://github.com/vercel/turborepo/issues/12251
pub const MAX_RAYON_THREADS: usize = 72;

/// Scale a CPU count to a safe rayon thread pool size.
///
/// Uses all available cores up to [`MAX_RAYON_THREADS`], then caps.
/// Always returns at least 1.
pub fn scale_thread_count(cpus: usize) -> usize {
    cpus.clamp(1, MAX_RAYON_THREADS)
}

/// Run a blocking closure, notifying the tokio runtime when possible.
///
/// On a multi-threaded tokio runtime this calls `tokio::task::block_in_place`
/// so the runtime can spawn a replacement worker. On current-thread runtimes
/// (common in tests) or outside of tokio the closure runs directly.
pub fn block_in_place<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_thread_count_always_at_least_one() {
        assert_eq!(scale_thread_count(0), 1);
        assert_eq!(scale_thread_count(1), 1);
    }

    #[test]
    fn scale_thread_count_preserves_small_values() {
        assert_eq!(scale_thread_count(2), 2);
        assert_eq!(scale_thread_count(4), 4);
        assert_eq!(scale_thread_count(8), 8);
        assert_eq!(scale_thread_count(16), 16);
    }

    #[test]
    fn scale_thread_count_caps_at_max() {
        assert_eq!(scale_thread_count(72), MAX_RAYON_THREADS);
        assert_eq!(scale_thread_count(96), MAX_RAYON_THREADS);
        assert_eq!(scale_thread_count(128), MAX_RAYON_THREADS);
        assert_eq!(scale_thread_count(256), MAX_RAYON_THREADS);
    }

    #[test]
    fn block_in_place_works_outside_runtime() {
        let result = block_in_place(|| 42);
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn block_in_place_works_on_current_thread_runtime() {
        let result = block_in_place(|| 42);
        assert_eq!(result, 42);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn block_in_place_works_on_multi_thread_runtime() {
        let result = block_in_place(|| 42);
        assert_eq!(result, 42);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn block_in_place_with_rayon_completes() {
        let result = block_in_place(|| {
            use rayon::prelude::*;
            (0..1000).into_par_iter().map(|i| i * 2).sum::<i64>()
        });
        assert_eq!(result, 999_000);
    }
}
