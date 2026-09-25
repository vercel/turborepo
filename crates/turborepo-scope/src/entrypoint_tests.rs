use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    discovery::{DiscoveryResponse, WorkspaceData},
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
use turborepo_types::{FilterMode, ScopeOpts};

use crate::{GitChangeDetector, ResolutionError, resolve_packages_with_change_detector};

type Call = (Option<String>, Option<String>, bool, bool, bool);

struct RecordingDetector {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl GitChangeDetector for RecordingDetector {
    fn changed_packages(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        self.calls.lock().unwrap().push((
            from.map(str::to_string),
            to.map(str::to_string),
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        ));
        Ok(HashMap::from([(
            PackageName::from("lib"),
            PackageInclusionReason::FileChanged {
                file: AnchoredSystemPathBuf::from_raw("packages/lib/src/index.ts").unwrap(),
            },
        )]))
    }
}

async fn injected_graph() -> (tempfile::TempDir, AbsoluteSystemPathBuf, PackageGraph) {
    let tmp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let app = root.join_components(&["packages", "app", "package.json"]);
    let lib = root.join_components(&["packages", "lib", "package.json"]);
    let response = DiscoveryResponse {
        package_manager: PackageManager::Npm,
        workspaces: [app.clone(), lib.clone()]
            .into_iter()
            .map(|path| WorkspaceData::new(path, None).unwrap())
            .collect(),
    };
    let manifests = HashMap::from([
        (
            app,
            PackageJson {
                name: Some(Spanned::new("app".to_string())),
                dependencies: Some(BTreeMap::from([("lib".to_string(), "*".to_string())])),
                ..Default::default()
            },
        ),
        (
            lib,
            PackageJson {
                name: Some(Spanned::new("lib".to_string())),
                ..Default::default()
            },
        ),
    ]);
    let graph = PackageGraph::builder(&root, PackageJson::default())
        .with_package_discovery(move || {
            let response = response.clone();
            async move { Ok(response) }
        })
        .with_package_json_loader(move |path: &AbsoluteSystemPath| {
            manifests.get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "missing in-memory manifest")
                    .into()
            })
        })
        .without_external_dependencies()
        .build()
        .await
        .unwrap();
    (tmp, root, graph)
}

#[tokio::test]
async fn injected_scope_entrypoint_resolves_affected_dependents_and_exclusions() {
    let (_tmp, root, graph) = injected_graph().await;
    assert_eq!(
        graph
            .filtering_relationships()
            .transitive_dependencies(&PackageName::from("app"))
            .unwrap(),
        [PackageName::from("lib")],
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let opts = ScopeOpts {
        affected_range: Some((Some("base".to_string()), Some("HEAD".to_string()))),
        ..Default::default()
    };
    let (packages, mode) = resolve_packages_with_change_detector(
        &opts,
        &root,
        &graph,
        RecordingDetector {
            calls: calls.clone(),
        },
    )
    .unwrap();
    assert_eq!(mode, FilterMode::ExplicitSelection);
    assert_eq!(
        packages.into_keys().collect::<HashSet<_>>(),
        [PackageName::from("app"), PackageName::from("lib")].into(),
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [(
            Some("base".to_string()),
            Some("HEAD".to_string()),
            true,
            true,
            true,
        )]
    );

    let opts = ScopeOpts {
        filter_patterns: vec!["!lib".to_string()],
        ..Default::default()
    };
    let (packages, mode) = resolve_packages_with_change_detector(
        &opts,
        &root,
        &graph,
        RecordingDetector {
            calls: calls.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        mode,
        FilterMode::ExcludeOnly {
            root_excluded: false
        }
    );
    assert_eq!(
        packages.into_keys().collect::<HashSet<_>>(),
        [PackageName::from("app")].into(),
    );
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "no Git call for exclusion-only selection"
    );
}

struct MatrixContributor {
    root: AbsoluteSystemPathBuf,
    toolchain: ToolchainId,
}

impl RepositoryContributor for MatrixContributor {
    fn id(&self) -> ToolchainId {
        self.toolchain.clone()
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let (app, lib, aggregate, app_manifest, lib_manifest, root_manifest) =
                match self.toolchain.as_str() {
                    "rust" => (
                        "cargo-app",
                        "cargo-lib",
                        "cargo-workspace",
                        "crates/app/Cargo.toml",
                        "crates/lib/Cargo.toml",
                        "Cargo.toml",
                    ),
                    "go" => (
                        "go-app",
                        "go-lib",
                        "go-workspace",
                        "apps/go/go.mod",
                        "packages/go/go.mod",
                        "go.work",
                    ),
                    "python" => (
                        "py-app",
                        "py-lib",
                        "py-workspace",
                        "packages/py-app/pyproject.toml",
                        "packages/py-lib/pyproject.toml",
                        "pyproject.toml",
                    ),
                    _ => unreachable!("fixture includes native toolchains only"),
                };
            let path = |manifest: &str| {
                self.root
                    .join_components(&manifest.split('/').collect::<Vec<_>>())
            };
            Ok(DiscoveredPackages::new(
                vec![
                    DiscoveredPackage::package(
                        Some(app.into()),
                        PackageJson::default(),
                        path(app_manifest),
                    )
                    .with_native_relationships(vec![Relationship::internal(
                        lib,
                        DependencyKind::Production,
                    )]),
                    DiscoveredPackage::package(
                        Some(lib.into()),
                        PackageJson::default(),
                        path(lib_manifest),
                    )
                    .with_native_relationships(Vec::new()),
                    DiscoveredPackage::aggregate(
                        aggregate.into(),
                        PackageJson::default(),
                        path(root_manifest),
                    )
                    .with_native_relationships(
                        [app, lib]
                            .into_iter()
                            .map(|name| Relationship::internal(name, DependencyKind::Production))
                            .collect(),
                    ),
                ],
                vec![WorkspaceRoot::new(
                    self.toolchain.as_str(),
                    self.root.clone(),
                )],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            let observed = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                observed.packages(),
                observed.workspace_roots(),
            ))
        })
    }
}

