//! Contents of small setup files (workspace manifests) cached by `(dev, inode,
//! size, mtime_ns)` in `<cache dir>/manifests`, so an unchanged workspace
//! is set up from one file read plus a `stat` per manifest instead of opening
//! every `package.json` (on macOS, file opens can be serialized by
//! endpoint-security software).
//!
//! Same rules as [`crate::hash::HashCache`]: an entry is used only when the
//! file's stat is unchanged, and not persisted when its mtime is within
//! [`crate::hash::RACY_WINDOW`] of the build start. A path read once in a build
//! is answered from memory afterwards.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use turborepo_trace_core::os::MetadataExt;

const MAGIC: &[u8; 8] = b"DTSMALL1";
/// Files larger than this are read but not cached.
const MAX_FILE: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
}

fn stamp(meta: &std::fs::Metadata) -> Stamp {
    Stamp {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.size(),
        mtime_ns: meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128,
    }
}

/// Bytes of a file (`None`: missing or unreadable).
pub type Bytes = Option<Arc<[u8]>>;

#[derive(Clone)]
struct Seen {
    stamp: Option<Stamp>,
    bytes: Bytes,
    persist: bool,
}

/// The cache (see the module docs).
pub struct SmallFiles {
    path: Option<PathBuf>,
    start_ns: i128,
    prev: Stored,
    seen: Mutex<HashMap<PathBuf, Seen>>,
    dirty: AtomicBool,
}

impl std::fmt::Debug for SmallFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmallFiles")
            .field("path", &self.path)
            .field("prev", &self.prev.len())
            .finish_non_exhaustive()
    }
}

impl SmallFiles {
    /// A cache that is not persisted.
    pub fn in_memory() -> Self {
        SmallFiles {
            path: None,
            start_ns: now_ns(),
            prev: HashMap::new(),
            seen: Mutex::default(),
            dirty: AtomicBool::new(false),
        }
    }

    /// The cache stored at `path` (empty when missing or corrupt).
    pub fn open(path: &Path) -> Self {
        let mut c = Self::in_memory();
        c.path = Some(path.to_path_buf());
        if let Ok(b) = std::fs::read(path) {
            c.prev = decode(&b).unwrap_or_default();
        }
        c
    }

    /// The contents of `path` (absolute), from the cache when its stat is
    /// unchanged.
    pub fn read(&self, path: &Path) -> Bytes {
        if let Some(s) = self.lock().get(path) {
            return s.bytes.clone();
        }
        let seen = match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => {
                let st = stamp(&meta);
                let persist = st.mtime_ns
                    < self.start_ns - crate::hash::RACY_WINDOW.as_nanos() as i128
                    && (st.size as usize) <= MAX_FILE;
                match self.prev.get(path) {
                    Some((s, b)) if *s == st => Seen {
                        stamp: Some(st),
                        bytes: Some(Arc::clone(b)),
                        persist,
                    },
                    _ => {
                        let bytes: Bytes = std::fs::read(path).ok().map(Arc::from);
                        // Re-stat: a write between the stat and the read must not be cached
                        // under the old stamp.
                        let same = std::fs::metadata(path).is_ok_and(|m| stamp(&m) == st);
                        let persist = persist && same && bytes.is_some();
                        if persist {
                            self.dirty.store(true, Ordering::Relaxed);
                        }
                        Seen {
                            stamp: Some(st),
                            bytes,
                            persist,
                        }
                    }
                }
            }
            _ => Seen {
                stamp: None,
                bytes: None,
                persist: false,
            },
        };
        let mut m = self.lock();
        m.entry(path.to_path_buf()).or_insert(seen).bytes.clone()
    }

    /// Reads `paths` in parallel, in order.
    pub fn read_many(&self, paths: &[PathBuf]) -> Vec<Bytes> {
        use rayon::prelude::*;
        paths.par_iter().map(|p| self.read(p)).collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, Seen>> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes the files read in this build (persistable ones) when that changes
    /// the stored cache.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let m = self.lock();
        let keep: Vec<(&PathBuf, &Seen)> = m
            .iter()
            .filter(|(_, s)| s.persist && s.stamp.is_some() && s.bytes.is_some())
            .collect();
        if !self.dirty.load(Ordering::Relaxed) && keep.len() == self.prev.len() && path.exists() {
            return Ok(());
        }
        let mut keep = keep;
        keep.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(keep.len() as u64).to_le_bytes());
        for (p, s) in keep {
            let (Some(st), Some(b)) = (s.stamp, s.bytes.as_ref()) else {
                continue;
            };
            let p = p.to_string_lossy();
            out.extend_from_slice(&(p.len() as u64).to_le_bytes());
            out.extend_from_slice(p.as_bytes());
            out.extend_from_slice(&st.dev.to_le_bytes());
            out.extend_from_slice(&st.ino.to_le_bytes());
            out.extend_from_slice(&st.size.to_le_bytes());
            out.extend_from_slice(&st.mtime_ns.to_le_bytes());
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(b);
        }
        let sum = xxhash_rust::xxh3::xxh3_64(&out);
        out.extend_from_slice(&sum.to_le_bytes());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::write_atomic(path, &out)
    }
}

