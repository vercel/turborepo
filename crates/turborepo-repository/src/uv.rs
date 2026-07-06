//! uv workspace discovery and Turborepo's knowledge of uv.
//!
//! Turborepo does not replace uv — uv owns Python resolution, environment
//! syncing, and package building. Turborepo's job is orchestration: decide
//! *which* projects are in scope and *whether* anything changed, then hand
//! the work to a single `uv` invocation and get out of the way.
//!
//! Discovery parses the workspace's `pyproject.toml` files directly
//! (`[tool.uv.workspace]` member/exclude globs, then each member's
//! `[project]` table); unlike Cargo, uv has no machine-readable metadata
//! command, but its workspace-membership semantics are a small, documented
//! surface. Projects are classified into two shapes:
//!
//! * **Packaged** projects — those uv will build into a wheel/sdist (they
//!   declare a `[build-system]`, or set `tool.uv.package = true`): the
//!   deliverables of the workspace. These get real `build` tasks (`uv build
//!   --package=<name>`), producing artifacts in the workspace root's `dist/`
//!   directory.
//! * **Virtual** projects — everything else. They exist in the package graph
//!   (so `--filter` and `--affected` propagate through them) but get no
//!   commands: uv cannot build a project with nothing to package.
//!
//! Environment-wide verbs run once at workspace scope on a synthetic package
//! named [`WORKSPACE_PACKAGE_NAME`]: `uv#sync` runs `uv sync --locked`
//! (`--locked` so a task never rewrites `uv.lock` — the lockfile is an input
//! to every uv task hash, and a task that mutates its own inputs would
//! invalidate itself).
//!
//! This module also owns the task-name → uv verb mapping (shared by the
//! executor and run summaries so display can't drift from execution) and the
//! input globs / env vars that participate in uv task hashes.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io,
    str::FromStr,
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

/// The conventional file name for a Python project manifest.
pub const PYPROJECT_TOML: &str = "pyproject.toml";

/// The conventional file name for a uv lockfile.
pub const UV_LOCK: &str = "uv.lock";

/// Name of the synthetic package that hosts workspace-scoped uv tasks
/// (`uv#sync`, ...). A real workspace member with this name collides and
/// hard-errors, like any other duplicate package name.
pub const WORKSPACE_PACKAGE_NAME: &str = "uv";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}: {error}")]
    ManifestRead {
        path: AbsoluteSystemPathBuf,
        #[source]
        error: io::Error,
    },
    #[error("failed to parse {path}: {error}")]
    ManifestParse {
        path: AbsoluteSystemPathBuf,
        error: Box<toml_edit::TomlError>,
    },
    #[error("invalid [tool.uv.workspace] glob: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error("failed to walk workspace members: {0}")]
    Walk(#[from] globwalk::WalkError),
    #[error("failed to read uv.lock: {0}")]
    LockfileRead(#[source] io::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::UvLockError),
    #[error("root pyproject.toml has no [tool.uv.workspace] table")]
    NotAWorkspace,
}

/// Map a Turborepo task name to the uv subcommand that implements it for a
/// packaged project. Only `build` maps today: uv has no per-project native
/// test/lint verbs (those run through arbitrary tools in the environment),
/// and guessing a tool would couple Turborepo to one Python stack.
pub fn member_subcommand(task: &str) -> Option<&'static str> {
    match task {
        "build" => Some("build"),
        _ => None,
    }
}

/// Map a Turborepo task name to the uv subcommand that implements it at
/// workspace scope (the synthetic [`WORKSPACE_PACKAGE_NAME`] package).
///
/// `sync` materializes the workspace environment; other tasks can depend on
/// `uv#sync` to guarantee `.venv` exists before they run.
pub fn workspace_subcommand(task: &str) -> Option<&'static str> {
    match task {
        "sync" => Some("sync"),
        _ => None,
    }
}

/// The uv subcommand a task resolves to for a package, given its
/// [`UvPackageKind`]. `None` means the task is a no-op for this package
/// (like a missing package.json script).
pub fn task_subcommand(kind: UvPackageKind, task: &str) -> Option<&'static str> {
    match kind {
        UvPackageKind::Packaged => member_subcommand(task),
        UvPackageKind::Workspace => workspace_subcommand(task),
        UvPackageKind::Virtual => None,
    }
}

/// Extra flags a workspace-scoped uv subcommand always runs with.
///
/// `uv sync` without `--locked` refreshes `uv.lock` when it drifts from the
/// manifests — but the lockfile participates in every uv task's hash, so a
/// task that rewrites it would invalidate itself and cascade into
/// dependents. `--locked` makes staleness a loud failure instead.
pub fn workspace_subcommand_flags(subcommand: &str) -> &'static [&'static str] {
    match subcommand {
        "sync" => &["--locked"],
        _ => &[],
    }
}

