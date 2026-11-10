//! Concurrent path interner.

use std::{
    borrow::Borrow,
    ffi::OsStr,
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, PoisonError, RwLock},
};

use dashmap::{DashMap, mapref::entry::Entry};

use crate::{FileId, os::OsStrExt};

/// Map key that hashes and compares an interned path by its raw bytes, so
/// lookups can use a plain `&[u8]` without allocating.
#[derive(Clone, Debug)]
struct PathKey(Arc<Path>);

impl PathKey {
    fn bytes(&self) -> &[u8] {
        self.0.as_os_str().as_bytes()
    }
}

impl Borrow<[u8]> for PathKey {
    fn borrow(&self) -> &[u8] {
        self.bytes()
    }
}

impl Hash for PathKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Must match `<[u8] as Hash>::hash` for `Borrow<[u8]>` lookups to work.
        self.bytes().hash(state);
    }
}

impl PartialEq for PathKey {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

impl Eq for PathKey {}

/// Assigns a dense [`FileId`] to each distinct path, safely from many threads.
///
/// Paths are compared as raw bytes ([`OsStrExt`]); they need not be UTF-8. The
/// interner does not normalize. Callers intern absolute, lexically normalized
/// paths (see [`crate::fs::normalize`]) so each file gets exactly one id.
///
/// # Why `dashmap`
///
/// Interning needs an atomic insert-or-get that runs a side effect (allocating
/// the next dense id) exactly once per key. `dashmap`'s `entry` API holds the
/// shard lock for that, which makes the id assignment trivially correct.
/// Lookups for already-interned paths, the dominant operation when many files
/// import the same modules, take only a shard read lock, so contention spreads
/// across shards. `papaya` has lock-free reads, but expressing "allocate an
/// id exactly once" needs its `compute` API and retries, and it is not yet in
/// our dependency set. Revisit with an interner contention micro bench.
///
/// The reverse mapping (`FileId` to path) is an append-only `Vec` behind an
/// `RwLock`. It is only written while holding the shard lock of a vacant entry,
/// and never the other way round, so the two locks cannot deadlock.
#[derive(Debug, Default)]
pub struct PathInterner {
    map: DashMap<PathKey, FileId>,
    paths: RwLock<Vec<Arc<Path>>>,
}

impl PathInterner {
    /// Creates an empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the id for `path`, assigning a new one if it has not been seen.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` distinct paths are interned.
    // Invariant: FileId is u32 by design; a repository has far fewer than 2^32 distinct paths.
    #[allow(clippy::expect_used)]
    pub fn intern(&self, path: &Path) -> FileId {
        let bytes = path.as_os_str().as_bytes();
        if let Some(id) = self.map.get(bytes) {
            return *id;
        }
        match self.map.entry(PathKey(Arc::from(path))) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(v) => {
                let mut paths = self.paths.write().unwrap_or_else(PoisonError::into_inner);
                let id =
                    FileId(u32::try_from(paths.len()).expect("more than u32::MAX paths interned"));
                paths.push(Arc::clone(&v.key().0));
                v.insert(id);
                id
            }
        }
    }

    /// Returns the id for `path` if it was interned.
    pub fn get(&self, path: &Path) -> Option<FileId> {
        self.map.get(path.as_os_str().as_bytes()).map(|r| *r)
    }

    /// Returns the path for `id`.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this interner.
    // Invariant: ids come only from this interner (the documented panic of `path`).
    #[allow(clippy::expect_used)]
    pub fn path(&self, id: FileId) -> Arc<Path> {
        self.try_path(id)
            .expect("FileId does not belong to this interner")
    }

    /// Returns the path for `id`, or `None` if `id` was not produced by this
    /// interner.
    pub fn try_path(&self, id: FileId) -> Option<Arc<Path>> {
        let paths = self.paths.read().unwrap_or_else(PoisonError::into_inner);
        paths.get(id.index()).cloned()
    }

    /// Number of interned paths.
    pub fn len(&self) -> usize {
        self.paths
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether nothing has been interned.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Raw bytes of a path, for byte-wise sorting and comparison.
#[inline]
pub fn path_bytes(p: &Path) -> &[u8] {
    p.as_os_str().as_bytes()
}

/// A path from raw bytes.
#[inline]
pub fn path_from_bytes(b: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(b))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn interns_once() {
        let i = PathInterner::new();
        let a = i.intern(Path::new("/a/b"));
        let b = i.intern(Path::new("/a/c"));
        assert_ne!(a, b);
        assert_eq!(i.intern(Path::new("/a/b")), a);
        assert_eq!(&*i.path(a), Path::new("/a/b"));
        assert_eq!(i.get(Path::new("/a/c")), Some(b));
        assert_eq!(i.get(Path::new("/zzz")), None);
        assert_eq!(i.len(), 2);
        assert!(i.try_path(FileId(99)).is_none());
    }

    #[test]
    fn non_utf8_paths() {
        let i = PathInterner::new();
        let p = Path::new(OsStr::from_bytes(b"/tmp/\xff\xfe"));
        let id = i.intern(p);
        assert_eq!(&*i.path(id), p);
    }

    #[test]
    fn concurrent_interning_is_consistent() {
        let i = PathInterner::new();
        let ids: Vec<Vec<FileId>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        (0..500)
                            .map(|n| i.intern(Path::new(&format!("/p/{n}"))))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(i.len(), 500);
        for other in &ids[1..] {
            assert_eq!(other, &ids[0]);
        }
        for (n, id) in ids[0].iter().enumerate() {
            assert_eq!(&*i.path(*id), Path::new(&format!("/p/{n}")));
        }
    }
}
