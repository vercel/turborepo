//! Foundational change knowledge for watch and affectedness.
//!
//! Repository knowledge projects package ownership. Ecosystems contribute
//! immutable observations about membership/resolution triggers and ignored
//! byproduct prefixes. Core owns subscriptions, coalescing, and publication.
//!
//! JavaScript change facts are projected from repository knowledge plus the
//! active package manager. Native ecosystems contribute observations with
//! their package discovery result.

use std::collections::{BTreeMap, HashSet};

use crate::{
    knowledge::RepositoryKnowledge, package_manager::PackageManager, toolchain::WatchSpec,
};

/// Immutable change facts for the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeKnowledge {
    /// Manifest file names that can change package membership wherever they
    /// appear (e.g. `package.json` for JavaScript).
    membership_file_names: Vec<String>,
    /// Manifest names whose changes require full package rediscovery.
    rediscovery_file_names: Vec<String>,
    /// Repo-root-relative unix paths that can change package membership
    /// (e.g. workspace configuration files).
    membership_paths: Vec<String>,
    /// Native definition/resolution paths whose changes require rediscovery.
    rediscovery_paths: Vec<String>,
    /// Repo-root-relative unix paths that can change external resolution
    /// (e.g. lockfiles).
    resolution_paths: Vec<String>,
    /// Repo-root-relative directory prefixes containing ecosystem byproducts
    /// that must not feed watch loops.
    ignore_prefixes: Vec<String>,
    /// Package identity → repo-relative unix directory for ownership.
    package_directories: BTreeMap<String, String>,
}

/// Parser-neutral change facts contributed by one discovery producer.
///
/// Definition names must be basenames. Resolution paths and ignore prefixes
/// must be non-empty, normalized repository-relative Unix paths. Invalid
/// observations fail package-graph construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeObservation {
    rediscovery_file_names: Vec<String>,
    resolution_paths: Vec<String>,
    ignore_prefixes: Vec<String>,
}

