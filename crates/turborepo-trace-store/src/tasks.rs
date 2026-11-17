//! Build tasks in the graph.
//!
//! During a build, every dirty first-party package's tasks are discovered and
//! recognized with [`turborepo_trace_tasks`] (inside the tracer's recording dir
//! cache, so config discovery probes and config reads are recorded). Each task
//! becomes a pseudo-file of the package's shard, keyed and invalidated with the
//! shard like the framework inputs:
//!
//! | Task part | Stored as |
//! |---|---|
//! | the task | a pseudo-file `#<task id>` with language `task`; the composed graph names its node by the id (`task:apps/web#build`) |
//! | entries (scripts, build entry points, code configs) | edges to the files (their closures follow through the graph); missing ones are probes |
//! | inputs | edges to the files or directories; missing ones are probes. Inputs the task only reads (`TaskSpec::data_inputs`) are data edges, like the hubs of read-only memberships below |
//! | configs, and the files discovery and recognition read (`package.json`, `turbo.json`, the tsconfig chain) | config inputs (contents in the shard key; inputs of the task in queries) |
//! | memberships (turbo `inputs`, tsconfig `include`, the package directory for unknown commands) | one shared hub pseudo-file per distinct membership (`#members:<dir>/<pattern>`, language `task-members`) with an edge to every walked file that matches it now and the listing as a membership (an added or deleted match changes it); the task has an edge to the hub |
//! | toolchain npm packages | edges resolved like `require('<name>/package.json')` from the package (opaque package nodes, lockfile-diffed) |
//! | task dependencies | ids in [`crate::shard::TaskRec::deps`]; the composed graph adds an edge to each task node that exists. The `package.json` files consulted to resolve them are config inputs; for `^task`, the transitive dependencies' manifests are path-only config inputs ([`crate::shard::Shard::path_configs`]) and the package's dependency closure is keyed instead ([`crate::shard::Shard::dep_closures`]) |
//! | outputs, env names | [`crate::shard::TaskRec`]; the composed graph links every node covered by an output, and every node that probes, reads or lists a covered path, to the task (generated-file linking) |
//! | parts ([`turborepo_trace_tasks::spec::TaskPart`]: the tsconfig projects of a `tsc -b` build) | one pseudo-file per part (`#tsproject:<tsconfig>`, language `task-part`) with the part's inputs, configs, membership hubs and toolchain, and an edge to every part it references. The same part in several packages (every `tsc -b` that builds the project) is one node of the composed graph. Outputs tagged with a part ([`crate::shard::PartRec`]) link to the part's node, not to the task: an importer of `packages/lib/dist/index.js` depends on the `packages/lib` project, not on every project of every solution that builds it |
//! | output packages (npm) | one output per `<dir>/node_modules/<name>` location from the package root up to the repo root, marked with the package name; the composed graph also links the package's opaque node and replaces bare-import uncertainties of linked nodes |
//! | uncertainties | `unknown_command` / `shell_unsupported` uncertainties on the pseudo-file (every task uncertainty comes with the package membership, so they are not counted by `affected` by default) |
//!
//! Pseudo-files are not part of the shard key's entry list; their observations
//! are.
//!
//! A package **without any task** has no pseudo-file to carry what task
//! discovery looked at (the `.github/workflows` listing, the task-file listings
//! and probes of every source, the manifest). Those observations stay in the
//! shard as key-only table entries ([`crate::normalize::add_key_only`]): the
//! package's first workflow, Makefile or script re-keys the shard, so a
//! refreshed graph has the tasks a fresh build has. `tests/tasks_refresh.rs`
//! checks refreshed against fresh for every task fixture.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use turborepo_trace_core::{
    AnalyzedEdge, Edge, FileAnalysis, Phase, RefFlags, RefKind, Span, Target, Tracer, Uncertainty,
    UncertaintyKind,
    record::{self, Observations},
};
use turborepo_trace_frameworks::fsx;
use turborepo_trace_tasks::{
    Options, Task,
    registry::Recognition,
    sources::{Workspace as TaskWorkspace, turbo as turbo_config},
    spec::{Membership, TaskId, TaskRef},
};

use crate::{
    paths::rel_to,
    shard::{TaskCommandRec, TaskOutputRec, TaskRec},
    workspace::Workspace,
};

/// Language id of task pseudo-files.
pub const TASK_LANG: &str = "task";
/// Language id of membership hub pseudo-files.
pub const MEMBERS_LANG: &str = "task-members";
/// Language id of task part pseudo-files (tsconfig projects of `tsc -b`
/// builds).
pub const PART_LANG: &str = "task-part";
/// Version of the task integration, part of the options digest.
pub const TASKS_VERSION: &str = "tasks-11";

/// The task workspace of one build, with name and root indexes for dependency
/// closures.
pub(crate) struct TaskGraph {
    pub ws: TaskWorkspace,
    by_name: HashMap<String, usize>,
}

impl TaskGraph {
    pub(crate) fn new(
        repo: &Path,
        ws: &Workspace,
        manifests: &crate::smallfiles::SmallFiles,
    ) -> Self {
        // Unrecorded: each package's own manifest is part of its key, and what other
        // manifests contribute to task resolution is keyed explicitly (dependency
        // closures, config inputs). Read through the build's manifest cache
        // (already read by discovery).
        let read = |p: &Path| -> Option<serde_json::Value> {
            serde_json::from_slice(&manifests.read(p)?).ok()
        };
        let ws =
            TaskWorkspace::from_roots_with(repo, ws.packages.iter().map(|p| p.root.clone()), &read);
        let mut by_name = HashMap::new();
        for (i, p) in ws.packages.iter().enumerate() {
            if let Some(n) = &p.name {
                by_name.entry(n.clone()).or_insert(i);
            }
        }
        TaskGraph { ws, by_name }
    }

    /// The package with name `name`, else root `name` (as
    /// [`TaskWorkspace::package`]).
    fn package_ix(&self, name: &str) -> Option<usize> {
        if let Some(&i) = self.by_name.get(name) {
            return Some(i);
        }
        let r = turborepo_trace_tasks::sources::norm_rel(name);
        self.ws
            .packages
            .binary_search_by(|p| p.rel.as_str().cmp(r.as_str()))
            .ok()
    }

