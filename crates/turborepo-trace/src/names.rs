//! Named paths in `affected` and `uncertainty`: the bound for reads whose path
//! cannot be evaluated (`turborepo_trace_graph::names`).
//!
//! The names of a source file are a fact of the file, recorded with its
//! extraction when it is traced (`turborepo_trace_core::names`, read from the
//! shards in place). A query only asks which of them name a changed path:
//! [`build`] makes the index for the changed paths of one query (a changed path
//! has a few dozen suffixes; every name of the graph is looked up in that small
//! set), so nothing is scanned or indexed per repository at query time.
//!
//! Data files (JSON, YAML, TOML) that source files read through an edge have
//! names too. Their contents are not part of the graph, so their names are kept
//! in `<graph>/cache/names-data`, validated per file by size and modification
//! time, and only rescanned when a file changed. The cache is keyed by path:
//! after the graph changed, only the list of data files is computed again.
//!
//! The manifests that tools read (`package.json`, `tsconfig.json`,
//! `turbo.json`, lockfiles, CI workflows) contribute no names: what tools do
//! with their strings is modeled by the graph's manifests and tasks. Runner
//! configuration needs no rule here: a test reaches its runner's config through
//! an edge that is not code loading (`RefFlags::CONFIG`), so the config's
//! strings never pair with the test's reads.
//!
//! `TURBO_TRACE_DEBUG_NAMES=0` turns the layer off (A/B measurements): the
//! reads are open again.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use turborepo_trace_core::names::{self as facts, Encoded};
use turborepo_trace_graph::{
    AffectedOptions, AffectedReport, Change, EdgeAttr, Engine, GraphView, NameIndex, NodeId,
    NodeSet, RefFlags, RefKind,
};
use turborepo_trace_store::Graph;

/// Data files larger than this are not scanned.
const MAX_DATA_BYTES: u64 = 256 << 10;

/// Whether the names layer is on.
pub(crate) fn enabled() -> bool {
    turborepo_trace_core::debug::var_os("NAMES").is_none_or(|v| v != "0")
}

/// The names that matter for one query, and what finding them cost.
pub(crate) struct Layer {
    pub index: NameIndex,
    /// Source files with a names fact.
    pub files: usize,
    /// Names looked at.
    pub names: usize,
    /// Data files with names (read through an edge by a source file).
    pub data_files: usize,
    /// Data files scanned in this run (new or changed since the cache was
    /// written).
    pub data_scanned: usize,
    pub elapsed: Duration,
}

/// Whether `path` is a manifest or configuration that tools read (the package
/// manager, the compiler, the task runner, CI): what they do with its strings
/// is modeled by the graph's manifests and tasks.
pub(crate) fn is_tool_manifest(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.starts_with(".github/")
        || matches!(
            name,
            "package.json"
                | "package-lock.json"
                | "pnpm-lock.yaml"
                | "pnpm-workspace.yaml"
                | "turbo.json"
                | "nx.json"
                | "project.json"
                | "lerna.json"
                | ".eslintrc.json"
                | ".prettierrc.json"
        )
        || ((name.starts_with("tsconfig") || name.starts_with("jsconfig"))
            && name.ends_with(".json"))
}

/// Whether `path` is a data file whose strings can be names, and whether it is
/// JSON.
fn data_kind(path: &str) -> Option<bool> {
    if is_tool_manifest(path) {
        return None;
    }
    match path.rsplit_once('.').map(|x| x.1) {
        Some("json" | "json5" | "jsonc") => Some(true),
        Some("yaml" | "yml" | "toml") => Some(false),
        _ => None,
    }
}

/// The `k`-th ancestor of the directory `dir` (`""` is the root).
fn ancestor(dir: &str, k: u32) -> &str {
    let mut d = dir;
    for _ in 0..k {
        if d.is_empty() {
            break;
        }
        d = d.rsplit_once('/').map_or("", |x| x.0);
    }
    d
}

/// What a changed path can be named by: every suffix of the path and of each of
/// its directories (and of the file without its extensions, for two or more
/// segments), and every directory above it.
struct Keys<'a> {
    shapes: HashSet<&'a str>,
    /// Every suffix of a changed path that starts at a segment boundary, in
    /// order: what a shape whose last segment can be continued is looked up
    /// in ([`Keys::continued`]).
    suffixes: Vec<&'a str>,
    dirs: HashSet<&'a str>,
    /// The changed paths by directory: `(file name, path)`.
    files: std::collections::HashMap<&'a str, Vec<(&'a str, &'a str)>>,
}

impl<'a> Keys<'a> {
    fn of(changes: &'a [Change]) -> Self {
        let mut shapes = HashSet::new();
        let mut suffixes = Vec::new();
        let mut dirs = HashSet::new();
        let mut files: std::collections::HashMap<&str, Vec<(&str, &str)>> = Default::default();
        dirs.insert("");
        for c in changes {
            let path = c.path.trim_matches('/');
            if path.is_empty() {
                continue;
            }
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
            files.entry(dir).or_default().push((name, path));
            let mut starts = vec![0usize];
            let mut ends = Vec::new();
            for (i, b) in path.bytes().enumerate() {
                if b == b'/' {
                    ends.push(i);
                    starts.push(i + 1);
                }
            }
            ends.push(path.len());
            suffixes.extend(starts.iter().map(|&s| &path[s..]));
            let n = starts.len();
            for end in 0..n {
                if end + 1 < n {
                    dirs.insert(&path[..ends[end]]);
                }
                for &s in &starts[..=end] {
                    shapes.insert(&path[s..ends[end]]);
                }
            }
            let last = &path[starts[n - 1]..];
            for (i, b) in last.bytes().enumerate() {
                if b == b'.' && i > 0 {
                    for &s in &starts[..n - 1] {
                        shapes.insert(&path[s..starts[n - 1] + i]);
                    }
                }
            }
        }
        suffixes.sort_unstable();
        suffixes.dedup();
        Keys {
            shapes,
            suffixes,
            dirs,
            files,
        }
    }

