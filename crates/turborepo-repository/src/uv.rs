//! The uv toolchain: Python packages as Turborepo packages.
//!
//! Turborepo does not replace uv — uv owns resolution, environments, and
//! installation. Turborepo's job is orchestration: decide *which* packages
//! are in scope and *whether* anything changed, then hand the work to uv
//! and get out of the way. uv is the only supported Python package manager.
//!
//! Discovery parses the root `pyproject.toml`'s `[tool.uv.workspace]` table
//! in-process: member globs are expanded against the filesystem and each
//! member's `pyproject.toml` is parsed for its identity, task configuration,
//! and internal relationships. Exact external resolution comes from
//! `uv workspace metadata --frozen --offline`, avoiding direct interpretation
//! of uv's unstable lockfile schema. Discovery also probes uv and its selected
//! Python interpreter so command tasks can be cached safely.
//!
//! Buildable packages register `build` (`uv build --package=<name>`), and all
//! packages register `format` and `check`. A synthetic package
//! anchored at the root `pyproject.toml` and depending on every member
//! represents the workspace itself; it registers workspace-wide versions of the
//! same quality tasks. A root pytest declaration registers one workspace-wide
//! `test` task, while a member pytest declaration registers `test` only for
//! that member. Every other task comes from normal task definitions (via the
//! `command` map's `python` key). The
//! workspace package's name is declared by the user in the root manifest —
//! using Turborepo with Python requires naming the workspace:
//!
//! ```toml
//! [tool.turbo]
//! name = "acme"
//! ```
//!
//! External dependencies hash from uv's workspace metadata per member (see
//! [`external_closures`]), scoped to each member's transitive closure, so a
//! dependency bump only invalidates the packages that depend on it. Resolved
//! uv and Python identities participate in every Python package hash.
//!
//! Support is experimental and gated behind
//! `futureFlags.experimentalPythonWorkspaces`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{self, Read},
    process::Command,
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    #[error("failed to run `uv workspace metadata --frozen`: {0}")]
    MetadataCommand(String),
    #[error("failed to parse `uv workspace metadata` output: {0}")]
    MetadataParse(#[source] serde_json::Error),
    #[error("uv workspace metadata references unknown resolution node {0:?}")]
    UnknownMetadataNode(String),
    #[error(
        "uv workspace metadata contains reachable local dependency {0:?}, but only uv workspace \
         members are supported. Add it to [tool.uv.workspace] or replace it with a non-local \
         source."
    )]
    UnsupportedLocalMetadataNode(String),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::UvLockError),
    #[error("uv workspace member manifest has no parent directory: {0}")]
    InvalidMemberManifestPath(String),
    #[error("uv workspace metadata returned member path outside the repository: {0}")]
    MetadataMemberOutsideRepository(String),
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
    build_system: Option<BuildSystemTable>,
    #[serde(default, rename = "dependency-groups")]
    dependency_groups: BTreeMap<String, toml::Value>,
    tool: Option<ToolTable>,
}

