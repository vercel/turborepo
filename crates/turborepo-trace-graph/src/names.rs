//! Named paths: the bound for reads whose path cannot be evaluated.
//!
//! A read with a computed path (`readFile(path)` with `path` a parameter, a
//! call result, a value the analysis does not follow) is an open uncertainty:
//! nothing says which file it reads, so under the default policy every entry
//! that reaches it is selected on every change. But the path is text, and it
//! was assembled by the code that ran: from strings of that code, from the
//! location of one of its files, or from text outside the repository
//! (the environment, the network, a temporary directory), which depends on no
//! repository file. Like a conservative garbage collector, which treats every
//! word that could be a pointer as one, this module treats every string of the
//! running code that could be (part of) a path as a possible name of the files
//! of that shape:
//!
//! > A read with a computed path can only read a repository file that some
//! > string or
//! > file location of the code loaded with it names.
//!
//! The record then needs no `--on-uncertain` verdict ([`Disposition::Named`]).
//! Instead:
//!
//! **Rule.** An entry is affected by a changed path `x` (added, modified or
//! deleted) when it reaches a module `P` whose closure along code-loading edges
//! between files contains both a file with a named record and a file that names
//! `x`.
//!
//! `P` stands for a process: strings only travel between modules loaded
//! together, so a build script reached through a task does not lend its strings
//! to the test that consumes the task's output (the script's own reads are
//! bounded by its own closure, and the test is affected through the task when
//! those are hit). Code-loading edges are imports, `require`, `import()`,
//! re-exports, workers and child processes; a type-only import is erased before
//! the code runs and loads nothing, and a data edge (a reporter that a
//! runner config names) is not a load into the tests' process.
//!
//! # Names
//!
//! The index ([`NameIndex`]) holds facts about files, collected by the language
//! plugin (JS/TS: `turborepo_trace_lang_js::names`; Go:
//! `turborepo_trace_lang_go::names` through the same predicate): *shapes* (runs
//! of literal path segments such as `fixtures/a.json`, `snapshots`,
//! `schema.sql`) and *locations* (a file that uses `__dirname` or
//! `import.meta.url` names everything below its own directory). A path is named
//! by a shape when the path or one of its ancestor directories ends with the
//! shape at a segment boundary, or does so up to the extension of its last
//! segment for shapes of two or more segments (`handlers/get` names
//! `src/handlers/get.ts`). So `fixtures` names every file below any directory
//! called `fixtures`, which also covers names computed from listings, hashes
//! and counters inside it.
//!
//! A string with a hole after its literal start (a template substitution, a
//! glob wildcard) keeps what follows the hole as a *rest* the path below the
//! shape has to fit ([`rest_matches`]): `` `/docs/configuration${x}` `` names
//! `docs/configuration.md` and `docs/configuration/a/b`, not `docs/other.md`;
//! `` `/services/${id}/apply` `` names nothing below a `services` directory
//! that has no `apply`. A hole stands for any text. The rest counts only in a
//! file without parent steps (`..`, `dirname`): such a step can cut the spelled
//! path back to the shape.
//!
//! Matching is textual, against the changed paths: nothing is resolved, no file
//! has to exist, and a file added later with that shape is matched like any
//! other change.
//!
//! Two kinds of name depend on the entry: a file that uses the working
//! directory names everything below the directories the entry runs in, and a
//! file that uses the runner's current test path names everything below the
//! directory of the entry.
//!
//! Data files (JSON, YAML, TOML) have names too. A data file belongs to a
//! process when a file of the process reads it through an edge. A data file
//! that is itself read through a computed path lends no names (which file it is
//! isn't known): what it names is covered as far as it lies below the directory
//! through which the data file was found, which a name or a location of the
//! process covers as a whole.
//!
//! # The package assumption
//!
//! A shape says what a path looks like, not where it is: `src` names the files
//! below every directory called `src`, in every package. The rule above alone
//! therefore lets a string of one package select tests for a change in any
//! other. By default the query assumes what holds for most code
//! ([`AffectedOptions::named_in_package`]):
//!
//! > A read with a computed path reads only files of the package it runs for:
//! > the package
//! > of the entry, or the package of the file that holds the read.
//!
//! So a name that does not say where it goes counts for a changed path `x` only
//! when `x` is a file of the entry's package, or of the package of a file with
//! a named record that `P` loads together with the naming file (a shared helper
//! that reads its own package's files). The file that holds the string lends
//! its shape, not its package. A package is a first-party package of the graph
//! (a workspace member, a Go module, a Python project, a crate); a package
//! inside another one is part of it (a Go module in the directory of
//! a service reads the service's files, and the service the module's), so the
//! package of a file is the outermost one around it other than the root
//! package. `x` is a file of a package when it lies at or below the package's
//! directory, and of the root package when it is one of its own files, the root
//! minus the other packages ([`Packages`]).
//!
//! This is an assumption, not a consequence of the rule, and it is the only one
//! the bound makes about where a path goes. It is wrong for code that reads
//! another package through a path that nothing in the code locates: a
//! root-relative string handed to a read while the process runs elsewhere, a
//! directory that comes from the environment or from an upward search.
//! `--strict` does not make it: it opens these reads altogether.
//!
//! **Evidence beats the assumption.** A name that says where it goes is not
//! bounded: a file's own location with its parent steps, a path built on it (a
//! literal one too: `join(__dirname, '../shared')` names what is at or below
//! that path), the directory a followed working-directory path evaluates to
//! (`join(cwd, '..', '..')` names what is below that directory, in whatever
//! package), and a string that is written with parent steps leading out of
//! where it starts (`'../../shared/a.json'` says that it leaves;
//! where it starts is not known, so it names its shape anywhere, as without the
//! assumption). Bounded are the names nothing locates: the other shapes, the
//! working directory and the test's directory as such, and what the parent
//! steps elsewhere in a file add to a followed working-directory path. Reads
//! with a known path are edges and are not concerned.
//!
//! # Who runs a process
//!
//! The names that depend on the entry (the working directory, the test's
//! directory, the package of the package assumption) are those of whoever runs
//! the process `P`. That is the entry when it loads `P` into its own process:
//! it reaches `P` without crossing a build task. When every way from `P` to the
//! entry crosses a build task, the entry only consumes what a task produces,
//! and `P` runs when the task runs: a build script reads relative to the
//! directory its task's command runs in, for the task's package, whoever
//! imports the built output later, and from wherever. The runner is then the
//! nearest task above `P` ([`AffectedOptions::named_by_runner`]):
//!
//! - its working directories are the directories the task's commands run in, as
//!   the caller knows them (the package directory for a package script; the
//!   repository root for a script of the root package), and its package is the
//!   task's package;
//! - an entry above such a task is selected when the task's names name the
//!   changed path, and not for a path that only the entry's own directory or
//!   package would name;
//! - **several tasks on the way:** the nearest one above `P` runs it; the tasks
//!   above consume its output like the entry does;
//! - **a task and a direct load:** an entry that also reaches `P` without a
//!   task keeps its own names, and is selected by either;
//! - **a task whose working directory is not known** (a command that changes
//!   directory, a task source that does not run in the task's directory): the
//!   entries above it keep the answer from before runners, their own names;
//! - **a shell script on the way** (the task runs a script, the script runs
//!   `P`): the script can change directory before it starts `P`, which nothing
//!   here follows, so the entries above it keep their own names too.
//!
//! The strings and locations of `P` are `P`'s whoever runs it; where a string
//! counts (the package the read runs for) follows the runner like the working
//! directory does.
//!
//! # What the bound does not cover
//!
//! Under the package assumption, a path into another package that nothing in
//! the code locates (see above).
//!
//! A path whose pieces are in none of the indexed files: strings of third-party
//! packages and of generated files that were not on disk, text from the
//! environment that the repository itself sets, pieces without a separator, an
//! extension or a path-function position (`'fix' + 'tures'`), a parent
//! directory reached by an upward search from an unknown base followed by a
//! listing, and a path that a data file found through a computed path gives for
//! a file outside the directory it was found in. Language plugins
//! without a name scan keep their records open, and so do module loads with a
//! computed specifier: what they load brings its own dependencies.

use std::collections::{HashMap, HashSet};

use crate::{
    change::{AffectedOptions, Cause, Change, Disposition, EntryResult, Verdict},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::{EdgeAttr, EdgeFilter, RefFlags, RefKind, UncertaintyKind},
};

const NONE: u32 = u32::MAX;

mod attr;
pub use attr::NamedGroup;

/// Where a name is: the file and the 1-based line.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NameRef {
    /// The file that contains the string.
    pub node: NodeId,
    /// 1-based line; 0 if unknown.
    pub line: u32,
    /// A string written with parent steps that lead out of the directory it
    /// starts from (`'../shared/a.json'`): the code says that the path
    /// leaves, so the package assumption does not hold it to a package.
    pub climbs: bool,
}

impl NameRef {
    fn at(node: NodeId, line: u32) -> Self {
        NameRef {
            node,
            line,
            climbs: false,
        }
    }
}

