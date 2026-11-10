//! Directory-listing cache and lexical path helpers.
//!
//! Each directory is read at most once with [`std::fs::read_dir`]. Entry types
//! come from `d_type` through [`std::fs::DirEntry::file_type`], so no `stat`
//! happens unless the kind is unknown. Existence, kind and listing queries are
//! then answered from memory. `realpath` and `stat` are only called for entries
//! that are symlinks.
//!
//! Lookups match listing names exactly, the way Linux does. When an exact
//! lookup fails, [`DirCache::lookup`] retries ASCII case-insensitively and
//! reports [`Lookup::CaseMismatch`], which catches files that resolve on macOS
//! but break on Linux.

use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use dashmap::DashMap;

use crate::os::{OsStrExt, OsStringExt};

/// The kind of a directory entry as reported by `d_type` (symlinks are not
/// followed).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link.
    Symlink,
    /// Anything else (socket, fifo, device).
    Other,
}

/// The kind of a path after following symlinks.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// Anything else.
    Other,
}

/// Result of [`DirCache::stat`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stat {
    /// Kind after following symlinks.
    pub kind: FileKind,
    /// Whether the last path component is a symlink.
    pub is_symlink: bool,
}

/// One cached directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    name: Box<[u8]>,
    kind: EntryKind,
}

impl DirEntry {
    /// The entry name.
    pub fn name(&self) -> &OsStr {
        OsStr::from_bytes(&self.name)
    }

    /// The entry name as raw bytes.
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// The entry kind (symlinks not followed).
    pub fn kind(&self) -> EntryKind {
        self.kind
    }
}

/// The cached contents of one directory, sorted by name bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirListing {
    entries: Box<[DirEntry]>,
}

impl DirListing {
    /// Looks up an entry by exact name.
    pub fn get(&self, name: &[u8]) -> Option<&DirEntry> {
        self.entries
            .binary_search_by(|e| (*e.name).cmp(name))
            .ok()
            .map(|i| &self.entries[i])
    }

    /// Looks up an entry by ASCII case-insensitive name. If several entries
    /// match, returns the first in byte order.
    pub fn get_ignore_ascii_case(&self, name: &[u8]) -> Option<&DirEntry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Entries in byte order of their names.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &DirEntry> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the directory is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Result of [`DirCache::lookup`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// The path exists with exactly this spelling.
    Found(Stat),
    /// The path exists only with different ASCII casing, spelled `actual` on
    /// disk.
    CaseMismatch {
        /// The path as spelled on disk.
        actual: PathBuf,
        /// What it is.
        stat: Stat,
    },
    /// Nothing matches.
    Missing,
}

/// What a [`DirCache`] was asked on this thread while [`answers`] ran, whatever
/// it answered and whether or not recording was paused
/// ([`crate::record::pause`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// The paths that were looked up ([`DirCache::stat`] and what goes through
    /// it), normalized, in the order of the lookups, possibly repeated.
    pub asked: Vec<PathBuf>,
    /// The files whose contents were read ([`DirCache::read_cached`],
    /// [`DirCache::read_manifest`]).
    pub read: Vec<PathBuf>,
    /// The directories that were listed, and whether with everything below them
    /// ([`DirCache::walk_files`]).
    pub listed: Vec<(PathBuf, bool)>,
}

thread_local! {
    /// The open answer logs of this thread ([`answers`]), innermost last.
    static ANSWERS: std::cell::RefCell<Vec<Answers>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Whether one is open (read on every lookup).
    static ANSWERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` and returns, with its result, everything the caches were asked on
/// this thread.
///
/// A lookup that finds nothing is recorded ([`crate::record::missing`]): the
/// path may appear. One that finds something leaves no record, because whoever
/// asked usually goes on to read or link what was found, and a component may
/// pause recording while it loads something it records itself (a cargo
/// workspace). A caller that keeps a result for others, computed by a component
/// that asks the cache itself, needs all of it: a path may go, appear, or come
/// to be of another kind.
pub fn answers<R>(f: impl FnOnce() -> R) -> (R, Answers) {
    // An unwinding `f` leaves no log behind.
    struct Close(usize);
    impl Drop for Close {
        fn drop(&mut self) {
            ANSWERS.with(|a| a.borrow_mut().truncate(self.0));
            ANSWERING.set(self.0 > 0);
        }
    }
    let depth = ANSWERS.with(|a| {
        let mut a = a.borrow_mut();
        a.push(Answers::default());
        a.len()
    });
    ANSWERING.set(true);
    let close = Close(depth - 1);
    let r = f();
    let mut log = ANSWERS.with(|a| {
        let mut a = a.borrow_mut();
        a.truncate(depth);
        a.pop().unwrap_or_default()
    });
    drop(close);
    // What `f` asked is a part of what an enclosing caller asked.
    ANSWERS.with(|a| {
        if let Some(outer) = a.borrow_mut().last_mut() {
            outer.asked.extend(log.asked.iter().cloned());
            outer.read.extend(log.read.iter().cloned());
            outer.listed.extend(log.listed.iter().cloned());
        }
    });
    log.asked.dedup();
    (r, log)
}

fn answered(f: impl FnOnce(&mut Answers)) {
    if ANSWERING.get() {
        ANSWERS.with(|a| {
            if let Some(top) = a.borrow_mut().last_mut() {
                f(top);
            }
        });
    }
}

/// Information about a symlink, resolved once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// The canonical (`realpath`) target.
    pub target: Arc<Path>,
    /// The kind of the target.
    pub kind: FileKind,
}

/// Counters for benchmarks and `--stats`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DirCacheStats {
    /// Directories read with `read_dir` (including failed reads).
    pub dirs_read: u64,
    /// Symlinks resolved with `realpath`.
    pub links_resolved: u64,
}

type Slot<T> = Arc<OnceLock<T>>;

/// A cached file read: contents, or the error kind.
type ReadResult = Result<Arc<[u8]>, io::ErrorKind>;

/// A cached manifest view: the bytes and whether they are the view (else the
/// file as it is).
type ViewResult = Result<(Arc<[u8]>, bool), io::ErrorKind>;

/// A concurrent, read-once cache of directory listings.
///
/// Paths should be absolute. Paths containing `.` or `..` components are
/// normalized lexically first (see [`normalize`]).
///
/// The cache never invalidates: it assumes the filesystem does not change while
/// it is in use. Call [`DirCache::clear`] to drop everything.
#[derive(Debug, Default)]
pub struct DirCache {
    dirs: DashMap<Box<[u8]>, Slot<Option<Arc<DirListing>>>>,
    links: DashMap<Box<[u8]>, Slot<Option<LinkInfo>>>,
    /// Memo for [`DirCache::read_link`].
    readlinks: DashMap<Box<[u8]>, Slot<Option<Arc<Path>>>>,
    /// Memo for [`DirCache::read_cached`].
    contents: DashMap<Box<[u8]>, Slot<ReadResult>>,
    /// Memo for [`DirCache::read_manifest`].
    views: DashMap<Box<[u8]>, Slot<ViewResult>>,
    /// Memo for [`DirCache::read_manifest_loader`].
    loader_views: DashMap<Box<[u8]>, Slot<ViewResult>>,
    /// Memo for [`DirCache::dir_exact`].
    exact_dirs: DashMap<Box<[u8]>, bool>,
    /// Full canonicalization (nothing kept), for [`DirCache::canonicalize`].
    canon: LinkResolver,
    dirs_read: AtomicU64,
    links_resolved: AtomicU64,
    /// Report observations to [`crate::record`] (graph mode).
    record: bool,
    /// The tree the cache's users are confined to ([`DirCache::confine`]).
    confine: OnceLock<Confine>,
    /// A directory the cache does not show ([`DirCache::hide`]).
    hidden: OnceLock<PathBuf>,
    /// A base and a name: the directories of that name below the base are not
    /// shown ([`DirCache::hide_named`]).
    hidden_named: OnceLock<(PathBuf, Box<[u8]>)>,
}

/// The byte length of the directories that `path` and `base` (absolute,
/// normalized) have in common: `path[n..]` starts with `/` or is empty, and `0`
/// when they share none. See [`DirCache::local_offset`].
pub fn shared_prefix(base: &Path, path: &Path) -> usize {
    let (p, b) = (path.as_os_str().as_bytes(), base.as_os_str().as_bytes());
    if p.first() != Some(&b'/') || b.first() != Some(&b'/') {
        return 0;
    }
    // The end of the last component both have.
    let mut end = 0;
    let mut ours = p[1..].split(|&x| x == b'/');
    let mut theirs = b[1..].split(|&x| x == b'/');
    loop {
        match (ours.next(), theirs.next()) {
            (Some(x), Some(y)) if x == y && !x.is_empty() => end += 1 + x.len(),
            _ => return end,
        }
    }
}

/// `text` (a record's detail, an edge's label) with the paths next to or above
/// `base` (absolute, normalized) written from `base`: `<the directory base is
/// in>/sibling/a` becomes `../sibling/a`. For text the tool makes from a path
/// that a config or a resolver computed from a relative one (a mapper target
/// `<rootDir>/../shared/x`, a config another one extends): a record must not
/// hold the directories of the machine that built it, so that the same tree
/// gives the same graph wherever it is checked out. A path inside `base`
/// stays as it is: the graph store writes those from the root.
///
/// Not for text the source spells out: `/tmp/out.js` in code is `/tmp/out.js`
/// in the record, also for a checkout below `/tmp`.
pub fn from_base(base: &Path, text: &str) -> String {
    let name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '@');
    // The base and the directories above it, nearest first, each with what stands
    // for it. `/` is in every absolute path: nothing can be said about it.
    let dirs: Vec<(String, String)> = base
        .ancestors()
        .enumerate()
        .filter(|(_, d)| d.parent().is_some())
        .map(|(up, d)| (format!("{}/", d.display()), "../".repeat(up)))
        .collect();
    let mut out = String::with_capacity(text.len());
    // Start of the text not yet copied.
    let mut at = 0;
    for (i, c) in text.char_indices() {
        // Only where a path starts: `src/work/x` and `/a/work/x` name other
        // directories.
        if i < at || c != '/' {
            continue;
        }
        if text[..i]
            .chars()
            .next_back()
            .is_some_and(|c| name(c) || c == '/')
        {
            continue;
        }
        let rest = &text[i..];
        // The longest one first: a path inside the base stays as it is.
        if let Some((up, (dir, stands))) = dirs
            .iter()
            .enumerate()
            .find(|(_, (dir, _))| rest.starts_with(dir.as_str()))
        {
            out.push_str(&text[at..i]);
            out.push_str(if up == 0 { dir } else { stands });
            at = i + dir.len();
        }
    }
    out.push_str(&text[at..]);
    out
}

