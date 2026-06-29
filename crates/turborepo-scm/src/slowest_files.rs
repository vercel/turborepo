//! A small, concurrently-writable structure that tracks the files that take
//! the longest to hash. Hashing reads file contents, so a single large file
//! (e.g. a multi-GB untracked temp file) can dominate startup time. When a
//! consumer such as the file watcher times out waiting for hashing to finish,
//! it can snapshot this structure to point at the likely culprit — including
//! files that are *still being hashed* (the most likely cause of a hang).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use turbopath::RelativeUnixPathBuf;

/// Number of completed entries to retain, ranked by hashing duration.
const TOP_N_COMPLETED: usize = 5;

#[derive(Debug, Clone)]
pub struct SlowestFile {
    /// Path relative to the git root (i.e. project-relative), as recorded by
    /// the hashing loop.
    pub path: RelativeUnixPathBuf,
    /// Final hashing duration, or for an in-flight file the time elapsed so
    /// far at the moment of the snapshot.
    pub duration: Duration,
    /// Whether the file was still being hashed when snapshotted.
    pub in_flight: bool,
}

#[derive(Debug, Default)]
struct Inner {
    /// Files currently being hashed, keyed by the id held by their guard.
    live: HashMap<u64, (RelativeUnixPathBuf, Instant)>,
    /// Top-N completed files by hashing duration, ascending so the cheapest
    /// (the eviction candidate) is first.
    completed: Vec<(RelativeUnixPathBuf, Duration)>,
}

/// Tracks the slowest-to-hash files. Cheap to share via `Arc`; the hot path
/// only locks briefly, and a completed file is only inserted when it beats the
/// current Nth-slowest.
#[derive(Debug, Default)]
pub struct SlowestFiles {
    next_id: AtomicU64,
    inner: Mutex<Inner>,
}

/// RAII guard returned by [`SlowestFiles::start`]. Records the file's hashing
/// duration when dropped, so the caller can't forget to mark completion.
pub struct HashGuard<'a> {
    files: &'a SlowestFiles,
    id: u64,
}

impl Drop for HashGuard<'_> {
    fn drop(&mut self) {
        self.files.finish(self.id);
    }
}

impl SlowestFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that hashing of `path` has started. The returned guard records
    /// the elapsed duration when it is dropped.
    #[must_use]
    pub fn start(&self, path: RelativeUnixPathBuf) -> HashGuard<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut inner) = self.inner.lock() {
            inner.live.insert(id, (path, Instant::now()));
        }
        HashGuard { files: self, id }
    }

    fn finish(&self, id: u64) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let Some((path, started)) = inner.live.remove(&id) else {
            return;
        };
        let elapsed = started.elapsed();

        // `completed` is kept ascending by duration, so index 0 is the cheapest
        // (eviction candidate). Skip entirely if we're full and this file isn't
        // slower than the current minimum.
        if inner.completed.len() == TOP_N_COMPLETED {
            match inner.completed.first() {
                Some((_, min)) if elapsed <= *min => return,
                _ => {
                    inner.completed.remove(0);
                }
            }
        }
        // Insertion sort: small N, and the common case appends near the end.
        let pos = inner
            .completed
            .partition_point(|(_, d)| *d <= elapsed);
        inner.completed.insert(pos, (path, elapsed));
    }

    /// Snapshot the slowest files: every in-flight file (with elapsed-so-far)
    /// plus the top-N completed. In-flight files are listed first (they are the
    /// likely cause of a hang), then both groups by descending duration.
    pub fn snapshot(&self) -> Vec<SlowestFile> {
        let now = Instant::now();
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };

        let mut files: Vec<SlowestFile> = inner
            .live
            .values()
            .map(|(path, started)| SlowestFile {
                path: path.clone(),
                duration: now.saturating_duration_since(*started),
                in_flight: true,
            })
            .chain(inner.completed.iter().map(|(path, duration)| SlowestFile {
                path: path.clone(),
                duration: *duration,
                in_flight: false,
            }))
            .collect();
        // In-flight first, then slowest first within each group.
        files.sort_by_key(|f| (!f.in_flight, std::cmp::Reverse(f.duration)));
        files
    }
}

#[cfg(test)]
mod test {
    use turbopath::RelativeUnixPathBuf;

    use super::{SlowestFiles, TOP_N_COMPLETED};

    fn p(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s).unwrap()
    }

    #[test]
    fn in_flight_files_appear_in_snapshot() {
        let sf = SlowestFiles::new();
        let _guard = sf.start(p("big.tmp"));
        let snap = sf.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap[0].in_flight);
        assert_eq!(snap[0].path, p("big.tmp"));
    }

    #[test]
    fn finished_files_move_to_completed() {
        let sf = SlowestFiles::new();
        drop(sf.start(p("a")));
        let snap = sf.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].in_flight);
    }

    #[test]
    fn completed_is_bounded_to_top_n() {
        let sf = SlowestFiles::new();
        for i in 0..(TOP_N_COMPLETED + 10) {
            drop(sf.start(p(&format!("f{i}"))));
        }
        let snap = sf.snapshot();
        assert_eq!(snap.len(), TOP_N_COMPLETED);
    }

    #[test]
    fn in_flight_listed_before_completed() {
        let sf = SlowestFiles::new();
        drop(sf.start(p("done")));
        let _live = sf.start(p("live"));
        let snap = sf.snapshot();
        assert!(snap[0].in_flight);
        assert_eq!(snap[0].path, p("live"));
    }
}
