//! Generic lazy-loading tests: engine construction over a graph that still
//! carries inventory-only scopes. The fake contributor uses only the open
//! `ToolchainId` — no language is named.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::json;
use tempfile::TempDir;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::Spanned;
use turborepo_repository::{
    native_tasks::{
        NativeCommandArguments, NativeCommandProgram, NativeTask, WorkingDirectoryPolicy,
    },
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    relationships::{DependencyKind, Relationship},
    toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackage,
        DiscoveredPackageScope, DiscoveredPackageScopes, DiscoveredPackages, RepositoryContributor,
        ToolchainId, WorkspaceRoot,
    },
};
use turborepo_task_id::{TaskId, TaskName};

use super::{MockDiscovery, MockLockfile, TestTurboJsonLoader, turbo_json};
use crate::{BuilderError, EngineBuilder, MissingTaskError, TaskNode};

/// A native contributor that is inventory-only until loaded: its inventory
/// names one scope; full discovery contributes a real `check` command task.
/// Open toolchain id — no language is named.
struct LazyNativeContributor {
    repo_root: AbsoluteSystemPathBuf,
    full_calls: Arc<AtomicUsize>,
}

impl LazyNativeContributor {
    fn new(repo_root: &AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root: repo_root.clone(),
            full_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl RepositoryContributor for LazyNativeContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::new("lazy-native")
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        self.full_calls.fetch_add(1, Ordering::SeqCst);
        let root = self.repo_root.clone();
        Box::pin(async move {
            let package = DiscoveredPackage::package(
                Some("native".to_string()),
                PackageJson::default(),
                root.join_components(&["native", "manifest"]),
            )
            .with_native_relationships(Vec::new())
            .with_native_tasks(vec![NativeTask::command_task(
                "check",
                "native check".to_string(),
                NativeCommandProgram::Tool("native".to_string()),
                NativeCommandArguments::new(vec!["check".to_string()]),
                None,
                WorkingDirectoryPolicy::PackageDirectory,
            )]);
            Ok(DiscoveredPackages::new(
                vec![package],
                vec![WorkspaceRoot::new("lazy-native", root)],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        let root = self.repo_root.clone();
        Box::pin(async move {
            Ok(DiscoveredPackageScopes::new(
                vec![DiscoveredPackageScope::new(
                    Some("native".to_string()),
                    root.join_components(&["native", "manifest"]),
                )],
                vec![WorkspaceRoot::new("lazy-native", root)],
            ))
        })
    }
}

/// A package.json scope with an explicitly contributed cross-language edge.
/// Loading this contributor must not load the target's native task catalogue.
struct DeclaredDependencyContributor {
    repo_root: AbsoluteSystemPathBuf,
    name: &'static str,
}

impl DeclaredDependencyContributor {
    fn manifest_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root
            .join_components(&["packages", self.name, "package.json"])
    }
}

impl RepositoryContributor for DeclaredDependencyContributor {
    fn id(&self) -> ToolchainId {
        ToolchainId::new("declared-relationships")
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            Ok(DiscoveredPackages::new(
                vec![
                    DiscoveredPackage::package(
                        Some(self.name.to_string()),
                        PackageJson::default(),
                        self.manifest_path(),
                    )
                    .with_native_relationships(vec![Relationship::internal(
                        "native",
                        DependencyKind::Production,
                    )]),
                ],
                vec![WorkspaceRoot::new(
                    "declared-relationships",
                    self.repo_root.clone(),
                )],
            ))
        })
    }

    fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
        Box::pin(async move {
            Ok(DiscoveredPackageScopes::new(
                vec![DiscoveredPackageScope::new(
                    Some(self.name.to_string()),
                    self.manifest_path(),
                )],
                vec![WorkspaceRoot::new(
                    "declared-relationships",
                    self.repo_root.clone(),
                )],
            ))
        })
    }
}

fn web_package_json(dependencies: &[(&str, &str)]) -> PackageJson {
    PackageJson {
        name: Some(Spanned::new("web".to_string())),
        dependencies: Some(
            dependencies
                .iter()
                .map(|(name, version)| (name.to_string(), version.to_string()))
                .collect(),
        ),
        ..Default::default()
    }
}