async fn injected_mixed_graph() -> (tempfile::TempDir, AbsoluteSystemPathBuf, PackageGraph) {
    let tmp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let web = root.join_components(&["packages", "web", "package.json"]);
    let shared = root.join_components(&["packages", "shared", "package.json"]);
    let discovery = DiscoveryResponse {
        package_manager: PackageManager::Npm,
        workspaces: [web.clone(), shared.clone()]
            .into_iter()
            .map(|path| WorkspaceData::new(path, None).unwrap())
            .collect(),
    };
    let manifests = HashMap::from([
        (
            web,
            PackageJson::from_value(serde_json::json!({
                "name": "web", "dependencies": {"shared": "*"}
            }))
            .unwrap(),
        ),
        (
            shared,
            PackageJson::from_value(serde_json::json!({
                "name": "shared"
            }))
            .unwrap(),
        ),
    ]);
    let mut graph = PackageGraph::builder(
        &root,
        PackageJson::from_value(serde_json::json!({"name": "root"})).unwrap(),
    )
    .with_package_discovery(move || {
        let discovery = discovery.clone();
        async move { Ok(discovery) }
    })
    .with_package_jsons(Some(manifests));
    for toolchain in [ToolchainId::GO, ToolchainId::RUST, ToolchainId::PYTHON] {
        graph = graph.with_contributor(Arc::new(MatrixContributor {
            root: root.clone(),
            toolchain,
        }));
    }
    let graph = graph.without_external_dependencies().build().await.unwrap();
    (tmp, root, graph)
}