/// The command displayed for a uv task in run summaries and dry-runs.
/// Derived from the same tables the executor uses, so summaries always show
/// the command that actually runs.
pub fn display_command(kind: UvPackageKind, task: &str, package: &str) -> Option<String> {
    let verb = task_subcommand(kind, task)?;
    Some(match kind {
        UvPackageKind::Packaged => format!("uv {verb} --package={package}"),
        UvPackageKind::Workspace => {
            let mut command = format!("uv {verb}");
            for flag in workspace_subcommand_flags(verb) {
                command.push(' ');
                command.push_str(flag);
            }
            command
        }
        UvPackageKind::Virtual => return None,
    })
}

/// Whether pass-through args for this uv subcommand must be placed after a
/// `--` separator. Neither `uv build` nor `uv sync` accepts one — their
/// pass-through args are appended directly as uv flags (e.g.
/// `turbo build -- --no-sources` becomes `uv build --package=x
/// --no-sources`).
pub fn pass_through_uses_separator(_subcommand: &str) -> bool {
    false
}

/// Environment variables that change what uv resolves, builds, or syncs.
/// These participate in a uv task's hash so flipping them invalidates the
/// cache: `UV_PYTHON` selects the interpreter, and the index URLs change
/// where packages come from.
pub const HASHED_ENV_VARS: &[&str] = &["UV_EXTRA_INDEX_URL", "UV_INDEX_URL", "UV_PYTHON"];

/// Input globs whose changes should invalidate a uv task's cache: the
/// workspace root manifest (workspace membership, `requires-python`, shared
/// `[tool.uv]` configuration all live there), uv's own config file, and the
/// pinned interpreter version — expressed relative to the task's package
/// directory via `prefix` (the path from the package to the repo root, e.g.
/// `../..`; empty for the workspace package). Globs that don't match
/// anything (e.g. a missing `.python-version`) simply contribute nothing.
///
/// uv.lock is deliberately absent: locked dependencies participate in each
/// member task's external-dependency hash, scoped to that member's
/// transitive closure (see [`external_closures`]), so a dependency bump only
/// invalidates the members that actually depend on it. The uv version
/// participates the same way (see [`uv_version`]).
pub fn hash_input_globs(prefix: &str) -> Vec<String> {
    [PYPROJECT_TOML, "uv.toml", ".python-version"]
        .iter()
        .map(|rel| join_prefix(prefix, rel))
        .collect()
}

/// The version of uv itself, as a hashable external-dependency identity, or
/// `None` (with a warning) when uv can't be queried.
///
/// uv is the build frontend for every task this module produces, and its
/// bundled build backend (`uv_build`) and resolver evolve across releases.
/// Participating in the external-dependency hash means building with a
/// different uv never restores another version's artifacts.
pub fn uv_version(repo_root: &AbsoluteSystemPath) -> Option<turborepo_lockfiles::Package> {
    let output = std::process::Command::new("uv")
        .arg("--version")
        .current_dir(repo_root.as_std_path())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!version.is_empty()).then_some(turborepo_lockfiles::Package {
                key: "uv".to_string(),
                version,
            })
        }
        Ok(output) => {
            tracing::warn!(
                "`uv --version` failed; the uv version will not participate in uv task hashes: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                "unable to run `uv --version`; the uv version will not participate in uv task \
                 hashes: {error}"
            );
            None
        }
    }
}

/// Per-member external dependency closures from uv.lock, for the members'
/// external-dependency hashes.
///
/// A missing uv.lock yields an empty map (the workspace is unpinned; uv
/// will create the lockfile on first sync). An unreadable or unparsable
/// lockfile is a hard error — silently hashing nothing would be unsound.
pub fn external_closures(
    repo_root: &AbsoluteSystemPath,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let lock_path = repo_root.join_component(UV_LOCK);
    let contents = match lock_path.read_to_string() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(Error::LockfileRead(error)),
    };
    Ok(turborepo_lockfiles::uv_external_closures(
        &contents, members,
    )?)
}

/// Output globs for a packaged project's `build` task: the wheel and sdist
/// uv writes to the workspace root's `dist/` directory. Artifact file names
/// start with the distribution name (PEP 503 normalized, hyphens replaced
/// with underscores) followed by `-<version>`, so `dist/<dist_name>-*`
/// captures both artifacts without matching other projects' output (the
/// literal `-` cannot appear inside another normalized distribution name).
///
/// Builds using `--out-dir` write elsewhere; declare explicit `outputs` in
/// turbo.json for those layouts.
pub fn deliverable_output_globs(prefix: &str, deliverables: &[Deliverable]) -> Vec<String> {
    deliverables
        .iter()
        .map(|deliverable| join_prefix(prefix, &format!("dist/{}-*", deliverable.dist_name)))
        .collect()
}