    /// Whether `shape`, continued within its last segment, can name a changed
    /// path: a suffix of a changed path at a segment boundary starts with it
    /// (`turborepo_trace_graph::names::shape_continues`). Every shape that
    /// names a changed path does.
    fn continued(&self, shape: &str) -> bool {
        let i = self.suffixes.partition_point(|s| *s < shape);
        self.suffixes.get(i).is_some_and(|s| s.starts_with(shape))
    }
}

/// A region of a source file whose strings are not names: a table whose values
/// another analysis resolved exactly (the modules a key selects are edges of
/// the callers), so its strings must not select every reader of the file again.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClosedSpan {
    pub node: NodeId,
    /// Byte offsets: start (inclusive) and end (exclusive).
    pub start: u32,
    pub end: u32,
}

/// What one file's fact contributes to a query.
struct FileNames {
    node: NodeId,
    /// Names looked at.
    seen: usize,
    /// Shapes that can name a changed path, with their rests and lines, and
    /// whether the string climbs
    /// (`turborepo_trace_core::names::Name::climbs`).
    shapes: Vec<(String, Option<String>, u32, bool)>,
    overflow: bool,
    /// The directories the file's location names, when a changed path is below
    /// one: the directory, the line, and whether the entry is conditional
    /// (`turborepo_trace_core::names::Anchored::conditional`).
    locations: Vec<(String, u32, bool)>,
    /// Changed paths named exactly by a pattern in a directory built on the
    /// file's location.
    paths: Vec<(String, u32, bool)>,
    /// Directories built on the working directory: relative part, pattern,
    /// line, conditional.
    cwd_dirs: Vec<(String, String, u32, bool)>,
    /// Paths that a string climbs to from the working directory (leading `..`,
    /// then the shape), with the line
    /// (`turborepo_trace_core::names::Name::climbs_from_cwd`): made by
    /// [`settle`] from `cwd_names`.
    cwd_climbs: Vec<(String, u32)>,
    /// The names whose strings climb from the working directory, when their
    /// shape continued can name a changed path: the shape, the line, how far
    /// they climb, and their index in `shapes` when they name a changed path
    /// as a shape. [`settle`] decides where they go.
    cwd_names: Vec<(String, u32, u32, Option<usize>)>,
    /// Go names whose strings climb from where nothing says, when their shape
    /// continued can name a changed path: the shape and the line. They name
    /// their shape, continued, anywhere
    /// (`turborepo_trace_graph::NameIndex::add_prefix`).
    prefixes: Vec<(String, u32)>,
    /// The file can move the base of its package's relative paths
    /// (`turborepo_trace_core::names::Names::moves_base`).
    moves_base: bool,
    /// The file changes the working directory
    /// (`turborepo_trace_core::names::Names::chdir`).
    chdir: bool,
    cwd: Option<u32>,
    test_path: Option<u32>,
    ups: u32,
}

