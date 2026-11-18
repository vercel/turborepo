//! Content hashes (xxh3-128) cached by `(dev, inode, size, mtime_ns)`, stored
//! in `<cache dir>/hashes`.
//!
//! Warm builds only `stat` unchanged files, at most once per build: a path
//! verified during this build (the same config file is an input of thousands of
//! shards) is answered from memory. An entry is not persisted when the file's
//! mtime is within [`RACY_WINDOW`] of the build start: a write in the same
//! timestamp granule after we hashed would otherwise go unnoticed (git's
//! "racily clean" problem).

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rayon::prelude::*;
use rkyv::{Archive, Deserialize, Serialize};
use turborepo_trace_core::os::MetadataExt;

/// Files modified this recently before the build started are hashed but not
/// cached.
pub const RACY_WINDOW: Duration = Duration::from_secs(2);

const MAGIC: &[u8; 8] = b"DTHASH1\0";

#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
}

#[derive(Archive, Serialize, Deserialize, Debug, Default)]
struct Table {
    entries: Vec<(String, Stamp, [u64; 2])>,
}

/// A cached hash: the stat it was computed for, whether it is persisted (not
/// racy), and whether this build already verified it against the file.
#[derive(Clone, Copy, Debug)]
struct Entry {
    stamp: Stamp,
    hash: u128,
    persist: bool,
    verified: bool,
}

const SHARDS: usize = 16;

/// One lock shard. The keys are paths of the user's own repository and the
/// table is rebuilt by every process: a fast hash, no need for a keyed one.
type Map = HashMap<PathBuf, Entry, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

/// The hash cache.
#[derive(Debug)]
pub struct HashCache {
    shards: Box<[RwLock<Map>]>,
    start_ns: i128,
    path: Option<PathBuf>,
    /// Whether an entry was added or replaced since the cache was loaded
    /// ([`HashCache::save`] skips an unchanged cache).
    dirty: std::sync::atomic::AtomicBool,
}

fn stamp(meta: &std::fs::Metadata) -> Stamp {
    Stamp {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.size(),
        mtime_ns: meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
    }
}

/// Splits a 128-bit hash for storage.
pub fn split(h: u128) -> [u64; 2] {
    [h as u64, (h >> 64) as u64]
}

/// Joins a stored 128-bit hash.
pub fn join(h: [u64; 2]) -> u128 {
    h[0] as u128 | ((h[1] as u128) << 64)
}

impl HashCache {
    /// An empty cache that is not persisted.
    pub fn in_memory() -> Self {
        Self::from_map(HashMap::new(), None)
    }