/// Rewrite the workspace root pyproject.toml for a pruned repository
/// containing only `kept_dirs` (workspace-relative unix paths of the
/// retained members, whose normalized names are `kept_names`).
///
/// * `[tool.uv.workspace].members` becomes the explicit kept list — glob
///   patterns like `packages/*` would otherwise still match, but explicitness
///   costs nothing and path hygiene needs the concrete set anyway.
/// * Root `[tool.uv.sources]` entries marked `workspace = true` whose names are
///   not kept are dropped: uv validates workspace sources eagerly, and anything
///   the root project actually needs is in the closure and therefore kept.
///
/// Everything else — `requires-python`, dependency groups, comments,
/// formatting — is preserved via `toml_edit`.
pub fn prune_root_manifest(
    contents: &str,
    kept_dirs: &[String],
    kept_names: &[String],
) -> Result<String, Error> {
    let mut doc: toml_edit::DocumentMut =
        contents.parse().map_err(|error| Error::ManifestParse {
            path: AbsoluteSystemPathBuf::default(),
            error: Box::new(error),
        })?;

    let workspace = doc
        .get_mut("tool")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|tool| tool.get_mut("uv"))
        .and_then(|item| item.as_table_like_mut())
        .and_then(|uv| uv.get_mut("workspace"))
        .and_then(|item| item.as_table_like_mut())
        .ok_or(Error::NotAWorkspace)?;

    let mut members = toml_edit::Array::new();
    let mut sorted_dirs = kept_dirs.to_vec();
    sorted_dirs.sort();
    sorted_dirs.dedup();
    for dir in &sorted_dirs {
        members.push(dir.as_str());
    }
    workspace.insert("members", toml_edit::value(members));

    let kept_names: HashSet<String> = kept_names.iter().map(|name| normalize_name(name)).collect();
    if let Some(sources) = doc
        .get_mut("tool")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|tool| tool.get_mut("uv"))
        .and_then(|item| item.as_table_like_mut())
        .and_then(|uv| uv.get_mut("sources"))
        .and_then(|item| item.as_table_like_mut())
    {
        let removed: Vec<String> = sources
            .iter()
            .filter(|(name, value)| {
                source_is_workspace(value) && !kept_names.contains(&normalize_name(name))
            })
            .map(|(name, _)| name.to_string())
            .collect();
        for name in removed {
            sources.remove(&name);
        }
    }

    Ok(doc.to_string())
}

/// Whether a `[tool.uv.sources]` value declares a workspace source, either
/// directly (`{ workspace = true }`) or in a marker-conditional list.
fn source_is_workspace(value: &toml_edit::Item) -> bool {
    fn table_like_is_workspace(table: &dyn toml_edit::TableLike) -> bool {
        table
            .get("workspace")
            .and_then(|item| item.as_bool())
            .unwrap_or(false)
    }
    if let Some(table) = value.as_table_like() {
        return table_like_is_workspace(table);
    }
    if let Some(array) = value.as_array() {
        return array.iter().any(|entry| {
            entry
                .as_inline_table()
                .is_some_and(|table| table_like_is_workspace(table))
        });
    }
    false
}

fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Whether `name` is a valid Python project name per PEP 508: ASCII
/// alphanumerics, `-`, `_`, and `.`, starting and ending with an
/// alphanumeric.
///
/// uv enforces this itself, but manifests are untrusted input and project
/// names flow into `uv --package=<name>` argv and cache output glob
/// patterns, so we validate at the discovery boundary instead of relying on
/// downstream consumers.
pub fn is_valid_project_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    let Some((&last, middle)) = rest.split_last() else {
        return first.is_ascii_alphanumeric();
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && middle
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Normalize a Python package name per PEP 503: lowercase, with runs of
/// `-`, `_`, and `.` collapsed to a single `-`. uv normalizes names
/// everywhere (lockfile entries, `--package` matching), so Turborepo uses
/// the normalized name as the package's canonical identity.
pub fn normalize_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut last_was_separator = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            last_was_separator = true;
        } else {
            if last_was_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            last_was_separator = false;
            normalized.push(c.to_ascii_lowercase());
        }
    }
    normalized
}

/// The distribution (artifact) name for a normalized project name: PEP 427
/// wheel file names replace `-` with `_`. uv names sdists the same way.
pub fn dist_name(normalized_name: &str) -> String {
    normalized_name.replace('-', "_")
}

