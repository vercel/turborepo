//! Recording of filesystem observations for graph building.
//!
//! In graph mode every reference is resolved inside a recording frame
//! ([`start`] / [`Frame::finish`]). Whatever the resolution observed about the
//! filesystem lands in the frame's [`Observations`]:
//!
//! - **missing paths** (probes): a path that was looked for and not found.
//!   Adding a file at that path, or below it, can change the resolution. So can
//!   a directory above it that goes or comes (a lookup that asks "is it a
//!   directory?" goes inside only when it is one, and what it then looks for is
//!   what it records): the graph store keeps, as the state of a probe, whether
//!   its directory is there and what is at the path (`turborepo_trace_store`'s
//!   `normalize::DIR_ABSENT`), and a query matches a changed path with every
//!   probe at, above or below it;
//! - **listings** (directory memberships): a directory whose contents, filtered
//!   by a glob relative to it, the resolution enumerated;
//! - **reads** (config inputs): files whose contents the resolution read, such
//!   as `package.json` or `tsconfig.json`;
//! - **path reads** (config inputs keyed by path): files the result depends on
//!   by path only, not by contents (the manifests of a task's transitive
//!   workspace dependencies, whose relevant fields a separate digest covers).
//!   They are inputs for queries like reads, but their contents do not enter
//!   the shard key;
//! - **view reads** (config inputs keyed by a view): manifests that resolution
//!   read through their [`crate::manifest::view`]. The reader got the view's
//!   bytes, so its result depends on those and not on the rest of the file;
//! - **loader reads**: manifests that the loader of a `tsconfig.json` read,
//!   through the narrower [`crate::manifest::loader_view`] (what the resolution
//!   of an `extends` entry can read of a manifest);
//! - **bare package names** that were looked up, which lets the store collapse
//!   `node_modules` walk-up probes into one record per package;
//! - **links** ([`link`]): symbolic links of the tree that an observed path
//!   went through. The observation is recorded at the real path (the one an
//!   edit names), and where each link leads is part of what was observed;
//! - **key observations** ([`key`]): what the result was computed from without
//!   being an input of it. A change of one makes the result stale (the graph
//!   store computes it again), and no query selects through it. For what
//!   decides a record by a view the reader does not otherwise depend on:
//!   whether a project that a `tsconfig.json` references loads, and which files
//!   it includes, decide the config of the files below the referencing config,
//!   and nothing else of the project reaches them. They are recorded as shared
//!   sets (one record for every file below that config, held by reference), so
//!   that a file names a set instead of repeating it;
//! - **aliased** ([`aliased`]): a fact, not a path. The file the resolution is
//!   for is governed by a `tsconfig.json` or `jsconfig.json` that gives
//!   `paths`, `baseUrl` or `rootDirs`, whether the resolver applied them or not
//!   (it does not below a directory called `node_modules`). Where the checkout
//!   lies then decides the file's records, so the graph store keys such a file
//!   by whether the checkout is below `node_modules`.
//!
//! The [`crate::DirCache`] records automatically when it was created with
//! [`crate::DirCache::with_recording`]: misses of the public existence queries, listings
//! through [`crate::DirCache::list`] and [`crate::DirCache::walk_files`], and
//! [`crate::DirCache::read_cached`] reads. Resolvers add what the cache cannot
//! see, for example probes a third-party resolver reports from behind its own
//! memo tables, and can [`pause`] the automatic recording around such calls so
//! that only their complete, deterministic report is kept.
//!
//! Cost: a cache without recording never touches this module (one predictable
//! branch on a plain field per query). With recording, each hook is one
//! thread-local access.
//!
//! Frames nest (a stack per thread), so work stolen by a rayon worker while a
//! frame is open records into its own frame.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
};

