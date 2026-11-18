//! File system access for adapters: lexical path normalization, bounded walks,
//! and every existence check, listing and read.
//!
//! All adapter file access goes through this module. Inside [`with_fs`] it uses
//! the given shared [`DirCache`] (the tracer's), so directories are listed
//! once, reads are cached, and with a recording cache the probes of config
//! discovery (`jest.config.*` next to and above a test, `conftest.py`
//! candidates, a missing `go.work`) and config reads are recorded into
//! the caller's [`turborepo_trace_core::record`] frame like resolver probes.
//! Outside [`with_fs`] the same helpers use `std::fs` directly. Adapters must
//! not call `std::fs` themselves; [`PathFsx`] provides drop-in replacements for
//! `Path::is_file`/`is_dir`/`exists`.
//!
//! With a confined cache ([`DirCache::confine`]: graph mode) a *host path*, one
//! outside the repository such as `/usr/bin/env`, `/tmp/out` or
//! `~/.kube/config` ([`HOME`]), is never looked up: it does not exist for an
//! adapter, and nothing is recorded for it. What a command or a config names
//! there depends on the machine, not on a file of the repository.
//!
//! # The access log
//!
//! A caller that keeps what a computation returned for others (the load of a
//! config, for every file the config is consulted for; the walk that finds a
//! file's config) needs to know everything the result is a function of.
//! [`with_log`] gives it: the [`Log`] of every access through this module while
//! the computation ran, by construction, since there is no other way to the
//! file system from an adapter.
//!
//! - every path that was asked about ([`is_file`], [`is_dir`], [`exists`],
//!   [`read`], [`realpath`]), **whatever the answer**: a path that is there
//!   leaves no record in the cache, and it may go, or come to be of another
//!   kind;
//! - every enumeration, as the directory and a pattern over what is below it:
//!   the entries of one directory ([`read_dir`]: `*`), everything below one
//!   ([`walk_files`], [`walk_dirs`]: `**`), or, for a walk that says which
//!   paths it wants ([`walk_files_matching`], [`expand_glob`]), exactly those.
//!   The record is the listing a graph store keeps for the pair
//!   (`turborepo_trace_graph::glob::Glob` over the paths relative to the
//!   directory, real directories only), and the narrowed walks compute their
//!   result from the entries that pattern matches and from nothing else, so the
//!   result cannot change while the listing stays the same;
//! - every symbolic link an entry is or a [`realpath`] crossed: where it leads
//!   is asked about ([`Log::asked`]) or recorded ([`Log::links`]); a narrowed
//!   walk enters a link to a directory where the runner's own glob library does
//!   ([`Links`]);
//! - what a component that an adapter hands the cache to asks of it
//!   ([`direct`]).
//!
//! What a file read contains is recorded by the cache
//! ([`turborepo_trace_core::record`]), as before. A computation whose result
//! depends on a part of what it accessed only for some of its consumers says so
//! ([`part`]): the part's accesses are kept apart, under a name, for the caller
//! to hand to those consumers alone.

use std::{
    cell::RefCell,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{DirCache, fs::EntryKind};
use turborepo_trace_graph::glob::Glob;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<DirCache>>> = const { RefCell::new(None) };
}

/// Runs `f` with adapter file access routed through `fs` on this thread (nested
/// calls restore the previous cache).
pub fn with_fs<R>(fs: &Arc<DirCache>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<DirCache>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            ACTIVE.with(|a| *a.borrow_mut() = prev);
        }
    }
    let prev = ACTIVE.with(|a| a.borrow_mut().replace(Arc::clone(fs)));
    let _restore = Restore(prev);
    f()
}

fn active() -> Option<Arc<DirCache>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// The active cache for a query about `p`; `Err` when `p` is a host path of a
/// confined cache: the answer is "missing", without touching the machine.
///
/// A path below a `node_modules` directory is an installed package wherever its
/// store lives: adapters reach a package store or a linked package outside the
/// repository through the links in the repository's own `node_modules`, by its
/// real path. The directories the checkout lives below are not such a store,
/// whatever they are called: the `node_modules` is looked for in the part of
/// `p` below where it leaves the way to the repository ([`DirCache::local`]).
fn active_for(p: &Path) -> Result<Option<Arc<DirCache>>, HostPath> {
    match active() {
        Some(fs) if fs.is_host_path(p) && !has_marker(fs.local(p)) => Err(HostPath),
        fs => Ok(fs),
    }
}

/// The directory installed packages live in.
const MARKER: &str = "node_modules";

fn has_marker(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str() == MARKER)
}

/// Where the part of `p` (absolute, normalized) starts that the place of the
/// checkout has no say in ([`DirCache::local_offset`]); `0` outside
/// [`with_fs`].
pub fn local_offset(p: &Path) -> usize {
    active().map_or(0, |fs| fs.local_offset(p))
}

/// The repository root: the base of the confined cache inside [`with_fs`]
/// ([`DirCache::base`]); `None` outside it or with a cache that is not
/// confined.
pub fn base() -> Option<PathBuf> {
    active().and_then(|fs| fs.base().map(Path::to_path_buf))
}

/// The part of `p` (absolute, normalized) that the place of the checkout has no
/// say in ([`DirCache::local`]): the path from the repository root on for a
/// file of the tree. `p` itself outside [`with_fs`].
pub fn local(p: &Path) -> &Path {
    let off = local_offset(p);
    if off == 0 {
        return p;
    }
    use turborepo_trace_core::os::OsStrExt;
    let rest = &p.as_os_str().as_bytes()[off..];
    Path::new(std::ffi::OsStr::from_bytes(
        rest.strip_prefix(b"/").unwrap_or(rest),
    ))
}

/// Whether `p` (absolute, normalized) is the directory of the checkout or one
/// above it, inside [`with_fs`] with a confined cache: what such a directory is
/// called is no part of the tree (`repo`, `checkout`, `path0`).
pub fn is_checkout_dir(p: &Path) -> bool {
    active().is_some_and(|fs| fs.confined() && fs.local_offset(p) == p.as_os_str().len())
}

