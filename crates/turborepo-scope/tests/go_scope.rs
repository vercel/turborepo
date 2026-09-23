//! Go scope selection at the filter and inference boundary.
//!
//! Every case builds repository knowledge from in-memory observation inputs
//! (`RepositoryContributor` plus the discovery/manifest seams), so no test here
//! invokes `go` or the assembled `turbo` binary. End-to-end coverage for real
//! Go execution, caching, pruning, pass-through arguments, and process
//! behaviour lives in `crates/turborepo/tests/go_workspace_test.rs`.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::{collections::HashMap, sync::Arc};

use tempfile::TempDir;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    package_manager::PackageManager,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor, ToolchainId,
        WorkspaceRoot,
    },
};
use turborepo_scope::{
    GitChangeDetector,
    filter::{FilterResolver, PackageInference, ResolutionError},
};

/// Native scopes observed in memory: `api` depends on `lib`, `service/v2` is a
/// versioned identity, and `independent` shares no relationship with any of
/// them.
struct GoFixtureContributor {
    root: AbsoluteSystemPathBuf,
}

impl RepositoryContributor for GoFixtureContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::GO
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let package = |name: &str, directory: &str, relationships: Vec<Relationship>| {
                DiscoveredPackage::package(
                    Some(name.to_string()),
                    PackageJson::default(),
                    self.root.join_components(
                        &directory.split('/').chain(["go.mod"]).collect::<Vec<_>>(),
                    ),
                )
                .with_native_relationships(relationships)
            };
            let packages = vec![
                package(
                    "api",
                    "apps/api",
                    vec![Relationship::internal("lib", DependencyKind::Production)],
                ),
                package("lib", "packages/lib", Vec::new()),
                package("service/v2", "apps/service", Vec::new()),
                package("independent", "tools/independent", Vec::new()),
                DiscoveredPackage::aggregate(
                    "go-workspace".to_string(),
                    PackageJson::default(),
                    self.root.join_component("go.work"),
                )
                .with_native_relationships(
                    ["api", "independent", "lib", "service/v2"]
                        .into_iter()
                        .map(|name| Relationship::internal(name, DependencyKind::Production))
                        .collect(),
                ),
            ];
            Ok(DiscoveredPackages::new(
                packages,
                vec![WorkspaceRoot::new("go", self.root.clone())],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            let output = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                output.packages(),
                output.workspace_roots(),
            ))
        })
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn go_graph(root: &AbsoluteSystemPath) -> PackageGraph {
    runtime()
        .block_on(
            PackageGraph::builder_optional(root, None)
                .with_package_jsons(Some(HashMap::new()))
                .with_contributor(Arc::new(GoFixtureContributor {
                    root: root.to_owned(),
                }))
                .build(),
        )
        .unwrap()
}

/// A JavaScript package that shares a directory with the `lib` Go module.
fn colocated_graph(root: &AbsoluteSystemPath) -> PackageGraph {
    let javascript = root.join_components(&["packages", "lib", "package.json"]);
    let response = turborepo_repository::discovery::DiscoveryResponse {
        package_manager: PackageManager::Npm,
        workspaces: vec![
            turborepo_repository::discovery::WorkspaceData::new(javascript.clone(), None).unwrap(),
        ],
    };
    let manifests = HashMap::from([(
        javascript,
        PackageJson {
            name: Some(turborepo_errors::Spanned::new("@repo/lib".to_string())),
            ..PackageJson::default()
        },
    )]);
    runtime()
        .block_on(
            PackageGraph::builder(root, PackageJson::default())
                .with_package_discovery(move || {
                    let response = response.clone();
                    async move { Ok(response) }
                })
                .with_package_json_loader(move |path: &AbsoluteSystemPath| {
                    manifests.get(path).cloned().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "missing fixture manifest",
                        )
                        .into()
                    })
                })
                .without_external_dependencies()
                .with_contributor(Arc::new(GoFixtureContributor {
                    root: root.to_owned(),
                }))
                .build(),
        )
        .unwrap()
}

/// Reports a fixed change set for the `HEAD` affected range.
struct HeadChanges(Vec<&'static str>);

impl GitChangeDetector for HeadChanges {
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        _to_ref: Option<&str>,
        _include_uncommitted: bool,
        _allow_unknown_objects: bool,
        _merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        assert_eq!(
            from_ref,
            Some("HEAD"),
            "the affected range must reach the change detector"
        );
        Ok(self
            .0
            .iter()
            .map(|name| {
                (
                    PackageName::from(*name),
                    PackageInclusionReason::IncludedByFilter {
                        filters: Vec::new(),
                    },
                )
            })
            .collect())
    }
}