/// How a uv-toolchain package participates in task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvPackageKind {
    /// A virtual (unpackaged) project: present in the package graph for
    /// `--filter`/`--affected` propagation, but tasks are no-ops — there is
    /// nothing for uv to build.
    Virtual,
    /// A project uv can build into a wheel/sdist: a deliverable. `build`
    /// tasks execute `uv build --package=<name>`.
    Packaged,
    /// The synthetic [`WORKSPACE_PACKAGE_NAME`] package hosting
    /// workspace-scoped verbs (`uv sync --locked`, ...).
    Workspace,
}

/// A deliverable artifact a packaged project produces: the distribution
/// name that prefixes the wheel/sdist file names uv writes to `dist/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deliverable {
    /// The distribution name: the project's normalized name with `-`
    /// replaced by `_`, matching the artifact file names.
    pub dist_name: String,
}

/// uv-specific details attached to a [`super::package_graph::PackageInfo`]
/// when its toolchain is Uv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvPackageDetails {
    pub kind: UvPackageKind,
    /// The project's deliverable artifacts (empty for virtual projects and
    /// the workspace package). Used to derive cacheable output paths.
    pub deliverables: Vec<Deliverable>,
}

/// A single Python project discovered within a uv workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvProject {
    /// The project's PEP 503-normalized name. uv normalizes names in
    /// `uv.lock` and `--package` matching, so the normalized form is the
    /// canonical identity everywhere in Turborepo.
    pub name: String,
    /// Absolute path to the project's `pyproject.toml`.
    pub manifest_path: AbsoluteSystemPathBuf,
    /// Normalized names of other workspace members this project depends on
    /// (declared dependencies with a `workspace = true` source).
    /// Dev-dependency-group edges that would form a cycle are dropped,
    /// since Python permits dependency cycles but the package graph must
    /// remain a DAG.
    pub internal_dependencies: Vec<String>,
    /// The project's deliverable artifacts. Non-empty exactly when the
    /// project is packaged (declares a `[build-system]` or sets
    /// `tool.uv.package = true`).
    pub deliverables: Vec<Deliverable>,
}

impl UvProject {
    /// Whether this project is packaged: uv can build it, so it gets a real
    /// `build` task.
    pub fn is_packaged(&self) -> bool {
        !self.deliverables.is_empty()
    }
}

/// Discover all Python projects in the uv workspace rooted at `repo_root`
/// by expanding the root manifest's `[tool.uv.workspace]` member globs and
/// parsing each member's `pyproject.toml`.
///
/// Returns an empty vec if `repo_root` has no `pyproject.toml`, or has one
/// without a `[tool.uv.workspace]` table (a standalone project, not a
/// workspace). A root manifest that fails to parse is an error — the user
/// opted into uv support, so silently discovering nothing would be
/// misleading.
///
/// Members whose manifests lack a `[project]` name, or whose names are
/// invalid, are skipped with a warning. A `[project]` in the root manifest
/// is skipped too: its directory would be the entire repository.
pub fn discover_projects(repo_root: &AbsoluteSystemPath) -> Result<Vec<UvProject>, Error> {
    let root_manifest_path = repo_root.join_component(PYPROJECT_TOML);
    let contents = match root_manifest_path.read_to_string() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(Error::ManifestRead {
                path: root_manifest_path,
                error,
            });
        }
    };
    let root_doc: toml_edit::DocumentMut =
        contents.parse().map_err(|error| Error::ManifestParse {
            path: root_manifest_path.clone(),
            error: Box::new(error),
        })?;

    let Some(workspace) = root_doc
        .get("tool")
        .and_then(|item| item.as_table_like())
        .and_then(|tool| tool.get("uv"))
        .and_then(|item| item.as_table_like())
        .and_then(|uv| uv.get("workspace"))
        .and_then(|item| item.as_table_like())
    else {
        return Ok(Vec::new());
    };

    let member_globs = string_array(workspace.get("members"));
    let exclude_globs = string_array(workspace.get("exclude"));

    let mut includes = Vec::with_capacity(member_globs.len());
    for member in &member_globs {
        let member = member.trim_end_matches('/');
        let pattern = format!("{member}/{PYPROJECT_TOML}");
        let glob = globwalk::fix_glob_pattern(&pattern);
        includes.push(globwalk::ValidatedGlob::from_str(&glob)?);
    }
    let mut excludes = Vec::with_capacity(exclude_globs.len());
    for exclude in &exclude_globs {
        let exclude = exclude.trim_end_matches('/');
        let pattern = format!("{exclude}/**");
        let glob = globwalk::fix_glob_pattern(&pattern);
        excludes.push(globwalk::ValidatedGlob::from_str(&glob)?);
    }

    let mut manifest_paths: Vec<AbsoluteSystemPathBuf> =
        globwalk::globwalk(repo_root, &includes, &excludes, globwalk::WalkType::Files)?
            .into_iter()
            .collect();
    manifest_paths.sort();

    let mut parsed = Vec::with_capacity(manifest_paths.len());
    for manifest_path in manifest_paths {
        if manifest_path == root_manifest_path {
            tracing::warn!(
                "ignoring [project] in the root pyproject.toml: a project at the repository root \
                 is not supported as a Turborepo package"
            );
            continue;
        }
        if let Some(project) = parse_member(&manifest_path)? {
            parsed.push(project);
        }
    }

    Ok(connect_projects(parsed))
}

