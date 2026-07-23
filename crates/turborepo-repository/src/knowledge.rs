//! Immutable, parser-neutral facts observed about a repository.
//!
//! This module deliberately contains no package manifests or native metadata.
//! Those remain compatibility inputs for relationship and task construction;
//! package identity, ownership boundaries, and definition sources live here.

use std::collections::HashMap;

use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::toolchain::{ToolchainId, WorkspaceRoot};

/// A workspace root paired by core with the registry entry that produced its
/// discovery envelope. The public adapter output cannot supply provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceRootObservation {
    root: WorkspaceRoot,
    producer: ToolchainId,
}

impl WorkspaceRootObservation {
    pub(crate) fn new(root: WorkspaceRoot, producer: ToolchainId) -> Self {
        Self { root, producer }
    }

    fn kind(&self) -> &str {
        self.root.kind()
    }

    fn path(&self) -> &AbsoluteSystemPath {
        self.root.path()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Package,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopeKnowledge {
    identity: String,
    directory: AnchoredSystemPathBuf,
    definition_path: AnchoredSystemPathBuf,
    toolchain: ToolchainId,
    kind: ScopeKind,
}

impl ScopeKnowledge {
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn user_facing_name(&self) -> &str {
        &self.identity
    }

    pub(crate) fn directory(&self) -> &AnchoredSystemPath {
        &self.directory
    }

    pub(crate) fn definition_path(&self) -> &AnchoredSystemPath {
        &self.definition_path
    }

    pub(crate) fn toolchain(&self) -> &ToolchainId {
        &self.toolchain
    }

    pub(crate) fn kind(&self) -> ScopeKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootJavaScriptScope {
    user_facing_name: Option<String>,
    definition_path: AnchoredSystemPathBuf,
    toolchain: ToolchainId,
}

impl RootJavaScriptScope {
    pub(crate) fn user_facing_name(&self) -> Option<&str> {
        self.user_facing_name.as_deref()
    }

    pub(crate) fn definition_path(&self) -> &AnchoredSystemPath {
        &self.definition_path
    }

    pub(crate) fn toolchain(&self) -> &ToolchainId {
        &self.toolchain
    }
}

/// One immutable generation of package and execution-scope facts.
#[derive(Debug)]
pub(crate) struct RepositoryKnowledge {
    repository_root: AbsoluteSystemPathBuf,
    repository_directory: AnchoredSystemPathBuf,
    root_javascript_scope: Option<RootJavaScriptScope>,
    workspace_roots: Vec<WorkspaceRootKnowledge>,
    scopes: Vec<ScopeKnowledge>,
    scope_lookup: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceRootKnowledge {
    kind: String,
    path: AnchoredSystemPathBuf,
    toolchain: ToolchainId,
}

impl WorkspaceRootKnowledge {
    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }

    pub(crate) fn path(&self) -> &AnchoredSystemPath {
        &self.path
    }

    pub(crate) fn toolchain(&self) -> &ToolchainId {
        &self.toolchain
    }
}

impl RepositoryKnowledge {
    pub(crate) fn repository_root(&self) -> &AbsoluteSystemPath {
        &self.repository_root
    }

    pub(crate) fn repository_directory(&self) -> &AnchoredSystemPath {
        &self.repository_directory
    }

    pub(crate) fn root_javascript_scope(&self) -> Option<&RootJavaScriptScope> {
        self.root_javascript_scope.as_ref()
    }

    pub(crate) fn workspace_roots(&self) -> impl Iterator<Item = &WorkspaceRootKnowledge> {
        self.workspace_roots.iter()
    }

    pub(crate) fn packages(&self) -> impl Iterator<Item = &ScopeKnowledge> {
        self.scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Package)
    }

    pub(crate) fn aggregate_scopes(&self) -> impl Iterator<Item = &ScopeKnowledge> {
        self.scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Aggregate)
    }

    pub(crate) fn scope(&self, identity: &str) -> Option<&ScopeKnowledge> {
        self.scope_lookup
            .get(identity)
            .map(|index| &self.scopes[*index])
    }

    pub(crate) fn build(
        repository_root: &AbsoluteSystemPath,
        root_javascript_name: Option<Option<String>>,
        observations: &[PackageScopeObservation],
        workspace_root_observations: &[WorkspaceRootObservation],
    ) -> Result<Self, Error> {
        let root_definition_path = AnchoredSystemPathBuf::from_raw("package.json")?;
        let root_javascript_scope =
            root_javascript_name.map(|user_facing_name| RootJavaScriptScope {
                user_facing_name,
                definition_path: root_definition_path,
                toolchain: ToolchainId::JAVASCRIPT,
            });

        let mut scopes = Vec::with_capacity(observations.len());
        let mut scope_lookup = HashMap::with_capacity(observations.len());
        let mut definitions = HashMap::<String, AnchoredSystemPathBuf>::new();
        let workspace_roots =
            validate_workspace_roots(repository_root, workspace_root_observations)?;

        for observation in observations {
            if !workspace_roots
                .iter()
                .any(|root| root.toolchain() == &observation.toolchain)
            {
                return Err(Error::MissingWorkspaceRoot {
                    toolchain: observation.toolchain.clone(),
                });
            }
        }

        for observation in observations {
            if !path_is_contained(repository_root, &observation.definition_path) {
                return Err(Error::DefinitionOutsideRepository {
                    path: observation.definition_path.clone(),
                    repository_root: repository_root.to_owned(),
                });
            }
            let Some(identity) = observation.identity.as_ref() else {
                continue;
            };
            let definition_path = AnchoredSystemPathBuf::relative_path_between(
                repository_root,
                &observation.definition_path,
            );
            if let Some(existing_path) =
                definitions.insert(identity.clone(), definition_path.clone())
            {
                return Err(Error::DuplicateScope {
                    name: identity.clone(),
                    path: definition_path,
                    existing_path,
                });
            }
            let directory = definition_path
                .parent()
                .map(AnchoredSystemPath::to_owned)
                .unwrap_or_default();
            scope_lookup.insert(identity.clone(), scopes.len());
            scopes.push(ScopeKnowledge {
                identity: identity.clone(),
                directory,
                definition_path,
                toolchain: observation.toolchain.clone(),
                kind: observation.scope_kind,
            });
        }

        Ok(Self {
            repository_root: repository_root.to_owned(),
            repository_directory: AnchoredSystemPathBuf::default(),
            root_javascript_scope,
            workspace_roots,
            scopes,
            scope_lookup,
        })
    }
}

fn validate_workspace_roots(
    repository_root: &AbsoluteSystemPath,
    observations: &[WorkspaceRootObservation],
) -> Result<Vec<WorkspaceRootKnowledge>, Error> {
    let mut accepted = HashMap::<String, (std::path::PathBuf, AnchoredSystemPathBuf)>::new();
    let mut roots = Vec::with_capacity(observations.len());

    for observation in observations {
        if !path_is_contained(repository_root, observation.path()) {
            return Err(Error::WorkspaceRootOutsideRepository {
                kind: observation.kind().to_string(),
                path: observation.path().to_owned(),
                repository_root: repository_root.to_owned(),
            });
        }
        let mut anchored_path =
            AnchoredSystemPathBuf::relative_path_between(repository_root, observation.path());
        if anchored_path.as_str() == "." {
            anchored_path = AnchoredSystemPathBuf::default();
        }
        let physical_path = canonical_physical_path(observation.path().as_std_path())
            .unwrap_or_else(|| observation.path().as_std_path().to_owned());
        if let Some((accepted_physical_path, accepted_path)) = accepted.get(observation.kind()) {
            if accepted_physical_path == &physical_path {
                if roots.iter().any(|root: &WorkspaceRootKnowledge| {
                    root.kind == observation.kind() && root.toolchain == observation.producer
                }) {
                    continue;
                }
                roots.push(WorkspaceRootKnowledge {
                    kind: observation.kind().to_string(),
                    path: accepted_path.clone(),
                    toolchain: observation.producer.clone(),
                });
                continue;
            }
            return Err(Error::DuplicateWorkspaceRoot {
                kind: observation.kind().to_string(),
                accepted_root: accepted_path.clone(),
                conflicting_root: anchored_path,
            });
        }
        accepted.insert(
            observation.kind().to_string(),
            (physical_path, anchored_path.clone()),
        );
        roots.push(WorkspaceRootKnowledge {
            kind: observation.kind().to_string(),
            path: anchored_path,
            toolchain: observation.producer.clone(),
        });
    }

    Ok(roots)
}

fn path_is_contained(
    repository_root: &AbsoluteSystemPath,
    definition_path: &AbsoluteSystemPath,
) -> bool {
    if !repository_root.contains(definition_path) {
        return false;
    }

    match (
        canonical_physical_path(repository_root.as_std_path()),
        canonical_physical_path(definition_path.as_std_path()),
    ) {
        (Some(repository_root), Some(definition_path)) => {
            definition_path.starts_with(repository_root)
        }
        _ => true,
    }
}

fn canonical_physical_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut existing = path.to_owned();
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(existing.file_name()?.to_owned());
        if !existing.pop() {
            return None;
        }
    }
    let mut canonical = dunce::canonicalize(existing).ok()?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Some(canonical)
}