    /// The workspace packages the package at `rel` depends on transitively
    /// (excluding itself), sorted by root:
    /// [`TaskWorkspace::transitive_deps`] with hash lookups.
    /// `None` if `rel` is not a package.
    pub(crate) fn closure(&self, rel: &str) -> Option<Vec<usize>> {
        let own = self
            .ws
            .packages
            .binary_search_by(|p| p.rel.as_str().cmp(rel))
            .ok()?;
        let pkg = &self.ws.packages[own];
        let mut out = Vec::new();
        let mut todo: Vec<&str> = pkg.deps.iter().map(String::as_str).collect();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        while let Some(n) = todo.pop() {
            if !seen.insert(n) {
                continue;
            }
            if let Some(i) = self.package_ix(n)
                && self.ws.packages[i].rel != pkg.rel
            {
                out.push(i);
                todo.extend(self.ws.packages[i].deps.iter().map(String::as_str));
            }
        }
        // Packages are sorted by root.
        out.sort_unstable();
        out.dedup();
        Some(out)
    }
}

/// Whether a walked file is a third-party package's file (the target
/// classification of [`crate::build`] links it to the package, not by path).
pub(crate) type ThirdPartyFn<'a> = &'a (dyn Fn(&Path) -> bool + Sync);

/// The members of one hub that keep stored edges (third-party files), per
/// distinct membership `(dir, pattern, excludes)`.
type HubMemo = std::sync::Mutex<HashMap<HubKey, std::sync::Arc<Vec<PathBuf>>>>;

/// A membership hub's identity: `(directory (absolute), pattern, excludes, own
/// files only)`.
type HubKey = (PathBuf, String, Vec<String>, bool);

/// [`HubKey`] by reference.
struct HubView<'a> {
    dir: &'a Path,
    pattern: &'a str,
    exclude: &'a [String],
    own: bool,
}

/// Task discovery for one build.
pub(crate) struct Tasks<'a> {
    tracer: &'a Tracer,
    repo: &'a Path,
    /// Walked files, repo-relative, sorted.
    files: &'a [String],
    graph: &'a TaskGraph,
    ws: &'a TaskWorkspace,
    third_party: ThirdPartyFn<'a>,
    hub_memo: HubMemo,
    /// [`Tasks::hub_state`] per hub.
    hub_states: std::sync::Mutex<HashMap<HubKey, u128>>,
    /// [`Tasks::third_party_files`].
    third_party_files: std::sync::OnceLock<Vec<String>>,
    /// The package roots, for hubs of a package's own files ([`crate::own`]).
    nested: crate::own::Nested,
}

/// A package's tasks as pseudo-file analyses (path relative to the package
/// root, analysis) and records (with `file` relative to the first pseudo-file;
/// the caller offsets it).
pub(crate) struct PackageTasks {
    pub files: Vec<(String, FileAnalysis, &'static str)>,
    pub recs: Vec<TaskRec>,
    /// Package roots whose dependency closure the tasks resolved
    /// ([`crate::shard::Shard::dep_closures`]), sorted.
    pub dep_closures: Vec<String>,
    /// Membership hubs: the pseudo-file path (relative to the package root) and
    /// the record (its `file` is set by the caller).
    pub hubs: Vec<(String, crate::shard::HubRec)>,
    /// Task parts: the pseudo-file path (relative to the package root) and the
    /// record (its `file` is set by the caller).
    pub parts: Vec<(String, crate::shard::PartRec)>,
    /// Files whose contents key the shard without being an input of any
    /// pseudo-file: turbo configs (the tasks' inputs are their sections,
    /// see [`turborepo_trace_tasks::sources::turbo`]).
    pub key_reads: Vec<PathBuf>,
    /// Key-only observations of the shard ([`crate::normalize::add_key_only`]):
    /// what decides which tasks the package has without being an input of
    /// one.
    ///
    /// - In a package where task discovery found **no task**: the listings,
    ///   probes and reads of every task source (the `.github/workflows`
    ///   listing, Makefile and task-file probes, the manifest), so the
    ///   package's first task file re-keys it. With tasks, every task
    ///   pseudo-file carries them.
    /// - What a source computed its tasks from beyond its config files
    ///   ([`turborepo_trace_tasks::Task::decided_by`]: the directories a Python
    ///   project's package discovery lists, the paths it asks about). A file
    ///   that no compiled language claims (an `__init__.py` without the Python
    ///   language) is no entry of the shard, so this is what makes a package
    ///   that comes or goes re-key it.
    pub discovery: Observations,
}

/// The repo-relative form of `p` (`""` for the root).
fn repo_rel(repo: &Path, p: &Path) -> Option<String> {
    p.strip_prefix(repo)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

impl<'a> Tasks<'a> {
    pub(crate) fn new(
        tracer: &'a Tracer,
        repo: &'a Path,
        files: &'a [String],
        graph: &'a TaskGraph,
        third_party: ThirdPartyFn<'a>,
    ) -> Self {
        Tasks {
            tracer,
            repo,
            files,
            graph,
            ws: &graph.ws,
            third_party,
            hub_memo: Default::default(),
            hub_states: Default::default(),
            third_party_files: Default::default(),
            nested: crate::own::Nested::new(graph.ws.packages.iter().map(|p| p.rel.clone())),
        }
    }

    fn edge(
        &self,
        from: &Path,
        kind: RefKind,
        spec: String,
        succ: Vec<turborepo_trace_core::FileId>,
        links: Vec<turborepo_trace_core::FileId>,
        uncertainty: Option<Uncertainty>,
    ) -> AnalyzedEdge {
        AnalyzedEdge {
            edge: Edge {
                from: self.tracer.intern(from),
                to: succ
                    .first()
                    .map_or(Target::Unresolved, |&f| Target::File(f)),
                kind,
                phase: Phase::Build,
                flags: RefFlags::empty(),
                span: Span::DUMMY,
                spec: spec.into(),
                uncertainty,
            },
            succ,
            links,
        }
    }

    /// `e` as a data edge ([`RefFlags::DATA`]): the task or part reads or
    /// compiles the target as an input and does not execute it, so the
    /// target's uncertainty does not propagate to it (changes do).
    fn data(mut e: AnalyzedEdge) -> AnalyzedEdge {
        e.edge.flags |= RefFlags::DATA;
        e
    }

    /// Whether a path a task names (an entry, an input, a config) is a host
    /// path: outside the repository and the languages' environment roots
    /// ([`DirCache::is_host_path`]), as the program `/usr/bin/env`,
    /// `~/.npmrc` or a file in `/tmp`. Whether it exists depends
    /// on the machine, and it can never be a changed file: it is not looked up
    /// and gets neither an edge nor a probe.
    ///
    /// [`DirCache::is_host_path`]: turborepo_trace_core::DirCache::is_host_path
    fn host(&self, p: &Path) -> bool {
        self.tracer.fs_arc().is_host_path(p)
    }

    /// The paths to look up for a path that a task names (an entry, an input, a
    /// config), each with the specifier of its edge when it is not the path
    /// itself:
    ///
    /// - an absolute path that a command of the task writes out (`written`,
    ///   [`turborepo_trace_tasks::spec::TaskSpec::written`]): the files of the
    ///   repository that its trailing components name, the repository seen from
    ///   another place ([`turborepo_trace_core::fs::seen_from_elsewhere`]). It
    ///   is not looked up where it lands, also when that is inside this
    ///   checkout: the task depends on the same files wherever the repository
    ///   is checked out, like the script that holds the command;
    /// - a host path ([`Tasks::host`]): nothing;
    /// - a path inside the repository: itself.
    ///
    /// A path of the list that does not exist is a probe.
    fn named(&self, p: &Path, written: &[PathBuf]) -> Vec<(PathBuf, Option<String>)> {
        if let Some(text) = p.to_str().filter(|_| written.iter().any(|w| w == p)) {
            return turborepo_trace_core::fs::seen_from_elsewhere(None, text)
                .into_iter()
                .map(|e| {
                    let label = format!(
                        "{}{}",
                        turborepo_trace_core::output::ELSEWHERE_PREFIX,
                        e.sentence(text, &e.rel)
                    );
                    (self.repo.join(&e.rel), Some(label))
                })
                .collect();
        }
        if self.host(p) {
            return Vec::new();
        }
        vec![(p.to_path_buf(), None)]
    }

    /// A task's `inputs` (then its `data_inputs`, flagged `true`) in an order
    /// that does not depend on where the checkout sits: the files of the
    /// repository by their path from the root, then the paths outside it by
    /// their absolute path. (`TaskSpec::normalize` sorts by absolute path,
    /// and an outside path such as `/app/notes.txt` would sort before the
    /// checkout's files at one place and after them at another: the edges and
    /// the shard's bytes would differ between places.) For a task whose
    /// inputs are all inside the repository this is the order of the spec.
    fn in_place_order<'s>(
        &self,
        inputs: &'s [PathBuf],
        data_inputs: &'s [PathBuf],
    ) -> Vec<(&'s PathBuf, bool)> {
        let sorted = |v: &'s [PathBuf], data: bool| {
            let mut v: Vec<_> = v.iter().map(|i| (i, data)).collect();
            v.sort_by(|a, b| self.place_key(a.0).cmp(&self.place_key(b.0)));
            v
        };
        let mut out = sorted(inputs, false);
        out.extend(sorted(data_inputs, true));
        out
    }

