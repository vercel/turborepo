//! Task sources: where tasks are declared.
//!
//! A [`TaskSource`] finds the tasks one package declares: `package.json`
//! scripts ([`npm`]), `turbo.json` ([`turbo`]), Makefiles ([`make`]),
//! Dockerfiles ([`docker`]), and the other sources. It reports each as a
//! [`DiscoveredTask`]: an id, the command lines to parse (if any) and the
//! declared parts of its [`TaskSpec`] (turbo `inputs`, make prerequisites,
//! `COPY` sources). The driver ([`crate::discover`]) parses the command
//! lines with [`crate::shell`], runs the [`crate::registry`] on each command,
//! and merges contributions with the same id.
//!
//! # Contract
//!
//! - `discover` is called once per workspace package (the repo root is a
//!   package too). It must only report files owned by that package: use
//!   [`SourceCx::find_files`], which skips nested packages and `node_modules`,
//!   for sources whose files can live in subdirectories (Dockerfiles,
//!   workflows).
//! - Read files only through [`turborepo_trace_frameworks::fsx`] (shared
//!   `DirCache`).
//! - The defining files (the `package.json`, `turbo.json`, `Makefile`) go into
//!   [`DiscoveredTask::files`]; they are config inputs of the task.
//! - [`Contribution::Augments`] adds to a task another source defines and is
//!   dropped when none does (turbo settings for a package without that script).
//! - Never execute anything; never panic on malformed files (report a
//!   [`crate::spec::TaskUncertainty`] instead).
//! - Declared inputs are never dropped later: the driver only adds recognized
//!   ones.

pub mod cargo_alias;
pub mod docker;
pub mod gha;
pub mod gogenerate;
pub mod just;
pub mod make;
pub mod npm;
pub mod nx;
pub mod pypackaging;
pub mod taskfile;
pub mod tox;
pub mod turbo;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Serialize;
use turborepo_trace_frameworks::fsx;

use crate::{
    registry::Recognizer,
    spec::{TaskId, TaskSpec},
};

/// Whether a contribution creates a task or only adds to one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Contribution {
    /// Creates the task (a script, a make target).
    #[default]
    Defines,
    /// Adds to a task some other source defines; dropped otherwise.
    Augments,
}

/// One task (or one contribution to a task) found by a [`TaskSource`].
#[derive(Clone, Debug, Default)]
pub struct DiscoveredTask {
    /// The id. Contributions with equal ids are merged.
    pub id: Option<TaskId>,
    /// Display name (`build`, `dist`).
    pub name: String,
    /// The working directory of the commands (absolute).
    pub cwd: PathBuf,
    /// Command lines, each parsed separately with [`crate::shell::parse`] (one
    /// per script; one per recipe line for make). Empty for
    /// declaration-only contributions.
    pub lines: Vec<String>,
    /// Variables known when parsing [`DiscoveredTask::lines`] (make variables,
    /// workflow `env:`), on top of the driver's environment.
    pub env: BTreeMap<String, String>,
    /// Declared inputs, outputs, deps, env, uncertainties.
    pub spec: TaskSpec,
    /// Files that define the task (config inputs).
    pub files: Vec<PathBuf>,
    /// Create or augment.
    pub mode: Contribution,
    /// What the contribution was computed from in the tree beyond the source's
    /// config files ([`TaskSource::config_file_names`], whose `**/{names}`
    /// listing stands for the source's own listings) and that is no input
    /// of the task: the entries of the directories a project layout's
    /// package discovery lists, and every path it asks about, whatever the
    /// answer. A store keys them (a change makes it analyze the package
    /// again) and no query selects through them
    /// ([`Task::decided_by`](crate::Task)).
    pub decided_by: turborepo_trace_core::Observations,
}

impl DiscoveredTask {
    /// A defining task with id `id` and display name `name`, running in `cwd`.
    pub fn new(id: TaskId, name: impl Into<String>, cwd: impl Into<PathBuf>) -> DiscoveredTask {
        DiscoveredTask {
            id: Some(id),
            name: name.into(),
            cwd: cwd.into(),
            ..Default::default()
        }
    }
}