/// The names of the fact `d` of the file `n` (at `path`) that can name a
/// changed path. `None`: nothing of it matters for this query. `climb_cwd`: a
/// string that climbs from the working directory names its rest at the
/// directories it climbs to, and its shape only inside the package
/// (`turborepo_trace_graph::names`, "Strings that climb from the working
/// directory"); otherwise its shape anywhere, as a string that climbs from
/// where nothing says.
fn file_names(
    keys: &Keys<'_>,
    n: NodeId,
    path: &str,
    d: &Encoded<'_>,
    data: bool,
    closed: &[ClosedSpan],
    climb_cwd: bool,
) -> FileNames {
    let mut out = FileNames {
        node: n,
        seen: 0,
        shapes: Vec::new(),
        overflow: d.overflow,
        locations: Vec::new(),
        paths: Vec::new(),
        cwd_dirs: Vec::new(),
        cwd_climbs: Vec::new(),
        cwd_names: Vec::new(),
        prefixes: Vec::new(),
        moves_base: d.moves_base,
        chdir: d.chdir,
        cwd: None,
        test_path: None,
        ups: 0,
    };
    // A Go string that climbs is the start of a path that code can complete
    // (`"../../shared/app." + env + ".yaml"`, `base + ".json"`): its last segment
    // can be continued.
    let go = !data && path.ends_with(".go");
    for (line, shape, rest) in d.shapes() {
        out.seen += 1;
        let keyed = keys.shapes.contains(shape);
        let from_cwd = facts::climbs_from_cwd(line);
        let marked = facts::climbs(line);
        let continued = go && (marked || from_cwd > 0) && keys.continued(shape);
        if !keyed && !continued {
            continue;
        }
        // A parent step in the file (`..`, `dirname`) can cut the spelled path back to
        // the shape: the rest then says nothing.
        let rest = rest.filter(|_| d.ups == 0 || data);
        let climbs = marked || (from_cwd > 0 && !climb_cwd);
        let (line, span) = facts::line_and_span(line);
        // Every string with this name lies inside one closed region: not a name.
        if let Some((first, last)) = span
            && closed.iter().any(|c| c.start <= first && last < c.end)
        {
            continue;
        }
        if go && climbs {
            // Its shape, continued, anywhere (that covers what the shape names).
            out.prefixes.push((shape.to_owned(), line));
            continue;
        }
        let at = keyed.then(|| {
            out.shapes
                .push((shape.to_owned(), rest.map(str::to_owned), line, climbs));
            out.shapes.len() - 1
        });
        // Where it climbs to: [`settle`] decides with the other files of the directory.
        if go && from_cwd > 0 && climb_cwd {
            out.cwd_names.push((shape.to_owned(), line, from_cwd, at));
        }
    }
    // A data file has no location of its own.
    if !data {
        out.ups = d.ups;
        out.cwd = d.cwd;
        out.test_path = d.test_path;
        if let Some(line) = d.anchor {
            let ups = if path.ends_with(".go") {
                crate::gonames::location_ups(d.ups)
            } else {
                d.ups
            };
            let dir = ancestor(path.rsplit_once('/').map_or("", |x| x.0), ups);
            if keys.dirs.contains(dir) {
                out.locations.push((dir.to_owned(), line, false));
            }
        }
        // Where the file's location and the working directory go, when they were
        // followed.
        for a in d.anchored() {
            if a.cwd {
                out.cwd_dirs.push((
                    a.rel.into_string(),
                    a.pattern.into_string(),
                    a.line,
                    a.conditional,
                ));
                continue;
            }
            let base =
                turborepo_trace_graph::join_rel(path.rsplit_once('/').map_or("", |x| x.0), &a.rel);
            if &*a.pattern == facts::EVERYTHING_BELOW {
                // The parent steps elsewhere in the file can still be applied to it.
                let dir = ancestor(&base, d.ups);
                if keys.dirs.contains(dir) {
                    out.locations.push((dir.to_owned(), a.line, a.conditional));
                }
                // The path itself, when it is a file.
                let (parent, name) = dir.rsplit_once('/').unwrap_or(("", dir));
                if keys
                    .files
                    .get(parent)
                    .is_some_and(|fs| fs.iter().any(|f| f.0 == name))
                {
                    out.paths.push((dir.to_owned(), a.line, a.conditional));
                }
            } else if let Some(changed) = keys.files.get(base.as_str()) {
                for (name, p) in changed {
                    if turborepo_trace_graph::glob_name(&a.pattern, name) {
                        out.paths.push(((*p).to_owned(), a.line, a.conditional));
                    }
                }
            }
        }
    }
    out
}

/// Where the strings of the Go files that climb from the working directory go
/// (`turborepo_trace_core::names::Name::climbs_from_cwd`), decided per
/// directory (a Go package):
///
/// - where some Go file of the directory can move the base
///   (`turborepo_trace_core::names::Names::moves_base`: a `Chdir`, a string
///   that only climbs), they name their shape, continued, anywhere, as a string
///   that climbs from where nothing says;
/// - where a Go file changes the working directory
///   (`turborepo_trace_core::names::Names::chdir`), every name of the
///   directory's Go files names anywhere too: after the change a relative path
///   is taken from elsewhere, a literal one too;
/// - where a symbolic link is below the working directory, or on the way from a
///   directory a string climbs to down to its place, the path the string spells
///   is not where it leads: they name their shape, continued, anywhere, as
///   before the anchoring ([`has_link_below`]);
/// - otherwise they name their rest at the directories they climb to, and their
///   shape below the working directory (continued, both). How far a string
///   climbs is raised by the parent steps of the directory's other Go files
///   (`turborepo_trace_core::names::Names::ups`): a string or a `filepath.Dir`
///   call of another file can be its base (`const Fixtures =
///   "../../../fixtures"` next to `filepath.Join(Fixtures,
///   "../shared/a.json")`).
fn settle(g: &Graph, root: &Path, files: &mut [FileNames]) {
    let dir = |n: NodeId| {
        let p = g.path(n);
        p.ends_with(".go")
            .then(|| p.rsplit_once('/').map_or("", |x| x.0))
    };
    let moved: HashSet<&str> = files
        .iter()
        .filter(|f| f.moves_base)
        .filter_map(|f| dir(f.node))
        .collect();
    let changed: HashSet<&str> = files
        .iter()
        .filter(|f| f.chdir)
        .filter_map(|f| dir(f.node))
        .collect();
    // Per directory: whether a symbolic link is below it.
    let mut links: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    // Per path: whether it is a symbolic link.
    let mut probes: std::collections::HashMap<PathBuf, bool> = std::collections::HashMap::new();
    // The parent steps of every directory's Go files.
    let mut ups: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for f in files.iter() {
        if let Some(d) = dir(f.node) {
            let u = ups.entry(d).or_default();
            *u = u.saturating_add(f.ups);
        }
    }
    for f in files.iter_mut() {
        let Some(d) = dir(f.node) else { continue };
        if changed.contains(d) {
            for s in &mut f.shapes {
                s.3 = true;
            }
        }
        let names = std::mem::take(&mut f.cwd_names);
        let others = ups.get(d).copied().unwrap_or(0).saturating_sub(f.ups);
        let k_of = |k: u32| k.saturating_add(others).min(facts::MAX_UPS);
        // A link below the working directory, or at a place: the spelled path leads
        // elsewhere.
        let linked = !names.is_empty()
            && (*links
                .entry(d.to_owned())
                .or_insert_with(|| has_link_below(root, d))
                || names.iter().any(|(shape, _, k, _)| {
                    (1..=k_of(*k)).any(|j| has_link_on(root, ancestor(d, j), shape, &mut probes))
                }));
        if moved.contains(d) || linked {
            for (shape, line, _, at) in names {
                if let Some(i) = at {
                    f.shapes[i].3 = true;
                }
                f.prefixes.push((shape, line));
            }
            continue;
        }
        for (shape, line, k, _) in names {
            f.cwd_climbs.extend(climb_places(&shape, line, k_of(k)));
        }
    }
}

