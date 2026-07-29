//! Parser-neutral external dependency declarations and exact resolutions.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ops::Range,
};

use sha2::{Digest, Sha256};
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};

use crate::{
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    relationships::{DependencyKind, RelationshipTarget},
    toolchain::ToolchainId,
};

/// One effective unresolved external declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDeclaration {
    source: String,
    declaration_name: String,
    package_name: String,
    specifier: String,
    kind: DependencyKind,
}

impl ExternalDeclaration {
    pub fn new(
        source: impl Into<String>,
        declaration_name: impl Into<String>,
        package_name: impl Into<String>,
        specifier: impl Into<String>,
        kind: DependencyKind,
    ) -> Self {
        Self {
            source: source.into(),
            declaration_name: declaration_name.into(),
            package_name: package_name.into(),
            specifier: specifier.into(),
            kind,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn declaration_name(&self) -> &str {
        &self.declaration_name
    }

    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    pub fn specifier(&self) -> &str {
        &self.specifier
    }

    pub fn kind(&self) -> DependencyKind {
        self.kind
    }
}

/// Immutable declaration-side input to external resolution producers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalDeclarations {
    declarations: Box<[ExternalDeclaration]>,
    ranges: HashMap<String, Range<usize>>,
}

impl ExternalDeclarations {
    pub(crate) fn build(relationships: &RelationshipKnowledge) -> Self {
        let mut declarations = Vec::new();
        let mut ranges = HashMap::new();
        for group in relationships.groups() {
            let start = declarations.len();
            let mut seen = HashSet::new();
            for relationship in group.relationships() {
                if !seen.insert(relationship.declaration_name()) {
                    continue;
                }
                let RelationshipTarget::UnresolvedExternal { name, specifier } =
                    relationship.target()
                else {
                    continue;
                };
                declarations.push(ExternalDeclaration::new(
                    group.source(),
                    relationship.declaration_name(),
                    name,
                    specifier,
                    relationship.kind(),
                ));
            }
            ranges.insert(group.source().to_string(), start..declarations.len());
        }
        Self {
            declarations: declarations.into_boxed_slice(),
            ranges,
        }
    }

    pub fn declarations(&self) -> &[ExternalDeclaration] {
        &self.declarations
    }

    pub fn for_package<'a>(&'a self, source: &'a str) -> PackageExternalDeclarations<'a> {
        let declarations = self
            .ranges
            .get(source)
            .map(|range| &self.declarations[range.clone()])
            .unwrap_or_default();
        PackageExternalDeclarations::new(declarations, source)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PackageExternalDeclarations<'a> {
    declarations: &'a [ExternalDeclaration],
    source: &'a str,
}

impl<'a> PackageExternalDeclarations<'a> {
    pub fn new(declarations: &'a [ExternalDeclaration], source: &'a str) -> Self {
        Self {
            declarations,
            source,
        }
    }

    pub fn iter(self) -> impl DoubleEndedIterator<Item = &'a ExternalDeclaration> + Clone + 'a {
        self.declarations
            .iter()
            .filter(move |declaration| declaration.source() == self.source)
    }
}

/// An exact, opaque external package identity.
///
/// Equality, ordering, and hashing are defined by `(key, version)` only.
/// `human_name` is display metadata captured at observation time so query
/// consumers do not re-read native lockfiles.
#[derive(Debug, Clone)]
pub struct ExternalPackageIdentity {
    key: String,
    version: String,
    human_name: Option<String>,
}

impl PartialEq for ExternalPackageIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.version == other.version
    }
}

impl Eq for ExternalPackageIdentity {}

impl PartialOrd for ExternalPackageIdentity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExternalPackageIdentity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.key, &self.version).cmp(&(&other.key, &other.version))
    }
}

impl std::hash::Hash for ExternalPackageIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.version.hash(state);
    }
}

impl ExternalPackageIdentity {
    pub fn new(key: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            version: version.into(),
            human_name: None,
        }
    }

    pub fn with_human_name(mut self, human_name: impl Into<String>) -> Self {
        self.human_name = Some(human_name.into());
        self
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Prefer the producer-supplied display name; otherwise the opaque key.
    pub fn display_name(&self) -> &str {
        self.human_name.as_deref().unwrap_or(&self.key)
    }

    pub fn human_name(&self) -> Option<&str> {
        self.human_name.as_deref()
    }
}

/// Why a resolved generation is incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionIncompleteReason {
    code: String,
    message: String,
}