/// Runs the discovery `f` of a source whose tasks depend on more of the tree
/// than its config files (the packages a Python project's layout makes, the
/// binaries of a crate an alias runs), and makes everything it asked of the
/// tree the [`DiscoveredTask::decided_by`] of each task it found: the listings,
/// every path it asked about whatever the answer, and the symbolic links on the
/// way to them.
///
/// The questions of a lookup (is it a file, list the directory) say nothing of
/// the links they went through, which may come to lead elsewhere with every
/// answer the same at the moment. So the way to every path that was asked about
/// and every directory that was listed is asked for as a real path
/// ([`fsx::realpath`], from the nearest one that is there): the links it
/// crosses are links of the record, and what is at each directory on
/// the way (also whether it is a link) is asked about. A link on the way that
/// leads to nothing has no real path and is a link of the record itself (where
/// it would lead is what a store compares). Without recording into the caller's
/// frame: what the way adds is only for the key.
pub fn with_decided_by(f: impl FnOnce() -> Vec<DiscoveredTask>) -> Vec<DiscoveredTask> {
    let mut nowhere: Vec<PathBuf> = Vec::new();
    let (mut tasks, log) = fsx::with_log(|| {
        let (tasks, log) = fsx::with_log(f);
        if !tasks.is_empty() {
            turborepo_trace_core::record::pause(|| {
                // Each path once (discovery asks about many twice), in the order asked.
                let mut once: std::collections::HashSet<&Path> = std::collections::HashSet::new();
                let dirs = log.listed.iter().map(|(d, _)| d);
                for p in log.asked.iter().chain(dirs) {
                    if !once.insert(p.as_path()) {
                        continue;
                    }
                    let mut at = Some(p.as_path());
                    while let Some(a) = at {
                        if fsx::realpath(a).is_some() {
                            break;
                        }
                        if fsx::is_link_to_nothing(a) {
                            nowhere.push(a.to_path_buf());
                        }
                        at = a.parent();
                    }
                }
            });
        }
        tasks
    });
    for t in &mut tasks {
        t.decided_by.listings.extend(log.listed.iter().cloned());
        t.decided_by.missing.extend(log.asked.iter().cloned());
        t.decided_by.links.extend(log.links.iter().cloned());
        t.decided_by.links.extend(nowhere.iter().cloned());
        t.decided_by.dedup();
    }
    tasks
}

/// One workspace package, as task sources see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Package {
    /// Root directory (absolute).
    pub dir: PathBuf,
    /// Root relative to the repo root, `/`-separated; `.` for the root package.
    pub rel: String,
    /// Manifest name (`package.json` `name`), if any.
    pub name: Option<String>,
    /// Names of workspace packages this one depends on (any dependency field).
    pub deps: Vec<String>,
}

/// The packages of a repository.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Workspace {
    /// Repository root (absolute).
    pub root: PathBuf,
    /// Packages, sorted by `rel`; the root package (`.`) is always present.
    pub packages: Vec<Package>,
    /// Lookup tables over [`Workspace::packages`], built on first use.
    #[serde(skip)]
    index: OnceLock<PackageIndex>,
}

/// Reads a `package.json` (absolute path) for [`Workspace::from_roots_with`].
pub type ManifestReader<'a> = dyn Fn(&Path) -> Option<serde_json::Value> + Sync + 'a;

/// Package indexes by directory and by name (first package in `rel` order per
/// name).
#[derive(Clone, Debug, Default)]
struct PackageIndex {
    by_dir: std::collections::HashMap<PathBuf, usize>,
    by_name: std::collections::HashMap<String, usize>,
    by_rel: std::collections::HashMap<String, usize>,
}

impl Workspace {
    /// Builds a workspace from package roots relative to `root` (`""` or `"."`
    /// for the root), reading each `package.json` for its name and
    /// dependencies.
    pub fn from_roots<S: AsRef<str>>(root: &Path, rels: impl IntoIterator<Item = S>) -> Workspace {
        Self::from_roots_inner(root, rels, None)
    }

    /// [`Workspace::from_roots`] with the manifests (`<dir>/package.json`,
    /// absolute) read through `read`, in parallel: for callers with their
    /// own (cached, unrecorded) reads.
    pub fn from_roots_with<S: AsRef<str>>(
        root: &Path,
        rels: impl IntoIterator<Item = S>,
        read: &ManifestReader<'_>,
    ) -> Workspace {
        Self::from_roots_inner(root, rels, Some(read))
    }