pub(crate) struct PackageScopeObservation {
    pub identity: Option<String>,
    pub definition_path: AbsoluteSystemPathBuf,
    pub toolchain: ToolchainId,
    pub scope_kind: ScopeKind,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("duplicate package or aggregate scope {name}")]
    DuplicateScope {
        name: String,
        path: AnchoredSystemPathBuf,
        existing_path: AnchoredSystemPathBuf,
    },
    #[error("package definition {path} is outside repository root {repository_root}")]
    DefinitionOutsideRepository {
        path: AbsoluteSystemPathBuf,
        repository_root: AbsoluteSystemPathBuf,
    },
    #[error(
        "multiple independent {kind} workspace roots are unsupported: accepted {accepted_root}, \
         conflicting {conflicting_root}"
    )]
    DuplicateWorkspaceRoot {
        kind: String,
        accepted_root: AnchoredSystemPathBuf,
        conflicting_root: AnchoredSystemPathBuf,
    },
    #[error("{kind} workspace root {path} is outside repository root {repository_root}")]
    WorkspaceRootOutsideRepository {
        kind: String,
        path: AbsoluteSystemPathBuf,
        repository_root: AbsoluteSystemPathBuf,
    },
    #[error("toolchain {toolchain} contributed packages without a workspace root")]
    MissingWorkspaceRoot { toolchain: ToolchainId },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}
