//! Cargo scope and affectedness planning from injected repository observations.
//! No Cargo metadata process, Rust compiler, or assembled turbo binary is used.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::{collections::HashMap, sync::Arc};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor, ToolchainId,
        WorkspaceRoot,
    },
};
use turborepo_scope::{
    GitChangeDetector,
    filter::{FilterResolver, ResolutionError},
};

struct ObservedCargoWorkspace(AbsoluteSystemPathBuf);

impl RepositoryContributor for ObservedCargoWorkspace {
    fn id(&self) -> ToolchainId {
        ToolchainId::RUST
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let package = |name: &str, relationships: Vec<Relationship>| {
                DiscoveredPackage::package(
                    Some(name.to_string()),
                    PackageJson::default(),
                    self.0.join_components(&["crates", name, "Cargo.toml"]),
                )
                .with_native_relationships(relationships)
            };
            let packages = vec![
                package(
                    "app",
                    vec![Relationship::internal("lib-a", DependencyKind::Production)],
                ),
                package(
                    "lib-a",
                    vec![Relationship::internal_input(
                        "test-util",
                        DependencyKind::Development,
                    )],
                ),
                package(
                    "test-util",
                    vec![Relationship::internal("lib-a", DependencyKind::Production)],
                ),
                DiscoveredPackage::aggregate(
                    "rust-workspace".to_string(),
                    PackageJson::default(),
                    self.0.join_component("Cargo.toml"),
                )
                .with_native_relationships(
                    ["app", "lib-a", "test-util"]
                        .into_iter()
                        .map(|name| Relationship::internal(name, DependencyKind::Production))
                        .collect(),
                ),
            ];
            Ok(DiscoveredPackages::new(
                packages,
                vec![WorkspaceRoot::new("cargo", self.0.clone())],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            let discovery = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                discovery.packages(),
                discovery.workspace_roots(),
            ))
        })
    }
}

struct ChangedCrate(&'static str);

impl GitChangeDetector for ChangedCrate {
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        _to_ref: Option<&str>,
        _include_uncommitted: bool,
        _allow_unknown_objects: bool,
        _merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        assert_eq!(from_ref, Some("HEAD"));
        Ok(HashMap::from([(
            PackageName::from(self.0),
            PackageInclusionReason::IncludedByFilter {
                filters: Vec::new(),
            },
        )]))
    }
}

fn graph(root: &AbsoluteSystemPath) -> PackageGraph {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            PackageGraph::builder_optional(root, None)
                .with_package_jsons(Some(HashMap::new()))
                .with_contributor(Arc::new(ObservedCargoWorkspace(root.to_owned())))
                .build(),
        )
        .unwrap()
}

#[test]
fn cargo_scopes_are_selected_by_authoritative_manifest_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
    let graph = graph(&root);
    let directories = graph
        .package_scope_directories()
        .map(|(name, directory)| (name.to_string(), directory.to_unix().to_string()))
        .collect::<HashMap<_, _>>();
    assert_eq!(directories["app"], "crates/app");
    assert_eq!(directories["lib-a"], "crates/lib-a");
    assert_eq!(directories["rust-workspace"], "");

    let resolver =
        FilterResolver::new_with_change_detector(&graph, &root, None, ChangedCrate("lib-a"));
    let (selected, _) = resolver.resolve(&None, &["lib-a".to_string()]).unwrap();
    assert_eq!(
        selected
            .into_keys()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
        ["lib-a"]
    );
}

#[test]
fn cargo_production_edges_affect_dependents_without_a_task_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
    let graph = graph(&root);
    let resolver =
        FilterResolver::new_with_change_detector(&graph, &root, None, ChangedCrate("lib-a"));
    let (affected, _) = resolver
        .resolve(&Some((Some("HEAD".to_string()), None)), &[])
        .unwrap();
    let names = affected
        .into_keys()
        .map(|name| name.to_string())
        .collect::<std::collections::HashSet<_>>();
    assert!(names.contains("lib-a"), "{names:?}");
    assert!(
        names.contains("test-util"),
        "the non-dev edge from test-util must propagate: {names:?}"
    );
    assert!(
        names.contains("app"),
        "transitive dependents must be included: {names:?}"
    );
    assert!(names.contains("rust-workspace"), "{names:?}");
}
