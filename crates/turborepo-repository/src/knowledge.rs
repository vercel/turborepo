//! Immutable, parser-neutral facts observed about a repository.
//!
//! This module deliberately contains no package manifests or native metadata.
//! Those remain compatibility inputs for relationship and task construction;
//! package identity, ownership boundaries, and definition sources live here.

use std::collections::HashMap;

use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::toolchain::ToolchainId;

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
    scopes: Vec<ScopeKnowledge>,
    scope_lookup: HashMap<String, usize>,
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

        for observation in observations {
            if !definition_is_contained(repository_root, &observation.definition_path) {
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
            scopes,
            scope_lookup,
        })
    }
}

fn definition_is_contained(
    repository_root: &AbsoluteSystemPath,
    definition_path: &AbsoluteSystemPath,
) -> bool {
    if !repository_root.contains(definition_path) {
        return false;
    }

    match (
        dunce::canonicalize(repository_root.as_std_path()),
        dunce::canonicalize(definition_path.as_std_path()),
    ) {
        (Ok(repository_root), Ok(definition_path)) => definition_path.starts_with(repository_root),
        // Discovery and watcher tests may supply definitions that do not exist
        // yet. Preserve lexical containment when either side cannot be observed.
        _ => true,
    }
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
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}