/// Whether `p` (absolute, normalized) goes through a `node_modules` directory:
/// a file of an installed package. Inside [`with_fs`] with a confined cache,
/// the directories the checkout lives below do not count ([`DirCache::local`]):
/// a checkout below `/ci/node_modules/app` holds first-party files.
pub fn in_node_modules(p: &Path) -> bool {
    match active() {
        Some(fs) => has_marker(fs.local(p)),
        None => has_marker(p),
    }
}

/// See [`active_for`].
struct HostPath;

/// The active shared cache, or a fresh private one outside [`with_fs`]: for the
/// task layer (`turborepo_trace_tasks`), whose records are its own. An adapter
/// whose result is kept for others hands the cache on with [`direct`], which
/// logs what is asked of it.
pub fn cache() -> Arc<DirCache> {
    active().unwrap_or_else(|| Arc::new(DirCache::new()))
}

/// Runs `f` with the active shared cache (a fresh private one outside
/// [`with_fs`]), for a component that takes a [`DirCache`] and asks it itself:
/// the Go package scanner, the cargo project model. What it asks is logged like
/// an access through this module ([`with_log`]): a path that was looked up is a
/// path that was asked about, a directory it listed is a listing of that
/// directory's entries (of everything below it, for a walk), a file it read is
/// read ([`Log::read`]). The log does not depend on what the cache records:
/// such a component may pause recording for what it records itself.
pub fn direct<R>(f: impl FnOnce(&Arc<DirCache>) -> R) -> R {
    let fs = cache();
    if LOGS.with(|l| l.borrow().is_empty()) {
        return f(&fs);
    }
    let (r, answers) = turborepo_trace_core::fs::answers(|| f(&fs));
    answered(&answers);
    r
}

/// Logs what a cache was asked ([`direct`]). (A host path is never logged:
/// [`active_for`].)
fn answered(answers: &turborepo_trace_core::fs::Answers) {
    let inside = |p: &Path| active_for(p).is_ok();
    for p in answers.asked.iter().filter(|p| inside(p)) {
        asked(p);
    }
    for p in answers.read.iter().filter(|p| inside(p)) {
        note(|l| l.read.push(p.clone()));
    }
    for (dir, deep) in answers.listed.iter().filter(|(dir, _)| inside(dir)) {
        listed(dir, if *deep { "**" } else { "*" });
    }
}

thread_local! {
    /// Contents that stand for files on this thread ([`with_contents`]).
    static CONTENTS: RefCell<Vec<(PathBuf, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `contents` standing for what the files they name contain, on
/// this thread: another revision of a config file, read where the file is
/// (`affected --since` loads the base revision of a changed config this way).
/// Only [`read`] and [`read_to_string`] see them; the files must exist, and
/// listings and existence checks are those of the disk. Nested calls restore
/// the previous contents.
pub fn with_contents<R>(contents: Vec<(PathBuf, Vec<u8>)>, f: impl FnOnce() -> R) -> R {
    struct Restore(Vec<(PathBuf, Vec<u8>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = std::mem::take(&mut self.0);
            CONTENTS.with(|c| *c.borrow_mut() = prev);
        }
    }
    let contents = contents
        .into_iter()
        .map(|(p, b)| (normalize(&p), b))
        .collect();
    let prev = CONTENTS.with(|c| std::mem::replace(&mut *c.borrow_mut(), contents));
    let _restore = Restore(prev);
    f()
}

/// The contents that stand for `p` on this thread ([`with_contents`]), if any.
fn stand_in(p: &Path) -> Option<Vec<u8>> {
    CONTENTS.with(|c| {
        let c = c.borrow();
        if c.is_empty() {
            return None;
        }
        let p = normalize(p);
        c.iter().find(|(q, _)| *q == p).map(|(_, b)| b.clone())
    })
}

/// Reads a file.
pub fn read(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let p = p.as_ref();
    if let Some(b) = stand_in(p) {
        return Ok(b);
    }
    match active_for(p) {
        Ok(fs) => {
            // Also a read that fails: the file may appear.
            asked(p);
            match fs {
                Some(fs) => fs.read_cached(p).map(|b| b.to_vec()),
                None => std::fs::read(p),
            }
        }
        Err(HostPath) => Err(io::ErrorKind::NotFound.into()),
    }
}

/// Reads the `package.json` at `p` as module resolution gets it: its
/// [`turborepo_trace_core::manifest::view`], the manifest without the fields
/// that describe the package to people and to the package manager (dependency
/// lists, scripts). For a reader that takes a field of the view and nothing
/// else (an embedded runner config, the entry points): with a recording cache
/// the read is recorded as a read of the view
/// ([`turborepo_trace_core::DirCache::read_manifest`]), so a dependency entry
/// that changes in the file is no change for the reader. A manifest without a
/// view (no JSON object) is returned as the file it is.
pub fn read_manifest(p: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let p = p.as_ref();
    let view = |b: Vec<u8>| turborepo_trace_core::manifest::view(&b).unwrap_or(b);
    if let Some(b) = stand_in(p) {
        return Ok(view(b));
    }
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => fs.read_manifest(p).map(|b| b.to_vec()),
                None => std::fs::read(p).map(view),
            }
        }
        Err(HostPath) => Err(io::ErrorKind::NotFound.into()),
    }
}

/// Reads a UTF-8 file.
pub fn read_to_string(p: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(p)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// What was accessed through this module while a log was open ([`with_log`];
/// see the module docs). Paths are absolute and normalized, in the order of the
/// accesses, possibly repeated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    /// The paths that were asked about (is it a file, a directory, there at
    /// all; a read), whatever the answer was. The answer is a function of
    /// what is at the path: nothing, a file, a directory.
    pub asked: Vec<PathBuf>,
    /// `(directory, pattern)`: the paths below the directory that match the
    /// pattern were enumerated (the pattern in the syntax of
    /// `turborepo_trace_graph::glob::Glob`, over paths relative to the
    /// directory: `*` its own entries, `**` everything below it).
    pub listed: Vec<(PathBuf, String)>,
    /// The symbolic links that a [`realpath`] crossed: where each one leads
    /// decided what was accessed after it.
    pub links: Vec<PathBuf>,
    /// The files that a component which asks the cache itself read
    /// ([`direct`]). (A read through this module is recorded by the cache.)
    pub read: Vec<PathBuf>,
    /// The accesses of the parts of the computation ([`part`]), by name.
    pub parts: Vec<(String, Log)>,
}