/// Most entries [`has_link_below`] looks at; a larger tree is taken to have a
/// link.
const MAX_LINK_SCAN: usize = 20_000;

/// The paths [`has_link_on`] asks the file system about in one query.
const MAX_LINK_PROBES: usize = 100_000;

/// Whether a symbolic link is at or below the directory `dir` (repository
/// relative) of the tree at `root`: a path spelled below it can lead anywhere.
/// Links are not followed; a tree of more than [`MAX_LINK_SCAN`] entries, or
/// one that cannot be read, counts as having one.
fn has_link_below(root: &Path, dir: &str) -> bool {
    let start = root.join(dir);
    match std::fs::symlink_metadata(&start) {
        Ok(m) if m.file_type().is_symlink() => return true,
        Ok(m) if m.is_dir() => {}
        _ => return false,
    }
    let mut stack = vec![start];
    let mut seen = 0usize;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            return true;
        };
        for e in rd {
            let Ok(e) = e else { return true };
            seen += 1;
            if seen > MAX_LINK_SCAN {
                return true;
            }
            let Ok(t) = e.file_type() else { return true };
            if t.is_symlink() {
                return true;
            }
            if t.is_dir() && e.file_name() != ".git" {
                stack.push(e.path());
            }
        }
    }
    false
}

/// Whether a symbolic link is on the way from the directory `base`
/// (repository relative) down the segments of `shape`: the place of a string
/// that climbs to `base` leads elsewhere.
///
/// `probes` holds what a query has asked the file system so far. Past
/// [`MAX_LINK_PROBES`] paths a place counts as linked, so its strings stay
/// unanchored.
fn has_link_on(
    root: &Path,
    base: &str,
    shape: &str,
    probes: &mut std::collections::HashMap<PathBuf, bool>,
) -> bool {
    let mut p = root.join(base);
    shape.split('/').filter(|s| !s.is_empty()).any(|seg| {
        p.push(seg);
        if let Some(&link) = probes.get(&p) {
            return link;
        }
        if probes.len() >= MAX_LINK_PROBES {
            return true;
        }
        let link = std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
        probes.insert(p.clone(), link);
        link
    })
}

/// The places of a string of shape `shape` at `line` that climbs `k`
/// directories from the working directory: below the working directory,
/// whatever package that is (a base below it in a nested package), and at the
/// directories it climbs to.
fn climb_places(shape: &str, line: u32, k: u32) -> Vec<(String, u32)> {
    std::iter::once((format!("**/{shape}"), line))
        .chain((1..=k).map(|j| (format!("{}{shape}", "../".repeat(j as usize)), line)))
        .collect()
}

/// Whether the environment registry owns the record `u` (it resolved what it
/// could; the rest is the environment's, which the default policy reports and
/// does not count).
fn env_owned(u: &turborepo_trace_graph::Uncertainty<'_>) -> bool {
    use turborepo_trace_graph::{Origin, UncertaintyKind};
    u.kind == UncertaintyKind::EnvDependent
        || (u.kind == UncertaintyKind::ComputedSpecifier
            && matches!(u.origin, Origin::Env | Origin::Argv))
}

/// Adds the names of `f`. `live(line)`: the file still has a record at `line`
/// that the environment registry does not own (for conditional anchored
/// entries).
fn add_names(index: &mut NameIndex, f: &FileNames, data: bool, live: &dyn Fn(u32) -> bool) {
    if f.overflow {
        index.add_everything(f.node);
    }
    for (shape, rest, line, climbs) in &f.shapes {
        index.add_name(f.node, shape, rest.as_deref(), *line, *climbs);
    }
    for (shape, line) in &f.prefixes {
        index.add_prefix(f.node, shape, *line, true);
    }
    if data {
        index.mark_data(f.node);
        return;
    }
    index.set_ups(f.node, f.ups);
    for (dir, line, conditional) in &f.locations {
        if !*conditional || live(*line) {
            index.add_location(f.node, dir, *line);
        }
    }
    for (path, line, conditional) in &f.paths {
        if !*conditional || live(*line) {
            index.add_path(f.node, path, *line);
        }
    }
    for (rel, pattern, line, conditional) in &f.cwd_dirs {
        if !*conditional || live(*line) {
            index.add_cwd_dir(f.node, rel, pattern, *line);
        }
    }
    for (rel, line) in &f.cwd_climbs {
        index.add_cwd_climb(f.node, rel, *line);
    }
    if let Some(line) = f.cwd {
        index.add_cwd_user(f.node, line);
    }
    if let Some(line) = f.test_path {
        index.add_test_path_user(f.node, line);
    }
}

