use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_lockfiles::Lockfile;

use super::{ExternalDependencyChange, PackageGraph, PackageName, ROOT_PKG_NAME, WorkspacePackage};
use crate::{
    external_resolution::{
        ExternalPackageIdentity, ExternalResolutionChanges, ExternalResolutionData,
        ExternalResolutionDomain, ExternalResolutionGeneration, JAVASCRIPT_RESOLUTION_DOMAIN,
        PackageResolution, ResolutionCompleteness, ResolutionUnavailableReason,
        compare_resolution_data,
    },
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    package_json::DependencyKind,
    package_manager::PackageManager,
    toolchain::ToolchainId,
};

/// One JavaScript resolution snapshot. Both production paths create this
/// exact generation-backed shape.
pub(super) struct ResolutionSnapshot {
    pub generation: Arc<ExternalResolutionGeneration>,
    pub warning: Option<String>,
}

fn resolution_packages(knowledge: &RepositoryKnowledge) -> Vec<(&str, &AnchoredSystemPath)> {
    let mut packages = knowledge.package_json_packages().collect::<Vec<_>>();
    packages.sort_unstable_by_key(|(identity, _)| *identity);
    packages
}

pub(super) fn external_dependencies(
    knowledge: &RepositoryKnowledge,
    relationships: &RelationshipKnowledge,
) -> HashMap<String, BTreeMap<String, String>> {
    let packages = resolution_packages(knowledge);
    let directories = packages
        .iter()
        .map(|(identity, directory)| ((*identity).to_string(), *directory))
        .collect::<HashMap<_, _>>();
    let mut by_source: BTreeMap<String, BTreeMap<String, String>> = packages
        .into_iter()
        .map(|(identity, _)| (identity.to_string(), BTreeMap::new()))
        .collect();
    // Iterate the relationship groups directly rather than materializing an
    // owned `ExternalDeclarations` (one struct per declaration, cloning the
    // group source and declaration name that this loop only reads). Only the
    // `name`/`specifier` that actually land in `by_source` are cloned. The
    // per-group dedup by declaration name — first occurrence wins, applied
    // before the external filter — matches `ExternalDeclarations::build`.
    for group in relationships.groups() {
        let Some(dependencies) = by_source.get_mut(group.source()) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for relationship in group.relationships() {
            if !seen.insert(relationship.declaration_name()) {
                continue;
            }
            let crate::relationships::RelationshipTarget::UnresolvedExternal { name, specifier } =
                relationship.target()
            else {
                continue;
            };
            if matches!(relationship.kind(), DependencyKind::Peer { .. }) {
                continue;
            }
            dependencies
                .entry(name.clone())
                .or_insert_with(|| specifier.clone());
        }
    }

    by_source
        .into_iter()
        .filter_map(|(identity, dependencies)| {
            let directory = directories.get(&identity)?;
            Some((directory.to_unix().to_string(), dependencies))
        })
        .collect()
}

pub(super) fn unavailable_resolution(
    knowledge: &RepositoryKnowledge,
    mut domains: Vec<ExternalResolutionDomain>,
    definition_source: AnchoredSystemPathBuf,
    code: &str,
    message: String,
    warning: Option<String>,
) -> Result<ResolutionSnapshot, String> {
    let members = resolution_packages(knowledge)
        .into_iter()
        .map(|(identity, _)| identity.to_string())
        .collect::<Vec<_>>();
    domains.push(ExternalResolutionDomain::new(
        JAVASCRIPT_RESOLUTION_DOMAIN.clone(),
        ToolchainId::JAVASCRIPT,
        AnchoredSystemPathBuf::default(),
        members,
        [definition_source],
        ExternalResolutionData::Unavailable(ResolutionUnavailableReason::new(code, message)),
    ));
    let generation = ExternalResolutionGeneration::build(knowledge, domains)
        .map_err(|error| error.to_string())?;
    Ok(ResolutionSnapshot {
        generation: Arc::new(generation),
        warning,
    })
}