impl Log {
    /// Whether nothing was accessed.
    pub fn is_empty(&self) -> bool {
        self.asked.is_empty()
            && self.listed.is_empty()
            && self.links.is_empty()
            && self.read.is_empty()
            && self.parts.iter().all(|(_, p)| p.is_empty())
    }

    /// Appends the accesses of `other` (its parts to the parts of the same
    /// names).
    pub fn extend(&mut self, other: Log) {
        self.asked.extend(other.asked);
        self.listed.extend(other.listed);
        self.links.extend(other.links);
        self.read.extend(other.read);
        for (name, p) in other.parts {
            self.part_mut(&name).extend(p);
        }
    }

    fn part_mut(&mut self, name: &str) -> &mut Log {
        let i = match self.parts.iter().position(|(n, _)| n == name) {
            Some(i) => i,
            None => {
                self.parts.push((name.to_owned(), Log::default()));
                self.parts.len() - 1
            }
        };
        &mut self.parts[i].1
    }
}

thread_local! {
    /// The open logs of this thread, innermost last.
    static LOGS: RefCell<Vec<Log>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with a new innermost log and returns what it logged.
fn logged<R>(f: impl FnOnce() -> R) -> (R, Log) {
    // An unwinding `f` leaves no log behind.
    struct Close(usize);
    impl Drop for Close {
        fn drop(&mut self) {
            LOGS.with(|l| l.borrow_mut().truncate(self.0));
        }
    }
    let depth = LOGS.with(|l| {
        let mut l = l.borrow_mut();
        l.push(Log::default());
        l.len()
    });
    let close = Close(depth - 1);
    let r = f();
    let log = LOGS.with(|l| {
        let mut l = l.borrow_mut();
        l.truncate(depth);
        l.pop().unwrap_or_default()
    });
    std::mem::forget(close);
    (r, log)
}

/// Runs `f` and returns, with its result, the [`Log`] of every access it made
/// through this module (on this thread). An enclosing log gets the same
/// accesses: what `f` computed is a part of what its caller computes.
pub fn with_log<R>(f: impl FnOnce() -> R) -> (R, Log) {
    let (r, log) = logged(f);
    note(|outer| outer.extend(log.clone()));
    (r, log)
}

/// Runs `f` as the part `name` of the computation that is being logged: its
/// accesses go to [`Log::parts`] under that name instead of the log itself. For
/// accesses that decide the result only for some of its consumers, which the
/// caller of [`with_log`] knows by the name. Two parts of one name are one
/// part. Without an open log, `f` just runs.
pub fn part<R>(name: &str, f: impl FnOnce() -> R) -> R {
    if LOGS.with(|l| l.borrow().is_empty()) {
        return f();
    }
    let (r, log) = logged(f);
    note(|outer| outer.part_mut(name).extend(log));
    r
}

/// Adds to the innermost open log, if any.
fn note(f: impl FnOnce(&mut Log)) {
    LOGS.with(|l| {
        if let Some(top) = l.borrow_mut().last_mut() {
            f(top);
        }
    });
}

/// Logs that `p` was asked about.
fn asked(p: &Path) {
    note(|l| l.asked.push(normalize(p)));
}

/// Logs that the paths below `dir` that match `pattern` were enumerated.
fn listed(dir: &Path, pattern: &str) {
    note(|l| l.listed.push((normalize(dir), pattern.to_owned())));
}

/// Logs the symbolic links that a real path was found through.
fn crossed(links: &[(PathBuf, PathBuf)]) {
    note(|l| l.links.extend(links.iter().map(|(link, _)| link.clone())));
}

/// Logs that the entries of `dir`, a directory behind a symbolic link, were
/// enumerated: as the entries of the real directory, with the links on the way
/// there as links that were crossed. (A listing is kept for a directory of the
/// tree; the path through a link is none, and what a store knows of it does not
/// move when the real directory gets an entry.)
fn listed_behind(dir: &Path) {
    match active() {
        Some(fs) => {
            let real = fs.canonicalize(dir);
            crossed(&real.links);
            listed(&real.path, "*");
        }
        None => listed(dir, "*"),
    }
}

/// Whether `p` is a file (following symlinks).
pub fn is_file(p: &Path) -> bool {
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => fs.is_file(p),
                None => p.is_file(),
            }
        }
        Err(HostPath) => false,
    }
}

/// Whether `p` is a directory (following symlinks).
pub fn is_dir(p: &Path) -> bool {
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => fs.is_dir(p),
                None => p.is_dir(),
            }
        }
        Err(HostPath) => false,
    }
}

/// Whether `p` exists.
pub fn exists(p: &Path) -> bool {
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => fs.exists(p),
                None => p.exists(),
            }
        }
        Err(HostPath) => false,
    }
}

/// The real path of `p` (every symlinked component resolved, as
/// `fs.realpathSync` does), or `None` if `p` does not exist. With a recording
/// cache the existence check is recorded, so creating the path later is seen;
/// the symbolic links crossed are logged ([`Log::links`]).
///
/// The real path is a function of what every name on the way is: a link (where
/// it leads is the link's record) or no link. A name that is no link may come
/// to be one, with what the path asks about still there behind it (a directory
/// that is replaced by a link to a directory): so every directory on the way is
/// asked about too, and the kind of a path that was asked about says whether it
/// is a link.
pub fn realpath(p: &Path) -> Option<PathBuf> {
    if !exists(p) {
        return None;
    }
    match active() {
        Some(fs) => {
            let real = fs.canonicalize(p);
            crossed(&real.links);
            on_the_way(p);
            Some(real.path)
        }
        None => std::fs::canonicalize(p).ok(),
    }
}

/// Whether `p` is a symbolic link that leads to nothing (an entry of its
/// directory with no file or directory at its end). Asked about like
/// [`exists`]; `false` outside the cache for a host path.
pub fn is_link_to_nothing(p: &Path) -> bool {
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => matches!(fs.stat_or_dangling(p), Err(true)),
                None => p.is_symlink() && !p.exists(),
            }
        }
        Err(HostPath) => false,
    }
}