/// One data file of the cache.
struct DataEntry {
    node: u32,
    len: u64,
    mtime: u128,
    fact: String,
}

const DATA_MAGIC: &[u8; 8] = b"TTNAMES3";

/// What the list of data files is a function of: the manifest's shards.
fn graph_key(g: &Graph) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    g.node_count().hash(&mut h);
    for s in &g.manifest().shards {
        s.key.hash(&mut h);
    }
    h.finish()
}

/// The cache: the graph it lists the data files of, and the entries with their
/// paths.
fn read_cache(file: &Path) -> Option<(u64, Vec<(String, DataEntry)>)> {
    let bytes = std::fs::read(file).ok()?;
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = bytes.get(at..at + n)?;
        at += n;
        Some(s)
    };
    if take(8)? != DATA_MAGIC {
        return None;
    }
    let key = u64::from_le_bytes(take(8)?.try_into().ok()?);
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let node = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let len = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let mtime = u128::from_le_bytes(take(16)?.try_into().ok()?);
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let path = std::str::from_utf8(take(n)?).ok()?.to_owned();
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        let fact = std::str::from_utf8(take(n)?).ok()?.to_owned();
        out.push((
            path,
            DataEntry {
                node,
                len,
                mtime,
                fact,
            },
        ));
    }
    Some((key, out))
}

fn write_cache(g: &Graph, file: &Path, key: u64, entries: &[DataEntry]) {
    let mut out = Vec::new();
    out.extend_from_slice(DATA_MAGIC);
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.node.to_le_bytes());
        out.extend_from_slice(&e.len.to_le_bytes());
        out.extend_from_slice(&e.mtime.to_le_bytes());
        let path = g.path(NodeId(e.node));
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&(e.fact.len() as u32).to_le_bytes());
        out.extend_from_slice(e.fact.as_bytes());
    }
    // Best effort: without the cache the next run scans again.
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn stat(path: &Path) -> Option<(u64, u128)> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((m.len(), mtime))
}

/// Edges through which a source file reads a data file's contents.
fn reads_data(a: EdgeAttr) -> bool {
    !a.flags().contains(RefFlags::TYPE_ONLY) && a.kind() != RefKind::Manifest
}

/// The data files that a first-party source file reads through an edge.
fn data_nodes<V: GraphView>(g: &Graph, engine: &Engine<'_, V>) -> Vec<u32> {
    let rev = engine.reverse();
    let mut out = Vec::new();
    for i in 0..g.node_count() {
        let n = NodeId(i);
        if g.lang(n).is_some() || g.is_external(n) || data_kind(g.path(n)).is_none() {
            continue;
        }
        let (srcs, attrs) = rev.neighbors(n);
        if srcs.iter().zip(attrs).any(|(&s, &a)| {
            reads_data(a) && g.lang(NodeId(s)).is_some_and(|l| !l.starts_with("task"))
        }) {
            out.push(i);
        }
    }
    out
}

/// The names of the data files, from the cache where a file is unchanged.
fn data_facts<V: GraphView>(
    g: &Graph,
    engine: &Engine<'_, V>,
    root: &Path,
    cache: &Path,
) -> (Vec<DataEntry>, usize) {
    let key = graph_key(g);
    let cached = read_cache(cache);
    let fresh_list = cached.as_ref().is_none_or(|c| c.0 != key);
    let mut entries: Vec<DataEntry> = match cached {
        // The same graph: the same data files, with the same node ids.
        Some((k, list)) if k == key => list.into_iter().map(|(_, e)| e).collect(),
        // Another graph (or no cache): the data files of this one, with what the cache
        // knows about each path.
        other => {
            let mut known: std::collections::HashMap<String, DataEntry> = other
                .map(|(_, list)| list.into_iter().collect())
                .unwrap_or_default();
            data_nodes(g, engine)
                .into_iter()
                .map(|node| match known.remove(g.path(NodeId(node))) {
                    Some(e) => DataEntry { node, ..e },
                    None => DataEntry {
                        node,
                        len: u64::MAX,
                        mtime: 0,
                        fact: String::new(),
                    },
                })
                .collect()
        }
    };
    // Size and modification time of every file, in parallel: the only per-run cost.
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let chunk = entries.len().div_ceil(workers).max(64);
    let stats: Vec<Option<(u64, u128)>> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|e| stat(&root.join(g.path(NodeId(e.node)))))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    });
    let mut scanned = 0;
    for (e, now) in entries.iter_mut().zip(stats) {
        let path = g.path(NodeId(e.node));
        let abs = root.join(path);
        if now == Some((e.len, e.mtime)) {
            continue;
        }
        scanned += 1;
        let (len, mtime) = now.unwrap_or((u64::MAX, 1));
        e.len = len;
        e.mtime = mtime;
        e.fact = if len > MAX_DATA_BYTES {
            // A large data file is not scanned: a documented gap.
            String::new()
        } else {
            match std::fs::read(&abs) {
                Ok(src) => {
                    let names = if data_kind(path) == Some(true) {
                        facts::scan_json(&src)
                    } else {
                        facts::scan_text(&src)
                    };
                    if names.is_empty() {
                        String::new()
                    } else {
                        names.encode()
                    }
                }
                Err(_) => String::new(),
            }
        };
    }
    if scanned > 0 || fresh_list {
        write_cache(g, cache, key, &entries);
    }
    (entries, scanned)
}