struct MatrixDetector {
    changed: Vec<PackageName>,
    fail: bool,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl GitChangeDetector for MatrixDetector {
    fn changed_packages(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        self.calls.lock().unwrap().push((
            from.map(str::to_string),
            to.map(str::to_string),
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        ));
        if self.fail {
            return Err(ResolutionError::Scm(turborepo_scm::Error::GitVersion(
                "detector-failure".into(),
            )));
        }
        Ok(self
            .changed
            .iter()
            .cloned()
            .map(|name| {
                (
                    name,
                    PackageInclusionReason::FileChanged {
                        file: AnchoredSystemPathBuf::from_raw("changed/file.txt").unwrap(),
                    },
                )
            })
            .collect())
    }
}

#[tokio::test]
async fn mixed_toolchains_follow_shared_filter_and_affected_contracts() {
    let (_tmp, root, graph) = injected_mixed_graph().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resolve = |opts: ScopeOpts, changed: &[&str], fail: bool| {
        let (selected, mode) = resolve_packages_with_change_detector(
            &opts,
            &root,
            &graph,
            MatrixDetector {
                changed: changed
                    .iter()
                    .map(|name| PackageName::from(*name))
                    .collect(),
                fail,
                calls: calls.clone(),
            },
        )?;
        Ok::<_, ResolutionError>((selected.into_keys().collect::<HashSet<_>>(), mode))
    };
    let (all, mode) = resolve(ScopeOpts::default(), &[], false).unwrap();
    assert_eq!(mode, FilterMode::AllPackages);
    for name in [
        "web",
        "shared",
        "go-app",
        "go-lib",
        "go-workspace",
        "cargo-app",
        "cargo-lib",
        "cargo-workspace",
        "py-app",
        "py-lib",
        "py-workspace",
    ] {
        assert!(all.contains(&PackageName::from(name)), "{name} missing");
    }
    assert_eq!(all.len(), 11);
    assert!(calls.lock().unwrap().is_empty());

    for (pattern, expected) in [
        ("web...", vec!["web", "shared"]),
        ("go-app...", vec!["go-app", "go-lib"]),
        ("cargo-app...", vec!["cargo-app", "cargo-lib"]),
        ("py-app...", vec!["py-app", "py-lib"]),
    ] {
        let (selected, mode) = resolve(
            ScopeOpts {
                filter_patterns: vec![pattern.into()],
                ..Default::default()
            },
            &[],
            false,
        )
        .unwrap();
        assert_eq!(mode, FilterMode::ExplicitSelection);
        assert_eq!(
            selected,
            expected.into_iter().map(PackageName::from).collect(),
            "{pattern}"
        );
    }
    // A plain package selector does not expand package dependencies. Task
    // execution's `--only` switch is applied downstream of this scope API.
    for package in ["web", "go-app", "cargo-app", "py-app"] {
        let (selected, mode) = resolve(
            ScopeOpts {
                filter_patterns: vec![package.into()],
                ..Default::default()
            },
            &[],
            false,
        )
        .unwrap();
        assert_eq!(mode, FilterMode::ExplicitSelection);
        assert_eq!(selected, [PackageName::from(package)].into());
    }
    let (selected, mode) = resolve(
        ScopeOpts {
            filter_patterns: vec!["!go-app".into(), "!py-app".into()],
            ..Default::default()
        },
        &[],
        false,
    )
    .unwrap();
    assert_eq!(
        mode,
        FilterMode::ExcludeOnly {
            root_excluded: false
        }
    );
    assert_eq!(selected.len(), 9);
    assert!(!selected.contains(&PackageName::from("go-app")));
    assert!(!selected.contains(&PackageName::from("py-app")));

    let (cargo_selected, cargo_mode) = resolve(
        ScopeOpts {
            filter_patterns: vec!["!cargo-lib".into()],
            ..Default::default()
        },
        &[],
        false,
    )
    .unwrap();
    assert_eq!(
        cargo_mode,
        FilterMode::ExcludeOnly {
            root_excluded: false
        }
    );
    assert_eq!(cargo_selected.len(), 10);
    assert!(cargo_selected.contains(&PackageName::from("cargo-app")));
    assert!(!cargo_selected.contains(&PackageName::from("cargo-lib")));
    assert!(cargo_selected.contains(&PackageName::from("cargo-workspace")));

    let (_, mode) = resolve(
        ScopeOpts {
            filter_patterns: vec!["!//".into()],
            ..Default::default()
        },
        &[],
        false,
    )
    .unwrap();
    assert_eq!(
        mode,
        FilterMode::ExcludeOnly {
            root_excluded: true
        }
    );
    assert!(calls.lock().unwrap().is_empty());

    let opts = ScopeOpts {
        affected_range: Some((Some("missing-base".into()), Some("HEAD".into()))),
        filter_patterns: vec!["go-*".into(), "!go-app".into()],
        ..Default::default()
    };
    let (selected, mode) = resolve(opts.clone(), &["go-lib"], false).unwrap();
    assert_eq!(mode, FilterMode::ExplicitSelection);
    assert_eq!(
        selected,
        ["go-lib", "go-workspace"].map(PackageName::from).into()
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [(
            Some("missing-base".into()),
            Some("HEAD".into()),
            true,
            true,
            true,
        )]
    );
    calls.lock().unwrap().clear();
    let affected = ScopeOpts {
        affected_range: Some((Some("base".into()), Some("HEAD".into()))),
        ..Default::default()
    };
    for (changed, expected) in [
        ("shared", vec!["shared", "web"]),
        ("go-lib", vec!["go-lib", "go-app", "go-workspace"]),
        (
            "cargo-lib",
            vec!["cargo-lib", "cargo-app", "cargo-workspace"],
        ),
        ("py-lib", vec!["py-lib", "py-app", "py-workspace"]),
    ] {
        let (selected, mode) = resolve(affected.clone(), &[changed], false).unwrap();
        assert_eq!(mode, FilterMode::ExplicitSelection);
        assert_eq!(
            selected,
            expected.into_iter().map(PackageName::from).collect(),
            "affected {changed}"
        );
    }
    assert_eq!(calls.lock().unwrap().len(), 4);
    assert!(calls.lock().unwrap().iter().all(|call| call.3));
    calls.lock().unwrap().clear();
    // Production maps a missing Git ref to all packages changed before this
    // boundary. Supply that detector result and check the filter still applies.
    let all_changed = all.iter().map(PackageName::as_str).collect::<Vec<_>>();
    let (selected, mode) = resolve(opts.clone(), &all_changed, false).unwrap();
    assert_eq!(mode, FilterMode::ExplicitSelection);
    assert_eq!(
        selected,
        ["go-lib", "go-workspace"].map(PackageName::from).into()
    );
    let error = resolve(opts, &[], true).unwrap_err();
    assert!(matches!(
        error,
        ResolutionError::Scm(turborepo_scm::Error::GitVersion(version))
            if version == "detector-failure"
    ));
    assert_eq!(calls.lock().unwrap().len(), 2);
    let error = resolve(
        ScopeOpts {
            filter_patterns: vec!["missing-package".into()],
            ..Default::default()
        },
        &[],
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ResolutionError::NoPackagesMatchedWithName(name) if name == "missing-package"
    ));
}