/// Logs that every directory above `p` was asked about, below the repository's
/// root (a host path is never looked up, [`active_for`], and what the root
/// itself is belongs to the machine).
fn on_the_way(p: &Path) {
    let p = normalize(p);
    for dir in p.ancestors().skip(1) {
        let inside = |d: &Path| active_for(d).is_ok();
        let Some(above) = dir.parent() else { break };
        if !inside(above) || !inside(dir) {
            break;
        }
        asked(dir);
    }
}

/// One entry of a directory.
struct Entry {
    path: PathBuf,
    /// A directory, or a symbolic link to one.
    dir: bool,
    /// A symbolic link.
    link: bool,
}

/// The entries of `dir` sorted by path, without logging; `None` if it cannot be
/// listed.
fn entries(dir: &Path) -> Option<Vec<Entry>> {
    let mut out: Vec<Entry> = match active_for(dir).ok()? {
        Some(fs) => {
            let listing = fs.list(dir)?;
            listing
                .iter()
                .map(|e| {
                    let path = dir.join(e.name());
                    let link = matches!(e.kind(), EntryKind::Symlink);
                    let dir = match e.kind() {
                        EntryKind::Dir => true,
                        // (Without a record of the cache's own: a link that leads nowhere
                        // would leave a path that was not found in the records of a walk
                        // that never asked for it, and only while it leads nowhere. A walk
                        // asks about the links it depends on.)
                        EntryKind::Symlink => {
                            turborepo_trace_core::record::pause(|| fs.is_dir(&path))
                        }
                        _ => false,
                    };
                    Entry { path, dir, link }
                })
                .collect()
        }
        None => std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| {
                let t = e.file_type().ok();
                Entry {
                    path: e.path(),
                    dir: t.is_some_and(|t| t.is_dir()),
                    link: t.is_some_and(|t| t.is_symlink()),
                }
            })
            .collect(),
    };
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Some(out)
}

/// The entries of directory `dir` as `(path, is_dir)`, sorted by path; `None`
/// if it cannot be listed.
pub fn read_dir(dir: &Path) -> Option<Vec<(PathBuf, bool)>> {
    if active_for(dir).is_err() {
        return None;
    }
    listed(dir, "*");
    let es = entries(dir)?;
    // What a link leads to is no part of the listing.
    for e in es.iter().filter(|e| e.link) {
        asked(&e.path);
    }
    Some(es.into_iter().map(|e| (e.path, e.dir)).collect())
}

/// `is_file`, `is_dir` and `exists` through this module, as methods.
pub trait PathFsx {
    /// [`is_file`].
    fn fx_is_file(&self) -> bool;
    /// [`is_dir`].
    fn fx_is_dir(&self) -> bool;
    /// [`exists`].
    fn fx_exists(&self) -> bool;
}

impl PathFsx for Path {
    fn fx_is_file(&self) -> bool {
        is_file(self)
    }
    fn fx_is_dir(&self) -> bool {
        is_dir(self)
    }
    fn fx_exists(&self) -> bool {
        exists(self)
    }
}

/// Maximum walk depth.
const MAX_DEPTH: usize = 64;

/// Directory names never walked.
pub fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | ".git" | ".hg" | ".svn" | ".turbotrace" | ".turbo" | ".next"
    )
}

/// Lexically normalizes `p` (`.` and `..` segments; `..` at the root is
/// dropped).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Stands for the user's home directory in paths ([`join`]): `~/.kube/config`
/// is `/~/.kube/config`. The real directory depends on the machine and is never
/// looked up.
pub const HOME: &str = "/~";

/// `base.join(rel)`, normalized (an absolute `rel` replaces `base`). A `rel` in
/// the home directory (`~` or `~/x`, as shells and CI configuration write it)
/// is not below `base`: it is the path below [`HOME`], outside every
/// repository.
pub fn join(base: &Path, rel: &str) -> PathBuf {
    match rel.strip_prefix('~') {
        Some("") => PathBuf::from(HOME),
        Some(rest) if rest.starts_with('/') => normalize(&Path::new(HOME).join(&rest[1..])),
        _ => normalize(&base.join(rel)),
    }
}

/// [`join`] for a path that text of the repository writes (the argument of a
/// command): a relative `rel` that climbs above the repository root and comes
/// back down names, inside the repository, what follows the directories it goes
/// through ([`turborepo_trace_core::fs::seen_from_elsewhere`]:
/// `../repo/data/x.json` from the root is `data/x.json` when the checkout's
/// directory is called `repo`). Where it lands on this machine depends on what
/// the checkout's directories are called, so it is not looked up there: the one
/// path it can name inside the repository is the result, at every place.
/// Every other path, and every path outside [`with_fs`] with a confined cache,
/// is [`join`]'s.
pub fn join_written(base: &Path, rel: &str) -> PathBuf {
    if !rel.starts_with(['/', '~'])
        && let Some(fs) = active()
        && let Some(root) = fs.base()
        && let Ok(below) = base.strip_prefix(root)
        && let [reading] =
            turborepo_trace_core::fs::seen_from_elsewhere(Some(below.components().count()), rel)
                .as_slice()
    {
        return root.join(&reading.rel);
    }
    join(base, rel)
}

/// What an absolute path that text of the repository writes out (`spelled`,
/// normalized) may name inside the repository: the repository seen from another
/// place ([`turborepo_trace_core::fs::seen_from_elsewhere`]), as paths inside
/// the root with the reading in one sentence. Empty outside [`with_fs`] with a
/// confined cache.
pub fn readings(spelled: &Path) -> Vec<(PathBuf, String)> {
    let (Some(fs), Some(text)) = (active(), spelled.to_str()) else {
        return Vec::new();
    };
    let Some(root) = fs.base() else {
        return Vec::new();
    };
    turborepo_trace_core::fs::seen_from_elsewhere(None, text)
        .into_iter()
        .map(|e| (root.join(&e.rel), e.sentence(text, &e.rel)))
        .collect()
}

fn name_of(p: &Path) -> std::borrow::Cow<'_, str> {
    p.file_name().unwrap_or_default().to_string_lossy()
}

/// Calls `f` for every file under `root` (sorted per directory), skipping
/// [`skip_dir`]s.
///
/// Logged as everything below `root` (`**`): a caller that wants only some of
/// the files says which with [`walk_files_matching`]. A symbolic link to a
/// directory is followed, as before; a listing does not go through one, so the
/// link is asked about and every directory behind it is logged by its own
/// entries ([`listed_behind`]).
pub fn walk_files(root: &Path, f: &mut dyn FnMut(&Path)) {
    if active_for(root).is_err() {
        return;
    }
    listed(root, "**");
    walk(root, 0, false, f);
}