/// Directories systems keep temporary files in. A fixed list rather than the
/// build machine's `TMPDIR`: the answer must not depend on the host.
///
/// The list is for paths the source writes out
/// (`require('/tmp/out/bundle.js')`, `import('/var/tmp/gen-' + name)`): the
/// absolute path is then the code's, the same on every machine. It is never
/// asked about a path that the place of the checkout is part of: a relative
/// path that leaves the repository (`../../other/x.js`) lands below `/tmp`
/// exactly when the checkout is kept there.
pub const TEMP_DIRS: &[&str] = &[
    "/tmp",
    "/var/tmp",
    "/var/folders",
    "/private/tmp",
    "/private/var/tmp",
    "/private/var/folders",
    "/dev/shm",
];

/// Whether `path` (absolute, normalized) lies in a temporary directory
/// ([`TEMP_DIRS`]).
pub fn is_temp_path(path: &Path) -> bool {
    TEMP_DIRS.iter().any(|t| path.starts_with(t))
}

/// Whether `path` is below `/proc`, `/sys` or `/dev`: what is there
/// (`/proc/self/…`, `/dev/null`) is made by the kernel of the machine that runs
/// the code. No checkout holds its files there.
pub fn is_pseudo_fs(path: &Path) -> bool {
    let b = path.as_os_str().as_bytes();
    [&b"/proc"[..], b"/sys", b"/dev"]
        .iter()
        .any(|p| b.starts_with(p) && matches!(b.get(p.len()), None | Some(b'/')))
}

/// Directories that an operating system's file-system layout defines as the
/// place of the system's own programs, libraries, configuration and state. A
/// checkout is kept below some of them (`/usr/src/app`, `/home/me/repo`,
/// `/var/www/site`, `/usr/local/app`), never in their place: none of them is a
/// directory a repository is mounted at ([`seen_from_elsewhere`]).
pub const SYSTEM_DIRS: &[&str] = &[
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libexec",
    "/usr",
    "/usr/bin",
    "/usr/sbin",
    "/usr/lib",
    "/usr/lib32",
    "/usr/lib64",
    "/usr/libexec",
    "/usr/include",
    "/usr/share",
    "/usr/local",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/usr/local/lib",
    "/usr/local/include",
    "/usr/local/share",
    "/etc",
    "/var",
    "/var/lib",
    "/var/log",
    "/var/cache",
    "/var/run",
    "/var/spool",
    "/run",
    "/boot",
    "/home",
    "/Users",
    "/Library",
    "/System",
    "/Applications",
    "/private",
    "/private/var",
    "/private/etc",
];

/// One file or directory of the repository that a path may name which leaves
/// the repository by the way it is written ([`seen_from_elsewhere`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Elsewhere {
    /// The path from the repository root (`data/abs.json`): at least one
    /// component, no `.` and no `..`.
    pub rel: String,
    /// When the written path names it, as the end of a sentence: `the
    /// repository is mounted at /vercel/cmp/repo`, `the repository is
    /// checked out in a directory called repo`.
    pub when: String,
    /// The written path is an absolute one (else a relative one that climbs
    /// above the root).
    pub absolute: bool,
}

impl Elsewhere {
    /// The reading in one sentence, for a path written as `spelled` that names
    /// `what` (the file, or the files of a directory or a pattern):
    /// `/vercel/cmp/repo/data/abs.json, an absolute path: it names
    /// data/abs.json if the repository is mounted at /vercel/cmp/repo`.
    pub fn sentence(&self, spelled: &str, what: &str) -> String {
        let how = if self.absolute {
            "an absolute path"
        } else {
            "a path that leaves the repository"
        };
        format!("{spelled}, {how}: it names {what} if {}", self.when)
    }
}

/// What a path that the code writes as `spec` may name inside the repository
/// although it leaves it by the way it is written: an absolute path, or a
/// relative one that climbs above the root from a directory `depth` levels
/// below it (`None`: a directory that is not inside the repository; then only
/// an absolute path counts). Empty for every other path, which names what it
/// says.
///
/// The tree does not say where it will be when its code runs: a container build
/// copies it to `/app`, a CI system to `/home/runner/work/x/x`, and code that
/// is only ever run there may name its own files by that place
/// (`readFileSync('/app/fixtures/x.json')`). Where the checkout is on the
/// machine that builds the graph says nothing about it, and must not change the
/// answer. So such a path is read as the repository seen from another place:
///
/// - An absolute path names, for every directory it could be mounted at, what
///   is left of it: `/vercel/cmp/repo/data/abs.json` names
///   `vercel/cmp/repo/data/abs.json` (mounted at `/`),
///   `cmp/repo/data/abs.json`, `repo/data/abs.json`, `data/abs.json` and
///   `abs.json` from the root. The path itself as the mount point (nothing
///   left) names no file, and a directory of the system is no mount point
///   ([`SYSTEM_DIRS`]): `/usr/lib/x.so` names `usr/lib/x.so`, not `lib/x.so` or
///   `x.so`, and `/usr/src/app/a.json` names `usr/src/app/a.json`, `app/a.json`
///   and `a.json`.
/// - A relative path that climbs `k` levels above the root goes through `k`
///   directories whose names the tree does not know (its own directory and the
///   ones above): it is back inside exactly when they are called what the path
///   says, and then names what follows them. `../repo/data/x.json` from the
///   root names `data/x.json`. One reading, since the number of levels is
///   known.
///
/// Whether a candidate exists is the caller's question: one that does is a
/// dependency, one that does not is a probe (a file appearing there changes
/// what the path names).
///
/// Not read this way, because no checkout is kept there under a name the code
/// writes out: a path below a temporary directory ([`TEMP_DIRS`]: whatever is
/// there was made at run time, under a name of that run) and below the kernel's
/// file systems ([`is_pseudo_fs`]). Neither is a path through a `.git`
/// directory: the metadata of some checkout, wherever it is.
pub fn seen_from_elsewhere(depth: Option<usize>, spec: &str) -> Vec<Elsewhere> {
    if spec.starts_with('/') {
        let mut parts: Vec<&str> = Vec::new();
        for c in spec.split('/') {
            match c {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c => parts.push(c),
            }
        }
        let whole = PathBuf::from(format!("/{}", parts.join("/")));
        if is_temp_path(&whole) || is_pseudo_fs(&whole) || parts.contains(&".git") {
            return Vec::new();
        }
        return (0..parts.len())
            .map(|i| (i, format!("/{}", parts[..i].join("/"))))
            .filter(|(_, mount)| !SYSTEM_DIRS.contains(&mount.as_str()))
            .map(|(i, mount)| Elsewhere {
                rel: parts[i..].join("/"),
                when: format!("the repository is mounted at {mount}"),
                absolute: true,
            })
            .collect();
    }
    let Some(mut below) = depth else {
        return Vec::new();
    };
    // Levels above the root, and the names that follow them.
    let mut up = 0;
    let mut parts: Vec<&str> = Vec::new();
    for c in spec.split('/') {
        match c {
            "" | "." => {}
            ".." if !parts.is_empty() => {
                parts.pop();
            }
            ".." if below > 0 => below -= 1,
            ".." => up += 1,
            c => parts.push(c),
        }
    }
    if up == 0 || parts.len() <= up || parts.contains(&".git") {
        return Vec::new();
    }
    vec![Elsewhere {
        rel: parts[up..].join("/"),
        when: format!(
            "the repository is checked out in a directory called {}",
            parts[..up].join("/")
        ),
        absolute: false,
    }]
}