impl ChangeObservation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rediscovery_file_name(mut self, name: impl Into<String>) -> Self {
        self.rediscovery_file_names.push(name.into());
        self
    }

    pub fn with_resolution_path(mut self, path: impl Into<String>) -> Self {
        self.resolution_paths.push(path.into());
        self
    }

    pub fn with_ignore_prefix(mut self, path: impl Into<String>) -> Self {
        self.ignore_prefixes.push(path.into());
        self
    }

    fn validate(&self) -> Result<(), Error> {
        for name in &self.rediscovery_file_names {
            if name.is_empty() || matches!(name.as_str(), "." | "..") || name.contains(['/', '\\'])
            {
                return Err(Error::InvalidFileName(name.clone()));
            }
        }
        for path in self.resolution_paths.iter().chain(&self.ignore_prefixes) {
            let valid = !path.is_empty()
                && !path.contains('\\')
                && turbopath::RelativeUnixPath::new(path).is_ok()
                && path
                    .split('/')
                    .all(|component| !component.is_empty() && !matches!(component, "." | ".."));
            if !valid {
                return Err(Error::InvalidPath(path.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("invalid change observation file name {0:?}")]
    InvalidFileName(String),
    #[error("invalid change observation path {0:?}")]
    InvalidPath(String),
}

impl ChangeKnowledge {
    /// Project package.json change facts and compose validated native facts.
    pub(crate) fn build(
        knowledge: &RepositoryKnowledge,
        package_manager: Option<&PackageManager>,
        native: Vec<ChangeObservation>,
    ) -> Result<Self, Error> {
        let mut membership_file_names = Vec::new();
        let mut membership_paths = Vec::new();
        let mut resolution_paths = Vec::new();

        let has_js = knowledge.package_json_packages().next().is_some();

        if has_js {
            membership_file_names.push("package.json".to_string());
            if let Some(package_manager) = package_manager {
                resolution_paths.push(package_manager.lockfile_name().to_string());
                if let Some(workspace_config) = package_manager.workspace_configuration_path() {
                    membership_paths.push(workspace_config.to_string());
                }
            }
        }

        let package_directories = knowledge
            .package_json_packages()
            .filter(|(identity, _)| *identity != "//")
            .map(|(identity, directory)| (identity.to_owned(), directory.to_unix().to_string()))
            .collect();

        let mut change = Self {
            membership_file_names,
            rediscovery_file_names: Vec::new(),
            membership_paths,
            rediscovery_paths: Vec::new(),
            resolution_paths,
            ignore_prefixes: Vec::new(),
            package_directories,
        };

        for observation in native {
            observation.validate()?;
            change
                .membership_file_names
                .extend(observation.rediscovery_file_names.iter().cloned());
            change
                .rediscovery_file_names
                .extend(observation.rediscovery_file_names);
            change
                .rediscovery_paths
                .extend(observation.resolution_paths.iter().cloned());
            change.resolution_paths.extend(observation.resolution_paths);
            change.ignore_prefixes.extend(observation.ignore_prefixes);
        }
        change.canonicalize();
        Ok(change)
    }

    fn canonicalize(&mut self) {
        for values in [
            &mut self.membership_file_names,
            &mut self.rediscovery_file_names,
            &mut self.membership_paths,
            &mut self.rediscovery_paths,
            &mut self.resolution_paths,
            &mut self.ignore_prefixes,
        ] {
            let mut seen = HashSet::with_capacity(values.len());
            values.retain(|value| seen.insert(value.clone()));
        }
    }

    pub fn membership_file_names(&self) -> &[String] {
        &self.membership_file_names
    }

    pub fn membership_paths(&self) -> &[String] {
        &self.membership_paths
    }

    pub fn resolution_paths(&self) -> &[String] {
        &self.resolution_paths
    }

    pub fn ignore_prefixes(&self) -> &[String] {
        &self.ignore_prefixes
    }

    pub fn package_directories(&self) -> &BTreeMap<String, String> {
        &self.package_directories
    }

    /// Project change knowledge into the watcher `WatchSpec` shape.
    ///
    /// Only rediscovery paths/names and ignore prefixes are projected
    /// into rediscovery triggers. Per-package `package.json` and lockfile
    /// changes continue to flow through `ChangeMapper` / lockfile content
    /// analysis so we preserve today's affectedness granularity; those facts
    /// remain available via [`Self::membership_file_names`] and
    /// [`Self::resolution_paths`].
    pub fn to_watch_spec(&self) -> WatchSpec {
        WatchSpec {
            definition_file_names: self.rediscovery_file_names.clone(),
            definition_paths: self
                .membership_paths
                .iter()
                .chain(&self.rediscovery_paths)
                .cloned()
                .collect(),
            ignore_prefixes: self.ignore_prefixes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{PackageScopeObservation, ScopeKind, WorkspaceRootObservation},
        package_manager::PackageManager,
        toolchain::{ToolchainId, WorkspaceRoot},
    };

    fn empty_repository() -> RepositoryKnowledge {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        RepositoryKnowledge::build(&root, None, &[], &[]).unwrap()
    }

    fn javascript_repository() -> RepositoryKnowledge {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        RepositoryKnowledge::build(
            &root,
            Some(Some("root".to_string())),
            &[PackageScopeObservation {
                identity: Some("web".to_string()),
                name_source: None,
                definition_path: root.join_components(&["apps", "web", "package.json"]),
                toolchain: ToolchainId::JAVASCRIPT,
                scope_kind: ScopeKind::Package,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("npm", root.clone()),
                ToolchainId::JAVASCRIPT,
            )],
        )
        .unwrap()
    }

    #[test]
    fn empty_repository_has_no_js_triggers() {
        let knowledge = empty_repository();
        let change = ChangeKnowledge::build(&knowledge, None, Vec::new()).unwrap();
        assert!(change.membership_file_names().is_empty());
        assert!(change.resolution_paths().is_empty());
        assert!(change.package_directories().is_empty());
    }

    #[test]
    fn javascript_includes_package_json_and_lockfile() {
        let knowledge = javascript_repository();
        let change =
            ChangeKnowledge::build(&knowledge, Some(&PackageManager::Npm), Vec::new()).unwrap();
        assert_eq!(change.membership_file_names(), ["package.json"]);
        assert_eq!(change.resolution_paths(), ["package-lock.json"]);
        assert!(change.package_directories().contains_key("web"));
        let watch = change.to_watch_spec();
        assert!(watch.definition_file_names.is_empty());
        assert!(watch.definition_paths.is_empty()); // npm has no workspace config path
    }

    #[test]
    fn pnpm_workspace_config_projects_into_watch_spec() {
        let knowledge = javascript_repository();
        let change =
            ChangeKnowledge::build(&knowledge, Some(&PackageManager::Pnpm), Vec::new()).unwrap();
        assert_eq!(change.resolution_paths(), ["pnpm-lock.yaml"]);
        let watch = change.to_watch_spec();
        assert!(
            watch
                .definition_paths
                .iter()
                .any(|path| path.contains("pnpm-workspace"))
        );
    }

    #[test]
    fn package_json_change_facts_ignore_producer_identity() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let custom = ToolchainId::new("custom");
        let knowledge = RepositoryKnowledge::build(
            &root,
            None,
            &[PackageScopeObservation {
                identity: Some("web".to_string()),
                name_source: None,
                definition_path: root.join_components(&["apps", "web", "package.json"]),
                toolchain: custom.clone(),
                scope_kind: ScopeKind::Package,
            }],
            &[WorkspaceRootObservation::new(
                WorkspaceRoot::new("custom", root.clone()),
                custom,
            )],
        )
        .unwrap();

        let change =
            ChangeKnowledge::build(&knowledge, Some(&PackageManager::Npm), Vec::new()).unwrap();
        assert_eq!(change.membership_file_names(), ["package.json"]);
        assert!(change.package_directories().contains_key("web"));
    }

    #[test]
    fn native_observations_do_not_require_matching_scope_provenance() {
        let observation = ChangeObservation::new()
            .with_rediscovery_file_name("Cargo.toml")
            .with_resolution_path("Cargo.lock")
            .with_ignore_prefix("target");
        let change = ChangeKnowledge::build(&empty_repository(), None, vec![observation]).unwrap();

        assert_eq!(change.membership_file_names(), ["Cargo.toml"]);
        assert_eq!(change.resolution_paths(), ["Cargo.lock"]);
        assert_eq!(change.ignore_prefixes(), ["target"]);
    }

    #[test]
    fn change_facts_preserve_core_resolution_priority_and_deduplicate() {
        let cargo = ChangeObservation::new()
            .with_rediscovery_file_name("Cargo.toml")
            .with_resolution_path("Cargo.lock");
        let duplicate = ChangeObservation::new()
            .with_resolution_path("Cargo.lock")
            .with_rediscovery_file_name("Cargo.toml");

        let change = ChangeKnowledge::build(
            &javascript_repository(),
            Some(&PackageManager::Npm),
            vec![cargo, duplicate],
        )
        .unwrap();

        assert_eq!(
            change.membership_file_names(),
            ["package.json", "Cargo.toml"]
        );
        assert_eq!(
            change.resolution_paths(),
            ["package-lock.json", "Cargo.lock"]
        );
    }

    #[test]
    fn malformed_native_observations_are_rejected() {
        for observation in [
            ChangeObservation::new().with_rediscovery_file_name("../Cargo.toml"),
            ChangeObservation::new().with_resolution_path("../Cargo.lock"),
            ChangeObservation::new().with_ignore_prefix(""),
        ] {
            assert!(ChangeKnowledge::build(&empty_repository(), None, vec![observation]).is_err());
        }
    }
}