    fn from_map(map: HashMap<PathBuf, (Stamp, u128)>, path: Option<PathBuf>) -> Self {
        let mut maps: Vec<Map> = (0..SHARDS)
            .map(|_| Map::with_capacity_and_hasher(map.len() / SHARDS + 1, Default::default()))
            .collect();
        for (p, (stamp, hash)) in map {
            let e = Entry {
                stamp,
                hash,
                persist: true,
                verified: false,
            };
            maps[shard_of(&p)].insert(p, e);
        }
        HashCache {
            shards: maps.into_iter().map(RwLock::new).collect(),
            start_ns: now_ns(),
            path,
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn shard(&self, path: &Path) -> usize {
        shard_of(path)
    }

    fn read(&self, path: &Path) -> RwLockReadGuard<'_, Map> {
        self.shards[self.shard(path)]
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self, path: &Path) -> RwLockWriteGuard<'_, Map> {
        self.shards[self.shard(path)]
            .write()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Loads the cache file (a missing or unreadable file gives an empty
    /// cache).
    ///
    /// A big repository has tens of thousands of entries and every refresh
    /// loads them all, so the entries are read from the archive in place
    /// and the lock shards are filled in parallel, each by one job, in file
    /// order (a later entry of a path replaces an earlier one, as inserting
    /// them one after the other would).
    pub fn open(path: &Path) -> Self {
        let mut maps: Vec<Map> = (0..SHARDS).map(|_| Map::default()).collect();
        if let Ok(bytes) = std::fs::read(path)
            && bytes.len() >= 8
            && &bytes[..8] == MAGIC
        {
            let mut buf = rkyv::util::AlignedVec::<16>::with_capacity(bytes.len() - 8);
            buf.extend_from_slice(&bytes[8..]);
            if let Ok(t) = rkyv::access::<ArchivedTable, rkyv::rancor::Error>(&buf) {
                let entries = t.entries.as_slice();
                let shard: Vec<u8> = entries
                    .par_iter()
                    .with_min_len(1024)
                    .map(|e| shard_of(Path::new(e.0.as_str())) as u8)
                    .collect();
                maps = (0..SHARDS)
                    .into_par_iter()
                    .map(|si| {
                        let n = shard.iter().filter(|&&s| s as usize == si).count();
                        let mut m = Map::with_capacity_and_hasher(n, Default::default());
                        for (e, _) in entries
                            .iter()
                            .zip(&shard)
                            .filter(|(_, s)| **s as usize == si)
                        {
                            m.insert(
                                PathBuf::from(e.0.as_str()),
                                Entry {
                                    stamp: Stamp {
                                        dev: e.1.dev.to_native(),
                                        ino: e.1.ino.to_native(),
                                        size: e.1.size.to_native(),
                                        mtime_ns: e.1.mtime_ns.to_native(),
                                    },
                                    hash: join([e.2[0].to_native(), e.2[1].to_native()]),
                                    persist: true,
                                    verified: false,
                                },
                            );
                        }
                        m
                    })
                    .collect();
            }
        }
        HashCache {
            shards: maps.into_iter().map(RwLock::new).collect(),
            start_ns: now_ns(),
            path: Some(path.to_path_buf()),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Hashes one file, from the cache when its stat is unchanged.
    pub fn hash(&self, path: &Path) -> std::io::Result<u128> {
        let cached = self.read(path).get(path).copied();
        if let Some(e) = cached
            && e.verified
        {
            return Ok(e.hash);
        }
        let meta = std::fs::metadata(path)?;
        let st = stamp(&meta);
        if let Some(e) = cached
            && e.stamp == st
        {
            if let Some(e) = self.write(path).get_mut(path) {
                e.verified = true;
            }
            return Ok(e.hash);
        }
        let bytes = std::fs::read(path)?;
        let h = xxhash_rust::xxh3::xxh3_128(&bytes);
        self.insert_stamped(path, st, h);
        Ok(h)
    }

    /// Hashes many files in parallel; `None` for files that cannot be read.
    pub fn hash_many(&self, paths: &[PathBuf]) -> Vec<Option<u128>> {
        paths.par_iter().map(|p| self.hash(p).ok()).collect()
    }

    /// Records a hash computed elsewhere (for example inline while parsing) for
    /// `path`.
    pub fn insert(&self, path: &Path, hash: u128) {
        if let Ok(meta) = std::fs::metadata(path) {
            self.insert_stamped(path, stamp(&meta), hash);
        }
    }

    fn insert_stamped(&self, path: &Path, st: Stamp, hash: u128) {
        let persist = st.mtime_ns + (RACY_WINDOW.as_nanos() as i128) < self.start_ns;
        let old = self.write(path).insert(
            path.to_path_buf(),
            Entry {
                stamp: st,
                hash,
                persist,
                verified: true,
            },
        );
        // The persisted table changes with a persisted entry or a dropped one.
        let same = old.is_some_and(|e| e.persist == persist && e.stamp == st && e.hash == hash);
        if !same && (persist || old.is_some_and(|e| e.persist)) {
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The persisted entries.
    fn entries(&self) -> Vec<(PathBuf, Stamp, u128)> {
        self.shards
            .iter()
            .flat_map(|s| {
                s.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .filter(|(_, e)| e.persist)
                    .map(|(p, e)| (p.clone(), e.stamp, e.hash))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Writes the cache back, keeping only entries for `keep` paths when given.
    pub fn save(&self, keep: Option<&dyn Fn(&Path) -> bool>) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if keep.is_none() && !self.dirty.load(std::sync::atomic::Ordering::Relaxed) && path.exists()
        {
            return Ok(());
        }
        let mut entries: Vec<(String, Stamp, [u64; 2])> = self
            .entries()
            .into_iter()
            .filter(|(p, _, _)| keep.is_none_or(|k| k(p)))
            .map(|(p, s, h)| (p.to_string_lossy().into_owned(), s, split(h)))
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&Table { entries })
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = Vec::with_capacity(8 + bytes.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&bytes);
        crate::write_atomic(path, &out)
    }
}

/// The lock shard of a path (a fixed-key hasher: it must not change between
/// calls). The high bits: the shard's own table hashes the path the same way
/// and uses the low ones.
fn shard_of(path: &Path) -> usize {
    let mut h = rustc_hash::FxHasher::default();
    path.hash(&mut h);
    (h.finish() >> 48) as usize % SHARDS
}

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_by_stat() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a");
        std::fs::write(&f, "x").unwrap();
        // Make the file old enough to be cached.
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let db = t.path().join("cache/hashes");
        let c = HashCache::open(&db);
        let h = c.hash(&f).unwrap();
        assert_eq!(h, xxhash_rust::xxh3::xxh3_128(b"x"));
        c.save(None).unwrap();
        let c2 = HashCache::open(&db);
        assert_eq!(c2.entries().len(), 1);
        // A fresh write is hashed again (stat changed) and not persisted (racy window).
        std::fs::write(&f, "yy").unwrap();
        assert_eq!(c2.hash(&f).unwrap(), xxhash_rust::xxh3::xxh3_128(b"yy"));
        assert!(c2.entries().is_empty());
        // Within one build, a verified path is answered without a stat.
        assert_eq!(c.hash(&f).unwrap(), xxhash_rust::xxh3::xxh3_128(b"x"));
    }
}
