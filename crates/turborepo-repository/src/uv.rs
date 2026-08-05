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
//! Buildable packages register `build` (`uv build --package=<name>`), and all
//! packages register `format` and `check`. A synthetic package
//! anchored at the root `pyproject.toml` and depending on every member
//! represents the workspace itself; it registers workspace-wide versions of the
//! same quality tasks. Every other task comes from normal task definitions (via
//! the `command` map's `python` key). The
//! workspace package's name is declared by the user in the root manifest —
//! using Turborepo with Python requires naming the workspace:
//!
//! ```toml
//! [tool.turbo]
//! name = "acme"
//! ```
//!
//! External dependencies hash from `uv.lock` per member (see
//! [`external_closures`]), scoped to each member's transitive closure, so a
//! dependency bump only invalidates the packages that depend on it.
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
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    change_knowledge::ChangeObservation,
    external_resolution::{
        ExternalPackageIdentity, ExternalResolutionData, ExternalResolutionDomain,
        PackageResolution, ResolutionCompleteness,
    },
    package_json::PackageJson,
    prune_knowledge::{PruneDomain, PrunePlan},
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
    #[error("failed to parse root pyproject.toml: {0}")]
    ManifestEdit(#[from] Box<toml_edit::TomlError>),
    #[error("root pyproject.toml has no [tool.uv.workspace] table")]
    NotAWorkspace,
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
    #[error("uv.lock is required for Python workspaces. Run `uv lock` and commit the result.")]
    MissingLockfile,
    #[error("failed to read uv.lock: {0}")]
    LockfileRead(#[source] io::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::UvLockError),
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
    ResolutionPath(#[from] turbopath::PathError),
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
fn dist_name(normalized_name: &str) -> String {
    normalized_name.replace('-', "_")
}

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
    #[serde(default, rename = "default-groups")]
    default_groups: Option<toml::Value>,
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

    fn tool_declarations(&self, owner: DeclarationOwner) -> ToolDeclarations {
        let mut declarations = ToolDeclarations::default();
        for dependency in self
            .project
            .as_ref()
            .map(|project| project.dependencies.as_slice())
            .unwrap_or_default()
        {
            if let Some(tool) = PythonTool::from_dependency(dependency) {
                declarations.insert(tool, owner, None);
            }
        }

        let default_groups = self.default_dependency_groups();
        for dependency in self.uv().into_iter().flat_map(|uv| &uv.dev_dependencies) {
            if let Some(tool) = PythonTool::from_dependency(dependency) {
                declarations.insert(
                    tool,
                    owner,
                    Some(DependencyGroup {
                        name: "dev".to_string(),
                        is_default: default_groups.contains("dev"),
                    }),
                );
            }
        }

        for group in self.dependency_groups.keys() {
            let mut dependencies = Vec::new();
            self.walk_dependency_group(group, &mut HashSet::new(), &mut dependencies);
            let declaration_group = DependencyGroup {
                name: group.clone(),
                is_default: default_groups.contains(group),
            };
            for dependency in dependencies {
                if let Some(tool) = PythonTool::from_dependency(dependency) {
                    declarations.insert(tool, owner, Some(declaration_group.clone()));
                }
            }
        }
        declarations
    }

    fn walk_dependency_group<'a>(
        &'a self,
        group: &str,
        visiting: &mut HashSet<String>,
        dependencies: &mut Vec<&'a str>,
    ) {
        if !visiting.insert(group.to_string()) {
            return;
        }
        if let Some(entries) = self
            .dependency_groups
            .get(group)
            .and_then(toml::Value::as_array)
        {
            for entry in entries {
                if let Some(dependency) = entry.as_str() {
                    dependencies.push(dependency);
                } else if let Some(included) = entry
                    .as_table()
                    .and_then(|table| table.get("include-group"))
                    .and_then(toml::Value::as_str)
                {
                    self.walk_dependency_group(included, visiting, dependencies);
                }
            }
        }
        visiting.remove(group);
    }

    fn default_dependency_groups(&self) -> HashSet<String> {
        match self.uv().and_then(|uv| uv.default_groups.as_ref()) {
            Some(toml::Value::String(value)) if value == "all" => self
                .dependency_groups
                .keys()
                .cloned()
                .chain(std::iter::once("dev".to_string()))
                .collect(),
            Some(toml::Value::Array(values)) => values
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect(),
            _ => HashSet::from(["dev".to_string()]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PythonTool {
    Ruff,
    Black,
    Mypy,
    Ty,
    Pyright,
}

impl PythonTool {
    fn name(self) -> &'static str {
        match self {
            Self::Ruff => "ruff",
            Self::Black => "black",
            Self::Mypy => "mypy",
            Self::Ty => "ty",
            Self::Pyright => "pyright",
        }
    }

    fn from_dependency(dependency: &str) -> Option<Self> {
        if dependency.contains(';') {
            return None;
        }
        match normalize_name(pep508_name(dependency)?).as_str() {
            "ruff" => Some(Self::Ruff),
            "black" => Some(Self::Black),
            "mypy" => Some(Self::Mypy),
            "ty" => Some(Self::Ty),
            "pyright" => Some(Self::Pyright),
            _ => None,
        }
    }

    fn supports_role(self, role: ToolRole) -> bool {
        match self {
            Self::Ruff => matches!(role, ToolRole::Lint | ToolRole::Format),
            Self::Black => role == ToolRole::Format,
            Self::Mypy | Self::Ty | Self::Pyright => role == ToolRole::Check,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolRole {
    Lint,
    Format,
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationOwner {
    Root,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DependencyGroup {
    name: String,
    is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolDeclaration {
    owner: DeclarationOwner,
    group: Option<DependencyGroup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ToolDeclarations(BTreeMap<PythonTool, ToolDeclaration>);

impl ToolDeclarations {
    fn insert(
        &mut self,
        tool: PythonTool,
        owner: DeclarationOwner,
        group: Option<DependencyGroup>,
    ) {
        let rank = |group: &Option<DependencyGroup>| match group {
            None => (0, String::new()),
            Some(group) if group.is_default => (1, group.name.clone()),
            Some(group) => (2, group.name.clone()),
        };
        match self.0.entry(tool) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(ToolDeclaration { owner, group });
            }
            std::collections::btree_map::Entry::Occupied(mut entry)
                if rank(&group) < rank(&entry.get().group) =>
            {
                entry.insert(ToolDeclaration { owner, group });
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }

    fn for_role(&self, role: ToolRole) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(tool, _)| tool.supports_role(role))
                .map(|(tool, declaration)| (*tool, declaration.clone()))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionOwner {
    Root,
    Member,
    AllMembers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolExecution {
    owner: ExecutionOwner,
    group: Option<DependencyGroup>,
}

impl ToolExecution {
    fn activation_group(&self) -> Option<&str> {
        self.group
            .as_ref()
            .filter(|group| !group.is_default)
            .map(|group| group.name.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct QualityPlan {
    lint: BTreeMap<PythonTool, ToolExecution>,
    format: BTreeMap<PythonTool, ToolExecution>,
    check: BTreeMap<PythonTool, ToolExecution>,
    lint_homogeneous: bool,
    format_homogeneous: bool,
    check_homogeneous: bool,
}

impl QualityPlan {
    fn effective(root: &ToolDeclarations, member: &ToolDeclarations) -> Self {
        let role = |role| {
            let member = member.for_role(role);
            let declarations = if member.0.is_empty() {
                root.for_role(role)
            } else {
                member
            };
            declarations
                .0
                .into_iter()
                .map(|(tool, declaration)| {
                    let owner = match declaration.owner {
                        DeclarationOwner::Root => ExecutionOwner::Root,
                        DeclarationOwner::Member => ExecutionOwner::Member,
                    };
                    (
                        tool,
                        ToolExecution {
                            owner,
                            group: declaration.group,
                        },
                    )
                })
                .collect()
        };
        Self {
            lint: role(ToolRole::Lint),
            format: role(ToolRole::Format),
            check: role(ToolRole::Check),
            lint_homogeneous: true,
            format_homogeneous: true,
            check_homogeneous: true,
        }
    }

    fn homogeneous(plans: &[Self]) -> Self {
        let merge = |select: fn(&Self) -> &BTreeMap<PythonTool, ToolExecution>| {
            let Some(first) = plans.first().map(select) else {
                return (false, BTreeMap::new());
            };
            if !plans
                .iter()
                .all(|plan| select(plan).keys().eq(first.keys()))
            {
                return (false, BTreeMap::new());
            }

            let mut tools = BTreeMap::new();
            for (tool, execution) in first {
                let executions: Vec<_> = plans.iter().map(|plan| &select(plan)[tool]).collect();
                let owner = if executions.iter().all(|candidate| {
                    candidate.owner == ExecutionOwner::Root
                        && candidate.activation_group() == execution.activation_group()
                }) {
                    ExecutionOwner::Root
                } else if executions.iter().all(|candidate| {
                    candidate.owner == ExecutionOwner::Member
                        && candidate.activation_group() == execution.activation_group()
                }) {
                    ExecutionOwner::AllMembers
                } else {
                    return (false, BTreeMap::new());
                };
                tools.insert(
                    *tool,
                    ToolExecution {
                        owner,
                        group: execution
                            .group
                            .as_ref()
                            .filter(|group| !group.is_default)
                            .cloned(),
                    },
                );
            }
            (true, tools)
        };
        let (lint_homogeneous, lint) = merge(|plan| &plan.lint);
        let (format_homogeneous, format) = merge(|plan| &plan.format);
        let (check_homogeneous, check) = merge(|plan| &plan.check);
        Self {
            lint,
            format,
            check,
            lint_homogeneous,
            format_homogeneous,
            check_homogeneous,
        }
    }

    #[allow(dead_code, reason = "quality-plan ownership assertion helper")]
    fn uses_root_tools(&self) -> bool {
        self.lint
            .values()
            .chain(self.format.values())
            .chain(self.check.values())
            .any(|execution| execution.owner == ExecutionOwner::Root)
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
    quality_plan: QualityPlan,
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
    #[allow(dead_code, reason = "consumed by native task synthesis layer")]
    quality_plan: QualityPlan,
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
            quality_plan: QualityPlan::default(),
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
            quality_plan: QualityPlan::default(),
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
    let quality_plan = QualityPlan::homogeneous(
        &packages
            .iter()
            .map(|package| package.quality_plan.clone())
            .collect::<Vec<_>>(),
    );

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
        quality_plan,
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
    let root_tools = root_manifest.tool_declarations(DeclarationOwner::Root);
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
                quality_plan: QualityPlan::effective(
                    &root_tools,
                    &manifest.tool_declarations(DeclarationOwner::Member),
                ),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Native tasks
// ---------------------------------------------------------------------------

/// How a Python-toolchain package participates in task execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvPackageKind {
    /// A buildable workspace member.
    Package,
    /// A workspace member uv cannot build as a distribution. It receives the
    /// quality tasks but no built-in build task.
    VirtualPackage,
    /// The user-named workspace aggregate hosting workspace-scoped quality
    /// tasks.
    Workspace,
}

fn uv_command_task(
    kind: UvPackageKind,
    name: &str,
    prefix: Vec<String>,
    suffix: Vec<String>,
    serial_group: Option<String>,
) -> crate::native_tasks::NativeTask {
    use crate::native_tasks::{
        NativeCommandArguments, NativeCommandProgram, NativeTask, NativeTaskContract,
        PassThroughPlacement, WorkingDirectoryPolicy,
    };

    let display = std::iter::once("uv".to_string())
        .chain(prefix.iter().cloned())
        .chain(suffix.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    NativeTask::command_task(
        name,
        display,
        NativeCommandProgram::Tool("uv".to_string()),
        NativeCommandArguments {
            prefix,
            pass_through_placement: PassThroughPlacement::BeforeSuffix,
            pass_through_separator: None,
            suffix,
        },
        serial_group,
        WorkingDirectoryPolicy::RepositoryRoot,
    )
    .with_contract(NativeTaskContract::new(
        toolchain::TaskDefaults { cache: Some(false) },
        Some(uv_task_entrypoint(kind)),
        true,
    ))
}

fn uv_task_entrypoint(kind: UvPackageKind) -> crate::native_tasks::TaskEntrypoint {
    use crate::native_tasks::TaskEntrypoint;

    match kind {
        UvPackageKind::Workspace => TaskEntrypoint::PreferredOnly,
        UvPackageKind::Package | UvPackageKind::VirtualPackage => TaskEntrypoint::Candidate,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UvTaskClass {
    Build,
    Quality,
}

fn fallback_task_class(kind: UvPackageKind, task: &str) -> Option<UvTaskClass> {
    match (kind, task) {
        (UvPackageKind::Package, "build") => Some(UvTaskClass::Build),
        (
            _,
            "lint:ruff" | "format" | "format:ruff" | "format:black" | "check" | "check:mypy"
            | "check:ty" | "check:pyright",
        ) => Some(UvTaskClass::Quality),
        _ => None,
    }
}

fn excluded_uv_task(name: &str) -> crate::native_tasks::NativeTask {
    use crate::native_tasks::{NativeTask, NativeTaskContract, TaskEntrypoint};

    NativeTask::contract_task(
        name,
        NativeTaskContract::new(
            toolchain::TaskDefaults::default(),
            Some(TaskEntrypoint::Excluded),
            false,
        ),
    )
}

/// Build the built-in uv fallbacks used when no authored command exists.
pub fn native_tasks_for_package(
    kind: UvPackageKind,
    package: &str,
    package_directory: &str,
    workspace_directories: &[String],
) -> Vec<crate::native_tasks::NativeTask> {
    let mut tasks = Vec::with_capacity(3);
    if kind == UvPackageKind::Package {
        tasks.push(uv_command_task(
            kind,
            "build",
            vec!["build".to_string(), format!("--package={package}")],
            Vec::new(),
            None,
        ));
    }

    let format_arguments = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec![
                "format".to_string(),
                "--".to_string(),
                package_directory.to_string(),
            ]
        }
        UvPackageKind::Workspace => std::iter::once("format".to_string())
            .chain(std::iter::once("--".to_string()))
            .chain(workspace_directories.iter().cloned())
            .collect(),
    };
    tasks.push(uv_command_task(
        kind,
        "format",
        format_arguments,
        Vec::new(),
        None,
    ));

    let check_arguments = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec!["check".to_string(), format!("--package={package}")]
        }
        UvPackageKind::Workspace => {
            vec!["check".to_string(), "--all-packages".to_string()]
        }
    };
    tasks.push(uv_command_task(
        kind,
        "check",
        check_arguments,
        Vec::new(),
        Some("uv".to_string()),
    ));

    if kind != UvPackageKind::Package {
        tasks.push(excluded_uv_task("build"));
    }
    tasks
}

fn aggregate_task(
    kind: UvPackageKind,
    name: &str,
    children: Vec<String>,
) -> crate::native_tasks::NativeTask {
    use crate::native_tasks::{NativeTask, NativeTaskContract};

    NativeTask::aggregate(name, children).with_contract(NativeTaskContract::new(
        toolchain::TaskDefaults::default(),
        Some(uv_task_entrypoint(kind)),
        false,
    ))
}

fn declared_tool_task(
    kind: UvPackageKind,
    task: &str,
    tool: PythonTool,
    execution: &ToolExecution,
    package: &str,
    targets: &[String],
) -> crate::native_tasks::NativeTask {
    let mut prefix = vec!["run".to_string(), "--frozen".to_string()];
    match execution.owner {
        ExecutionOwner::Root => {}
        ExecutionOwner::Member => {
            prefix.extend(["--package".to_string(), package.to_string()]);
        }
        ExecutionOwner::AllMembers => prefix.push("--all-packages".to_string()),
    }
    if let Some(group) = execution.activation_group() {
        prefix.extend([
            "--no-default-groups".to_string(),
            "--group".to_string(),
            group.to_string(),
        ]);
    }
    prefix.push(tool.name().to_string());
    match tool {
        PythonTool::Ruff => prefix.push(
            if task.starts_with("lint") {
                "check"
            } else {
                "format"
            }
            .to_string(),
        ),
        PythonTool::Ty => prefix.push("check".to_string()),
        PythonTool::Black | PythonTool::Mypy | PythonTool::Pyright => {}
    }
    uv_command_task(kind, task, prefix, targets.to_vec(), Some("uv".to_string()))
}

fn warn_formatter_precedence(scope: &str, formatters: &[PythonTool], selected: PythonTool) {
    if formatters.len() < 2 {
        return;
    }
    let detected = formatters
        .iter()
        .map(|tool| tool.name())
        .collect::<Vec<_>>()
        .join(", ");
    let alternatives = formatters
        .iter()
        .map(|tool| format!("format:{}", tool.name()))
        .collect::<Vec<_>>()
        .join(", ");
    tracing::warn!(
        "Python scope {scope:?} declares multiple formatters ({detected}); selected {} because \
         the formatter precedence is Ruff before Black. Run a qualified task to choose \
         explicitly: {alternatives}.",
        selected.name()
    );
}

/// Layer resolved quality tools over the built-in uv fallback tasks.
fn quality_tasks_for_package(
    kind: UvPackageKind,
    package: &str,
    package_directory: &str,
    workspace_directories: &[String],
    plan: &QualityPlan,
    emit_formatter_warning: bool,
) -> Vec<crate::native_tasks::NativeTask> {
    let targets = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec![package_directory.to_string()]
        }
        UvPackageKind::Workspace => workspace_directories.to_vec(),
    };
    let mut tasks =
        native_tasks_for_package(kind, package, package_directory, workspace_directories);

    if plan.lint_homogeneous {
        let children: Vec<_> = plan
            .lint
            .iter()
            .map(|(tool, execution)| {
                let name = format!("lint:{}", tool.name());
                tasks.push(declared_tool_task(
                    kind, &name, *tool, execution, package, &targets,
                ));
                name
            })
            .collect();
        if !children.is_empty() {
            tasks.push(aggregate_task(kind, "lint", children));
        }
    }

    if plan.format_homogeneous {
        let formatters: Vec<_> = plan.format.keys().copied().collect();
        if let Some(selected) = [PythonTool::Ruff, PythonTool::Black]
            .into_iter()
            .find(|tool| plan.format.contains_key(tool))
        {
            tasks.retain(|task| task.name() != "format");
            for (tool, execution) in &plan.format {
                let name = format!("format:{}", tool.name());
                tasks.push(declared_tool_task(
                    kind, &name, *tool, execution, package, &targets,
                ));
            }
            if emit_formatter_warning {
                warn_formatter_precedence(package, &formatters, selected);
            }
            tasks.push(declared_tool_task(
                kind,
                "format",
                selected,
                &plan.format[&selected],
                package,
                &targets,
            ));
        }
    } else {
        tasks.retain(|task| task.name() != "format");
    }

    if plan.check_homogeneous {
        let children: Vec<_> = plan
            .check
            .iter()
            .map(|(tool, execution)| {
                let name = format!("check:{}", tool.name());
                tasks.push(declared_tool_task(
                    kind, &name, *tool, execution, package, &targets,
                ));
                name
            })
            .collect();
        if !children.is_empty() {
            tasks.retain(|task| task.name() != "check");
            tasks.push(aggregate_task(kind, "check", children));
        }
    } else {
        tasks.retain(|task| task.name() != "check");
    }

    const CLASSIFIED_TASKS: &[&str] = &[
        "build",
        "lint",
        "lint:ruff",
        "format",
        "format:ruff",
        "format:black",
        "check",
        "check:mypy",
        "check:ty",
        "check:pyright",
    ];
    for name in CLASSIFIED_TASKS {
        if !tasks.iter().any(|task| task.name() == *name) {
            tasks.push(excluded_uv_task(name));
        }
    }
    tasks
}

// ---------------------------------------------------------------------------
// Task contract
// ---------------------------------------------------------------------------

/// Standard uv and pip environment variables that can change what a uv
/// invocation resolves, installs, or builds. Credentials and purely
/// cosmetic settings are deliberately excluded.
pub const HASHED_ENV_VARS: &[&str] = &[
    "APPDATA",
    "HOME",
    "UV_BUILD_CONSTRAINT",
    "UV_COMPILE_BYTECODE",
    "UV_CONFIG_FILE",
    "UV_CONSTRAINT",
    "UV_DEFAULT_INDEX",
    "UV_EXCLUDE",
    "UV_EXCLUDE_NEWER",
    "UV_INDEX",
    "UV_INDEX_STRATEGY",
    "UV_INDEX_URL",
    "UV_EXTRA_INDEX_URL",
    "UV_FIND_LINKS",
    "UV_FORK_STRATEGY",
    "UV_GIT_LFS",
    "UV_INSECURE_HOST",
    "UV_LINK_MODE",
    "UV_MANAGED_PYTHON",
    "UV_NO_BUILD_ISOLATION",
    "UV_NO_BUILD_ISOLATION_PACKAGE",
    "UV_PYTHON",
    "UV_PYTHON_DOWNLOADS",
    "UV_PYTHON_PREFERENCE",
    "UV_PROJECT",
    "UV_PROJECT_ENVIRONMENT",
    "UV_NO_BINARY",
    "UV_NO_BINARY_PACKAGE",
    "UV_NO_BUILD",
    "UV_NO_BUILD_PACKAGE",
    "UV_NO_CONFIG",
    "UV_NO_EDITABLE",
    "UV_NO_MANAGED_PYTHON",
    "UV_NO_SOURCES_PACKAGE",
    "UV_NO_SYSTEM_CONFIG",
    "UV_NO_SOURCES",
    "UV_OFFLINE",
    "UV_OVERRIDE",
    "UV_RESOLUTION",
    "UV_PRERELEASE",
    "UV_SYSTEM_CERTS",
    "UV_WORKING_DIR",
    "XDG_CONFIG_HOME",
    "PIP_INDEX_URL",
    "PIP_EXTRA_INDEX_URL",
];

const UV_PATH_ENV_VARS: &[&str] = &[
    "UV_BUILD_CONSTRAINT",
    "UV_CONFIG_FILE",
    "UV_CONSTRAINT",
    "UV_EXCLUDE",
    "UV_OVERRIDE",
    "UV_PROJECT",
    "UV_WORKING_DIR",
];

fn has_untracked_uv_path_env(environment: &toolchain::TaskIOEnvironment) -> bool {
    UV_PATH_ENV_VARS
        .iter()
        .any(|name| environment.get(name).is_some())
}

fn has_untracked_uv_configuration(environment: &toolchain::TaskIOEnvironment) -> bool {
    if has_untracked_uv_path_env(environment) {
        return true;
    }
    if environment.get("UV_NO_CONFIG").is_some_and(|value| {
        !matches!(
            value.to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        )
    }) {
        return false;
    }
    let mut paths = Vec::new();
    if let Some(config_home) = environment.get("XDG_CONFIG_HOME") {
        paths.push(std::path::PathBuf::from(config_home).join("uv/uv.toml"));
    } else if let Some(home) = environment.get("HOME") {
        paths.push(std::path::PathBuf::from(home).join(".config/uv/uv.toml"));
    }
    if let Some(app_data) = environment.get("APPDATA") {
        paths.push(std::path::PathBuf::from(app_data).join("uv/uv.toml"));
    }
    if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
        paths.push(std::path::PathBuf::from(program_data).join("uv/uv.toml"));
    }
    #[cfg(unix)]
    paths.push(std::path::PathBuf::from("/etc/uv/uv.toml"));

    paths.into_iter().any(|path| path.is_file())
}

/// Input globs whose changes should invalidate a Python task's cache: the
/// workspace root manifest (workspace membership, sources, and
/// requires-python live there), uv's optional configuration file, and the
/// pinned interpreter version — expressed relative to the task's package
/// directory via `prefix` (the path from the package to the repo root;
/// empty for the workspace package). Globs that don't match anything (e.g.
/// a missing `.python-version`) simply contribute nothing.
///
/// uv.lock is deliberately absent: locked dependencies participate in each
/// package task's external-dependency hash, scoped to that package's
/// transitive closure (see [`external_closures`]), so a dependency bump
/// only invalidates the packages that actually depend on it.
pub fn hash_input_globs(prefix: &str) -> Vec<String> {
    [
        PYPROJECT_TOML,
        "uv.toml",
        ".python-version",
        "ruff.toml",
        ".ruff.toml",
        "mypy.ini",
        ".mypy.ini",
        "pyrightconfig.json",
        "setup.cfg",
        "ty.toml",
    ]
    .iter()
    .map(|rel| join_prefix(prefix, rel))
    .collect()
}

const PYTHON_CACHE_GLOBS: [&str; 6] = [
    ".venv/**",
    ".ruff_cache/**",
    ".mypy_cache/**",
    ".pyright/**",
    ".ty/**",
    "**/__pycache__/**",
];

fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// uv-specific details captured in immutable task-contract knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvTaskContract {
    kind: UvPackageKind,
    /// The package's distribution file-name stem (normalized name with `_`
    /// separators), used to derive `uv build` output globs. Empty for the
    /// workspace aggregate.
    dist_name: String,
    /// Repository-relative member directories for workspace-wide quality
    /// tasks. Empty for member packages.
    workspace_directories: Vec<String>,
}

impl UvTaskContract {
    fn new(kind: UvPackageKind, package_name: &str) -> Self {
        Self {
            kind,
            dist_name: match kind {
                UvPackageKind::Package | UvPackageKind::VirtualPackage => dist_name(package_name),
                UvPackageKind::Workspace => String::new(),
            },
            workspace_directories: Vec::new(),
        }
    }

    fn workspace(package_name: &str, workspace_directories: Vec<String>) -> Self {
        Self {
            workspace_directories,
            ..Self::new(UvPackageKind::Workspace, package_name)
        }
    }

    /// Classifies Python package sources for dependent derived-input
    /// closures. Workspace aggregates have no package source directory to
    /// include.
    pub(crate) fn dependency_source_inputs(&self) -> crate::task_contracts::DependencySourceInputs {
        match self.kind {
            UvPackageKind::Package | UvPackageKind::VirtualPackage => {
                crate::task_contracts::DependencySourceInputs::Include
            }
            UvPackageKind::Workspace => crate::task_contracts::DependencySourceInputs::Exclude,
        }
    }

    pub(crate) fn derived_task_io(
        &self,
        _package: &crate::package_graph::PackageTaskContext<'_>,
        task: &str,
        path_to_root: &str,
        dependencies: &[crate::package_graph::PackageTaskContext<'_>],
        wants_automatic_inputs: bool,
        context: &toolchain::TaskIOContext<'_>,
    ) -> Option<toolchain::DerivedTaskIO> {
        let task_class = fallback_task_class(self.kind, task)?;
        let mut io = toolchain::DerivedTaskIO {
            input_globs: hash_input_globs(path_to_root),
            env: HASHED_ENV_VARS.iter().map(|var| var.to_string()).collect(),
            ..Default::default()
        };
        io.input_globs
            .extend(PYTHON_CACHE_GLOBS.map(|cache| format!("!{cache}")));
        // These variables point at files whose contents affect uv. Until the
        // paths can be resolved against the repository safely, fail closed
        // instead of restoring an artifact hashed only by the path string.
        if has_untracked_uv_configuration(context.environment) {
            io.input_safety = toolchain::DerivedInputSafety::Untracked;
        }
        match self.kind {
            UvPackageKind::Package | UvPackageKind::VirtualPackage => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(true);
                    if task == "check" || task.starts_with("check:") {
                        let mut globs: Vec<String> = dependencies
                            .iter()
                            .filter(|dependency| {
                                dependency.task_contract().dependency_source_inputs()
                                    == crate::task_contracts::DependencySourceInputs::Include
                            })
                            .flat_map(|dependency| {
                                let directory = join_prefix(
                                    path_to_root,
                                    dependency.directory().to_unix().as_str(),
                                );
                                std::iter::once(format!("{directory}/**"))
                                    .chain(std::iter::once(format!("!{directory}/.turbo/**")))
                                    .chain(
                                        PYTHON_CACHE_GLOBS
                                            .map(|cache| format!("!{directory}/{cache}")),
                                    )
                            })
                            .collect();
                        globs.sort();
                        globs.dedup();
                        io.input_globs.extend(globs);
                    }
                }
                if task_class == UvTaskClass::Build {
                    // `uv build` writes `<dist_name>-<version>*` sdists and
                    // wheels into the workspace root's dist directory. Extra
                    // task args can relocate the output (`--out-dir`), so
                    // outputs resolve only for the bare invocation.
                    io.outputs = if context.task_args.is_some_and(|args| !args.is_empty()) {
                        toolchain::DerivedOutputs::Unavailable
                    } else {
                        toolchain::DerivedOutputs::Resolved(vec![join_prefix(
                            path_to_root,
                            &format!("dist/{}-*", self.dist_name),
                        )])
                    };
                }
            }
            UvPackageKind::Workspace => {
                if wants_automatic_inputs {
                    // The aggregate is anchored at the repository root;
                    // default-hashing the entire repository would be wrong.
                    io.package_default_inputs = Some(false);
                    let mut globs: Vec<String> = self
                        .workspace_directories
                        .iter()
                        .flat_map(|directory| {
                            let directory = join_prefix(path_to_root, directory);
                            std::iter::once(format!("{directory}/**"))
                                .chain(std::iter::once(format!("!{directory}/.turbo/**")))
                                .chain(
                                    PYTHON_CACHE_GLOBS.map(|cache| format!("!{directory}/{cache}")),
                                )
                        })
                        .collect();
                    globs.sort();
                    globs.dedup();
                    io.input_globs.extend(globs);
                }
            }
        }
        Some(io)
    }
}

// ---------------------------------------------------------------------------
// External dependency hashing
// ---------------------------------------------------------------------------

/// Per-package external dependency closures from uv.lock, for the packages'
/// external-dependency hashes.
///
/// A missing, unreadable, or unparsable lockfile is a hard error — silently
/// hashing nothing would be unsound.
pub fn external_closures(
    repo_root: &AbsoluteSystemPath,
    members: &[String],
    workspace_paths: &HashMap<String, String>,
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let lock_path = repo_root.join_component(UV_LOCK);
    let contents = match lock_path.read_to_string() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(Error::MissingLockfile);
        }
        Err(error) => return Err(Error::LockfileRead(error)),
    };
    Ok(turborepo_lockfiles::uv_external_closures(
        &contents,
        members,
        workspace_paths,
    )?)
}

fn read_lockfile(repo_root: &AbsoluteSystemPath) -> Result<String, Error> {
    match repo_root.join_component(UV_LOCK).read_to_string() {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Error::MissingLockfile),
        Err(error) => Err(Error::LockfileRead(error)),
    }
}

fn package_resolution(
    package: impl Into<String>,
    identities: &HashSet<turborepo_lockfiles::Package>,
) -> PackageResolution {
    PackageResolution::new(
        package,
        identities.iter().map(|identity| {
            ExternalPackageIdentity::new(identity.key.clone(), identity.version.clone())
        }),
    )
}

// ---------------------------------------------------------------------------
// Prune
// ---------------------------------------------------------------------------

/// Rewrite the workspace root manifest for a pruned repository.
pub fn prune_root_manifest(
    contents: &str,
    kept_dirs: &[String],
    kept_names: &HashSet<String>,
) -> Result<String, Error> {
    let mut document: toml_edit::DocumentMut = contents.parse().map_err(Box::new)?;
    let normalized_kept: HashSet<String> = kept_dirs.iter().map(|dir| normalize_dir(dir)).collect();

    let uv = document
        .get_mut("tool")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|tool| tool.get_mut("uv"))
        .and_then(|item| item.as_table_like_mut())
        .ok_or(Error::NotAWorkspace)?;

    {
        let workspace = uv
            .get_mut("workspace")
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
        workspace.remove("exclude");
    }

    if let Some(sources) = uv
        .get_mut("sources")
        .and_then(|item| item.as_table_like_mut())
    {
        let removed: Vec<String> = sources
            .iter()
            .filter(|(name, value)| {
                let workspace_target = value
                    .get("workspace")
                    .and_then(|workspace| workspace.as_bool())
                    .unwrap_or(false);
                if workspace_target {
                    return !kept_names.contains(&normalize_name(name));
                }
                value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .is_some_and(|path| !normalized_kept.contains(&normalize_dir(path)))
            })
            .map(|(name, _)| name.to_string())
            .collect();
        for name in removed {
            sources.remove(&name);
        }
    }

    Ok(document.to_string())
}

fn normalize_dir(dir: &str) -> String {
    dir.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

#[derive(Debug)]
struct UvPruneKnowledge {
    domain: crate::prune_knowledge::PruneDomainId,
    lockfile: String,
    root_manifest: String,
    package_directories: HashMap<String, String>,
    root_project_name: Option<String>,
}

impl UvPruneKnowledge {
    fn discover(
        repo_root: &AbsoluteSystemPath,
        package_directories: HashMap<String, String>,
        root_project_name: Option<String>,
        lockfile: String,
    ) -> Result<Self, Error> {
        let root_manifest = repo_root
            .join_component(PYPROJECT_TOML)
            .read_to_string()
            .map_err(|source| Error::ManifestRead {
                path: repo_root.join_component(PYPROJECT_TOML).to_string(),
                source,
            })?;
        Ok(Self {
            domain: crate::prune_knowledge::PYTHON_PRUNE_DOMAIN.clone(),
            lockfile,
            root_manifest,
            package_directories,
            root_project_name,
        })
    }
}

impl PruneDomain for UvPruneKnowledge {
    fn id(&self) -> &crate::prune_knowledge::PruneDomainId {
        &self.domain
    }

    fn plan(
        &self,
        kept_packages: &[String],
    ) -> Result<Option<PrunePlan>, crate::prune_knowledge::Error> {
        if kept_packages.is_empty() {
            return Ok(None);
        }
        let failed = |error: Error| crate::prune_knowledge::Error::Failed(Box::new(error));
        let mut roots = kept_packages.to_vec();
        if let Some(root_project) = &self.root_project_name {
            roots.push(root_project.clone());
        }
        let pruned_lock =
            turborepo_lockfiles::uv_prune_lock(&self.lockfile, &roots, &self.package_directories)
                .map_err(|error| failed(Error::Lockfile(error)))?;

        let mut kept_dirs = Vec::with_capacity(pruned_lock.members.len());
        let mut kept_names = HashSet::with_capacity(pruned_lock.members.len());
        let mut extra_packages = Vec::new();
        let requested_packages: HashSet<&str> = kept_packages.iter().map(String::as_str).collect();
        for member in &pruned_lock.members {
            if self.root_project_name.as_deref() == Some(member.as_str()) {
                kept_names.insert(member.clone());
                continue;
            }
            let Some(directory) = self.package_directories.get(member) else {
                tracing::warn!(
                    "uv.lock member {member} is not a discovered workspace package; skipping"
                );
                continue;
            };
            kept_dirs.push(directory.clone());
            kept_names.insert(member.clone());
            if !requested_packages.contains(member.as_str()) {
                extra_packages.push(member.clone());
            }
        }
        let pruned_manifest =
            prune_root_manifest(&self.root_manifest, &kept_dirs, &kept_names).map_err(failed)?;
        Ok(Some(PrunePlan {
            extra_packages,
            root_files: vec![
                (UV_LOCK.to_string(), pruned_lock.lockfile),
                (PYPROJECT_TOML.to_string(), pruned_manifest),
            ],
            copy_paths: [".python-version", "uv.toml"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }))
    }
}

fn uv_change_observation(package_directories: &[String]) -> ChangeObservation {
    let mut observation = ChangeObservation::new()
        .with_rediscovery_file_name(PYPROJECT_TOML)
        .with_resolution_path(UV_LOCK)
        .with_ignore_prefix(".venv")
        .with_ignore_prefix("dist");
    for directory in std::iter::once("").chain(package_directories.iter().map(String::as_str)) {
        for cache in [
            ".ruff_cache",
            ".mypy_cache",
            ".pyright",
            ".ty",
            "__pycache__",
        ] {
            observation = observation.with_ignore_prefix(join_prefix(directory, cache));
        }
    }
    observation
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

            let lockfile = read_lockfile(&self.repo_root)
                .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            let mut package_directories: HashMap<String, String> = packages
                .iter()
                .map(|package| {
                    let directory = package.manifest_path.parent().ok_or_else(|| {
                        Error::InvalidMemberManifestPath(package.manifest_path.to_string())
                    })?;
                    let directory = AnchoredSystemPathBuf::new(&self.repo_root, directory)?;
                    Ok((package.name.clone(), directory.to_unix().to_string()))
                })
                .collect::<Result<_, Error>>()
                .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;
            if let Some(root_project) = &workspace.root_project_name {
                package_directories.insert(root_project.clone(), ".".to_string());
            }
            let mut workspace_directories: Vec<String> = packages
                .iter()
                .filter_map(|package| package_directories.get(&package.name).cloned())
                .collect();
            workspace_directories.sort();
            workspace_directories.dedup();
            let change_observation = uv_change_observation(&workspace_directories);
            let prune_domain = UvPruneKnowledge::discover(
                &self.repo_root,
                package_directories.clone(),
                workspace.root_project_name.clone(),
                lockfile.clone(),
            )
            .map_err(|error| toolchain::Error::Failed(Box::new(error)))?;

            // Each package contributes its already-classified native
            // internal relationships directly. External dependencies (locked
            // registry/git/URL distributions) participate in each package
            // task's hash through the same external-dependency mechanism JS
            // packages use, scoped to the package's transitive closure — a
            // dependency bump only invalidates packages that actually depend
            // on it.
            let mut closure_members: Vec<String> = packages
                .iter()
                .map(|package| package.name.clone())
                .collect();
            if let Some(root_project) = &workspace.root_project_name {
                closure_members.push(root_project.clone());
            }
            let mut closures = turborepo_lockfiles::uv_external_closures(
                &lockfile,
                &closure_members,
                &package_directories,
            )
            .map_err(Error::from)
            .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;

            // The workspace-scoped closure covers every member plus the root
            // project's own dependencies (when the root is a package).
            let workspace_externals: HashSet<turborepo_lockfiles::Package> =
                closures.values().flatten().cloned().collect();

            let mut discovered = Vec::with_capacity(packages.len() + 1);
            let mut resolutions = Vec::with_capacity(packages.len() + 1);
            let mut package_names = Vec::with_capacity(packages.len());
            for package in packages {
                let kind = if package.buildable {
                    UvPackageKind::Package
                } else {
                    UvPackageKind::VirtualPackage
                };
                let package_directory = package_directories
                    .get(&package.name)
                    .map_or(".", String::as_str);
                let native_tasks = quality_tasks_for_package(
                    kind,
                    &package.name,
                    package_directory,
                    &[],
                    &package.quality_plan,
                    !workspace.quality_plan.format_homogeneous,
                );
                let task_contract = UvTaskContract::new(kind, &package.name);
                let mut external_dependencies = closures.remove(&package.name).unwrap_or_default();
                if package.quality_plan.uses_root_tools() {
                    // Root-owned tools execute against the root environment.
                    external_dependencies.extend(workspace_externals.iter().cloned());
                }
                resolutions.push(package_resolution(
                    package.name.clone(),
                    &external_dependencies,
                ));
                package_names.push(package.name.clone());
                discovered.push(
                    DiscoveredPackage::package(
                        Some(package.name),
                        PackageJson::default(),
                        package.manifest_path,
                    )
                    .with_native_relationships(package.relationships)
                    .with_native_tasks(native_tasks)
                    .with_task_contract(
                        crate::task_contracts::ScopeTaskContract::python(task_contract),
                    ),
                );
            }

            // The workspace aggregate, anchored at the root pyproject.toml
            // and named by the user via `[tool.turbo] name`. It depends on
            // every package so `--affected` and dependent-filters propagate
            // package changes to it.
            let workspace_native_tasks = quality_tasks_for_package(
                UvPackageKind::Workspace,
                &workspace_name,
                ".",
                &workspace_directories,
                &workspace.quality_plan,
                true,
            );
            let workspace_task_contract =
                UvTaskContract::workspace(&workspace_name, workspace_directories);
            package_names.sort();
            let workspace_relationships = package_names
                .into_iter()
                .map(|name| Relationship::internal(name, DependencyKind::Production))
                .collect();
            resolutions.push(package_resolution(
                workspace_name.clone(),
                &workspace_externals,
            ));
            discovered.push(
                DiscoveredPackage::aggregate(
                    workspace_name,
                    PackageJson::default(),
                    self.repo_root.join_component(PYPROJECT_TOML),
                )
                .with_native_relationships(workspace_relationships)
                .with_native_tasks(workspace_native_tasks)
                .with_task_contract(
                    crate::task_contracts::ScopeTaskContract::python(workspace_task_contract),
                ),
            );

            let members = resolutions
                .iter()
                .map(|resolution| resolution.package().to_string())
                .collect::<Vec<_>>();
            let resolution = ExternalResolutionDomain::new(
                crate::external_resolution::PYTHON_RESOLUTION_DOMAIN.clone(),
                ToolchainId::PYTHON,
                AnchoredSystemPathBuf::default(),
                members,
                [AnchoredSystemPathBuf::from_raw(UV_LOCK)
                    .map_err(Error::from)
                    .map_err(|error| toolchain::Error::Failed(Box::new(error)))?],
                ExternalResolutionData::Resolved {
                    completeness: ResolutionCompleteness::Complete,
                    packages: resolutions,
                },
            );
            Ok(DiscoveredPackages::new(discovered, workspace_roots)
                .with_external_resolution(resolution)
                .with_change_observation(change_observation)
                .with_prune_domain(Arc::new(prune_domain)))
        })
    }
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;

    use super::*;
    use crate::package_graph::{PackageName, PackageTaskContext, PackageTaskContextKind};

    fn resolved_args(
        task: &crate::native_tasks::NativeTask,
        pass_through: &[&str],
    ) -> Vec<OsString> {
        let repo_root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let context = PackageTaskContext::new_for_test(
            PackageName::from("app"),
            &repo_root,
            turbopath::AnchoredSystemPath::new("packages/app").unwrap(),
            PackageTaskContextKind::Package,
            None,
        );
        let binary = std::path::Path::new(if cfg!(windows) {
            r"C:\bin\uv.exe"
        } else {
            "/bin/uv"
        });
        let pass_through = pass_through
            .iter()
            .map(|argument| (*argument).to_string())
            .collect::<Vec<_>>();

        crate::native_tasks::resolve_task_command(
            &context,
            task,
            None,
            None,
            Some(binary),
            Some(&pass_through),
            None,
        )
        .unwrap()
        .unwrap()
        .args
    }

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_name("Django"), "django");
        assert_eq!(normalize_name("My_Package"), "my-package");
        assert_eq!(normalize_name("a.b--c__d"), "a-b-c-d");
        assert_eq!(normalize_name("already-normal"), "already-normal");
        assert_eq!(normalize_name(""), "");
    }

    #[test]
    fn test_dist_name() {
        assert_eq!(dist_name("py-app"), "py_app");
        assert_eq!(dist_name("single"), "single");
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

    #[test]
    fn test_tool_declaration_sources() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[project]
dependencies = ["ruff>=0.12"]

[dependency-groups]
types = ["mypy"]

[tool.uv]
dev-dependencies = ["black"]
"#,
        )
        .unwrap();

        let declarations = manifest.tool_declarations(DeclarationOwner::Root);
        assert_eq!(
            declarations.0[&PythonTool::Ruff],
            ToolDeclaration {
                owner: DeclarationOwner::Root,
                group: None,
            }
        );
        assert_eq!(
            declarations.0[&PythonTool::Mypy]
                .group
                .as_ref()
                .unwrap()
                .name,
            "types"
        );
        let legacy = declarations.0[&PythonTool::Black].group.as_ref().unwrap();
        assert_eq!(legacy.name, "dev");
        assert!(legacy.is_default);
    }

    #[test]
    fn test_tool_declaration_recursive_includes_use_declaring_group() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[dependency-groups]
dev = [{ include-group = "quality" }]
quality = [{ include-group = "typing" }, "ruff"]
typing = ["mypy", { include-group = "missing" }]
"#,
        )
        .unwrap();

        let declarations = manifest.tool_declarations(DeclarationOwner::Member);
        for tool in [PythonTool::Ruff, PythonTool::Mypy] {
            let declaration = &declarations.0[&tool];
            assert_eq!(declaration.owner, DeclarationOwner::Member);
            assert_eq!(declaration.group.as_ref().unwrap().name, "dev");
            assert!(declaration.group.as_ref().unwrap().is_default);
        }
    }

    #[test]
    fn test_tool_declaration_include_cycles_terminate() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[dependency-groups]
dev = [{ include-group = "quality" }]
quality = ["black", { include-group = "dev" }]
self-cycle = ["pyright", { include-group = "self-cycle" }]
"#,
        )
        .unwrap();

        let declarations = manifest.tool_declarations(DeclarationOwner::Member);
        let black = declarations.0[&PythonTool::Black].group.as_ref().unwrap();
        assert_eq!(black.name, "dev");
        assert!(black.is_default);
        assert_eq!(
            declarations.0[&PythonTool::Pyright]
                .group
                .as_ref()
                .unwrap()
                .name,
            "self-cycle"
        );
    }

    #[test]
    fn test_tool_declaration_names_are_normalized() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[project]