/// A workspace member parsed from its pyproject.toml, before dependency
/// edges are resolved against the discovered member set.
struct ParsedProject {
    name: String,
    manifest_path: AbsoluteSystemPathBuf,
    /// Normalized names of declared dependencies with a `workspace = true`
    /// source, with `dev` marking dependency-group (unpublished) edges.
    workspace_deps: Vec<(String, bool)>,
    packaged: bool,
}

fn parse_member(manifest_path: &AbsoluteSystemPathBuf) -> Result<Option<ParsedProject>, Error> {
    let contents = manifest_path
        .read_to_string()
        .map_err(|error| Error::ManifestRead {
            path: manifest_path.clone(),
            error,
        })?;
    let doc: toml_edit::DocumentMut = contents.parse().map_err(|error| Error::ManifestParse {
        path: manifest_path.clone(),
        error: Box::new(error),
    })?;

    let project = doc.get("project").and_then(|item| item.as_table_like());
    let Some(name) = project
        .and_then(|project| project.get("name"))
        .and_then(|item| item.as_str())
    else {
        tracing::warn!("skipping uv workspace member {manifest_path}: no [project] name");
        return Ok(None);
    };
    if !is_valid_project_name(name) {
        tracing::warn!("skipping uv manifest {manifest_path}: invalid project name {name:?}");
        return Ok(None);
    }
    let name = normalize_name(name);
    if name == WORKSPACE_PACKAGE_NAME {
        tracing::warn!(
            "skipping uv project {name:?}: the name is reserved for Turborepo's synthetic \
             workspace package"
        );
        return Ok(None);
    }

    let tool_uv = doc
        .get("tool")
        .and_then(|item| item.as_table_like())
        .and_then(|tool| tool.get("uv"))
        .and_then(|item| item.as_table_like());

    // A project is packaged (buildable into a wheel/sdist) when it declares
    // a build backend; `tool.uv.package` overrides in either direction.
    let packaged = tool_uv
        .and_then(|uv| uv.get("package"))
        .and_then(|item| item.as_bool())
        .unwrap_or_else(|| doc.get("build-system").is_some());

    // Dependencies with `{ workspace = true }` sources are the workspace
    // edges; everything else resolves through the registry and is covered
    // by the lockfile closure.
    let workspace_sources: HashSet<String> = tool_uv
        .and_then(|uv| uv.get("sources"))
        .and_then(|item| item.as_table_like())
        .map(|sources| {
            sources
                .iter()
                .filter(|(_, value)| source_is_workspace(value))
                .map(|(dep_name, _)| normalize_name(dep_name))
                .collect()
        })
        .unwrap_or_default();

    let mut workspace_deps = Vec::new();
    let mut push_deps = |requirements: &[String], dev: bool| {
        for requirement in requirements {
            let Some(dep_name) = requirement_name(requirement) else {
                continue;
            };
            let dep_name = normalize_name(&dep_name);
            if workspace_sources.contains(&dep_name) {
                workspace_deps.push((dep_name, dev));
            }
        }
    };
    if let Some(project) = project {
        push_deps(&string_array(project.get("dependencies")), false);
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(|item| item.as_table_like())
        {
            for (_, requirements) in optional.iter() {
                push_deps(&string_array(Some(requirements)), false);
            }
        }
    }
    // PEP 735 dependency groups are development-only: like Cargo
    // dev-dependencies, they may legitimately form cycles between members.
    if let Some(groups) = doc
        .get("dependency-groups")
        .and_then(|item| item.as_table_like())
    {
        for (_, requirements) in groups.iter() {
            push_deps(&string_array(Some(requirements)), true);
        }
    }

    Ok(Some(ParsedProject {
        name,
        manifest_path: manifest_path.clone(),
        workspace_deps,
        packaged,
    }))
}