/// The reading of `spec`, an absolute path that the code writes for a directory
/// (the literal start of a computed path, `'/app/' + name`, or a directory it
/// lists, `readdirSync('/app')`), in which the path itself is the place the
/// repository is mounted at: it names the repository root, [`Elsewhere::rel`]
/// is empty. [`seen_from_elsewhere`] leaves this reading out, because a path
/// that names a file names none there; a computed name below the directory, or
/// its listing, is a read of the root with a computed name.
///
/// `None` for a path that is no place a repository is mounted at: a relative
/// one, the file-system root itself (`'/' + name` says nothing about a place),
/// a directory of the system ([`SYSTEM_DIRS`]), a temporary directory, the
/// kernel's file systems, and a path through `.git`.
pub fn mounted_at(spec: &str) -> Option<Elsewhere> {
    if !spec.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for c in spec.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mount = format!("/{}", parts.join("/"));
    let whole = Path::new(&mount);
    if parts.is_empty()
        || is_temp_path(whole)
        || is_pseudo_fs(whole)
        || parts.contains(&".git")
        || SYSTEM_DIRS.contains(&mount.as_str())
    {
        return None;
    }
    Some(Elsewhere {
        rel: String::new(),
        when: format!("the repository is mounted at {mount}"),
        absolute: true,
    })
}

/// See [`DirCache::confine`].
#[derive(Debug)]
struct Confine {
    base: PathBuf,
    /// Directories outside `base` that belong to the project all the same.
    roots: Vec<PathBuf>,
}

impl DirCache {
    /// Declares the tree the result is a function of
    /// ([`crate::TraceOptions::confined`]): `base` (the repository root)
    /// plus `roots`, the directories outside it that hold the
    /// project's installed dependencies (a language's module cache or site
    /// directory). Every other path is a *host path*
    /// ([`DirCache::is_host_path`]).
    ///
    /// The cache itself still answers any query: module resolution legitimately
    /// follows links out of the base (a package store, a `node_modules`
    /// above it). The declaration is for the cache's users that take paths
    /// from source text: the asset resolvers
    /// ([`crate::ResolveCx::is_host_path`]) and the task and framework
    /// adapters, which must not ask about a host path. The first call wins.
    pub fn confine(&self, base: &Path, roots: impl IntoIterator<Item = PathBuf>) {
        let _ = self.confine.set(Confine {
            base: normalize(base),
            roots: roots.into_iter().collect(),
        });
    }

    /// Declares `dir` (absolute) as not part of the tree: its parent's listing
    /// leaves it out and it cannot be listed, so for every query of the
    /// cache neither it nor anything below it exists. Graph mode hides the
    /// graph directory when it is inside the repository: a build writes
    /// into it, so an observation of it (an entry of a listing, a
    /// probe below it, its mtime) would differ after every build and no refresh
    /// would find the tree unchanged. Call it before the cache is used; the
    /// first call wins.
    pub fn hide(&self, dir: &Path) {
        let _ = self.hidden.set(normalize(dir));
    }

    /// [`DirCache::hide`] for every directory called `name` in `base`
    /// (absolute) and below it. Graph mode hides `.turbotrace`, the default
    /// name of a graph directory: a graph that another build keeps inside
    /// the repository is not part of the tree either, so two builds of one
    /// tree agree wherever each keeps its graph. Call it before the cache
    /// is used; the first call wins.
    pub fn hide_named(&self, base: &Path, name: &str) {
        let _ = self
            .hidden_named
            .set((normalize(base), name.as_bytes().into()));
    }

    /// Whether the cache is confined ([`DirCache::confine`]).
    pub fn confined(&self) -> bool {
        self.confine.get().is_some()
    }

    /// The base of a confined cache ([`DirCache::confine`]): the repository
    /// root.
    pub fn base(&self) -> Option<&Path> {
        self.confine.get().map(|c| c.base.as_path())
    }

    /// Whether `path` (absolute, normalized) is a host path: the cache is
    /// confined ([`DirCache::confine`]) and `path` is neither inside the
    /// base nor below one of the declared roots. What exists at a host path
    /// (`/tmp`, `/proc`, a home directory, a sibling checkout) is a
    /// property of the machine, not of the repository.
    ///
    /// The base's ancestors are host paths too: which directories the checkout
    /// lives below (`/tmp/ci/repo`, `/home/me/repo`) is an accident of the
    /// machine, and a graph must not depend on it.
    pub fn is_host_path(&self, path: &Path) -> bool {
        let Some(c) = self.confine.get() else {
            return false;
        };
        (!path.starts_with(&c.base) && !c.roots.iter().any(|r| path.starts_with(r)))
            || self.is_repository_metadata(path)
    }