pub(super) fn resolve_dependencies(
    knowledge: &RepositoryKnowledge,
    mut domains: Vec<ExternalResolutionDomain>,
    lockfile: &dyn Lockfile,
    external_dependencies: HashMap<String, BTreeMap<String, String>>,
    ignore_missing_packages: bool,
    definition_source: AnchoredSystemPathBuf,
) -> Result<ResolutionSnapshot, String> {
    let closures = match turborepo_lockfiles::all_transitive_closures_sorted(
        lockfile,
        external_dependencies,
        ignore_missing_packages,
    ) {
        Ok(closures) => closures,
        Err(error) => {
            let message = error.to_string();
            return unavailable_resolution(
                knowledge,
                domains,
                definition_source,
                "closure-unavailable",
                message.clone(),
                Some(message),
            );
        }
    };
    let packages = resolution_packages(knowledge)
        .into_iter()
        .map(|(identity, directory)| {
            let exact_identities = closures
                .get(directory.to_unix().as_str())
                .into_iter()
                .flatten()
                .map(|package| {
                    let mut identity =
                        ExternalPackageIdentity::new(package.key.clone(), package.version.clone());
                    if let Some(human_name) = lockfile.human_name(package) {
                        identity = identity.with_human_name(human_name);
                    }
                    identity
                });
            PackageResolution::new(identity, exact_identities)
        })
        .collect::<Vec<_>>();
    let members = packages
        .iter()
        .map(|package| package.package().to_string())
        .collect::<Vec<_>>();
    domains.push(ExternalResolutionDomain::new(
        JAVASCRIPT_RESOLUTION_DOMAIN.clone(),
        ToolchainId::JAVASCRIPT,
        AnchoredSystemPathBuf::default(),
        members,
        [definition_source],
        ExternalResolutionData::Resolved {
            completeness: ResolutionCompleteness::Complete,
            packages,
        },
    ));
    let generation = ExternalResolutionGeneration::build(knowledge, domains)
        .map_err(|error| error.to_string())?;
    Ok(ResolutionSnapshot {
        generation: Arc::new(generation),
        warning: None,
    })
}

fn resolution_data(
    generation: &ExternalResolutionGeneration,
) -> Result<&ExternalResolutionData, ChangedPackagesError> {
    generation
        .domain(&JAVASCRIPT_RESOLUTION_DOMAIN)
        .map(ExternalResolutionDomain::data)
        .ok_or(ChangedPackagesError::ResolutionUnavailable)
}

impl PackageGraph {
    pub fn changed_packages_from_lockfile_contents(
        &self,
        previous_lockfile_contents: &[u8],
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let package_manager = self
            .package_manager()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let root_package_json = self
            .root_package_json
            .as_ref()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let yarnrc = matches!(package_manager, PackageManager::Berry)
            .then(|| crate::package_manager::yarnrc::YarnRc::from_file(self.repo_root()))
            .transpose()?;
        let previous = package_manager.parse_lockfile(
            root_package_json,
            previous_lockfile_contents,
            yarnrc,
        )?;
        self.changed_packages_from_lockfile(previous.as_ref())
    }

    /// Returns packages whose normalized external resolution changed from the
    /// provided previous lockfile. Callers remain responsible for detecting
    /// descriptor changes independently.
    pub fn changed_packages_from_lockfile(
        &self,
        previous_lockfile: &dyn Lockfile,
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let current_lockfile = self.lockfile().ok_or(ChangedPackagesError::NoLockfile)?;
        let package_manager = self
            .package_manager()
            .ok_or(ChangedPackagesError::NoLockfile)?;
        let definition_source = AnchoredSystemPathBuf::from_raw(package_manager.lockfile_name())?;
        let previous_resolution = resolve_dependencies(
            &self.knowledge,
            Vec::new(),
            previous_lockfile,
            external_dependencies(&self.knowledge, &self.relationship_knowledge),
            true,
            definition_source,
        )
        .map_err(ChangedPackagesError::Resolution)?;
        let current_resolution = self
            .external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changes = compare_resolution_data(
            resolution_data(
                current_resolution
                    .generation
                    .as_deref()
                    .ok_or(ChangedPackagesError::ResolutionUnavailable)?,
            )?,
            resolution_data(&previous_resolution.generation)?,
            current_lockfile.global_change(previous_lockfile),
            ROOT_PKG_NAME,
        )
        .map_err(|_| ChangedPackagesError::ResolutionUnavailable)?;

        let all_changes = || {
            self.package_task_contexts()
                .map(|context| ExternalDependencyChange {
                    package: WorkspacePackage {
                        name: context.package().clone(),
                        path: context.directory().to_owned(),
                    },
                    added: Vec::new(),
                    removed: Vec::new(),
                })
                .collect()
        };
        let ExternalResolutionChanges::Packages(changes) = changes else {
            return Ok(all_changes());
        };
        changes
            .into_iter()
            .map(|change| {
                let name = PackageName::from(change.package);
                let context = self
                    .package_task_context(&name)
                    .ok_or(ChangedPackagesError::ResolutionUnavailable)?;
                let to_lockfile_package = |identity: ExternalPackageIdentity| {
                    turborepo_lockfiles::Package::new(identity.key(), identity.version())
                };
                Ok(ExternalDependencyChange {
                    package: WorkspacePackage {
                        name,
                        path: context.directory().to_owned(),
                    },
                    added: change.added.into_iter().map(to_lockfile_package).collect(),
                    removed: change
                        .removed
                        .into_iter()
                        .map(to_lockfile_package)
                        .collect(),
                })
            })
            .collect()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ChangedPackagesError {
    #[error("No lockfile")]
    NoLockfile,
    #[error("External resolution unavailable")]
    ResolutionUnavailable,
    #[error("External resolution failed: {0}")]
    Resolution(String),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("Package manager error: {0}")]
    PackageManager(#[from] crate::package_manager::Error),
    #[error("Yarn config error: {0}")]
    Yarnrc(#[from] crate::package_manager::yarnrc::Error),
}
