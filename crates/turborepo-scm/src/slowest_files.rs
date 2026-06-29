//! A small, concurrently-writable structure that tracks the files that take
//! the longest to hash. Hashing reads file contents, so a single large file
//! (e.g. a multi-GB untracked temp file) can dominate startup time. When a
//! consumer such as the file watcher times out waiting for hashing to finish,
//! it can snapshot this structure to point at the likely culprit — including
//! files that are *still being hashed* (the most likely cause of a hang).

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use turbopath::RelativeUnixPathBuf;

/// Number of completed entries to retain, ranked by hashing duration.
const TOP_N_COMPLETED: usize = 5;

/// An opaque handle returned by [`SlowestFiles::start`] and passed back to
/// [`SlowestFiles::finish`]. Identifies the in-flight entry so it can be
/// converted to a completed one.
#[derive(Clone, Copy, Debug)]
pub struct HashTicket(u64);

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

#[derive(Debug)]
struct Inner {
    /// Files currently being hashed, keyed by ticket id.
    live: Vec<(u64, RelativeUnixPathBuf, Instant)>,
    /// Top-N completed files by hashing duration, ascending so the cheapest
    /// (the eviction candidate) is first.
    completed: Vec<(RelativeUnixPathBuf, Duration)>,
}

/// Tracks the slowest-to-hash files. Cheap to share via `Arc`; the hot path
/// (`start`/`finish`) only locks briefly and `finish` only inserts into the
/// completed set when a file beats the current Nth-slowest.
#[derive(Debug)]
pub struct SlowestFiles {
    next_id: AtomicU64,
    inner: Mutex<Inner>,
}

impl Default for SlowestFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl SlowestFiles {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(0),
            inner: Mutex::new(Inner {
                live: Vec::new(),
                completed: Vec::new(),
            }),
        }
    }

    /// Record that hashing of `path` has started. Returns a ticket to pass to
    /// [`finish`](Self::finish) on completion.
    pub fn start(&self, path: RelativeUnixPathBuf) -> HashTicket {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        if let Ok(mut inner) = self.inner.lock() {
            inner.live.push((id, path, now));
        }
        HashTicket(id)
    }

    /// Record that hashing of the file identified by `ticket` has finished.
    pub fn finish(&self, ticket: HashTicket) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let Some(pos) = inner.live.iter().position(|(id, _, _)| *id == ticket.0) else {
            return;
        };
        let (_, path, started) = inner.live.swap_remove(pos);
        let elapsed = started.elapsed();

        // Only retain if it beats the cheapest tracked entry (or we have room).
        if inner.completed.len() < TOP_N_COMPLETED {
            inner.completed.push((path, elapsed));
        } else if let Some((_, min)) = inner.completed.first() {
            if elapsed > *min {
                inner.completed[0] = (path, elapsed);
            } else {
                return;
            }
        }
        // Keep ascending by duration so index 0 is the eviction candidate.
        inner.completed.sort_by_key(|(_, a)| *a);
    }

    /// Snapshot the slowest files: every in-flight file (with elapsed-so-far)
    /// plus the top-N completed, sorted slowest-first. In-flight files are
    /// listed first since they are the likely cause of a hang.
    pub fn snapshot(&self) -> Vec<SlowestFile> {
        let now = Instant::now();
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };

        let mut live: Vec<SlowestFile> = inner
            .live
            .iter()
            .map(|(_, path, started)| SlowestFile {
                path: path.clone(),
                duration: now.saturating_duration_since(*started),
                in_flight: true,
            })
            .collect();
        live.sort_by_key(|f| std::cmp::Reverse(f.duration));

        let mut completed: Vec<SlowestFile> = inner
            .completed
            .iter()
            .map(|(path, duration)| SlowestFile {
                path: path.clone(),
                duration: *duration,
                in_flight: false,
            })
            .collect();
        completed.sort_by_key(|f| std::cmp::Reverse(f.duration));

        live.extend(completed);
        live
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use turbopath::RelativeUnixPathBuf;

    use super::{SlowestFiles, TOP_N_COMPLETED};

    fn p(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s).unwrap()
    }

    #[test]
    fn in_flight_files_appear_in_snapshot() {
        let sf = SlowestFiles::new();
        let _t = sf.start(p("big.tmp"));
        let snap = sf.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap[0].in_flight);
        assert_eq!(snap[0].path, p("big.tmp"));
    }

    #[test]
    fn finished_files_move_to_completed() {
        let sf = SlowestFiles::new();
        let t = sf.start(p("a"));
        sf.finish(t);
        let snap = sf.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].in_flight);
    }

    #[test]
    fn completed_is_bounded_to_top_n() {
        let sf = Arc::new(SlowestFiles::new());
        for i in 0..(TOP_N_COMPLETED + 10) {
            let t = sf.start(p(&format!("f{i}")));
            sf.finish(t);
        }
        let snap = sf.snapshot();
        assert_eq!(snap.len(), TOP_N_COMPLETED);
    }

    #[test]
    fn in_flight_listed_before_completed() {
        let sf = SlowestFiles::new();
        let done = sf.start(p("done"));
        sf.finish(done);
        let _live = sf.start(p("live"));
        let snap = sf.snapshot();
        assert!(snap[0].in_flight);
        assert_eq!(snap[0].path, p("live"));
    }
}
