//! One flattened index of the repository tree per build, for deep membership
//! listings.
//!
//! A deep listing (`**/__mocks__/**` in a directory, `**/package.json` at the
//! root) descends every subdirectory but `node_modules`, `.git` and
//! `.turbotrace` through the build's [`DirCache`]
//! ([`crate::build::membership_listing_in`]). A changed tree invalidates the
//! deep listings that cover it, and big repositories recompute hundreds of
//! them, each a traversal of most of the tree. The index makes that traversal
//! once: every entry reachable by that descent from the repository root, sorted
//! by repo-relative path, so a directory's subtree is one contiguous range. A
//! listing is then a range scan with the same literal prefilter and glob, over
//! the same `DirCache` listings, so its result is identical.
//!
//! Most deep patterns say more than "somewhere below": a path component every
//! match has (`__mocks__` in `**/__mocks__/**`, `a` in `**/a/__init__.py*`),
//! the name of the entry (`**/package.json`) or how that name starts
//! (`**/six.py*`). The index keeps its entries by name as well (by the hash of
//! the name up to its first `.`: [`stem`]), so such a listing looks at the
//! entries with that name (and what lies below them) instead of at the
//! whole range ([`Narrow`]): a repository-wide listing for a name that is
//! nowhere costs a binary search, not a scan of the tree. The entries a listing
//! looks at are a superset of its matches, and the glob decides as before.
//!
//! The index is built on a background thread with its own thread pool, started
//! when the tree changed; listings wait for it (never on a rayon worker of the
//! waiting pool: the builder does not use the global pool).

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use rayon::prelude::*;
use turborepo_trace_core::fs::{DirCache, EntryKind};

/// Entries beyond this: no index (the listings fall back to their own
/// traversal).
const MAX_ENTRIES: usize = 4_000_000;

const DIR: u8 = 1;
/// A directory whose entries are in the index (not `node_modules`, `.git`,
/// `.turbotrace`).
const DESCENDED: u8 = 2;
/// A descended directory of the walk.
const WALKED: u8 = 4;

/// The index (see the module docs).
pub(crate) struct TreeIndex {
    root: PathBuf,
    paths: Vec<String>,
    flags: Vec<u8>,
    /// The entries by the hash of their name's [`stem`], then by position.
    by_stem: Vec<(u64, u32)>,
    /// The directories the index descends into that the walk did not, by
    /// position.
    outside: Vec<u32>,
}

/// What a pattern says about every path it matches, besides the glob itself:
/// where to look in the index. Patterns with alternations, classes, negations
/// or escapes say nothing here (as for [`crate::build::required_literal`]).
#[derive(Debug, Default, PartialEq, Eq)]
struct Narrow<'a> {
    /// Whole path components: the segments of the pattern without a wildcard,
    /// but the last.
    components: Vec<&'a str>,
    /// The base name, when the last segment has no wildcard.
    name: Option<&'a str>,
    /// How the base name starts, when the last segment has one (`six.py` of
    /// `six.py*`).
    head: Option<&'a str>,
}