    /// Whether `path` (absolute, normalized) is the repository's `.git` or lies
    /// inside it: the cache is confined ([`DirCache::confine`]) and a
    /// component of `path` below the base is `.git`. What `.git` holds
    /// (objects, refs, hooks, a worktree's link file) is the history and
    /// the state of one checkout, not part of the tree a graph is a function
    /// of: two checkouts of one commit differ there. So it is a host path
    /// ([`DirCache::is_host_path`]): never looked up, listed or counted.
    pub fn is_repository_metadata(&self, path: &Path) -> bool {
        let Some(c) = self.confine.get() else {
            return false;
        };
        path.strip_prefix(&c.base)
            .is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == ".git"))
    }

    /// Whether `dir` is one of the confined base's ancestors (`false` when not
    /// confined).
    pub fn is_above_base(&self, dir: &Path) -> bool {
        self.confine
            .get()
            .is_some_and(|c| c.base != dir && c.base.starts_with(dir))
    }

    /// Where the part of `path` (absolute, normalized) starts that the place of
    /// the checkout has no say in: the byte offset behind the last
    /// directory that `path` and the confined base have in common, so that
    /// `path[offset..]` starts with `/` or is empty. `0` when the cache is
    /// not confined.
    ///
    /// The directories a checkout lives below are an accident of the machine
    /// ([`DirCache::is_host_path`]), and so are their names: a checkout below
    /// `/ci/node_modules/app` or `/home/me/vendor` holds the same files as one
    /// below `/work`. Whatever the names of a path's components decide (a
    /// package marker such as `node_modules`, `.git`, a test directory) is
    /// decided on the part from this offset on. For a path inside the base
    /// that is the path relative to it; for one outside (a package store
    /// next to the checkout, reached through a link) it is the part below
    /// the directory where the path leaves the way to the base.
    pub fn local_offset(&self, path: &Path) -> usize {
        self.confine
            .get()
            .map_or(0, |c| shared_prefix(&c.base, path))
    }

    /// The part of `path` (absolute, normalized) from
    /// [`DirCache::local_offset`] on, as a relative path: for a path inside
    /// the confined base the path relative to it. The whole path when it
    /// shares no directory with the base or the cache is not confined.
    pub fn local<'p>(&self, path: &'p Path) -> &'p Path {
        let off = self.local_offset(path);
        if off == 0 {
            return path;
        }
        let rest = &path.as_os_str().as_bytes()[off..];
        Path::new(OsStr::from_bytes(rest.strip_prefix(b"/").unwrap_or(rest)))
    }

    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty cache that reports what it is asked to the open
    /// [`crate::record`] frame of the calling thread (graph mode): misses of
    /// the public existence queries ([`DirCache::stat`] and everything
    /// built on it) as missing paths, [`DirCache::list`] and
    /// [`DirCache::walk_files`] as `(dir, "**")` listings, and
    /// [`DirCache::read_cached`] hits as reads. Internal lookups (ancestor
    /// spelling checks, symlink walks) are not reported.
    pub fn with_recording() -> Self {
        DirCache {
            record: true,
            ..Self::default()
        }
    }

    /// Whether this cache reports to [`crate::record`].
    pub fn records(&self) -> bool {
        self.record
    }

    /// Returns the listing of `dir`, reading it on first access.
    ///
    /// Returns `None` if `dir` does not exist, is not a directory, or cannot be
    /// read. Each directory is read at most once even under concurrent
    /// access: the slot is created under the shard lock, and the read
    /// happens outside it through a `OnceLock`.
    ///
    /// With recording, the call is reported as a `(dir, "**")` listing: the
    /// caller enumerates the directory, and a new entry can appear through
    /// a file added anywhere below it.
    pub fn list(&self, dir: &Path) -> Option<Arc<DirListing>> {
        if self.record {
            crate::record::listing(&normalize_if_needed(dir), "**");
        }
        answered(|a| {
            a.listed
                .push((normalize_if_needed(dir).into_owned(), false))
        });
        self.list_quiet(dir)
    }

    /// The listing of `dir` if this cache has already read it (no read, no
    /// recording).
    pub fn cached_listing(&self, dir: &Path) -> Option<Arc<DirListing>> {
        let dir = normalize_if_needed(dir);
        let slot = Arc::clone(&*self.dirs.get(dir.as_os_str().as_bytes())?);
        slot.get().cloned().flatten()
    }

    /// [`DirCache::list`] without recording, for internal existence checks.
    pub(crate) fn list_quiet(&self, dir: &Path) -> Option<Arc<DirListing>> {
        let dir = normalize_if_needed(dir);
        let key = dir.as_os_str().as_bytes();
        let slot = match self.dirs.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.dirs.entry(key.into()).or_default()),
        };
        slot.get_or_init(|| self.read_dir(&dir)).clone()
    }

    fn read_dir(&self, dir: &Path) -> Option<Arc<DirListing>> {
        self.dirs_read.fetch_add(1, Ordering::Relaxed);
        // The hidden directory: not listed, and not an entry of its parent.
        let hidden = self.hidden.get();
        if hidden.is_some_and(|h| dir.starts_with(h)) {
            return None;
        }
        let hidden_name = hidden
            .filter(|h| h.parent() == Some(dir))
            .and_then(|h| h.file_name());
        // Directories hidden by name, in `dir` when it is the base or below it; `dir`
        // itself is hidden when one of its components below the base has that name.
        let mut named: Option<&[u8]> = None;
        if let Some((base, name)) = self.hidden_named.get()
            && let Ok(rel) = dir.strip_prefix(base)
        {
            let name: &[u8] = name;
            if rel.components().any(|c| c.as_os_str().as_bytes() == name) {
                return None;
            }
            named = Some(name);
        }
        let rd = std::fs::read_dir(dir).ok()?;
        let mut entries: Vec<DirEntry> = rd
            .filter_map(|e| {
                let e = e.ok()?;
                if hidden_name.is_some_and(|n| n == e.file_name()) {
                    return None;
                }
                // `file_type` uses `d_type` and only falls back to `lstat` when the
                // filesystem reports DT_UNKNOWN.
                let kind = match e.file_type() {
                    Ok(t) if t.is_symlink() => EntryKind::Symlink,
                    Ok(t) if t.is_dir() => EntryKind::Dir,
                    Ok(t) if t.is_file() => EntryKind::File,
                    Ok(_) => EntryKind::Other,
                    Err(_) => return None,
                };
                let hidden_by_name =
                    kind == EntryKind::Dir && named.is_some_and(|n| n == e.file_name().as_bytes());
                if hidden_by_name {
                    return None;
                }
                Some(DirEntry {
                    name: e.file_name().into_vec().into_boxed_slice(),
                    kind,
                })
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Some(Arc::new(DirListing {
            entries: entries.into_boxed_slice(),
        }))
    }

    /// The kind of `path`, following symlinks, or `None` if it does not exist
    /// exactly as spelled.
    pub fn stat(&self, path: &Path) -> Option<Stat> {
        let r = self.stat_quiet(path);
        if self.record && r.is_none() {
            crate::record::missing(&normalize_if_needed(path));
        }
        answered(|a| a.asked.push(normalize_if_needed(path).into_owned()));
        r
    }

    fn stat_quiet(&self, path: &Path) -> Option<Stat> {
        self.stat_or_link(path).ok()
    }

    /// [`DirCache::stat`], which also says what a path without a kind is:
    /// `Err(true)` for a symbolic link that leads to nothing (an entry of
    /// its directory, with no file or directory at its end), `Err(false)`
    /// when nothing of that name is there. A lookup that follows links
    /// finds nothing at either, and goes on at another place for the first:
    /// where the link leads.
    pub fn stat_or_dangling(&self, path: &Path) -> Result<Stat, bool> {
        let r = self.stat_or_link(path);
        if self.record && r.is_err() {
            crate::record::missing(&normalize_if_needed(path));
        }
        r
    }

    fn stat_or_link(&self, path: &Path) -> Result<Stat, bool> {
        let path = normalize_if_needed(path);
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            // The root directory (or an empty path).
            return if path.as_os_str() == "/" {
                Ok(Stat {
                    kind: FileKind::Dir,
                    is_symlink: false,
                })
            } else {
                Err(false)
            };
        };
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if !self.dir_exact(parent) {
            return Err(false);
        }
        let Some(listing) = self.list_quiet(parent) else {
            return Err(false);
        };
        let Some(entry) = listing.get(name.as_bytes()) else {
            return Err(false);
        };
        let kind = match entry.kind {
            EntryKind::File => FileKind::File,
            EntryKind::Dir => FileKind::Dir,
            EntryKind::Other => FileKind::Other,
            EntryKind::Symlink => {
                return match self.link_info(&path) {
                    Some(l) => Ok(Stat {
                        kind: l.kind,
                        is_symlink: true,
                    }),
                    None => Err(true),
                };
            }
        };
        Ok(Stat {
            kind,
            is_symlink: false,
        })
    }

    /// Whether directory `dir` exists with every component spelled exactly as
    /// on disk.
    ///
    /// On case-insensitive filesystems (macOS default) `read_dir` succeeds for
    /// a mis-cased directory, so listing success alone does not prove exact
    /// spelling. Memoized per directory; costs one binary search per
    /// component on first use.
    pub fn dir_exact(&self, dir: &Path) -> bool {
        let key = dir.as_os_str().as_bytes();
        if let Some(v) = self.exact_dirs.get(key) {
            return *v;
        }
        let ok = match (dir.parent(), dir.file_name()) {
            (Some(parent), Some(name)) => {
                let parent = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
                self.dir_exact(parent)
                    && self
                        .list_quiet(parent)
                        .is_some_and(|l| match l.get(name.as_bytes()) {
                            Some(e) => match e.kind {
                                EntryKind::Dir => true,
                                EntryKind::Symlink => matches!(
                                    self.link_info(dir),
                                    Some(LinkInfo {
                                        kind: FileKind::Dir,
                                        ..
                                    })
                                ),
                                _ => false,
                            },
                            None => false,
                        })
            }
            // Root, `.` or empty: trust the OS.
            _ => self.list_quiet(dir).is_some(),
        };
        self.exact_dirs.insert(key.into(), ok);
        ok
    }

    /// Whether `path` exists exactly as spelled (following symlinks).
    pub fn exists(&self, path: &Path) -> bool {
        self.stat(path).is_some()
    }

    /// Whether `path` is a regular file (following symlinks).
    pub fn is_file(&self, path: &Path) -> bool {
        matches!(
            self.stat(path),
            Some(Stat {
                kind: FileKind::File,
                ..
            })
        )
    }

    /// Whether `path` is a directory (following symlinks).
    pub fn is_dir(&self, path: &Path) -> bool {
        matches!(
            self.stat(path),
            Some(Stat {
                kind: FileKind::Dir,
                ..
            })
        )
    }

    /// Like [`DirCache::stat`], but on a miss retries with ASCII
    /// case-insensitive matching of every component and reports
    /// [`Lookup::CaseMismatch`] with the on-disk spelling.
    pub fn lookup(&self, path: &Path) -> Lookup {
        let path = normalize_if_needed(path);
        if let Some(stat) = self.stat(&path) {
            return Lookup::Found(stat);
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Lookup::Missing;
        };
        let parent_actual = if self.dir_exact(parent) {
            parent.to_path_buf()
        } else {
            match self.lookup(parent) {
                Lookup::CaseMismatch {
                    actual,
                    stat:
                        Stat {
                            kind: FileKind::Dir,
                            ..
                        },
                } => actual,
                _ => return Lookup::Missing,
            }
        };
        let Some(listing) = self.list_quiet(&parent_actual) else {
            return Lookup::Missing;
        };
        if let Some(e) = listing.get_ignore_ascii_case(name.as_bytes()) {
            let actual = parent_actual.join(e.name());
            if let Some(stat) = self.stat(&actual) {
                return Lookup::CaseMismatch { actual, stat };
            }
        }
        Lookup::Missing
    }

    /// Resolves the symlink at `path` (realpath of the target plus its kind),
    /// once.
    ///
    /// Returns `None` for dangling links or when resolution fails. Only call
    /// this for paths whose last component is a symlink; [`DirCache::stat`]
    /// does so automatically.
    pub fn link_info(&self, path: &Path) -> Option<LinkInfo> {
        let key = path.as_os_str().as_bytes();
        let slot = match self.links.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.links.entry(key.into()).or_default()),
        };
        slot.get_or_init(|| {
            self.links_resolved.fetch_add(1, Ordering::Relaxed);
            let target = std::fs::canonicalize(path).ok()?;
            let meta = std::fs::metadata(&target).ok()?;
            let kind = if meta.is_dir() {
                FileKind::Dir
            } else if meta.is_file() {
                FileKind::File
            } else {
                FileKind::Other
            };
            Some(LinkInfo {
                target: Arc::from(target),
                kind,
            })
        })
        .clone()
    }

    /// The raw target of the symlink at `path` (`readlink`), read once. `None`
    /// if `path` is not a symlink or cannot be read. Only call this for
    /// entries the listing reports as [`EntryKind::Symlink`].
    pub fn read_link(&self, path: &Path) -> Option<Arc<Path>> {
        let key = path.as_os_str().as_bytes();
        let slot = match self.readlinks.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.readlinks.entry(key.into()).or_default()),
        };
        slot.get_or_init(|| {
            self.links_resolved.fetch_add(1, Ordering::Relaxed);
            std::fs::read_link(path).ok().map(Arc::from)
        })
        .clone()
    }

    /// The contents of a small configuration file (`package.json`,
    /// `tsconfig.json`, `go.mod`, ...), read at most once and shared by
    /// every caller.
    ///
    /// Existence is answered from the directory listing first, so probing for a
    /// missing manifest in every ancestor directory costs no syscall. Only
    /// use this for files that are small and read repeatedly: contents stay
    /// cached until [`DirCache::clear`].
    pub fn read_cached(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        let path = normalize_if_needed(path);
        if !self.is_file(&path) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if self.record {
            crate::record::read(&path);
        }
        answered(|a| a.read.push(path.to_path_buf()));
        self.contents_of(&path).map_err(io::Error::from)
    }

    /// The cached contents of the file at `path` (normalized), without
    /// recording.
    fn contents_of(&self, path: &Path) -> ReadResult {
        let key = path.as_os_str().as_bytes();
        let slot = match self.contents.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.contents.entry(key.into()).or_default()),
        };
        slot.get_or_init(|| std::fs::read(path).map(Arc::from).map_err(|e| e.kind()))
            .clone()
    }

    /// The `package.json` at `path` as module resolution gets it: its
    /// [`crate::manifest::view`], the file without the fields resolution never
    /// reads. The read is recorded as a view read
    /// ([`crate::record::view_read`]): whoever resolves through this
    /// depends on the view's bytes, and a dependency entry or a script that
    /// changes in the file is no change for it.
    ///
    /// A manifest that has no view (it is not a JSON object) is returned, and
    /// recorded, as the file it is. Read once and shared, like
    /// [`DirCache::read_cached`].
    pub fn read_manifest(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        let path = normalize_if_needed(path);
        if !self.is_file(&path) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let key = path.as_os_str().as_bytes();
        let slot = match self.views.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.views.entry(key.into()).or_default()),
        };
        let r = slot
            .get_or_init(|| {
                let bytes = self.contents_of(&path)?;
                Ok(match crate::manifest::view(&bytes) {
                    Some(v) => (Arc::from(v), true),
                    None => (bytes, false),
                })
            })
            .clone();
        if self.record {
            match &r {
                Ok((_, true)) => crate::record::view_read(&path),
                _ => crate::record::read(&path),
            }
        }
        answered(|a| a.read.push(path.to_path_buf()));
        r.map(|(bytes, _)| bytes).map_err(io::Error::from)
    }

    /// The `package.json` at `path` as the loader of a `tsconfig.json` gets it:
    /// its [`crate::manifest::loader_view`], the fields that the resolution
    /// of an `extends` entry can read. The read is recorded as a loader
    /// read ([`crate::record::loader_read`]): a config load depends on
    /// those bytes and on nothing else of the file.
    ///
    /// A manifest that is not a JSON object is returned, and recorded, as the
    /// file it is. Read once and shared, like [`DirCache::read_cached`].
    pub fn read_manifest_loader(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        let path = normalize_if_needed(path);
        if !self.is_file(&path) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let key = path.as_os_str().as_bytes();
        let slot = match self.loader_views.get(key) {
            Some(s) => Arc::clone(&s),
            None => Arc::clone(&self.loader_views.entry(key.into()).or_default()),
        };
        let r = slot
            .get_or_init(|| {
                let bytes = self.contents_of(&path)?;
                Ok(match crate::manifest::loader_view(&bytes) {
                    Some(v) => (Arc::from(v), true),
                    None => (bytes, false),
                })
            })
            .clone();
        if self.record {
            match &r {
                Ok((_, true)) => crate::record::loader_read(&path),
                _ => crate::record::read(&path),
            }
        }
        r.map(|(bytes, _)| bytes).map_err(io::Error::from)
    }

    /// The real path of `path` with every symlinked component resolved, plus
    /// each link crossed. See [`LinkResolver`]; this variant resolves every
    /// component.
    pub fn canonicalize(&self, path: &Path) -> RealPath {
        self.canon.resolve(self, path)
    }

    /// If the last component of `path` is a symlink, its canonical target.
    pub fn symlink_target(&self, path: &Path) -> Option<Arc<Path>> {
        match self.stat(path) {
            Some(Stat {
                is_symlink: true, ..
            }) => self.link_info(&normalize_if_needed(path)).map(|l| l.target),
            _ => None,
        }
    }

    /// All files below `dir`, recursively (depth-first, entries in name order),
    /// stopping after `limit` files. Returns the files sorted by path bytes
    /// and whether the walk was truncated.
    ///
    /// Symlinks to files are included; symlinks to directories are not
    /// descended into, which avoids cycles.
    pub fn walk_files(&self, dir: &Path, limit: usize) -> (Vec<PathBuf>, bool) {
        let mut out = Vec::new();
        let dir = normalize_if_needed(dir);
        if self.record {
            crate::record::listing(&dir, "**");
        }
        answered(|a| a.listed.push((dir.to_path_buf(), true)));
        let truncated = self.walk_into(&dir, limit, &mut out);
        out.sort_unstable_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
        (out, truncated)
    }

    fn walk_into(&self, dir: &Path, limit: usize, out: &mut Vec<PathBuf>) -> bool {
        let Some(listing) = self.list_quiet(dir) else {
            return false;
        };
        for e in listing.iter() {
            let path = dir.join(e.name());
            let is_file = match e.kind {
                EntryKind::File => true,
                EntryKind::Dir => {
                    if self.walk_into(&path, limit, out) {
                        return true;
                    }
                    false
                }
                EntryKind::Symlink => {
                    matches!(
                        self.link_info(&path),
                        Some(LinkInfo {
                            kind: FileKind::File,
                            ..
                        })
                    )
                }
                EntryKind::Other => false,
            };
            if is_file {
                if out.len() >= limit {
                    return true;
                }
                out.push(path);
            }
        }
        false
    }

    /// Counters.
    pub fn stats(&self) -> DirCacheStats {
        DirCacheStats {
            dirs_read: self.dirs_read.load(Ordering::Relaxed),
            links_resolved: self.links_resolved.load(Ordering::Relaxed),
        }
    }

    /// Drops all cached listings and links.
    pub fn clear(&self) {
        self.dirs.clear();
        self.links.clear();
        self.readlinks.clear();
        self.contents.clear();
        self.views.clear();
        self.loader_views.clear();
        self.exact_dirs.clear();
        self.canon.clear();
    }
}

