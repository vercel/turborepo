//! Immutable, parser-neutral facts observed about a repository.
//!
//! This module deliberately contains no package manifests or native metadata.
//! Parsers contribute normalized facts here; descriptors remain transient
//! inputs to repository construction.

use std::collections::HashMap;

use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_errors::Spanned;

use crate::{
    relationships::{Relationship, RelationshipTarget},
    toolchain::{ToolchainId, WorkspaceRoot},
};

/// All normalized relationships observed for one authoritative source.
#[derive(Debug)]
pub(crate) struct RelationshipGroup {
    source: String,
    relationships: Box<[Relationship]>,
}

impl RelationshipGroup {
    pub(crate) fn new(source: impl Into<String>, relationships: Vec<Relationship>) -> Self {
        Self {
            source: source.into(),
            relationships: relationships.into_boxed_slice(),
        }
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }
}

/// One validated, immutable generation of normalized package relationships.
#[derive(Debug)]
pub(crate) struct RelationshipKnowledge {
    groups: Box<[RelationshipGroup]>,
}

impl RelationshipKnowledge {
    pub(crate) fn build(
        repository: &RepositoryKnowledge,
        mut groups: Vec<RelationshipGroup>,
    ) -> Result<Self, Error> {
        for group in &groups {
            if !repository.has_identity(group.source()) {
                return Err(Error::UnknownRelationshipSource {
                    identity: group.source().to_string(),
                });
            }
            for relationship in group.relationships() {
                if let RelationshipTarget::Internal(target) = relationship.target()
                    && !repository.has_identity(target)
                {
                    return Err(Error::UnknownRelationshipTarget {
                        identity: target.clone(),
                    });
                }
            }
        }

        // Stable sorting makes source grouping deterministic while preserving
        // declaration order within each source, where first occurrence carries
        // compatibility precedence.
        groups.sort_by(|left, right| left.source.cmp(&right.source));

        Ok(Self {
            groups: groups.into_boxed_slice(),
        })
    }

    pub(crate) fn groups(&self) -> &[RelationshipGroup] {
        &self.groups
    }

    pub(crate) fn relationships_for_source(&self, source: &str) -> &[Relationship] {
        self.groups
            .binary_search_by(|group| group.source().cmp(source))
            .ok()
            .map(|index| self.groups[index].relationships())
            .unwrap_or_default()
    }
}