/// What one resolution observed. Paths are absolute and normalized, in
/// recording order, possibly with duplicates; the store sorts, deduplicates and
/// compacts them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observations {
    /// Paths looked for and not found.
    pub missing: Vec<PathBuf>,
    /// `(directory, glob relative to it)` listings the resolution depended on.
    /// `**` covers the whole subtree.
    pub listings: Vec<(PathBuf, String)>,
    /// Files whose contents were read (manifests, configs).
    pub reads: Vec<PathBuf>,
    /// Inputs depended on by path only (contents not keyed); a path also in
    /// `reads` is a content input.
    pub path_reads: Vec<PathBuf>,
    /// Manifests read through their resolution view
    /// ([`crate::manifest::view`]): the reader depends on the view's bytes,
    /// not on the file's.
    pub view_reads: Vec<PathBuf>,
    /// Manifests read through the view of a config loader
    /// ([`crate::manifest::loader_view`], [`loader_read`]): the load depends on
    /// those bytes, a part of the resolution view.
    pub loader_reads: Vec<PathBuf>,
    /// Bare package names looked up (`react`, `@scope/pkg`).
    pub packages: Vec<String>,
    /// Symbolic links crossed on the way to an observed path ([`link`]): where
    /// each leads is part of what was observed.
    pub links: Vec<PathBuf>,
    /// Key observations ([`key`]): sets of observations that decide the result
    /// and are no inputs of it (see the module docs). Each set is shared by
    /// every resolution that depends on it; a set has no key observations
    /// of its own.
    pub key: Vec<Arc<Observations>>,
    /// The fact [`aliased`]: the file is governed by a config with aliases.
    pub aliased: bool,
}

impl Observations {
    /// Whether nothing was observed.
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
            && self.listings.is_empty()
            && self.reads.is_empty()
            && self.path_reads.is_empty()
            && self.view_reads.is_empty()
            && self.loader_reads.is_empty()
            && self.packages.is_empty()
            && self.links.is_empty()
            && self.key.iter().all(|k| k.is_empty())
            && !self.aliased
    }

    /// Appends everything from `other`.
    pub fn extend(&mut self, other: Observations) {
        self.missing.extend(other.missing);
        self.listings.extend(other.listings);
        self.reads.extend(other.reads);
        self.path_reads.extend(other.path_reads);
        self.view_reads.extend(other.view_reads);
        self.loader_reads.extend(other.loader_reads);
        self.packages.extend(other.packages);
        self.links.extend(other.links);
        self.key.extend(other.key);
        self.aliased |= other.aliased;
    }

    /// Appends clones of everything in `other`.
    pub fn extend_from(&mut self, other: &Observations) {
        self.missing.extend_from_slice(&other.missing);
        self.listings.extend_from_slice(&other.listings);
        self.reads.extend_from_slice(&other.reads);
        self.path_reads.extend_from_slice(&other.path_reads);
        self.view_reads.extend_from_slice(&other.view_reads);
        self.loader_reads.extend_from_slice(&other.loader_reads);
        self.packages.extend_from_slice(&other.packages);
        self.links.extend_from_slice(&other.links);
        self.key.extend_from_slice(&other.key);
        self.aliased |= other.aliased;
    }

    /// Removes repeated entries, keeping the first occurrence of each (so
    /// recording order is otherwise preserved). Memoized steps replayed by
    /// several lookups of one reference ([`crate::memo::Memo`]) report the
    /// same paths many times; the scheduler calls this once per reference
    /// group.
    pub fn dedup(&mut self) {
        fn dedup_vec<T: Eq + std::hash::Hash + Clone>(v: &mut Vec<T>) {
            if v.len() < 2 {
                return;
            }
            let mut seen = std::collections::HashSet::with_capacity(v.len());
            v.retain(|x| seen.insert(x.clone()));
        }
        dedup_vec(&mut self.missing);
        dedup_vec(&mut self.listings);
        dedup_vec(&mut self.reads);
        dedup_vec(&mut self.path_reads);
        dedup_vec(&mut self.view_reads);
        dedup_vec(&mut self.loader_reads);
        dedup_vec(&mut self.packages);
        dedup_vec(&mut self.links);
        // The same set (by identity: a set is recorded by reference), once.
        if self.key.len() > 1 {
            let mut seen = std::collections::HashSet::with_capacity(self.key.len());
            self.key.retain(|k| seen.insert(Arc::as_ptr(k)));
        }
    }
}

#[derive(Default)]
struct State {
    frames: Vec<Observations>,
    /// Nesting depth of [`pause`]: everything is dropped while positive.
    paused: u32,
    /// Nesting depth of [`pause_listings`].
    listings_paused: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// An open recording frame. Close it with [`Frame::finish`]; dropping it
/// discards the observations.
#[must_use = "call `finish` to collect the observations"]
#[derive(Debug)]
pub struct Frame {
    depth: usize,
    done: bool,
}

/// Opens a recording frame on this thread.
pub fn start() -> Frame {
    let depth = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.frames.push(Observations::default());
        s.frames.len()
    });
    Frame { depth, done: false }
}

