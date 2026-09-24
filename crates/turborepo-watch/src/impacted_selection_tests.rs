use std::{collections::HashSet, fs, sync::Arc};

use serde_json::json;
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_daemon::PackageChangeEvent;
use turborepo_engine::Building;
use turborepo_repository::{
    discovery::{DiscoveryResponse, WorkspaceData},
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    package_manager::PackageManager,
    toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor, ToolchainId,
        WorkspaceRoot,
    },
};
use turborepo_task_filter::Engine;
use turborepo_task_id::TaskId;
use turborepo_types::{TaskDefinition, TaskInputs};

use super::{ChangedPackages, WatchClient, WatchTaskSelection, package_graph_invalidated};

struct NativeContributor {
    root: AbsoluteSystemPathBuf,
    toolchain: ToolchainId,
    package: &'static str,
    manifest: &'static str,
}

impl RepositoryContributor for NativeContributor {
    fn id(&self) -> ToolchainId {
        self.toolchain.clone()
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let manifest = self
                .root
                .join_components(&self.manifest.split('/').collect::<Vec<_>>());
            Ok(DiscoveredPackages::new(
                vec![DiscoveredPackage::package(
                    Some(self.package.into()),
                    PackageJson::default(),
                    manifest,
                )],
                vec![WorkspaceRoot::new(
                    self.toolchain.as_str(),
                    self.root.clone(),
                )],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            let packages = self.discover_packages().await?;
            Ok(DiscoveredPackageScopes::from_full_observation(
                packages.packages(),
                packages.workspace_roots(),
            ))
        })
    }
}

fn changed(paths: &[&str]) -> HashSet<AnchoredSystemPathBuf> {
    paths
        .iter()
        .map(|path| AnchoredSystemPathBuf::from_raw(path).unwrap())
        .collect()
}

fn names(packages: &HashSet<PackageName>) -> HashSet<String> {
    packages.iter().map(|package| package.to_string()).collect()
}