    /// The key that orders the paths of a task the same at every place
    /// ([`Tasks::in_place_order`]): inside the repository the path from the
    /// root, then outside it the absolute path.
    fn place_key<'p>(&self, p: &'p Path) -> (bool, &'p Path) {
        match p.strip_prefix(self.repo) {
            Ok(rel) => (false, rel),
            Err(_) => (true, p),
        }
    }

    /// `items` in the order of [`Tasks::place_key`] of `path` (stable: items of
    /// one path keep the spec's order).
    fn by_place<'s, T>(&self, items: &'s [T], path: impl Fn(&T) -> &Path) -> Vec<&'s T> {
        let mut v: Vec<&T> = items.iter().collect();
        v.sort_by(|a, b| self.place_key(path(a)).cmp(&self.place_key(path(b))));
        v
    }

    fn display(&self, p: &Path) -> String {
        repo_rel(self.repo, p)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| p.to_string_lossy().into_owned())
    }

    /// The pseudo-file path of the task part `kind` at `path`
    /// (`#tsproject:packages/lib/tsconfig.json`): the same in every package
    /// that has it.
    fn part_rel(&self, kind: &str, path: &Path) -> String {
        format!("#{kind}:{}", self.display(path))
    }

    /// The members of hub `m` whose edges are stored: walked files that
    /// classify as third-party, sorted. Memoized per distinct membership
    /// (the same repo-wide hub in many packages).
    fn hub_special(&self, m: &Membership) -> std::sync::Arc<Vec<PathBuf>> {
        self.hub_special_of(&(m.dir.clone(), m.pattern.clone(), m.exclude.clone(), m.own))
    }

    /// [`Tasks::hub_special`] of the hub `(directory, pattern, excludes, own
    /// files only)`.
    fn hub_special_of(&self, key: &HubKey) -> std::sync::Arc<Vec<PathBuf>> {
        if let Some(v) = self
            .hub_memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
        {
            return std::sync::Arc::clone(v);
        }
        // Among the walked files that are third-party files at all (usually none): a
        // repository-wide hub does not classify every file of the repository again.
        let v: std::sync::Arc<Vec<PathBuf>> =
            std::sync::Arc::new(self.members(key, self.third_party_files()));
        self.hub_memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_insert(v)
            .clone()
    }

    /// The listings a hub of a package's own files records instead of its
    /// membership (`crate::own`), or `None` for any other hub: it records
    /// `(dir, pattern)`.
    fn own_listings(
        &self,
        dir: &str,
        pattern: &str,
        exclude: &[String],
        own: bool,
    ) -> Option<Vec<(String, &'static str)>> {
        (own && pattern == "**" && exclude.is_empty())
            .then(|| self.nested.listings(dir, self.files))
    }

    /// The digest of everything a shard stores that depends on **which files**
    /// the hub `(dir, pattern, exclude, own)` matches now (`dir`
    /// repo-relative, `""` for the root): the members whose edges are
    /// stored ([`crate::shard::HubRec::skip`]: third-party files) and, for
    /// a hub of a package's own files, the listings that cover them. The
    /// rest of the member list is not in the shard: the composed graph expands
    /// it from the snapshot's walked-file list.
    ///
    /// [`Tasks::package_tasks`] builds a hub's records from the same two
    /// functions, so a shard whose hubs have this state holds the hub
    /// records a fresh trace would write: this, not the listing, is what
    /// the shard key covers of a listing that only hubs
    /// reference ([`crate::shard::Shard::hub_members`]).
    pub(crate) fn hub_state(
        &self,
        dir: &str,
        pattern: &str,
        exclude: &[String],
        own: bool,
    ) -> u128 {
        let abs = if dir.is_empty() {
            self.repo.to_path_buf()
        } else {
            self.repo.join(dir)
        };
        let key: HubKey = (abs, pattern.to_owned(), exclude.to_vec(), own);
        if let Some(d) = self
            .hub_states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return *d;
        }
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        let mut put = |s: &str| {
            h.update(&(s.len() as u64).to_le_bytes());
            h.update(s.as_bytes());
        };
        for f in self.hub_special_of(&key).iter() {
            put("skip");
            put(&repo_rel(self.repo, f).unwrap_or_default());
        }
        for (d, p) in self
            .own_listings(dir, pattern, exclude, own)
            .unwrap_or_default()
        {
            put("listing");
            put(&d);
            put(p);
        }
        let d = h.digest128();
        self.hub_states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, d);
        d
    }

    /// The walked files that are files of a third-party package (repo-relative,
    /// sorted): vendored modules and the like. Classified once per build,
    /// on first use.
    fn third_party_files(&self) -> &[String] {
        use rayon::prelude::*;
        crate::once::init_once(&self.third_party_files, || {
            self.files
                .par_iter()
                .filter(|f| (self.third_party)(&self.repo.join(f)))
                .cloned()
                .collect()
        })
    }

    /// The files among `files` (walked files, repo-relative, sorted) that match
    /// the hub `key` (absolute paths).
    fn members(&self, key: &HubKey, files: &[String]) -> Vec<PathBuf> {
        let m = HubView {
            dir: &key.0,
            pattern: &key.1,
            exclude: &key.2,
            own: key.3,
        };
        let Some(dir) = repo_rel(self.repo, m.dir) else {
            return Vec::new();
        };
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let start = files.partition_point(|f| f.as_str() < prefix.as_str());
        // Compiled once per membership (the root package's hubs see every walked file).
        let pat = crate::glob::FastGlob::new(m.pattern);
        let exc: Vec<crate::glob::FastGlob> = m
            .exclude
            .iter()
            .map(|e| crate::glob::FastGlob::new(e))
            .collect();
        let mut out = Vec::new();
        for f in &files[start..] {
            let Some(rel) = f.strip_prefix(prefix.as_str()) else {
                break;
            };
            if pat.matches(rel)
                && !exc.iter().any(|e| e.matches(rel))
                && !(m.own && self.nested.inside(&dir, f))
            {
                out.push(self.repo.join(f));
            }
        }
        out
    }

    /// Discovers, recognizes and converts the tasks of the package rooted at
    /// `root` (repo-relative, `""` for the root).
    pub(crate) fn package_tasks(&self, root: &str, root_abs: &Path) -> PackageTasks {
        let rel = if root.is_empty() {
            ".".to_string()
        } else {
            root.to_string()
        };
        let mut out = PackageTasks {
            files: Vec::new(),
            recs: Vec::new(),
            dep_closures: Vec::new(),
            hubs: Vec::new(),
            parts: Vec::new(),
            key_reads: Vec::new(),
            discovery: Observations::default(),
        };
        // Packages are sorted by `rel`.
        let Some(pkg) = self
            .ws
            .packages
            .binary_search_by(|p| p.rel.as_str().cmp(rel.as_str()))
            .ok()
            .map(|i| &self.ws.packages[i])
        else {
            return out;
        };
        let frame = record::start();
        let tasks: Vec<Task> = fsx::with_fs(self.tracer.fs_arc(), || {
            turborepo_trace_tasks::analyze_package(self.ws, pkg, &Options::default())
        });
        let shared_obs = frame.finish();
        for t in &tasks {
            out.discovery.extend_from(&t.decided_by);
        }
        if tasks.is_empty() {
            // Nothing to attach the observations to: the shard keeps them key-only.
            out.discovery = shared_obs;
            return out;
        }
        // Toolchain npm packages, resolved once per package.
        let mut npm_tools: BTreeSet<String> = BTreeSet::new();
        for t in &tasks {
            let part_tools = t.spec.parts.iter().flat_map(|p| p.toolchain.iter());
            for tc in t.spec.toolchain.iter().chain(part_tools) {
                if tc.ecosystem == "npm" && is_npm_name(&tc.name) {
                    npm_tools.insert(tc.name.clone());
                }
            }
        }
        let tool_edges = self.resolve_tools(root_abs, &npm_tools);
        // Task parts, one per distinct (kind, path) in the package: the first task's
        // (the same project read by another task gives the same part).
        let mut parts: BTreeMap<String, &turborepo_trace_tasks::spec::TaskPart> = BTreeMap::new();
        for t in &tasks {
            for p in &t.spec.parts {
                parts.entry(self.part_rel(&p.kind, &p.path)).or_insert(p);
            }
        }
        // Membership hubs.
        let mut hubs: BTreeMap<Membership, String> = BTreeMap::new();
        let task_memberships = tasks.iter().flat_map(|t| t.spec.memberships.iter());
        let part_memberships = parts.values().flat_map(|p| p.memberships.iter());
        for m in task_memberships.chain(part_memberships) {
            hubs.entry(m.key()).or_insert_with(|| {
                let mut name = match repo_rel(self.repo, &m.dir) {
                    Some(d) if d.is_empty() => m.pattern.clone(),
                    _ => format!("{}/{}", self.display(&m.dir), m.pattern),
                };
                for e in &m.exclude {
                    name.push_str(" !");
                    name.push_str(e);
                }
                if m.own {
                    // The package's own files: nested packages are not members.
                    name.push_str(" (own files)");
                }
                format!("#members:{name}")
            });
        }
        // Each hub's edges are the walked files below its directory that match: the
        // composed graph expands them from the walk (`HubRec`), so only the members
        // that classify as third-party files keep stored edges. Scanned once
        // per distinct membership in the build, in parallel, in map order.
        let hub_files: Vec<(String, FileAnalysis, &'static str)> = {
            use rayon::prelude::*;
            let hubs: Vec<(&Membership, &String)> = hubs.iter().collect();
            let parts: Vec<_> = hubs
                .par_iter()
                .map(|(m, hub_rel)| {
                    let path = root_abs.join(hub_rel);
                    let mut obs = Observations::default();
                    // The own files of a package: the listings that cover them and not the
                    // nested packages (`crate::own`). What the hub's records take from the
                    // tree is this and `hub_special`: [`Tasks::hub_state`].
                    match repo_rel(self.repo, &m.dir)
                        .and_then(|dir| self.own_listings(&dir, &m.pattern, &m.exclude, m.own))
                    {
                        Some(listings) => {
                            for (d, pattern) in listings {
                                obs.listings.push((self.repo.join(d), pattern.to_owned()));
                            }
                        }
                        None => obs.listings.push((m.dir.clone(), m.pattern.clone())),
                    }
                    let special = self.hub_special(m);
                    let rec = repo_rel(self.repo, &m.dir).map(|dir| crate::shard::HubRec {
                        file: 0,
                        dir,
                        pattern: m.pattern.clone(),
                        exclude: m.exclude.clone(),
                        skip: special
                            .iter()
                            .filter_map(|f| repo_rel(self.repo, f))
                            .collect(),
                        own: m.own,
                    });
                    let edges: Vec<AnalyzedEdge> = special
                        .iter()
                        .map(|f| {
                            let id = self.tracer.intern(f);
                            self.edge(
                                &path,
                                RefKind::FsRead,
                                String::new(),
                                vec![id],
                                Vec::new(),
                                None,
                            )
                        })
                        .collect();
                    (
                        ((*hub_rel).clone(), analysis(path, edges, obs), MEMBERS_LANG),
                        rec.map(|r| ((*hub_rel).clone(), r)),
                    )
                })
                .collect();
            let mut files = Vec::with_capacity(parts.len());
            for (f, r) in parts {
                files.push(f);
                out.hubs.extend(r);
            }
            files
        };
        out.files.extend(hub_files);
        // Part pseudo-files: the part's inputs as edges and observations, like a
        // task's, but nothing of the tasks that contain it (their manifests,
        // `turbo.json`, dependencies).
        for (rel_path, p) in &parts {
            let path = root_abs.join(rel_path);
            let mut obs = p.observations.clone();
            let mut edges = Vec::new();
            for (i, data) in self.in_place_order(&p.inputs, &p.data_inputs) {
                if self.host(i) {
                    continue;
                }
                if fsx::exists(i) {
                    let id = self.tracer.intern(i);
                    let e = self.edge(
                        &path,
                        RefKind::FsRead,
                        format!("input {}", self.display(i)),
                        vec![id],
                        Vec::new(),
                        None,
                    );
                    // A file the part only reads (a type-checked project's `files`).
                    edges.push(if data { Self::data(e) } else { e });
                } else {
                    obs.missing.push(i.clone());
                }
            }
            for c in self.by_place(&p.configs, |c| c.as_path()) {
                if self.host(c) {
                    continue;
                }
                if fsx::is_file(c) {
                    obs.reads.push(c.clone());
                } else {
                    obs.missing.push(c.clone());
                }
            }
            for m in &p.memberships {
                if let Some(h) = hubs.get(&m.key()) {
                    let id = self.tracer.intern(&root_abs.join(h));
                    let e = self.edge(
                        &path,
                        RefKind::FsRead,
                        format!("{} ({})", &h[1..], m.reason),
                        vec![id],
                        Vec::new(),
                        None,
                    );
                    edges.push(if m.data { Self::data(e) } else { e });
                }
            }
            for tc in &p.toolchain {
                if let Some((succ, links, tobs)) =
                    tool_edges.get(&tc.name).filter(|_| tc.ecosystem == "npm")
                {
                    edges.push(self.edge(
                        &path,
                        RefKind::Require,
                        tc.name.clone(),
                        succ.clone(),
                        links.clone(),
                        None,
                    ));
                    obs.extend_from(tobs);
                }
            }
            for d in &p.deps {
                let dep = self.part_rel(&p.kind, d);
                if parts.contains_key(&dep) {
                    let id = self.tracer.intern(&root_abs.join(&dep));
                    edges.push(Self::data(self.edge(
                        &path,
                        RefKind::Import,
                        format!("references {}", self.display(d)),
                        vec![id],
                        Vec::new(),
                        None,
                    )));
                }
            }
            obs.dedup();
            // Outputs that link (a part of a persistent or CI task alone links nothing).
            let mut outputs: Vec<TaskOutputRec> = Vec::new();
            for t in &tasks {
                if !t
                    .spec
                    .parts
                    .iter()
                    .any(|q| q.kind == p.kind && q.path == p.path)
                {
                    continue;
                }
                for o in self.by_place(&t.spec.outputs, |o| o.path.as_path()) {
                    if o.part.as_deref() != Some(p.path.as_path()) || !o.links_producer() {
                        continue;
                    }
                    let rec = TaskOutputRec {
                        path: rel_to(root_abs, self.repo, &o.path),
                        pattern: o.pattern.clone(),
                        exclude: o.exclude.clone(),
                        package: None,
                    };
                    if !outputs.contains(&rec) {
                        outputs.push(rec);
                    }
                }
            }
            out.files
                .push((rel_path.clone(), analysis(path, edges, obs), PART_LANG));
            out.parts
                .push((rel_path.clone(), crate::shard::PartRec { file: 0, outputs }));
        }
        // Per task, in parallel (the root package has hundreds of tasks, each merging
        // the package-wide observations), in task order.
        use rayon::prelude::*;
        type PerTask = (
            TaskRec,
            (String, FileAnalysis, &'static str),
            Vec<String>,
            Vec<PathBuf>,
        );
        let per_task: Vec<PerTask> = tasks
            .par_iter()
            .map(|t| {
                let mut dep_closures: Vec<String> = Vec::new();
                let rel_path = format!("#{}", t.id);
                let path = root_abs.join(&rel_path);
                let mut obs = shared_obs.clone();
                obs.extend_from(&t.observations);
                let mut edges = Vec::new();
                let written = t.spec.written.as_slice();
                for e in self.by_place(&t.spec.entries, |e| e.path.as_path()) {
                    for (p, reading) in self.named(&e.path, written) {
                        if fsx::exists(&p) {
                            let id = self.tracer.intern(&p);
                            edges.push(self.edge(
                                &path,
                                RefKind::Import,
                                reading.unwrap_or_else(|| format!("entry {}", self.display(&p))),
                                vec![id],
                                Vec::new(),
                                None,
                            ));
                        } else {
                            obs.missing.push(p);
                        }
                    }
                }
                for (i, data) in self.in_place_order(&t.spec.inputs, &t.spec.data_inputs) {
                    for (p, reading) in self.named(i, written) {
                        if fsx::exists(&p) {
                            let id = self.tracer.intern(&p);
                            let e = self.edge(
                                &path,
                                RefKind::FsRead,
                                reading.unwrap_or_else(|| format!("input {}", self.display(&p))),
                                vec![id],
                                Vec::new(),
                                None,
                            );
                            // A file the task only reads: a data edge, unless another
                            // command also runs or copies it (then it is an entry or a
                            // plain input too).
                            edges.push(if data { Self::data(e) } else { e });
                        } else {
                            obs.missing.push(p);
                        }
                    }
                }
                for c in self.by_place(&t.spec.configs, |c| c.as_path()) {
                    for (p, _) in self.named(c, written) {
                        if fsx::is_file(&p) {
                            obs.reads.push(p);
                        } else {
                            obs.missing.push(p);
                        }
                    }
                }
                for m in &t.spec.memberships {
                    if let Some(h) = hubs.get(&m.key()) {
                        let id = self.tracer.intern(&root_abs.join(h));
                        let e = self.edge(
                            &path,
                            RefKind::FsRead,
                            format!("{} ({})", &h[1..], m.reason),
                            vec![id],
                            Vec::new(),
                            None,
                        );
                        // A data edge where the task only reads the members. A hub that
                        // another membership of the task may run or compile (an unknown
                        // command's package next to the turbo inputs of the same
                        // directory) also gets a plain edge.
                        edges.push(if m.data { Self::data(e) } else { e });
                    }
                }
                for tc in &t.spec.toolchain {
                    if let Some((succ, links, tobs)) =
                        tool_edges.get(&tc.name).filter(|_| tc.ecosystem == "npm")
                    {
                        edges.push(self.edge(
                            &path,
                            RefKind::Require,
                            tc.name.clone(),
                            succ.clone(),
                            links.clone(),
                            None,
                        ));
                        obs.extend_from(tobs);
                    }
                }
                let deps = self.resolve_deps(pkg, t, &mut obs, &mut dep_closures);
                for u in &t.spec.uncertainties {
                    let kind = if u.kind == UncertaintyKind::ShellUnsupported {
                        UncertaintyKind::ShellUnsupported
                    } else {
                        UncertaintyKind::UnknownCommand
                    };
                    let detail = if u.kind == kind {
                        u.detail.clone()
                    } else {
                        format!("{}: {}", u.kind, u.detail)
                    };
                    edges.push(self.edge(
                        &path,
                        RefKind::FsRead,
                        format!("task: {detail}"),
                        Vec::new(),
                        Vec::new(),
                        Some(Uncertainty::new(kind, Span::DUMMY, detail)),
                    ));
                }
                // turbo configs: a task depends on its own entries of the task table and on
                // the global keys, not on the file as a whole (an edit of the `lint` entry
                // used to reach every importer of every build output). Its inputs are the
                // sections, as pseudo-paths; the file's contents key the shard.
                let mut turbo: Vec<PathBuf> = Vec::new();
                obs.reads.retain(|p| {
                    let is = turbo_config::is_config(&p.to_string_lossy());
                    if is {
                        turbo.push(p.clone());
                    }
                    !is
                });
                turbo.sort();
                turbo.dedup();
                let name = match &t.id {
                    TaskId::Script { name, .. } => name,
                    _ => &t.name,
                };
                for f in &turbo {
                    for s in [
                        turbo_config::GLOBAL_SECTION.to_string(),
                        turbo_config::task_section(name),
                    ] {
                        obs.path_reads
                            .push(PathBuf::from(format!("{}{s}", f.to_string_lossy())));
                    }
                }
                obs.dedup();
                let rec = TaskRec {
                    file: 0,
                    id: t.id.to_string(),
                    name: t.name.clone(),
                    sources: t.sources.iter().map(|s| s.to_string()).collect(),
                    lines: t.lines.clone(),
                    commands: t
                        .commands
                        .iter()
                        .map(|c| TaskCommandRec {
                            text: c.text.clone(),
                            program: c.program.clone(),
                            recognizer: c.recognizer.map(str::to_string),
                            recognition: match c.recognition {
                                Recognition::Full => 0,
                                Recognition::Partial => 1,
                                Recognition::Ignored => 2,
                                Recognition::Unknown => 3,
                            },
                        })
                        .collect(),
                    // Only outputs that link importers to their producer (generated, declared,
                    // artifacts). In-place edits of source files and CI side effects stay in the
                    // task's own record but must not make the edited files depend on the task.
                    // A part's outputs link to the part (`PartRec`). An unplaced write
                    // (`$BUILD_DIR/x`) is kept: its links are listing-only (`compose::listed`).
                    outputs: self
                        .by_place(&t.spec.outputs, |o| o.path.as_path())
                        .into_iter()
                        .filter(|o| {
                            (o.links_producer() || o.links_listings_only()) && o.part.is_none()
                        })
                        .map(|o| TaskOutputRec {
                            path: rel_to(root_abs, self.repo, &o.path),
                            pattern: o.pattern.clone(),
                            exclude: o.exclude.clone(),
                            package: None,
                        })
                        .chain(self.package_outputs(t, root_abs))
                        .collect(),
                    env: t.spec.env.clone(),
                    toolchain: t
                        .spec
                        .toolchain
                        .iter()
                        .map(|x| format!("{}:{}", x.ecosystem, x.name))
                        .collect(),
                    deps: deps.iter().map(|d| d.to_string()).collect(),
                    uncertainties: t
                        .spec
                        .uncertainties
                        .iter()
                        .map(|u| {
                            // No record names the directory the checkout lives in (a
                            // recognizer reports a computed working directory by its
                            // absolute path).
                            let text = format!("{}: {}", u.kind, u.detail);
                            crate::paths::without_repo(&self.repo.to_string_lossy(), &text)
                                .into_owned()
                        })
                        .collect(),
                    runs: encode_runs(&t.spec.runs, &t.spec.definition, &pkg.rel, self.repo),
                };
                (
                    rec,
                    (rel_path, analysis(path, edges, obs), TASK_LANG),
                    dep_closures,
                    turbo,
                )
            })
            .collect();
        for (rec, file, closures, turbo) in per_task {
            out.recs.push(rec);
            out.files.push(file);
            out.dep_closures.extend(closures);
            out.key_reads.extend(turbo);
        }
        out.dep_closures.sort();
        out.dep_closures.dedup();
        out.key_reads.sort();
        out.key_reads.dedup();
        out
    }

    /// The locations of the task's output packages (npm only), each an output
    /// marked with its package name (see
    /// [`turborepo_trace_tasks::spec::OutputPackage`]).
    fn package_outputs(&self, t: &Task, root_abs: &Path) -> Vec<TaskOutputRec> {
        t.spec
            .output_packages
            .iter()
            .filter(|p| p.ecosystem == "npm")
            .flat_map(|p| {
                p.locations(root_abs, self.repo)
                    .into_iter()
                    .map(|l| TaskOutputRec {
                        path: rel_to(root_abs, self.repo, &l),
                        pattern: None,
                        exclude: Vec::new(),
                        package: Some(p.name.clone()),
                    })
            })
            .collect()
    }

    /// `require('<name>/package.json')` from the package root, per tool.
    #[allow(clippy::type_complexity)]
    fn resolve_tools(
        &self,
        root_abs: &Path,
        names: &BTreeSet<String>,
    ) -> BTreeMap<
        String,
        (
            Vec<turborepo_trace_core::FileId>,
            Vec<turborepo_trace_core::FileId>,
            Observations,
        ),
    > {
        let mut out = BTreeMap::new();
        if names.is_empty() {
            return out;
        }
        let names: Vec<&String> = names.iter().collect();
        let src: String = names
            .iter()
            .map(|n| {
                format!(
                    "require({});\n",
                    serde_json::to_string(&format!("{n}/package.json")).unwrap_or_default()
                )
            })
            .collect();
        let a = self
            .tracer
            .analyze_source(&root_abs.join("__turbotrace_task__.js"), src.as_bytes());
        for g in &a.groups {
            let Some(i) = g.ref_index else { continue };
            let Some(name) = names.get(i as usize) else {
                continue;
            };
            let mut succ = Vec::new();
            let mut links = Vec::new();
            for e in &a.edges[g.edges.clone()] {
                succ.extend(e.succ.iter().copied());
                links.extend(e.links.iter().copied());
            }
            out.insert((*name).clone(), (succ, links, g.observations.clone()));
        }
        out
    }

    /// Resolves the task's references to task ids (existence is checked when
    /// the graph is composed); records the manifests consulted. For
    /// `^task`, the result depends only on the package's dependency closure
    /// (its roots, recorded in `closures` and keyed), so the
    /// closure's manifests are path-only inputs: a version bump of a low-level
    /// package does not re-key its transitive dependents, while queries
    /// still see the manifests as task inputs.
    fn resolve_deps(
        &self,
        pkg: &turborepo_trace_tasks::sources::Package,
        t: &Task,
        obs: &mut Observations,
        closures: &mut Vec<String>,
    ) -> BTreeSet<TaskId> {
        let mut out = BTreeSet::new();
        let listing = (self.repo.to_path_buf(), "**/package.json".to_string());
        for d in &t.spec.task_deps {
            match &d.target {
                TaskRef::Local { name } => {
                    out.insert(TaskId::script(pkg.rel.clone(), name.clone()));
                }
                TaskRef::Upstream { name } => {
                    obs.reads.push(pkg.dir.join("package.json"));
                    closures.push(pkg.rel.clone());
                    for i in self.graph.closure(&pkg.rel).unwrap_or_default() {
                        let p = &self.ws.packages[i];
                        obs.path_reads.push(p.dir.join("package.json"));
                        out.insert(TaskId::script(p.rel.clone(), name.clone()));
                    }
                }
                TaskRef::Package { package, name } => {
                    obs.listings.push(listing.clone());
                    let ids: BTreeSet<TaskId> = [d.target.clone()]
                        .iter()
                        .flat_map(|r| self.package_targets(r, package, name, obs))
                        .collect();
                    out.extend(ids);
                }
                TaskRef::All { name } => {
                    obs.listings.push(listing.clone());
                    for p in &self.ws.packages {
                        let pj = p.dir.join("package.json");
                        obs.reads.push(pj.clone());
                        let has = turborepo_trace_tasks::sources::read_json(&pj)
                            .and_then(|m| m.get("scripts").and_then(|s| s.get(name)).map(|_| ()))
                            .is_some();
                        if has {
                            out.insert(TaskId::script(p.rel.clone(), name.clone()));
                        }
                    }
                }
                TaskRef::Id { id } => {
                    out.insert(id.clone());
                }
            }
        }
        out.remove(&t.id);
        out
    }

    fn package_targets(
        &self,
        _r: &TaskRef,
        package: &str,
        name: &str,
        obs: &mut Observations,
    ) -> Vec<TaskId> {
        let mut out = Vec::new();
        if turborepo_trace_tasks::glob::is_glob(package) {
            let pat = turborepo_trace_tasks::sources::norm_rel(package);
            for p in &self.ws.packages {
                obs.reads.push(p.dir.join("package.json"));
                if p.name
                    .as_deref()
                    .is_some_and(|n| turborepo_trace_tasks::glob::matches(package, n))
                    || turborepo_trace_tasks::glob::matches(&pat, &p.rel)
                {
                    out.push(TaskId::script(p.rel.clone(), name));
                }
            }
        } else if let Some(p) = self.ws.package(package) {
            obs.reads.push(p.dir.join("package.json"));
            out.push(TaskId::script(p.rel.clone(), name));
        }
        out
    }
}