impl<'a> Narrow<'a> {
    fn of(pattern: &'a str) -> Narrow<'a> {
        let mut out = Narrow::default();
        if pattern.contains(['{', '}', '[', ']', '!', '\\', '(', ')', '+', '@']) {
            return out;
        }
        let wild = |s: &str| s.contains(['*', '?']);
        let mut segments: Vec<&str> = pattern.split('/').collect();
        // `*` and `?` never match `/`, and `**` is a segment of its own (or, written
        // inside a segment, a `*`): what the last segment matches is the base
        // name.
        match segments.pop() {
            Some(last) if !wild(last) => out.name = Some(last).filter(|l| !l.is_empty()),
            Some(last) => {
                let head = &last[..last.find(['*', '?']).unwrap_or(last.len())];
                out.head = Some(head).filter(|h| !h.is_empty());
            }
            None => {}
        }
        out.components = segments
            .into_iter()
            .filter(|s| !s.is_empty() && !wild(s))
            .collect();
        out
    }
}

fn descends(name: &str) -> bool {
    !matches!(name, "node_modules" | ".git" | ".turbotrace")
}

impl TreeIndex {
    /// Indexes the tree below `root` through `fs` on `threads` threads of a
    /// private pool.
    pub fn build(
        fs: &DirCache,
        root: &Path,
        walked: &std::collections::HashSet<PathBuf>,
        threads: usize,
    ) -> Option<TreeIndex> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .thread_name(|i| format!("turbo-trace-treeindex-{i}"))
            .build()
            .ok()?;
        let count = std::sync::atomic::AtomicUsize::new(0);
        // Everything parallel runs in the private pool: the global pool's workers may
        // all be waiting for this index.
        let entries: Vec<(String, u8)> = pool.install(|| {
            let mut e = collect(fs, root, "", walked, &count)?;
            e.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));
            Some(e)
        })?;
        let (paths, flags): (Vec<String>, Vec<u8>) = entries.into_iter().unzip();
        u32::try_from(paths.len()).ok()?;
        let by_stem = pool.install(|| {
            let mut v: Vec<(u64, u32)> = paths
                .par_iter()
                .enumerate()
                .map(|(i, p)| (stem_hash(stem(base_name(p))), i as u32))
                .collect();
            v.par_sort_unstable();
            v
        });
        let outside = (0..paths.len() as u32)
            .filter(|&i| {
                let f = flags[i as usize];
                f & DESCENDED != 0 && f & WALKED == 0
            })
            .collect();
        Some(TreeIndex {
            root: root.to_path_buf(),
            paths,
            flags,
            by_stem,
            outside,
        })
    }

    /// The positions of the entries below the directory at position `i`: `None`
    /// when the index did not descend into it (or it is no directory).
    fn below(&self, i: usize) -> Option<(usize, usize)> {
        if self.flags[i] & DESCENDED == 0 {
            return None;
        }
        // Entries such as `a-b` and `a.d` sort between `a` and `a/…`.
        let prefix = format!("{}/", self.paths[i]);
        let lo = i + 1 + self.paths[i + 1..].partition_point(|p| p.as_str() < prefix.as_str());
        let n = self.paths[lo..].partition_point(|p| p.starts_with(prefix.as_str()));
        Some((lo, lo + n))
    }

    /// The positions in `lo..hi` of the entries with the stem `stem` whose name
    /// `keep` accepts; `None` when more than `limit` entries have the stem.
    fn named(
        &self,
        stem: &str,
        (lo, hi): (usize, usize),
        limit: usize,
        keep: impl Fn(&str) -> bool,
    ) -> Option<Vec<u32>> {
        let h = stem_hash(stem);
        let key = |&(x, i): &(u64, u32)| (x, i as usize);
        let first = self.by_stem.partition_point(|e| key(e) < (h, lo));
        let n = self.by_stem[first..].partition_point(|e| key(e) < (h, hi));
        if n > limit {
            return None;
        }
        Some(
            self.by_stem[first..first + n]
                .iter()
                .map(|&(_, i)| i)
                .filter(|&i| keep(base_name(&self.paths[i as usize])))
                .collect(),
        )
    }

    /// The positions in `lo..hi` that a match of a pattern with `narrow` can
    /// have, as ascending disjoint ranges; `None` when the pattern does not
    /// narrow the range to a fraction of it (a scan of the range is as
    /// good).
    fn candidates(&self, narrow: &Narrow<'_>, lo: usize, hi: usize) -> Option<Vec<(usize, usize)>> {
        // An entry by its own name costs one glob match; an entry on the way costs two
        // searches for what lies below it, and a common name (`src`) covers most of the
        // range anyway.
        let own_limit = ((hi - lo) / 8).max(8);
        let way_limit = ((hi - lo) / 256).max(8);
        // The entry itself, by its name or the head of its name.
        let own: Option<Vec<u32>> = match (narrow.name, narrow.head) {
            (Some(n), _) => self.named(stem(n), (lo, hi), own_limit, |x| x == n),
            (None, Some(h)) => stem_of_head(h)
                .and_then(|s| self.named(s, (lo, hi), own_limit, |x| x.starts_with(h))),
            (None, None) => None,
        };
        // An entry on the way, by its name: the rarest.
        let on_the_way: Option<Vec<u32>> = narrow
            .components
            .iter()
            .filter_map(|c| self.named(stem(c), (lo, hi), own_limit, |x| x == *c))
            .filter(|w| w.len() <= way_limit)
            .min_by_key(Vec::len);
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        match (own, on_the_way) {
            // The segment is followed by another: a match lies below such an entry.
            (own, Some(way)) if own.as_ref().is_none_or(|o| way.len() < o.len()) => {
                ranges.extend(way.iter().filter_map(|&i| self.below(i as usize)));
            }
            (Some(own), _) => ranges.extend(own.iter().map(|&i| (i as usize, i as usize + 1))),
            (None, _) => return None,
        }
        ranges.sort_unstable();
        // Directories of one name nest (`a/x/a`): one range per entry.
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for (s, e) in ranges {
            match merged.last_mut() {
                Some(last) if s < last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        Some(merged)
    }

    /// The listing [`crate::build::membership_listing_in`] computes for a deep
    /// pattern (its `glob` and required literal `lit`) in `dir` with the
    /// walked directories the index was built with; `None` when the index
    /// cannot answer (a directory it does not descend into, a listing over the
    /// budget, too many outside directories).
    pub fn listing(
        &self,
        dir: &Path,
        glob: Option<&turborepo_trace_graph::glob::Glob>,
        lit: Option<&str>,
        budget: usize,
        max_outside: usize,
    ) -> Option<(Vec<String>, Option<Vec<PathBuf>>)> {
        let rel = dir.strip_prefix(&self.root).ok()?.to_str()?;
        let (lo, hi, plen) = if rel.is_empty() {
            (0, self.paths.len(), 0)
        } else {
            let i = self.paths.binary_search_by(|p| p.as_str().cmp(rel)).ok()?;
            if self.flags[i] & DESCENDED == 0 {
                return None;
            }
            let prefix = format!("{rel}/");
            let lo = i + 1 + self.paths[i + 1..].partition_point(|p| p.as_str() < prefix.as_str());
            let n = self.paths[lo..].partition_point(|p| p.starts_with(prefix.as_str()));
            (lo, lo + n, prefix.len())
        };
        // The traversal's budget counts every listed entry.
        if hi - lo > budget {
            return None;
        }
        let first = self.outside.partition_point(|&i| (i as usize) < lo);
        let n = self.outside[first..].partition_point(|&i| (i as usize) < hi);
        if n > max_outside {
            return None;
        }
        let mut outside: Vec<PathBuf> = self.outside[first..first + n]
            .iter()
            .map(|&i| self.root.join(&self.paths[i as usize]))
            .collect();
        let mut out = Vec::new();
        let mut scan = |from: usize, to: usize| {
            for i in from..to {
                let p = &self.paths[i][plen..];
                // A literal fragment has no '/': it is in the parent's path or the name.
                if lit.is_some_and(|l| !p.contains(l)) {
                    continue;
                }
                if glob.is_none_or(|g| g.matches(p)) {
                    if self.flags[i] & DIR != 0 {
                        out.push(format!("{p}/"));
                    } else {
                        out.push(p.to_owned());
                    }
                }
            }
        };
        match glob.and_then(|g| self.candidates(&Narrow::of(g.as_str()), lo, hi)) {
            Some(ranges) => ranges.into_iter().for_each(|(s, e)| scan(s, e)),
            None => scan(lo, hi),
        }
        out.sort_unstable();
        outside.sort();
        Some((out, Some(outside)))
    }
}

/// The last component of a repo-relative path.
fn base_name(path: &str) -> &str {
    &path[path.rfind('/').map_or(0, |i| i + 1)..]
}

/// What the entries a pattern can name have in common: the name up to its first
/// `.`, a leading `.` aside (`.env.local` is `.env`). The module files
/// `six.py`, `six.pyc` and `six.abi3.so` share it, as do `package.json` and a
/// directory `package`.
fn stem(name: &str) -> &str {
    match name.bytes().skip(1).position(|b| b == b'.') {
        Some(i) => &name[..i + 1],
        None => name,
    }
}

/// The stem of every name that starts with `head`, if `head` says it (`six.py`
/// does, `six` does not: `sixty.py` starts with it).
fn stem_of_head(head: &str) -> Option<&str> {
    let s = stem(head);
    (s.len() < head.len()).then_some(s)
}

fn stem_hash(stem: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(stem.as_bytes())
}

fn collect(
    fs: &DirCache,
    d: &Path,
    rel: &str,
    walked: &std::collections::HashSet<PathBuf>,
    count: &std::sync::atomic::AtomicUsize,
) -> Option<Vec<(String, u8)>> {
    let Some(listing) = fs.list(d) else {
        return Some(Vec::new());
    };
    if count.fetch_add(listing.len(), std::sync::atomic::Ordering::Relaxed) > MAX_ENTRIES {
        return None;
    }
    let mut out = Vec::with_capacity(listing.len());
    let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
    for e in listing.iter() {
        let name = e.name().to_string_lossy();
        let r = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        let mut f = 0;
        if matches!(e.kind(), EntryKind::Dir) {
            f |= DIR;
            if descends(&name) {
                f |= DESCENDED;
                let sub = d.join(e.name());
                if walked.contains(&sub) {
                    f |= WALKED;
                }
                subdirs.push((sub, r.clone()));
            }
        }
        out.push((r, f));
    }
    let nested: Vec<Option<Vec<(String, u8)>>> = subdirs
        .par_iter()
        .map(|(p, r)| collect(fs, p, r, walked, count))
        .collect();
    for n in nested {
        out.extend(n?);
    }
    Some(out)
}

/// The index of a build: started in the background, taken by the first listing.
#[derive(Default)]
pub(crate) struct Slot {
    handle: Mutex<Option<std::thread::JoinHandle<Option<TreeIndex>>>>,
    index: OnceLock<Option<Arc<TreeIndex>>>,
}

impl Slot {
    /// Starts building the index of `root`.
    pub fn start(
        &self,
        fs: Arc<DirCache>,
        root: PathBuf,
        walked: std::collections::HashSet<PathBuf>,
    ) {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let h = std::thread::Builder::new()
            .name("turbo-trace-treeindex".into())
            .spawn(move || TreeIndex::build(&fs, &root, &walked, threads));
        if let Ok(h) = h {
            *self.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
        }
    }

    /// The index, waiting for it if it is being built; `None` if it was not
    /// started or could not be built.
    pub fn get(&self) -> Option<&TreeIndex> {
        self.index
            .get_or_init(|| {
                let h = self
                    .handle
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()?;
                h.join().ok().flatten().map(Arc::new)
            })
            .as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::membership_listing_in;

    #[test]
    fn index_listings_equal_traversal_listings() {
        let t = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(t.path()).unwrap();
        for f in [
            "package.json",
            "a/package.json",
            "a/src/__mocks__/x.js",
            "a/src/b/__mocks__/y.js",
            "a/src/b/z.ts",
            "a/src/b/z.snap",
            "a-b/package.json",
            "a.d/c.ts",
            "a/node_modules/dep/package.json",
            "node_modules/x/package.json",
            ".git/HEAD",
            "dist/out/package.json",
            "dist/out/__mocks__/m.js",
            "services/s/production.yaml",
            "services/s/deep/er/production.yaml",
            "__mocks__/r.js",
        ] {
            let p = r.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let fs = DirCache::new();
        // `dist` is not walked (ignored): its directories are outside covers.
        let walked: std::collections::HashSet<PathBuf> = [
            "",
            "a",
            "a/src",
            "a/src/__mocks__",
            "a/src/b",
            "a/src/b/__mocks__",
            "a-b",
            "a.d",
            "services",
            "services/s",
            "services/s/deep",
            "services/s/deep/er",
            "__mocks__",
        ]
        .iter()
        .map(|d| if d.is_empty() { r.clone() } else { r.join(d) })
        .collect();
        let ix = TreeIndex::build(&fs, &r, &walked, 2).unwrap();
        for dir in [
            "",
            "a",
            "a/src",
            "dist",
            "services",
            "a/node_modules",
            "missing",
        ] {
            for pattern in [
                "**",
                "**/package.json",
                "**/__mocks__/**",
                "**/*.{snap,ts}",
                "*/**/*/production.yaml",
                "*/package.json",
                "src/**",
                "**/b",
                // Narrowed by a component on the way, by the name, by the head of the name.
                "**/b/**",
                "**/a/**",
                "**/src/b/z.*",
                "**/z.t*",
                "**/z.*",
                "**/er/production.yaml",
                "**/s/**/production.yaml",
                "**/a*/package.json",
                "a/src/**",
                "**/__mocks__/*.js",
                "**/out/__mocks__/**",
                "**/a",
                "**/a/",
                "**/nowhere.py*",
                "**/nowhere/__init__.py*",
            ] {
                let d = if dir.is_empty() {
                    r.clone()
                } else {
                    r.join(dir)
                };
                let want = membership_listing_in(&fs, &d, pattern, Some(&walked));
                let got = ix.listing(
                    &d,
                    turborepo_trace_graph::glob::Glob::new(pattern)
                        .ok()
                        .as_ref(),
                    crate::build::required_literal(pattern),
                    200_000,
                    100_000,
                );
                match got {
                    Some(got) => assert_eq!(got, want, "{dir:?} {pattern:?}"),
                    // Only directories the index does not descend into fall back.
                    None => assert!(
                        matches!(dir, "a/node_modules" | "missing"),
                        "{dir:?} {pattern:?}"
                    ),
                }
            }
        }
        // Over budget: falls back (the traversal truncates).
        assert!(ix.listing(&r, None, None, 3, 100).is_none());
    }

    #[test]
    fn what_a_pattern_says_about_its_matches() {
        let n = |components: &[&'static str], name, head| Narrow {
            components: components.to_vec(),
            name,
            head,
        };
        assert_eq!(Narrow::of("**/six.py*"), n(&[], None, Some("six.py")));
        assert_eq!(Narrow::of("**/six.*so"), n(&[], None, Some("six.")));
        assert_eq!(
            Narrow::of("**/a/__init__.py*"),
            n(&["a"], None, Some("__init__.py"))
        );
        assert_eq!(Narrow::of("**/a/b/**"), n(&["a", "b"], None, None));
        assert_eq!(
            Narrow::of("**/package.json"),
            n(&[], Some("package.json"), None)
        );
        assert_eq!(Narrow::of("src/**"), n(&["src"], None, None));
        assert_eq!(Narrow::of("*/a?/x/*.ts"), n(&["x"], None, None));
        assert_eq!(Narrow::of("a/"), n(&["a"], None, None));
        // Nothing to go by: no literal, or a syntax this does not read.
        assert_eq!(Narrow::of("**"), Narrow::default());
        assert_eq!(Narrow::of("**/*.snap"), Narrow::default());
        assert_eq!(Narrow::of("**/a/*.{snap,ts}"), Narrow::default());
        assert_eq!(Narrow::of("**/[ab]/x"), Narrow::default());
    }

    #[test]
    fn stems() {
        assert_eq!(stem("six.py"), "six");
        assert_eq!(stem("six.cpython-312-x86_64-linux-gnu.so"), "six");
        assert_eq!(stem("package.json"), "package");
        assert_eq!(stem("src"), "src");
        assert_eq!(stem(".env.local"), ".env");
        assert_eq!(stem(".gitignore"), ".gitignore");
        assert_eq!(stem("."), ".");
        assert_eq!(stem(""), "");
        assert_eq!(stem("é.py"), "é");
        // Every name that starts with the head has the head's stem, or the head does
        // not say.
        assert_eq!(stem_of_head("six.py"), Some("six"));
        assert_eq!(stem_of_head("six."), Some("six"));
        assert_eq!(stem_of_head("__init__.py"), Some("__init__"));
        assert_eq!(stem_of_head(".env."), Some(".env"));
        assert_eq!(stem_of_head("six"), None);
        assert_eq!(stem_of_head(".env"), None);
        assert_eq!(stem_of_head("."), None);
    }

    /// A tree of few names at many places (names that nest, names that sort
    /// between a directory and its entries, module-like files): every
    /// listing through the index, narrowed or not, is the traversal's.
    #[test]
    fn narrowed_listings_equal_traversal_listings() {
        let t = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(t.path()).unwrap();
        let dirs = ["a", "b", "a-b", "a.d", "src", "node_modules", "pkg"];
        let files = [
            "a.py",
            "a.pyc",
            "a.ts",
            "a",
            "b.cpython-312-x86_64-linux-gnu.so",
            "__init__.py",
            "package.json",
            "b",
            "pkg.pyd",
        ];
        // A fixed pseudo-random sequence.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut all_dirs: Vec<String> = vec![String::new()];
        for _ in 0..400 {
            let depth = 1 + next(5);
            let mut d = String::new();
            let mut blocked = false;
            for _ in 0..depth {
                let sub = if d.is_empty() {
                    dirs[next(dirs.len())].to_owned()
                } else {
                    format!("{d}/{}", dirs[next(dirs.len())])
                };
                // A file of that name may be there already.
                if r.join(&sub).is_file() {
                    blocked = true;
                    break;
                }
                d = sub;
                if !all_dirs.contains(&d) {
                    all_dirs.push(d.clone());
                }
            }
            let f = r.join(&d).join(files[next(files.len())]);
            if blocked || f.is_dir() {
                continue;
            }
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "").unwrap();
        }
        // Names at few places: what narrowing is for.
        for f in [
            "zone/a/rare/a/x.py",
            "zone/a/rare/a.d/rare/y.py",
            "zone/rare/b/z.py",
            "zone/a-b/rare-not/x.py",
            "zone/pkg/once/__init__.py",
            "zone/a/b/lone.py",
            "zone/lone.pyc",
            "zone/src/lone/a.py",
        ] {
            let f = r.join(f);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "").unwrap();
        }
        for d in [
            "zone",
            "zone/a",
            "zone/a/rare",
            "zone/a/rare/a.d",
            "zone/rare",
            "zone/pkg/once",
            "zone/src",
        ] {
            all_dirs.push(d.to_owned());
        }
        let fs = DirCache::new();
        // Every third directory is outside the walk.
        let walked: std::collections::HashSet<PathBuf> = all_dirs
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 3 != 2)
            .map(|(_, d)| if d.is_empty() { r.clone() } else { r.join(d) })
            .collect();
        let ix = TreeIndex::build(&fs, &r, &walked, 2).unwrap();
        let patterns = [
            "**/a.py*",
            "**/a.*so",
            "**/b.*so",
            "**/pkg.*pyd",
            "**/a/__init__.py*",
            "**/pkg/__init__.py*",
            "**/a/b.py*",
            "**/a/b.*so",
            "**/a/b/**",
            "**/a-b/**",
            "**/a.d/a/**",
            "**/src/**",
            "**/package.json",
            "**/a",
            "**/b",
            "**/a/a",
            "**/a/*",
            "a/**",
            "a/b/**",
            "*/a/**",
            "**/pkg/**/a.py",
            "**/node_modules/**",
            "**/a?b/**",
            "**/zzz.py*",
            "**/zzz/**",
            "**/rare/**",
            "**/rare/a/**",
            "**/rare/*/*.py",
            "**/once/__init__.py*",
            "**/lone.py*",
            "**/lone/a.py*",
            "**/a/b/lone.py",
            "**",
        ];
        let missing = "missing".to_owned();
        let mut answered = 0;
        for dir in all_dirs.iter().chain([&missing]) {
            let d = if dir.is_empty() {
                r.clone()
            } else {
                r.join(dir)
            };
            for pattern in patterns {
                let want = membership_listing_in(&fs, &d, pattern, Some(&walked));
                let got = ix.listing(
                    &d,
                    turborepo_trace_graph::glob::Glob::new(pattern)
                        .ok()
                        .as_ref(),
                    crate::build::required_literal(pattern),
                    200_000,
                    100_000,
                );
                if let Some(got) = got {
                    answered += 1;
                    assert_eq!(got, want, "{dir:?} {pattern:?}");
                } else {
                    assert!(
                        dir == "missing" || dir.split('/').any(|c| c == "node_modules"),
                        "{dir:?} {pattern:?}"
                    );
                }
            }
        }
        assert!(answered > 500, "{answered}");
        // The repository-wide listings found something, and narrowing was at work.
        let root = |pattern: &str| {
            ix.listing(
                &r,
                turborepo_trace_graph::glob::Glob::new(pattern)
                    .ok()
                    .as_ref(),
                crate::build::required_literal(pattern),
                200_000,
                100_000,
            )
            .unwrap()
            .0
        };
        assert!(!root("**/a.py*").is_empty());
        assert!(!root("**/a/__init__.py*").is_empty());
        assert!(root("**/zzz.py*").is_empty());
        let (lo, hi) = (0, ix.paths.len());
        // (Pattern, the entries a narrowed listing looks at, or none: a scan.)
        let looked_at = |pattern: &str| {
            ix.candidates(&Narrow::of(pattern), lo, hi)
                .map(|ranges| ranges.iter().map(|(s, e)| e - s).sum::<usize>())
        };
        assert_eq!(looked_at("**/zzz.py*"), Some(0));
        assert_eq!(looked_at("**/zzz/**"), Some(0));
        assert_eq!(looked_at("**/lone.py*"), Some(2));
        assert_eq!(looked_at("**/a/b/lone.py"), Some(1));
        // Below `a/rare`, `a/rare/a.d/rare` (inside the first) and `rare`.
        assert_eq!(looked_at("**/rare/**"), Some(root("**/rare/**").len()));
        assert_eq!(root("**/rare/**").len(), 7);
        assert_eq!(looked_at("**/once/__init__.py*"), Some(1));
        assert_eq!(looked_at("**"), None);
        assert_eq!(looked_at("**/*.py"), None);
        // A name at many places: a scan.
        assert_eq!(looked_at("**/a/**"), None);
    }

    /// Every worker of the global pool waits for the index: building it must
    /// not need them.
    #[test]
    fn waiting_global_workers_do_not_block_the_index() {
        let t = tempfile::tempdir().unwrap();
        let r = std::fs::canonicalize(t.path()).unwrap();
        for i in 0..64 {
            let p = r.join(format!("d{}/e{}/f.txt", i % 8, i));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let fs = Arc::new(DirCache::new());
            let slot = Slot::default();
            slot.start(Arc::clone(&fs), r.clone(), Default::default());
            let n = rayon::current_num_threads() * 2;
            let found: Vec<bool> = (0..n)
                .into_par_iter()
                .map(|_| slot.get().is_some())
                .collect();
            let _ = tx.send(found.iter().all(|&b| b));
        });
        let ok = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("tree index deadlocked");
        assert!(ok);
    }
}
