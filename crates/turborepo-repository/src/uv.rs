//! The uv toolchain: Python packages as Turborepo packages.
//!
//! Turborepo does not replace uv — uv owns resolution, environments, and
//! installation. Turborepo's job is orchestration: decide *which* packages
//! are in scope and *whether* anything changed, then hand the work to uv
//! and get out of the way. uv is the only supported Python package manager.
//!
//! Discovery parses the root `pyproject.toml`'s `[tool.uv.workspace]` table
//! in-process: member globs are expanded against the filesystem and each
//! member's `pyproject.toml` is parsed for its identity and dependencies.
//! Unlike Cargo (whose membership semantics only `cargo metadata` can
//! answer), uv workspace membership is declarative globs — and requiring
//! the `uv` binary at discovery time would break graph construction on
//! machines that only orchestrate. The `uv` binary is required only to
//! execute tasks.
//!
//! A synthetic package anchored at the root `pyproject.toml` and depending on
//! every member represents the workspace itself. Its name is declared by the
//! user in the root manifest; using Turborepo with Python requires naming the
//! workspace:
//!
//! ```toml
//! [tool.turbo]
//! name = "acme"
//! ```
//!
//! Support is experimental and gated behind
//! `futureFlags.experimentalPythonWorkspaces`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    str::FromStr as _,
    sync::Arc,
};

use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        self, DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
        ToolchainId, WorkspaceRoot,
    },
};

/// The conventional file name for a Python project manifest.
pub const PYPROJECT_TOML: &str = "pyproject.toml";