/// One test run of a task ([`turborepo_trace_tasks::spec::TestRun`]) as stored
/// in [`TaskRec::runs`], for the task of the package at `pkg_rel` (`.` for the
/// root):
///
/// | Run | Encoding |
/// |---|---|
/// | the tests of a framework below a directory | `tests:<framework>:<repo-relative dir>` (`.` for the root; an absolute path outside the repository as is) |
/// | a script of this package, of a package by name, directory or glob, or of every package | `script:<package dir>#<name>`, `script:<package>#<name>`, `script:*#<name>` |
/// | a task by id | `id:<task id>` |
/// | a test launch that can't be mapped | `unknown:<reason>` |
/// | a task runner with a computed task name | `computed:<reason>` |
/// | a service the task starts before what it runs | `service:<task id>` (the compose file's task) |
///
/// The references are resolved against the composed graph's task nodes at query
/// time (no manifest is read for them, so they add no inputs to the task). The
/// same list carries the task's definition files
/// ([`turborepo_trace_tasks::spec::TaskSpec::definition`]) as
/// `def:<repo-relative path>` ([`encode_runs`]).
pub fn encode_run(
    run: &turborepo_trace_tasks::spec::TestRun,
    pkg_rel: &str,
    repo: &Path,
) -> String {
    use turborepo_trace_tasks::spec::TestRun;
    match run {
        TestRun::Tests { framework, dir } => {
            let d = match repo_rel(repo, dir) {
                Some(d) if d.is_empty() => ".".to_string(),
                Some(d) => d,
                None => dir.to_string_lossy().into_owned(),
            };
            format!("tests:{framework}:{d}")
        }
        TestRun::Task { target } => match target {
            TaskRef::Local { name } => format!("script:{pkg_rel}#{name}"),
            TaskRef::Package { package, name } => format!("script:{package}#{name}"),
            // The dependencies' tasks: any package (a superset).
            TaskRef::Upstream { name } | TaskRef::All { name } => format!("script:*#{name}"),
            TaskRef::Id { id } => format!("id:{id}"),
        },
        TestRun::Unknown { reason } => format!("unknown:{reason}"),
        TestRun::Computed { reason } => format!("computed:{reason}"),
        TestRun::Service { target } => match target {
            TaskRef::Id { id } => format!("service:{id}"),
            // Services are defined by tasks with ids of their own (compose files); a script
            // reference would be a task the service definition is in.
            TaskRef::Local { name } => format!("service:task:{pkg_rel}#{name}"),
            TaskRef::Package { package, name } => format!("service:task:{package}#{name}"),
            TaskRef::Upstream { name } | TaskRef::All { name } => format!("service:task:*#{name}"),
        },
    }
}