/// Maximum symlinks followed while resolving one path (Linux `MAXSYMLINKS` is
/// 40).
const MAX_LINK_DEPTH: u32 = 40;

/// A path with its symlinked components resolved, from
/// [`LinkResolver::resolve`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RealPath {
    /// The resolved path.
    pub path: PathBuf,
    /// Every symlink crossed, in walk order, as `(link path, resolved target)`.
    /// Each link path is spelled with its parent already resolved, so it is
    /// a real location on disk.
    pub links: Vec<(PathBuf, PathBuf)>,
}

/// Resolves symlinked path components the way `realpath` does, but from the dir
/// cache: components are looked up in cached listings, and `readlink` is only
/// called for entries whose `d_type` says they are symlinks. Every
/// link crossed is reported, which is what pnpm layouts (`node_modules/foo ->
/// .pnpm/foo@1/node_modules/foo`) need for nft's `symlinks` map and for
/// including the link paths themselves.
///
/// Components that are the `keep` directory or one of its ancestors are never
/// resolved, as in nft, which only resolves symlinks below its base. This keeps
/// output paths relative to a base like `/var/...` on macOS (where `/var` links
/// to `/private/var`). With `keep = "/"` every component is resolved.
///
/// Link targets are resolved lexically against the (resolved) link directory
/// and then walked again, so chains of links are followed. Results for
/// directories are memoized.
#[derive(Debug)]
pub struct LinkResolver {
    keep: PathBuf,
    dirs: DashMap<Box<[u8]>, Arc<RealPath>>,
}

impl Default for LinkResolver {
    fn default() -> Self {
        LinkResolver::new("/")
    }
}

impl LinkResolver {
    /// A resolver that leaves `keep` and its ancestors unresolved.
    pub fn new(keep: impl AsRef<Path>) -> Self {
        LinkResolver {
            keep: normalize(keep.as_ref()),
            dirs: DashMap::new(),
        }
    }

    /// The directory whose components (and ancestors) are kept as spelled.
    pub fn keep(&self) -> &Path {
        &self.keep
    }

    /// Resolves `path` (absolute; normalized lexically first).
    pub fn resolve(&self, fs: &DirCache, path: &Path) -> RealPath {
        let path = normalize_if_needed(path);
        self.resolve_at(fs, &path, 0)
    }

    /// Drops memoized results.
    pub fn clear(&self) {
        self.dirs.clear();
    }