/// The names of the graph that can name one of `changes`. `cache_dir` is
/// `<graph>/cache`; `closed` are regions whose strings are not names
/// ([`ClosedSpan`]).
pub(crate) fn build<V: GraphView>(
    g: &Graph,
    engine: &Engine<'_, V>,
    root: &Path,
    cache_dir: &Path,
    changes: &[Change],
    closed: &[ClosedSpan],
    climb_cwd: bool,
) -> Layer {
    let t = Instant::now();
    let keys = Keys::of(changes);
    let mut index = NameIndex::new();
    let mut per_file = g.map_facts(facts::TAG, &|n, fact| {
        facts::decode(fact).map(|d| {
            let mine: Vec<ClosedSpan> = closed.iter().filter(|c| c.node == n).copied().collect();
            file_names(&keys, n, g.path(n), &d, false, &mine, climb_cwd)
        })
    });
    settle(g, root, &mut per_file);
    let files = per_file.len();
    let mut names = 0;
    for f in &per_file {
        names += f.seen;
        let node = f.node;
        add_names(&mut index, f, false, &|line| {
            engine
                .graph()
                .uncertainties(node)
                .any(|u| u.line == line && !env_owned(&u))
        });
    }
    let t_facts = t.elapsed();
    let cache: PathBuf = cache_dir.join(turborepo_trace_store::cache_file::NAMES_DATA);
    let (data, data_scanned) = data_facts(g, engine, root, &cache);
    let mut data_files = 0;
    for e in &data {
        if let Some(d) = facts::decode(&e.fact) {
            data_files += 1;
            let n = NodeId(e.node);
            let f = file_names(&keys, n, g.path(n), &d, true, &[], climb_cwd);
            names += f.seen;
            add_names(&mut index, &f, true, &|_| true);
        }
    }
    if turborepo_trace_core::debug::var_os("NAMES_TIMING").is_some() {
        eprintln!(
            "names facts: source files {:.1} ms, data files {:.1} ms",
            t_facts.as_secs_f64() * 1e3,
            (t.elapsed() - t_facts).as_secs_f64() * 1e3
        );
    }
    Layer {
        index,
        files,
        names,
        data_files,
        data_scanned,
        elapsed: t.elapsed(),
    }
}

/// The directories each entry runs in: the test runner's working directories
/// when the graph knows them, else the entry's package root.
fn entry_cwds(g: &Graph) -> impl Fn(NodeId) -> Vec<String> + '_ {
    let known = g.tests_with_cwds();
    move |n| match known.get(&n.0) {
        Some(dirs) => dirs.iter().map(|d| (*d).to_owned()).collect(),
        // `go test` runs a package's tests in the package directory.
        None if crate::gonames::test_cwd(g.path(n)).is_some() => {
            vec![
                crate::gonames::test_cwd(g.path(n))
                    .unwrap_or_default()
                    .to_owned(),
            ]
        }
        None => vec![
            g.package_info(g.package(n))
                .root
                .trim_matches('/')
                .to_owned(),
        ],
    }
}

/// The directories the commands of a build task run in, when that is known
/// (`turborepo_trace_graph::names`, "Who runs a process"): a package script
/// (run by npm, pnpm, yarn or bun, through turbo too) runs in the directory of
/// its package, the one in the task's id. `None` for a task of another source,
/// for a script with a command line that changes directory or hands one to its
/// tool, which the stored task does not resolve, and for a script that starts a
/// shell or a script file ([`starts_a_script`]): what that runs, it runs where
/// it chooses.
fn task_dirs(g: &Graph) -> impl Fn(NodeId) -> Option<Vec<String>> + '_ {
    move |n| {
        let t = g.task(n)?;
        let package_script = !t.sources.is_empty()
            && t.sources
                .iter()
                .all(|s| matches!(s.as_str(), "npm" | "turbo"));
        if !package_script
            || t.lines.iter().any(|l| changes_directory(l.as_str()))
            || t.commands
                .iter()
                .any(|c| starts_a_script(c.program.as_ref().map(|p| p.as_str())))
        {
            return None;
        }
        let dir = t.id.as_str().strip_prefix("task:")?.rsplit_once('#')?.0;
        Some(vec![if dir == "." {
            String::new()
        } else {
            dir.trim_matches('/').to_owned()
        }])
    }
}

/// Whether a command line can run a program somewhere else than where it
/// starts: `cd`, `pushd`, or a tool's option that names a directory to work in.
fn changes_directory(line: &str) -> bool {
    line.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
        .any(|w| {
            matches!(
                w,
                "cd" | "pushd" | "popd" | "-C" | "--cwd" | "--dir" | "--prefix" | "--cd"
            ) || ["--cwd=", "--dir=", "--prefix=", "--cd="]
                .iter()
                .any(|p| w.starts_with(p))
        })
}