#[tokio::test]
async fn mixed_toolchain_watch_selects_impacted_tasks_and_partial_reruns_without_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    for path in [
        "packages/web/src/main.ts",
        "packages/web/dev.config",
        "apps/go/src/main.go",
        "crates/cargo/src/main.rs",
        "packages/py/src/main.py",
        "turbo.json",
    ] {
        let path = root.join_components(&path.split('/').collect::<Vec<_>>());
        fs::create_dir_all(path.as_path().parent().unwrap()).unwrap();
        fs::write(path, "source").unwrap();
    }

    let web_manifest = root.join_components(&["packages", "web", "package.json"]);
    let discovery = DiscoveryResponse {
        package_manager: PackageManager::Npm,
        workspaces: vec![WorkspaceData::new(web_manifest.clone(), None).unwrap()],
    };
    let mut builder = PackageGraph::builder(&root, PackageJson::default())
        .with_package_discovery(move || {
            let discovery = discovery.clone();
            async move { Ok(discovery) }
        })
        .with_package_json_loader(move |path: &turbopath::AbsoluteSystemPath| {
            (path == web_manifest.as_ref())
                .then(|| PackageJson::from_value(json!({"name": "web"})).unwrap())
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "unknown manifest").into()
                })
        });
    for (toolchain, package, manifest) in [
        (ToolchainId::GO, "go-app", "apps/go/go.mod"),
        (ToolchainId::RUST, "cargo-app", "crates/cargo/Cargo.toml"),
        (ToolchainId::PYTHON, "py-app", "packages/py/pyproject.toml"),
    ] {
        builder = builder.with_contributor(Arc::new(NativeContributor {
            root: root.clone(),
            toolchain: toolchain.clone(),
            package,
            manifest,
        }));
    }
    let graph = builder
        .without_external_dependencies()
        .build()
        .await
        .unwrap();
    for (package, toolchain) in [
        ("web", ToolchainId::JAVASCRIPT),
        ("go-app", ToolchainId::GO),
        ("cargo-app", ToolchainId::RUST),
        ("py-app", ToolchainId::PYTHON),
    ] {
        assert_eq!(
            graph
                .package_task_context(&PackageName::from(package))
                .unwrap()
                .toolchain(),
            Some(&toolchain)
        );
    }

    let web = TaskId::new("web", "build").into_owned();
    let dev = TaskId::new("web", "dev").into_owned();
    let go = TaskId::new("go-app", "build").into_owned();
    let cargo = TaskId::new("cargo-app", "build").into_owned();
    let py = TaskId::new("py-app", "build").into_owned();
    let input = |pattern: &str| TaskDefinition {
        inputs: TaskInputs::new(vec![pattern.into()]),
        ..Default::default()
    };
    let mut engine: Engine<Building> = Engine::new();
    for (id, definition) in [
        (web.clone(), input("src/main.ts")),
        (
            dev.clone(),
            TaskDefinition {
                persistent: true,
                interruptible: false,
                ..input("dev.config")
            },
        ),
        (go.clone(), input("src/main.go")),
        (cargo.clone(), input("src/main.rs")),
        (py.clone(), input("src/main.py")),
    ] {
        engine.get_index(&id);
        engine.add_definition(id, definition);
    }
    for (dependent, dependency) in [(&web, &go), (&py, &cargo)] {
        let dependent = engine.get_index(dependent);
        let dependency = engine.get_index(dependency);
        engine.task_graph_mut().add_edge(dependent, dependency, ());
    }
    let engine = engine.seal();
    let packages = |names: &[&str]| names.iter().map(|name| PackageName::from(*name)).collect();
    let task_input_selection = WatchTaskSelection {
        engine: &engine,
        graph: &graph,
        repo_root: &root,
        global_deps: &[],
        task_inputs: true,
    };
    for (changed_package, file, expected_packages, expected_tasks) in [
        (
            "go-app",
            "apps/go/src/main.go",
            vec!["go-app", "web"],
            vec![go.clone(), web.clone()],
        ),
        (
            "cargo-app",
            "crates/cargo/src/main.rs",
            vec!["cargo-app", "py-app"],
            vec![cargo.clone(), py.clone()],
        ),
        (
            "py-app",
            "packages/py/src/main.py",
            vec!["py-app", "cargo-app"],
            vec![py.clone(), cargo.clone()],
        ),
        (
            "web",
            "packages/web/src/main.ts",
            vec!["web", "go-app"],
            vec![web.clone(), go.clone()],
        ),
    ] {
        let selected =
            task_input_selection.impacted_by(&packages(&[changed_package]), &changed(&[file]));
        assert_eq!(
            names(&selected.packages),
            expected_packages.into_iter().map(str::to_string).collect(),
            "{file}"
        );
        assert_eq!(
            selected.stoppable_ids.into_iter().collect::<HashSet<_>>(),
            expected_tasks.into_iter().collect(),
            "{file}"
        );
    }

    // A persistent, non-interruptible dev task cannot restart on input changes.
    let dev_only = task_input_selection
        .impacted_by(&packages(&["web"]), &changed(&["packages/web/dev.config"]));
    assert!(dev_only.packages.is_empty());
    assert!(dev_only.stoppable_ids.is_empty());
    let missing = task_input_selection.impacted_by(
        &packages(&["go-app"]),
        &changed(&["apps/go/src/deleted.go"]),
    );
    assert!(
        missing.packages.is_empty(),
        "deleted files do not match existing task inputs"
    );

    // A package-aware run (and the task-input fallback without file paths)
    // restarts only the affected task graph's package owners.
    let package_selection = WatchTaskSelection {
        task_inputs: false,
        ..task_input_selection
    };
    let selected =
        package_selection.impacted_by(&packages(&["go-app"]), &changed(&["apps/go/src/main.go"]));
    assert_eq!(
        names(&selected.packages),
        HashSet::from(["go-app".into(), "web".into()])
    );
    assert_eq!(
        selected.stoppable_ids.into_iter().collect::<HashSet<_>>(),
        HashSet::from([go.clone(), web.clone()])
    );
    let fallback = task_input_selection.impacted_by(&packages(&["go-app"]), &HashSet::new());
    assert_eq!(
        names(&fallback.packages),
        HashSet::from(["go-app".into(), "web".into()])
    );
    let selected = package_selection.impacted_by(&packages(&["web"]), &HashSet::new());
    assert_eq!(names(&selected.packages), HashSet::from(["web".into()]));
    assert_eq!(
        selected.stoppable_ids.into_iter().collect::<HashSet<_>>(),
        HashSet::from([web])
    );

    let mut partial = ChangedPackages::Some {
        packages: fallback.packages,
        changed_files: changed(&["apps/go/src/main.go"]),
    };
    partial.filter_to_watched(&packages(&["go-app"]));
    assert!(
        matches!(partial, ChangedPackages::Some { ref packages, .. } if names(packages) == HashSet::from(["go-app".into()]))
    );

    // A root task configuration change affects all restartable domains; a
    // Rediscover event supersedes pending partial work and bypasses the watched
    // package filter, ready for RunBuilder's full-graph path.
    let all = task_input_selection.impacted_by(&packages(&["web"]), &changed(&["turbo.json"]));
    assert_eq!(
        names(&all.packages),
        HashSet::from([
            "web".into(),
            "go-app".into(),
            "cargo-app".into(),
            "py-app".into()
        ])
    );
    assert!(!all.stoppable_ids.contains(&dev));
    let pending = std::sync::Mutex::new(partial);
    WatchClient::handle_change_event(&pending, PackageChangeEvent::Rediscover);
    let mut rediscovered = WatchClient::take_pending_changes(&pending).unwrap();
    rediscovered.filter_to_watched(&packages(&["go-app"]));
    assert!(matches!(rediscovered, ChangedPackages::All));
    assert!(package_graph_invalidated(
        &changed(&["crates/cargo/Cargo.toml"]),
        graph.package_manager().map(|pm| pm.lockfile_name()),
        &turborepo_repository::toolchain::WatchSpec {
            definition_file_names: vec![
                "Cargo.toml".into(),
                "go.mod".into(),
                "pyproject.toml".into()
            ],
            ..Default::default()
        },
    ));
}