impl Frame {
    /// Closes the frame and returns what was recorded in it.
    pub fn finish(mut self) -> Observations {
        self.done = true;
        pop(self.depth)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if !self.done {
            pop(self.depth);
        }
    }
}

fn pop(depth: usize) -> Observations {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        debug_assert_eq!(
            s.frames.len(),
            depth,
            "recording frames closed out of order"
        );
        // Frames close in LIFO order; be robust if one leaked anyway.
        s.frames.truncate(depth);
        s.frames.pop().unwrap_or_default()
    })
}

/// Whether a frame is open on this thread and recording is not paused.
pub fn is_recording() -> bool {
    STATE.with(|s| {
        let s = s.borrow();
        !s.frames.is_empty() && s.paused == 0
    })
}

fn with_frame(f: impl FnOnce(&mut Observations)) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.paused > 0 {
            return;
        }
        if let Some(top) = s.frames.last_mut() {
            f(top);
        }
    });
}

/// Records a path that was looked for and not found.
pub fn missing(path: &Path) {
    with_frame(|o| o.missing.push(path.to_path_buf()));
}

/// Records that the contents of `dir` matching `pattern` (relative to `dir`;
/// `**` for the whole subtree) were enumerated.
pub fn listing(dir: &Path, pattern: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.paused > 0 || s.listings_paused > 0 {
            return;
        }
        if let Some(top) = s.frames.last_mut() {
            top.listings.push((dir.to_path_buf(), pattern.to_owned()));
        }
    });
}

/// The pattern of a [`listing`] that matches exactly the entry `name` and
/// nothing else: the name, with a backslash before each character that a
/// pattern reads as syntax (`*`, `?`, `[`, `{` and the backslash). It is the
/// record of a lookup that asked for one name: "is `d/name` a directory?", "is
/// the file `d/name` there?".
///
/// A name is not a pattern. `[id]` (a route segment of Next.js) read as one is
/// a class that matches `i` and `d` and never the directory; recorded as "every
/// entry" instead, the record would move with every file that comes or goes
/// next to it and still not say which name was asked for.
/// (`turborepo_trace_graph::glob::escape` is the same function, for
/// the side that matches a changed path against the record.)
pub fn literal_pattern(name: &str) -> String {
    const SPECIAL: [char; 5] = ['*', '?', '[', '{', '\\'];
    if !name.contains(SPECIAL) {
        return name.to_owned();
    }
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if SPECIAL.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The name that the listing pattern `pattern` asks for when it asks for
/// exactly one (the inverse of [`literal_pattern`]); `None` for a pattern with
/// a wildcard, a class or braces.
pub fn pattern_name(pattern: &str) -> Option<String> {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '*' | '?' | '[' | '{' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// Records a file whose contents were read.
pub fn read(path: &Path) {
    with_frame(|o| o.reads.push(path.to_path_buf()));
}

/// Records a manifest that was read through its resolution view
/// ([`crate::manifest::view`], [`crate::DirCache::read_manifest`]).
pub fn view_read(path: &Path) {
    with_frame(|o| o.view_reads.push(path.to_path_buf()));
}

/// Records a manifest that the loader of a config read through its view
/// ([`crate::manifest::loader_view`],
/// [`crate::DirCache::read_manifest_loader`]).
pub fn loader_read(path: &Path) {
    with_frame(|o| o.loader_reads.push(path.to_path_buf()));
}

/// Records a bare package name that was looked up in package directories.
pub fn package(name: &str) {
    with_frame(|o| o.packages.push(name.to_owned()));
}

/// Records a symbolic link that an observed path went through
/// ([`Observations::links`]). The observation itself is recorded at the place
/// the link leads to, which is the path an edit of it names; the link says that
/// it was reached this way, and a link that leads elsewhere (or goes) is
/// another observation.
pub fn link(path: &Path) {
    with_frame(|o| o.links.push(path.to_path_buf()));
}

/// Records the fact [`Observations::aliased`]: the file being resolved is
/// governed by a `tsconfig.json` or `jsconfig.json` with `paths`, `baseUrl` or
/// `rootDirs`, applied or not.
pub fn aliased() {
    with_frame(|o| o.aliased = true);
}

/// Re-records observations (for example from a nested frame the caller
/// post-processed) into the current frame.
pub fn extend(obs: Observations) {
    with_frame(|o| o.extend(obs));
}

/// [`extend`] from a borrowed record (clones only when a frame is recording).
pub fn extend_from(obs: &Observations) {
    with_frame(|o| o.extend_from(obs));
}

/// Runs `f` in a fresh frame with every pause lifted, and returns its result
/// with everything it observed. Nothing reaches the enclosing frame: pass the
/// observations to [`replay`] (now and on every later use of a memoized
/// result), which applies the pauses in effect at that point. So a memoized
/// computation records exactly what running it again would record, whichever
/// caller happened to compute it first.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, Observations) {
    struct Restore(u32, u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.paused = self.0;
                s.listings_paused = self.1;
            });
        }
    }
    let saved = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let saved = Restore(s.paused, s.listings_paused);
        s.paused = 0;
        s.listings_paused = 0;
        saved
    });
    let frame = start();
    let r = f();
    let obs = frame.finish();
    drop(saved);
    (r, obs)
}