    fn from_roots_inner<S: AsRef<str>>(
        root: &Path,
        rels: impl IntoIterator<Item = S>,
        read: Option<&ManifestReader<'_>>,
    ) -> Workspace {
        let mut packages: Vec<Package> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let rels: Vec<String> = rels
            .into_iter()
            .map(|r| norm_rel(r.as_ref()))
            .chain([".".to_string()])
            .filter(|r| seen.insert(r.clone()))
            .collect();
        let dir_of = |r: &str| {
            if r == "." {
                root.to_path_buf()
            } else {
                root.join(r)
            }
        };
        // Manifests read up front, on several threads when nothing observes the reads
        // (no recording frame, no shared dir cache: plain file reads, as on
        // this thread).
        let manifests = match read {
            Some(read) => {
                let threads = std::thread::available_parallelism()
                    .map_or(1, |n| n.get())
                    .min(8);
                let chunk = rels.len().div_ceil(threads).max(1);
                std::thread::scope(|s| {
                    let handles: Vec<_> = rels
                        .chunks(chunk)
                        .map(|c| {
                            let dir_of = &dir_of;
                            s.spawn(move || {
                                c.iter()
                                    .map(|r| read(&dir_of(r).join("package.json")))
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .flat_map(|h| h.join().unwrap_or_default())
                        .collect()
                })
            }
            None => read_manifests(&rels, &dir_of),
        };
        for (r, manifest) in rels.into_iter().zip(manifests) {
            let dir = dir_of(&r);
            let name = manifest
                .as_ref()
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string);
            let mut deps = Vec::new();
            if let Some(m) = &manifest {
                for field in [
                    "dependencies",
                    "devDependencies",
                    "peerDependencies",
                    "optionalDependencies",
                ] {
                    if let Some(o) = m.get(field).and_then(|v| v.as_object()) {
                        deps.extend(o.keys().cloned());
                    }
                }
            }
            packages.push(Package {
                dir,
                rel: r,
                name,
                deps,
            });
        }
        let names: std::collections::BTreeSet<String> =
            packages.iter().filter_map(|p| p.name.clone()).collect();
        for p in &mut packages {
            p.deps
                .retain(|d| names.contains(d) && Some(d) != p.name.as_ref());
            p.deps.sort();
            p.deps.dedup();
        }
        packages.sort_by(|a, b| a.rel.cmp(&b.rel));
        Workspace {
            root: root.to_path_buf(),
            packages,
            index: OnceLock::new(),
        }
    }

    fn index(&self) -> &PackageIndex {
        self.index.get_or_init(|| {
            let mut ix = PackageIndex::default();
            for (i, p) in self.packages.iter().enumerate() {
                // `package_of` takes the deepest match, ties to the last one.
                ix.by_dir.insert(p.dir.clone(), i);
                if let Some(n) = &p.name {
                    ix.by_name.entry(n.clone()).or_insert(i);
                }
                ix.by_rel.entry(p.rel.clone()).or_insert(i);
            }
            ix
        })
    }

    /// Discovers JS workspace packages from the root `package.json`
    /// `workspaces` and `pnpm-workspace.yaml` (with `!` exclusions).
    pub fn discover(root: &Path) -> Workspace {
        let mut patterns: Vec<String> = Vec::new();
        if let Some(m) = read_json(&root.join("package.json")) {
            match m.get("workspaces") {
                Some(serde_json::Value::Array(a)) => {
                    patterns.extend(a.iter().filter_map(|v| v.as_str().map(str::to_owned)))
                }
                Some(serde_json::Value::Object(o)) => {
                    if let Some(a) = o.get("packages").and_then(|v| v.as_array()) {
                        patterns.extend(a.iter().filter_map(|v| v.as_str().map(str::to_owned)));
                    }
                }
                _ => {}
            }
        }
        if let Ok(y) = fsx::read_to_string(root.join("pnpm-workspace.yaml")) {
            patterns.extend(pnpm_workspace_packages(&y));
        }
        let (exclude, include): (Vec<_>, Vec<_>) =
            patterns.iter().partition(|p| p.starts_with('!'));
        let exclude: Vec<String> = exclude.iter().map(|p| norm_rel(&p[1..])).collect();
        let mut rels = Vec::new();
        for pat in include {
            let pat = norm_rel(pat);
            let (prefix, _) = crate::glob::split_static_prefix(&pat);
            let base = if prefix.is_empty() {
                root.to_path_buf()
            } else {
                root.join(prefix)
            };
            fsx::walk_dirs(&base, &mut |d| {
                let Ok(rel) = d.strip_prefix(root) else {
                    return;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                if rel.is_empty() || !crate::glob::matches(&pat, &rel) {
                    return;
                }
                if exclude.iter().any(|e| crate::glob::matches(e, &rel)) {
                    return;
                }
                if fsx::is_file(&d.join("package.json")) {
                    rels.push(rel);
                }
            });
        }
        Workspace::from_roots(root, rels)
    }

    /// The package with manifest name `name`, else with root `name`
    /// (`apps/web`).
    pub fn package(&self, name: &str) -> Option<&Package> {
        let ix = self.index();
        ix.by_name
            .get(name)
            .or_else(|| ix.by_rel.get(&norm_rel(name)))
            .map(|&i| &self.packages[i])
    }

    /// The deepest package containing `path` (absolute).
    pub fn package_of(&self, path: &Path) -> Option<&Package> {
        // Package directories are distinct, so the deepest one containing `path` is its
        // nearest ancestor (or itself) that is a package directory.
        let ix = self.index();
        path.ancestors()
            .find_map(|a| ix.by_dir.get(a))
            .map(|&i| &self.packages[i])
    }

    /// `path` relative to the repo root, `/`-separated (`.` for the root
    /// itself).
    pub fn rel(&self, path: &Path) -> String {
        match path.strip_prefix(&self.root) {
            Ok(r) if r.as_os_str().is_empty() => ".".into(),
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// Workspace packages `pkg` depends on, transitively (excluding itself).
    pub fn transitive_deps(&self, pkg: &Package) -> Vec<&Package> {
        let mut out: Vec<&Package> = Vec::new();
        let mut todo: Vec<&str> = pkg.deps.iter().map(String::as_str).collect();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.to_string()) {
                continue;
            }
            if let Some(p) = self.package(n)
                && p.rel != pkg.rel
            {
                out.push(p);
                todo.extend(p.deps.iter().map(String::as_str));
            }
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        out
    }
}

/// `./a/b/` → `a/b`; empty → `.`.
pub fn norm_rel(s: &str) -> String {
    let s = s.trim().trim_start_matches("./").trim_end_matches('/');
    if s.is_empty() || s == "." {
        ".".into()
    } else {
        s.to_string()
    }
}

/// Reads and parses a JSON file (through the shared cache); `None` when missing
/// or invalid.
pub fn read_json(p: &Path) -> Option<serde_json::Value> {
    let s = fsx::read(p).ok()?;
    serde_json::from_slice(&s).ok()
}

/// `packages:` entries of a `pnpm-workspace.yaml` (block list, quoted or not).
pub fn pnpm_workspace_packages(y: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_pk = false;
    for line in y.lines() {
        let t = line.trim_end();
        if !t.starts_with(' ') && !t.starts_with('-') && !t.is_empty() {
            in_pk = t.trim_start().starts_with("packages:");
            if let Some(rest) = t.strip_prefix("packages:") {
                // Flow style: packages: ['a/*', 'b']
                let rest = rest.trim();
                if rest.starts_with('[') {
                    for item in rest.trim_matches(['[', ']']).split(',') {
                        let v = item.trim().trim_matches(['\'', '"']);
                        if !v.is_empty() {
                            out.push(v.to_string());
                        }
                    }
                }
            }
            continue;
        }
        if in_pk && let Some(item) = t.trim_start().strip_prefix('-') {
            let v = item
                .split(" #")
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"']);
            if !v.is_empty() {
                out.push(v.to_string());
            }
        }
    }
    out
}

/// What a task source sees.
pub struct SourceCx<'a> {
    /// The workspace.
    pub workspace: &'a Workspace,
    /// The package being scanned.
    pub package: &'a Package,
}

impl SourceCx<'_> {
    /// Whether `dir` (absolute) belongs to this package rather than a nested
    /// one.
    pub fn owns(&self, dir: &Path) -> bool {
        self.workspace
            .package_of(dir)
            .is_some_and(|p| p.rel == self.package.rel)
    }

    /// Files under the package directory (not in nested packages,
    /// `node_modules`, `.git`, ...) for which `pred(file name)` holds,
    /// sorted. Walks at most `max_depth` levels.
    pub fn find_files(&self, max_depth: usize, pred: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
        let mut out = Vec::new();
        self.find_in(&self.package.dir, 0, max_depth, pred, &mut out);
        out.sort();
        out
    }

    fn find_in(
        &self,
        dir: &Path,
        depth: usize,
        max: usize,
        pred: &dyn Fn(&str) -> bool,
        out: &mut Vec<PathBuf>,
    ) {
        let Some(entries) = fsx::read_dir(dir) else {
            return;
        };
        for (p, is_dir) in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_dir {
                if depth < max
                    && !fsx::skip_dir(name)
                    && name != "dist"
                    && name != "build"
                    && name != "target"
                    && self.owns(&p)
                {
                    self.find_in(&p, depth + 1, max, pred, out);
                }
            } else if pred(name) {
                out.push(p);
            }
        }
    }

    /// `task:<this package>#<name>`.
    pub fn script_id(&self, name: &str) -> TaskId {
        TaskId::script(self.package.rel.clone(), name)
    }
}

/// Discovers tasks in one package.
pub trait TaskSource: Send + Sync {
    /// Stable id (`npm`, `turbo`, `make`).
    fn id(&self) -> &'static str;

    /// File names this source reads (for change detection and store
    /// invalidation): a change to such a file re-discovers the package's
    /// tasks.
    fn config_file_names(&self) -> &'static [&'static str];

    /// The tasks `cx.package` declares.
    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask>;

    /// Whether the tasks of this source are CI or container tasks (GitHub
    /// Actions jobs, Docker images): everything they write is an
    /// [`OutputKind::CiSideEffect`](crate::spec::OutputKind::CiSideEffect),
    /// which never takes part in generated-file linking. Their inputs are
    /// unaffected (they stay valid `affected --tasks` entries).
    fn ci_side_effects(&self) -> bool {
        false
    }
}

/// Every compiled-in task source, in merge order.
pub fn sources() -> &'static [&'static dyn TaskSource] {
    static S: OnceLock<Vec<&'static dyn TaskSource>> = OnceLock::new();
    S.get_or_init(|| {
        let mut v: Vec<&'static dyn TaskSource> = Vec::new();
        v.extend(npm::sources());
        v.extend(turbo::sources());
        v.extend(make::sources());
        v.extend(docker::sources());
        v.extend(nx::sources());
        v.extend(just::sources());
        v.extend(taskfile::sources());
        v.extend(tox::sources());
        v.extend(cargo_alias::sources());
        v.extend(gha::sources());
        v.extend(pypackaging::sources());
        v.extend(gogenerate::sources());
        v
    })
}