#[derive(Debug, Default, Deserialize)]
struct BuildSystemTable {
    #[serde(default)]
    requires: Vec<String>,
    #[serde(rename = "build-backend")]
    build_backend: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectTable {
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
    workspace: Option<toml::Value>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Vec<String>,
    #[serde(default, rename = "default-groups")]
    default_groups: Option<toml::Value>,
    package: Option<bool>,
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

    fn has_workspace(&self) -> bool {
        self.tool
            .as_ref()
            .and_then(|tool| tool.uv.as_ref())
            .and_then(|uv| uv.workspace.as_ref())
            .is_some()
    }

    fn uv(&self) -> Option<&UvToolTable> {
        self.tool.as_ref()?.uv.as_ref()
    }

    fn is_buildable(&self) -> bool {
        self.build_system.is_some() || self.uv().and_then(|uv| uv.package) == Some(true)
    }

    fn bundled_uv_build_requirement(&self) -> Option<&str> {
        let build_system = self.build_system.as_ref()?;
        (build_system.build_backend.as_deref() == Some("uv_build")
            && build_system.requires.len() == 1
            && normalize_name(pep508_name(&build_system.requires[0])?) == "uv-build")
            .then(|| build_system.requires[0].as_str())
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
    Pytest,
}

impl PythonTool {
    fn name(self) -> &'static str {
        match self {
            Self::Ruff => "ruff",
            Self::Black => "black",
            Self::Mypy => "mypy",
            Self::Ty => "ty",
            Self::Pyright => "pyright",
            Self::Pytest => "pytest",
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
            "pytest" => Some(Self::Pytest),
            _ => None,
        }
    }

    fn supports_role(self, role: ToolRole) -> bool {
        match self {
            Self::Ruff => matches!(role, ToolRole::Lint | ToolRole::Format),
            Self::Black => role == ToolRole::Format,
            Self::Mypy | Self::Ty | Self::Pyright => role == ToolRole::Check,
            Self::Pytest => false,
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

    fn execution(&self, tool: PythonTool) -> Option<ToolExecution> {
        let declaration = self.0.get(&tool)?;
        Some(ToolExecution {
            owner: match declaration.owner {
                DeclarationOwner::Root => ExecutionOwner::Root,
                DeclarationOwner::Member => ExecutionOwner::Member,
            },
            group: declaration.group.clone(),
        })
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
    bundled_uv_build_requirement: Option<String>,
    quality_plan: QualityPlan,
    pytest: Option<ToolExecution>,
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
    pytest: Option<ToolExecution>,
}

/// Discover the uv workspace from uv's authoritative metadata, parsing member
/// manifests only for Turborepo task synthesis and dependency-kind labels.
fn discover_workspace_from_metadata(
    repo_root: &AbsoluteSystemPath,
    metadata: &UvWorkspaceMetadata,
) -> Result<DiscoveredWorkspace, Error> {
    let root_manifest_path = repo_root.join_component(PYPROJECT_TOML);
    let Some(root_manifest) = PyProjectManifest::load(&root_manifest_path)? else {
        return Ok(DiscoveredWorkspace {
            name: None,
            packages: Vec::new(),
            root_project_name: None,
            quality_plan: QualityPlan::default(),
            pytest: None,
        });
    };
    let name = workspace_name(&root_manifest)?;
    if !root_manifest.has_workspace() {
        tracing::warn!(
            "the root pyproject.toml has no [tool.uv.workspace] table; Turborepo's Python support \
             requires a uv workspace, so no Python packages were discovered"
        );
        return Ok(DiscoveredWorkspace {
            name,
            packages: Vec::new(),
            root_project_name: None,
            quality_plan: QualityPlan::default(),
            pytest: None,
        });
    }

    let real_repo_root = repo_root.to_realpath()?;
    let mut parsed = Vec::new();
    let mut root_project_name = None;
    for member in &metadata.members {
        let member_path = AbsoluteSystemPathBuf::from_cwd(&member.path)?;
        let real_member = member_path.to_realpath()?;
        if !real_member.starts_with(&real_repo_root) {
            return Err(Error::MetadataMemberOutsideRepository(member.path.clone()));
        }
        let normalized = normalize_name(&member.name);
        if real_member == real_repo_root {
            root_project_name = Some(normalized);
            continue;
        }
        let manifest_path = member_path.join_component(PYPROJECT_TOML);
        let Some(manifest) = PyProjectManifest::load(&manifest_path)? else {
            continue;
        };
        parsed.push((normalized, manifest_path, manifest));
    }
    parsed.sort_by(|left, right| left.0.cmp(&right.0));

    let root_tools = root_manifest.tool_declarations(DeclarationOwner::Root);
    let pytest = root_tools.execution(PythonTool::Pytest);
    let packages = connect_packages(parsed, metadata, &root_tools);
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
        pytest,
    })
}

/// Resolve dependency edges to package names. Dependency-group
/// (development) edges that would form a cycle remain compilation inputs
/// but do not order tasks, since PEP 735 groups permit cycles while the
/// task graph is a DAG.
fn metadata_internal_dependencies(
    metadata: &UvWorkspaceMetadata,
) -> HashMap<String, HashSet<String>> {
    let members_by_id = metadata
        .members
        .iter()
        .map(|member| (member.id.as_str(), normalize_name(&member.name)))
        .collect::<HashMap<_, _>>();
    metadata
        .members
        .iter()
        .map(|member| {
            let mut dependencies = HashSet::new();
            let mut pending = metadata
                .resolution
                .get(&member.id)
                .into_iter()
                .flat_map(|node| {
                    node.dependencies
                        .iter()
                        .chain(&node.optional_dependencies)
                        .chain(&node.dependency_groups)
                })
                .map(|dependency| dependency.id.as_str())
                .collect::<Vec<_>>();
            let mut visited = HashSet::new();
            while let Some(id) = pending.pop() {
                if !visited.insert(id) {
                    continue;
                }
                if let Some(name) = members_by_id.get(id) {
                    dependencies.insert(name.clone());
                    continue;
                }
                let Some(node) = metadata.resolution.get(id) else {
                    continue;
                };
                if node.kind.is_object() {
                    pending.extend(
                        node.dependencies
                            .iter()
                            .chain(&node.optional_dependencies)
                            .chain(&node.dependency_groups)
                            .map(|dependency| dependency.id.as_str()),
                    );
                }
            }
            (normalize_name(&member.name), dependencies)
        })
        .collect()
}

fn connect_packages(
    parsed: Vec<(String, AbsoluteSystemPathBuf, PyProjectManifest)>,
    metadata: &UvWorkspaceMetadata,
    root_tools: &ToolDeclarations,
) -> Vec<UvPackage> {
    let internal_dependencies = metadata_internal_dependencies(metadata);
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
            if to == *from
                || !internal_dependencies
                    .get(from)
                    .is_some_and(|dependencies| dependencies.contains(&to))
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
            let member_tools = manifest.tool_declarations(DeclarationOwner::Member);
            let mut package_relationships = relationships.remove(name.as_str()).unwrap_or_default();
            package_relationships
                .sort_by(|left, right| left.declaration_name().cmp(right.declaration_name()));
            package_relationships.dedup();
            UvPackage {
                name,
                manifest_path,
                relationships: package_relationships,
                buildable: manifest.is_buildable(),
                bundled_uv_build_requirement: manifest
                    .bundled_uv_build_requirement()
                    .map(str::to_string),
                quality_plan: QualityPlan::effective(root_tools, &member_tools),
                pytest: member_tools.execution(PythonTool::Pytest),
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
    cacheable: bool,
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
        toolchain::TaskDefaults {
            cache: Some(cacheable),
        },
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
    Test,
}

fn fallback_task_class(kind: UvPackageKind, task: &str) -> Option<UvTaskClass> {
    match (kind, task) {
        (UvPackageKind::Package, "build") => Some(UvTaskClass::Build),
        (
            _,
            "lint:ruff" | "format" | "format:ruff" | "format:black" | "check" | "check:mypy"
            | "check:ty" | "check:pyright",
        ) => Some(UvTaskClass::Quality),
        (_, "test") => Some(UvTaskClass::Test),
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
    build_cacheable: bool,
) -> Vec<crate::native_tasks::NativeTask> {
    let mut tasks = Vec::with_capacity(3);
    if kind == UvPackageKind::Package {
        tasks.push(uv_command_task(
            kind,
            "build",
            vec!["build".to_string(), format!("--package={package}")],
            Vec::new(),
            None,
            build_cacheable,
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
        false,
    ));

    let check_arguments = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec![
                "check".to_string(),
                "--frozen".to_string(),
                format!("--package={package}"),
            ]
        }
        UvPackageKind::Workspace => {
            vec![
                "check".to_string(),
                "--frozen".to_string(),
                "--all-packages".to_string(),
            ]
        }
    };
    tasks.push(uv_command_task(
        kind,
        "check",
        check_arguments,
        Vec::new(),
        Some("uv".to_string()),
        false,
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
    serial_group: Option<String>,
    toolchain_identified: bool,
) -> crate::native_tasks::NativeTask {
    let mut prefix = vec![
        "run".to_string(),
        "--active".to_string(),
        "--frozen".to_string(),
    ];
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
        PythonTool::Black | PythonTool::Mypy | PythonTool::Pyright | PythonTool::Pytest => {}
    }
    uv_command_task(
        kind,
        task,
        prefix,
        targets.to_vec(),
        serial_group,
        toolchain_identified && !task.starts_with("format"),
    )
}

fn pytest_task(
    kind: UvPackageKind,
    execution: &ToolExecution,
    package: &str,
    package_directory: &str,
    toolchain_identified: bool,
) -> crate::native_tasks::NativeTask {
    let targets = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec![package_directory.to_string()]
        }
        UvPackageKind::Workspace => Vec::new(),
    };
    declared_tool_task(
        kind,
        "test",
        PythonTool::Pytest,
        execution,
        package,
        &targets,
        None,
        toolchain_identified,
    )
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

/// Layer declared tools over the built-in uv fallback tasks.
fn python_tasks_for_package(
    kind: UvPackageKind,
    package: &str,
    package_directory: &str,
    workspace_directories: &[String],
    plan: &QualityPlan,
    pytest: Option<&ToolExecution>,
    emit_formatter_warning: bool,
    toolchain_identified: bool,
    build_cacheable: bool,
) -> Vec<crate::native_tasks::NativeTask> {
    let targets = match kind {
        UvPackageKind::Package | UvPackageKind::VirtualPackage => {
            vec![package_directory.to_string()]
        }
        UvPackageKind::Workspace => workspace_directories.to_vec(),
    };
    let mut tasks = native_tasks_for_package(
        kind,
        package,
        package_directory,
        workspace_directories,
        build_cacheable,
    );

    if plan.lint_homogeneous {
        let children: Vec<_> = plan
            .lint
            .iter()
            .map(|(tool, execution)| {
                let name = format!("lint:{}", tool.name());
                tasks.push(declared_tool_task(
                    kind,
                    &name,
                    *tool,
                    execution,
                    package,
                    &targets,
                    Some("uv".to_string()),
                    toolchain_identified,
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
                    kind,
                    &name,
                    *tool,
                    execution,
                    package,
                    &targets,
                    Some("uv".to_string()),
                    toolchain_identified,
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
                Some("uv".to_string()),
                toolchain_identified,
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
                    kind,
                    &name,
                    *tool,
                    execution,
                    package,
                    &targets,
                    Some("uv".to_string()),
                    toolchain_identified,
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

    if let Some(execution) = pytest {
        tasks.push(pytest_task(
            kind,
            execution,
            package,
            package_directory,
            toolchain_identified,
        ));
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
    "UV_ENV_FILE",
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
    "UV_NO_DEFAULT_GROUPS",
    "UV_NO_DEV",
    "UV_NO_EDITABLE",
    "UV_NO_ENV_FILE",
    "UV_NO_MANAGED_PYTHON",
    "UV_NO_PROJECT",
    "UV_NO_GROUP",
    "UV_NO_SOURCES_PACKAGE",
    "UV_NO_SYSTEM_CONFIG",
    "UV_NO_SOURCES",
    "UV_NO_SYNC",
    "UV_OFFLINE",
    "UV_OVERRIDE",
    "UV_RESOLUTION",
    "UV_PRERELEASE",
    "UV_SYSTEM_CERTS",
    "UV_ISOLATED",
    "UV_WORKING_DIR",
    "XDG_CONFIG_HOME",
    "PIP_INDEX_URL",
    "PIP_EXTRA_INDEX_URL",
    "PYTHONHOME",
    "PYTHONPATH",
    "VIRTUAL_ENV",
];

const UV_PATH_ENV_VARS: &[&str] = &[
    "UV_BUILD_CONSTRAINT",
    "UV_CONFIG_FILE",
    "UV_CONSTRAINT",
    "UV_ENV_FILE",
    "UV_EXCLUDE",
    "UV_OVERRIDE",
    "UV_PROJECT",
    "UV_WORKING_DIR",
    "PYTHONHOME",
    "PYTHONPATH",
];

fn effective_virtual_environment(environment: &toolchain::TaskIOEnvironment) -> &str {
    environment
        .get("VIRTUAL_ENV")
        .filter(|path| !path.is_empty())
        .or_else(|| {
            environment
                .get("UV_PROJECT_ENVIRONMENT")
                .filter(|path| !path.is_empty())
        })
        .unwrap_or(".venv")
}

fn virtual_environment_path(
    package: &crate::package_graph::PackageTaskContext<'_>,
    environment: &toolchain::TaskIOEnvironment,
) -> Option<String> {
    let configured = effective_virtual_environment(environment);
    let absolute = AbsoluteSystemPathBuf::from_unknown(package.repository_root(), configured);
    let repo_root = package.repository_root().to_realpath().ok()?;
    let absolute = absolute.to_realpath().unwrap_or(absolute);
    let repo_relative = repo_root.anchor(&absolute).ok()?;
    Some(
        turbopath::AnchoredSystemPathBuf::relative_path_between(
            &repo_root.resolve(package.directory()),
            &repo_root.resolve(&repo_relative),
        )
        .to_unix()
        .to_string(),
    )
}

fn environment_flag(environment: &toolchain::TaskIOEnvironment, name: &str) -> bool {
    environment.get(name).is_some_and(|value| {
        !matches!(
            value.to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        )
    })
}

fn has_untracked_uv_path_env(environment: &toolchain::TaskIOEnvironment) -> bool {
    UV_PATH_ENV_VARS
        .iter()
        .any(|name| environment.get(name).is_some())
}

fn has_untracked_uv_configuration(environment: &toolchain::TaskIOEnvironment) -> bool {
    if has_untracked_uv_path_env(environment) {
        return true;
    }
    if environment_flag(environment, "UV_NO_SYNC") {
        return true;
    }
    if environment_flag(environment, "UV_NO_PROJECT") {
        return true;
    }
    if environment_flag(environment, "UV_NO_CONFIG") {
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
/// uv.lock is deliberately absent: uv workspace metadata supplies each package
/// task's external-dependency hash, scoped to that package's transitive closure
/// (see [`external_closures`]), so a dependency bump only invalidates packages
/// that actually depend on it.
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
        ".pytest.ini",
        ".pytest.toml",
        "pytest.ini",
        "pytest.toml",
        "setup.py",
        "setup.cfg",
        "tox.ini",
        "ty.toml",
        "conftest.py",
    ]
    .iter()
    .map(|rel| join_prefix(prefix, rel))
    .collect()
}

const PYTHON_CACHE_GLOBS: [&str; 7] = [
    ".venv/**",
    ".pytest_cache/**",
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
        package: &crate::package_graph::PackageTaskContext<'_>,
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
        if let Some(venv) = virtual_environment_path(package, context.environment) {
            io.input_globs.push(format!("!{venv}/**"));
            io.forbidden_output_prefixes.push(venv);
        }
        // These variables point at files whose contents affect uv. Until the
        // paths can be resolved against the repository safely, fail closed
        // instead of restoring an artifact hashed only by the path string.
        if has_untracked_uv_configuration(context.environment) {
            io.input_safety = toolchain::DerivedInputSafety::Untracked;
        }
        if context.task_args.is_some_and(|args| !args.is_empty()) {
            // Native tools accept path-valued and mutating options that cannot
            // be inferred uniformly. Explicit cache configuration can opt in.
            io.input_safety = toolchain::DerivedInputSafety::Untracked;
        }
        match self.kind {
            UvPackageKind::Package | UvPackageKind::VirtualPackage => {
                if wants_automatic_inputs {
                    io.package_default_inputs = Some(true);
                    if task == "test" || task == "check" || task.starts_with("check:") {
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
                    if task_class == UvTaskClass::Test {
                        // Bare pytest can collect root tests and files outside
                        // uv members, so the workspace test hashes the whole
                        // repository rather than guessing collection roots.
                        io.package_default_inputs = Some(true);
                    } else {
                        // Quality aggregates target only discovered members;
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
            }
        }
        Some(io)
    }
}

// ---------------------------------------------------------------------------
// External dependency hashing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
struct UvPythonIdentity {
    key: String,
    version: String,
    #[serde(skip_serializing)]
    path: String,
    os: String,
    variant: String,
    implementation: String,
    arch: String,
    libc: String,
    #[serde(default)]
    binary_sha256: String,
    #[serde(default)]
    host: String,
}

struct UvToolchainIdentity {
    packages: [turborepo_lockfiles::Package; 2],
    uv_version: node_semver::Version,
}

fn bundled_uv_build_matches(requirement: &str, uv_version: &node_semver::Version) -> bool {
    let Some(name) = pep508_name(requirement) else {
        return false;
    };
    if normalize_name(name) != "uv-build" {
        return false;
    }
    let specifier = requirement[name.len()..].trim();
    if specifier.is_empty() {
        return true;
    }
    let mut range = Vec::new();
    for clause in specifier.split(',').map(str::trim) {
        let Some((operator, version)) =
            [">=", "<=", "==", ">", "<"]
                .into_iter()
                .find_map(|operator| {
                    clause
                        .strip_prefix(operator)
                        .map(|version| (operator, version))
                })
        else {
            return false;
        };
        if version.is_empty()
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            return false;
        }
        let mut release = version.split('.').collect::<Vec<_>>();
        if release.len() > 3 || release.iter().any(|component| component.is_empty()) {
            return false;
        }
        release.resize(3, "0");
        let version = release.join(".");
        range.push(format!(
            "{}{version}",
            if operator == "==" { "=" } else { operator }
        ));
    }
    node_semver::Range::parse(range.join(" ")).is_ok_and(|range| range.satisfies(uv_version))
}

fn parse_python_identity(
    stdout: &str,
    python_path: &std::path::Path,
    binary_sha256: String,
    host: String,
) -> Option<String> {
    let mut identity = serde_json::from_str::<Vec<UvPythonIdentity>>(stdout)
        .ok()?
        .into_iter()
        .find(|identity| {
            dunce::canonicalize(&identity.path).is_ok_and(|path| path == python_path)
        })?;
    identity.binary_sha256 = binary_sha256;
    identity.host = host;
    serde_json::to_string(&identity).ok()
}

fn file_sha256(path: &std::path::Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

#[cfg(target_os = "linux")]
fn host_compatibility_identity() -> Option<String> {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let runtime = ["/usr/bin/ldd", "/bin/ldd"].into_iter().find_map(|path| {
        let path = std::path::Path::new(path);
        path.exists().then(|| {
            let output = Command::new(path).arg("--version").output().ok()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let value = format!("{stdout}{stderr}");
            (!value.trim().is_empty()).then(|| value.trim().to_string())
        })?
    })?;
    Some(format!("{kernel}\n{os_release}\n{runtime}"))
}

#[cfg(target_os = "macos")]
fn host_compatibility_identity() -> Option<String> {
    let mut command = Command::new("/usr/bin/sw_vers");
    command.arg("-productVersion");
    successful_stdout(command)
}

#[cfg(windows)]
fn host_compatibility_identity() -> Option<String> {
    let cmd = std::path::PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/cmd.exe");
    let mut command = Command::new(cmd);
    command.args(["/C", "ver"]);
    successful_stdout(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn host_compatibility_identity() -> Option<String> {
    None
}

fn successful_stdout(mut command: Command) -> Option<String> {
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!stdout.is_empty()).then(|| stdout.to_string())
}

/// Resolve the exact uv frontend and Python interpreter selected for this
/// workspace. Discovery remains available without either binary, but native
/// command tasks stay uncached until both identities can be proven.
fn toolchain_identities(repo_root: &AbsoluteSystemPath) -> Option<UvToolchainIdentity> {
    // Avoid Windows verbatim (`\\?\`) paths: uv does not match them against
    // the ordinary paths returned by `uv python list`.
    let uv = dunce::canonicalize(which::which("uv").ok()?).ok()?;
    if uv.starts_with(repo_root.as_std_path()) {
        return None;
    }
    let uv_sha256 = file_sha256(&uv)?;

    let mut uv_version = Command::new(&uv);
    uv_version
        .arg("--version")
        .current_dir(repo_root.as_std_path());
    let uv_version_output = successful_stdout(uv_version)?;
    let uv_version = node_semver::Version::parse(
        uv_version_output
            .strip_prefix("uv ")?
            .split_whitespace()
            .next()?,
    )
    .ok()?;
    let uv_identity = format!("{uv_version_output}\nsha256:{uv_sha256}");

    let mut python = Command::new(&uv);
    python
        .args(["python", "find", "--resolve-links", "--no-python-downloads"])
        .current_dir(repo_root.as_std_path());
    let python = successful_stdout(python)?;
    let python_path = dunce::canonicalize(&python).ok()?;
    let python_sha256 = file_sha256(&python_path)?;
    let host = host_compatibility_identity()?;

    let mut python_identity = Command::new(&uv);
    python_identity
        .args([
            "python",
            "list",
            "--only-installed",
            "--output-format",
            "json",
        ])
        .current_dir(repo_root.as_std_path());
    let python_identity = successful_stdout(python_identity)?;
    let python_identity =
        parse_python_identity(&python_identity, &python_path, python_sha256, host)?;

    Some(UvToolchainIdentity {
        packages: [
            turborepo_lockfiles::Package {
                key: "uv".to_string(),
                version: uv_identity,
            },
            turborepo_lockfiles::Package {
                key: "python".to_string(),
                version: python_identity,
            },
        ],
        uv_version,
    })
}

#[derive(Debug, Clone, Deserialize)]
struct UvWorkspaceMetadata {
    members: Vec<UvMetadataMember>,
    resolution: HashMap<String, UvMetadataNode>,
}

#[derive(Debug, Clone, Deserialize)]
struct UvMetadataMember {
    name: String,
    path: String,
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UvMetadataNode {
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    kind: serde_json::Value,
    source: Option<serde_json::Value>,
    #[serde(default)]
    dependencies: Vec<UvMetadataDependency>,
    #[serde(default)]
    optional_dependencies: Vec<UvMetadataDependency>,
    #[serde(default)]
    dependency_groups: Vec<UvMetadataDependency>,
    sdist: Option<UvMetadataArtifact>,
    #[serde(default)]
    wheels: Vec<UvMetadataArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
struct UvMetadataDependency {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UvMetadataArtifact {
    #[serde(default)]
    hashes: BTreeMap<String, String>,
}

fn workspace_metadata(repo_root: &AbsoluteSystemPath) -> Result<UvWorkspaceMetadata, Error> {
    let output = Command::new("uv")
        .args([
            "workspace",
            "metadata",
            "--frozen",
            "--offline",
            "--preview-features",
            "workspace-metadata",
        ])
        .current_dir(repo_root.as_std_path())
        .output()
        .map_err(|error| Error::MetadataCommand(error.to_string()))?;
    if !output.status.success() {
        return Err(Error::MetadataCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(Error::MetadataParse)
}

pub fn discover_workspace(repo_root: &AbsoluteSystemPath) -> Result<DiscoveredWorkspace, Error> {
    let root_manifest_path = repo_root.join_component(PYPROJECT_TOML);
    let Some(root_manifest) = PyProjectManifest::load(&root_manifest_path)? else {
        return Ok(empty_workspace(None));
    };
    let name = workspace_name(&root_manifest)?;
    if !root_manifest.has_workspace() {
        return Ok(empty_workspace(name));
    }
    let metadata = workspace_metadata(repo_root)?;
    discover_workspace_from_metadata(repo_root, &metadata)
}

fn empty_workspace(name: Option<String>) -> DiscoveredWorkspace {
    DiscoveredWorkspace {
        name,
        packages: Vec::new(),
        root_project_name: None,
        quality_plan: QualityPlan::default(),
        pytest: None,
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .map(canonical_json)
            .collect::<Vec<_>>()
            .join(","),
        serde_json::Value::Object(values) => {
            let mut values = values.iter().collect::<Vec<_>>();
            values.sort_unstable_by_key(|(key, _)| *key);
            values
                .into_iter()
                .map(|(key, value)| format!("{key}={}", canonical_json(value)))
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}

fn metadata_package_identity(node: &UvMetadataNode) -> Option<turborepo_lockfiles::Package> {
    let source = node.source.as_ref()?.as_object()?;
    let name = node.name.clone()?;
    let mut version = node.version.clone().unwrap_or_default();
    for (key, value) in source {
        version.push(' ');
        version.push_str(key);
        version.push('+');
        version.push_str(&canonical_json(value));
    }
    let mut hashes = node
        .sdist
        .iter()
        .chain(&node.wheels)
        .flat_map(|artifact| &artifact.hashes)
        .map(|(algorithm, hash)| format!("{algorithm}:{hash}"))
        .collect::<Vec<_>>();
    hashes.sort_unstable();
    hashes.dedup();
    for hash in hashes {
        version.push(' ');
        version.push_str(&hash);
    }
    Some(turborepo_lockfiles::Package { key: name, version })
}

fn collect_metadata_nodes<'a>(
    metadata: &'a UvWorkspaceMetadata,
    root: &'a str,
    visited: &mut HashSet<&'a str>,
) -> Result<(), Error> {
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let node = metadata
            .resolution
            .get(id)
            .ok_or_else(|| Error::UnknownMetadataNode(id.to_string()))?;
        pending.extend(
            node.dependencies
                .iter()
                .chain(&node.optional_dependencies)
                .chain(&node.dependency_groups)
                .map(|dependency| dependency.id.as_str()),
        );
    }
    Ok(())
}

fn metadata_closure(
    metadata: &UvWorkspaceMetadata,
    root: &str,
) -> Result<HashSet<turborepo_lockfiles::Package>, Error> {
    let member_ids = metadata
        .members
        .iter()
        .map(|member| member.id.as_str())
        .collect::<HashSet<_>>();
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    let mut packages = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let node = metadata
            .resolution
            .get(id)
            .ok_or_else(|| Error::UnknownMetadataNode(id.to_string()))?;
        let is_local = node
            .source
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .is_some_and(|source| {
                source.keys().any(|key| {
                    matches!(key.as_str(), "editable" | "virtual" | "directory" | "path")
                })
            });
        if is_local && node.kind == "package" && !member_ids.contains(id) {
            return Err(Error::UnsupportedLocalMetadataNode(id.to_string()));
        }
        if !is_local && let Some(package) = metadata_package_identity(node) {
            packages.insert(package);
        }
        pending.extend(
            node.dependencies
                .iter()
                .chain(&node.optional_dependencies)
                .chain(&node.dependency_groups)
                .map(|dependency| dependency.id.as_str()),
        );
    }
    Ok(packages)
}

/// Per-package external dependency closures from uv's supported metadata API.
fn external_closures(
    metadata: &UvWorkspaceMetadata,
    members: &[String],
) -> Result<HashMap<String, HashSet<turborepo_lockfiles::Package>>, Error> {
    let member_ids = metadata
        .members
        .iter()
        .map(|member| (normalize_name(&member.name), member.id.as_str()))
        .collect::<HashMap<_, _>>();
    members
        .iter()
        .map(|member| {
            let id = member_ids
                .get(member)
                .ok_or_else(|| Error::UnknownMetadataNode(member.clone()))?;
            Ok((member.clone(), metadata_closure(metadata, id)?))
        })
        .collect()
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
    metadata: UvWorkspaceMetadata,
}

impl UvPruneKnowledge {
    fn discover(
        repo_root: &AbsoluteSystemPath,
        package_directories: HashMap<String, String>,
        root_project_name: Option<String>,
        lockfile: String,
        metadata: UvWorkspaceMetadata,
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
            metadata,
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
        let requested_packages: HashSet<&str> = kept_packages.iter().map(String::as_str).collect();
        let mut roots = kept_packages.to_vec();
        if let Some(root_project) = &self.root_project_name {
            roots.push(root_project.clone());
        }
        let member_ids = self
            .metadata
            .members
            .iter()
            .map(|member| (normalize_name(&member.name), member.id.as_str()))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::new();
        for root in &roots {
            let id = member_ids
                .get(root)
                .ok_or_else(|| failed(Error::UnknownMetadataNode(root.clone())))?;
            collect_metadata_nodes(&self.metadata, id, &mut reachable).map_err(failed)?;
        }
        let kept_packages = reachable
            .iter()
            .filter_map(|id| self.metadata.resolution.get(*id))
            .filter_map(|node| {
                Some(turborepo_lockfiles::UvPackageKey {
                    name: node.name.clone()?,
                    version: node.version.clone(),
                })
            })
            .collect::<HashSet<_>>();
        let members = self
            .metadata
            .members
            .iter()
            .filter(|member| reachable.contains(member.id.as_str()))
            .map(|member| normalize_name(&member.name))
            .collect::<HashSet<_>>();
        let pruned_lock =
            turborepo_lockfiles::uv_prune_lock(&self.lockfile, &kept_packages, &members)
                .map_err(|error| failed(Error::Lockfile(error)))?;

        let mut kept_dirs = Vec::with_capacity(pruned_lock.members.len());
        let mut kept_names = HashSet::with_capacity(pruned_lock.members.len());
        let mut extra_packages = Vec::new();
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

fn uv_change_observation(
    repo_root: &AbsoluteSystemPath,
    package_directories: &[String],
) -> ChangeObservation {
    let mut observation = ChangeObservation::new()
        .with_rediscovery_file_name(PYPROJECT_TOML)
        .with_resolution_path(UV_LOCK)
        .with_resolution_path(".python-version")
        .with_resolution_path("uv.toml")
        .with_ignore_prefix(".venv")
        .with_ignore_prefix("dist");
    for name in ["VIRTUAL_ENV", "UV_PROJECT_ENVIRONMENT"] {
        if let Some(path) = std::env::var_os(name)
            .map(|path| {
                AbsoluteSystemPathBuf::from_unknown(repo_root, path.to_string_lossy().as_ref())
            })
            .and_then(|path| repo_root.anchor(&path).ok())
            .filter(|path| path.components().next().is_some())
        {
            observation = observation.with_ignore_prefix(path.to_unix().to_string());
        }
    }
    for directory in std::iter::once("").chain(package_directories.iter().map(String::as_str)) {
        for cache in [
            ".ruff_cache",
            ".pytest_cache",
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
            // uv owns workspace membership and external resolution. Invoke its
            // metadata command once, then parse manifests only for task details.
            let (metadata, workspace) = turborepo_rayon_compat::block_in_place(|| {
                let metadata = workspace_metadata(&self.repo_root)?;
                let workspace = discover_workspace_from_metadata(&self.repo_root, &metadata)?;
                Ok::<_, Error>((metadata, workspace))
            })
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
            let change_observation = uv_change_observation(&self.repo_root, &workspace_directories);
            let prune_domain = UvPruneKnowledge::discover(
                &self.repo_root,
                package_directories.clone(),
                workspace.root_project_name.clone(),
                lockfile.clone(),
                metadata.clone(),
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
            let mut closures = external_closures(&metadata, &closure_members)
                .map_err(|err| toolchain::Error::Failed(Box::new(err)))?;
            let toolchain_identity =
                turborepo_rayon_compat::block_in_place(|| toolchain_identities(&self.repo_root));
            let toolchain_identified = toolchain_identity.is_some();
            let toolchain_packages = toolchain_identity
                .as_ref()
                .map(|identity| identity.packages.as_slice())
                .unwrap_or_default();

            // The workspace-scoped closure covers every member plus the root
            // project's own dependencies (when the root is a package).
            let workspace_externals: HashSet<turborepo_lockfiles::Package> = closures
                .values()
                .flatten()
                .cloned()
                .chain(toolchain_packages.iter().cloned())
                .collect();

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
                let build_cacheable = toolchain_identity.as_ref().is_some_and(|identity| {
                    package
                        .bundled_uv_build_requirement
                        .as_deref()
                        .is_some_and(|requirement| {
                            bundled_uv_build_matches(requirement, &identity.uv_version)
                        })
                });
                let native_tasks = python_tasks_for_package(
                    kind,
                    &package.name,
                    package_directory,
                    &[],
                    &package.quality_plan,
                    package.pytest.as_ref(),
                    !workspace.quality_plan.format_homogeneous,
                    toolchain_identified,
                    build_cacheable,
                );
                let task_contract = UvTaskContract::new(kind, &package.name);
                let mut external_dependencies = closures.remove(&package.name).unwrap_or_default();
                if package.quality_plan.uses_root_tools() {
                    // Root-owned tools execute against the root environment.
                    external_dependencies.extend(workspace_externals.iter().cloned());
                }
                external_dependencies.extend(toolchain_packages.iter().cloned());
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
            let workspace_native_tasks = python_tasks_for_package(
                UvPackageKind::Workspace,
                &workspace_name,
                ".",
                &workspace_directories,
                &workspace.quality_plan,
                workspace.pytest.as_ref(),
                true,
                toolchain_identified,
                false,
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

    #[test]
    fn test_workspace_metadata_external_closures() {
        let metadata: UvWorkspaceMetadata = serde_json::from_value(serde_json::json!({
            "members": [{ "name": "app", "path": "/workspace/app", "id": "app" }],
            "resolution": {
                "app": {
                    "name": "app",
                    "version": "0.1.0",
                    "source": { "editable": "/workspace/app" },
                    "dependencies": [{ "id": "six" }]
                },
                "six": {
                    "name": "six",
                    "version": "1.17.0",
                    "source": { "registry": { "url": "https://pypi.org/simple" } },
                    "dependencies": [],
                    "sdist": { "hashes": { "sha256": "sdist" } },
                    "wheels": [{ "hashes": { "sha256": "wheel" } }]
                }
            }
        }))
        .unwrap();

        let closure = metadata_closure(&metadata, "app").unwrap();
        assert_eq!(closure.len(), 1);
        let package = closure.iter().next().unwrap();
        assert_eq!(package.key, "six");
        assert_eq!(
            package.version,
            "1.17.0 registry+url=https://pypi.org/simple sha256:sdist sha256:wheel"
        );
    }

    #[test]
    fn test_workspace_metadata_unknown_node_errors() {
        let metadata = UvWorkspaceMetadata {
            members: Vec::new(),
            resolution: HashMap::new(),
        };
        assert!(matches!(
            metadata_closure(&metadata, "missing"),
            Err(Error::UnknownMetadataNode(node)) if node == "missing"
        ));
    }

    #[test]
    fn test_workspace_metadata_rejects_non_workspace_local_dependency() {
        let metadata: UvWorkspaceMetadata = serde_json::from_value(serde_json::json!({
            "members": [{ "name": "app", "path": "/workspace/app", "id": "app" }],
            "resolution": {
                "app": {
                    "name": "app",
                    "kind": "package",
                    "source": { "editable": "/workspace/app" },
                    "dependencies": [{ "id": "shared" }]
                },
                "shared": {
                    "name": "shared",
                    "kind": "package",
                    "source": { "directory": "/shared" },
                    "dependencies": []
                }
            }
        }))
        .unwrap();
        assert!(matches!(
            metadata_closure(&metadata, "app"),
            Err(Error::UnsupportedLocalMetadataNode(node)) if node == "shared"
        ));
    }

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
  "PyTest",
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
                PythonTool::Pytest,
            ]
        );
    }

    #[test]
    fn test_optional_only_tool_declarations_are_excluded() {
        let manifest: PyProjectManifest = toml::from_str(
            r#"
[project.optional-dependencies]
quality = ["ruff", "black", "mypy", "pytest"]
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
dependencies = ["ruff; python_version >= '3.12'", "pytest; python_version >= '3.12'"]

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
        let locked = Command::new("uv")
            .args(["lock", "--offline"])
            .current_dir(root.as_std_path())
            .status()
            .is_ok_and(|status| status.success());
        if !locked {
            return;
        }

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
        assert_eq!(
            py_app.pytest.as_ref().unwrap().owner,
            ExecutionOwner::Member
        );
        assert!(workspace.packages[1].pytest.is_none());
        assert!(workspace.pytest.is_none());
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

        let bundled: PyProjectManifest = toml::from_str(
            "[project]\nname = \"app\"\n[build-system]\nrequires = \
             [\"uv_build>=0.12,<0.13\"]\nbuild-backend = \"uv_build\"\n",
        )
        .unwrap();
        assert_eq!(
            bundled.bundled_uv_build_requirement(),
            Some("uv_build>=0.12,<0.13")
        );
    }

    #[test]
    fn test_bundled_uv_build_version_compatibility() {
        let version = node_semver::Version::parse("0.12.1").unwrap();
        assert!(bundled_uv_build_matches("uv_build>=0.12,<0.13", &version));
        assert!(bundled_uv_build_matches("uv-build==0.12.1", &version));
        assert!(!bundled_uv_build_matches("uv-build==0.12", &version));
        assert!(bundled_uv_build_matches(
            "uv-build==0.12",
            &node_semver::Version::parse("0.12.0").unwrap()
        ));
        assert!(!bundled_uv_build_matches("uv_build>=0.13", &version));
        assert!(!bundled_uv_build_matches("uv_build~=0.12", &version));
        assert!(!bundled_uv_build_matches("hatchling>=1", &version));
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

        let no_sync = toolchain::TaskIOEnvironment::new(HashMap::from([(
            "UV_NO_SYNC".to_string(),
            "true".to_string(),
        )]));
        assert!(has_untracked_uv_configuration(&no_sync));
    }

    #[test]
    fn test_python_identity_omits_installation_paths() {
        let python_path = dunce::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let identity = parse_python_identity(
            &format!(
                r#"[{{"key":"cpython-3.13.10-linux-x86_64-gnu","version":"3.13.10","path":"/other/python","os":"linux","variant":"default","implementation":"cpython","arch":"x86_64","libc":"gnu"}},{{"key":"cpython-3.13.11-linux-x86_64-gnu","version":"3.13.11","path":{},"os":"linux","variant":"default","implementation":"cpython","arch":"x86_64","libc":"gnu"}}]"#,
                serde_json::to_string(&python_path).unwrap()
            ),
            &python_path,
            "binary-hash".to_string(),
            "host-identity".to_string(),
        )
        .unwrap();

        assert!(identity.contains("cpython-3.13.11-linux-x86_64-gnu"));
        assert!(identity.contains("\"libc\":\"gnu\""));
        assert!(identity.contains("\"binary_sha256\":\"binary-hash\""));
        assert!(identity.contains("\"host\":\"host-identity\""));
        assert!(!identity.contains("/home/user"));
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
        let tasks = python_tasks_for_package(
            UvPackageKind::Package,
            "py-app",
            "packages/py-app",
            &[],
            &QualityPlan::effective(&ToolDeclarations::default(), &ToolDeclarations::default()),
            None,
            true,
            true,
            false,
        );
        let display = |name| {
            tasks
                .iter()
                .find(|task| task.name() == name)
                .and_then(|task| task.display())
        };
        assert_eq!(display("build"), Some("uv build --package=py-app"));
        assert_eq!(display("format"), Some("uv format -- packages/py-app"));
        assert_eq!(display("check"), Some("uv check --frozen --package=py-app"));
        let build = tasks.iter().find(|task| task.name() == "build").unwrap();
        assert_eq!(build.contract().defaults().cache, Some(false));
        let format = tasks.iter().find(|task| task.name() == "format").unwrap();
        assert_eq!(format.contract().defaults().cache, Some(false));
        let check = tasks.iter().find(|task| task.name() == "check").unwrap();
        assert_eq!(check.contract().defaults().cache, Some(false));
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
        assert!(!tasks.iter().any(|task| task.name() == "test"));
    }

    #[test]
    fn test_exact_root_and_member_pytest_commands() {
        let root: PyProjectManifest =
            toml::from_str("[project]\ndependencies=['pytest']\n").unwrap();
        let root_execution = root
            .tool_declarations(DeclarationOwner::Root)
            .execution(PythonTool::Pytest)
            .unwrap();
        let root_tasks = python_tasks_for_package(
            UvPackageKind::Workspace,
            "acme",
            ".",
            &["packages/app".to_string()],
            &QualityPlan::default(),
            Some(&root_execution),
            true,
            true,
            false,
        );
        let root_test = root_tasks
            .iter()
            .find(|task| task.name() == "test")
            .unwrap();
        assert_eq!(root_test.display(), Some("uv run --active --frozen pytest"));
        assert_eq!(
            root_test.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::PreferredOnly)
        );

        let member: PyProjectManifest =
            toml::from_str("[dependency-groups]\ntests=['pytest']\n[tool.uv]\ndefault-groups=[]\n")
                .unwrap();
        let member_execution = member
            .tool_declarations(DeclarationOwner::Member)
            .execution(PythonTool::Pytest)
            .unwrap();
        let member_tasks = python_tasks_for_package(
            UvPackageKind::VirtualPackage,
            "app",
            "packages/app",
            &[],
            &QualityPlan::default(),
            Some(&member_execution),
            true,
            true,
            false,
        );
        let member_test = member_tasks
            .iter()
            .find(|task| task.name() == "test")
            .unwrap();
        assert_eq!(
            member_test.display(),
            Some(
                "uv run --active --frozen --package app --no-default-groups --group tests pytest \
                 packages/app"
            )
        );
        assert_eq!(
            member_test.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
        assert!(member_test.command().unwrap().serial_group.is_none());
        assert_eq!(
            resolved_args(member_test, &["-k", "smoke"]),
            [
                "run",
                "--active",
                "--frozen",
                "--package",
                "app",
                "--no-default-groups",
                "--group",
                "tests",
                "pytest",
                "-k",
                "smoke",
                "packages/app",
            ]
            .map(OsString::from)
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
        let tasks = python_tasks_for_package(
            UvPackageKind::VirtualPackage,
            "app",
            "packages/app",
            &[],
            &plan,
            None,
            true,
            true,
            false,
        );
        let task = |name| tasks.iter().find(|task| task.name() == name).unwrap();
        assert_eq!(
            task("lint:ruff").display(),
            Some("uv run --active --frozen ruff check packages/app")
        );
        assert_eq!(
            task("check:pyright").display(),
            Some(
                "uv run --active --frozen --package app --no-default-groups --group types pyright \
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
            [
                "run",
                "--active",
                "--frozen",
                "ruff",
                "check",
                "--fix",
                "packages/app",
            ]
            .map(OsString::from)
        );
        assert_eq!(
            resolved_args(task("check:pyright"), &["--warnings"]),
            [
                "run",
                "--active",
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
        let tasks = python_tasks_for_package(
            UvPackageKind::Workspace,
            "acme",
            ".",
            &["packages/one".to_string(), "packages/two".to_string()],
            &plan,
            None,
            true,
            true,
            false,
        );
        let lint = tasks
            .iter()
            .find(|task| task.name() == "lint:ruff")
            .unwrap();
        assert_eq!(
            lint.display(),
            Some("uv run --active --frozen --all-packages ruff check packages/one packages/two")
        );
        assert_eq!(
            resolved_args(lint, &["--fix", "--unsafe-fixes"]),
            [
                "run",
                "--active",
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
        let tasks = python_tasks_for_package(
            UvPackageKind::VirtualPackage,
            "app",
            "app",
            &[],
            &plan,
            None,
            true,
            true,
            false,
        );
        let task = |name| tasks.iter().find(|task| task.name() == name).unwrap();
        assert_eq!(
            task("format:ruff").display(),
            Some("uv run --active --frozen --package app ruff format app")
        );
        assert_eq!(
            task("format:black").display(),
            Some("uv run --active --frozen --package app black app")
        );
        assert_eq!(
            task("format").display(),
            task("format:ruff").display(),
            "the unqualified formatter must prefer Ruff while retaining Black's qualified task"
        );
        assert_eq!(
            task("check:mypy").display(),
            Some("uv run --active --frozen --package app mypy app")
        );
        assert_eq!(
            task("check:ty").display(),
            Some("uv run --active --frozen --package app ty check app")
        );
        assert_eq!(
            task("check:pyright").display(),
            Some("uv run --active --frozen --package app pyright app")
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
        assert_eq!(mypy.contract().defaults().cache, Some(true));
        assert_eq!(
            mypy.contract().entrypoint(),
            Some(crate::native_tasks::TaskEntrypoint::Candidate)
        );
        assert!(mypy.contract().derives_io());
    }

    #[test]
    fn test_uv_commands_stay_uncached_without_toolchain_identity() {
        let tasks = native_tasks_for_package(
            UvPackageKind::Package,
            "py-app",
            "packages/py-app",
            &[],
            false,
        );

        for task in tasks.iter().filter(|task| task.command().is_some()) {
            assert_eq!(
                task.contract().defaults().cache,
                Some(false),
                "{} must fail closed",
                task.name()
            );
        }
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
                ".pytest.ini",
                ".pytest.toml",
                "pytest.ini",
                "pytest.toml",
                "setup.py",
                "setup.cfg",
                "tox.ini",
                "ty.toml",
                "conftest.py",
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
        assert!(io.input_globs.contains(&"!.pytest_cache/**".to_string()));

        let args = vec!["--fix".to_string()];
        let context = toolchain::TaskIOContext {
            task_args: Some(&args),
            environment: &environment,
        };
        let io = UvTaskContract::new(UvPackageKind::VirtualPackage, "app")
            .derived_task_io(&package, "check", "../..", &[], true, &context)
            .unwrap();
        assert_eq!(io.input_safety, toolchain::DerivedInputSafety::Untracked);
    }

    #[test]
    fn test_pytest_inputs_follow_collection_scope() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let package = PackageTaskContext::new_for_test(
            PackageName::from("app"),
            &root,
            turbopath::AnchoredSystemPath::new("packages/app").unwrap(),
            PackageTaskContextKind::Package,
            None,
        );
        let dependency = PackageTaskContext::new_for_test_with_native_tasks(
            PackageName::from("lib"),
            &root,
            turbopath::AnchoredSystemPath::new("packages/lib").unwrap(),
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
        let member_io = UvTaskContract::new(UvPackageKind::VirtualPackage, "app")
            .derived_task_io(&package, "test", "../..", &[dependency], true, &context)
            .unwrap();
        assert_eq!(member_io.package_default_inputs, Some(true));
        assert!(
            member_io
                .input_globs
                .contains(&"../../packages/lib/**".to_string())
        );

        let workspace_io = UvTaskContract::workspace(
            "acme",
            vec!["packages/app".to_string(), "packages/lib".to_string()],
        )
        .derived_task_io(&package, "test", "", &[], true, &context)
        .unwrap();
        assert_eq!(workspace_io.package_default_inputs, Some(true));
        assert!(
            !workspace_io
                .input_globs
                .contains(&"packages/app/**".to_string())
        );
    }

    #[test]
    fn test_python_watch_ignores_root_and_member_caches() {
        let root = AbsoluteSystemPathBuf::cwd().unwrap();
        let observation = uv_change_observation(&root, &["packages/app".to_string()]);
        let expected = ChangeObservation::new()
            .with_rediscovery_file_name(PYPROJECT_TOML)
            .with_resolution_path(UV_LOCK)
            .with_resolution_path(".python-version")
            .with_resolution_path("uv.toml")
            .with_ignore_prefix(".venv")
            .with_ignore_prefix("dist");
        let expected = ["", "packages/app"]
            .into_iter()
            .fold(expected, |observation, dir| {
                [
                    ".ruff_cache",
                    ".pytest_cache",
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