fn web_package_jsons(
    repo_root: &AbsoluteSystemPathBuf,
    web: PackageJson,
) -> HashMap<AbsoluteSystemPathBuf, PackageJson> {
    HashMap::from([(
        repo_root.join_components(&["packages", "web", "package.json"]),
        web,
    )])
}

fn current_thread_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The inventory graph: authoritative JavaScript (`root` + `web`) plus the
/// native contributor's inventory-only scope.
fn inventory_graph(
    repo_root: &AbsoluteSystemPathBuf,
    web: PackageJson,
    contributor: Arc<LazyNativeContributor>,
) -> PackageGraph {
    let runtime = current_thread_runtime();
    let lazy = runtime
        .block_on(
            PackageGraph::builder(repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(web_package_jsons(repo_root, web)))
                .with_contributor(contributor)
                .build_lazy(),
        )
        .unwrap();
    let (graph, _plan) = lazy.into_parts();
    Arc::try_unwrap(graph).expect("freshly built graph is uniquely owned")
}

fn cross_dep_loader() -> TestTurboJsonLoader {
    TestTurboJsonLoader::new(HashMap::from([
        (
            PackageName::Root,
            turbo_json(json!({ "tasks": { "build": {} } })),
        ),
        (
            PackageName::from("web"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": { "build": { "dependsOn": ["native#check"] } }
            })),
        ),
    ]))
}

fn plain_loader() -> TestTurboJsonLoader {
    TestTurboJsonLoader::new(HashMap::from([
        (
            PackageName::Root,
            turbo_json(json!({ "tasks": { "build": {} } })),
        ),
        (
            PackageName::from("web"),
            turbo_json(json!({ "extends": ["//"], "tasks": {} })),
        ),
    ]))
}

fn web_build_builder<'a>(
    repo_root: &'a AbsoluteSystemPathBuf,
    package_graph: &'a PackageGraph,
    loader: &'a TestTurboJsonLoader,
    task: &'static str,
) -> EngineBuilder<'a, TestTurboJsonLoader> {
    EngineBuilder::new(repo_root, package_graph, loader, false)
        .with_workspaces(vec![PackageName::from("web")])
        .with_tasks(Some(Spanned::new(TaskName::from(task))))
}