    fn resolve_at(&self, fs: &DirCache, path: &Path, depth: u32) -> RealPath {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return RealPath {
                path: path.to_path_buf(),
                links: Vec::new(),
            };
        };
        if self.keep.starts_with(path) || parent.as_os_str().is_empty() {
            return RealPath {
                path: path.to_path_buf(),
                links: Vec::new(),
            };
        }
        let dir = self.resolve_dir(fs, parent, depth);
        self.step(fs, &dir, name, depth)
    }

    fn resolve_dir(&self, fs: &DirCache, dir: &Path, depth: u32) -> Arc<RealPath> {
        let key = dir.as_os_str().as_bytes();
        if let Some(r) = self.dirs.get(key) {
            return Arc::clone(&r);
        }
        let r = Arc::new(self.resolve_at(fs, dir, depth));
        Arc::clone(&self.dirs.entry(key.into()).or_insert(r))
    }

    fn step(&self, fs: &DirCache, dir: &RealPath, name: &OsStr, depth: u32) -> RealPath {
        let cand = dir.path.join(name);
        let is_link = fs
            .list_quiet(&dir.path)
            .and_then(|l| l.get(name.as_bytes()).map(|e| e.kind == EntryKind::Symlink))
            .unwrap_or(false);
        let mut links = dir.links.clone();
        if is_link
            && depth < MAX_LINK_DEPTH
            && let Some(target) = fs.read_link(&cand)
        {
            let resolved = normalize(&dir.path.join(&*target));
            let r = self.resolve_at(fs, &resolved, depth + 1);
            links.push((cand, r.path.clone()));
            links.extend(r.links);
            return RealPath {
                path: r.path,
                links,
            };
        }
        RealPath { path: cand, links }
    }
}