/// Whether the program of a command (after wrapper stripping; `None`: not
/// recognized) is a shell or a script file run by its path (`bash gen.sh`,
/// `./scripts/build`, `x.sh`): it can change directory before it starts the
/// program that reads.
fn starts_a_script(program: Option<&str>) -> bool {
    match program {
        None => true,
        Some(p) => {
            matches!(
                p,
                "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish" | "source" | "."
            ) || p.contains('/')
                || p.ends_with(".sh")
                || p.ends_with(".bash")
        }
    }
}

/// Adds to `report` the entries that `changes` affect through names. Returns
/// how many were added (entries that nothing else selected).
///
/// `also`: a second set of entries, evaluated without type-only edges, whose
/// results go to a report of their own (`crate::nextconfigdiff`: what a Next.js
/// config loads). They share the pass with `entries` (a result belongs to its
/// entry alone, and most of a pass does not depend on how many entries there
/// are), unless a node is in both sets: it would need both of its modes, so the
/// second set then gets a pass of its own.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply<V: GraphView>(
    g: &Graph,
    engine: &Engine<'_, V>,
    layer: &Layer,
    changes: &[Change],
    entries: &NodeSet,
    runtime_entries: &NodeSet,
    opts: &AffectedOptions,
    report: &mut AffectedReport,
    also: Option<(&NodeSet, &mut AffectedReport)>,
) -> usize {
    if !opts.named {
        return 0;
    }
    let cwds = entry_cwds(g);
    let tasks = task_dirs(g);
    let pass = |entries: &NodeSet, runtime: &NodeSet| {
        engine.named_affected(
            &layer.index,
            changes,
            entries,
            Some(runtime),
            opts,
            &cwds,
            &tasks,
        )
    };
    match also {
        Some((extra, into)) if !extra.iter().any(|n| entries.contains(n)) => {
            let (mut all, mut runtime) = (entries.clone(), runtime_entries.clone());
            for n in extra.iter() {
                all.insert(n);
                runtime.insert(n);
            }
            let (theirs, ours): (Vec<_>, Vec<_>) = pass(&all, &runtime)
                .into_iter()
                .partition(|r| extra.contains(r.entry));
            merge(g, theirs, into);
            merge(g, ours, report)
        }
        Some((extra, into)) => {
            merge(g, pass(extra, extra), into);
            merge(g, pass(entries, runtime_entries), report)
        }
        None => merge(g, pass(entries, runtime_entries), report),
    }
}

/// Merges the results of a names pass into `report`; returns how many entries
/// were added.
fn merge(
    g: &Graph,
    named: Vec<turborepo_trace_graph::names::NamedResult>,
    report: &mut AffectedReport,
) -> usize {
    if named.is_empty() {
        return 0;
    }
    let mut at: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
    for (i, e) in report.entries.iter().enumerate() {
        at.insert(e.entry, i);
    }
    let mut added = 0;
    for r in named {
        match at.get(&r.entry) {
            // A change is a stronger reason than uncertainty.
            Some(&i) if report.entries[i].cause.is_none() => {
                report.entries[i] = r.into_entry();
            }
            Some(_) => {}
            None => {
                added += 1;
                report.entries.push(r.into_entry());
            }
        }
    }
    report
        .entries
        .sort_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
    added
}

#[cfg(test)]
mod tests {
    use turborepo_trace_graph::ChangeKind;

    use super::*;

    #[test]
    fn where_a_task_runs_is_not_known_behind_a_directory_change_or_a_script() {
        for line in [
            "cd scripts && node ../build.js",
            "pnpm --dir ../x build",
            "make -C sub",
            "node x.js --cwd=..",
            "(pushd a; node b.js)",
            "npm --prefix web run build",
        ] {
            assert!(changes_directory(line), "{line}");
        }
        for line in [
            "node build.js",
            "tsx scripts/build.ts --watch",
            "next build && node post.js",
            "node build.js --define=cdn",
        ] {
            assert!(!changes_directory(line), "{line}");
        }
        for program in [
            None,
            Some("bash"),
            Some("sh"),
            Some("./scripts/gen"),
            Some("scripts/gen.sh"),
            Some("gen.sh"),
        ] {
            assert!(starts_a_script(program), "{program:?}");
        }
        for program in ["node", "tsx", "next", "go", "tsc"] {
            assert!(!starts_a_script(Some(program)), "{program}");
        }
    }

    #[test]
    fn tool_manifests() {
        for yes in [
            "package.json",
            "pkg/a/tsconfig.build.json",
            "turbo.json",
            ".github/workflows/ci.yml",
            "pnpm-lock.yaml",
        ] {
            assert!(is_tool_manifest(yes), "{yes}");
        }
        for no in [
            "data/routes.json",
            "config/default.yaml",
            "src/package.json.ts",
        ] {
            assert!(!is_tool_manifest(no), "{no}");
        }
    }