/// Recognizers contributed by task-source modules (`make -C`, `docker build`,
/// `just x`, ...), registered before the generic fallbacks.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    let mut v: Vec<&'static dyn Recognizer> = Vec::new();
    v.extend(npm::recognizers());
    v.extend(turbo::recognizers());
    v.extend(make::recognizers());
    v.extend(docker::recognizers());
    v.extend(nx::recognizers());
    v.extend(just::recognizers());
    v.extend(taskfile::recognizers());
    v.extend(tox::recognizers());
    v.extend(cargo_alias::recognizers());
    v.extend(gha::recognizers());
    v.extend(pypackaging::recognizers());
    v
}

/// `package.json` of each package directory (`dir_of(rel)`), in order. Read on
/// scoped threads when the reads are unobserved (see
/// [`Workspace::from_roots`]); otherwise on this thread through [`read_json`]
/// (recorded, shared cache).
fn read_manifests(
    rels: &[String],
    dir_of: &(dyn Fn(&str) -> PathBuf + Sync),
) -> Vec<Option<serde_json::Value>> {
    let unobserved = !turborepo_trace_core::record::is_recording() && {
        let (a, b) = (fsx::cache(), fsx::cache());
        !std::sync::Arc::ptr_eq(&a, &b)
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    if !unobserved || rels.len() < 64 || threads < 2 {
        return rels
            .iter()
            .map(|r| read_json(&dir_of(r).join("package.json")))
            .collect();
    }
    let chunk = rels.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = rels
            .chunks(chunk)
            .map(|c| {
                (
                    c.len(),
                    s.spawn(move || {
                        c.iter()
                            .map(|r| read_json(&dir_of(r).join("package.json")))
                            .collect::<Vec<_>>()
                    }),
                )
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|(n, h)| h.join().unwrap_or_else(|_| vec![None; n]))
            .collect()
    })
}