/// Resolve dependency edges against the discovered member set and drop
/// dev-group edges that would form a cycle (Python permits dependency
/// cycles; the package graph is a DAG).
fn connect_projects(parsed: Vec<ParsedProject>) -> Vec<UvProject> {
    let member_names: HashSet<&str> = parsed.iter().map(|p| p.name.as_str()).collect();

    let mut adjacency: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut dev_edges: Vec<(&str, &str)> = Vec::new();
    for project in &parsed {
        let from = project.name.as_str();
        adjacency.entry(from).or_default();
        for (dep, dev) in &project.workspace_deps {
            let Some(&to) = member_names.get(dep.as_str()) else {
                // A `workspace = true` source pointing at a non-member; uv
                // itself errors on this at lock time, so just skip the edge.
                continue;
            };
            if to == from {
                continue;
            }
            if *dev {
                dev_edges.push((from, to));
            } else {
                adjacency.entry(from).or_default().insert(to);
            }
        }
    }
    // Deterministic order so the same dev edge always wins when a cycle
    // must be broken.
    dev_edges.sort_unstable();
    dev_edges.dedup();
    for (from, to) in dev_edges {
        if reaches(&adjacency, to, from) {
            tracing::debug!(
                "dropping dev dependency-group edge {from} -> {to}: it would create a cycle in \
                 the package graph"
            );
        } else {
            adjacency.entry(from).or_default().insert(to);
        }
    }

    let mut edges: HashMap<String, Vec<String>> = adjacency
        .into_iter()
        .map(|(name, deps)| {
            (
                name.to_string(),
                deps.into_iter().map(String::from).collect(),
            )
        })
        .collect();

    parsed
        .into_iter()
        .map(|project| UvProject {
            internal_dependencies: edges.remove(project.name.as_str()).unwrap_or_default(),
            deliverables: project
                .packaged
                .then(|| Deliverable {
                    dist_name: dist_name(&project.name),
                })
                .into_iter()
                .collect(),
            name: project.name,
            manifest_path: project.manifest_path,
        })
        .collect()
}

/// Whether `target` is reachable from `start` in the current adjacency map.
fn reaches(adjacency: &HashMap<&str, BTreeSet<&str>>, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }
    let mut stack = vec![start];
    let mut visited = HashSet::new();
    while let Some(node) = stack.pop() {
        if !visited.insert(node) {
            continue;
        }
        if let Some(next) = adjacency.get(node) {
            for &dep in next {
                if dep == target {
                    return true;
                }
                stack.push(dep);
            }
        }
    }
    false
}