fn walk(dir: &Path, depth: usize, linked: bool, f: &mut dyn FnMut(&Path)) {
    if depth > MAX_DEPTH {
        return;
    }
    if linked {
        listed_behind(dir);
    }
    let Some(entries) = entries(dir) else {
        return;
    };
    for e in entries {
        if e.link {
            asked(&e.path);
        }
        if e.dir {
            if !skip_dir(&name_of(&e.path)) {
                walk(&e.path, depth + 1, linked || e.link, f);
            }
        } else {
            f(&e.path);
        }
    }
}

/// Calls `f` for every directory under `root` (including `root`), skipping
/// [`skip_dir`]s. Logged like [`walk_files`].
pub fn walk_dirs(root: &Path, f: &mut dyn FnMut(&Path)) {
    if !is_dir(root) {
        return;
    }
    listed(root, "**");
    walk_dirs_inner(root, 0, false, f);
}

fn walk_dirs_inner(dir: &Path, depth: usize, linked: bool, f: &mut dyn FnMut(&Path)) {
    if depth > MAX_DEPTH {
        return;
    }
    f(dir);
    if linked {
        listed_behind(dir);
    }
    let Some(entries) = entries(dir) else {
        return;
    };
    for e in entries {
        if e.link {
            asked(&e.path);
        }
        if e.dir && !skip_dir(&name_of(&e.path)) {
            walk_dirs_inner(&e.path, depth + 1, linked || e.link, f);
        }
    }
}

/// How the glob library of a runner treats a symbolic link to a directory on
/// the way to a match ([`expand_glob`]). Measured with the runners (jest 29.7
/// and 30.5, vitest 2.1 and 3.2, Python 3.14).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Links {
    /// Never entered: jest's file map lists no file behind a link.
    Skip,
    /// node-glob, which expands jest's `projects`: a link is entered where a
    /// segment of the pattern names it (a literal, a `*`), and a `**` ends
    /// at a link: it matches the link and what the following segments name
    /// below it, and does not go on below it.
    NodeGlob,
    /// Entered like a directory at every depth: tinyglobby and fast-glob
    /// (vitest's `projects`), Python's `glob` (pytest's `testpaths`).
    Follow,
}

/// The links one path of a walk may go through, one behind the other (a link
/// that leads back above itself would otherwise be walked until [`MAX_DEPTH`]).
const MAX_LINKS: usize = 4;

/// Where a walk is ([`matching`]).
#[derive(Copy, Clone)]
struct At<'a> {
    dir: &'a Path,
    /// The path of `dir` relative to the walk's root.
    rel: &'a str,
    depth: usize,
    /// The links the walk went through to get here.
    links: usize,
    /// Which segments of the pattern can come next ([`Wanted::step`]).
    states: u64,
}

/// Calls `f` for every entry below `at.dir` whose path relative to the walk's
/// root matches the glob of `want`, sorted per directory. A real directory is
/// entered (no [`skip_dir`]), like the listing a graph store keeps for a
/// directory and a pattern: what the walk passes to `f` there is a function of
/// that listing and of what each matching link leads to (asked about here). A
/// symbolic link to a directory is entered as the runner's glob library does
/// ([`Links`]); a listing does not go through one, so the link is asked about
/// and every directory behind it is logged by its own entries.
fn matching(at: At<'_>, want: &Wanted<'_>, f: &mut dyn FnMut(&Entry)) {
    let (levels, glob) = (want.levels, &want.glob);
    if at.depth > MAX_DEPTH {
        return;
    }
    if at.links > 0 {
        listed_behind(at.dir);
    }
    let Some(entries) = entries(at.dir) else {
        return;
    };
    for e in entries {
        let name = name_of(&e.path);
        if e.dir && skip_dir(&name) {
            continue;
        }
        let rel = if at.rel.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", at.rel)
        };
        // Which segments can come next after this name (node-glob: a `**` does not go
        // on behind a link to a directory).
        let node_glob = want.links == Links::NodeGlob;
        let states = if node_glob {
            want.step(at.states, &name, e.link)
        } else {
            at.states
        };
        // Behind a link, node-glob matches only what no `**` had to go through a link
        // for.
        if levels.is_none_or(|n| at.depth + 1 == n)
            && want.literal.is_none_or(|l| rel.contains(l))
            && glob.matches(&rel)
            && (at.links == 0 || !node_glob || want.accepts(states))
        {
            if e.link {
                asked(&e.path);
            }
            f(&e);
        }
        // A place where a directory is entered.
        if !levels.is_none_or(|n| at.depth + 1 < n) {
            continue;
        }
        let below = At {
            dir: &e.path,
            rel: &rel,
            depth: at.depth + 1,
            links: at.links + usize::from(e.link),
            states,
        };
        if !e.link {
            if e.dir {
                matching(below, want, f);
            }
            continue;
        }
        // A link: entered where the runner's library enters a link to a directory. What
        // it leads to is asked about whether or not that is a directory now: a link to
        // a file, or to nothing, may come to lead to one.
        let enters = match want.links {
            Links::Skip => false,
            Links::Follow => true,
            Links::NodeGlob => states != 0,
        };
        if enters && at.links < MAX_LINKS {
            asked(below.dir);
            if e.dir {
                matching(below, want, f);
            }
        }
    }
}

/// One segment of the pattern of a narrowed walk.
enum Segment {
    /// `**`: any number of directories.
    Deep,
    /// One name.
    Name(Glob),
}

/// The pattern of a narrowed walk ([`matching`]), with what can be said of
/// every match before the glob runs.
struct Wanted<'a> {
    glob: Glob,
    /// The number of segments of every match, when the pattern says it (no
    /// `**`): the walk goes no deeper.
    levels: Option<usize>,
    /// A segment of the pattern without wildcards: every match has it as a
    /// component.
    literal: Option<&'a str>,
    /// How links to directories are treated.
    links: Links,
    /// The pattern by segments, for the rule of [`Links::NodeGlob`] (at most
    /// 63).
    segments: Vec<Segment>,
}