fn missing_task_names(error: &BuilderError) -> Vec<String> {
    match error {
        BuilderError::MissingTasks(errors) => errors
            .iter()
            .filter_map(|error| match error {
                MissingTaskError::MissingTaskDefinition { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect(),
        other => panic!("expected missing tasks, got {other:?}"),
    }
}

#[test]
fn cross_package_task_dependency_demands_only_the_reached_owner() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let contributor = Arc::new(LazyNativeContributor::new(&repo_root));
    let package_graph = inventory_graph(&repo_root, web_package_json(&[]), contributor);
    assert_eq!(
        package_graph.unloaded_scope_owner(&PackageName::from("native")),
        Some(&ToolchainId::new("lazy-native"))
    );

    let loader = cross_dep_loader();
    let (engine, demands) = web_build_builder(&repo_root, &package_graph, &loader, "build")
        .build_with_unloaded_demands()
        .unwrap();
    assert_eq!(
        demands,
        HashSet::from([ToolchainId::new("lazy-native")]),
        "the explicit cross-package dependency demands exactly the reached owner"
    );
    assert!(
        engine
            .task_definition(&TaskId::new("native", "check"))
            .is_none(),
        "the pass never guesses the native task before its owner is loaded"
    );
    assert!(
        engine
            .task_definition(&TaskId::new("web", "build"))
            .is_some()
    );
}

#[test]
fn topological_dependency_through_declared_native_dep_loads_real_metadata() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    // `web` has an explicit cross-language relationship, not an npm dependency
    // on a native manifest. Load that relationship while leaving the target's
    // catalogue inventory-only; `^check` must demand the real native metadata.
    let runtime = current_thread_runtime();
    let contributor = LazyNativeContributor::new(&repo_root);
    let full_calls = contributor.full_calls.clone();
    let (_graph, mut plan) = runtime
        .block_on(
            PackageGraph::builder(&repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(HashMap::new()))
                .with_contributor(Arc::new(DeclaredDependencyContributor {
                    repo_root: repo_root.clone(),
                    name: "web",
                }))
                .with_contributor(Arc::new(contributor))
                .build_lazy(),
        )
        .unwrap()
        .into_parts();
    let graph = runtime
        .block_on(plan.load(&HashSet::from([ToolchainId::new("declared-relationships")])))
        .unwrap();
    assert_eq!(full_calls.load(Ordering::SeqCst), 0);

    let loader = TestTurboJsonLoader::new(HashMap::from([
        (
            PackageName::Root,
            turbo_json(json!({ "tasks": { "build": {} } })),
        ),
        (
            PackageName::from("web"),
            turbo_json(json!({
                "extends": ["//"],
                "tasks": { "build": { "dependsOn": ["^check"] } }
            })),
        ),
    ]));

    let (engine, demands) = web_build_builder(&repo_root, &graph, &loader, "build")
        .build_with_unloaded_demands()
        .unwrap();
    assert_eq!(demands, HashSet::from([ToolchainId::new("lazy-native")]));
    assert!(
        engine
            .task_definition(&TaskId::new("native", "check"))
            .is_none(),
        "the inventory pass must not create a guessed commandless native task"
    );

    // Load the demanded owner and rebuild: the `^check` edge now resolves to
    // the authoritative native task.
    let owners = HashSet::from([ToolchainId::new("lazy-native")]);
    let loaded = runtime.block_on(plan.load(&owners)).unwrap();
    assert_eq!(
        loaded.unloaded_scope_owner(&PackageName::from("native")),
        None
    );
    let (engine, demands) = web_build_builder(&repo_root, &loaded, &loader, "build")
        .build_with_unloaded_demands()
        .unwrap();
    assert!(demands.is_empty(), "loading settles the demand");
    assert!(
        engine
            .task_definition(&TaskId::new("native", "check"))
            .is_some(),
        "after loading, the topological dependency resolves to the real native task"
    );
    assert!(
        engine
            .dependencies(&TaskId::new("web", "build"))
            .is_some_and(|deps| {
                deps.iter()
                    .any(|node| matches!(node, TaskNode::Task(task) if task.task() == "check"))
            }),
        "the dependency edge reaches the native task"
    );
}

#[test]
fn whole_workspace_enumeration_never_demands_unselected_owners() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let contributor = Arc::new(LazyNativeContributor::new(&repo_root));
    let package_graph = inventory_graph(&repo_root, web_package_json(&[]), contributor);
    let loader = plain_loader();

    // Repository-wide enumeration is given every loaded namespace; the
    // inventory-only scope is excluded because package-level selection
    // already loaded every scope the query named. Enumeration alone must
    // not demand unrelated native owners.
    let (engine, demands) = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(vec![PackageName::Root, PackageName::from("web")])
        .add_all_tasks()
        .build_with_unloaded_demands()
        .unwrap();
    assert!(
        demands.is_empty(),
        "whole-workspace enumeration respects the query scope"
    );
    assert!(
        engine
            .task_definition(&TaskId::new("native", "check"))
            .is_none()
    );
}

#[test]
fn normal_build_surfaces_unloaded_consultations_instead_of_ignoring_them() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let contributor = Arc::new(LazyNativeContributor::new(&repo_root));
    let package_graph = inventory_graph(&repo_root, web_package_json(&[]), contributor);
    let loader = cross_dep_loader();

    let error = web_build_builder(&repo_root, &package_graph, &loader, "build")
        .build()
        .unwrap_err();
    assert!(
        missing_task_names(&error)
            .iter()
            .any(|name| name == "native#check"),
        "the unloaded consultation must surface as a missing task definition"
    );
}