/// Stored files: stamp and contents by path.
type Stored = HashMap<PathBuf, (Stamp, Arc<[u8]>)>;

fn decode(b: &[u8]) -> Option<Stored> {
    if b.len() < 24 || &b[..8] != MAGIC {
        return None;
    }
    let (body, sum) = b.split_at(b.len() - 8);
    if xxhash_rust::xxh3::xxh3_64(body).to_le_bytes() != sum {
        return None;
    }
    let mut pos = 8usize;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let s = body.get(*pos..*pos + n)?;
        *pos += n;
        Some(s)
    };
    let u64_ = |pos: &mut usize| -> Option<u64> {
        Some(u64::from_le_bytes(take(pos, 8)?.try_into().ok()?))
    };
    let n = u64_(&mut pos)? as usize;
    let mut map = HashMap::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let pl = u64_(&mut pos)? as usize;
        let p = std::str::from_utf8(take(&mut pos, pl)?).ok()?;
        let dev = u64_(&mut pos)?;
        let ino = u64_(&mut pos)?;
        let size = u64_(&mut pos)?;
        let mtime_ns = i128::from_le_bytes(take(&mut pos, 16)?.try_into().ok()?);
        let bl = u64_(&mut pos)? as usize;
        let bytes: Arc<[u8]> = Arc::from(take(&mut pos, bl)?);
        map.insert(
            PathBuf::from(p),
            (
                Stamp {
                    dev,
                    ino,
                    size,
                    mtime_ns,
                },
                bytes,
            ),
        );
    }
    Some(map)
}

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn age(f: &Path) {
        let old = SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(f)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    #[test]
    fn serves_unchanged_files_and_rereads_changed_ones() {
        let t = tempfile::tempdir().unwrap();
        let (a, b, gone) = (t.path().join("a"), t.path().join("b"), t.path().join("c"));
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();
        age(&a);
        let db = t.path().join("cache/manifests");
        let c = SmallFiles::open(&db);
        assert_eq!(c.read(&a).as_deref(), Some(&b"one"[..]));
        assert_eq!(c.read(&b).as_deref(), Some(&b"two"[..]));
        assert_eq!(c.read(&gone), None);
        c.save().unwrap();
        let c = SmallFiles::open(&db);
        // `a` is persisted (old), `b` is racy (just written) and is not.
        assert_eq!(c.prev.len(), 1);
        assert!(c.prev.contains_key(&a));
        // Same size, different content and mtime: re-read.
        std::fs::write(&a, "uno").unwrap();
        age(&a);
        std::fs::File::options()
            .write(true)
            .open(&a)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(30))
            .unwrap();
        assert_eq!(c.read(&a).as_deref(), Some(&b"uno"[..]));
        std::fs::remove_file(&b).unwrap();
        assert_eq!(c.read(&b), None);
        c.save().unwrap();
        let c = SmallFiles::open(&db);
        assert_eq!(c.read(&a).as_deref(), Some(&b"uno"[..]));
    }

    #[test]
    fn a_corrupt_cache_is_ignored() {
        let t = tempfile::tempdir().unwrap();
        let db = t.path().join("m");
        std::fs::write(&db, b"DTSMALL1garbage-garbage-garbage").unwrap();
        let c = SmallFiles::open(&db);
        assert!(c.prev.is_empty());
    }
}