/// Lexically normalizes a path: removes `.` components and empty components,
/// resolves `..` against the preceding component, and strips trailing slashes.
///
/// `..` at the root of an absolute path stays at the root. Leading `..` in a
/// relative path is kept. Symlinks are not consulted, matching Node's
/// `path.resolve` semantics.
pub fn normalize(path: &Path) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    let absolute = bytes.first() == Some(&b'/');
    let mut parts: Vec<&[u8]> = Vec::new();
    for comp in bytes.split(|&b| b == b'/') {
        match comp {
            b"" | b"." => {}
            b".." => match parts.last() {
                Some(&last) if last != b".." => {
                    parts.pop();
                }
                _ if absolute => {}
                _ => parts.push(comp),
            },
            _ => parts.push(comp),
        }
    }
    let mut out = Vec::with_capacity(bytes.len());
    if absolute {
        out.push(b'/');
    }
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(b'/');
        }
        out.extend_from_slice(p);
    }
    if out.is_empty() {
        out.push(b'.');
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// Joins `rel` onto `base` and normalizes. An absolute `rel` replaces `base`.
pub fn join_normalize(base: &Path, rel: impl AsRef<Path>) -> PathBuf {
    normalize(&base.join(rel))
}

/// Whether `path` would be changed by [`normalize`].
pub fn needs_normalize(path: &Path) -> bool {
    let b = path.as_os_str().as_bytes();
    if b == b"/" {
        return false;
    }
    if b.len() > 1 && b.last() == Some(&b'/') {
        return true;
    }
    b.split(|&c| c == b'/')
        .enumerate()
        .any(|(i, c)| c == b"." || c == b".." || (c.is_empty() && i > 0))
}

fn normalize_if_needed(path: &Path) -> std::borrow::Cow<'_, Path> {
    if needs_normalize(path) {
        std::borrow::Cow::Owned(normalize(path))
    } else {
        std::borrow::Cow::Borrowed(path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn text_is_written_from_the_base() {
        let base = Path::new("/work/ci/repo");
        let t = |s: &str| from_base(base, s);
        // Inside: left to the store.
        assert_eq!(
            t("not found /work/ci/repo/a/tsconfig.json"),
            "not found /work/ci/repo/a/tsconfig.json"
        );
        assert_eq!(
            t("not found /work/ci/base/tsconfig.json"),
            "not found ../base/tsconfig.json"
        );
        assert_eq!(t("x -> /work/outside/thing"), "x -> ../../outside/thing");
        assert_eq!(
            t("/etc/x, /work/ci/repo2/y and /work/ci/repo/z"),
            "/etc/x, ../repo2/y and /work/ci/repo/z"
        );
        assert_eq!(t("nothing here"), "nothing here");
        // Only where a path starts: a directory of the repository, or of another path,
        // that is called like one the checkout lives in is not that directory.
        assert_eq!(
            t("@/src/work/x -> /work/ci/repo/src/work/x"),
            "@/src/work/x -> /work/ci/repo/src/work/x"
        );
        assert_eq!(
            t("a -> /work/outside/work/ci/x"),
            "a -> ../../outside/work/ci/x"
        );
        assert_eq!(t("/opt/work/ci/x"), "/opt/work/ci/x");
        assert_eq!(
            from_base(Path::new("/repo"), "/x/y and /repo/z"),
            "/x/y and /repo/z"
        );
    }

    /// The reading in which an absolute directory is itself the place the
    /// repository is mounted at: for a computed name below it and for its
    /// listing.
    #[test]
    fn an_absolute_directory_may_be_where_the_repository_is_mounted() {
        for (spec, mount) in [
            ("/app/", "/app"),
            ("/app", "/app"),
            ("/usr/src/app/", "/usr/src/app"),
            ("/home/runner/work/x/x/", "/home/runner/work/x/x"),
            ("/app/src/../", "/app"),
            ("/var/task/", "/var/task"),
        ] {
            let e = mounted_at(spec).unwrap_or_else(|| panic!("{spec}"));
            assert_eq!(e.rel, "", "{spec}");
            assert!(e.absolute, "{spec}");
            assert_eq!(
                e.when,
                format!("the repository is mounted at {mount}"),
                "{spec}"
            );
        }
        assert_eq!(
            mounted_at("/app/")
                .unwrap()
                .sentence("/app/*", "the files of the repository"),
            "/app/*, an absolute path: it names the files of the repository if the repository is \
             mounted at /app"
        );
        // No place a repository is mounted at.
        for spec in [
            "/",
            "",
            "app/",
            "../app/",
            "/usr/",
            "/usr/lib",
            "/etc/",
            "/home",
            "/tmp/",
            "/tmp/x/",
            "/var/folders/ab/",
            "/proc/1/",
            "/dev/shm/",
            "/srv/app/.git/",
        ] {
            assert_eq!(mounted_at(spec), None, "{spec}");
        }
    }

    #[test]
    fn a_path_that_leaves_the_repository_names_what_its_trailing_components_name() {
        let rels = |depth: Option<usize>, spec: &str| -> Vec<String> {
            seen_from_elsewhere(depth, spec)
                .into_iter()
                .map(|e| e.rel)
                .collect()
        };
        // An absolute path: every place it could be mounted at, the file system root
        // first.
        assert_eq!(
            rels(Some(0), "/vercel/cmp/repo/data/abs.json"),
            [
                "vercel/cmp/repo/data/abs.json",
                "cmp/repo/data/abs.json",
                "repo/data/abs.json",
                "data/abs.json",
                "abs.json"
            ]
        );
        // The same from every directory, also from one outside the repository.
        assert_eq!(rels(Some(3), "/app/x.json"), ["app/x.json", "x.json"]);
        assert_eq!(rels(None, "/app/x.json"), ["app/x.json", "x.json"]);
        assert_eq!(
            rels(Some(0), "/app/./a/../x.json"),
            ["app/x.json", "x.json"]
        );
        assert_eq!(
            rels(Some(0), "/app/fixtures/"),
            ["app/fixtures", "fixtures"]
        );
        // The mount point itself names no file.
        assert_eq!(rels(Some(0), "/"), [""; 0]);
        assert_eq!(rels(Some(0), "/app"), ["app"]);
        let e = &seen_from_elsewhere(Some(0), "/vercel/cmp/repo/data/abs.json")[3];
        assert_eq!(
            e.sentence("/vercel/cmp/repo/data/abs.json", &e.rel),
            "/vercel/cmp/repo/data/abs.json, an absolute path: it names data/abs.json if the \
             repository is mounted at /vercel/cmp/repo"
        );
        assert_eq!(
            seen_from_elsewhere(Some(0), "/x.json")[0].when,
            "the repository is mounted at /"
        );
        // Places no checkout is kept at under a name the code writes out.
        for spec in [
            "/tmp/ci/repo/data/abs.json",
            "/var/folders/ab/T/x/package.json",
            "/private/var/tmp/x",
            "/dev/shm/ci/repo/x.json",
            "/proc/self/status",
            "/sys/fs/cgroup/memory.max",
            "/dev/null",
            "/home/me/repo/.git/hooks/pre-commit",
        ] {
            assert_eq!(rels(Some(0), spec), [""; 0], "{spec}");
        }
        // Not those: an ordinary directory that is called like one.
        assert_eq!(rels(Some(0), "/tmpx/a"), ["tmpx/a", "a"]);
        // A directory of the system is no mount point; a checkout can be below one.
        assert_eq!(
            rels(Some(0), "/usr/src/app/a.json"),
            ["usr/src/app/a.json", "app/a.json", "a.json"]
        );
        assert_eq!(rels(Some(0), "/usr/lib/addon.node"), ["usr/lib/addon.node"]);
        assert_eq!(rels(Some(0), "/usr/bin/env"), ["usr/bin/env"]);
        assert_eq!(rels(Some(0), "/etc/passwd"), ["etc/passwd"]);
        assert_eq!(
            rels(Some(0), "/etc/app/package.json"),
            ["etc/app/package.json", "package.json"]
        );
        assert_eq!(
            rels(Some(0), "/home/me/repo/x.json"),
            ["home/me/repo/x.json", "repo/x.json", "x.json"]
        );
        assert_eq!(rels(Some(0), "/var/lib/"), ["var/lib"]);
        for spec in [
            "/app/x.json",
            "/workspace/x.json",
            "/vercel/path0/x.json",
            "/github/workspace/x.json",
            "/usr/src/app/x.json",
            "/home/runner/work/r/r/x.json",
            "/var/task/x.json",
            "/opt/app/x.json",
        ] {
            assert_eq!(
                rels(Some(0), spec).last().map(String::as_str),
                Some("x.json"),
                "{spec}"
            );
        }
        // A climb above the root: back inside when the directories it goes through are
        // called what it says. One reading, since the number of levels is known.
        assert_eq!(rels(Some(0), "../repo/data/x.json"), ["data/x.json"]);
        assert_eq!(rels(Some(1), "../../repo/data/x.json"), ["data/x.json"]);
        assert_eq!(
            rels(Some(1), "../../../a/repo/data/x.json"),
            ["data/x.json"]
        );
        assert_eq!(rels(Some(0), "a/../../repo/./data/x.json"), ["data/x.json"]);
        assert_eq!(rels(Some(0), "../sibling/x.js"), ["x.js"]);
        let e = &seen_from_elsewhere(Some(1), "../../../a/repo/data/x.json")[0];
        assert_eq!(
            e.sentence("../../../a/repo/data/x.json", &e.rel),
            "../../../a/repo/data/x.json, a path that leaves the repository: it names data/x.json \
             if the repository is checked out in a directory called a/repo"
        );
        // Not back inside: the directory of the checkout itself, or above it.
        assert_eq!(rels(Some(0), "../repo"), [""; 0]);
        assert_eq!(rels(Some(0), "../.."), [""; 0]);
        assert_eq!(rels(Some(2), "../../../../a/b"), [""; 0]);
        assert_eq!(rels(Some(0), "../.git/hooks/x"), [""; 0]);
        // A path that stays inside names what it says.
        for (depth, spec) in [
            (0, "data/x.json"),
            (1, "../data/x.json"),
            (2, "../../a/../b"),
            (0, "a/../b"),
        ] {
            assert_eq!(rels(Some(depth), spec), [""; 0], "{spec}");
        }
        // From a directory that is not in the repository only an absolute path counts.
        assert_eq!(rels(None, "../repo/data/x.json"), [""; 0]);
    }

    #[test]
    fn the_local_part_of_a_path_starts_where_it_leaves_the_way_to_the_base() {
        let fs = DirCache::new();
        // Not confined: the whole path.
        assert_eq!(fs.local_offset(Path::new("/a/node_modules/x")), 0);
        assert_eq!(fs.local(Path::new("/a/b")), Path::new("/a/b"));
        fs.confine(Path::new("/ci/node_modules/app/repo"), []);
        let local = |p: &str| fs.local(Path::new(p)).to_str().unwrap().to_owned();
        // Inside the base: relative to it.
        assert_eq!(local("/ci/node_modules/app/repo/src/a.js"), "src/a.js");
        assert_eq!(
            local("/ci/node_modules/app/repo/node_modules/x/i.js"),
            "node_modules/x/i.js"
        );
        assert_eq!(local("/ci/node_modules/app/repo"), "");
        // Outside: below the last directory shared with the base.
        assert_eq!(
            local("/ci/node_modules/app/store/x@1/node_modules/x"),
            "store/x@1/node_modules/x"
        );
        assert_eq!(local("/ci/node_modules/other/i.js"), "other/i.js");
        assert_eq!(local("/ci/node_modules/app"), "");
        assert_eq!(local("/ci/node_modules/application/x"), "application/x");
        assert_eq!(local("/ci/node_modules/app/repo2/x"), "repo2/x");
        // Nothing shared: the path as it is.
        assert_eq!(
            local("/usr/lib/node_modules/npm"),
            "/usr/lib/node_modules/npm"
        );
        assert_eq!(local("/"), "/");
        let p = Path::new("/ci/node_modules/app/repo/src/a.js");
        assert_eq!(
            &p.as_os_str().as_bytes()[fs.local_offset(p)..],
            b"/src/a.js"
        );
    }

    fn tree() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        fs::create_dir_all(r.join("src/Nested")).unwrap();
        fs::write(r.join("src/a.txt"), "a").unwrap();
        fs::write(r.join("src/Nested/B.txt"), "b").unwrap();
        fs::write(r.join("top.txt"), "t").unwrap();
        crate::os::symlink(r.join("src/a.txt"), r.join("link.txt")).unwrap();
        crate::os::symlink(r.join("src"), r.join("srclink")).unwrap();
        crate::os::symlink(r.join("nowhere"), r.join("dangling")).unwrap();
        t
    }

    /// A hidden directory is no entry of its parent, cannot be listed and holds
    /// nothing; a directory of the same name elsewhere is an ordinary one.
    #[test]
    fn a_hidden_directory_does_not_exist() {
        let t = tree();
        let r = fs::canonicalize(t.path()).unwrap();
        fs::create_dir_all(r.join("graph/cache")).unwrap();
        fs::write(r.join("graph/graph.json"), "{}").unwrap();
        fs::write(r.join("graph/cache/obs"), "x").unwrap();
        fs::create_dir_all(r.join("src/graph")).unwrap();
        fs::write(r.join("src/graph/kept.txt"), "k").unwrap();
        let c = DirCache::new();
        c.hide(&r.join("graph"));
        let names = |d: &Path| -> Vec<String> {
            c.list(d)
                .map(|l| {
                    l.iter()
                        .map(|e| e.name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        assert!(!names(&r).contains(&"graph".to_owned()), "{:?}", names(&r));
        assert!(names(&r).contains(&"src".to_owned()));
        assert!(c.list(&r.join("graph")).is_none());
        assert!(c.list(&r.join("graph/cache")).is_none());
        assert!(c.stat(&r.join("graph")).is_none());
        assert!(!c.is_dir(&r.join("graph/cache")));
        assert!(!c.is_file(&r.join("graph/graph.json")));
        assert!(!c.is_file(&r.join("graph/cache/obs")));
        let (files, _) = c.walk_files(&r, 100);
        assert!(files.iter().all(|f| !f.starts_with(r.join("graph"))));
        assert!(files.contains(&r.join("src/graph/kept.txt")));
        assert!(c.is_file(&r.join("src/graph/kept.txt")));
        // Without it, everything is there.
        let c = DirCache::new();
        assert!(c.is_file(&r.join("graph/cache/obs")));
        // By name, below a base: every directory called `graph` there, not a file of
        // that name, and nothing outside the base.
        fs::write(r.join("src/Nested/graph"), "a file").unwrap();
        let c = DirCache::new();
        c.hide_named(&r.join("src"), "graph");
        assert!(c.stat(&r.join("src/graph")).is_none());
        assert!(!c.is_file(&r.join("src/graph/kept.txt")));
        assert!(c.list(&r.join("src/graph")).is_none());
        assert!(c.is_file(&r.join("src/Nested/graph")));
        assert!(c.is_file(&r.join("src/a.txt")));
        assert!(c.is_file(&r.join("graph/cache/obs")));
    }

    #[test]
    fn normalize_paths() {
        let n = |s: &str| normalize(Path::new(s)).to_string_lossy().into_owned();
        assert_eq!(n("/a/./b/../c/"), "/a/c");
        assert_eq!(n("/../a"), "/a");
        assert_eq!(n("a/../../b"), "../b");
        assert_eq!(n("//a//b"), "/a/b");
        assert_eq!(n("/"), "/");
        assert_eq!(n("."), ".");
        assert_eq!(n("a/.."), ".");
        assert!(needs_normalize(Path::new("/a/./b")));
        assert!(needs_normalize(Path::new("/a/")));
        assert!(needs_normalize(Path::new("/a//b")));
        assert!(!needs_normalize(Path::new("/a/b")));
        assert!(!needs_normalize(Path::new("/")));
        assert_eq!(
            join_normalize(Path::new("/x/y"), "../z"),
            PathBuf::from("/x/z")
        );
        assert_eq!(
            join_normalize(Path::new("/x/y"), "/abs"),
            PathBuf::from("/abs")
        );
    }

    #[test]
    fn listing_and_stat() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let l = c.list(&r.join("src")).unwrap();
        let names: Vec<_> = l.iter().map(|e| e.name().to_owned()).collect();
        assert_eq!(names, ["Nested", "a.txt"]);
        assert!(c.is_file(&r.join("src/a.txt")));
        assert!(c.is_dir(&r.join("src/Nested")));
        assert!(!c.exists(&r.join("src/missing")));
        assert!(c.list(&r.join("src/a.txt")).is_none());
        assert!(c.list(&r.join("nope")).is_none());
        assert!(c.is_dir(Path::new("/")));
        assert!(c.is_file(&r.join("src/Nested/../a.txt")));
        // Reads are cached.
        let before = c.stats().dirs_read;
        assert!(c.is_file(&r.join("src/a.txt")));
        assert_eq!(c.stats().dirs_read, before);
    }

    #[test]
    fn symlinks() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let s = c.stat(&r.join("link.txt")).unwrap();
        assert_eq!(
            s,
            Stat {
                kind: FileKind::File,
                is_symlink: true
            }
        );
        let target = c.symlink_target(&r.join("link.txt")).unwrap();
        assert_eq!(&*target, fs::canonicalize(r.join("src/a.txt")).unwrap());
        assert!(c.is_dir(&r.join("srclink")));
        assert!(c.is_file(&r.join("srclink/a.txt")));
        assert!(!c.exists(&r.join("dangling")));
        assert!(c.symlink_target(&r.join("top.txt")).is_none());
    }

    #[test]
    fn case_mismatch() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        assert!(matches!(c.lookup(&r.join("src/a.txt")), Lookup::Found(_)));
        match c.lookup(&r.join("src/A.TXT")) {
            Lookup::CaseMismatch { actual, stat } => {
                assert_eq!(actual, r.join("src/a.txt"));
                assert_eq!(stat.kind, FileKind::File);
            }
            other => panic!("{other:?}"),
        }
        // Mismatch in a parent directory component.
        match c.lookup(&r.join("SRC/nested/b.txt")) {
            Lookup::CaseMismatch { actual, .. } => {
                assert_eq!(actual, r.join("src/Nested/B.txt"))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(c.lookup(&r.join("src/zzz")), Lookup::Missing);
        assert_eq!(c.lookup(&r.join("zzz/a.txt")), Lookup::Missing);
    }

    #[test]
    fn walk() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        let (files, truncated) = c.walk_files(r, 100);
        assert!(!truncated);
        let rel: Vec<_> = files
            .iter()
            .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rel,
            ["link.txt", "src/Nested/B.txt", "src/a.txt", "top.txt"]
        );
        let (files, truncated) = c.walk_files(r, 2);
        assert!(truncated);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn concurrent_reads_read_once() {
        let t = tree();
        let r = t.path().to_path_buf();
        let c = DirCache::new();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    for _ in 0..100 {
                        assert!(c.is_file(&r.join("src/a.txt")));
                    }
                });
            }
        });
        // Same number of reads as a single sequential lookup: each directory (the
        // parent and its ancestors, for exact-spelling checks) is read exactly
        // once.
        let sequential = DirCache::new();
        assert!(sequential.is_file(&r.join("src/a.txt")));
        assert_eq!(c.stats().dirs_read, sequential.stats().dirs_read);
    }

    /// A pnpm-style layout below a (possibly symlinked, e.g. macOS `/var`) temp
    /// root: `node_modules/foo -> .pnpm/foo@1.0.0/node_modules/foo`, and
    /// foo's dependency `.pnpm/foo@1.0.0/node_modules/bar ->
    /// ../../bar@2.0.0/node_modules/bar`.
    fn pnpm_tree() -> tempfile::TempDir {
        use crate::os::symlink;
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let foo_dir = r.join("node_modules/.pnpm/foo@1.0.0/node_modules/foo");
        let bar_dir = r.join("node_modules/.pnpm/bar@2.0.0/node_modules/bar");
        fs::create_dir_all(&foo_dir).unwrap();
        fs::create_dir_all(&bar_dir).unwrap();
        fs::write(foo_dir.join("index.js"), "").unwrap();
        fs::write(bar_dir.join("index.js"), "").unwrap();
        symlink(
            ".pnpm/foo@1.0.0/node_modules/foo",
            r.join("node_modules/foo"),
        )
        .unwrap();
        symlink(
            "../../bar@2.0.0/node_modules/bar",
            r.join("node_modules/.pnpm/foo@1.0.0/node_modules/bar"),
        )
        .unwrap();
        // A file link, a chain of links and a cycle.
        symlink("index.js", bar_dir.join("alias.js")).unwrap();
        symlink("foo", r.join("node_modules/chain")).unwrap();
        symlink("loop2", r.join("loop1")).unwrap();
        symlink("loop1", r.join("loop2")).unwrap();
        t
    }

    #[test]
    fn link_resolver_pnpm_layout() {
        let t = pnpm_tree();
        let r = normalize(t.path());
        let c = DirCache::new();
        let lr = LinkResolver::new(&r);
        let real_foo = r.join("node_modules/.pnpm/foo@1.0.0/node_modules/foo");
        let real_bar = r.join("node_modules/.pnpm/bar@2.0.0/node_modules/bar");

        // Non-links come back unchanged with no links.
        let rp = lr.resolve(&c, &real_foo.join("index.js"));
        assert_eq!(rp.path, real_foo.join("index.js"));
        assert!(rp.links.is_empty());

        // Symlinked parent directory.
        let rp = lr.resolve(&c, &r.join("node_modules/foo/index.js"));
        assert_eq!(rp.path, real_foo.join("index.js"));
        assert_eq!(
            rp.links,
            [(r.join("node_modules/foo"), real_foo.clone())],
            "the link is reported with its target"
        );

        // A dependency reached from the real location of foo.
        let rp = lr.resolve(
            &c,
            &r.join("node_modules/.pnpm/foo@1.0.0/node_modules/bar/alias.js"),
        );
        assert_eq!(rp.path, real_bar.join("index.js"));
        assert_eq!(
            rp.links,
            [
                (
                    r.join("node_modules/.pnpm/foo@1.0.0/node_modules/bar"),
                    real_bar.clone()
                ),
                (real_bar.join("alias.js"), real_bar.join("index.js")),
            ]
        );

        // Two links in the path: foo, then bar inside foo's real node_modules.
        let rp = lr.resolve(&c, &r.join("node_modules/foo/../bar/index.js"));
        // Lexical normalization happens first (Node's path.resolve semantics).
        assert_eq!(rp.path, r.join("node_modules/bar/index.js"));

        // A chain: chain -> foo (a link itself) -> real foo.
        let rp = lr.resolve(&c, &r.join("node_modules/chain/index.js"));
        assert_eq!(rp.path, real_foo.join("index.js"));
        assert_eq!(rp.links.len(), 2);
        assert_eq!(
            rp.links[0],
            (r.join("node_modules/chain"), real_foo.clone())
        );
        assert_eq!(rp.links[1], (r.join("node_modules/foo"), real_foo.clone()));

        // A cycle terminates.
        let rp = lr.resolve(&c, &r.join("loop1/x"));
        assert!(rp.links.len() as u32 >= MAX_LINK_DEPTH);

        // readlink only ran for symlink entries: foo, bar, alias, chain, loop1, loop2.
        assert_eq!(c.stats().links_resolved, 6);
    }

    #[test]
    fn link_resolver_keeps_base_ancestors() {
        use crate::os::symlink;
        let t = tempfile::tempdir().unwrap();
        let real = normalize(&fs::canonicalize(t.path()).unwrap());
        fs::create_dir_all(real.join("target")).unwrap();
        fs::write(real.join("target/a.js"), "").unwrap();
        symlink(real.join("target"), real.join("base")).unwrap();
        let c = DirCache::new();
        // The base itself is a symlink: kept as spelled.
        let base = real.join("base");
        let rp = LinkResolver::new(&base).resolve(&c, &base.join("a.js"));
        assert_eq!(rp.path, base.join("a.js"));
        assert!(rp.links.is_empty());
        // With nothing kept, it is resolved.
        let rp = c.canonicalize(&base.join("a.js"));
        assert_eq!(rp.path, real.join("target/a.js"));
        assert_eq!(rp.links, [(base, real.join("target"))]);
    }

    #[test]
    fn recording_hooks() {
        let t = tree();
        let r = normalize(t.path());
        let plain = DirCache::new();
        let f = crate::record::start();
        assert!(!plain.exists(&r.join("missing")));
        assert!(f.finish().is_empty(), "a plain cache never records");

        let c = DirCache::with_recording();
        // Outside a frame nothing is kept.
        assert!(!c.exists(&r.join("missing")));
        let f = crate::record::start();
        assert!(c.is_file(&r.join("src/a.txt")));
        assert!(!c.is_file(&r.join("src/b.txt")));
        assert!(c.read_cached(&r.join("top.txt")).is_ok());
        assert!(c.read_cached(&r.join("nope.json")).is_err());
        let _ = c.list(&r.join("src"));
        let _ = c.walk_files(&r.join("src/Nested"), 10);
        let o = f.finish();
        assert_eq!(o.missing, [r.join("src/b.txt"), r.join("nope.json")]);
        assert_eq!(o.reads, [r.join("top.txt")]);
        assert_eq!(
            o.listings,
            [
                (r.join("src"), "**".to_string()),
                (r.join("src/Nested"), "**".to_string())
            ]
        );
    }

    #[test]
    fn read_cached_reads_once() {
        let t = tree();
        let r = t.path();
        let c = DirCache::new();
        assert_eq!(&*c.read_cached(&r.join("src/a.txt")).unwrap(), b"a");
        fs::write(r.join("src/a.txt"), "changed").unwrap();
        assert_eq!(&*c.read_cached(&r.join("src/a.txt")).unwrap(), b"a");
        let err = c.read_cached(&r.join("src/missing.json")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        // Wrong case is a miss even on case-insensitive filesystems.
        assert!(c.read_cached(&r.join("src/A.txt")).is_err());
        c.clear();
        assert_eq!(&*c.read_cached(&r.join("src/a.txt")).unwrap(), b"changed");
    }
}
