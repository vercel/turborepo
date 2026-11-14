//! Memoized lookups that keep their filesystem observations (graph mode).
//!
//! Resolvers memoize expensive steps (the nearest `go.mod`, a parsed
//! `Cargo.toml`, a package directory listing). In graph mode
//! ([`crate::record`]) every reference must report what its resolution
//! observed, including what a memoized step observed the first time it ran.
//! [`Memo::get`] runs the computation inside its own recording frame, stores
//! the frame's observations next to the value (deduplicated), and replays them
//! into the caller's frame on this and every later call. Without recording
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
    pub fn get(&self, fs: &DirCache, key: &K, f: impl FnOnce() -> V) -> V {
        if let Some(e) = self.map.get(key) {
            replay(e.1.as_deref());
            return e.0.clone();
        }
        let frame = fs.records().then(record::start);
        let v = f();
        let obs = frame.map(|fr| {
            let mut o = fr.finish();
            o.dedup();
            Arc::new(o)
        });
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
        record::extend_from(o);
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