impl<'a> Wanted<'a> {
    fn new(pattern: &'a str, links: Links) -> Option<Self> {
        let segments: Vec<Segment> = pattern
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| match s {
                "**" => Some(Segment::Deep),
                s => Glob::new(s).ok().map(Segment::Name),
            })
            .collect::<Option<_>>()?;
        Some(Wanted {
            glob: Glob::new(pattern).ok()?,
            levels: (!pattern.contains("**")).then(|| pattern.split('/').count()),
            literal: pattern
                .split('/')
                .filter(|s| !s.is_empty() && !s.contains(['*', '?', '[', ']', '{', '}', ',']))
                .max_by_key(|s| s.len()),
            // (A pattern of more segments than the states have bits follows links.)
            links: if segments.len() > 62 && links == Links::NodeGlob {
                Links::Follow
            } else {
                links
            },
            segments,
        })
    }

    /// The walk's root: the first segment comes next.
    fn at<'d>(&self, root: &'d Path) -> At<'d> {
        At {
            dir: root,
            rel: "",
            depth: 0,
            links: 0,
            states: 1,
        }
    }

    /// The states with a `**` that matches no directory passed over.
    fn closed(&self, mut states: u64) -> u64 {
        for (i, s) in self.segments.iter().enumerate() {
            if states & (1 << i) != 0 && matches!(s, Segment::Deep) {
                states |= 1 << (i + 1);
            }
        }
        states
    }

    /// The segments that can come next after the name `name` (a state is the
    /// index of a segment; a `**` that goes on keeps its state). `link`:
    /// the name is a link to a directory, which a `**` matches as its last
    /// directory and does not go through.
    fn step(&self, states: u64, name: &str, link: bool) -> u64 {
        let states = self.closed(states);
        let mut next = 0;
        for (i, s) in self.segments.iter().enumerate() {
            if states & (1 << i) == 0 {
                continue;
            }
            match s {
                Segment::Deep => {
                    next |= 1 << (i + 1);
                    if !link {
                        next |= 1 << i;
                    }
                }
                Segment::Name(g) if g.matches(name) => next |= 1 << (i + 1),
                Segment::Name(_) => {}
            }
        }
        next
    }

    /// Whether a path that led to `states` is a match: every segment was
    /// passed.
    fn accepts(&self, states: u64) -> bool {
        self.closed(states) & (1 << self.segments.len()) != 0
    }
}

/// Calls `f` for every file below `root` whose path relative to `root` matches
/// `pattern` (a `turborepo_trace_graph::glob::Glob`), sorted per directory,
/// skipping [`skip_dir`]s. Logged as exactly that listing ([`Log::listed`]): a
/// file that appears elsewhere below `root` is no change for the caller.
/// Symbolic links to directories are not followed (jest's file map does not
/// follow them either: [`Links::Skip`]).
pub fn walk_files_matching(root: &Path, pattern: &str, f: &mut dyn FnMut(&Path)) {
    if active_for(root).is_err() {
        return;
    }
    let Some(want) = Wanted::new(pattern, Links::Skip) else {
        // No pattern of a listing: every file, logged as such.
        return walk_files(root, f);
    };
    listed(root, pattern);
    matching(want.at(root), &want, &mut |e| {
        if !e.dir {
            f(&e.path);
        }
    });
}

/// The pattern of a listing that matches every path the runner glob `rest`
/// (relative to its literal directory prefix) can match: what [`Glob`] also
/// knows is kept (`*`, `?`, `**` segments, braces within a segment), a segment
/// with a character class is any name, and from the first segment with syntax a
/// listing does not have (an extglob, an alternation, a negation, an escape, a
/// brace range, a group across segments) on, every path.
fn widen(rest: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut braces = 0i32;
    // (A pattern that ends in `/` names the same entries.)
    for seg in rest.split('/').filter(|s| !s.is_empty()) {
        // A brace group that goes on in the next segment.
        let open = braces + seg.matches('{').count() as i32 - seg.matches('}').count() as i32;
        if braces != 0
            || open != 0
            || seg.contains(['(', ')', '|', '\\', '!'])
            || seg.contains("..")
        {
            out.push("**");
            return out.join("/");
        }
        braces = open;
        out.push(if seg == "**" {
            "**"
        } else if seg.contains("**") || seg.contains(['[', ']']) {
            "*"
        } else {
            seg
        });
    }
    out.join("/")
}

/// Paths under `base` matching the glob `pattern` (absolute, after `<rootDir>`
/// substitution): directories (`dirs`) or files. Walks from the longest literal
/// directory prefix and is logged as the listing of that directory and the
/// pattern as far as a listing can say it ([`widen`]): only an entry of that
/// listing, or of a directory behind a link that the runner's glob library
/// enters (`links`, logged by its own entries), is matched against the pattern.
pub fn expand_glob(pattern: &str, dirs: bool, links: Links) -> Vec<PathBuf> {
    let first_meta = pattern
        .find(['*', '?', '[', '{', '('])
        .unwrap_or(pattern.len());
    let (base, rest) = match pattern[..first_meta].rfind('/') {
        Some(i) => (&pattern[..i.max(1)], &pattern[i + 1..]),
        None => (".", pattern),
    };
    let Ok(re) = regex::Regex::new(&crate::pattern::glob_to_regex(pattern).0) else {
        return Vec::new();
    };
    let base = Path::new(base);
    if active_for(base).is_err() {
        return Vec::new();
    }
    let mut wide = widen(rest);
    if Glob::new(&wide).is_err() {
        wide = "**".to_owned();
    }
    let Some(want) = Wanted::new(&wide, links) else {
        return Vec::new();
    };
    listed(base, &wide);
    let mut out = Vec::new();
    // The base itself (`a/**` names `a`).
    if dirs && re.is_match(&base.to_string_lossy()) && is_dir(base) {
        out.push(base.to_path_buf());
    }
    matching(want.at(base), &want, &mut |e| {
        if e.dir == dirs && re.is_match(&e.path.to_string_lossy()) {
            out.push(e.path.clone());
        }
    });
    out
}

/// Whether `p` itself is a symbolic link, whatever it leads to (the kind of a
/// path that was asked about says so).
pub fn is_link(p: &Path) -> bool {
    match active_for(p) {
        Ok(fs) => {
            asked(p);
            match fs {
                Some(fs) => fs.stat(p).is_some_and(|s| s.is_symlink),
                None => p.is_symlink(),
            }
        }
        Err(HostPath) => false,
    }
}