impl ResolutionIncompleteReason {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Whether all declarations in a resolution domain were resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionCompleteness {
    Complete,
    Partial(ResolutionIncompleteReason),
}

/// Why no terminal resolution data is available for a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionUnavailableReason {
    code: String,
    message: String,
}

impl ResolutionUnavailableReason {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Stable producer-supplied identity for one resolved domain generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionFingerprint(String);

impl ResolutionFingerprint {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_packages(packages: &[PackageResolution]) -> Self {
        fn update(hasher: &mut Sha256, value: &str) {
            hasher.update((value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }

        let mut packages: Vec<_> = packages.iter().collect();
        packages.sort_unstable_by_key(|package| package.package());
        let mut hasher = Sha256::new();
        for package in packages {
            update(&mut hasher, package.package());
            let mut identities: Vec<_> = package.identities().iter().collect();
            identities.sort_unstable();
            for identity in identities {
                update(&mut hasher, identity.key());
                update(&mut hasher, identity.version());
            }
        }
        Self::new(format!("{:x}", hasher.finalize()))
    }
}

/// Exact external identities attributed to one authoritative package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageResolution {
    package: String,
    identities: Vec<ExternalPackageIdentity>,
    fingerprint: Option<ResolutionFingerprint>,
}

impl PackageResolution {
    pub fn new(
        package: impl Into<String>,
        identities: impl IntoIterator<Item = ExternalPackageIdentity>,
    ) -> Self {
        let mut identities: Vec<_> = identities.into_iter().collect();
        identities.sort_unstable();
        identities.dedup();
        Self {
            package: package.into(),
            identities,
            fingerprint: None,
        }
    }

    pub fn package(&self) -> &str {
        &self.package
    }

    pub fn identities(&self) -> &[ExternalPackageIdentity] {
        &self.identities
    }

    pub fn fingerprint(&self) -> Option<&ResolutionFingerprint> {
        self.fingerprint.as_ref()
    }

    pub(crate) fn set_fingerprint(&mut self, fingerprint: ResolutionFingerprint) {
        self.fingerprint = Some(fingerprint);
    }
}

/// Terminal resolution data for one domain.
///
/// `Resolved` with an empty package set is intentionally distinct from
/// `Unavailable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalResolutionData {
    Resolved {
        completeness: ResolutionCompleteness,
        fingerprint: ResolutionFingerprint,
        packages: Vec<PackageResolution>,
    },
    Unavailable(ResolutionUnavailableReason),
}

/// Resolution knowledge available to package-scoped consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageResolutionState {
    Resolved {
        completeness: ResolutionCompleteness,
        fingerprint: ResolutionFingerprint,
    },
    Unavailable(ResolutionUnavailableReason),
    Missing,
    NotApplicable,
}

impl PackageResolutionState {
    /// Existing unavailable and non-applicable states remain cache-eligible.
    /// Partial resolution is explicit and cannot safely participate in caching.
    pub fn cache_eligible(&self) -> bool {
        !matches!(
            self,
            Self::Resolved {
                completeness: ResolutionCompleteness::Partial(_),
                ..
            }
        )
    }

    pub fn task_hash(&self) -> Option<&str> {
        match self {
            Self::Resolved { fingerprint, .. } => Some(fingerprint.as_str()),
            Self::Unavailable(_) | Self::NotApplicable => Some(""),
            Self::Missing => None,
        }
    }
}

/// One parser-neutral external resolution domain contributed by a toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalResolutionDomain {
    toolchain: ToolchainId,
    root: AnchoredSystemPathBuf,
    definition_sources: Vec<AnchoredSystemPathBuf>,
    data: ExternalResolutionData,
}

impl ExternalResolutionDomain {
    pub fn new(
        toolchain: ToolchainId,
        root: AnchoredSystemPathBuf,
        definition_sources: impl IntoIterator<Item = AnchoredSystemPathBuf>,
        data: ExternalResolutionData,
    ) -> Self {
        Self {
            toolchain,
            root,
            definition_sources: definition_sources.into_iter().collect(),
            data,
        }
    }

    pub fn toolchain(&self) -> &ToolchainId {
        &self.toolchain
    }

    pub fn root(&self) -> &AnchoredSystemPath {
        &self.root
    }

    pub fn definition_sources(&self) -> &[AnchoredSystemPathBuf] {
        &self.definition_sources
    }