/// Re-records observations from [`capture`] into the current frame, as if they
/// were being observed now: dropped while recording is paused, listings dropped
/// while listings are paused.
pub fn replay(obs: &Observations) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.paused > 0 {
            return;
        }
        let listings = s.listings_paused == 0;
        if let Some(top) = s.frames.last_mut() {
            top.missing.extend_from_slice(&obs.missing);
            if listings {
                top.listings.extend_from_slice(&obs.listings);
            }
            top.reads.extend_from_slice(&obs.reads);
            top.path_reads.extend_from_slice(&obs.path_reads);
            top.view_reads.extend_from_slice(&obs.view_reads);
            top.loader_reads.extend_from_slice(&obs.loader_reads);
            top.packages.extend_from_slice(&obs.packages);
            top.links.extend_from_slice(&obs.links);
            top.key.extend_from_slice(&obs.key);
            top.aliased |= obs.aliased;
        }
    });
}

/// Records `set` as a key observation of the current frame
/// ([`Observations::key`]): what it holds decides the result of the resolution
/// and is no input of it. The set is kept by reference: record the same one
/// (from [`capture`], memoized) for every resolution that depends on it.
/// Dropped while recording is paused; an empty set is not recorded.
pub fn key(set: &Arc<Observations>) {
    debug_assert!(set.key.is_empty(), "a key set has no key observations");
    if set.is_empty() {
        return;
    }
    with_frame(|o| o.key.push(Arc::clone(set)));
}

/// The pause state of this thread (all recording, listings), for work moved to
/// another thread ([`with_pauses`]).
pub fn pauses() -> (u32, u32) {
    STATE.with(|s| {
        let s = s.borrow();
        (s.paused, s.listings_paused)
    })
}

/// Runs `f` with this thread's pause state set to `p` (from [`pauses`] on the
/// thread the work came from), restoring it afterwards.
pub fn with_pauses<R>(p: (u32, u32), f: impl FnOnce() -> R) -> R {
    struct Restore(u32, u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.paused = self.0;
                s.listings_paused = self.1;
            });
        }
    }
    let _saved = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let saved = Restore(s.paused, s.listings_paused);
        s.paused = p.0;
        s.listings_paused = p.1;
        saved
    });
    f()
}

/// Runs `f` with all recording on this thread paused, for calls whose
/// observations the caller reports itself (for example from a resolver's own
/// dependency log).
pub fn pause<R>(f: impl FnOnce() -> R) -> R {
    let _g = Pause::new(false);
    f()
}

/// Runs `f` with listing recording paused, for glob expansions whose precise
/// `(directory, pattern)` membership the caller records itself.
pub fn pause_listings<R>(f: impl FnOnce() -> R) -> R {
    let _g = Pause::new(true);
    f()
}

struct Pause {
    listings_only: bool,
}