/// A workspace root paired by core with the contributor that produced its
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
    name_source: Option<Spanned<()>>,
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

    pub(crate) fn name_source(&self) -> Option<&Spanned<()>> {
        self.name_source.as_ref()
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

    fn is_package_json_package(&self) -> bool {
        self.kind == ScopeKind::Package
            && self.definition_path.as_path().file_name() == Some("package.json".as_ref())
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
    fn has_identity(&self, identity: &str) -> bool {
        (identity == "//" && self.root_javascript_scope.is_some())
            || self.scope_lookup.contains_key(identity)
    }

    /// Real package scopes whose authoritative definition is package.json.
    /// Contributor identity remains provenance and does not select membership.
    pub(crate) fn package_json_packages(
        &self,
    ) -> impl Iterator<Item = (&str, &AnchoredSystemPath)> {
        let root = self.root_javascript_scope.as_ref().and_then(|scope| {
            (scope.definition_path.as_path().file_name() == Some("package.json".as_ref()))
                .then_some(("//", self.repository_directory.as_ref()))
        });
        root.into_iter().chain(
            self.scopes
                .iter()
                .filter(|scope| {
                    scope.is_package_json_package()
                        && !(self.root_javascript_scope.is_some()
                            && scope.directory().as_str().is_empty())
                })
                .map(|scope| (scope.identity(), scope.directory())),
        )
    }

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

    pub(crate) fn scopes(&self) -> impl Iterator<Item = &ScopeKnowledge> {
        self.scopes.iter()
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
        let root_physical_definition = root_javascript_scope.as_ref().and_then(|_| {
            canonical_physical_path(repository_root.join_component("package.json").as_std_path())
        });

        let mut scopes = Vec::with_capacity(observations.len());
        let mut scope_lookup = HashMap::with_capacity(observations.len());
        let mut definitions = HashMap::<String, AnchoredSystemPathBuf>::new();
        let mut definition_owners =
            HashMap::<std::path::PathBuf, (String, ToolchainId, ScopeKind)>::new();
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
            let physical_definition_path =
                canonical_physical_path(observation.definition_path.as_std_path())
                    .unwrap_or_else(|| observation.definition_path.as_std_path().to_owned());
            if identity == "//" {
                return Err(Error::ReservedRootIdentity {
                    path: definition_path,
                });
            }
            if let Some(existing_path) =
                definitions.insert(identity.clone(), definition_path.clone())
            {
                return Err(Error::DuplicateScope {
                    name: identity.clone(),
                    path: definition_path,
                    existing_path,
                });
            }
            if root_physical_definition.as_ref() == Some(&physical_definition_path)
                && definition_path.as_str() != "package.json"
            {
                return Err(Error::DuplicateDefinitionPath {
                    path: definition_path,
                    identity: identity.clone(),
                    existing_identity: "//".to_string(),
                });
            }
            if let Some((existing_identity, existing_toolchain, existing_kind)) = definition_owners
                .insert(
                    physical_definition_path,
                    (
                        identity.clone(),
                        observation.toolchain.clone(),
                        observation.scope_kind,
                    ),
                )
                && (existing_toolchain != observation.toolchain
                    || existing_kind == observation.scope_kind)
            {
                return Err(Error::DuplicateDefinitionPath {
                    path: definition_path,
                    identity: identity.clone(),
                    existing_identity,
                });
            }
            let directory = definition_path
                .parent()
                .map(AnchoredSystemPath::to_owned)
                .unwrap_or_default();
            scope_lookup.insert(identity.clone(), scopes.len());
            scopes.push(ScopeKnowledge {
                identity: identity.clone(),
                name_source: observation.name_source.clone(),
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
    let mut accepted =
        HashMap::<ToolchainId, (String, std::path::PathBuf, AnchoredSystemPathBuf)>::new();
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
        if let Some((accepted_kind, accepted_physical_path, accepted_path)) =
            accepted.get(&observation.producer)
        {
            if accepted_kind == observation.kind() && accepted_physical_path == &physical_path {
                continue;
            }
            return Err(Error::MultipleWorkspaceRoots {
                toolchain: observation.producer.clone(),
                accepted_kind: accepted_kind.clone(),
                accepted_root: accepted_path.clone(),
                conflicting_kind: observation.kind().to_string(),
                conflicting_root: anchored_path,
            });
        }
        accepted.insert(
            observation.producer.clone(),
            (
                observation.kind().to_string(),
                physical_path,
                anchored_path.clone(),
            ),
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
    pub name_source: Option<Spanned<()>>,
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
    #[error("package definition {path} is claimed by both {existing_identity} and {identity}")]
    DuplicateDefinitionPath {
        path: AnchoredSystemPathBuf,
        identity: String,
        existing_identity: String,
    },
    #[error("package definition {path} is outside repository root {repository_root}")]
    DefinitionOutsideRepository {
        path: AbsoluteSystemPathBuf,
        repository_root: AbsoluteSystemPathBuf,
    },
    #[error(
        "toolchain {toolchain} contributed multiple workspace roots: accepted {accepted_kind} \
         root {accepted_root}, conflicting {conflicting_kind} root {conflicting_root}"
    )]
    MultipleWorkspaceRoots {
        toolchain: ToolchainId,
        accepted_kind: String,
        accepted_root: AnchoredSystemPathBuf,
        conflicting_kind: String,
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
    #[error("package or aggregate scope at {path} uses reserved root identity //")]
    ReservedRootIdentity { path: AnchoredSystemPathBuf },
    #[error("relationship source {identity} has no authoritative repository scope")]
    UnknownRelationshipSource { identity: String },
    #[error("internal relationship target {identity} has no authoritative repository scope")]
    UnknownRelationshipTarget { identity: String },
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relationships::{DependencyKind, Relationship};

    fn repository(with_root_javascript: bool) -> RepositoryKnowledge {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        RepositoryKnowledge::build(
            &root,
            with_root_javascript.then_some(Some("root".to_string())),
            &[
                PackageScopeObservation {
                    identity: Some("app".to_string()),
                    name_source: None,
                    definition_path: root.join_components(&["apps", "app", "package.json"]),
                    toolchain: ToolchainId::JAVASCRIPT,
                    scope_kind: ScopeKind::Package,
                },
                PackageScopeObservation {
                    identity: Some("lib".to_string()),
                    name_source: None,
                    definition_path: root.join_components(&["packages", "lib", "package.json"]),
                    toolchain: ToolchainId::JAVASCRIPT,
                    scope_kind: ScopeKind::Package,
                },
            ],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("npm", root.clone()),
                ToolchainId::JAVASCRIPT,
            )],
        )
        .unwrap()
    }

    #[test]
    fn relationship_knowledge_rejects_unknown_source() {
        let repository = repository(true);
        let error = RelationshipKnowledge::build(
            &repository,
            vec![RelationshipGroup::new(
                "missing",
                vec![Relationship::new(
                    "missing",
                    DependencyKind::Production,
                    RelationshipTarget::Internal("lib".to_string()),
                )],
            )],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::UnknownRelationshipSource { identity } if identity == "missing"
        ));
    }

    #[test]
    fn repository_knowledge_rejects_reserved_root_package_identity() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let result = RepositoryKnowledge::build(
            &root,
            None,
            &[PackageScopeObservation {
                identity: Some("//".to_string()),
                name_source: None,
                definition_path: root.join_components(&["native", "manifest"]),
                toolchain: ToolchainId::new("native"),
                scope_kind: ScopeKind::Aggregate,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("native", root.clone()),
                ToolchainId::new("native"),
            )],
        );

        assert!(matches!(result, Err(Error::ReservedRootIdentity { .. })));
    }

    #[test]
    fn repository_knowledge_preserves_package_name_provenance() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let source = Spanned::new(())
            .with_range(9..14)
            .with_text(r#"{"name": "app"}"#)
            .with_path("apps/app/package.json".into());
        let repository = RepositoryKnowledge::build(
            &root,
            None,
            &[PackageScopeObservation {
                identity: Some("app".to_string()),
                name_source: Some(source.clone()),
                definition_path: root.join_components(&["apps", "app", "package.json"]),
                toolchain: ToolchainId::JAVASCRIPT,
                scope_kind: ScopeKind::Package,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("npm", root.clone()),
                ToolchainId::JAVASCRIPT,
            )],
        )
        .unwrap();

        assert_eq!(
            repository.scope("app").unwrap().name_source(),
            Some(&source)
        );
    }

    #[test]
    fn package_json_projection_uses_definition_and_scope_kind_not_provenance() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let custom = ToolchainId::new("custom");
        let repository = RepositoryKnowledge::build(
            &root,
            None,
            &[
                PackageScopeObservation {
                    identity: Some("custom-web".to_string()),
                    name_source: None,
                    definition_path: root.join_components(&["apps", "custom-web", "package.json"]),
                    toolchain: custom.clone(),
                    scope_kind: ScopeKind::Package,
                },
                PackageScopeObservation {
                    identity: Some("spoofed-js".to_string()),
                    name_source: None,
                    definition_path: root.join_components(&["crates", "spoofed-js", "Cargo.toml"]),
                    toolchain: ToolchainId::JAVASCRIPT,
                    scope_kind: ScopeKind::Package,
                },
                PackageScopeObservation {
                    identity: Some("aggregate".to_string()),
                    name_source: None,
                    definition_path: root.join_components(&["aggregate", "package.json"]),
                    toolchain: custom.clone(),
                    scope_kind: ScopeKind::Aggregate,
                },
            ],
            &[
                WorkspaceRootObservation::new(WorkspaceRoot::new("custom", root.clone()), custom),
                WorkspaceRootObservation::new(
                    WorkspaceRoot::new("javascript", root.clone()),
                    ToolchainId::JAVASCRIPT,
                ),
            ],
        )
        .unwrap();

        let packages = repository
            .package_json_packages()
            .map(|(identity, _)| identity)
            .collect::<Vec<_>>();
        assert_eq!(packages, ["custom-web"]);
    }

    #[test]
    fn repository_knowledge_rejects_duplicate_definition_owners() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let definition_path = root.join_components(&["apps", "shared", "package.json"]);
        let custom = ToolchainId::new("custom");
        let result = RepositoryKnowledge::build(
            &root,
            None,
            &[
                PackageScopeObservation {
                    identity: Some("first".to_string()),
                    name_source: None,
                    definition_path: definition_path.clone(),
                    toolchain: custom.clone(),
                    scope_kind: ScopeKind::Package,
                },
                PackageScopeObservation {
                    identity: Some("second".to_string()),
                    name_source: None,
                    definition_path,
                    toolchain: custom.clone(),
                    scope_kind: ScopeKind::Package,
                },
            ],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("custom", root.clone()),
                custom,
            )],
        );

        assert!(matches!(result, Err(Error::DuplicateDefinitionPath { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn repository_knowledge_rejects_root_definition_symlink_alias() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::new(temp.path().to_string_lossy().to_string()).unwrap();
        root.join_component("package.json")
            .create_with_contents("{}")
            .unwrap();
        let alias = root.join_component("alias");
        std::os::unix::fs::symlink(root.as_std_path(), alias.as_std_path()).unwrap();

        let result = RepositoryKnowledge::build(
            &root,
            Some(Some("root".to_string())),
            &[PackageScopeObservation {
                identity: Some("alias".to_string()),
                name_source: None,
                definition_path: alias.join_component("package.json"),
                toolchain: ToolchainId::JAVASCRIPT,
                scope_kind: ScopeKind::Package,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("npm", root.clone()),
                ToolchainId::JAVASCRIPT,
            )],
        );

        assert!(matches!(result, Err(Error::DuplicateDefinitionPath { .. })));
    }

    #[test]
    fn relationship_knowledge_rejects_unknown_internal_target() {
        let repository = repository(true);
        let error = RelationshipKnowledge::build(
            &repository,
            vec![RelationshipGroup::new(
                "app",
                vec![Relationship::new(
                    "missing",
                    DependencyKind::Development,
                    RelationshipTarget::Internal("missing".to_string()),
                )],
            )],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::UnknownRelationshipTarget { identity } if identity == "missing"
        ));
    }

    #[test]
    fn root_identity_requires_and_accepts_root_javascript_scope() {
        let observation = || {
            RelationshipGroup::new(
                "//",
                vec![Relationship::new(
                    "//",
                    DependencyKind::Production,
                    RelationshipTarget::Internal("app".to_string()),
                )],
            )
        };

        assert!(matches!(
            RelationshipKnowledge::build(&repository(false), vec![observation()]),
            Err(Error::UnknownRelationshipSource { identity }) if identity == "//"
        ));
        assert!(matches!(
            RelationshipKnowledge::build(
                &repository(false),
                vec![RelationshipGroup::new("app", vec![Relationship::new(
                    "//",
                    DependencyKind::Production,
                    RelationshipTarget::Internal("//".to_string()),
                )])]
            ),
            Err(Error::UnknownRelationshipTarget { identity }) if identity == "//"
        ));

        let knowledge = RelationshipKnowledge::build(&repository(true), vec![observation()])
            .expect("root JavaScript scope makes // authoritative");
        assert_eq!(knowledge.groups()[0].source(), "//");

        let root_target = RelationshipKnowledge::build(
            &repository(true),
            vec![RelationshipGroup::new(
                "app",
                vec![Relationship::new(
                    "//",
                    DependencyKind::Production,
                    RelationshipTarget::Internal("//".to_string()),
                )],
            )],
        );
        assert!(root_target.is_ok());
    }
}