    pub fn data(&self) -> &ExternalResolutionData {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut ExternalResolutionData {
        &mut self.data
    }
}

/// Lifecycle state for resolution production, kept separate from terminal
/// generation data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalResolutionStatus {
    Pending,
    Complete,
}

/// A validated, immutable generation of external resolution knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalResolutionGeneration {
    domains: Box<[ExternalResolutionDomain]>,
}

impl ExternalResolutionGeneration {
    pub(crate) fn build(
        repository: &RepositoryKnowledge,
        mut domains: Vec<ExternalResolutionDomain>,
    ) -> Result<Self, ExternalResolutionError> {
        for domain in &mut domains {
            domain.definition_sources.sort();
            domain.definition_sources.dedup();
            if let ExternalResolutionData::Resolved { packages, .. } = &mut domain.data {
                for package in packages.iter_mut() {
                    package.identities.sort();
                    package.identities.dedup();
                }
                packages.sort_by(|left, right| left.package.cmp(&right.package));
            }
        }
        domains.sort_by(|left, right| {
            left.toolchain
                .cmp(&right.toolchain)
                .then_with(|| left.root.cmp(&right.root))
        });

        if let Some(duplicate) = domains
            .windows(2)
            .find(|pair| pair[0].toolchain == pair[1].toolchain)
        {
            return Err(ExternalResolutionError::DuplicateDomain {
                toolchain: duplicate[0].toolchain.clone(),
            });
        }

        for domain in &mut domains {
            if !repository.workspace_roots().any(|root| {
                root.toolchain() == &domain.toolchain && root.path() == domain.root.as_ref()
            }) {
                return Err(ExternalResolutionError::UnknownDomain {
                    toolchain: domain.toolchain.clone(),
                    root: domain.root.clone(),
                });
            }

            let ExternalResolutionData::Resolved { packages, .. } = &mut domain.data else {
                continue;
            };
            if let Some(duplicate) = packages
                .windows(2)
                .find(|pair| pair[0].package == pair[1].package)
            {
                return Err(ExternalResolutionError::DuplicatePackage {
                    toolchain: domain.toolchain.clone(),
                    identity: duplicate[0].package.clone(),
                });
            }
            for package in packages {
                let is_authoritative = (package.package() == "//"
                    && repository.root_javascript_scope().is_some())
                    || repository.scope(package.package()).is_some();
                if !is_authoritative {
                    return Err(ExternalResolutionError::UnknownPackage {
                        toolchain: domain.toolchain.clone(),
                        identity: package.package.clone(),
                    });
                }
            }
        }
        Ok(Self {
            domains: domains.into_boxed_slice(),
        })
    }