dependencies = [
  "RuFf[format]>=0.12",
  "BLACK @ https://example.com/black.whl",
  "MyPy (>=1.0)",
  "TY",
  "PyRight",
  "not-a-tool",
]
"#,
        )
        .unwrap();

        let declarations = manifest.tool_declarations(DeclarationOwner::Member);
        assert_eq!(
            declarations.0.keys().copied().collect::<Vec<_>>(),
            [
                PythonTool::Ruff,
                PythonTool::Black,
                PythonTool::Mypy,
                PythonTool::Ty,
                PythonTool::Pyright,
            ]
        );
    }

    #[test]
    fn test_optional_only_tool_declarations_are_excluded() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[project.optional-dependencies]
quality = ["ruff", "black", "mypy"]
"#,
        )
        .unwrap();

        assert!(
            manifest
                .tool_declarations(DeclarationOwner::Member)
                .0
                .is_empty()
        );
    }

    #[test]
    fn test_marker_qualified_tool_declarations_are_excluded() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[project]
dependencies = ["ruff; python_version >= '3.12'"]

[dependency-groups]
types = ["mypy ; platform_system == 'Linux'"]

[tool.uv]
dev-dependencies = ["black; implementation_name == 'cpython'"]
"#,
        )
        .unwrap();

        assert!(
            manifest
                .tool_declarations(DeclarationOwner::Member)
                .0
                .is_empty()
        );
    }

    #[test]
    fn test_legacy_dev_declaration_respects_default_groups() {
        let default_manifest: PyProjectManifest =
            toml::from_str("[tool.uv]\ndev-dependencies = ['ruff']\n").unwrap();
        let disabled_manifest: PyProjectManifest =
            toml::from_str("[tool.uv]\ndev-dependencies = ['ruff']\ndefault-groups = []\n")
                .unwrap();

        let declaration = |manifest: &PyProjectManifest| {
            manifest
                .tool_declarations(DeclarationOwner::Member)
                .0
                .remove(&PythonTool::Ruff)
                .unwrap()
        };
        assert!(declaration(&default_manifest).group.unwrap().is_default);
        assert!(!declaration(&disabled_manifest).group.unwrap().is_default);
    }

    #[test]
    fn test_configured_default_groups() {
        let listed: PyProjectManifest = toml::from_str(
            r#"
[dependency-groups]
quality = ["ruff"]
types = ["mypy"]

[tool.uv]
default-groups = ["quality"]
"#,
        )
        .unwrap();
        let declarations = listed.tool_declarations(DeclarationOwner::Member);
        assert!(
            declarations.0[&PythonTool::Ruff]
                .group
                .as_ref()
                .unwrap()
                .is_default
        );
        assert!(
            !declarations.0[&PythonTool::Mypy]
                .group
                .as_ref()
                .unwrap()
                .is_default
        );

        let all: PyProjectManifest = toml::from_str(
            "[dependency-groups]\nquality = ['ruff']\n[tool.uv]\ndev-dependencies = \
             ['black']\ndefault-groups = 'all'\n",
        )
        .unwrap();
        assert!(
            all.tool_declarations(DeclarationOwner::Member)
                .0
                .values()
                .all(|declaration| declaration.group.as_ref().unwrap().is_default)
        );
    }

    #[test]
    fn test_quality_plan_root_defaults_and_member_role_overrides() {
        let root: PyProjectManifest = toml::from_str(
            r#"
[project]
dependencies = ["ruff", "mypy", "ty"]
"#,
        )
        .unwrap();
        let member: PyProjectManifest = toml::from_str(
            r#"
[project]
dependencies = ["black"]

[dependency-groups]
types = ["pyright"]
"#,
        )
        .unwrap();
        let root = root.tool_declarations(DeclarationOwner::Root);
        let member = member.tool_declarations(DeclarationOwner::Member);
        let plan = QualityPlan::effective(&root, &member);

        assert_eq!(
            plan.lint.keys().copied().collect::<Vec<_>>(),
            [PythonTool::Ruff]
        );
        assert_eq!(
            plan.format.keys().copied().collect::<Vec<_>>(),
            [PythonTool::Black]
        );
        assert_eq!(
            plan.check.keys().copied().collect::<Vec<_>>(),
            [PythonTool::Pyright]
        );
        assert_eq!(plan.lint[&PythonTool::Ruff].owner, ExecutionOwner::Root);
        assert_eq!(
            plan.format[&PythonTool::Black].owner,
            ExecutionOwner::Member
        );
        assert_eq!(
            plan.check[&PythonTool::Pyright]
                .group
                .as_ref()
                .map(|group| group.name.as_str()),
            Some("types")
        );
        assert!(plan.uses_root_tools());

        let inherited = QualityPlan::effective(&root, &ToolDeclarations::default());
        assert_eq!(
            inherited.format.keys().copied().collect::<Vec<_>>(),
            [PythonTool::Ruff]
        );
        assert_eq!(
            inherited.check.keys().copied().collect::<Vec<_>>(),
            [PythonTool::Mypy, PythonTool::Ty]
        );
        assert!(
            inherited
                .check
                .values()
                .all(|execution| execution.owner == ExecutionOwner::Root)
        );
    }

    #[test]
    fn test_homogeneous_and_heterogeneous_quality_plans() {
        let declarations = |contents: &str| {
            toml::from_str::<PyProjectManifest>(contents)
                .unwrap()
                .tool_declarations(DeclarationOwner::Member)
        };
        let ruff = declarations("[project]\ndependencies=['ruff']\n");
        let black = declarations("[project]\ndependencies=['black']\n");
        let quality_group = declarations("[dependency-groups]\nquality=['ruff']\n");
        let other_group = declarations("[dependency-groups]\nother=['ruff']\n");
        let effective = |member: &ToolDeclarations| {
            QualityPlan::effective(&ToolDeclarations::default(), member)
        };

        let member_owned = QualityPlan::homogeneous(&[effective(&ruff), effective(&ruff)]);
        assert!(member_owned.lint_homogeneous);
        assert!(member_owned.format_homogeneous);
        assert_eq!(
            member_owned.lint[&PythonTool::Ruff].owner,
            ExecutionOwner::AllMembers
        );

        let grouped =
            QualityPlan::homogeneous(&[effective(&quality_group), effective(&quality_group)]);
        assert_eq!(
            grouped.lint[&PythonTool::Ruff]
                .group
                .as_ref()
                .map(|group| group.name.as_str()),
            Some("quality")
        );
        let incompatible_groups =
            QualityPlan::homogeneous(&[effective(&quality_group), effective(&other_group)]);
        assert!(!incompatible_groups.lint_homogeneous);

        let different_tools = QualityPlan::homogeneous(&[effective(&ruff), effective(&black)]);
        assert!(!different_tools.lint_homogeneous);
        assert!(!different_tools.format_homogeneous);
        assert!(different_tools.check_homogeneous);

        let root_manifest: PyProjectManifest =
            toml::from_str("[project]\ndependencies=['ruff']\n").unwrap();
        let root = root_manifest.tool_declarations(DeclarationOwner::Root);
        let root_owned = QualityPlan::effective(&root, &ToolDeclarations::default());
        let root_workspace = QualityPlan::homogeneous(&[root_owned.clone(), root_owned.clone()]);
        assert_eq!(
            root_workspace.lint[&PythonTool::Ruff].owner,
            ExecutionOwner::Root
        );
        assert!(root_workspace.uses_root_tools());

        let mixed_owners = QualityPlan::homogeneous(&[root_owned, effective(&ruff)]);
        assert!(!mixed_owners.lint_homogeneous);
        assert!(!mixed_owners.format_homogeneous);
    }

    fn write_workspace(root: &AbsoluteSystemPath) {
        root.join_component(PYPROJECT_TOML)
            .create_with_contents(
                r#"
[project]
name = "root-project"
version = "0.1.0"
dependencies = ["py-app", "mypy"]

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
dependencies = ["py-lib", "click>=8.1", "ruff"]

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
        assert_eq!(
            py_app.quality_plan.lint[&PythonTool::Ruff].owner,
            ExecutionOwner::Member
        );
        assert!(!workspace.quality_plan.lint_homogeneous);
        assert!(!workspace.quality_plan.format_homogeneous);
        assert!(workspace.quality_plan.check_homogeneous);
        assert_eq!(
            workspace.quality_plan.check[&PythonTool::Mypy].owner,
            ExecutionOwner::Root
        );
        assert!(workspace.packages.iter().all(|package| {
            package.quality_plan.check[&PythonTool::Mypy].owner == ExecutionOwner::Root
        }));
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
    fn test_uv_path_environment_disables_automatic_inputs() {
        let environment = toolchain::TaskIOEnvironment::new(HashMap::from([(
            "UV_CONFIG_FILE".to_string(),
            "/outside/uv.toml".to_string(),
        )]));
        assert!(has_untracked_uv_configuration(&environment));
        assert!(!has_untracked_uv_configuration(
            &toolchain::TaskIOEnvironment::default()
        ));
    }

    #[test]
    fn test_no_config_does_not_hide_other_path_inputs() {
        let environment = toolchain::TaskIOEnvironment::new(HashMap::from([
            ("UV_NO_CONFIG".to_string(), "true".to_string()),
            (
                "UV_BUILD_CONSTRAINT".to_string(),
                "/outside/constraints.txt".to_string(),
            ),
        ]));
        assert!(has_untracked_uv_configuration(&environment));
    }

    #[test]
    fn test_user_uv_config_disables_automatic_inputs() {
        let tempdir = tempfile::tempdir().unwrap();
        let config_dir = tempdir.path().join("uv");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("uv.toml"), "offline = true\n").unwrap();
        let environment = toolchain::TaskIOEnvironment::new(HashMap::from([(
            "XDG_CONFIG_HOME".to_string(),
            tempdir.path().to_string_lossy().to_string(),
        )]));
        assert!(has_untracked_uv_configuration(&environment));
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

    #[test]
    fn test_quality_task_fallbacks_preserve_build() {
        let tasks = quality_tasks_for_package(
            UvPackageKind::Package,
            "py-app",
            "packages/py-app",
            &[],
            &QualityPlan::effective(&ToolDeclarations::default(), &ToolDeclarations::default()),
            true,
        );
        let display = |name| {
            tasks
                .iter()
                .find(|task| task.name() == name)
                .and_then(|task| task.display())
        };
        assert_eq!(display("build"), Some("uv build --package=py-app"));
        assert_eq!(display("format"), Some("uv format -- packages/py-app"));
        assert_eq!(display("check"), Some("uv check --package=py-app"));
        let build = tasks.iter().find(|task| task.name() == "build").unwrap();
        assert_eq!(build.contract().defaults().cache, Some(false));
        assert_eq!(
            build.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
        assert!(build.contract().derives_io());

        let lint = tasks.iter().find(|task| task.name() == "lint").unwrap();
        assert!(!lint.participates());
        assert_eq!(
            lint.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Excluded)
        );
    }

    #[test]
    fn test_exact_root_and_member_quality_commands() {
        let root: PyProjectManifest = toml::from_str(
            "[dependency-groups]\nquality=['ruff']\n[tool.uv]\ndefault-groups=['quality']\n",
        )
        .unwrap();
        let member: PyProjectManifest =
            toml::from_str("[dependency-groups]\ntypes=['pyright']\n").unwrap();
        let plan = QualityPlan::effective(
            &root.tool_declarations(DeclarationOwner::Root),
            &member.tool_declarations(DeclarationOwner::Member),
        );
        let tasks = quality_tasks_for_package(
            UvPackageKind::VirtualPackage,
            "app",
            "packages/app",
            &[],
            &plan,
            true,
        );
        let task = |name| tasks.iter().find(|task| task.name() == name).unwrap();
        assert_eq!(
            task("lint:ruff").display(),
            Some("uv run --frozen ruff check packages/app")
        );
        assert_eq!(
            task("check:pyright").display(),
            Some(
                "uv run --frozen --package app --no-default-groups --group types pyright \
                 packages/app"
            )
        );
        let arguments = &task("check:pyright").command().unwrap().arguments;
        assert_eq!(
            arguments.pass_through_placement,
            crate::native_tasks::PassThroughPlacement::BeforeSuffix
        );
        assert_eq!(arguments.suffix, ["packages/app"]);
        assert_eq!(
            resolved_args(task("lint:ruff"), &["--fix"]),
            ["run", "--frozen", "ruff", "check", "--fix", "packages/app",].map(OsString::from)
        );
        assert_eq!(
            resolved_args(task("check:pyright"), &["--warnings"]),
            [
                "run",
                "--frozen",
                "--package",
                "app",
                "--no-default-groups",
                "--group",
                "types",
                "pyright",
                "--warnings",
                "packages/app",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn test_workspace_all_members_command() {
        let manifest: PyProjectManifest =
            toml::from_str("[project]\ndependencies=['ruff']\n").unwrap();
        let declarations = manifest.tool_declarations(DeclarationOwner::Member);
        let member = QualityPlan::effective(&ToolDeclarations::default(), &declarations);
        let plan = QualityPlan::homogeneous(&[member.clone(), member]);
        let tasks = quality_tasks_for_package(
            UvPackageKind::Workspace,
            "acme",
            ".",
            &["packages/one".to_string(), "packages/two".to_string()],
            &plan,
            true,
        );
        let lint = tasks
            .iter()
            .find(|task| task.name() == "lint:ruff")
            .unwrap();
        assert_eq!(
            lint.display(),
            Some("uv run --frozen --all-packages ruff check packages/one packages/two")
        );
        assert_eq!(
            resolved_args(lint, &["--fix", "--unsafe-fixes"]),
            [
                "run",
                "--frozen",
                "--all-packages",
                "ruff",
                "check",
                "--fix",
                "--unsafe-fixes",
                "packages/one",
                "packages/two",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn test_quality_aggregates_fan_out_and_ruff_formats_canonically() {
        let manifest: PyProjectManifest =
            toml::from_str("[project]\ndependencies=['ruff', 'black', 'mypy', 'ty', 'pyright']\n")
                .unwrap();
        let declarations = manifest.tool_declarations(DeclarationOwner::Member);
        let plan = QualityPlan::effective(&ToolDeclarations::default(), &declarations);
        let tasks = quality_tasks_for_package(
            UvPackageKind::VirtualPackage,
            "app",
            "app",
            &[],
            &plan,
            true,
        );
        let task = |name| tasks.iter().find(|task| task.name() == name).unwrap();
        assert_eq!(
            task("format:ruff").display(),
            Some("uv run --frozen --package app ruff format app")
        );
        assert_eq!(
            task("format:black").display(),
            Some("uv run --frozen --package app black app")
        );
        assert_eq!(
            task("format").display(),
            task("format:ruff").display(),
            "the unqualified formatter must prefer Ruff while retaining Black's qualified task"
        );
        assert_eq!(
            task("check:mypy").display(),
            Some("uv run --frozen --package app mypy app")
        );
        assert_eq!(
            task("check:ty").display(),
            Some("uv run --frozen --package app ty check app")
        );
        assert_eq!(
            task("check:pyright").display(),
            Some("uv run --frozen --package app pyright app")
        );
        assert!(matches!(
            task("lint").execution(),
            crate::native_tasks::NativeTaskExecution::Aggregate(children)
                if children.as_ref() == ["lint:ruff"]
        ));
        let check = task("check");
        assert!(matches!(
            check.execution(),
            crate::native_tasks::NativeTaskExecution::Aggregate(children)
                if children.as_ref() == ["check:mypy", "check:ty", "check:pyright"]
        ));
        assert!(check.command().is_none());
        assert_eq!(check.contract().defaults().cache, None);
        assert_eq!(
            check.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
        assert!(!check.contract().derives_io());

        let mypy = task("check:mypy");
        assert_eq!(mypy.command().unwrap().serial_group.as_deref(), Some("uv"));
        assert_eq!(mypy.contract().defaults().cache, Some(false));
        assert_eq!(
            mypy.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
        assert!(mypy.contract().derives_io());
    }

    #[test]
    fn test_derived_outputs_for_build() {
        let contract = UvTaskContract::new(UvPackageKind::Package, "py-app");
        assert_eq!(contract.dist_name, "py_app");
        assert_eq!(
            hash_input_globs("../.."),
            [
                "pyproject.toml",
                "uv.toml",
                ".python-version",
                "ruff.toml",
                ".ruff.toml",
                "mypy.ini",
                ".mypy.ini",
                "pyrightconfig.json",
                "setup.cfg",
                "ty.toml",
            ]
            .map(|path| format!("../../{path}"))
        );
    }

    #[test]
    fn test_check_inputs_include_dependency_sources_and_exclude_python_caches() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let package_directory = turbopath::AnchoredSystemPath::new("packages/app").unwrap();
        let dependency_directory = turbopath::AnchoredSystemPath::new("packages/lib").unwrap();
        let package = PackageTaskContext::new_for_test(
            PackageName::from("app"),
            &root,
            package_directory,
            PackageTaskContextKind::Package,
            None,
        );
        let dependency = PackageTaskContext::new_for_test_with_native_tasks(
            PackageName::from("lib"),
            &root,
            dependency_directory,
            PackageTaskContextKind::Package,
            None,
            None,
            Some(crate::task_contracts::ScopeTaskContract::python(
                UvTaskContract::new(UvPackageKind::Package, "lib"),
            )),
        );
        let environment = toolchain::TaskIOEnvironment::default();
        let context = toolchain::TaskIOContext {
            task_args: None,
            environment: &environment,
        };
        let io = UvTaskContract::new(UvPackageKind::VirtualPackage, "app")
            .derived_task_io(
                &package,
                "check:mypy",
                "../..",
                &[dependency],
                true,
                &context,
            )
            .unwrap();

        assert!(
            io.input_globs
                .contains(&"../../packages/lib/**".to_string())
        );
        assert!(
            io.input_globs
                .contains(&"!../../packages/lib/.mypy_cache/**".to_string())
        );
        assert!(io.input_globs.contains(&"!**/__pycache__/**".to_string()));
    }

    #[test]
    fn test_python_watch_ignores_root_and_member_caches() {
        let observation = uv_change_observation(&["packages/app".to_string()]);
        let expected = ChangeObservation::new()
            .with_rediscovery_file_name(PYPROJECT_TOML)
            .with_resolution_path(UV_LOCK)
            .with_ignore_prefix(".venv")
            .with_ignore_prefix("dist");
        let expected = ["", "packages/app"]
            .into_iter()
            .fold(expected, |observation, dir| {
                [
                    ".ruff_cache",
                    ".mypy_cache",
                    ".pyright",
                    ".ty",
                    "__pycache__",
                ]
                .into_iter()
                .fold(observation, |observation, cache| {
                    observation.with_ignore_prefix(join_prefix(dir, cache))
                })
            });

        assert_eq!(observation, expected);
    }

    #[test]
    fn test_prune_root_manifest() {
        let manifest = r#"
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
gone = { workspace = true }
local = { path = "packages/gone-dir" }
"#;
        let kept_names: HashSet<String> = ["py-app".to_string(), "root-project".to_string()].into();
        let pruned =
            prune_root_manifest(manifest, &["packages/py-app".to_string()], &kept_names).unwrap();
        assert!(pruned.contains(r#"members = ["packages/py-app"]"#));
        assert!(!pruned.contains("exclude"));
        assert!(pruned.contains("py-app = { workspace = true }"));
        assert!(!pruned.contains("gone"));
        assert!(!pruned.contains("local"));
        assert!(pruned.contains("name = \"root-project\""));
        assert!(pruned.contains("name = \"acme\""));
    }

    #[test]
    fn test_prune_root_manifest_requires_workspace() {
        let error =
            prune_root_manifest("[project]\nname = \"x\"\n", &[], &HashSet::new()).unwrap_err();
        assert!(matches!(error, Error::NotAWorkspace));
    }
}
