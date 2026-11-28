//! Memoized lookups that keep their filesystem observations (graph mode).
//!
//! Resolvers memoize expensive steps (the nearest `go.mod`, a parsed
//! `Cargo.toml`, a package directory listing). In graph mode
//! ([`crate::record`]) every reference must report what its resolution
//! observed, including what a memoized step observed the first time it ran.
//! [`Memo::get`] runs the computation inside its own recording frame with every
//! pause lifted, stores the frame's observations next to the value
//! (deduplicated), and replays them into the caller's frame, under the caller's
//! pauses, on this and every later call. Without recording
//! ([`DirCache::records`] is false) it is a plain concurrent memo table.
//!
//! Nested memos compose: an inner hit replays into the outer computation's
//! frame, so the outer entry stores the union. Replays can still repeat paths
//! across steps; the scheduler deduplicates each reference group's observations
//! ([`crate::Observations::dedup`]).

use std::{hash::Hash, sync::Arc};

use dashmap::DashMap;

use crate::{
    fs::DirCache,
    record::{self, Observations},
};

/// A concurrent memo table that replays recorded observations on hits. See the
/// module docs.
pub struct Memo<K, V> {
    map: DashMap<K, (V, Option<Arc<Observations>>)>,
}

impl<K: Eq + Hash, V> std::fmt::Debug for Memo<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memo")
            .field("len", &self.map.len())
            .finish()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Memo<K, V> {
    /// An empty table.
    pub fn new() -> Self {
        Memo {
            map: DashMap::new(),
        }
    }

    /// The value for `key`, computing it with `f` on first use. With a
    /// recording `fs`, `f` runs in its own frame whose observations are
    /// stored and re-reported to the caller's frame on this and every later
    /// call.
    ///
    /// `f` may call [`Memo::get`] on this or other tables (the table is not
    /// locked while `f` runs). Two threads may compute the same key
    /// concurrently; the first stored result wins and both report its
    /// observations.
    ///
    /// `f` runs with every pause lifted ([`record::capture`]), and what is
    /// reported is subject to the caller's pauses ([`record::replay`]): the
    /// stored observations are what the computation observes, whichever caller
    /// happens to run it first. A first caller that pauses recording (one that
    /// reports what it found itself, a workspace load) does not leave the entry
    /// without observations for the later callers that record.
    pub fn get(&self, fs: &DirCache, key: &K, f: impl FnOnce() -> V) -> V {
        if let Some(e) = self.map.get(key) {
            replay(e.1.as_deref());
            return e.0.clone();
        }
        let (v, obs) = if fs.records() {
            let (v, mut o) = record::capture(f);
            o.dedup();
            (v, Some(Arc::new(o)))
        } else {
            (f(), None)
        };
        let e = self.map.entry(key.clone()).or_insert((v, obs));
        replay(e.1.as_deref());
        e.0.clone()
    }

    /// The cached value for `key`, without computing or replaying anything.
    pub fn peek(&self, key: &K) -> Option<V> {
        self.map.get(key).map(|e| e.0.clone())
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Drops everything.
    pub fn clear(&self) {
        self.map.clear();
    }
}

fn replay(obs: Option<&Observations>) {
    if let Some(o) = obs
        && !o.is_empty()
    {
        record::replay(o);
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Default for Memo<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[test]
    fn replays_observations_on_hits() {
        let fs = DirCache::with_recording();
        let memo: Memo<u32, u32> = Memo::new();
        let calls = AtomicUsize::new(0);
        let compute = || {
            calls.fetch_add(1, Ordering::Relaxed);
            record::missing(Path::new("/a"));
            record::missing(Path::new("/a"));
            record::read(Path::new("/b"));
            7
        };
        let f = record::start();
        assert_eq!(memo.get(&fs, &1, compute), 7);
        let first = f.finish();
        let f = record::start();
        assert_eq!(memo.get(&fs, &1, || unreachable!()), 7);
        let second = f.finish();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(first, second);
        // Stored deduplicated.
        assert_eq!(first.missing, [PathBuf::from("/a")]);
        assert_eq!(first.reads, [PathBuf::from("/b")]);
    }

    #[test]
    fn nested_memos_compose() {
        let fs = DirCache::with_recording();
        let inner: Memo<u32, u32> = Memo::new();
        let outer: Memo<u32, u32> = Memo::new();
        let f = record::start();
        inner.get(&fs, &1, || {
            record::missing(Path::new("/inner"));
            1
        });
        drop(f);
        let f = record::start();
        outer.get(&fs, &1, || {
            record::missing(Path::new("/outer"));
            inner.get(&fs, &1, || unreachable!()) + 1
        });
        let got = f.finish();
        assert_eq!(
            got.missing,
            [PathBuf::from("/outer"), PathBuf::from("/inner")]
        );
    }

    /// What a memoized step reports does not depend on whether the caller that
    /// computed it first had recording paused: every caller gets what its own
    /// pauses let through.
    #[test]
    fn a_paused_first_caller_keeps_the_observations_for_the_others() {
        let fs = DirCache::with_recording();
        let compute = || {
            record::missing(Path::new("/m"));
            record::listing(Path::new("/d"), "**");
            1
        };
        let listing = [(PathBuf::from("/d"), "**".to_string())];
        // First asked with all recording paused: nothing reaches that caller, and a
        // caller that records gets the step's observations.
        let memo: Memo<u32, u32> = Memo::new();
        let f = record::start();
        record::pause(|| memo.get(&fs, &1, compute));
        assert!(f.finish().is_empty());
        let f = record::start();
        memo.get(&fs, &1, || unreachable!());
        let got = f.finish();
        assert_eq!(got.missing, [PathBuf::from("/m")]);
        assert_eq!(got.listings, listing);
        // First asked with listings paused: the same for the listing, and a later
        // hit under a listings pause does not report it either.
        let memo: Memo<u32, u32> = Memo::new();
        let f = record::start();
        record::pause_listings(|| memo.get(&fs, &1, compute));
        let got = f.finish();
        assert_eq!(got.missing, [PathBuf::from("/m")]);
        assert!(got.listings.is_empty());
        let f = record::start();
        record::pause_listings(|| memo.get(&fs, &1, || unreachable!()));
        assert!(f.finish().listings.is_empty());
        let f = record::start();
        memo.get(&fs, &1, || unreachable!());
        assert_eq!(f.finish().listings, listing);
    }

    #[test]
    fn plain_without_recording() {
        let fs = DirCache::new();
        let memo: Memo<&'static str, String> = Memo::new();
        assert_eq!(memo.get(&fs, &"k", || "v".to_string()), "v");
        assert_eq!(memo.peek(&"k").as_deref(), Some("v"));
        assert_eq!(memo.len(), 1);
        memo.clear();
        assert!(memo.is_empty());
    }
}
