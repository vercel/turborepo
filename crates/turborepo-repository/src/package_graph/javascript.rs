use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_lockfiles::Lockfile;

use super::{
    ExternalDependencyChange, PackageGraph, PackageName, ROOT_PKG_NAME, WorkspacePackage,
    builder::{ClosureHasher, apply_resolution_fingerprints},
};
use crate::{
    external_resolution::{
        ExternalDeclarations, ExternalPackageIdentity, ExternalResolutionChanges,
        ExternalResolutionData, ExternalResolutionDomain, ExternalResolutionGeneration,
        PackageResolution, ResolutionCompleteness, ResolutionFingerprint,
        ResolutionUnavailableReason, compare_resolution_data,
    },
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    package_json::DependencyKind,
    package_manager::PackageManager,
    toolchain::ToolchainId,
};

/// One JavaScript resolution snapshot. Both production paths create this
/// exact generation-backed shape; compatibility projections onto `PackageInfo`
/// have been deleted.
pub(super) struct ResolutionSnapshot {
    pub generation: Arc<ExternalResolutionGeneration>,
    pub warning: Option<String>,
}

fn scope_directory_and_toolchain<'a>(
    knowledge: &'a RepositoryKnowledge,
    name: &PackageName,
) -> Option<(&'a AnchoredSystemPath, &'a ToolchainId)> {
    match name {
        PackageName::Root => knowledge
            .root_javascript_scope()
            .map(|scope| (knowledge.repository_directory(), scope.toolchain())),
        PackageName::Other(name) => knowledge
            .scope(name)
            .map(|scope| (scope.directory(), scope.toolchain())),
    }
}

fn resolution_packages(knowledge: &RepositoryKnowledge) -> Vec<(&str, &AnchoredSystemPath)> {
    let mut packages = Vec::new();
    if knowledge.root_javascript_scope().is_some() {
        packages.push((ROOT_PKG_NAME, knowledge.repository_directory()));
    }
    packages.extend(
        knowledge
            .scopes()
            .filter(|scope| scope.toolchain() == &ToolchainId::JAVASCRIPT)
            .map(|scope| (scope.identity(), scope.directory())),
    );
    packages.sort_unstable_by_key(|(identity, _)| *identity);
    packages
}

pub(super) fn external_dependencies(
    knowledge: &RepositoryKnowledge,
    relationships: &RelationshipKnowledge,
) -> HashMap<String, BTreeMap<String, String>> {
    let mut by_source: BTreeMap<String, BTreeMap<String, String>> = resolution_packages(knowledge)
        .into_iter()
        .map(|(identity, _)| (identity.to_string(), BTreeMap::new()))
        .collect();
    for declaration in ExternalDeclarations::build(relationships).declarations() {
        if matches!(declaration.kind(), DependencyKind::Peer { .. }) {
            continue;
        }
        let Some(dependencies) = by_source.get_mut(declaration.source()) else {
            continue;
        };
        dependencies
            .entry(declaration.package_name().to_string())
            .or_insert_with(|| declaration.specifier().to_string());
    }

    by_source
        .into_iter()
        .filter_map(|(identity, dependencies)| {
            let name = PackageName::from(identity.as_str());
            let (directory, toolchain) = scope_directory_and_toolchain(knowledge, &name)?;
            (toolchain == &ToolchainId::JAVASCRIPT)
                .then(|| (directory.to_unix().to_string(), dependencies))
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
    closure_hasher: Option<&ClosureHasher>,
) -> Result<ResolutionSnapshot, String> {
    domains.push(ExternalResolutionDomain::new(
        ToolchainId::JAVASCRIPT,
        AnchoredSystemPathBuf::default(),
        [definition_source],
        ExternalResolutionData::Unavailable(ResolutionUnavailableReason::new(code, message)),
    ));
    apply_resolution_fingerprints(&mut domains, closure_hasher);
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
    closure_hasher: Option<&ClosureHasher>,
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
                closure_hasher,
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
    let fingerprint = ResolutionFingerprint::from_packages(&packages);
    domains.push(ExternalResolutionDomain::new(
        ToolchainId::JAVASCRIPT,
        AnchoredSystemPathBuf::default(),
        [definition_source],
        ExternalResolutionData::Resolved {
            completeness: ResolutionCompleteness::Complete,
            fingerprint,
            packages,
        },
    ));
    apply_resolution_fingerprints(&mut domains, closure_hasher);
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
        .domains()
        .iter()
        .find(|domain| domain.toolchain() == &ToolchainId::JAVASCRIPT)
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
            .root_package_json()
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
            None,
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