/// How a path is named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedBy<'a> {
    /// By a string of that shape.
    Shape(&'a str),
    /// By the location of a file in that directory (`__dirname`,
    /// `import.meta.url`).
    Location(&'a str),
    /// By a path the file builds on its own location (a file-name pattern in a
    /// directory).
    Path(&'a str),
    /// By a file with too many names to list.
    Everything,
}

/// Whether the segments below a shape fit its rest: some path spelled by the
/// rest is `below` itself or one of its ancestors (a name stands for everything
/// below it too).
///
/// A rest is a list of elements joined with `/`: a literal segment; `text*`, a
/// segment that starts with the text, followed by any number of segments;
/// `*text`, any number of segments and one that ends with the text; `*`, any
/// number of segments (a hole is any text, with separators or empty). A literal
/// element at the very end also fits the last segment without its extensions
/// (`handlers/*/get` names `handlers/v1/get.ts`).
pub fn rest_matches(rest: &str, below: &[&str]) -> bool {
    let elems: Vec<&str> = rest.split('/').filter(|e| !e.is_empty()).collect();
    fits(&elems, below, false)
}

/// `any`: any number of segments may be skipped first.
fn fits(elems: &[&str], segs: &[&str], any: bool) -> bool {
    let Some((&e, more)) = elems.split_first() else {
        return true;
    };
    if e == "*" {
        return fits(more, segs, true);
    }
    let last = more.is_empty();
    let skips = if any { segs.len() } else { 0 };
    for (i, seg) in segs.iter().enumerate().take(skips + 1) {
        let after = &segs[i + 1..];
        let ok = if let Some(text) = e.strip_suffix('*') {
            seg.starts_with(text) && fits(more, after, true)
        } else if let Some(text) = e.strip_prefix('*') {
            // The hole in front may hold separators: checked at every segment.
            return segs[i..]
                .iter()
                .enumerate()
                .any(|(j, s)| s.ends_with(text) && fits(more, &segs[i + j + 1..], false));
        } else {
            (*seg == e && fits(more, after, false))
                || (last
                    && after.is_empty()
                    && seg.match_indices('.').any(|(k, _)| k > 0 && &seg[..k] == e))
        };
        if ok {
            return true;
        }
    }
    false
}

/// A shape with a rest.
#[derive(Clone, Debug)]
struct Pattern {
    rest: Box<str>,
    /// `shape/rest`, for messages.
    text: Box<str>,
    at: NameRef,
}

/// The names of the indexed files.
#[derive(Clone, Debug, Default)]
pub struct NameIndex {
    shapes: HashMap<Box<str>, Vec<NameRef>>,
    /// Shapes with a rest, by shape.
    patterns: HashMap<Box<str>, Vec<Pattern>>,
    /// Directory (repository relative, `""` for the root) to the files that use
    /// their own location.
    locations: HashMap<Box<str>, Vec<NameRef>>,
    /// Paths named exactly (a pattern in a directory built on a file's
    /// location, matched against the changed paths by the caller).
    paths: HashMap<Box<str>, Vec<NameRef>>,
    /// Directories built on the working directory: the relative part and what
    /// is read there (`**`, or a file-name pattern for files directly in
    /// it).
    cwd_dirs: Vec<(NameRef, Box<str>, Box<str>)>,
    everything: Vec<NameRef>,
    /// Files that use the working directory.
    cwd_users: Vec<NameRef>,
    /// Files that use the runner's current test path.
    test_path_users: Vec<NameRef>,
    /// Data files: their names reach a process through whatever reads them.
    data: HashSet<u32>,
    /// Per file: how many directories above an entry-relative base it can reach
    /// (`..` segments and `dirname` calls).
    ups: HashMap<u32, u32>,
    count: usize,
}

impl NameIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// `node` has a string of shape `shape` (literal segments joined with `/`)
    /// at `line`.
    pub fn add_shape(&mut self, node: NodeId, shape: &str, line: u32) {
        self.add_name(node, shape, None, line, false);
    }

    /// `node` has a string of shape `shape` at `line` whose continuation has to
    /// fit `rest` ([`rest_matches`]).
    pub fn add_pattern(&mut self, node: NodeId, shape: &str, rest: &str, line: u32) {
        self.add_name(node, shape, Some(rest), line, false);
    }

    /// `node` has a string of shape `shape` at `line`, with a rest its
    /// continuation has to fit, if any. `climbs`: the string was written
    /// with parent steps that lead out of the directory it starts from
    /// ([`NameRef::climbs`]).
    pub fn add_name(
        &mut self,
        node: NodeId,
        shape: &str,
        rest: Option<&str>,
        line: u32,
        climbs: bool,
    ) {
        let shape = shape.trim_matches('/');
        if shape.is_empty() {
            return;
        }
        let at = NameRef { node, line, climbs };
        match rest {
            Some(rest) => self
                .patterns
                .entry(shape.into())
                .or_default()
                .push(Pattern {
                    rest: rest.into(),
                    text: format!("{shape}/{rest}").into(),
                    at,
                }),
            None => self.shapes.entry(shape.into()).or_default().push(at),
        }
        self.count += 1;
    }

    /// `node`, a file in the directory `dir` (repository relative, `""` for the
    /// root), uses its own location at `line`.
    pub fn add_location(&mut self, node: NodeId, dir: &str, line: u32) {
        self.locations
            .entry(dir.trim_matches('/').into())
            .or_default()
            .push(NameRef::at(node, line));
        self.count += 1;
    }

    /// `node` names exactly the repository-relative `path` (by a path it builds
    /// on its own location at `line`).
    pub fn add_path(&mut self, node: NodeId, path: &str, line: u32) {
        self.paths
            .entry(path.trim_matches('/').into())
            .or_default()
            .push(NameRef::at(node, line));
        self.count += 1;
    }

    /// `node` builds the directory `rel` (relative, possibly with leading `..`)
    /// on the working directory at `line` and names `pattern` there: `**`
    /// for everything at or below it, otherwise a file-name pattern for
    /// files directly in it.
    pub fn add_cwd_dir(&mut self, node: NodeId, rel: &str, pattern: &str, line: u32) {
        self.cwd_dirs
            .push((NameRef::at(node, line), rel.into(), pattern.into()));
        self.count += 1;
    }

    /// `node` names every path.
    pub fn add_everything(&mut self, node: NodeId) {
        self.everything.push(NameRef::at(node, 0));
        self.count += 1;
    }

    /// `node` uses the working directory at `line`: it names everything below
    /// the directories an entry that loads it runs in.
    pub fn add_cwd_user(&mut self, node: NodeId, line: u32) {
        self.cwd_users.push(NameRef::at(node, line));
        self.count += 1;
    }

    /// `node` uses the runner's current test path at `line`: it names
    /// everything below the directory of an entry that loads it.
    pub fn add_test_path_user(&mut self, node: NodeId, line: u32) {
        self.test_path_users.push(NameRef::at(node, line));
        self.count += 1;
    }

    /// `node` can reach `ups` directories above the working directory or the
    /// test's directory (its `..` segments and `dirname` calls).
    pub fn set_ups(&mut self, node: NodeId, ups: u32) {
        if ups > 0 {
            self.ups.insert(node.0, ups);
        }
    }

    /// `node` is a data file: no code is loaded from it.
    pub fn mark_data(&mut self, node: NodeId) {
        self.data.insert(node.0);
    }

    /// Whether `node` was marked as a data file.
    pub fn is_data(&self, node: NodeId) -> bool {
        self.data.contains(&node.0)
    }

    /// Number of names.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the index has no names.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Calls `f` for every name that names the repository-relative `path`.
    pub fn named(&self, path: &str, f: &mut dyn FnMut(NameRef, NamedBy<'_>)) {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return;
        }
        for &r in &self.everything {
            f(r, NamedBy::Everything);
        }
        if let Some(refs) = self.paths.get(path) {
            for &r in refs {
                f(r, NamedBy::Path(path));
            }
        }
        // Segment boundaries: `starts[i]` is the offset of segment `i`, `ends[i]` its
        // end.
        let mut starts = vec![0usize];
        let mut ends = Vec::new();
        for (i, b) in path.bytes().enumerate() {
            if b == b'/' {
                ends.push(i);
                starts.push(i + 1);
            }
        }
        ends.push(path.len());
        let n = starts.len();
        // Locations: every ancestor directory of the path.
        if !self.locations.is_empty() {
            let mut dirs: Vec<&str> = vec![""];
            dirs.extend(ends[..n - 1].iter().map(|&e| &path[..e]));
            for d in dirs {
                if let Some(refs) = self.locations.get(d) {
                    for &r in refs {
                        f(r, NamedBy::Location(d));
                    }
                }
            }
        }
        // Shapes with a rest: the segments after the shape have to fit it.
        if !self.patterns.is_empty() {
            let segs: Vec<&str> = (0..n).map(|i| &path[starts[i]..ends[i]]).collect();
            for end in 0..n {
                for start in 0..=end {
                    if let Some(ps) = self.patterns.get(&path[starts[start]..ends[end]]) {
                        for p in ps {
                            if rest_matches(&p.rest, &segs[end + 1..]) {
                                f(p.at, NamedBy::Shape(&p.text));
                            }
                        }
                    }
                }
            }
        }
        if self.shapes.is_empty() {
            return;
        }
        let lookup = |key: &str, f: &mut dyn FnMut(NameRef, NamedBy<'_>)| {
            if let Some((shape, refs)) = self.shapes.get_key_value(key) {
                for &r in refs {
                    f(r, NamedBy::Shape(shape));
                }
            }
        };
        // The path itself and each ancestor directory, by every suffix of its segments.
        for end in 0..n {
            for start in 0..=end {
                lookup(&path[starts[start]..ends[end]], f);
            }
        }
        // The file without its extensions, for shapes of two or more segments.
        let last = &path[starts[n - 1]..];
        for (i, b) in last.bytes().enumerate() {
            if b == b'.' && i > 0 {
                for &s in &starts[..n - 1] {
                    lookup(&path[s..starts[n - 1] + i], f);
                }
            }
        }
    }
}

/// Edges that load code into the process of the loading file. A type-only
/// import is erased before the code runs, and a data edge links a file that
/// runs in another role (a reporter a runner config names) or is only read.
fn loads_code(a: EdgeAttr) -> bool {
    !a.flags().intersects(RefFlags::TYPE_ONLY | RefFlags::DATA)
        && !a.is_config()
        && matches!(
            a.kind(),
            RefKind::Import
                | RefKind::Require
                | RefKind::DynamicImport
                | RefKind::ReExport
                | RefKind::Worker
                | RefKind::ChildProcess
                | RefKind::Package
                | RefKind::Mod
        )
}

/// Edges through which a file reads a data file's contents (a JSON module is
/// one too).
fn reads_data(a: EdgeAttr) -> bool {
    !a.flags().contains(RefFlags::TYPE_ONLY) && a.kind() != RefKind::Manifest
}

/// A named change for one entry: what [`Engine::named_affected`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedResult {
    /// The entry.
    pub entry: NodeId,
    /// Chain from the naming file to the entry: `witness[i + 1]` has an edge to
    /// `witness[i]`.
    pub witness: Vec<NodeId>,
    /// The cause ([`Cause::Named`]).
    pub cause: Cause,
}

impl NamedResult {
    /// As an entry result with the verdict affected.
    pub fn into_entry(self) -> EntryResult {
        EntryResult {
            entry: self.entry,
            verdict: Verdict::Affected,
            witness: self.witness,
            cause: Some(self.cause),
            uncertain: None,
        }
    }
}

/// One row of [`Engine::named_report`].
#[derive(Clone, Debug)]
pub struct NameReport {
    /// The file with the names.
    pub node: NodeId,
    /// 1-based line of its first name of a changed path.
    pub line: u32,
    /// The names (at most four), readable.
    pub by: Vec<String>,
    /// A changed path they name.
    pub path: String,
    /// Entries this file's names select.
    pub inclusive: usize,
    /// Entries no other naming file selects.
    pub exclusive: usize,
    /// The selected entries (node ids).
    pub entries: Vec<u32>,
}

/// A file that names a changed path.
#[derive(Clone)]
struct Hit {
    /// The changed path.
    path: String,
    /// What names it, readable.
    by: String,
    line: u32,
}

