//! Foundational change knowledge for watch and affectedness.
//!
//! Ecosystems contribute immutable observations about package ownership,
//! membership/resolution triggers, and ignored byproduct prefixes. Core owns
//! subscriptions, coalescing, and generation publication.
//!
//! JavaScript observations are produced from repository knowledge plus the
//! active package manager. Cargo temporarily retains `Toolchain::watch_spec`
//! until its Rust port.

use std::collections::BTreeMap;

use crate::{
    knowledge::RepositoryKnowledge,
    package_manager::PackageManager,
    toolchain::{ToolchainId, WatchSpec},
};

/// Immutable change facts for the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeKnowledge {
    /// Manifest file names that can change package membership wherever they
    /// appear (e.g. `package.json` for JavaScript).
    membership_file_names: Vec<String>,
    /// Repo-root-relative unix paths that can change package membership
    /// (e.g. workspace configuration files).
    membership_paths: Vec<String>,
    /// Repo-root-relative unix paths that can change external resolution
    /// (e.g. lockfiles).
    resolution_paths: Vec<String>,
    /// Repo-root-relative directory prefixes containing ecosystem byproducts
    /// that must not feed watch loops.
    ignore_prefixes: Vec<String>,
    /// Package identity → repo-relative unix directory for ownership.
    package_directories: BTreeMap<String, String>,
}

impl ChangeKnowledge {
    /// Produce JavaScript change observations from repository knowledge.
    pub(crate) fn javascript(
        knowledge: &RepositoryKnowledge,
        package_manager: Option<&PackageManager>,
    ) -> Self {
        let mut membership_file_names = Vec::new();
        let mut membership_paths = Vec::new();
        let mut resolution_paths = Vec::new();

        let has_js = knowledge.root_javascript_scope().is_some()
            || knowledge
                .scopes()
                .any(|scope| scope.toolchain() == &ToolchainId::JAVASCRIPT);

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
            .scopes()
            .filter(|scope| scope.toolchain() == &ToolchainId::JAVASCRIPT)
            .map(|scope| {
                (
                    scope.identity().to_owned(),
                    scope.directory().to_unix().to_string(),
                )
            })
            .collect();

        Self {
            membership_file_names,
            membership_paths,
            resolution_paths,
            ignore_prefixes: Vec::new(),
            package_directories,
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
    /// Only workspace-configuration paths and ignore prefixes are projected
    /// into rediscovery triggers. Per-package `package.json` and lockfile
    /// changes continue to flow through `ChangeMapper` / lockfile content
    /// analysis so we preserve today's affectedness granularity; those facts
    /// remain available via [`Self::membership_file_names`] and
    /// [`Self::resolution_paths`].
    pub fn to_watch_spec(&self) -> WatchSpec {
        WatchSpec {
            definition_file_names: Vec::new(),
            definition_paths: self.membership_paths.clone(),
            ignore_prefixes: self.ignore_prefixes.clone(),
        }
    }

    /// Combine foundational change knowledge with toolchain WatchSpecs.
    pub fn combined_watch_spec(&self, toolchain_spec: WatchSpec) -> WatchSpec {
        let mut combined = self.to_watch_spec();
        combined.extend(toolchain_spec);
        combined
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::{
        knowledge::{PackageScopeObservation, ScopeKind, WorkspaceRootObservation},
        package_manager::PackageManager,
        toolchain::WorkspaceRoot,
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
        let change = ChangeKnowledge::javascript(&knowledge, None);
        assert!(change.membership_file_names().is_empty());
        assert!(change.resolution_paths().is_empty());
        assert!(change.package_directories().is_empty());
    }

    #[test]
    fn javascript_includes_package_json_and_lockfile() {
        let knowledge = javascript_repository();
        let change = ChangeKnowledge::javascript(&knowledge, Some(&PackageManager::Npm));
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
        let change = ChangeKnowledge::javascript(&knowledge, Some(&PackageManager::Pnpm));
        assert_eq!(change.resolution_paths(), ["pnpm-lock.yaml"]);
        let watch = change.to_watch_spec();
        assert!(
            watch
                .definition_paths
                .iter()
                .any(|path| path.contains("pnpm-workspace"))
        );
    }
}