/// A still-missing unqualified task whose repo-wide search reaches an
/// inventory-only catalogue: the verdict is deferred to the owner's
/// authoritative discovery, never guessed — and once loaded, a genuinely
/// unknown task errors exactly as the eager baseline does.
#[test]
fn unqualified_missing_task_loads_catalogue_then_errors_exactly() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = current_thread_runtime();
    let contributor = LazyNativeContributor::new(&repo_root);
    let full_calls = contributor.full_calls.clone();
    let (graph, mut plan) = runtime
        .block_on(
            PackageGraph::builder(&repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(web_package_jsons(&repo_root, web_package_json(&[]))))
                .with_contributor(Arc::new(contributor))
                .build_lazy(),
        )
        .unwrap()
        .into_parts();
    let loader = plain_loader();

    // First pass: `doesnotexist` is defined by no loaded scope or config
    // chain, and the native catalogue is unread. The pass defers the verdict
    // by demanding the owner instead of guessing either way.
    let (_engine, demands) = web_build_builder(&repo_root, &graph, &loader, "doesnotexist")
        .build_with_unloaded_demands()
        .unwrap();
    assert_eq!(
        demands,
        HashSet::from([ToolchainId::new("lazy-native")]),
        "the repo-wide search reaches the native catalogue and defers to it"
    );

    // Load and re-probe: the native catalogue does not define the task, so
    // the baseline missing-task diagnostic fires with the exact name.
    let owners = HashSet::from([ToolchainId::new("lazy-native")]);
    let loaded = runtime.block_on(plan.load(&owners)).unwrap();
    let error = web_build_builder(&repo_root, &loaded, &loader, "doesnotexist")
        .build_with_unloaded_demands()
        .unwrap_err();
    assert!(
        missing_task_names(&error)
            .iter()
            .any(|name| name == "doesnotexist"),
        "after loading, an unknown task errors exactly as the eager baseline"
    );
    assert_eq!(full_calls.load(Ordering::SeqCst), 1);
}

/// An unqualified task that only the native catalogue defines, requested for
/// a selection that does not include the native scope: after the deferral
/// loads the owner, the probe proves the task defined, so the run proceeds
/// with zero tasks — exactly the eager baseline (the task exists, but no
/// selected package has it).
#[test]
fn unqualified_task_defined_only_by_native_catalogue_settles_to_zero_tasks() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = current_thread_runtime();
    let contributor = LazyNativeContributor::new(&repo_root);
    let full_calls = contributor.full_calls.clone();
    let (graph, mut plan) = runtime
        .block_on(
            PackageGraph::builder(&repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(web_package_jsons(&repo_root, web_package_json(&[]))))
                .with_contributor(Arc::new(contributor))
                .build_lazy(),
        )
        .unwrap()
        .into_parts();
    let loader = plain_loader();

    let (_engine, demands) = web_build_builder(&repo_root, &graph, &loader, "check")
        .build_with_unloaded_demands()
        .unwrap();
    assert_eq!(demands, HashSet::from([ToolchainId::new("lazy-native")]));

    let owners = HashSet::from([ToolchainId::new("lazy-native")]);
    let loaded = runtime.block_on(plan.load(&owners)).unwrap();
    let (engine, demands) = web_build_builder(&repo_root, &loaded, &loader, "check")
        .build_with_unloaded_demands()
        .unwrap();
    assert!(demands.is_empty(), "the deferral is settled");
    assert!(
        engine
            .task_definition(&TaskId::new("web", "check"))
            .is_none()
            && engine
                .task_definition(&TaskId::new("native", "check"))
                .is_none(),
        "the native scope is not in the selection, so the engine keeps zero of its tasks"
    );
    assert_eq!(full_calls.load(Ordering::SeqCst), 1);
}

/// A qualified missing task names one scope; its verdict never depends on
/// other scopes' catalogues, so it errors on the first pass and no native
/// owner is ever invoked.
#[test]
fn qualified_missing_js_task_errors_without_loading_other_owners() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let contributor = Arc::new(LazyNativeContributor::new(&repo_root));
    let full_calls = contributor.full_calls.clone();
    let package_graph = inventory_graph(&repo_root, web_package_json(&[]), contributor);
    let loader = plain_loader();

    let error = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
        .with_workspaces(vec![PackageName::from("web")])
        .with_tasks(Some(Spanned::new(TaskName::from("web#doesnotexist"))))
        .build_with_unloaded_demands()
        .unwrap_err();
    assert!(
        missing_task_names(&error)
            .iter()
            .any(|name| name == "web#doesnotexist"),
        "a qualified missing task errors immediately"
    );
    assert_eq!(
        full_calls.load(Ordering::SeqCst),
        0,
        "no native owner is invoked for a qualified missing JavaScript task"
    );
}