/// [`TaskRec::runs`] of a task: its runs ([`encode_run`]) and its definition
/// files as `def:<repo-relative path>`. `package.json` and lockfiles are left
/// out: their changes apply field by field and package by package, not as a
/// file.
pub fn encode_runs(
    runs: &[turborepo_trace_tasks::spec::TestRun],
    definition: &[PathBuf],
    pkg_rel: &str,
    repo: &Path,
) -> Vec<String> {
    let mut out: Vec<String> = runs.iter().map(|r| encode_run(r, pkg_rel, repo)).collect();
    for d in definition {
        const BY_FIELD: &[&str] = &[
            "package.json",
            "pnpm-lock.yaml",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "yarn.lock",
            "bun.lock",
            "Cargo.lock",
            "go.sum",
            "uv.lock",
            "poetry.lock",
        ];
        if d.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| BY_FIELD.contains(&n))
        {
            continue;
        }
        if let Some(rel) = repo_rel(repo, d).filter(|r| !r.is_empty()) {
            out.push(format!("def:{rel}"));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_npm_name(n: &str) -> bool {
    !n.is_empty()
        && !n.contains(' ')
        && !n.starts_with('.')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'@' | b'/' | b'-' | b'_' | b'.'))
}

fn analysis(path: PathBuf, edges: Vec<AnalyzedEdge>, observations: Observations) -> FileAnalysis {
    let n = edges.len();
    FileAnalysis {
        path,
        extraction: None,
        hash: None,
        edges,
        groups: vec![turborepo_trace_core::RefGroup {
            ref_index: None,
            edges: 0..n,
            observations,
        }],
        read_error: None,
        head: None,
    }
}

/// Whether a shard file record is a task, membership hub or task part
/// pseudo-file.
pub fn is_pseudo_lang(lang: Option<&str>) -> bool {
    lang.is_some_and(|l| l == TASK_LANG || l == MEMBERS_LANG || l == PART_LANG)
}

/// The node path of a pseudo-file: its shard path without the leading `#` (a
/// task id, `members:<dir>/<pattern>`, or a part's `<kind>:<path>`).
pub fn pseudo_node_path(rel: &str) -> &str {
    rel.strip_prefix('#').unwrap_or(rel)
}

/// The workspace of `root` as the graph build sees it (every first-party
/// package, any language), for task discovery without a graph. `out_dir` is
/// the graph directory, which the walk skips.
pub fn discover_workspace(root: &Path, out_dir: &Path) -> TaskWorkspace {
    let files = crate::walk::walk(root, out_dir, 0);
    let ws = Workspace::discover(root, &files);
    TaskWorkspace::from_roots(root, ws.packages.iter().map(|p| p.root.clone()))
}