/// The conventional file name for a uv lockfile.
pub const UV_LOCK: &str = "uv.lock";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}: {source}")]
    ManifestRead {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ManifestParse {
        path: String,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error(
        "The uv workspace has no name.\n\nTurborepo needs a name for the workspace's tasks \
         (`<name>#check`), filters (`--filter=<name>`), and configuration. Add one to the root \
         pyproject.toml:\n\n    [tool.turbo]\n    name = \"my-workspace\""
    )]
    MissingWorkspaceName,
    #[error(
        "invalid uv workspace name {name:?}: {reason}. Set a valid name in the root \
         pyproject.toml under `[tool.turbo] name`."
    )]
    InvalidWorkspaceName { name: String, reason: String },
    #[error(
        "the uv workspace name {name:?} collides with the package of the same name at {dir}. Pick \
         a different `[tool.turbo] name`."
    )]
    WorkspaceNameCollision { name: String, dir: String },
    #[error(
        "uv workspace members {first} and {second} share the normalized package name {name:?}. \
         Rename one of them."
    )]
    DuplicateMemberName {
        name: String,
        first: String,
        second: String,
    },
    #[error("invalid uv workspace member glob: {0}")]
    MemberGlob(#[from] globwalk::GlobError),
    #[error("failed to walk uv workspace members: {0}")]
    MemberWalk(#[from] globwalk::WalkError),
    #[error("uv workspace member manifest has no parent directory: {0}")]
    InvalidMemberManifestPath(String),
    #[error("unsafe uv workspace glob {0:?}: patterns must be relative and cannot contain `..`")]
    UnsafeWorkspaceGlob(String),
    #[error("uv workspace declares too many member/exclude globs (maximum {0})")]
    TooManyWorkspaceGlobs(usize),
    #[error("uv workspace expands to too many members (maximum {0})")]
    TooManyWorkspaceMembers(usize),
    #[error("uv workspace glob exceeds the maximum length of {0} bytes")]
    WorkspaceGlobTooLong(usize),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

/// Normalize a Python package name per PEP 503: lowercase, with runs of
/// `-`, `_`, and `.` collapsed to a single `-`. uv.lock entries and
/// distribution file names use the normalized form; manifests may use any
/// spelling, so every name comparison must normalize first.
pub fn normalize_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut last_was_separator = false;
    for char in name.chars() {
        if matches!(char, '-' | '_' | '.') {
            last_was_separator = true;
        } else {
            if last_was_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            last_was_separator = false;
            normalized.push(char.to_ascii_lowercase());
        }
    }
    normalized
}

/// The distribution-name form of a normalized package name: `-` becomes
/// `_`, matching the file names `uv build` writes to the dist directory
/// (PEP 625 / the wheel file name convention).
/// Extract the package name from a PEP 508 dependency string: the leading
/// identifier before any extras (`[...]`), version specifier, URL (`@`),
/// or environment marker (`;`). Returns `None` for strings that do not
/// start with a valid name.
fn pep508_name(dependency: &str) -> Option<&str> {
    let dependency = dependency.trim_start();
    let end = dependency
        .find(|char: char| !(char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.')))
        .unwrap_or(dependency.len());
    (end > 0).then(|| &dependency[..end])
}

/// Whether `name` is a valid uv workspace name for our purposes. The name
/// becomes a package name — it appears in task keys (`<name>#check`) and
/// `--filter` expressions — so it follows the same shape rules as package
/// names.
pub fn is_valid_workspace_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

// ---------------------------------------------------------------------------
// pyproject.toml parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct PyProjectManifest {
    project: Option<ProjectTable>,
    #[serde(rename = "build-system")]
    build_system: Option<toml::Value>,
    #[serde(default, rename = "dependency-groups")]
    dependency_groups: BTreeMap<String, toml::Value>,
    tool: Option<ToolTable>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectTable {
    name: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolTable {
    uv: Option<UvToolTable>,
    turbo: Option<TurboToolTable>,
}

#[derive(Debug, Default, Deserialize)]
struct UvToolTable {
    workspace: Option<UvWorkspaceTable>,
    #[serde(default)]
    sources: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Vec<String>,
    package: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct UvWorkspaceTable {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TurboToolTable {
    name: Option<toml::Value>,
}

impl PyProjectManifest {
    fn load(path: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let contents = match path.read_to_string() {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::ManifestRead {
                    path: path.to_string(),
                    source,
                });
            }
        };
        toml::from_str(&contents)
            .map(Some)
            .map_err(|source| Error::ManifestParse {
                path: path.to_string(),
                source: Box::new(source),
            })
    }

    fn workspace(&self) -> Option<&UvWorkspaceTable> {
        self.tool.as_ref()?.uv.as_ref()?.workspace.as_ref()
    }

    fn project_name(&self) -> Option<&str> {
        self.project.as_ref()?.name.as_deref()
    }

    fn uv(&self) -> Option<&UvToolTable> {
        self.tool.as_ref()?.uv.as_ref()
    }

    fn source(&self, normalized_name: &str) -> Option<&toml::Value> {
        self.uv()?
            .sources
            .iter()
            .find_map(|(name, source)| (normalize_name(name) == normalized_name).then_some(source))
    }

    fn is_buildable(&self) -> bool {
        self.build_system.is_some() || self.uv().and_then(|uv| uv.package) == Some(true)
    }

    /// All declared dependency strings, tagged with their semantic role.
    /// PEP 735 dependency groups can nest `{ include-group = "…" }` tables;
    /// only the string entries carry package names, and every group is
    /// walked, so includes add nothing.
    fn dependencies_with_kind(&self) -> impl Iterator<Item = (&str, DependencyKind)> {
        let project = self.project.as_ref();
        let dependencies = project
            .map(|project| project.dependencies.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|dependency| (dependency.as_str(), DependencyKind::Production));
        let optional = project
            .map(|project| &project.optional_dependencies)
            .into_iter()
            .flatten()
            .flat_map(|(_, dependencies)| dependencies)
            .map(|dependency| (dependency.as_str(), DependencyKind::Optional));
        let groups = self
            .dependency_groups
            .values()
            .filter_map(toml::Value::as_array)
            .flatten()
            .filter_map(toml::Value::as_str)
            .map(|dependency| (dependency, DependencyKind::Development));
        let legacy_dev = self
            .uv()
            .into_iter()
            .flat_map(|uv| &uv.dev_dependencies)
            .map(|dependency| (dependency.as_str(), DependencyKind::Development));
        dependencies.chain(optional).chain(groups).chain(legacy_dev)
    }
}

fn effective_source<'a>(
    root: &'a PyProjectManifest,
    member: &'a PyProjectManifest,
    normalized_name: &str,
) -> Option<&'a toml::Value> {
    member
        .source(normalized_name)
        .or_else(|| root.source(normalized_name))
}

fn source_uses_current_workspace(source: &toml::Value) -> bool {
    match source {
        toml::Value::Table(table) => table
            .get("workspace")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        toml::Value::Array(sources) => sources.iter().any(source_uses_current_workspace),
        _ => false,
    }
}

/// Extract and validate the user-declared workspace name from the
/// `[tool.turbo]` table.
fn workspace_name(manifest: &PyProjectManifest) -> Result<Option<String>, Error> {
    let Some(value) = manifest
        .tool
        .as_ref()
        .and_then(|tool| tool.turbo.as_ref())
        .and_then(|turbo| turbo.name.as_ref())
    else {
        return Ok(None);
    };
    let Some(name) = value.as_str() else {
        return Err(Error::InvalidWorkspaceName {
            name: value.to_string(),
            reason: "must be a string".to_string(),
        });
    };
    if !is_valid_workspace_name(name) {
        return Err(Error::InvalidWorkspaceName {
            name: name.to_string(),
            reason: "names may only contain alphanumeric characters, `-`, and `_`".to_string(),
        });
    }
    // Legal, but re-introduces exactly the toolchain-id/package-name
    // confusion user-chosen names exist to remove.
    if name == "python" || name == "javascript" || name == "rust" {
        tracing::warn!(
            "the uv workspace is named {name:?}, which is also a toolchain id; consider a more \
             distinctive name"
        );
    }
    Ok(Some(name.to_string()))
}

// ---------------------------------------------------------------------------
// Workspace discovery
// ---------------------------------------------------------------------------

/// A single Python package discovered within a uv workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvPackage {
    /// The package's PEP 503-normalized name. uv.lock and distribution file
    /// names use this form, and it is the package's Turborepo identity.
    pub name: String,
    /// Absolute path to the package's `pyproject.toml`.
    pub manifest_path: AbsoluteSystemPathBuf,
    /// Direct relationships to other workspace packages. Dependency-group
    /// edges that would make task ordering cyclic remain as
    /// hash/affectedness inputs without participating in ordering.
    pub relationships: Vec<Relationship>,
    /// Whether uv can build this member as a Python distribution.
    pub buildable: bool,
}

/// The result of uv workspace discovery: the member packages plus the
/// user-declared workspace name.
#[derive(Debug)]
pub struct DiscoveredWorkspace {
    /// The workspace's name from `[tool.turbo] name`, validated against the
    /// package set when present. Not required at this layer — it only
    /// becomes mandatory when the workspace package is actually synthesized
    /// (see [`RepositoryContributor::discover_packages`]), so manifests
    /// without members don't demand a name for nothing.
    pub name: Option<String>,
    pub packages: Vec<UvPackage>,
    /// The normalized name of the root `[project]`, when the workspace root
    /// is itself a package rather than a virtual workspace. The root
    /// project is not modeled as a Turborepo package (its directory would
    /// be the entire repository), but its uv.lock entry participates in
    /// workspace-scoped hashing and pruning.
    pub root_project_name: Option<String>,
}

/// Discover the uv workspace rooted at `repo_root` by parsing
/// `pyproject.toml` manifests in-process.
///
/// Returns an empty workspace if `repo_root` has no `pyproject.toml`, and
/// warns (rather than errors) when one exists without a
/// `[tool.uv.workspace]` table — the flag being enabled in a repository
/// whose Python code is not a uv workspace should not break unrelated runs.
///
/// Members whose manifests live outside the repository root, resolve to the
/// repository root itself, or have no `[project].name` are skipped with a
/// warning.
pub fn discover_workspace(repo_root: &AbsoluteSystemPath) -> Result<DiscoveredWorkspace, Error> {
    let root_manifest_path = repo_root.join_component(PYPROJECT_TOML);
    let Some(root_manifest) = PyProjectManifest::load(&root_manifest_path)? else {
        return Ok(DiscoveredWorkspace {
            name: None,
            packages: Vec::new(),
            root_project_name: None,
        });
    };
    let name = workspace_name(&root_manifest)?;
    let Some(workspace) = root_manifest.workspace() else {
        tracing::warn!(
            "the root pyproject.toml has no [tool.uv.workspace] table; Turborepo's Python support \
             requires a uv workspace, so no Python packages were discovered"
        );
        return Ok(DiscoveredWorkspace {
            name,
            packages: Vec::new(),
            root_project_name: None,
        });
    };

    let manifest_paths = member_manifest_paths(repo_root, workspace)?;
    let real_repo_root = repo_root.to_realpath()?;
    let real_root_manifest = root_manifest_path.to_realpath()?;
    let mut parsed: Vec<(String, AbsoluteSystemPathBuf, PyProjectManifest)> = Vec::new();
    let mut seen: HashMap<String, AbsoluteSystemPathBuf> = HashMap::new();
    for manifest_path in manifest_paths {
        let real_manifest = manifest_path.to_realpath()?;
        if real_manifest == real_root_manifest {
            // The root is never a member of itself through globs; its
            // [project] (when present) participates only through the
            // workspace package.
            continue;
        }
        if !real_manifest.starts_with(&real_repo_root) {
            tracing::warn!(
                "skipping uv workspace member {manifest_path}: it resolves outside the repository"
            );
            continue;
        }
        let Some(manifest) = PyProjectManifest::load(&manifest_path)? else {
            continue;
        };
        let Some(project_name) = manifest.project_name() else {
            tracing::warn!(
                "skipping uv workspace member {manifest_path}: pyproject.toml has no [project] \
                 name"
            );
            continue;
        };
        let normalized = normalize_name(project_name);
        if normalized.is_empty() {
            tracing::warn!(
                "skipping uv workspace member {manifest_path}: invalid package name \
                 {project_name:?}"
            );
            continue;
        }
        if let Some(first) = seen.get(&normalized) {
            return Err(Error::DuplicateMemberName {
                name: normalized,
                first: first.to_string(),
                second: manifest_path.to_string(),
            });
        }
        seen.insert(normalized.clone(), manifest_path.clone());
        parsed.push((normalized, manifest_path, manifest));
    }
    parsed.sort_by(|left, right| left.0.cmp(&right.0));

    let root_project_name = root_manifest
        .project_name()
        .map(normalize_name)
        .filter(|name| !name.is_empty());
    let member_names: HashSet<String> = parsed.iter().map(|(name, _, _)| name.clone()).collect();
    let packages = connect_packages(parsed, &member_names, &root_manifest);

    if let Some(name) = &name {
        let collision = packages
            .iter()
            .find(|package| &package.name == name)
            .map(|package| {
                package
                    .manifest_path
                    .parent()
                    .map(|dir| dir.to_string())
                    .unwrap_or_default()
            })
            .or_else(|| {
                (root_project_name.as_deref() == Some(name.as_str())).then(|| repo_root.to_string())
            });
        if let Some(dir) = collision {
            return Err(Error::WorkspaceNameCollision {
                name: name.clone(),
                dir,
            });
        }
    }

    Ok(DiscoveredWorkspace {
        name,
        packages,
        root_project_name,
    })
}

/// Expand `[tool.uv.workspace]` member globs into member `pyproject.toml`
/// paths, subtracting the exclude globs.
fn member_manifest_paths(
    repo_root: &AbsoluteSystemPath,
    workspace: &UvWorkspaceTable,
) -> Result<Vec<AbsoluteSystemPathBuf>, Error> {
    const MAX_WORKSPACE_GLOBS: usize = 1024;
    const MAX_WORKSPACE_GLOB_BYTES: usize = 4096;
    const MAX_WORKSPACE_MEMBERS: usize = 10_000;

    if workspace.members.len() + workspace.exclude.len() > MAX_WORKSPACE_GLOBS {
        return Err(Error::TooManyWorkspaceGlobs(MAX_WORKSPACE_GLOBS));
    }
    for pattern in workspace.members.iter().chain(&workspace.exclude) {
        if pattern.len() > MAX_WORKSPACE_GLOB_BYTES {
            return Err(Error::WorkspaceGlobTooLong(MAX_WORKSPACE_GLOB_BYTES));
        }
        validate_workspace_pattern(pattern)?;
    }
    let inclusions = workspace
        .members
        .iter()
        .map(|member| {
            let mut glob = member.clone();
            if !glob.ends_with('/') {
                glob.push('/');
            }
            glob.push_str(PYPROJECT_TOML);
            globwalk::ValidatedGlob::from_str(&glob)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let exclusions = workspace
        .exclude
        .iter()
        .map(|exclude| globwalk::ValidatedGlob::from_str(exclude))
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths: Vec<_> = globwalk::globwalk_with_settings(
        repo_root,
        &inclusions,
        &exclusions,
        globwalk::WalkType::Files,
        globwalk::Settings::default(),
    )?
    .into_iter()
    .collect();
    if paths.len() > MAX_WORKSPACE_MEMBERS {
        return Err(Error::TooManyWorkspaceMembers(MAX_WORKSPACE_MEMBERS));
    }
    paths.sort();
    Ok(paths)
}

fn validate_workspace_pattern(pattern: &str) -> Result<(), Error> {
    let bytes = pattern.as_bytes();
    let has_windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let is_unsafe = pattern.starts_with('/')
        || pattern.starts_with('\\')
        || has_windows_drive
        || pattern
            .split(['/', '\\'])
            .any(|component| component == "..");
    if is_unsafe {
        return Err(Error::UnsafeWorkspaceGlob(pattern.to_string()));
    }
    Ok(())
}

/// Resolve dependency edges to package names. Dependency-group
/// (development) edges that would form a cycle remain compilation inputs
/// but do not order tasks, since PEP 735 groups permit cycles while the
/// task graph is a DAG.
fn connect_packages(
    parsed: Vec<(String, AbsoluteSystemPathBuf, PyProjectManifest)>,
    member_names: &HashSet<String>,
    root_manifest: &PyProjectManifest,
) -> Vec<UvPackage> {
    let mut graph = petgraph::Graph::<(), ()>::new();
    let node_indices: HashMap<&str, petgraph::graph::NodeIndex> = parsed
        .iter()
        .map(|(name, _, _)| (name.as_str(), graph.add_node(())))
        .collect();
    let mut relationships: HashMap<String, Vec<Relationship>> = HashMap::new();
    let mut dev_edges: Vec<(String, String)> = Vec::new();
    for (name, _, manifest) in &parsed {
        let from = name.as_str();
        relationships.entry(from.to_string()).or_default();
        for (dependency, kind) in manifest.dependencies_with_kind() {
            let Some(dependency_name) = pep508_name(dependency) else {
                continue;
            };
            let to = normalize_name(dependency_name);
            // A matching name is not enough: uv only resolves the dependency
            // to this workspace when the effective source selects it.
            if to == *from
                || !member_names.contains(&to)
                || !effective_source(root_manifest, manifest, &to)
                    .is_some_and(source_uses_current_workspace)
            {
                continue;
            }
            if kind == DependencyKind::Development {
                dev_edges.push((from.to_string(), to));
            } else {
                graph.add_edge(node_indices[from], node_indices[to.as_str()], ());
                relationships
                    .entry(from.to_string())
                    .or_default()
                    .push(Relationship::internal(to, kind));
            }
        }
    }
    dev_edges.sort();
    dev_edges.dedup();
    for (from, to) in &dev_edges {
        graph.add_edge(node_indices[from.as_str()], node_indices[to.as_str()], ());
    }
    let sccs = petgraph::algo::tarjan_scc(&graph);
    let mut scc_of = HashMap::with_capacity(node_indices.len());
    for (scc, nodes) in sccs.iter().enumerate() {
        for &node in nodes {
            scc_of.insert(node, (scc, nodes.len()));
        }
    }
    for (from, to) in dev_edges {
        let from_scc = scc_of[&node_indices[from.as_str()]];
        let to_scc = scc_of[&node_indices[to.as_str()]];
        if from_scc.0 == to_scc.0 && from_scc.1 > 1 {
            tracing::debug!(
                "dropping dependency-group edge {from} -> {to}: it would create a cycle in the \
                 package graph"
            );
            relationships
                .entry(from)
                .or_default()
                .push(Relationship::internal_input(
                    to,
                    DependencyKind::Development,
                ));
        } else {
            relationships
                .entry(from)
                .or_default()
                .push(Relationship::internal(to, DependencyKind::Development));
        }
    }

    parsed
        .into_iter()
        .map(|(name, manifest_path, manifest)| {
            let mut package_relationships = relationships.remove(name.as_str()).unwrap_or_default();
            package_relationships
                .sort_by(|left, right| left.declaration_name().cmp(right.declaration_name()));
            package_relationships.dedup();
            UvPackage {
                name,
                manifest_path,
                relationships: package_relationships,
                buildable: manifest.is_buildable(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The contributor
// ---------------------------------------------------------------------------

/// The uv repository contributor. Registered during graph construction when
/// `futureFlags.experimentalPythonWorkspaces` is enabled.
pub(crate) struct UvContributor {
    repo_root: AbsoluteSystemPathBuf,
}

impl UvContributor {
    pub(crate) fn new(repo_root: AbsoluteSystemPathBuf) -> Arc<Self> {
        Arc::new(Self { repo_root })
    }
}

impl RepositoryContributor for UvContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::PYTHON
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            // Discovery reads manifests and walks member globs
            // synchronously, so keep it off the async runtime like the
            // JavaScript manifest-parsing path.
            let workspace =
                turborepo_rayon_compat::block_in_place(|| discover_workspace(&self.repo_root))
                    .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            let workspace_roots = self
                .repo_root
                .join_component(PYPROJECT_TOML)
                .exists()
                .then(|| WorkspaceRoot::new("uv", self.repo_root.clone()))
                .into_iter()
                .collect();
            let packages = workspace.packages;
            if packages.is_empty() {
                return Ok(DiscoveredPackages::new(Vec::new(), workspace_roots));
            }

            // Using Turborepo with Python requires naming the workspace: the
            // synthetic workspace package is a real package (task keys,
            // filters), and every package must have a name. Only enforced
            // when there are members to host — a memberless manifest doesn't
            // demand a name for nothing.
            let workspace_name = workspace
                .name
                .ok_or_else(|| toolchain::Error::Failed(Box::new(Error::MissingWorkspaceName)))?;

            let mut package_names = Vec::with_capacity(packages.len());
            let mut discovered = Vec::with_capacity(packages.len() + 1);
            for package in packages {
                package_names.push(package.name.clone());
                discovered.push(
                    DiscoveredPackage::package(
                        Some(package.name),
                        PackageJson::default(),
                        package.manifest_path,
                    )
                    .with_native_relationships(package.relationships),
                );
            }

            package_names.sort();
            let workspace_relationships = package_names
                .into_iter()
                .map(|name| Relationship::internal(name, DependencyKind::Production))
                .collect();
            discovered.push(
                DiscoveredPackage::aggregate(
                    workspace_name,
                    PackageJson::default(),
                    self.repo_root.join_component(PYPROJECT_TOML),
                )
                .with_native_relationships(workspace_relationships),
            );

            Ok(DiscoveredPackages::new(discovered, workspace_roots))
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_name("Django"), "django");
        assert_eq!(normalize_name("My_Package"), "my-package");
        assert_eq!(normalize_name("a.b--c__d"), "a-b-c-d");
        assert_eq!(normalize_name("already-normal"), "already-normal");
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn test_pep508_name() {
        assert_eq!(pep508_name("requests"), Some("requests"));
        assert_eq!(pep508_name("requests>=2.31"), Some("requests"));
        assert_eq!(pep508_name("requests[socks]>=2.31"), Some("requests"));
        assert_eq!(
            pep508_name("typing-extensions ; python_version < '3.12'"),
            Some("typing-extensions")
        );
        assert_eq!(pep508_name("My_Package (>=1.0)"), Some("My_Package"));
        assert_eq!(
            pep508_name("pkg @ https://example.com/pkg.whl"),
            Some("pkg")
        );
        assert_eq!(pep508_name(""), None);
        assert_eq!(pep508_name(">=1.0"), None);
    }

    fn write_workspace(root: &AbsoluteSystemPath) {
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                r#"
[project]
name = "root-project"
version = "0.1.0"
dependencies = ["py-app"]

[tool.turbo]
name = "acme"

[tool.uv.workspace]
members = ["packages/*"]
exclude = ["packages/skipped"]

[tool.uv.sources]
py-app = { workspace = true }
"#,
            )
            .unwrap();
        for (dir, contents) in [
            (
                "py-app",
                r#"
[project]
name = "py-app"
version = "0.1.0"
dependencies = ["py-lib", "click>=8.1"]

[dependency-groups]
dev = ["pytest>=8", "py-lib"]

[tool.uv.sources]
py-lib = { workspace = true }
"#,
            ),
            (
                "py-lib",
                r#"
[project]
name = "Py_Lib"
version = "0.1.0"
dependencies = ["six>=1.16"]
"#,
            ),
            (
                "skipped",
                r#"
[project]
name = "skipped"
version = "0.1.0"
"#,
            ),
        ] {
            let package_dir = root.join_components(&["packages", dir]);
            package_dir.create_dir_all().unwrap();
            package_dir
                .join_component(PYPROJECT_TOML)
                .create_with_contents(contents)
                .unwrap();
        }
    }

    #[test]
    fn test_discover_workspace() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        write_workspace(&root);

        let workspace = discover_workspace(&root).unwrap();
        assert_eq!(workspace.name.as_deref(), Some("acme"));
        assert_eq!(workspace.root_project_name.as_deref(), Some("root-project"));
        let names: Vec<&str> = workspace
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        // Members are normalized (Py_Lib -> py-lib) and excluded globs are
        // subtracted.
        assert_eq!(names, ["py-app", "py-lib"]);

        let py_app = &workspace.packages[0];
        // py-lib is declared as both a production dependency and a
        // dependency-group entry; both facts are kept (like Cargo), and both
        // target the same internal package.
        assert_eq!(py_app.relationships.len(), 2);
        assert!(
            py_app
                .relationships
                .iter()
                .all(|relationship| relationship.declaration_name() == "py-lib"
                    && relationship.orders_tasks())
        );
        assert!(workspace.packages[1].relationships.is_empty());
    }

    #[test]
    fn test_discover_workspace_without_pyproject_is_empty() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let workspace = discover_workspace(&root).unwrap();
        assert!(workspace.packages.is_empty());
        assert!(workspace.name.is_none());
    }

    #[test]
    fn test_buildable_member_detection() {
        let virtual_project: PyProjectManifest =
            toml::from_str("[project]\nname = \"app\"\n").unwrap();
        assert!(!virtual_project.is_buildable());

        let standard_project: PyProjectManifest = toml::from_str(
            "[project]\nname = \"app\"\n[build-system]\nrequires = [\"hatchling\"]\n",
        )
        .unwrap();
        assert!(standard_project.is_buildable());

        let legacy_project: PyProjectManifest =
            toml::from_str("[project]\nname = \"app\"\n[tool.uv]\npackage = true\n").unwrap();
        assert!(legacy_project.is_buildable());

        let explicit_build: PyProjectManifest = toml::from_str(
            "[project]\nname = \"app\"\n[build-system]\nrequires = \
             [\"hatchling\"]\n[tool.uv]\npackage = false\n",
        )
        .unwrap();
        assert!(explicit_build.is_buildable());
    }

    #[test]
    fn test_dev_cycle_breaks_deterministically() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                r#"
[tool.turbo]
name = "acme"

[tool.uv.workspace]
members = ["packages/*"]
"#,
            )
            .unwrap();
        for (name, dependencies, group) in [
            ("pkg-a", r#"["pkg-b"]"#, "[]"),
            ("pkg-b", "[]", r#"["pkg-a"]"#),
        ] {
            let source = if name == "pkg-a" { "pkg-b" } else { "pkg-a" };
            let dir = root.join_components(&["packages", name]);
            dir.create_dir_all().unwrap();
            dir.join_component(PYPROJECT_TOML)
                .create_with_contents(format!(
                    r#"
[project]
name = "{name}"
version = "0.1.0"
dependencies = {dependencies}

[dependency-groups]
dev = {group}

[tool.uv.sources]
{source} = {{ workspace = true }}
"#
                ))
                .unwrap();
        }

        let workspace = discover_workspace(&root).unwrap();
        let pkg_a = &workspace.packages[0];
        let pkg_b = &workspace.packages[1];
        // a -> b is a production edge; b -> a is a dev edge that would form
        // a cycle, so it demotes to a non-ordering input edge.
        assert!(pkg_a.relationships[0].orders_tasks());
        assert!(!pkg_b.relationships[0].orders_tasks());
    }

    #[test]
    fn test_internal_edges_require_workspace_source() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                r#"
[tool.turbo]
name = "acme"

[tool.uv.workspace]
members = ["packages/*"]

[tool.uv.sources]
inherited = { workspace = true }
overridden = { workspace = true }
"#,
            )
            .unwrap();
        for (name, contents) in [
            (
                "app",
                r#"
[project]
name = "app"
dependencies = ["inherited", "overridden", "same-name"]

[tool.uv.sources]
overridden = { index = "private" }
"#,
            ),
            ("inherited", "[project]\nname = \"inherited\"\n"),
            ("overridden", "[project]\nname = \"overridden\"\n"),
            ("same-name", "[project]\nname = \"same-name\"\n"),
        ] {
            let dir = root.join_components(&["packages", name]);
            dir.create_dir_all().unwrap();
            dir.join_component(PYPROJECT_TOML)
                .create_with_contents(contents)
                .unwrap();
        }

        let workspace = discover_workspace(&root).unwrap();
        let app = workspace
            .packages
            .iter()
            .find(|package| package.name == "app")
            .unwrap();
        assert_eq!(app.relationships.len(), 1);
        assert_eq!(app.relationships[0].declaration_name(), "inherited");
    }

    #[test]
    fn test_legacy_uv_dev_dependencies_create_edges() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                "[tool.turbo]\nname = \"acme\"\n[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
            )
            .unwrap();
        for (name, contents) in [
            (
                "app",
                "[project]\nname = \"app\"\n[tool.uv]\ndev-dependencies = \
                 [\"lib\"]\n[tool.uv.sources]\nlib = { workspace = true }\n",
            ),
            ("lib", "[project]\nname = \"lib\"\n"),
        ] {
            let dir = root.join_components(&["packages", name]);
            dir.create_dir_all().unwrap();
            dir.join_component(PYPROJECT_TOML)
                .create_with_contents(contents)
                .unwrap();
        }
        let workspace = discover_workspace(&root).unwrap();
        assert_eq!(workspace.packages[0].relationships.len(), 1);
        assert_eq!(
            workspace.packages[0].relationships[0].declaration_name(),
            "lib"
        );
    }

    #[test]
    fn test_unsafe_workspace_globs_are_rejected() {
        for pattern in ["../outside", "/outside", "C:/outside", r"..\outside"] {
            let tempdir = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
            root.join_component(PYPROJECT_TOML)
                .create_with_contents(format!("[tool.uv.workspace]\nmembers = [{pattern:?}]\n"))
                .unwrap();
            assert!(matches!(
                discover_workspace(&root),
                Err(Error::UnsafeWorkspaceGlob(_))
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_recursive_glob_does_not_follow_external_symlink() {
        use std::os::unix::fs::symlink;

        let tempdir = tempfile::tempdir().unwrap();
        let temp_root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let root = temp_root.join_component("repo");
        root.create_dir_all().unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                "[tool.turbo]\nname = \"acme\"\n[tool.uv.workspace]\nmembers = [\"packages/**\"]\n",
            )
            .unwrap();
        let outside = temp_root.join_component("outside");
        outside.create_dir_all().unwrap();
        outside
            .join_component(PYPROJECT_TOML)
            .create_with_contents("[project]\nname = \"outside\"\n")
            .unwrap();
        let packages = root.join_component("packages");
        packages.create_dir_all().unwrap();
        symlink(
            outside.as_std_path(),
            packages.join_component("external").as_std_path(),
        )
        .unwrap();

        let workspace = discover_workspace(&root).unwrap();
        assert!(workspace.packages.is_empty());
    }

    #[test]
    fn test_missing_workspace_table_discovers_nothing() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents("[project]\nname = \"solo\"\nversion = \"0.1.0\"\n")
            .unwrap();
        let workspace = discover_workspace(&root).unwrap();
        assert!(workspace.packages.is_empty());
    }

    #[test]
    fn test_duplicate_normalized_names_error() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents("[tool.uv.workspace]\nmembers = [\"packages/*\"]\n")
            .unwrap();
        for (dir, name) in [("one", "my_pkg"), ("two", "My-Pkg")] {
            let package_dir = root.join_components(&["packages", dir]);
            package_dir.create_dir_all().unwrap();
            package_dir
                .join_component(PYPROJECT_TOML)
                .create_with_contents(format!(
                    "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
                ))
                .unwrap();
        }
        let error = discover_workspace(&root).unwrap_err();
        assert!(matches!(error, Error::DuplicateMemberName { .. }));
    }

    #[test]
    fn test_workspace_name_collision_errors() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                "[tool.turbo]\nname = \"py-app\"\n\n[tool.uv.workspace]\nmembers = \
                 [\"packages/*\"]\n",
            )
            .unwrap();
        let package_dir = root.join_components(&["packages", "py-app"]);
        package_dir.create_dir_all().unwrap();
        package_dir
            .join_component(PYPROJECT_TOML)
            .create_with_contents("[project]\nname = \"py-app\"\nversion = \"0.1.0\"\n")
            .unwrap();
        let error = discover_workspace(&root).unwrap_err();
        assert!(matches!(error, Error::WorkspaceNameCollision { .. }));
    }

    #[test]
    fn test_invalid_workspace_name_errors() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                "[tool.turbo]\nname = \"not a name\"\n\n[tool.uv.workspace]\nmembers = []\n",
            )
            .unwrap();
        let error = discover_workspace(&root).unwrap_err();
        assert!(matches!(error, Error::InvalidWorkspaceName { .. }));
    }
}