/// Whether `path` is `dir` or below it (`""` is the repository root).
fn below(path: &str, dir: &str) -> bool {
    dir.is_empty()
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The directory `rel` (segments, `..` for the parent) taken from the directory
/// `dir` (repository relative, `""` for the root). A climb above the repository
/// root stays at the root.
pub fn join_rel(dir: &str, rel: &str) -> String {
    let mut out: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// Whether the file name `name` fits `pattern`: literal text and `*` for any
/// text.
pub fn glob_name(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    // No `*`: the whole name.
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };
    for p in parts {
        match rest.find(p) {
            Some(i) => rest = &rest[i + p.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
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

/// The entry-relative names.
#[derive(Copy, Clone, PartialEq, Eq)]
enum EntryBase {
    Cwd,
    TestDir,
}

impl<G: GraphView> Engine<'_, G> {
    /// Per node: whether it is a file that code can be loaded from or into (not
    /// a task, a task part or a membership hub).
    fn code_files(&self) -> Vec<bool> {
        let g = self.graph();
        (0..g.node_count())
            .map(|i| !g.lang(NodeId(i)).is_some_and(|l| l.starts_with("task")))
            .collect()
    }

    /// Reverse reachability from `seeds` along code-loading edges between
    /// files: `parent[n]` is the next node toward a seed (`n` itself for a
    /// seed), `NONE` when `n` does not reach one. From a seed that is a
    /// data file of `data`, the first step is any edge through which a code
    /// file reads it.
    fn code_ancestors(
        &self,
        code: &[bool],
        seeds: impl Iterator<Item = NodeId>,
        data: Option<&NameIndex>,
    ) -> Vec<u32> {
        let rev = self.reverse();
        let mut parent = vec![NONE; self.graph().node_count() as usize];
        let mut queue: Vec<u32> = Vec::new();
        for s in seeds {
            if parent[s.index()] == NONE {
                parent[s.index()] = s.0;
                queue.push(s.0);
            }
        }
        let mut head = 0;
        while head < queue.len() {
            let n = NodeId(queue[head]);
            head += 1;
            if !code[n.index()] {
                continue;
            }
            let is_data = data.is_some_and(|d| d.is_data(n));
            let (srcs, attrs) = rev.neighbors(n);
            for (&s, &a) in srcs.iter().zip(attrs) {
                let accepted = if is_data {
                    reads_data(a)
                } else {
                    loads_code(a)
                };
                if parent[s as usize] == NONE && accepted && code[s as usize] {
                    parent[s as usize] = n.0;
                    queue.push(s);
                }
            }
        }
        parent
    }

    /// Reverse reachability from `roots` along the edges `filter` accepts, as a
    /// parent array (see [`Engine::code_ancestors`]).
    fn up_from(&self, roots: &[NodeId], filter: EdgeFilter) -> Vec<u32> {
        let rev = self.reverse();
        let mut parent = vec![NONE; self.graph().node_count() as usize];
        let mut queue: Vec<u32> = Vec::new();
        for r in roots {
            parent[r.index()] = r.0;
            queue.push(r.0);
        }
        let every = filter.is_all();
        let mut head = 0;
        while head < queue.len() {
            let n = NodeId(queue[head]);
            head += 1;
            let (srcs, attrs) = rev.neighbors(n);
            for (&s, &a) in srcs.iter().zip(attrs) {
                if parent[s as usize] == NONE && (every || filter.accepts(a)) {
                    parent[s as usize] = n.0;
                    queue.push(s);
                }
            }
        }
        parent
    }

    /// [`Engine::up_from`] that tells apart who runs the roots. First the walk
    /// stops at build tasks and shell scripts (`task`): what it reaches
    /// loads a root without one in between. The tasks it stops at are the
    /// nearest ones above a root. From those that are not `known`, and from
    /// the shell scripts (a root that is one included: it is the
    /// script that starts the program), it goes on without stopping, through
    /// other tasks too: an entry above a task whose
    /// working directory is not known is treated as if it ran the root itself.
    /// Returns the parent array and the known nearest tasks, which are not
    /// walked through.
    fn up_from_by_runner(
        &self,
        roots: &[NodeId],
        filter: EdgeFilter,
        task: &[bool],
        known: &dyn Fn(NodeId) -> bool,
    ) -> (Vec<u32>, Vec<NodeId>) {
        let rev = self.reverse();
        let mut parent = vec![NONE; self.graph().node_count() as usize];
        let mut queue: Vec<u32> = Vec::new();
        for r in roots {
            parent[r.index()] = r.0;
            queue.push(r.0);
        }
        let every = filter.is_all();
        // Per node, for the tasks the first walk stopped at: 1 known, 2 not known; 0
        // for every other node, and for those once walked through.
        let mut stopped = vec![0u8; parent.len()];
        let mut at: Vec<u32> = Vec::new();
        let mut head = 0;
        while head < queue.len() {
            let n = NodeId(queue[head]);
            head += 1;
            if task[n.index()] {
                stopped[n.index()] = if known(n) { 1 } else { 2 };
                at.push(n.0);
                continue;
            }
            let (srcs, attrs) = rev.neighbors(n);
            for (&s, &a) in srcs.iter().zip(attrs) {
                if parent[s as usize] == NONE && (every || filter.accepts(a)) {
                    parent[s as usize] = n.0;
                    queue.push(s);
                }
            }
        }
        at.sort_unstable();
        let nearest: Vec<NodeId> = at
            .iter()
            .filter(|t| stopped[**t as usize] == 1)
            .map(|&t| NodeId(t))
            .collect();
        let mut queue: Vec<u32> = at
            .iter()
            .copied()
            .filter(|t| stopped[*t as usize] == 2)
            .collect();
        for t in &queue {
            stopped[*t as usize] = 0;
        }
        let mut head = 0;
        while head < queue.len() {
            let n = NodeId(queue[head]);
            head += 1;
            let (srcs, attrs) = rev.neighbors(n);
            for (&s, &a) in srcs.iter().zip(attrs) {
                if !(every || filter.accepts(a)) {
                    continue;
                }
                if parent[s as usize] == NONE {
                    parent[s as usize] = n.0;
                    queue.push(s);
                } else if stopped[s as usize] == 1 {
                    // A known task that is also above a task that is not known: what is
                    // above it is reached through the latter too.
                    stopped[s as usize] = 0;
                    queue.push(s);
                }
            }
        }
        (parent, nearest)
    }

    /// How a change query treats the record `u` of `n`:
    /// [`AffectedOptions::disposition`], except that a record the view
    /// resolves per entry ([`GraphView::is_entry_resolved`]: a
    /// working-directory-relative literal path under a test runner) keeps
    /// that exact treatment instead of being bounded by names or by the
    /// declared packages.
    pub fn disposition_of(
        &self,
        n: NodeId,
        u: &crate::view::Uncertainty<'_>,
        opts: &AffectedOptions,
    ) -> Disposition {
        let g = self.graph();
        match opts.disposition(u, g.lang(n)) {
            Disposition::Named | Disposition::Declared
                if g.resolved_entries().is_some() && g.is_entry_resolved(n, u) =>
            {
                Disposition::Open
            }
            d => d,
        }
    }

    /// The first-party nodes with a record that names bound under `opts`
    /// ([`Disposition::Named`]).
    pub fn named_sites(&self, opts: &AffectedOptions) -> Vec<NodeId> {
        let g = self.graph();
        let mut out = Vec::new();
        for &n in self.uncertain_nodes() {
            if g.is_external(n) {
                continue;
            }
            if g.uncertainties(n)
                .any(|u| self.disposition_of(n, &u, opts) == Disposition::Named)
            {
                out.push(n);
            }
        }
        out
    }

    /// Which names select how many entries: for every file with a string or a
    /// location that names a changed path, the entries it selects
    /// (`inclusive`: whatever else selects them) and the entries only it
    /// selects among the naming files (`exclusive`). The working-directory
    /// and test-path names are not part of it, and the package assumption
    /// is not applied: the report shows what a name reaches in the whole
    /// repository ([`Engine::named_groups`] applies it). One reachability
    /// pass per naming file: a report, not part of a normal query.
    pub fn named_report(
        &self,
        index: &NameIndex,
        changes: &[Change],
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        opts: &AffectedOptions,
    ) -> Vec<NameReport> {
        let g = self.graph();
        let sites = self.named_sites(opts);
        if sites.is_empty() {
            return Vec::new();
        }
        let code = self.code_files();
        let to_site = self.code_ancestors(&code, sites.iter().copied(), None);
        let all = self.options().traversal.for_uncertainty();
        let runtime = all.without_flags(RefFlags::TYPE_ONLY);
        let split = runtime_entries.filter(|_| runtime != all);
        // Per naming file: the strings that name a changed path.
        let mut hits: HashMap<u32, (u32, Vec<String>, String)> = HashMap::new();
        for c in changes {
            index.named(&c.path, &mut |r, by| {
                let by = match by {
                    NamedBy::Shape(s) => format!("'{s}'"),
                    NamedBy::Location("") => "its location (the repository root)".to_owned(),
                    NamedBy::Location(d) => format!("its location (below {d}/)"),
                    NamedBy::Path(p) => format!("a path on its location ({p})"),
                    NamedBy::Everything => "everything".to_owned(),
                };
                let h = hits
                    .entry(r.node.0)
                    .or_insert_with(|| (r.line, Vec::new(), c.path.clone()));
                if !h.1.contains(&by) && h.1.len() < 4 {
                    h.1.push(by);
                }
            });
        }
        let mut order: Vec<u32> = hits.keys().copied().collect();
        order.sort_unstable();
        let mut count = vec![0u32; g.node_count() as usize];
        let mut last = vec![NONE; g.node_count() as usize];
        let mut sets: Vec<Vec<u32>> = Vec::with_capacity(order.len());
        for (hi, &h) in order.iter().enumerate() {
            let to_name = self.code_ancestors(&code, std::iter::once(NodeId(h)), Some(index));
            let roots: Vec<NodeId> = (0..g.node_count())
                .filter(|&i| to_name[i as usize] != NONE && to_site[i as usize] != NONE)
                .map(NodeId)
                .collect();
            let mut selected: Vec<u32> = Vec::new();
            if !roots.is_empty() {
                let with_types = self.up_from(&roots, all);
                let without = split.map(|_| self.up_from(&roots, runtime));
                for e in entries.iter() {
                    let parent = match (&without, split) {
                        (Some(w), Some(rt)) if rt.contains(e) => w,
                        _ => &with_types,
                    };
                    if parent[e.index()] != NONE {
                        selected.push(e.0);
                        count[e.index()] += 1;
                        last[e.index()] = hi as u32;
                    }
                }
            }
            sets.push(selected);
        }
        let mut out: Vec<NameReport> = order
            .iter()
            .zip(&sets)
            .enumerate()
            .map(|(hi, (&h, set))| {
                let (line, by, path) = &hits[&h];
                NameReport {
                    node: NodeId(h),
                    line: *line,
                    by: by.clone(),
                    path: path.clone(),
                    inclusive: set.len(),
                    exclusive: set
                        .iter()
                        .filter(|&&e| count[e as usize] == 1 && last[e as usize] == hi as u32)
                        .count(),
                    entries: set.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| b.inclusive.cmp(&a.inclusive).then(a.node.0.cmp(&b.node.0)));
        out
    }

    /// Which reads select how many entries through names: for every file with a
    /// named record, the entries selected because that file and a file
    /// naming a changed path are loaded together (`inclusive`), and the
    /// entries for which it is the only such file (`exclusive`: resolving
    /// that file's reads clears them). The package assumption is
    /// not applied. One reachability pass per file with a named record: a
    /// report, not part of a normal query.
    pub fn named_site_report(
        &self,
        index: &NameIndex,
        changes: &[Change],
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        opts: &AffectedOptions,
    ) -> Vec<NameReport> {
        let g = self.graph();
        let sites = self.named_sites(opts);
        let mut hits: Vec<NodeId> = Vec::new();
        for c in changes {
            index.named(&c.path, &mut |r, _| hits.push(r.node));
        }
        hits.sort_unstable_by_key(|n| n.0);
        hits.dedup();
        if sites.is_empty() || hits.is_empty() {
            return Vec::new();
        }
        let code = self.code_files();
        let to_name = self.code_ancestors(&code, hits.iter().copied(), Some(index));
        let all = self.options().traversal.for_uncertainty();
        let runtime = all.without_flags(RefFlags::TYPE_ONLY);
        let split = runtime_entries.filter(|_| runtime != all);
        let mut count = vec![0u32; g.node_count() as usize];
        let mut last = vec![NONE; g.node_count() as usize];
        let mut sets: Vec<Vec<u32>> = Vec::with_capacity(sites.len());
        for (si, &s) in sites.iter().enumerate() {
            let to_site = self.code_ancestors(&code, std::iter::once(s), None);
            let roots: Vec<NodeId> = (0..g.node_count())
                .filter(|&i| to_name[i as usize] != NONE && to_site[i as usize] != NONE)
                .map(NodeId)
                .collect();
            let mut selected: Vec<u32> = Vec::new();
            if !roots.is_empty() {
                let with_types = self.up_from(&roots, all);
                let without = split.map(|_| self.up_from(&roots, runtime));
                for e in entries.iter() {
                    let parent = match (&without, split) {
                        (Some(w), Some(rt)) if rt.contains(e) => w,
                        _ => &with_types,
                    };
                    if parent[e.index()] != NONE {
                        selected.push(e.0);
                        count[e.index()] += 1;
                        last[e.index()] = si as u32;
                    }
                }
            }
            sets.push(selected);
        }
        let mut out: Vec<NameReport> = sites
            .iter()
            .zip(&sets)
            .enumerate()
            .filter(|(_, (_, set))| !set.is_empty())
            .map(|(si, (&s, set))| NameReport {
                node: s,
                line: 0,
                by: Vec::new(),
                path: String::new(),
                inclusive: set.len(),
                exclusive: set
                    .iter()
                    .filter(|&&e| count[e as usize] == 1 && last[e as usize] == si as u32)
                    .count(),
                entries: set.clone(),
            })
            .collect();
        out.sort_by(|a, b| b.inclusive.cmp(&a.inclusive).then(a.node.0.cmp(&b.node.0)));
        out
    }

    /// The entries that the changed paths affect through names (see the
    /// [module documentation](crate::names)): for each, a chain from the file
    /// with the name up to the entry, and the cause. `runtime_entries` are
    /// evaluated without type-only edges, as in [`Engine::affected_with`].
    /// `cwds` gives the directories an entry runs in (repository relative,
    /// `""` for the root); an entry with none is taken to run
    /// anywhere. `task_dirs` gives the directories the commands of a build task
    /// run in, `None` when that is not known (see "Who runs a process" in
    /// the module documentation). One result per entry, in node order; the
    /// caller merges them with the change-driven results (a direct change
    /// is the stronger reason).
    #[allow(clippy::too_many_arguments)]
    pub fn named_affected(
        &self,
        index: &NameIndex,
        changes: &[Change],
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        opts: &AffectedOptions,
        cwds: &dyn Fn(NodeId) -> Vec<String>,
        task_dirs: &dyn Fn(NodeId) -> Option<Vec<String>>,
    ) -> Vec<NamedResult> {
        let g = self.graph();
        let timing =
            turborepo_trace_core::debug::var_os("NAMES_TIMING").map(|_| std::time::Instant::now());
        let lap = |what: &str| {
            if let Some(t) = timing {
                eprintln!(
                    "names pass: {what} at {:.1} ms",
                    t.elapsed().as_secs_f64() * 1e3
                );
            }
        };
        let Some(p) = self.pairing(changes, entries, runtime_entries, opts, task_dirs) else {
            return Vec::new();
        };
        // The directories and the package that `runner` (an entry, or the task that
        // runs the process) gives a process.
        let dirs_of = |runner: NodeId| -> Vec<String> {
            match p.runners.get(&runner.0) {
                Some(r) => r.dirs.clone(),
                None => cwds(runner),
            }
        };
        // ` of task:pkg#build`, for a name that a task gives.
        let of_runner = |runner: NodeId| -> String {
            if p.runners.contains_key(&runner.0) {
                format!(" of {}", g.path(runner))
            } else {
                String::new()
            }
        };
        lap("modules that load a named read");
        let mut out: Vec<NamedResult> = Vec::new();
        let mut done = NodeSet::new(g.node_count());

        // 1. Strings and file locations. A location says where it goes; a string counts
        // for the packages that hold the changed path (the package assumption).
        let hits = p.hits(index, changes);
        let mut add = |e: NodeId, chain: Vec<NodeId>, hit: &Hit, site: NodeId| {
            if !done.insert(e) {
                return;
            }
            out.push(NamedResult {
                entry: e,
                witness: chain,
                cause: Cause::Named {
                    path: hit.path.clone(),
                    by: hit.by.clone(),
                    line: hit.line,
                    site: g.path(site).to_owned(),
                    site_line: p.site_line(site),
                },
            });
        };
        if !hits.free.is_empty() {
            let to_name = p.to_names(index, hits.free.keys().copied());
            for (e, chain, root) in p.reach(&to_name, &p.to_site, None) {
                if let Some(hit) = chain.first().and_then(|n| hits.free.get(&n.0)) {
                    let site = p.site_node(root, &p.to_site);
                    add(e, chain, hit, site);
                }
            }
        }
        // The packages with the same naming files share the passes.
        let mut tasks_hit: Vec<Seed<Hit>> = Vec::new();
        for group in hits.groups() {
            let sites = p.sites_in(&group.packages);
            let entries = group.packages.iter().any(|k| p.has_runners(k));
            if !entries && sites.is_empty() {
                continue;
            }
            let to_name = p.to_names(index, group.files.iter().copied());
            // The first name of the file `n` for a path the package at `pkg` holds.
            let hit_of = |n: NodeId, pkg: &str| {
                hits.scoped
                    .get(pkg)
                    .and_then(|m| m.get(&n.0))
                    .or_else(|| group.packages.iter().find_map(|k| hits.scoped[k].get(&n.0)))
            };
            // The changed path is in the package the process runs for: the package of the
            // entry, or of the task that runs it.
            if entries {
                let found = p.reach_by_runner(
                    &to_name,
                    &p.to_site,
                    Some(&group.packages),
                    &|_| false,
                    &|_, _| true,
                    &mut |r, chain, _| chain.first().and_then(|&n| hit_of(n, p.home(r))).cloned(),
                    &mut tasks_hit,
                );
                for s in found {
                    add(s.entry, s.chain, &s.hit, s.site);
                }
            }
            // It is in the package of a file with a named read.
            if !sites.is_empty() {
                let to_site = self.code_ancestors(&p.code, sites.iter().copied(), None);
                for (e, chain, root) in p.reach(&to_name, &to_site, None) {
                    let site = p.site_node(root, &to_site);
                    if let Some(hit) = chain.first().and_then(|&n| hit_of(n, p.home(site))) {
                        add(e, chain, hit, site);
                    }
                }
            }
        }
        for s in p.above(&tasks_hit, &|_| false) {
            add(s.entry, s.chain, &s.hit, s.site);
        }
        lap("modules that load a naming file");

        // 2. Names that depend on the entry: the working directory and the test's own
        // directory. A file with a named working-directory record uses the working
        // directory whether or not it says so.
        let mut cwd_users: HashMap<u32, u32> = HashMap::new();
        for r in &index.cwd_users {
            cwd_users.entry(r.node.0).or_insert(r.line);
        }
        // Files that only have a working-directory record: the reference was resolved
        // against the repository root when the file was traced, so the root is covered
        // by the edges, probes and memberships recorded on the file itself.
        let mut record_only: HashSet<u32> = HashSet::new();
        for &n in &p.sites {
            if let Some(u) = g.uncertainties(n).find(|u| {
                u.kind == UncertaintyKind::CwdDependent
                    && self.disposition_of(n, u, opts) == Disposition::Named
            }) && !cwd_users.contains_key(&n.0)
            {
                cwd_users.insert(n.0, u.line);
                record_only.insert(n.0);
            }
        }
        let mut test_users: HashMap<u32, u32> = HashMap::new();
        for r in &index.test_path_users {
            test_users.entry(r.node.0).or_insert(r.line);
        }
        // The rounds after the first (the packages with named reads that hold a changed
        // path) only matter for an entry whose directory reaches such a path and no
        // path of its own package: the changed paths by the directories above
        // them, and the directories of the entries by package.
        let scopes = p.scopes(changes);
        // Every changed path by the directories above it: a runner whose directories
        // have no changed path below them names none, and most runners are in
        // that case.
        let mut changed_below = Below::default();
        for c in changes {
            changed_below.add(&c.path);
        }
        // The same per package that holds them (the package assumption): what a runner
        // of that package can read in the first round.
        let mut changed_in: HashMap<&str, Below<'_>> = HashMap::new();
        if let Some(pk) = &p.packages {
            for c in changes {
                for k in pk.holders(&c.path) {
                    changed_in.entry(k).or_default().add(&c.path);
                }
            }
        }
        // Whether a runner of the package at `home` can read a changed path at or below
        // `dir` in the round `round`: a path of its own package in the first round, of
        // the round's packages later.
        let near_in = |home: &str, round: usize, dir: &str| -> bool {
            if round > 0 {
                scopes.rounds[round - 1].any(dir)
            } else if p.packages.is_some() {
                changed_in.get(home).is_some_and(|b| b.any(dir))
            } else {
                changed_below.any(dir)
            }
        };
        let mut cwd_classes: Vec<(String, &str)> = Vec::new();
        let mut test_classes: Vec<(&str, &str)> = Vec::new();
        if !scopes.rounds.is_empty() {
            let mut seen: HashSet<(String, &str)> = HashSet::new();
            let mut seen_tests: HashSet<(&str, &str)> = HashSet::new();
            let any_cwd = !cwd_users.is_empty() || !index.cwd_dirs.is_empty();
            for e in entries.iter() {
                let pkg = p.home(e);
                if any_cwd {
                    let mut dirs = cwds(e);
                    if dirs.is_empty() {
                        dirs.push(String::new());
                    }
                    for d in dirs {
                        if seen.insert((d.clone(), pkg)) {
                            cwd_classes.push((d, pkg));
                        }
                    }
                }
                if !test_users.is_empty() {
                    let d = g.path(e).rsplit_once('/').map_or("", |x| x.0);
                    if seen_tests.insert((d, pkg)) {
                        test_classes.push((d, pkg));
                    }
                }
            }
            // The tasks that run a process for an entry lend it their directories.
            for (&t, r) in &p.runners {
                let pkg = p.home(NodeId(t));
                for d in &r.dirs {
                    if any_cwd && seen.insert((d.clone(), pkg)) {
                        cwd_classes.push((d.clone(), pkg));
                    }
                    if !test_users.is_empty() && seen_tests.insert((d.as_str(), pkg)) {
                        test_classes.push((d.as_str(), pkg));
                    }
                }
            }
        }
        for (users, base) in [
            (cwd_users, EntryBase::Cwd),
            (test_users, EntryBase::TestDir),
        ] {
            if users.is_empty() {
                continue;
            }
            // An entry can load several users, and each names other directories for it:
            // as far up as its own parent steps reach, and without the repository root
            // for a file that only has a record. One pass per kind of user, so that an
            // entry is asked about every kind it loads (one pass over all users would ask
            // only about the nearest).
            let mut tasks_hit: Vec<Seed<(String, String)>> = Vec::new();
            let emit =
                |s: Selected<(String, String)>, done: &mut NodeSet, out: &mut Vec<NamedResult>| {
                    if !done.insert(s.entry) {
                        return;
                    }
                    let (path, dir) = s.hit;
                    let line = s
                        .chain
                        .first()
                        .and_then(|n| users.get(&n.0))
                        .copied()
                        .unwrap_or(0);
                    let dir = if dir.is_empty() {
                        "the repository root".to_owned()
                    } else {
                        format!("{dir}/")
                    };
                    let what = match base {
                        EntryBase::Cwd => "the working directory",
                        EntryBase::TestDir => "the directory of the running test",
                    };
                    out.push(NamedResult {
                        entry: s.entry,
                        witness: s.chain,
                        cause: Cause::Named {
                            path,
                            by: format!("{what}{} ({dir})", of_runner(s.runner)),
                            line,
                            site: g.path(s.site).to_owned(),
                            site_line: p.site_line(s.site),
                        },
                    });
                };
            let mut kinds: std::collections::BTreeMap<(bool, u32), Vec<u32>> =
                std::collections::BTreeMap::new();
            for &n in users.keys() {
                let only_record = base == EntryBase::Cwd && record_only.contains(&n);
                let ups = index.ups.get(&n).copied().unwrap_or(0);
                kinds.entry((only_record, ups)).or_default().push(n);
            }
            for ((only_record, ups), mut seeds) in kinds {
                seeds.sort_unstable();
                let to_user = self.code_ancestors(&p.code, seeds.iter().map(|&n| NodeId(n)), None);
                for (round, (scope, to_site)) in p.rounds().into_iter().enumerate() {
                    // A later round is for the entries whose directory, raised by this kind
                    // of user, reaches a changed path of the round's packages and none of
                    // their own package (those were asked in the first round, and an entry
                    // that reaches no named read there reaches none here).
                    if round > 0 {
                        let below = &scopes.rounds[round - 1];
                        let useful = |d: &str, pkg: &str| {
                            let d = ancestor(d, ups);
                            below.any(d) && !scopes.own(pkg).is_some_and(|own| own.any(d))
                        };
                        let any = match base {
                            EntryBase::Cwd => cwd_classes
                                .iter()
                                .any(|(d, pkg)| !(only_record && d.is_empty()) && useful(d, pkg)),
                            EntryBase::TestDir => {
                                test_classes.iter().any(|(d, pkg)| useful(d, pkg))
                            }
                        };
                        if !any {
                            continue;
                        }
                    }
                    // The names the runner of the process gives it: the entry, or the task
                    // that runs it for the entry.
                    let found = p.reach_by_runner(
                        &to_user,
                        to_site,
                        None,
                        &|e| done.contains(e),
                        &|dirs, home| dirs.iter().any(|d| near_in(home, round, ancestor(d, ups))),
                        &mut |runner, _, _| {
                            let own = [p.home(runner)];
                            let pkgs = scope.unwrap_or(&own);
                            let is_task = p.runners.contains_key(&runner.0);
                            let dirs: Vec<String> = if base == EntryBase::Cwd && only_record {
                                dirs_of(runner)
                                    .into_iter()
                                    .filter(|d| !d.is_empty())
                                    .collect()
                            } else if base == EntryBase::Cwd || is_task {
                                // A task has no test path: its directories stand for it.
                                let d = dirs_of(runner);
                                if d.is_empty() { vec![String::new()] } else { d }
                            } else {
                                vec![
                                    g.path(runner)
                                        .rsplit_once('/')
                                        .map_or("", |x| x.0)
                                        .to_owned(),
                                ]
                            };
                            if !dirs.iter().any(|d| changed_below.any(ancestor(d, ups))) {
                                return None;
                            }
                            // As far up as the files of this kind can step, inside the
                            // package.
                            changes
                                .iter()
                                .filter(|c| p.inside(pkgs, &c.path))
                                .find_map(|c| {
                                    dirs.iter()
                                        .map(|d| ancestor(d, ups))
                                        .find(|d| below(&c.path, d))
                                        .map(|d| (c.path.clone(), d.to_owned()))
                                })
                        },
                        &mut tasks_hit,
                    );
                    for s in found {
                        emit(s, &mut done, &mut out);
                    }
                }
            }
            for s in p.above(&tasks_hit, &|e| done.contains(e)) {
                emit(s, &mut done, &mut out);
            }
            lap("modules that load a working-directory or test-path user");
        }
        // 3. Paths a file builds on the working directory: the directories an entry
        //    runs
        // in, with the file's relative part and what is read there. Files with the same
        // parts share one pass. The directory is where the path goes, whatever package
        // it is in; the parent steps elsewhere in the file raise it only inside
        // the package.
        if !index.cwd_dirs.is_empty() {
            type Parts<'x> = Vec<(&'x str, &'x str)>;
            let mut per_file: std::collections::BTreeMap<u32, (u32, Parts<'_>)> =
                std::collections::BTreeMap::new();
            for (r, rel, pattern) in &index.cwd_dirs {
                per_file
                    .entry(r.node.0)
                    .or_insert((r.line, Vec::new()))
                    .1
                    .push((rel, pattern));
            }
            let mut groups: Vec<(Parts<'_>, Vec<(u32, u32)>)> = Vec::new();
            for (node, (line, mut parts)) in per_file {
                parts.sort_unstable();
                parts.dedup();
                match groups.iter_mut().find(|g| g.0 == parts) {
                    Some(g) => g.1.push((node, line)),
                    None => groups.push((parts, vec![(node, line)])),
                }
            }
            for (parts, users) in groups {
                // As far up as a file of the group can step elsewhere.
                let max_ups = users
                    .iter()
                    .map(|u| index.ups.get(&u.0).copied().unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                let to_user = self.code_ancestors(&p.code, users.iter().map(|u| NodeId(u.0)), None);
                let mut tasks_hit: Vec<Seed<(String, String)>> = Vec::new();
                // `\u{0}` in the name stands for the runner.
                let emit = |s: Selected<(String, String)>,
                            done: &mut NodeSet,
                            out: &mut Vec<NamedResult>| {
                    if !done.insert(s.entry) {
                        return;
                    }
                    let (path, by) = s.hit;
                    let user = s.chain.first().map_or(NONE, |n| n.0);
                    let line = users.iter().find(|u| u.0 == user).map_or(0, |u| u.1);
                    out.push(NamedResult {
                        entry: s.entry,
                        witness: s.chain,
                        cause: Cause::Named {
                            path,
                            by: by.replace('\u{0}', &of_runner(s.runner)),
                            line,
                            site: g.path(s.site).to_owned(),
                            site_line: p.site_line(s.site),
                        },
                    });
                };
                for (round, (scope, to_site)) in p.rounds().into_iter().enumerate() {
                    // A later round is for what the parent steps add: a directory of the
                    // group, raised, that reaches a changed path of the round's packages.
                    if round > 0 {
                        let below = &scopes.rounds[round - 1];
                        let any = max_ups > 0
                            && cwd_classes.iter().any(|(d, _)| {
                                parts.iter().any(|(rel, pattern)| {
                                    *pattern == "**" && {
                                        let base = join_rel(d, rel);
                                        (1..=max_ups).any(|k| below.any(ancestor(&base, k)))
                                    }
                                })
                            });
                        if !any {
                            continue;
                        }
                    }
                    let found = p.reach_by_runner(
                        &to_user,
                        to_site,
                        None,
                        &|e| done.contains(e),
                        &|dirs, home| {
                            dirs.iter().any(|d| {
                                parts.iter().any(|(rel, _)| {
                                    let base = join_rel(d, rel);
                                    changed_below.any(&base)
                                        || (1..=max_ups)
                                            .any(|k| near_in(home, round, ancestor(&base, k)))
                                })
                            })
                        },
                        &mut |runner, chain, _| {
                            let own = [p.home(runner)];
                            let pkgs = scope.unwrap_or(&own);
                            let dirs = {
                                let d = dirs_of(runner);
                                if d.is_empty() { vec![String::new()] } else { d }
                            };
                            let of = '\u{0}';
                            let user = chain.first().map_or(NONE, |n| n.0);
                            // As far up as the file can step elsewhere.
                            let ups = index.ups.get(&user).copied().unwrap_or(0);
                            // The directories the parts are built on, per directory of the
                            // runner and part.
                            let bases: Vec<(String, &str)> = dirs
                                .iter()
                                .flat_map(|d| {
                                    parts
                                        .iter()
                                        .map(move |(rel, pattern)| (join_rel(d, rel), *pattern))
                                })
                                .collect();
                            let near = bases.iter().any(|(base, _)| {
                                changed_below.any(base) || changed_below.any(ancestor(base, ups))
                            });
                            if !near {
                                return None;
                            }
                            changes.iter().find_map(|c| {
                                bases
                                    .iter()
                                    .find_map(|(base, pattern)| {
                                        if *pattern == "**" {
                                            let b = if below(&c.path, base) {
                                                base.as_str()
                                            } else {
                                                ancestor(base, ups)
                                            };
                                            let named = below(&c.path, b)
                                                && (b == base.as_str() || p.inside(pkgs, &c.path));
                                            named.then(|| {
                                                if b.is_empty() {
                                                    format!(
                                                        "the working directory{of} (the \
                                                         repository root)"
                                                    )
                                                } else {
                                                    format!("the working directory{of} ({b}/)")
                                                }
                                            })
                                        } else {
                                            let (dir, name) =
                                                c.path.rsplit_once('/').unwrap_or(("", &c.path));
                                            (dir == base.as_str() && glob_name(pattern, name)).then(
                                                || {
                                                    format!(
                                                        "the working directory{of} ('{pattern}' \
                                                         in {})",
                                                        if base.is_empty() {
                                                            "the repository root"
                                                        } else {
                                                            base.as_str()
                                                        }
                                                    )
                                                },
                                            )
                                        }
                                    })
                                    .map(|by| (c.path.clone(), by))
                            })
                        },
                        &mut tasks_hit,
                    );
                    for s in found {
                        emit(s, &mut done, &mut out);
                    }
                }
                for s in p.above(&tasks_hit, &|e| done.contains(e)) {
                    emit(s, &mut done, &mut out);
                }
            }
            lap("paths on the working directory");
        }
        lap("done");
        if timing.is_some() {
            let s = p.spent.borrow();
            eprintln!(
                "names pass: {} runner task(s); {} search(es) by runner: walks {:.1} ms, entries \
                 {:.1} ms ({} asked), tasks {:.1} ms ({} seen, {} asked); above tasks: {} \
                 walk(s), {:.1} ms",
                p.runners.len(),
                s.calls,
                s.walks.as_secs_f64() * 1e3,
                s.entries.as_secs_f64() * 1e3,
                s.entries_asked,
                s.tasks.as_secs_f64() * 1e3,
                s.tasks_seen,
                s.tasks_asked,
                s.above_calls,
                s.above.as_secs_f64() * 1e3,
            );
        }
        out.sort_by_key(|r| r.entry.0);
        out
    }
}

/// An entry reached from a naming file: the entry, the chain from the naming
/// file to it, and the module that loads both the naming file and a named read.
type Reached = (NodeId, Vec<NodeId>, NodeId);

/// The files that name a changed path, by how far their names reach.
#[derive(Default)]
struct Hits<'c> {
    /// Names that say where they go (a file's location, a path built on it),
    /// and every name when the package assumption is off: they count for
    /// every entry.
    free: HashMap<u32, Hit>,
    /// By package: the files with a string that names a changed path the
    /// package holds.
    scoped: std::collections::BTreeMap<&'c str, HashMap<u32, Hit>>,
}

/// The changed paths of one scope by the directories above them.
#[derive(Default)]
struct Below<'c> {
    /// Every changed path and every directory above one (`""` when there is
    /// any).
    dirs: HashSet<&'c str>,
}

impl<'c> Below<'c> {
    fn add(&mut self, path: &'c str) {
        let path = path.trim_matches('/');
        if path.is_empty() {
            return;
        }
        self.dirs.insert("");
        let mut d = path;
        while self.dirs.insert(d) {
            match d.rsplit_once('/') {
                Some((up, _)) => d = up,
                None => break,
            }
        }
    }

    /// Whether a changed path is `dir` or lies below it.
    fn any(&self, dir: &str) -> bool {
        self.dirs.contains(dir)
    }
}

/// With the package assumption, the changed paths by what may read them: the
/// ones each package holds, and the ones the packages of each round of
/// [`Pairing::rounds`] after the first hold. Without it there is one round and
/// nothing to tell apart.
#[derive(Default)]
struct Scopes<'c> {
    by_package: HashMap<&'c str, Below<'c>>,
    rounds: Vec<Below<'c>>,
}

impl<'c> Scopes<'c> {
    /// The changed paths that a read running for the package at `package` can
    /// read.
    fn own(&self, package: &str) -> Option<&Below<'c>> {
        self.by_package.get(package)
    }
}

/// Packages whose scoped naming files are the same: they share the reachability
/// passes.
struct HitGroup<'c> {
    packages: Vec<&'c str>,
    files: Vec<u32>,
}

/// Most groups of [`Hits::groups`] that get passes of their own; the packages
/// beyond are evaluated together with the union of their naming files (which
/// selects no less).
const MAX_HIT_GROUPS: usize = 24;

/// Most packages with named reads that get a round of their own for the names
/// that depend on the entry ([`Pairing::rounds`]); beyond, one round stands for
/// all of them.
const MAX_SITE_ROUNDS: usize = 8;

impl<'c> Hits<'c> {
    /// The packages of [`Hits::scoped`] by their naming files.
    fn groups(&self) -> Vec<HitGroup<'c>> {
        let mut by_files: std::collections::BTreeMap<Vec<u32>, Vec<&'c str>> =
            std::collections::BTreeMap::new();
        for (pkg, files) in &self.scoped {
            let mut files: Vec<u32> = files.keys().copied().collect();
            files.sort_unstable();
            by_files.entry(files).or_default().push(*pkg);
        }
        let mut groups: Vec<HitGroup<'c>> = by_files
            .into_iter()
            .map(|(files, packages)| HitGroup { packages, files })
            .collect();
        if groups.len() > MAX_HIT_GROUPS {
            // The groups with the most packages keep their own passes.
            groups.sort_by_key(|g| std::cmp::Reverse(g.packages.len()));
            let mut rest = HitGroup {
                packages: Vec::new(),
                files: Vec::new(),
            };
            for g in groups.split_off(MAX_HIT_GROUPS - 1) {
                rest.packages.extend(g.packages);
                rest.files.extend(g.files);
            }
            rest.packages.sort_unstable();
            rest.files.sort_unstable();
            rest.files.dedup();
            groups.push(rest);
        }
        groups
    }
}

/// What the passes of one names query share: the modules that load a named
/// read, the classes of the entries, and the package assumption.
struct Pairing<'a, 'g, G: GraphView> {
    engine: &'a Engine<'g, G>,
    opts: &'a AffectedOptions,
    entries: &'a NodeSet,
    runtime: Option<&'a NodeSet>,
    /// The files with a named record.
    sites: Vec<NodeId>,
    code: Vec<bool>,
    /// The modules that load a file with a named record.
    to_site: Vec<u32>,
    /// Edge filters by entry class: 0 the default edges, 1 runtime entries, 2
    /// build tasks (`crate::taskmode`: a file that only has its bytes read
    /// lends its named reads to nobody).
    passes: Vec<(EdgeFilter, u8)>,
    /// The package assumption; `None`: off.
    packages: Option<Packages>,
    /// With the assumption: the entries by the root of their package.
    by_package: HashMap<&'g str, Vec<NodeId>>,
    /// With the assumption: the files with a named record by the root of their
    /// package.
    sites_by_package: HashMap<&'g str, Vec<NodeId>>,
    /// With the assumption: the packages that hold a changed path and have
    /// files with a named record, with the modules that load one of those
    /// files. One package each, or all of them at once when there are more
    /// than [`MAX_SITE_ROUNDS`].
    site_scopes: Vec<(Vec<&'a str>, Vec<u32>)>,
    /// Per node: a build task, or a shell script (it starts programs in
    /// directories of its own choosing): where the walk from a process to
    /// its runner stops.
    task: Vec<bool>,
    /// The build tasks whose commands run in directories that are known
    /// ([`AffectedOptions::named_by_runner`]; empty when it is off): a process
    /// that an entry reaches only through such a task takes its
    /// entry-relative names from the task.
    runners: HashMap<u32, Runner<'g>>,
    /// Per node: the class of the runner task it is ([`Runner::class`]), else
    /// `NONE`.
    runner_at: Vec<u32>,
    /// One more than the largest class.
    runner_classes: u32,
    /// The packages (by root) of the tasks of `runners`.
    runner_packages: HashSet<&'g str>,
    /// Where the runner passes spend their time
    /// (`TURBO_TRACE_DEBUG_NAMES_TIMING`).
    spent: std::cell::RefCell<Spent>,
}

/// Counters of [`Pairing::reach_by_runner`] and [`Pairing::above`].
#[derive(Default)]
struct Spent {
    calls: u32,
    walks: std::time::Duration,
    entries: std::time::Duration,
    entries_asked: u64,
    tasks: std::time::Duration,
    tasks_seen: u64,
    tasks_asked: u64,
    above_calls: u32,
    above: std::time::Duration,
}

/// A build task that runs processes: where its commands run.
struct Runner<'g> {
    /// Repository relative; `""` is the root.
    dirs: Vec<String>,
    /// The root of its package ([`Pairing::home`]).
    home: &'g str,
    /// The tasks with the same directories and package have the same number:
    /// what a names pass finds for one of them holds for all.
    class: u32,
}

/// An entry that a names pass selects.
struct Selected<H> {
    entry: NodeId,
    /// From the naming file to the entry.
    chain: Vec<NodeId>,
    /// Who runs the module that loads the naming file and the named read, for
    /// the entry: the entry itself, or a build task.
    runner: NodeId,
    /// The file with the named read.
    site: NodeId,
    /// What the pass found for the runner.
    hit: H,
}

/// A build task with an answer: the entries above it are selected through it
/// ([`Pairing::above`]).
struct Seed<H> {
    /// Index into [`Pairing::passes`]: the class of entries it was found for.
    pass: usize,
    task: NodeId,
    /// From the naming file to the task.
    chain: Vec<NodeId>,
    site: NodeId,
    hit: H,
}

/// What a names pass asks about a runner: the runner, the chain from the naming
/// file to it, and the module that loads both the naming file and a named read.
type AskRunner<'x, H> = dyn FnMut(NodeId, &[NodeId], NodeId) -> Option<H> + 'x;

const TASK: u8 = 2;

impl<'g, G: GraphView> Engine<'g, G> {
    /// What the passes of a names query share; `None` when there is nothing to
    /// evaluate.
    fn pairing<'a>(
        &'a self,
        changes: &'a [Change],
        entries: &'a NodeSet,
        runtime: Option<&'a NodeSet>,
        opts: &'a AffectedOptions,
        task_dirs: &dyn Fn(NodeId) -> Option<Vec<String>>,
    ) -> Option<Pairing<'a, 'g, G>> {
        let g = self.graph();
        if changes.is_empty() {
            return None;
        }
        let sites = self.named_sites(opts);
        if sites.is_empty() {
            return None;
        }
        let code = self.code_files();
        let to_site = self.code_ancestors(&code, sites.iter().copied(), None);
        // Like open uncertainty, a named read of a file that a task only reads
        // (compiles, type-checks) says nothing about the task's consumers.
        let all = self.options().traversal.for_uncertainty();
        let runtime_filter = all.without_flags(RefFlags::TYPE_ONLY);
        let mut p = Pairing {
            engine: self,
            opts,
            entries,
            runtime,
            sites,
            code,
            to_site,
            passes: Vec::new(),
            packages: opts.named_in_package.then(|| Packages::of(g)),
            by_package: HashMap::new(),
            sites_by_package: HashMap::new(),
            site_scopes: Vec::new(),
            task: Vec::new(),
            runners: HashMap::new(),
            runner_at: Vec::new(),
            runner_classes: 0,
            runner_packages: HashSet::new(),
            spent: std::cell::RefCell::new(Spent::default()),
        };
        if opts.named_by_runner {
            p.task = (0..g.node_count())
                .map(|i| {
                    crate::taskmode::is_task(g, NodeId(i)) || g.lang(NodeId(i)) == Some("shell")
                })
                .collect();
            let mut classes: HashMap<(Vec<String>, &'g str), u32> = HashMap::new();
            for i in 0..g.node_count() {
                if crate::taskmode::is_task(g, NodeId(i))
                    && let Some(dirs) = task_dirs(NodeId(i))
                {
                    let home = p.home(NodeId(i));
                    let next = classes.len() as u32;
                    let class = *classes.entry((dirs.clone(), home)).or_insert(next);
                    p.runners.insert(i, Runner { dirs, home, class });
                }
            }
            // Only the tasks that can be the nearest one above a module that loads a named
            // read are runners of anything here: the others would only widen what the
            // passes have to consider (their directories, their packages).
            if !p.runners.is_empty() {
                let modules: Vec<NodeId> = (0..g.node_count())
                    .filter(|&i| p.to_site[i as usize] != NONE)
                    .map(NodeId)
                    .collect();
                let (_, nearest) = self
                    .up_from_by_runner(&modules, all, &p.task, &|t| p.runners.contains_key(&t.0));
                let nearest: HashSet<u32> = nearest.into_iter().map(|t| t.0).collect();
                p.runners.retain(|t, _| nearest.contains(t));
            }
            p.runner_at = vec![NONE; g.node_count() as usize];
            for (&t, r) in &p.runners {
                p.runner_at[t as usize] = r.class;
            }
            p.runner_classes = classes.len() as u32;
        }
        let mut passes: Vec<(EdgeFilter, u8)> = vec![(all, 0)];
        if runtime.is_some() && runtime_filter != all {
            passes.push((runtime_filter, 1));
        }
        passes.push((crate::taskmode::filter(all), TASK));
        // Only the passes of the classes that are present.
        passes.retain(|&(_, c)| entries.iter().any(|e| p.class(e) == c));
        p.passes = passes;
        p.runner_packages = p.runners.values().map(|r| r.home).collect();
        if let Some(pk) = &p.packages {
            for e in entries.iter() {
                p.by_package
                    .entry(pk.outermost(package_root(g, e)))
                    .or_default()
                    .push(e);
            }
            for &s in &p.sites {
                p.sites_by_package
                    .entry(pk.outermost(package_root(g, s)))
                    .or_default()
                    .push(s);
            }
            let mut holders: Vec<&str> = changes.iter().flat_map(|c| pk.holders(&c.path)).collect();
            holders.sort_unstable();
            holders.dedup();
            holders.retain(|k| p.sites_by_package.contains_key(*k));
            let scopes: Vec<Vec<&str>> = if holders.len() > MAX_SITE_ROUNDS {
                vec![holders]
            } else {
                holders.into_iter().map(|k| vec![k]).collect()
            };
            for packages in scopes {
                let sites = p.sites_in(&packages);
                let to = self.code_ancestors(&p.code, sites.iter().copied(), None);
                p.site_scopes.push((packages, to));
            }
        }
        Some(p)
    }
}

impl<'a, 'g, G: GraphView> Pairing<'a, 'g, G> {
    fn class(&self, e: NodeId) -> u8 {
        if crate::taskmode::is_task(self.engine.graph(), e) {
            TASK
        } else {
            u8::from(self.runtime.is_some_and(|r| r.contains(e)))
        }
    }

    /// The root of the package of `n` for the assumption: the outermost package
    /// around it other than the root package ([`Packages::outermost`]).
    fn home(&self, n: NodeId) -> &'g str {
        let root = package_root(self.engine.graph(), n);
        match &self.packages {
            Some(pk) => pk.outermost(root),
            None => root,
        }
    }

    /// Whether a read that runs for one of the packages at `packages` can read
    /// `path`: always, without the package assumption.
    fn inside(&self, packages: &[&str], path: &str) -> bool {
        self.packages
            .as_ref()
            .is_none_or(|pk| packages.iter().any(|k| pk.within(k, path)))
    }

    /// Whether a process can run for the package at `package`: some entry
    /// belongs to it, or a task that runs processes.
    fn has_runners(&self, package: &str) -> bool {
        self.by_package.contains_key(package) || self.runner_packages.contains(package)
    }

    /// The files with a named record in the packages at `packages`.
    fn sites_in(&self, packages: &[&str]) -> Vec<NodeId> {
        packages
            .iter()
            .filter_map(|k| self.sites_by_package.get(*k))
            .flatten()
            .copied()
            .collect()
    }

    /// The rounds of an entry-relative name: first every entry, bounded by its
    /// own package (`None`); then, for the packages that hold a changed
    /// path and have named reads, the entries that load one of those reads,
    /// bounded by those packages.
    fn rounds(&self) -> Vec<(Option<&[&'a str]>, &[u32])> {
        let mut out: Vec<(Option<&[&'a str]>, &[u32])> = vec![(None, self.to_site.as_slice())];
        out.extend(
            self.site_scopes
                .iter()
                .map(|(k, to)| (Some(k.as_slice()), to.as_slice())),
        );
        out
    }

    /// The changed paths by what may read them; empty when there is one round
    /// only.
    fn scopes<'c>(&self, changes: &'c [Change]) -> Scopes<'c> {
        let mut scopes = Scopes::default();
        let Some(pk) = &self.packages else {
            return scopes;
        };
        if self.site_scopes.is_empty() {
            return scopes;
        }
        for c in changes {
            for k in pk.holders(&c.path) {
                scopes.by_package.entry(k).or_default().add(&c.path);
            }
        }
        for (packages, _) in &self.site_scopes {
            let mut b = Below::default();
            for c in changes.iter().filter(|c| self.inside(packages, &c.path)) {
                b.add(&c.path);
            }
            scopes.rounds.push(b);
        }
        scopes
    }

    /// The files that name one of `changes`, with their first such name.
    fn hits<'c>(&self, index: &NameIndex, changes: &'c [Change]) -> Hits<'c> {
        let mut hits = Hits::default();
        for c in changes {
            let holders = self.packages.as_ref().map(|pk| pk.holders(&c.path));
            index.named(&c.path, &mut |r, by| {
                // A location and a path built on it say where they go; so does a string
                // that climbs out of where it starts.
                let located = matches!(by, NamedBy::Location(_) | NamedBy::Path(_)) || r.climbs;
                let hit = || Hit {
                    path: c.path.clone(),
                    by: match &by {
                        NamedBy::Shape(s) => format!("the string '{s}'"),
                        NamedBy::Location("") => {
                            "its own location (the repository root)".to_owned()
                        }
                        NamedBy::Location(d) => format!("its own location (below {d}/)"),
                        NamedBy::Path(_) => "a path it builds on its own location".to_owned(),
                        NamedBy::Everything => "more path-like strings than are indexed".to_owned(),
                    },
                    line: r.line,
                };
                match &holders {
                    Some(holders) if !located => {
                        for k in holders {
                            hits.scoped
                                .entry(*k)
                                .or_default()
                                .entry(r.node.0)
                                .or_insert_with(&hit);
                        }
                    }
                    _ => {
                        hits.free.entry(r.node.0).or_insert_with(&hit);
                    }
                }
            });
        }
        hits
    }

    /// The modules that load one of the naming files `files`.
    fn to_names(&self, index: &NameIndex, files: impl Iterator<Item = u32>) -> Vec<u32> {
        let mut naming: Vec<u32> = files.collect();
        naming.sort_unstable();
        self.engine
            .code_ancestors(&self.code, naming.iter().map(|&n| NodeId(n)), Some(index))
    }

    /// The file with a named read that the module `root` loads (`to_site`: the
    /// parent array `root` was found with).
    fn site_node(&self, root: NodeId, to_site: &[u32]) -> NodeId {
        let mut n = root;
        while to_site[n.index()] != n.0 {
            n = NodeId(to_site[n.index()]);
        }
        n
    }

    /// The line of the first named record of the file `site`.
    fn site_line(&self, site: NodeId) -> u32 {
        self.engine
            .graph()
            .uncertainties(site)
            .find(|u| self.engine.disposition_of(site, u, self.opts) == Disposition::Named)
            .map_or(0, |u| u.line)
    }

    /// The entries that reach a module loading both a seed of `to_user` and a
    /// named record of `to_site`. `only`: the entries of the packages at
    /// those roots.
    fn reach(&self, to_user: &[u32], to_site: &[u32], only: Option<&[&str]>) -> Vec<Reached> {
        let wanted: Option<Vec<NodeId>> = only.map(|ks| {
            ks.iter()
                .filter_map(|k| self.by_package.get(*k))
                .flatten()
                .copied()
                .collect()
        });
        let wanted = wanted.as_deref();
        let g = self.engine.graph();
        let mut found = Vec::new();
        if wanted.is_some_and(|es| es.is_empty()) {
            return found;
        }
        let roots: Vec<NodeId> = (0..g.node_count())
            .filter(|&i| to_user[i as usize] != NONE && to_site[i as usize] != NONE)
            .map(NodeId)
            .collect();
        if roots.is_empty() {
            return found;
        }
        for &(filter, pass) in &self.passes {
            if wanted.is_some_and(|es| !es.iter().any(|&e| self.class(e) == pass)) {
                continue;
            }
            let parent = self.engine.up_from(&roots, filter);
            let mut visit = |e: NodeId| {
                if parent[e.index()] == NONE || self.class(e) != pass {
                    return;
                }
                let mut chain = vec![e];
                let mut n = e;
                while parent[n.index()] != n.0 {
                    n = NodeId(parent[n.index()]);
                    chain.push(n);
                }
                let root = n;
                while to_user[n.index()] != n.0 {
                    n = NodeId(to_user[n.index()]);
                    chain.push(n);
                }
                chain.reverse();
                found.push((e, chain, root));
            };
            match wanted {
                Some(es) => es.iter().copied().for_each(&mut visit),
                None => self.entries.iter().for_each(&mut visit),
            }
        }
        found
    }

    /// [`Pairing::reach`] for the names that depend on who runs the process:
    /// the entries that reach a module loading both a seed of `to_user` and
    /// a named record of `to_site`, with `hit` asked about the runner of
    /// that module for the entry.
    ///
    /// The runner is the entry when the entry reaches the module without
    /// crossing a build task (it loads it, however indirectly, into its own
    /// process), and also when the nearest task on the way is one whose
    /// working directory is not known (the answer before runners). Those
    /// entries are returned. Otherwise the module only runs when a
    /// task runs it, and the entry consumes what the task produces: the runner
    /// is that task, the nearest one above the module. The tasks with an
    /// answer are added to `tasks_hit`; [`Pairing::above`] then selects
    /// every entry above one of them. An entry reached both ways is asked
    /// first for itself.
    ///
    /// `only`: the runners of the packages at those roots; `skip(entry)`:
    /// entries that need no answer; `near(directories, package)`: whether a
    /// task of that package that runs in those directories can have an
    /// answer at all (a cheap test: most tasks of a large repository run
    /// far from a change). `hit` gets the runner, the chain from the naming
    /// file to the runner, and the module. A task's answer depends on where
    /// it runs, on its package and on the naming file, so it is asked once
    /// for the tasks that share those.
    #[allow(clippy::too_many_arguments)]
    fn reach_by_runner<H: Clone>(
        &self,
        to_user: &[u32],
        to_site: &[u32],
        only: Option<&[&str]>,
        skip: &dyn Fn(NodeId) -> bool,
        near: &dyn Fn(&[String], &str) -> bool,
        hit: &mut AskRunner<'_, H>,
        tasks_hit: &mut Vec<Seed<H>>,
    ) -> Vec<Selected<H>> {
        let g = self.engine.graph();
        let mut found: Vec<Selected<H>> = Vec::new();
        let roots: Vec<NodeId> = (0..g.node_count())
            .filter(|&i| to_user[i as usize] != NONE && to_site[i as usize] != NONE)
            .map(NodeId)
            .collect();
        if roots.is_empty() {
            return found;
        }
        let mut selected = NodeSet::new(g.node_count());
        // The entries of the packages asked for.
        let wanted: Option<NodeSet> = only.map(|ks| {
            let mut set = NodeSet::new(g.node_count());
            for e in ks.iter().filter_map(|k| self.by_package.get(*k)).flatten() {
                set.insert(*e);
            }
            set
        });
        // The chain from the naming file to `n`, and the module.
        let chain_to = |parent: &[u32], n: NodeId| -> (Vec<NodeId>, NodeId) {
            let mut chain = vec![n];
            let mut n = n;
            while parent[n.index()] != n.0 {
                n = NodeId(parent[n.index()]);
                chain.push(n);
            }
            let root = n;
            while to_user[n.index()] != n.0 {
                n = NodeId(to_user[n.index()]);
                chain.push(n);
            }
            chain.reverse();
            (chain, root)
        };
        let mut spent = self.spent.borrow_mut();
        spent.calls += 1;
        // A task is asked once per class of task and naming file, for every class of
        // entry; the naming file of a module is looked up once.
        let mut asked: HashMap<(u32, u32), Option<H>> = HashMap::new();
        let mut naming: HashMap<u32, u32> = HashMap::new();
        // Per class of task: 1 near (and of a package asked for), 2 not, 0 not looked
        // at.
        let mut near_class = vec![0u8; self.runner_classes as usize];
        for (pi, &(filter, pass)) in self.passes.iter().enumerate() {
            let t0 = std::time::Instant::now();
            let (parent, tasks) = if self.runners.is_empty() {
                (self.engine.up_from(&roots, filter), Vec::new())
            } else {
                self.engine
                    .up_from_by_runner(&roots, filter, &self.task, &|t| {
                        self.runner_at[t.index()] != NONE
                    })
            };
            let t1 = std::time::Instant::now();
            spent.walks += t1 - t0;
            // The entries that run the module themselves.
            for e in self.entries.iter() {
                if parent[e.index()] == NONE
                    || self.class(e) != pass
                    || selected.contains(e)
                    || skip(e)
                    || !wanted.as_ref().is_none_or(|w| w.contains(e))
                {
                    continue;
                }
                let (chain, root) = chain_to(&parent, e);
                spent.entries_asked += 1;
                if let Some(h) = hit(e, &chain, root) {
                    selected.insert(e);
                    found.push(Selected {
                        entry: e,
                        site: self.site_node(root, to_site),
                        chain,
                        runner: e,
                        hit: h,
                    });
                }
            }
            // The tasks that run it.
            let t2 = std::time::Instant::now();
            spent.entries += t2 - t1;
            spent.tasks_seen += tasks.len() as u64;
            for t in tasks {
                let class = self.runner_at[t.index()];
                if class == NONE {
                    continue;
                }
                if near_class[class as usize] == 0 {
                    let ok = self.runners.get(&t.0).is_some_and(|r| {
                        !only.is_some_and(|ks| !ks.contains(&r.home)) && near(&r.dirs, r.home)
                    });
                    near_class[class as usize] = if ok { 1 } else { 2 };
                }
                if near_class[class as usize] == 2 {
                    continue;
                }
                let Some(runner) = self.runners.get(&t.0) else {
                    continue;
                };
                let mut module = t;
                while parent[module.index()] != module.0 {
                    module = NodeId(parent[module.index()]);
                }
                let first = *naming.entry(module.0).or_insert_with(|| {
                    let mut n = module;
                    while to_user[n.index()] != n.0 {
                        n = NodeId(to_user[n.index()]);
                    }
                    n.0
                });
                let h = match asked.get(&(runner.class, first)) {
                    Some(h) => h.clone(),
                    None => {
                        let (chain, root) = chain_to(&parent, t);
                        spent.tasks_asked += 1;
                        let h = hit(t, &chain, root);
                        asked.insert((runner.class, first), h.clone());
                        h
                    }
                };
                if let Some(h) = h {
                    let (chain, root) = chain_to(&parent, t);
                    tasks_hit.push(Seed {
                        pass: pi,
                        task: t,
                        site: self.site_node(root, to_site),
                        chain,
                        hit: h,
                    });
                }
            }
            spent.tasks += t2.elapsed();
        }
        found
    }

    /// The entries above the tasks of `seeds` (the tasks with an answer, from
    /// [`Pairing::reach_by_runner`]): each is selected through the first such
    /// task below it. One walk per class of entry, whatever the number of
    /// seeds. `skip(entry)`: entries that need no answer.
    fn above<H: Clone>(
        &self,
        seeds: &[Seed<H>],
        skip: &dyn Fn(NodeId) -> bool,
    ) -> Vec<Selected<H>> {
        let mut found: Vec<Selected<H>> = Vec::new();
        if seeds.is_empty() {
            return found;
        }
        let g = self.engine.graph();
        let t0 = std::time::Instant::now();
        let mut selected = NodeSet::new(g.node_count());
        for (pi, &(filter, pass)) in self.passes.iter().enumerate() {
            // The first answer of a task stands for it.
            let mut by_task: HashMap<u32, &Seed<H>> = HashMap::new();
            let mut from: Vec<NodeId> = Vec::new();
            for s in seeds.iter().filter(|s| s.pass == pi) {
                if let std::collections::hash_map::Entry::Vacant(v) = by_task.entry(s.task.0) {
                    v.insert(s);
                    from.push(s.task);
                }
            }
            if from.is_empty() {
                continue;
            }
            let up = self.engine.up_from(&from, filter);
            for e in self.entries.iter() {
                if up[e.index()] == NONE || self.class(e) != pass || selected.contains(e) || skip(e)
                {
                    continue;
                }
                let mut tail = vec![e];
                let mut n = e;
                while up[n.index()] != n.0 {
                    n = NodeId(up[n.index()]);
                    tail.push(n);
                }
                let Some(s) = by_task.get(&n.0) else {
                    continue;
                };
                tail.pop();
                tail.reverse();
                let mut chain = s.chain.clone();
                chain.extend(tail);
                selected.insert(e);
                found.push(Selected {
                    entry: e,
                    chain,
                    runner: s.task,
                    site: s.site,
                    hit: s.hit.clone(),
                });
            }
        }
        let mut spent = self.spent.borrow_mut();
        spent.above_calls += 1;
        spent.above += t0.elapsed();
        found
    }
}

/// The root of the package of `n` (`""` for the root package).
fn package_root<G: GraphView>(g: &G, n: NodeId) -> &str {
    let root = g.package_info(g.package(n)).root.trim_matches('/');
    if root == "." { "" } else { root }
}

/// The first-party packages by directory: what "inside the package" means for
/// the package assumption (see the [module documentation](self)).
#[derive(Clone, Debug, Default)]
pub struct Packages {
    /// The roots of the first-party packages other than the root package.
    roots: HashSet<Box<str>>,
}

impl Packages {
    /// The first-party packages of `g`.
    pub fn of<G: GraphView>(g: &G) -> Self {
        let mut roots = HashSet::new();
        for i in 0..g.package_count() {
            let info = g.package_info(crate::view::PackageId(i));
            let root = info.root.trim_matches('/');
            if matches!(info.version, crate::view::PackageVersion::Workspace)
                && !root.is_empty()
                && root != "."
            {
                roots.insert(root.into());
            }
        }
        Packages { roots }
    }

    /// From the package roots (repository relative; the root package needs no
    /// entry).
    pub fn new<S: AsRef<str>>(roots: impl IntoIterator<Item = S>) -> Self {
        let mut out = HashSet::new();
        for r in roots {
            let r = r.as_ref().trim_matches('/');
            if !r.is_empty() && r != "." {
                out.insert(r.into());
            }
        }
        Packages { roots: out }
    }

    /// The root of the nearest package around the file `path` (`""`: the root
    /// package).
    pub fn home<'p>(&self, path: &'p str) -> &'p str {
        let mut dir = path.trim_matches('/');
        loop {
            dir = dir.rsplit_once('/').map_or("", |x| x.0);
            if dir.is_empty() || self.roots.contains(dir) {
                return dir;
            }
        }
    }

    /// The root of the outermost package other than the root package whose
    /// directory is `dir` or lies above it (`""`: none, the root package).
    /// A package inside another one is part of it.
    pub fn outermost<'p>(&self, dir: &'p str) -> &'p str {
        let mut out = "";
        let mut d = dir.trim_matches('/');
        while !d.is_empty() {
            if self.roots.contains(d) {
                out = d;
            }
            d = d.rsplit_once('/').map_or("", |x| x.0);
        }
        out
    }

    /// Whether `path` is a file of the package at `package`: at or below its
    /// directory, and for the root package (`""`) one of its own files, the
    /// root minus the other packages.
    pub fn within(&self, package: &str, path: &str) -> bool {
        if package.is_empty() {
            self.home(path).is_empty()
        } else {
            below(path.trim_matches('/'), package)
        }
    }

    /// The packages `path` is a file of ([`Packages::within`]): every package
    /// around it other than the root package, nearest first, or the root
    /// package alone.
    pub fn holders<'p>(&self, path: &'p str) -> Vec<&'p str> {
        let mut out = Vec::new();
        let mut dir = path.trim_matches('/');
        loop {
            dir = dir.rsplit_once('/').map_or("", |x| x.0);
            if dir.is_empty() {
                break;
            }
            if self.roots.contains(dir) {
                out.push(dir);
            }
        }
        if out.is_empty() {
            out.push("");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(ix: &NameIndex, path: &str) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        ix.named(path, &mut |r, by| {
            out.push((
                r.node.0,
                match by {
                    NamedBy::Shape(s) => s.to_owned(),
                    NamedBy::Location(d) => format!("@{d}"),
                    NamedBy::Path(p) => format!("={p}"),
                    NamedBy::Everything => "*".to_owned(),
                },
            ))
        });
        out.sort();
        out
    }

    #[test]
    fn shapes_match_by_suffix_at_segment_boundaries() {
        let mut ix = NameIndex::new();
        ix.add_shape(NodeId(1), "fixtures/a.json", 3);
        ix.add_shape(NodeId(2), "fixtures", 4);
        ix.add_shape(NodeId(3), "a.json", 5);
        ix.add_shape(NodeId(4), "tures/a.json", 6);
        ix.add_shape(NodeId(5), "pkg/test", 7);
        assert_eq!(
            named(&ix, "pkg/test/fixtures/a.json"),
            [
                (1, "fixtures/a.json".to_owned()),
                (2, "fixtures".to_owned()),
                (3, "a.json".to_owned()),
                (5, "pkg/test".to_owned()),
            ]
        );
        // A directory shape names everything below it, at any depth, and nothing beside
        // it.
        assert_eq!(
            named(&ix, "x/fixtures/deep/er/b.bin"),
            [(2, "fixtures".to_owned())]
        );
        assert_eq!(named(&ix, "x/fixtures2/b.bin"), []);
        assert_eq!(named(&ix, "x/my-fixtures/b.bin"), []);
        // The file itself.
        assert_eq!(named(&ix, "a.json"), [(3, "a.json".to_owned())]);
    }

    #[test]
    fn rests_constrain_what_is_below_the_shape() {
        let fit = |rest: &str, below: &str| {
            let segs: Vec<&str> = below.split('/').filter(|s| !s.is_empty()).collect();
            rest_matches(rest, &segs)
        };
        // `text*`: the next segment starts with the text; anything may follow.
        assert!(fit("configuration*", "configuration.md"));
        assert!(fit("configuration*", "configuration/a/b.md"));
        assert!(fit("configuration*", "configuration-files/x"));
        assert!(!fit("configuration*", "zz-null-dir/zz.bin"));
        assert!(!fit("configuration*", ""));
        // `*/apply`: a hole (any text, with separators or empty), then `apply`.
        assert!(fit("*/apply", "x/apply"));
        assert!(fit("*/apply", "x/y/apply/z.ts"));
        assert!(fit("*/apply", "apply"));
        assert!(fit("*/apply", "x/apply.ts"));
        assert!(!fit("*/apply", "x/apply.ts/y"));
        assert!(!fit("*/apply", "zz-null-dir/zz.bin"));
        // `*text`: some segment ends with the text.
        assert!(fit("*.json", "a.json"));
        assert!(fit("*.json", "a/b.json"));
        assert!(!fit("*.json", "a/b.yaml"));
        // Literal elements are whole segments.
        assert!(fit("a/b", "a/b/c"));
        assert!(!fit("a/b", "a/bc"));
        assert!(!fit("a/b", "a"));
        assert!(fit("*/services/*/apply", "p1/services/s1/apply"));
        assert!(!fit("*/services/*/apply", "p1/services/s1/other"));

        let mut ix = NameIndex::new();
        ix.add_pattern(NodeId(1), "docs", "configuration*", 5);
        ix.add_shape(NodeId(2), "docs", 9);
        assert_eq!(
            named(&ix, "docs/configuration/git.md"),
            [
                (1, "docs/configuration*".to_owned()),
                (2, "docs".to_owned())
            ]
        );
        assert_eq!(named(&ix, "docs/other.md"), [(2, "docs".to_owned())]);
        assert_eq!(
            named(&ix, "apps/site/docs/configuration.md"),
            [
                (1, "docs/configuration*".to_owned()),
                (2, "docs".to_owned())
            ]
        );
    }

    #[test]
    fn extension_completion_needs_two_segments() {
        let mut ix = NameIndex::new();
        ix.add_shape(NodeId(1), "handlers/get", 1);
        ix.add_shape(NodeId(2), "get", 1);
        ix.add_shape(NodeId(3), "handlers/get.test", 1);
        assert_eq!(
            named(&ix, "src/handlers/get.test.ts"),
            [
                (1, "handlers/get".to_owned()),
                (3, "handlers/get.test".to_owned())
            ]
        );
        assert_eq!(named(&ix, "src/handlers/get/index.ts").len(), 2);
        assert_eq!(named(&ix, "src/handlers/getter.ts"), []);
    }

    #[test]
    fn locations_name_what_is_below_them() {
        let mut ix = NameIndex::new();
        ix.add_location(NodeId(1), "pkg/test", 2);
        ix.add_location(NodeId(2), "", 1);
        ix.add_everything(NodeId(3));
        assert_eq!(
            named(&ix, "pkg/test/data/x.bin"),
            [
                (1, "@pkg/test".to_owned()),
                (2, "@".to_owned()),
                (3, "*".to_owned())
            ]
        );
        assert_eq!(
            named(&ix, "pkg/tests/x.bin"),
            [(2, "@".to_owned()), (3, "*".to_owned())]
        );
        assert_eq!(ix.len(), 3);
    }
    #[test]
    fn relative_directories_and_file_name_patterns() {
        assert_eq!(join_rel("packages/config", "../.."), "");
        assert_eq!(join_rel("packages/config", ""), "packages/config");
        assert_eq!(join_rel("packages/config", "../data/x"), "packages/data/x");
        // Above the repository root is the root.
        assert_eq!(join_rel("a", "../../.."), "");
        assert_eq!(join_rel("", "x/./y/../z"), "x/z");
        assert!(glob_name(".env*", ".env"));
        assert!(glob_name(".env*", ".env.test.local"));
        assert!(!glob_name(".env*", "a.env"));
        assert!(glob_name("a*b*c", "aXbYc"));
        assert!(!glob_name("a*b*c", "aXcYb"));
        assert!(glob_name("exact", "exact"));
        assert!(!glob_name("exact", "exactly"));
        assert!(glob_name("ab*b", "abb"));
        assert!(!glob_name("ab*b", "ab"));
        assert!(glob_name("*", "anything"));
    }

    #[test]
    fn packages_hold_their_directory_and_the_root_its_own_files() {
        let pk = Packages::new(["services/a", "services/a/tools", "packages/lib/"]);
        assert_eq!(pk.home("services/a/src/x.ts"), "services/a");
        assert_eq!(pk.home("services/a/tools/x.ts"), "services/a/tools");
        assert_eq!(pk.home("services/b/x.ts"), "");
        assert_eq!(pk.home("services/a.ts"), "");
        assert_eq!(pk.home("x.ts"), "");
        // A package holds everything at or below its directory, packages inside it too.
        assert!(pk.within("services/a", "services/a/src/x.ts"));
        assert!(pk.within("services/a", "services/a/tools/x.ts"));
        assert!(!pk.within("services/a", "services/ab/x.ts"));
        assert!(!pk.within("services/a/tools", "services/a/src/x.ts"));
        // The root package: the root without the other packages.
        assert!(pk.within("", "docs/x.md"));
        assert!(pk.within("", "services/README.md"));
        assert!(!pk.within("", "services/a/src/x.ts"));
        assert_eq!(
            pk.holders("services/a/tools/x.ts"),
            ["services/a/tools", "services/a"]
        );
        assert_eq!(pk.holders("services/a/x.ts"), ["services/a"]);
        // A package inside another one is part of it.
        assert_eq!(pk.outermost("services/a/tools"), "services/a");
        assert_eq!(pk.outermost("services/a"), "services/a");
        assert_eq!(pk.outermost("services/b"), "");
        assert_eq!(pk.outermost(""), "");
        assert_eq!(pk.holders("docs/x.md"), [""]);
        // No packages: everything is the root package's.
        let none = Packages::default();
        assert!(none.within("", "any/where.ts"));
        assert_eq!(none.holders("any/where.ts"), [""]);
    }

    #[test]
    fn exact_paths_are_named() {
        let mut ix = NameIndex::new();
        ix.add_path(NodeId(3), ".env.test", 7);
        ix.add_path(NodeId(4), "services/a/.env", 2);
        assert_eq!(named(&ix, ".env.test"), [(3, "=.env.test".to_owned())]);
        assert_eq!(
            named(&ix, "services/a/.env"),
            [(4, "=services/a/.env".to_owned())]
        );
        // Not what is below or beside it.
        assert!(named(&ix, "sub/.env.test").is_empty());
        assert!(named(&ix, "services/a/.env/x").is_empty());
    }
}