fn fixture() -> (TempDir, AbsoluteSystemPathBuf) {
    let tempdir = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
    std::fs::create_dir_all(root.join_components(&["packages", "lib"]).as_std_path()).unwrap();
    (tempdir, root)
}

fn filter_resolver<'a>(
    graph: &'a PackageGraph,
    root: &'a AbsoluteSystemPath,
    changes: HeadChanges,
) -> FilterResolver<'a, HeadChanges> {
    FilterResolver::new_with_change_detector(graph, root, None, changes)
}

fn names(
    packages: HashMap<PackageName, PackageInclusionReason>,
) -> std::collections::BTreeSet<String> {
    packages.into_keys().map(|name| name.to_string()).collect()
}

#[test]
fn go_scope_identities_and_directories_are_authoritative() {
    let (_tempdir, root) = fixture();
    let graph = go_graph(&root);

    let directories = graph
        .package_scope_directories()
        .map(|(name, directory)| (name.to_string(), directory.to_unix().to_string()))
        .collect::<HashMap<_, _>>();
    assert_eq!(directories["api"], "apps/api");
    assert_eq!(directories["lib"], "packages/lib");
    assert_eq!(directories["independent"], "tools/independent");
    assert_eq!(
        directories["service/v2"], "apps/service",
        "a trailing major version stays part of the Go identity"
    );
    assert_eq!(
        directories["go-workspace"], "",
        "the aggregate is an execution scope at the repository root"
    );
}

#[test]
fn filter_by_name_selects_only_the_go_module() {
    let (_tempdir, root) = fixture();
    let graph = go_graph(&root);
    let resolver = filter_resolver(&graph, &root, HeadChanges(Vec::new()));

    let (selected, _mode) = resolver.resolve(&None, &["lib".to_string()]).unwrap();

    assert_eq!(
        names(selected),
        ["lib".to_string()].into_iter().collect(),
        "a Go module name selects exactly that module"
    );
}

#[test]
fn filter_by_directory_selects_colocated_scopes() {
    let (_tempdir, root) = fixture();
    let graph = colocated_graph(&root);
    let resolver = filter_resolver(&graph, &root, HeadChanges(Vec::new()));

    let (selected, _mode) = resolver
        .resolve(&None, &["./packages/lib".to_string()])
        .unwrap();

    assert_eq!(
        names(selected),
        ["@repo/lib".to_string(), "lib".to_string()]
            .into_iter()
            .collect(),
        "one directory owns both co-located scopes"
    );
}

#[test]
fn colocated_cwd_inference_selects_javascript_and_go_scopes() {
    let (_tempdir, root) = fixture();
    let graph = colocated_graph(&root);
    let directory = AnchoredSystemPathBuf::try_from("packages/lib").unwrap();
    let inference = Some(PackageInference::calculate(&root, &directory, &graph));
    let resolver =
        FilterResolver::new_with_change_detector(&graph, &root, inference, HeadChanges(Vec::new()));

    let (selected, _mode) = resolver.resolve(&None, &[]).unwrap();

    assert_eq!(
        names(selected),
        ["@repo/lib".to_string(), "lib".to_string()]
            .into_iter()
            .collect(),
        "a cwd inside a co-located directory selects every scope there"
    );
}

#[test]
fn affected_go_modules_include_native_dependents() {
    let (_tempdir, root) = fixture();
    let graph = go_graph(&root);
    let resolver = filter_resolver(&graph, &root, HeadChanges(vec!["lib"]));

    let (affected, _mode) = resolver
        .resolve(&Some((Some("HEAD".to_string()), None)), &[])
        .unwrap();

    let affected = names(affected);
    assert!(affected.contains("lib"), "{affected:?}");
    assert!(
        affected.contains("api"),
        "an internal Go dependency edge must make dependents affected: {affected:?}"
    );
    assert!(
        affected.contains("go-workspace"),
        "the aggregate is affected by any module it aggregates: {affected:?}"
    );
    assert!(
        !affected.contains("independent"),
        "unrelated modules must not be affected: {affected:?}"
    );
}

#[test]
fn affected_go_modules_do_not_cross_independent_modules() {
    let (_tempdir, root) = fixture();
    let graph = go_graph(&root);
    let resolver = filter_resolver(&graph, &root, HeadChanges(vec!["independent"]));

    let (affected, _mode) = resolver
        .resolve(&Some((Some("HEAD".to_string()), None)), &[])
        .unwrap();

    let affected = names(affected);
    assert!(affected.contains("independent"), "{affected:?}");
    for unrelated in ["api", "lib", "service/v2"] {
        assert!(
            !affected.contains(unrelated),
            "{unrelated} must not be affected by an independent module: {affected:?}"
        );
    }
}