impl Pause {
    fn new(listings_only: bool) -> Self {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if listings_only {
                s.listings_paused += 1;
            } else {
                s.paused += 1;
            }
        });
        Pause { listings_only }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if self.listings_only {
                s.listings_paused -= 1;
            } else {
                s.paused -= 1;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_lifts_pauses_and_replay_applies_them() {
        let work = || {
            missing(Path::new("/m"));
            listing(Path::new("/d"), "**");
            read(Path::new("/r"));
            7
        };
        // Captured inside a listings pause: everything is captured, nothing leaks out.
        let outer = start();
        let (v, obs) = pause_listings(|| capture(work));
        assert_eq!(v, 7);
        assert_eq!(obs.listings, [(PathBuf::from("/d"), "**".to_string())]);
        assert_eq!(obs.missing, [PathBuf::from("/m")]);
        assert!(outer.finish().is_empty());
        // Replay applies the pauses in effect at the replay.
        let outer = start();
        pause_listings(|| replay(&obs));
        let got = outer.finish();
        assert!(got.listings.is_empty());
        assert_eq!(got.reads, [PathBuf::from("/r")]);
        let outer = start();
        replay(&obs);
        pause(|| replay(&obs));
        let got = outer.finish();
        assert_eq!(got, obs);
        // The pause state is restored after a capture.
        let outer = start();
        pause_listings(|| {
            let _ = capture(|| ());
            listing(Path::new("/x"), "*");
        });
        assert!(outer.finish().listings.is_empty());
    }

    #[test]
    fn frames_nest_and_pause() {
        assert!(!is_recording());
        missing(Path::new("/dropped"));
        let outer = start();
        assert!(is_recording());
        missing(Path::new("/a"));
        let inner = start();
        read(Path::new("/b"));
        let got = inner.finish();
        assert_eq!(got.reads, [PathBuf::from("/b")]);
        assert!(got.missing.is_empty());
        pause(|| missing(Path::new("/paused")));
        pause_listings(|| {
            listing(Path::new("/d"), "**");
            missing(Path::new("/c"));
        });
        listing(Path::new("/e"), "*.json");
        package("react");
        let got = outer.finish();
        assert_eq!(got.missing, [PathBuf::from("/a"), PathBuf::from("/c")]);
        assert_eq!(got.listings, [(PathBuf::from("/e"), "*.json".to_string())]);
        assert_eq!(got.packages, ["react"]);
        assert!(!is_recording());
    }

    #[test]
    fn dedup_keeps_first_occurrence() {
        let mut o = Observations {
            missing: vec!["/b".into(), "/a".into(), "/b".into()],
            listings: vec![("/d".into(), "*".into()), ("/d".into(), "*".into())],
            reads: vec!["/r".into()],
            path_reads: vec!["/p".into(), "/p".into()],
            view_reads: vec!["/v".into(), "/v".into()],
            loader_reads: vec!["/lv".into(), "/lv".into()],
            packages: vec!["x".into(), "x".into()],
            links: vec!["/l".into(), "/l".into()],
            key: Vec::new(),
            aliased: false,
        };
        let set = Arc::new(Observations {
            reads: vec!["/k".into()],
            ..Observations::default()
        });
        o.key = vec![Arc::clone(&set), Arc::clone(&set)];
        o.dedup();
        assert_eq!(o.missing, [PathBuf::from("/b"), PathBuf::from("/a")]);
        assert_eq!(o.listings.len(), 1);
        assert_eq!(o.view_reads, [PathBuf::from("/v")]);
        assert_eq!(o.loader_reads, [PathBuf::from("/lv")]);
        assert_eq!(o.packages, ["x"]);
        assert_eq!(o.links, [PathBuf::from("/l")]);
        assert_eq!(o.key.len(), 1);
    }

    #[test]
    fn key_observations_stay_apart() {
        let set = Arc::new(Observations {
            reads: vec!["/k".into()],
            missing: vec!["/km".into()],
            ..Observations::default()
        });
        let outer = start();
        read(Path::new("/input"));
        key(&set);
        pause(|| key(&set));
        key(&Arc::new(Observations::default()));
        missing(Path::new("/m"));
        let got = outer.finish();
        assert_eq!(got.reads, [PathBuf::from("/input")]);
        assert_eq!(got.missing, [PathBuf::from("/m")]);
        assert_eq!(got.key.len(), 1);
        assert!(Arc::ptr_eq(&got.key[0], &set), "held by reference");
        assert!(!got.is_empty());
        // A capture keeps them, and a replay records them again, still apart.
        let outer = start();
        let ((), obs) = capture(|| {
            read(Path::new("/c"));
            key(&set);
        });
        assert_eq!(obs.reads, [PathBuf::from("/c")]);
        replay(&obs);
        replay(&got);
        let mut all = outer.finish();
        assert_eq!(all.reads, [PathBuf::from("/c"), PathBuf::from("/input")]);
        assert_eq!(all.key.len(), 2);
        all.dedup();
        assert_eq!(all.key.len(), 1);
        // Nothing is recorded without a frame.
        key(&set);
        let f = start();
        assert!(f.finish().is_empty());
    }

    #[test]
    fn dropped_frame_discards() {
        let f = start();
        missing(Path::new("/x"));
        drop(f);
        assert!(!is_recording());
        let f = start();
        assert!(f.finish().is_empty());
    }
}