/// The first existing file among `dir/name` for `names`.
pub fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| is_file(p))
}

/// The 1-based line of the first occurrence of `word` (as a property name) in
/// `src`, or 0.
pub fn line_of_field(src: &[u8], word: &str) -> u32 {
    let w = word.as_bytes();
    if w.is_empty() {
        return 0;
    }
    let mut i = 0;
    while let Some(off) = src
        .get(i..)
        .and_then(|s| s.windows(w.len()).position(|x| x == w))
    {
        let at = i + off;
        let before = at.checked_sub(1).map(|j| src[j]);
        let after = src.get(at + w.len()).copied();
        let ident =
            |b: Option<u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$');
        if !ident(before) && !ident(after) {
            return src[..at].iter().filter(|b| **b == b'\n').count() as u32 + 1;
        }
        i = at + w.len();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_paths() {
        assert_eq!(
            normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(join(Path::new("/a/b"), "../x"), PathBuf::from("/a/x"));
        assert_eq!(join(Path::new("/a/b"), "/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn the_home_directory_is_not_below_the_base() {
        let base = Path::new("/repo/pkg");
        assert_eq!(
            join(base, "~/.kube/config"),
            PathBuf::from("/~/.kube/config")
        );
        assert_eq!(join(base, "~"), PathBuf::from("/~"));
        assert_eq!(join(base, "~/"), PathBuf::from("/~"));
        // Only the shell form: other names that start with `~` are names.
        assert_eq!(join(base, "~lib/x"), PathBuf::from("/repo/pkg/~lib/x"));
        assert_eq!(join(base, "a/~/x"), PathBuf::from("/repo/pkg/a/~/x"));
    }

    #[test]
    fn a_confined_cache_never_looks_at_host_paths() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path().canonicalize().unwrap();
        let (repo, host) = (t.join("repo"), t.join("host"));
        std::fs::create_dir_all(repo.join("d")).unwrap();
        std::fs::create_dir_all(host.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(t.join("store")).unwrap();
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        std::fs::write(host.join("h.txt"), "x").unwrap();
        std::fs::write(host.join("node_modules/pkg/package.json"), "{}").unwrap();
        std::fs::write(t.join("store/s.txt"), "x").unwrap();
        let fs = Arc::new(DirCache::with_recording());
        fs.confine(&repo, [t.join("store")]);
        let frame = turborepo_trace_core::record::start();
        with_fs(&fs, || {
            // Inside the repository: as before.
            assert!(repo.join("a.txt").fx_is_file());
            assert!(repo.join("d").fx_is_dir());
            assert!(!repo.join("missing").fx_exists());
            // Host paths do not exist, whatever is there.
            assert!(!host.join("h.txt").fx_exists());
            assert!(!host.join("h.txt").fx_is_file());
            assert!(!host.fx_is_dir());
            assert!(read(host.join("h.txt")).is_err());
            assert!(read_dir(&host).is_none());
            assert!(realpath(&host.join("h.txt")).is_none());
            assert!(!Path::new(HOME).join(".npmrc").fx_exists());
            // So are the directories the checkout lives below.
            assert!(!t.fx_is_dir());
            assert!(read_dir(&t).is_none());
            // A declared root (a language's module cache) and an installed package belong
            // to the project.
            assert!(t.join("store/s.txt").fx_is_file());
            assert!(host.join("node_modules/pkg/package.json").fx_is_file());
        });
        let obs = frame.finish();
        // Nothing is recorded for a host path.
        assert!(obs.missing.contains(&repo.join("missing")), "{obs:?}");
        assert!(
            !obs.missing
                .iter()
                .chain(&obs.reads)
                .any(|p| p.starts_with(&host) && !p.starts_with(host.join("node_modules"))),
            "{obs:?}"
        );
        assert!(!obs.listings.iter().any(|(d, _)| d.starts_with(&host)));
        // Without the confinement the same queries see the machine.
        let open = Arc::new(DirCache::new());
        with_fs(&open, || assert!(host.join("h.txt").fx_is_file()));
    }

    #[test]
    fn routes_through_the_shared_cache_and_records_probes() {
        let t = std::env::temp_dir().join(format!("fsx-{}", std::process::id()));
        std::fs::create_dir_all(t.join("d")).unwrap();
        std::fs::write(t.join("a.txt"), "x").unwrap();
        let fs = Arc::new(DirCache::with_recording());
        let frame = turborepo_trace_core::record::start();
        let (a, missing, listed) = with_fs(&fs, || {
            (
                read_to_string(t.join("a.txt")).unwrap(),
                t.join("jest.config.js").fx_is_file(),
                read_dir(&t).unwrap(),
            )
        });
        let obs = frame.finish();
        assert_eq!(a, "x");
        assert!(!missing);
        assert_eq!(listed, [(t.join("a.txt"), false), (t.join("d"), true)]);
        assert!(obs.missing.contains(&t.join("jest.config.js")), "{obs:?}");
        assert!(obs.reads.contains(&t.join("a.txt")), "{obs:?}");
        // Outside `with_fs`, std::fs is used and nothing is recorded.
        assert!(active().is_none());
        assert!(t.join("a.txt").fx_is_file());
        std::fs::remove_dir_all(&t).unwrap();
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn the_log_has_every_question_whatever_the_answer() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path().canonicalize().unwrap();
        touch(&t, "a.txt");
        touch(&t, "d/b.txt");
        let (_, log) = with_log(|| {
            assert!(t.join("a.txt").fx_is_file());
            assert!(t.join("d").fx_is_dir());
            assert!(!t.join("a.txt").fx_is_dir());
            assert!(!t.join("nope").fx_exists());
            assert!(read(t.join("d/b.txt")).is_ok());
            assert!(read(t.join("d/nope.txt")).is_err());
            assert!(read_manifest(t.join("package.json")).is_err());
            // Its own part.
            part("p", || assert!(t.join("p.txt").fx_exists() || true));
        });
        let asked: Vec<&Path> = log
            .asked
            .iter()
            .map(|p| p.strip_prefix(&t).unwrap())
            .collect();
        assert_eq!(
            asked,
            [
                "a.txt",
                "d",
                "a.txt",
                "nope",
                "d/b.txt",
                "d/nope.txt",
                "package.json"
            ]
            .map(Path::new)
        );
        assert_eq!(log.parts.len(), 1);
        assert_eq!(log.parts[0].1.asked, [t.join("p.txt")]);
        // Nothing is logged outside `with_log`, and a nested log is the outer one's
        // too.
        assert!(t.join("a.txt").fx_is_file());
        let (_, outer) = with_log(|| {
            let (_, inner) = with_log(|| t.join("a.txt").fx_is_file());
            assert_eq!(inner.asked.len(), 1);
            t.join("d").fx_is_dir()
        });
        assert_eq!(outer.asked, [t.join("a.txt"), t.join("d")]);
    }

    #[test]
    fn walks_are_logged_as_the_listing_they_are() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path().canonicalize().unwrap();
        for f in [
            "packages/a/jest.config.js",
            "packages/a/src/__mocks__/m.js",
            "packages/a/src/x.js",
            "packages/b/deep/jest.config.js",
            "packages/node_modules/n/jest.config.js",
            "elsewhere/__mocks__/e.js",
        ] {
            touch(&t, f);
        }
        turborepo_trace_core::os::symlink(t.join("elsewhere"), t.join("packages/link")).unwrap();
        // (Through a cache, as in a graph build: it says what a link leads to.)
        let fs = Arc::new(DirCache::new());
        with_fs(&fs, || walks(&t));
    }

    fn walks(t: &Path) {
        let rel = |ps: Vec<PathBuf>| -> Vec<String> {
            ps.iter()
                .map(|p| p.strip_prefix(t).unwrap().to_string_lossy().into_owned())
                .collect()
        };
        let glob = |pattern: &str, dirs: bool| {
            // (A walk that enters no link; the libraries that do: `tests/projects.rs`.)
            let (found, log) =
                with_log(|| expand_glob(&format!("{}/{pattern}", t.display()), dirs, Links::Skip));
            assert_eq!(log.listed.len(), 1, "{:?}", log.listed);
            let (dir, listed) = log.listed[0].clone();
            let asked = rel(log.asked);
            (rel(found), rel(vec![dir]).remove(0), listed, asked)
        };
        // Directories one level down: a link to a directory is one, and is asked about.
        let (found, dir, listed, asked) = glob("packages/*", true);
        assert_eq!(found, ["packages/a", "packages/b", "packages/link"]);
        assert_eq!((dir.as_str(), listed.as_str()), ("packages", "*"));
        assert_eq!(asked, ["packages/link"]);
        // Files two levels down: nothing deeper, nothing behind the link.
        let (found, dir, listed, _) = glob("packages/*/jest.config.js", false);
        assert_eq!(found, ["packages/a/jest.config.js"]);
        assert_eq!(
            (dir.as_str(), listed.as_str()),
            ("packages", "*/jest.config.js")
        );
        // Braces are a listing's syntax; an extglob is not, and everything is listed.
        let (found, _, listed, _) = glob("{packages,elsewhere}/*", true);
        assert_eq!(
            found,
            [
                "elsewhere/__mocks__",
                "packages/a",
                "packages/b",
                "packages/link"
            ]
        );
        assert_eq!(listed, "{packages,elsewhere}/*");
        let (found, _, listed, _) = glob("packages/@(a|b)", true);
        assert_eq!(found, ["packages/a", "packages/b"]);
        assert_eq!(listed, "**");
        // Any depth.
        let (found, _, listed, _) = glob("packages/**/jest.config.js", false);
        assert_eq!(
            found,
            [
                "packages/a/jest.config.js",
                "packages/b/deep/jest.config.js"
            ]
        );
        assert_eq!(listed, "**/jest.config.js");

        // A walk that says what it wants.
        let mut found = Vec::new();
        let (_, log) = with_log(|| {
            walk_files_matching(&t.join("packages"), "**/__mocks__/**", &mut |p| {
                found.push(p.to_path_buf())
            })
        });
        assert_eq!(rel(found), ["packages/a/src/__mocks__/m.js"]);
        assert_eq!(
            log.listed,
            [(t.join("packages"), "**/__mocks__/**".to_owned())]
        );
        // A walk of everything follows the link, and logs what is behind it by
        // directory: the real directories (a listing is kept for a directory of
        // the tree), and the link as one that was crossed.
        let mut found = Vec::new();
        let (_, log) =
            with_log(|| walk_files(&t.join("packages"), &mut |p| found.push(p.to_path_buf())));
        assert!(rel(found).contains(&"packages/link/__mocks__/e.js".to_owned()));
        assert_eq!(
            log.listed,
            [
                (t.join("packages"), "**".to_owned()),
                (t.join("elsewhere"), "*".to_owned()),
                (t.join("elsewhere/__mocks__"), "*".to_owned()),
            ]
        );
        assert!(
            log.links.contains(&t.join("packages/link")),
            "{:?}",
            log.links
        );
        assert_eq!(log.asked, [t.join("packages/link")]);
    }

    #[test]
    fn a_runner_glob_as_a_listing() {
        assert_eq!(widen("*"), "*");
        assert_eq!(widen("*/"), "*");
        assert_eq!(widen("*/jest.config.js"), "*/jest.config.js");
        assert_eq!(widen("**/x-*.js"), "**/x-*.js");
        assert_eq!(widen("{a,b}/*"), "{a,b}/*");
        // A class: any name. `**` inside a segment is `*`.
        assert_eq!(widen("[ab]*/x"), "*/x");
        assert_eq!(widen("a**b/x"), "*/x");
        // No syntax of a listing: everything from there on.
        assert_eq!(widen("a/@(b|c)/d"), "a/**");
        assert_eq!(widen("!(a)/x"), "**");
        assert_eq!(widen("{a/b,c}/x"), "**");
        assert_eq!(widen("a/{1..3}/x"), "a/**");
    }

    #[test]
    fn field_lines() {
        let src = b"module.exports = {\n  roots: [],\n  testMatch: ['x'],\n};\n";
        assert_eq!(line_of_field(src, "testMatch"), 3);
        assert_eq!(line_of_field(src, "test"), 0);
        assert_eq!(line_of_field(src, "roots"), 2);
    }
}