    pub(crate) fn domains(&self) -> &[ExternalResolutionDomain] {
        &self.domains
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ExternalResolutionError {
    #[error("toolchain {toolchain} contributed multiple external resolution domains")]
    DuplicateDomain { toolchain: ToolchainId },
    #[error("toolchain {toolchain} contributed unknown resolution domain root {root}")]
    UnknownDomain {
        toolchain: ToolchainId,
        root: AnchoredSystemPathBuf,
    },
    #[error("toolchain {toolchain} resolved unknown package {identity}")]
    UnknownPackage {
        toolchain: ToolchainId,
        identity: String,
    },
    #[error("toolchain {toolchain} resolved package {identity} more than once")]
    DuplicatePackage {
        toolchain: ToolchainId,
        identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExternalResolutionChanges {
    All,
    Packages(Vec<PackageResolutionChange>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageResolutionChange {
    pub package: String,
    pub added: Vec<ExternalPackageIdentity>,
    pub removed: Vec<ExternalPackageIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ExternalResolutionComparisonError {
    #[error("external resolution unavailable")]
    Unavailable,
    #[error("previous external resolution is missing package {0}")]
    MissingPackage(String),
}

/// Compares two complete, normalized resolution domains without knowing which
/// ecosystem produced them.
pub(crate) fn compare_resolution_data(
    current: &ExternalResolutionData,
    previous: &ExternalResolutionData,
    invalidate_all: bool,
    root_package: &str,
) -> Result<ExternalResolutionChanges, ExternalResolutionComparisonError> {
    fn resolved_packages(
        data: &ExternalResolutionData,
    ) -> Result<&[PackageResolution], ExternalResolutionComparisonError> {
        match data {
            ExternalResolutionData::Resolved {
                completeness: ResolutionCompleteness::Complete,
                packages,
                ..
            } => Ok(packages),
            ExternalResolutionData::Resolved {
                completeness: ResolutionCompleteness::Partial(_),
                ..
            }
            | ExternalResolutionData::Unavailable(_) => {
                Err(ExternalResolutionComparisonError::Unavailable)
            }
        }
    }
    let current = resolved_packages(current)?;
    let previous = resolved_packages(previous)?;
    if invalidate_all {
        return Ok(ExternalResolutionChanges::All);
    }

    let previous_by_package: BTreeMap<_, _> = previous
        .iter()
        .map(|package| (package.package(), package))
        .collect();
    let mut changes = Vec::new();
    for package in current {
        let previous = previous_by_package.get(package.package()).ok_or_else(|| {
            ExternalResolutionComparisonError::MissingPackage(package.package().to_string())
        })?;
        if package.identities() == previous.identities() {
            continue;
        }
        if package.package() == root_package {
            return Ok(ExternalResolutionChanges::All);
        }

        let previous: BTreeSet<_> = previous.identities().iter().collect();
        let current: BTreeSet<_> = package.identities().iter().collect();
        changes.push(PackageResolutionChange {
            package: package.package().to_string(),
            added: current
                .difference(&previous)
                .map(|identity| (*identity).clone())
                .collect(),
            removed: previous
                .difference(&current)
                .map(|identity| (*identity).clone())
                .collect(),
        });
    }
    Ok(ExternalResolutionChanges::Packages(changes))
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{
            PackageScopeObservation, RelationshipGroup, ScopeKind, WorkspaceRootObservation,
        },
        relationships::Relationship,
        toolchain::WorkspaceRoot,
    };

    fn repository() -> RepositoryKnowledge {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        RepositoryKnowledge::build(
            &root,
            Some(Some("root".to_string())),
            &[
                PackageScopeObservation {
                    identity: Some("app".to_string()),
                    definition_path: root.join_components(&["apps", "app", "package.json"]),
                    toolchain: ToolchainId::JAVASCRIPT,
                    scope_kind: ScopeKind::Package,
                },
                PackageScopeObservation {
                    identity: Some("crate".to_string()),
                    definition_path: root.join_components(&["crates", "crate", "Cargo.toml"]),
                    toolchain: ToolchainId::RUST,
                    scope_kind: ScopeKind::Package,
                },
            ],
            &[
                WorkspaceRootObservation::new(
                    WorkspaceRoot::new("npm", root.clone()),
                    ToolchainId::JAVASCRIPT,
                ),
                WorkspaceRootObservation::new(
                    WorkspaceRoot::new("cargo", root.clone()),
                    ToolchainId::RUST,
                ),
            ],
        )
        .unwrap()
    }

    fn resolved(packages: Vec<PackageResolution>) -> ExternalResolutionData {
        ExternalResolutionData::Resolved {
            completeness: ResolutionCompleteness::Complete,
            fingerprint: ResolutionFingerprint::new("fingerprint"),
            packages,
        }
    }

    #[test]
    fn declaration_view_preserves_effective_external_declarations() {
        let repository = repository();
        let relationships = RelationshipKnowledge::build(
            &repository,
            vec![RelationshipGroup::new(
                "app",
                vec![
                    Relationship::new(
                        "alias",
                        DependencyKind::Production,
                        RelationshipTarget::UnresolvedExternal {
                            name: "package".to_string(),
                            specifier: "1.0.0".to_string(),
                        },
                    ),
                    Relationship::new(
                        "alias",
                        DependencyKind::Development,
                        RelationshipTarget::UnresolvedExternal {
                            name: "ignored-duplicate".to_string(),
                            specifier: "2.0.0".to_string(),
                        },
                    ),
                    Relationship::new(
                        "peer",
                        DependencyKind::Peer { optional: false },
                        RelationshipTarget::UnresolvedExternal {
                            name: "peer".to_string(),
                            specifier: "3.0.0".to_string(),
                        },
                    ),
                ],
            )],
        )
        .unwrap();

        let view = ExternalDeclarations::build(&relationships);
        assert_eq!(view.declarations().len(), 2);
        let declaration = &view.declarations()[0];
        assert_eq!(declaration.source(), "app");
        assert_eq!(declaration.declaration_name(), "alias");
        assert_eq!(declaration.package_name(), "package");
        assert_eq!(declaration.specifier(), "1.0.0");
        assert_eq!(declaration.kind(), DependencyKind::Production);
    }

    #[test]
    fn resolved_empty_is_distinct_from_unavailable() {
        let complete_empty = ExternalResolutionGeneration::build(
            &repository(),
            vec![ExternalResolutionDomain::new(
                ToolchainId::JAVASCRIPT,
                AnchoredSystemPathBuf::default(),
                [AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap()],
                resolved(Vec::new()),
            )],
        )
        .unwrap();
        let unavailable = ExternalResolutionGeneration::build(
            &repository(),
            vec![ExternalResolutionDomain::new(
                ToolchainId::JAVASCRIPT,
                AnchoredSystemPathBuf::default(),
                [AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap()],
                ExternalResolutionData::Unavailable(ResolutionUnavailableReason::new(
                    "missing-definition",
                    "package-lock.json does not exist",
                )),
            )],
        )
        .unwrap();

        assert_ne!(complete_empty, unavailable);
    }

    #[test]
    fn package_resolution_state_preserves_hash_and_cache_semantics() {
        let unavailable = PackageResolutionState::Unavailable(ResolutionUnavailableReason::new(
            "missing",
            "missing lockfile",
        ));
        let partial = PackageResolutionState::Resolved {
            completeness: ResolutionCompleteness::Partial(ResolutionIncompleteReason::new(
                "partial",
                "partial resolution",
            )),
            fingerprint: ResolutionFingerprint::new("partial-hash"),
        };

        assert_eq!(unavailable.task_hash(), Some(""));
        assert!(unavailable.cache_eligible());
        assert_eq!(PackageResolutionState::Missing.task_hash(), None);
        assert!(!partial.cache_eligible());
    }

    #[test]
    fn generation_normalizes_all_ordering() {
        let source_a = AnchoredSystemPathBuf::from_raw("a.lock").unwrap();
        let source_z = AnchoredSystemPathBuf::from_raw("z.lock").unwrap();
        let identity_a = ExternalPackageIdentity::new("a", "1");
        let identity_z = ExternalPackageIdentity::new("z", "2");
        let make = |reverse: bool| {
            let (sources, identities) = if reverse {
                (
                    vec![source_z.clone(), source_a.clone(), source_z.clone()],
                    vec![identity_z.clone(), identity_a.clone(), identity_z.clone()],
                )
            } else {
                (
                    vec![source_a.clone(), source_z.clone()],
                    vec![identity_a.clone(), identity_z.clone()],
                )
            };
            let javascript = ExternalResolutionDomain::new(
                ToolchainId::JAVASCRIPT,
                AnchoredSystemPathBuf::default(),
                sources,
                resolved(vec![PackageResolution::new("app", identities)]),
            );
            let rust = ExternalResolutionDomain::new(
                ToolchainId::RUST,
                AnchoredSystemPathBuf::default(),
                [AnchoredSystemPathBuf::from_raw("Cargo.lock").unwrap()],
                resolved(vec![PackageResolution::new("crate", Vec::new())]),
            );
            let domains = if reverse {
                vec![rust, javascript]
            } else {
                vec![javascript, rust]
            };
            ExternalResolutionGeneration::build(&repository(), domains).unwrap()
        };

        let ordered = make(false);
        let reversed = make(true);
        assert_eq!(ordered, reversed);
        assert_eq!(ordered.domains()[0].toolchain(), &ToolchainId::JAVASCRIPT);
        let ExternalResolutionData::Resolved { packages, .. } = ordered.domains()[0].data() else {
            panic!("expected resolved data")
        };
        assert_eq!(packages[0].identities(), &[identity_a, identity_z]);
    }

    #[test]
    fn generation_rejects_duplicate_domains_and_unknown_packages() {
        let domain = || {
            ExternalResolutionDomain::new(
                ToolchainId::JAVASCRIPT,
                AnchoredSystemPathBuf::default(),
                Vec::new(),
                resolved(Vec::new()),
            )
        };
        assert!(matches!(
            ExternalResolutionGeneration::build(&repository(), vec![domain(), domain()]),
            Err(ExternalResolutionError::DuplicateDomain { toolchain })
                if toolchain == ToolchainId::JAVASCRIPT
        ));

        let unknown = ExternalResolutionDomain::new(
            ToolchainId::RUST,
            AnchoredSystemPathBuf::default(),
            Vec::new(),
            resolved(vec![PackageResolution::new("missing", Vec::new())]),
        );
        assert!(matches!(
            ExternalResolutionGeneration::build(&repository(), vec![unknown]),
            Err(ExternalResolutionError::UnknownPackage { identity, .. })
                if identity == "missing"
        ));
    }
}