    #[test]
    fn names_inside_a_closed_region_are_left_out() {
        let changes = [Change::new(
            ChangeKind::Modified,
            "services/a/src/routes.ts",
        )];
        let keys = Keys::of(&changes);
        // Two names of the changed path: one string at offset 120, one at 40 and 300.
        let fact = "names:0,0,0,0,0\n7@120\tservices/a/src/routes\n3@40-300\tsrc/routes";
        let d = facts::decode(fact).unwrap();
        let n = NodeId(4);
        let shapes = |closed: &[ClosedSpan]| -> Vec<String> {
            file_names(&keys, n, "lib/table.ts", &d, false, closed, true)
                .shapes
                .into_iter()
                .map(|s| s.0)
                .collect()
        };
        assert_eq!(shapes(&[]), ["services/a/src/routes", "src/routes"]);
        // The table literal spans 100..200: the name whose only string is inside goes;
        // the one that also occurs outside stays.
        let table = ClosedSpan {
            node: n,
            start: 100,
            end: 200,
        };
        assert_eq!(shapes(&[table]), ["src/routes"]);
        let all = ClosedSpan {
            node: n,
            start: 0,
            end: 400,
        };
        assert!(shapes(&[all]).is_empty());
    }

    #[test]
    fn a_string_that_climbs_from_the_working_directory_names_its_places() {
        let fact = [
            "names:0,1,0,5,0",
            "4>3\tpackage.json",
            "6^\tshared/a.json",
            "8>2\tother.json",
            "9>1\tshared/app.",
        ]
        .join("\n");
        let d = facts::decode(&fact).unwrap();
        let n = NodeId(4);
        let changes = [
            Change::new(ChangeKind::Modified, "web/package.json"),
            Change::new(ChangeKind::Modified, "x/shared/a.json"),
            Change::new(ChangeKind::Modified, "shared/app.staging.yaml"),
        ];
        let keys = Keys::of(&changes);
        let f = file_names(&keys, n, "svc/a/x_test.go", &d, false, &[], true);
        // The shape counts inside the package; where the strings climb to is left
        // to `settle`. A plain climbing mark names its shape, continued, anywhere; a
        // name of no changed path is left out; a shape that only a continuation of
        // its last segment makes a name of a changed path climbs too.
        assert_eq!(f.shapes, [("package.json".to_owned(), None, 4, false)]);
        assert_eq!(f.prefixes, [("shared/a.json".to_owned(), 6)]);
        assert_eq!(
            f.cwd_names,
            [
                ("package.json".to_owned(), 4, 3, Some(0)),
                ("shared/app.".to_owned(), 9, 1, None)
            ]
        );
        assert_eq!(
            climb_places("package.json", 4, 3),
            [
                ("**/package.json".to_owned(), 4),
                ("../package.json".to_owned(), 4),
                ("../../package.json".to_owned(), 4),
                ("../../../package.json".to_owned(), 4)
            ]
        );
        // Off (A/B, or without the package assumption): the shape, continued,
        // anywhere.
        let f = file_names(&keys, n, "svc/a/x_test.go", &d, false, &[], false);
        assert!(f.shapes.is_empty() && f.cwd_names.is_empty());
        assert_eq!(
            f.prefixes,
            [
                ("package.json".to_owned(), 4),
                ("shared/a.json".to_owned(), 6),
                ("shared/app.".to_owned(), 9)
            ]
        );
    }

    #[test]
    fn keys_of_a_changed_path() {
        let changes = [Change::new(
            ChangeKind::Modified,
            "pkg/src/handlers/get.test.ts",
        )];
        let k = Keys::of(&changes);
        for shape in [
            "pkg/src/handlers/get.test.ts",
            "handlers/get.test.ts",
            "get.test.ts",
            "handlers",
            "src/handlers",
            "pkg",
            "handlers/get",
            "src/handlers/get.test",
        ] {
            assert!(k.shapes.contains(shape), "{shape}");
        }
        for not in ["get", "get.test", "ts", "src/get.test.ts"] {
            assert!(!k.shapes.contains(not), "{not}");
        }
        // Continued within the last segment, at a segment boundary.
        for shape in ["handlers/ge", "get.te", "pkg/src/hand", "g"] {
            assert!(k.continued(shape), "{shape}");
        }
        for not in ["et.test.ts", "andlers", "src/get"] {
            assert!(!k.continued(not), "{not}");
        }
        let mut dirs: Vec<&str> = k.dirs.iter().copied().collect();
        dirs.sort();
        assert_eq!(dirs, ["", "pkg", "pkg/src", "pkg/src/handlers"]);
    }

    #[cfg(unix)]
    #[test]
    fn links_below_a_directory_and_on_a_place() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join("svc/a/testdata/deep")).unwrap();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        assert!(!has_link_below(root, "svc/a"));
        assert!(!has_link_on(
            root,
            "svc",
            "shared/x.json",
            &mut Default::default()
        ));
        std::os::unix::fs::symlink("../../../../shared", root.join("svc/a/testdata/deep/data"))
            .unwrap();
        assert!(has_link_below(root, "svc/a"));
        assert!(!has_link_below(root, "shared"));
        std::os::unix::fs::symlink("../shared", root.join("svc/shared")).unwrap();
        assert!(has_link_on(
            root,
            "svc",
            "shared/x.json",
            &mut Default::default()
        ));
        assert!(!has_link_on(
            root,
            "",
            "shared/x.json",
            &mut Default::default()
        ));
    }

    #[test]
    fn ancestors() {
        assert_eq!(ancestor("a/b/c", 0), "a/b/c");
        assert_eq!(ancestor("a/b/c", 2), "a");
        assert_eq!(ancestor("a/b/c", 9), "");
    }
}
