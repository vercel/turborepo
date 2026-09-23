//! Test-only discovery and in-memory package graph fixtures.
//!
//! Enable the `test-util` feature in a downstream crate's dev-dependencies to
//! use these helpers without changing the production dependency graph.

use std::collections::{BTreeMap, HashMap};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_lockfiles::Lockfile;

use crate::{
    discovery::{self, DiscoveryResponse, PackageDiscovery, WorkspaceData},
    package_graph::{self, PackageGraph},
    package_json::PackageJson,
    package_manager::PackageManager,
};

/// An in-memory discovery strategy with a configurable package manager and
/// workspace list. Both discovery methods return the same snapshot.
#[derive(Debug, Clone)]
pub struct MockPackageDiscovery {
    response: DiscoveryResponse,
}

impl MockPackageDiscovery {
    pub fn new(package_manager: PackageManager) -> Self {
        Self {
            response: DiscoveryResponse {
                package_manager,
                workspaces: Vec::new(),
            },
        }
    }

    pub fn with_workspaces(mut self, workspaces: Vec<WorkspaceData>) -> Self {
        self.response.workspaces = workspaces;
        self
    }
}

impl PackageDiscovery for MockPackageDiscovery {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, discovery::Error> {
        Ok(self.response.clone())
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, discovery::Error> {
        self.discover_packages().await
    }
}

/// Builds a JavaScript package graph from typed manifests, without reading
/// workspace manifests, walking the filesystem, or spawning the `turbo` binary.
/// External resolution is skipped unless a lockfile is supplied explicitly.
pub struct PackageGraphFixture<'a> {
    repo_root: &'a AbsoluteSystemPath,
    root_package_json: PackageJson,
    package_manager: PackageManager,
    package_jsons: HashMap<AbsoluteSystemPathBuf, PackageJson>,
    lockfile: Option<Box<dyn Lockfile>>,
}

impl<'a> PackageGraphFixture<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            root_package_json: PackageJson::default(),
            package_manager: PackageManager::Npm,
            package_jsons: HashMap::new(),
            lockfile: None,
        }
    }

    pub fn with_package_manager(mut self, package_manager: PackageManager) -> Self {
        self.package_manager = package_manager;
        self
    }

    pub fn with_root_package_json(mut self, package_json: PackageJson) -> Self {
        self.root_package_json = package_json;
        self
    }

    /// Add a named package at a repository-relative directory (e.g.
    /// `packages/web`). For custom scripts or dependency kinds, use
    /// [`Self::with_package_json`] instead.
    pub fn with_package(self, name: &str, directory: &str) -> Self {
        self.with_package_json(
            directory,
            PackageJson {
                name: Some(Spanned::new(name.to_string())),
                ..Default::default()
            },
        )
    }

    /// Add a typed package manifest at a repository-relative directory.
    pub fn with_package_json(mut self, directory: &str, package_json: PackageJson) -> Self {
        let directory = AnchoredSystemPathBuf::try_from(directory)
            .expect("fixture package directory must be repository-relative");
        let path = self
            .repo_root
            .resolve(&directory)
            .join_component("package.json");
        self.package_jsons.insert(path, package_json);
        self
    }

    /// Add an internal workspace dependency between two previously added
    /// packages. The `workspace:*` specifier works with all package managers.
    pub fn with_dependency(mut self, from: &str, to: &str) -> Self {
        assert!(
            self.package_jsons.values().any(|package| package
                .name
                .as_ref()
                .is_some_and(|name| name.as_str() == to)),
            "fixture package {to} does not exist"
        );
        let package = self
            .package_jsons
            .values_mut()
            .find(|package| {
                package
                    .name
                    .as_ref()
                    .is_some_and(|name| name.as_str() == from)
            })
            .unwrap_or_else(|| panic!("fixture package {from} does not exist"));
        package
            .dependencies
            .get_or_insert_with(BTreeMap::new)
            .insert(to.to_string(), "workspace:*".to_string());
        self
    }

    /// Supply external resolution when a test needs lockfile-backed graph data.
    pub fn with_lockfile(mut self, lockfile: Box<dyn Lockfile>) -> Self {
        self.lockfile = Some(lockfile);
        self
    }

    pub async fn build(self) -> Result<PackageGraph, package_graph::Error> {
        let Self {
            repo_root,
            root_package_json,
            package_manager,
            package_jsons,
            lockfile,
        } = self;
        let builder = PackageGraph::builder(repo_root, root_package_json)
            .with_package_manager(package_manager.clone())
            .with_package_discovery(MockPackageDiscovery::new(package_manager))
            .with_package_jsons(Some(package_jsons));
        match lockfile {
            Some(lockfile) => builder.with_lockfile(Some(lockfile)).build().await,
            None => builder.without_external_dependencies().build().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPath;

    use super::*;
    use crate::package_graph::{PackageName, PackageNode};

    #[tokio::test]
    async fn discovery_returns_configured_workspaces_from_both_methods() {
        let dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(dir.path()).unwrap();
        let workspace = WorkspaceData::new(
            root.join_components(&["apps", "web", "package.json"]),
            Some(root.join_components(&["apps", "web", "turbo.json"])),
        )
        .unwrap();
        let discovery = MockPackageDiscovery::new(PackageManager::Pnpm6)
            .with_workspaces(vec![workspace.clone()]);

        for response in [
            discovery.discover_packages().await.unwrap(),
            discovery.discover_packages_blocking().await.unwrap(),
        ] {
            assert_eq!(response.package_manager, PackageManager::Pnpm6);
            assert_eq!(response.workspaces, vec![workspace.clone()]);
        }
    }

    #[tokio::test]
    async fn builds_a_graph_with_paths_scripts_and_internal_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(dir.path()).unwrap();
        let graph = PackageGraphFixture::new(root)
            .with_package("web", "apps/web")
            .with_package_json(
                "packages/lib",
                PackageJson {
                    name: Some(Spanned::new("lib".to_string())),
                    scripts: BTreeMap::from([(
                        "build".to_string(),
                        Spanned::new("echo build".to_string()),
                    )]),
                    ..Default::default()
                },
            )
            .with_dependency("web", "lib")
            .build()
            .await
            .unwrap();

        let web = PackageNode::Workspace(PackageName::Other("web".to_string()));
        let lib = PackageNode::Workspace(PackageName::Other("lib".to_string()));
        assert!(graph.immediate_dependencies(&web).unwrap().contains(&lib));
        let snapshot = graph.repository_discovery_snapshot();
        let lib_scope = snapshot
            .scopes
            .iter()
            .find(|scope| scope.name == *lib.as_package_name())
            .unwrap();
        assert_eq!(
            lib_scope.manifest_path,
            root.join_components(&["packages", "lib", "package.json"])
        );
        assert_eq!(lib_scope.tasks, vec!["build"]);
        assert_eq!(graph.package_manager(), Some(&PackageManager::Npm));
    }
}