/// The values of a TOML array of strings; non-strings are ignored (e.g.
/// `{ include-group = ... }` entries in dependency groups).
fn string_array(item: Option<&toml_edit::Item>) -> Vec<String> {
    item.and_then(|item| item.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Extract the package name from a PEP 508 requirement string: the leading
/// run of name characters, before any extras (`[`), version specifiers,
/// URL (`@`), or environment markers (`;`).
fn requirement_name(requirement: &str) -> Option<String> {
    let name: String = requirement
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn write_fixture(root: &AbsoluteSystemPathBuf, files: &[(&[&str], &str)]) {
        for (path, contents) in files {
            let file = root.join_components(path);
            file.ensure_dir().unwrap();
            file.create_with_contents(contents).unwrap();
        }
    }

    fn tmpdir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = AbsoluteSystemPathBuf::try_from(dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (dir, path)
    }

    const ROOT_MANIFEST: &str = r#"
[project]
name = "root-project"
version = "0.1.0"
requires-python = ">=3.12"

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/skipped"]
"#;

    fn app_manifest(extra: &str) -> String {
        format!(
            r#"
[project]
name = "app"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["lib-a>=0.1", "requests>=2"]

[build-system]
requires = ["uv_build>=0.11,<0.12"]
build-backend = "uv_build"

[tool.uv.sources]
lib-a = {{ workspace = true }}
{extra}"#
        )
    }

    const LIB_MANIFEST: &str = r#"
[project]
name = "Lib_A"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[build-system]
requires = ["uv_build>=0.11,<0.12"]
build-backend = "uv_build"
"#;

    const VIRT_MANIFEST: &str = r#"
[project]
name = "virt"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []
"#;

    #[test]
    fn test_discovers_members_edges_and_kinds() {
        let (_tmp, root) = tmpdir();
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (
                    &["packages", "app", PYPROJECT_TOML],
                    app_manifest("").as_str(),
                ),
                (&["packages", "lib-a", PYPROJECT_TOML], LIB_MANIFEST),
                (&["packages", "virt", PYPROJECT_TOML], VIRT_MANIFEST),
                (&["packages", "skipped", PYPROJECT_TOML], VIRT_MANIFEST),
            ],
        );

        let projects = discover_projects(&root).unwrap();
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        // `Lib_A` normalizes to `lib-a`; `skipped` is excluded.
        assert_eq!(names, vec!["app", "lib-a", "virt"]);

        let app = &projects[0];
        assert_eq!(app.internal_dependencies, vec!["lib-a"]);
        assert!(app.is_packaged());
        assert_eq!(
            app.deliverables,
            vec![Deliverable {
                dist_name: "app".to_string()
            }]
        );

        let lib = &projects[1];
        assert!(lib.internal_dependencies.is_empty());
        assert_eq!(
            lib.deliverables,
            vec![Deliverable {
                dist_name: "lib_a".to_string()
            }]
        );

        let virt = &projects[2];
        assert!(!virt.is_packaged());
        assert!(virt.deliverables.is_empty());
    }

    #[test]
    fn test_no_workspace_table_is_not_a_workspace() {
        let (_tmp, root) = tmpdir();
        write_fixture(&root, &[(&[PYPROJECT_TOML], VIRT_MANIFEST)]);
        assert!(discover_projects(&root).unwrap().is_empty());
    }

    #[test]
    fn test_missing_root_manifest_is_empty() {
        let (_tmp, root) = tmpdir();
        assert!(discover_projects(&root).unwrap().is_empty());
    }

    #[test]
    fn test_dev_cycle_edges_are_dropped() {
        let (_tmp, root) = tmpdir();
        // app depends on lib-a; lib-a's dev group depends back on app.
        let lib = r#"
[project]
name = "lib-a"
version = "0.1.0"
dependencies = []

[dependency-groups]
dev = ["app"]

[tool.uv.sources]
app = { workspace = true }
"#;
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (
                    &["packages", "app", PYPROJECT_TOML],
                    app_manifest("").as_str(),
                ),
                (&["packages", "lib-a", PYPROJECT_TOML], lib),
            ],
        );

        let projects = discover_projects(&root).unwrap();
        let lib = projects.iter().find(|p| p.name == "lib-a").unwrap();
        assert!(lib.internal_dependencies.is_empty());
        let app = projects.iter().find(|p| p.name == "app").unwrap();
        assert_eq!(app.internal_dependencies, vec!["lib-a"]);
    }

    #[test]
    fn test_dev_group_edges_without_cycle_are_kept() {
        let (_tmp, root) = tmpdir();
        let app = r#"
[project]
name = "app"
version = "0.1.0"
dependencies = []

[dependency-groups]
dev = ["lib-a"]

[tool.uv.sources]
lib-a = { workspace = true }
"#;
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (&["packages", "app", PYPROJECT_TOML], app),
                (&["packages", "lib-a", PYPROJECT_TOML], LIB_MANIFEST),
            ],
        );

        let projects = discover_projects(&root).unwrap();
        let app = projects.iter().find(|p| p.name == "app").unwrap();
        assert_eq!(app.internal_dependencies, vec!["lib-a"]);
    }

    #[test]
    fn test_package_override_controls_kind() {
        let (_tmp, root) = tmpdir();
        // Opted out of packaging despite a build-system, and opted in
        // without one.
        let opted_out = r#"
[project]
name = "opted-out"
version = "0.1.0"

[build-system]
requires = ["uv_build"]
build-backend = "uv_build"

[tool.uv]
package = false
"#;
        let opted_in = r#"
[project]
name = "opted-in"
version = "0.1.0"

[tool.uv]
package = true
"#;
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (&["packages", "opted-out", PYPROJECT_TOML], opted_out),
                (&["packages", "opted-in", PYPROJECT_TOML], opted_in),
            ],
        );

        let projects = discover_projects(&root).unwrap();
        assert!(
            !projects
                .iter()
                .find(|p| p.name == "opted-out")
                .unwrap()
                .is_packaged()
        );
        assert!(
            projects
                .iter()
                .find(|p| p.name == "opted-in")
                .unwrap()
                .is_packaged()
        );
    }

    #[test]
    fn test_member_without_project_name_is_skipped() {
        let (_tmp, root) = tmpdir();
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (&["packages", "nameless", PYPROJECT_TOML], "[tool.uv]\n"),
                (&["packages", "virt", PYPROJECT_TOML], VIRT_MANIFEST),
            ],
        );
        let projects = discover_projects(&root).unwrap();
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["virt"]);
    }

    #[test]
    fn test_reserved_workspace_name_is_skipped() {
        let (_tmp, root) = tmpdir();
        let reserved = r#"
[project]
name = "uv"
version = "0.1.0"
"#;
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], ROOT_MANIFEST),
                (&["packages", "uv", PYPROJECT_TOML], reserved),
            ],
        );
        assert!(discover_projects(&root).unwrap().is_empty());
    }

    #[test]
    fn test_root_project_matched_by_globs_is_skipped() {
        let (_tmp, root) = tmpdir();
        let manifest = r#"
[project]
name = "root-project"
version = "0.1.0"

[tool.uv.workspace]
members = [".", "packages/*"]
"#;
        write_fixture(
            &root,
            &[
                (&[PYPROJECT_TOML], manifest),
                (&["packages", "virt", PYPROJECT_TOML], VIRT_MANIFEST),
            ],
        );
        let projects = discover_projects(&root).unwrap();
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["virt"]);
    }

    #[test]
    fn test_subcommand_tables() {
        assert_eq!(
            task_subcommand(UvPackageKind::Packaged, "build"),
            Some("build")
        );
        assert_eq!(task_subcommand(UvPackageKind::Packaged, "sync"), None);
        assert_eq!(task_subcommand(UvPackageKind::Packaged, "test"), None);
        assert_eq!(
            task_subcommand(UvPackageKind::Workspace, "sync"),
            Some("sync")
        );
        assert_eq!(task_subcommand(UvPackageKind::Workspace, "build"), None);
        assert_eq!(task_subcommand(UvPackageKind::Virtual, "build"), None);
    }

    #[test]
    fn test_display_command() {
        assert_eq!(
            display_command(UvPackageKind::Packaged, "build", "app"),
            Some("uv build --package=app".to_string())
        );
        assert_eq!(
            display_command(UvPackageKind::Workspace, "sync", "uv"),
            Some("uv sync --locked".to_string())
        );
        assert_eq!(
            display_command(UvPackageKind::Virtual, "build", "lib"),
            None
        );
    }

    #[test]
    fn test_hash_input_globs_prefixed() {
        assert_eq!(
            hash_input_globs(""),
            vec!["pyproject.toml", "uv.toml", ".python-version"]
        );
        assert_eq!(
            hash_input_globs("../.."),
            vec![
                "../../pyproject.toml",
                "../../uv.toml",
                "../../.python-version"
            ]
        );
    }

    #[test]
    fn test_deliverable_output_globs() {
        let deliverables = vec![Deliverable {
            dist_name: "lib_a".to_string(),
        }];
        assert_eq!(
            deliverable_output_globs("../..", &deliverables),
            vec!["../../dist/lib_a-*"]
        );
    }

    #[test]
    fn test_normalize_and_dist_name() {
        assert_eq!(normalize_name("Friendly.Bard_Robot"), "friendly-bard-robot");
        assert_eq!(normalize_name("lib---a"), "lib-a");
        assert_eq!(dist_name("lib-a"), "lib_a");
    }

    #[test]
    fn test_project_name_validation() {
        assert!(is_valid_project_name("app"));
        assert!(is_valid_project_name("Lib_A.2"));
        assert!(is_valid_project_name("a"));
        assert!(!is_valid_project_name(""));
        assert!(!is_valid_project_name("-app"));
        assert!(!is_valid_project_name("app-"));
        assert!(!is_valid_project_name("app name"));
        assert!(!is_valid_project_name("app;rm -rf"));
    }

    #[test]
    fn test_prune_root_manifest() {
        let manifest = r#"# workspace root
[project]
name = "root-project"
version = "0.1.0"

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/skipped"]

[tool.uv.sources]
lib-a = { workspace = true }
gone = { workspace = true }
requests = { git = "https://example.com/requests" }
"#;
        let pruned = prune_root_manifest(
            manifest,
            &[
                "packages/app".to_string(),
                "packages/lib-a".to_string(),
                "packages/app".to_string(),
            ],
            &["app".to_string(), "lib-a".to_string()],
        )
        .unwrap();
        assert!(pruned.contains(r#"members = ["packages/app", "packages/lib-a"]"#));
        assert!(pruned.contains("# workspace root"));
        assert!(pruned.contains("lib-a = { workspace = true }"));
        assert!(!pruned.contains("gone"));
        // Non-workspace sources are untouched.
        assert!(pruned.contains("requests = { git"));
    }

    #[test]
    fn test_prune_root_manifest_requires_workspace() {
        let err = prune_root_manifest("[project]\nname = \"solo\"\n", &[], &[]).unwrap_err();
        assert!(matches!(err, Error::NotAWorkspace));
    }

    #[test]
    fn test_requirement_name_extraction() {
        assert_eq!(requirement_name("lib-a"), Some("lib-a".to_string()));
        assert_eq!(requirement_name("lib_a>=0.1"), Some("lib_a".to_string()));
        assert_eq!(requirement_name("pkg[extra]==1.0"), Some("pkg".to_string()));
        assert_eq!(
            requirement_name("pkg @ https://example.com/pkg.whl"),
            Some("pkg".to_string())
        );
        assert_eq!(
            requirement_name("pkg ; python_version < '3.12'"),
            Some("pkg".to_string())
        );
        assert_eq!(requirement_name(""), None);
    }
}