/// Strict task entrypoint selection asks, per requested task, whether any
/// scope's catalogue participates in it (`command_task_entrypoints`). An
/// inventory-only catalogue silently answers no, which would change the
/// selected JavaScript orchestration versus the eager baseline — so the run
/// treats the flag as a complete-graph catalogue query and loads every
/// owner before selection. This pins those catalogue semantics with the
/// generic fake.
#[test]
fn strict_entrypoint_catalogue_query_answers_only_from_loaded_owners() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = current_thread_runtime();
    let contributor = LazyNativeContributor::new(&repo_root);
    let (graph, mut plan) = runtime
        .block_on(
            PackageGraph::builder(&repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(web_package_jsons(&repo_root, web_package_json(&[]))))
                .with_contributor(Arc::new(contributor))
                .build_lazy(),
        )
        .unwrap()
        .into_parts();

    let catalogue_participates = |graph: &PackageGraph| {
        graph
            .package_task_contexts()
            .any(|context| context.native_tasks().participates("check"))
    };
    assert!(
        !catalogue_participates(&graph),
        "an inventory-only catalogue silently reports no participation, so the run must load \
         every owner before strict entrypoint selection"
    );

    let owners = HashSet::from([ToolchainId::new("lazy-native")]);
    let loaded = runtime.block_on(plan.load(&owners)).unwrap();
    assert!(
        catalogue_participates(&loaded),
        "the loaded catalogue answers the participation query exactly"
    );
}

/// The root's internal dependencies are file-hashed into every task, so the
/// run's construction loop must settle their closure: an inventory-only
/// member has unknown outgoing edges, and loading its owner can grow the
/// closure further. This test pins the graph facts that drive that loading.
#[test]
fn root_internal_native_dependency_joins_the_closure_before_hashing() {
    let repo = TempDir::new().unwrap();
    let repo_root = AbsoluteSystemPathBuf::try_from(repo.path().to_path_buf()).unwrap();
    let runtime = current_thread_runtime();
    let contributor = LazyNativeContributor::new(&repo_root);
    let full_calls = contributor.full_calls.clone();
    // Root npm dependencies target package.json scopes. An explicit relationship
    // from that package can still put a native scope in the transitive closure.
    let root = PackageJson {
        dependencies: Some(
            [("bridge", "workspace:*")]
                .into_iter()
                .map(|(name, version)| (name.to_string(), version.to_string()))
                .collect(),
        ),
        ..Default::default()
    };
    let (_graph, mut plan) = runtime
        .block_on(
            PackageGraph::builder(&repo_root, root)
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(web_package_jsons(&repo_root, web_package_json(&[]))))
                .with_contributor(Arc::new(DeclaredDependencyContributor {
                    repo_root: repo_root.clone(),
                    name: "bridge",
                }))
                .with_contributor(Arc::new(contributor))
                .build_lazy(),
        )
        .unwrap()
        .into_parts();
    let graph = runtime
        .block_on(plan.load(&HashSet::from([ToolchainId::new("declared-relationships")])))
        .unwrap();
    assert_eq!(full_calls.load(Ordering::SeqCst), 0);

    // The root -> bridge -> native relationships put the inventory-only native
    // scope in the closure whose directories the run hashes for every task.
    let closure = graph.root_internal_package_dependencies();
    assert!(
        closure
            .iter()
            .any(|package| package.name == PackageName::from("native")),
        "the root's declared native dependency joins the hashed closure"
    );
    assert_eq!(
        graph.unloaded_scope_owner(&PackageName::from("native")),
        Some(&ToolchainId::new("lazy-native")),
        "the closure member is inventory-only: its edges, and therefore the closure and hashed \
         directory set, are unknown until its owner loads"
    );

    // Loading the owner settles the closure member: no scope of the hashed
    // closure remains inventory-only, so the hashed directory set is final.
    let owners = HashSet::from([ToolchainId::new("lazy-native")]);
    let loaded = runtime.block_on(plan.load(&owners)).unwrap();
    let closure = loaded.root_internal_package_dependencies();
    assert!(
        closure
            .iter()
            .any(|package| package.name == PackageName::from("native")),
        "the native scope stays in the closure after loading"
    );
    assert!(
        closure
            .iter()
            .all(|package| loaded.unloaded_scope_owner(&package.name).is_none()),
        "after loading, no hashed closure member is inventory-only"
    );
}
