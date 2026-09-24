use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use turborepo_repository::{
    change_mapper::{AllPackageChangeReason, PackageInclusionReason},
    package_graph::PackageName,
};

use crate::{Error, QueryRun, QueryTaskId};

/// Why a specific task is affected by changes.
#[derive(Debug, Clone)]
pub enum TaskChangeReason {
    /// A file that is part of this task's inputs changed directly.
    FileChanged { file_path: String },
    /// An upstream task dependency is affected, causing this task to be
    /// affected.
    DependencyTaskChanged {
        task_name: String,
        package_name: String,
    },
    /// This package's lockfile-derived external dependency closure changed.
    /// Unlike `DependencyTaskChanged`, there is no specific upstream task.
    PackageDependencyChanged { package_name: String },
    /// A global file (package.json, turbo.json, etc.) changed, affecting all
    /// tasks.
    GlobalFileChanged { file_path: String },
    /// A configured global dependency changed.
    GlobalDepsChanged { file_path: String },
    /// All tasks affected due to a lockfile, git ref, or other global change.
    AllTasksChanged { description: String },
}

/// A task that was determined to be affected by changes.
#[derive(Debug)]
pub struct AffectedTask {
    pub task_id: QueryTaskId,
    pub reason: TaskChangeReason,
}

/// Computes which tasks are affected by changes between two git refs.
///
/// # Algorithm
///
/// 1. **All-packages check**: If `calculate_affected_packages` reports a global
///    change (lockfile, global dep, missing git ref), every task in the engine
///    is returned immediately with the corresponding reason.
///
/// 2. **Direct input matching**: Each task's `inputs` globs are checked against
///    the changed files. Packages whose lockfile-derived external dependency
///    closure changed seed their own tasks directly, even when no package-local
///    file changed.
///
/// 3. **Graph propagation**: BFS from directly affected tasks through the task
///    dependency graph in O(V + E). If task A depends on task B and B is
///    affected, A is marked affected with a `DependencyTaskChanged` reason.
pub fn calculate_affected_tasks(
    run: &Arc<dyn QueryRun>,
    base: Option<String>,
    head: Option<String>,
) -> Result<Vec<AffectedTask>, Error> {
    let affected_packages = run.calculate_affected_packages(base.clone(), head.clone())?;
    calculate_affected_tasks_with_packages(run, base, head, &affected_packages)
}

/// Reuse legacy package affectedness when the caller also needs its detailed
/// reasons. This remains raw task affectedness, without scheduled
/// prerequisites.
pub(crate) fn calculate_affected_tasks_with_packages(
    run: &Arc<dyn QueryRun>,
    base: Option<String>,
    head: Option<String>,
    affected_packages: &HashMap<PackageName, PackageInclusionReason>,
) -> Result<Vec<AffectedTask>, Error> {
    // Check if this is an "all packages changed" scenario
    let all_packages_reason = affected_packages.values().find_map(|reason| match reason {
        PackageInclusionReason::All(all_reason) => Some(all_reason.clone()),
        _ => None,
    });

    if let Some(all_reason) = all_packages_reason {
        // Every task in the engine is affected
        let description = match &all_reason {
            AllPackageChangeReason::GlobalDepsChanged { file } => {
                return Ok(run
                    .task_ids()
                    .into_iter()
                    .map(|task_id| AffectedTask {
                        task_id: task_id.clone(),
                        reason: TaskChangeReason::GlobalDepsChanged {
                            file_path: file.to_string(),
                        },
                    })
                    .collect());
            }
            AllPackageChangeReason::DefaultGlobalFileChanged { file } => {
                return Ok(run
                    .task_ids()
                    .into_iter()
                    .map(|task_id| AffectedTask {
                        task_id: task_id.clone(),
                        reason: TaskChangeReason::GlobalFileChanged {
                            file_path: file.to_string(),
                        },
                    })
                    .collect());
            }
            AllPackageChangeReason::LockfileChangeDetectionFailed => {
                "lockfile change detection failed".to_string()
            }
            AllPackageChangeReason::LockfileChangedWithoutDetails => "lockfile changed".to_string(),
            AllPackageChangeReason::RootInternalDepChanged { root_internal_dep } => {
                format!("root internal dependency changed: {root_internal_dep}")
            }
            AllPackageChangeReason::GitRefNotFound { .. } => "git ref not found".to_string(),
            AllPackageChangeReason::ScmError { ref error } => {
                format!("SCM error: {error}")
            }
            AllPackageChangeReason::ConservativeFallback => {
                "conservative affectedness fallback".to_string()
            }
        };

        return Ok(run
            .task_ids()
            .into_iter()
            .map(|task_id| AffectedTask {
                task_id,
                reason: TaskChangeReason::AllTasksChanged {
                    description: description.clone(),
                },
            })
            .collect());
    }

    // Get the raw changed files for input-level matching
    let changed_files = run.changed_files(base.as_deref(), head.as_deref())?;

    // Phase 1: Direct task affectedness — check each task's inputs against
    // changed files. The run side owns the engine-specific matching and returns
    // only task identities and matching file paths.
    let matched = match run.match_tasks_against_changed_files(&changed_files) {
        Ok(matched) => matched,
        Err(error) => {
            tracing::error!("failed to determine affected tasks: {error}");
            return Ok(run
                .task_ids()
                .into_iter()
                .map(|task_id| AffectedTask {
                    task_id,
                    reason: TaskChangeReason::AllTasksChanged {
                        description: "conservative affectedness fallback".to_string(),
                    },
                })
                .collect());
        }
    };
    let mut affected: HashMap<QueryTaskId, TaskChangeReason> = matched
        .into_iter()
        .map(|(task_id, file_path)| (task_id, TaskChangeReason::FileChanged { file_path }))
        .collect();

    let lockfile_changed_packages: HashSet<&str> = affected_packages
        .iter()
        .filter_map(|(package_name, reason)| match reason {
            PackageInclusionReason::ConservativeRootLockfileChanged
            | PackageInclusionReason::LockfileChanged { .. } => Some(package_name.as_str()),
            _ => None,
        })
        .collect();

    if !lockfile_changed_packages.is_empty() {
        for task_id in run.task_ids() {
            if lockfile_changed_packages.contains(task_id.package.as_str()) {
                affected.entry(task_id.clone()).or_insert_with(|| {
                    TaskChangeReason::PackageDependencyChanged {
                        package_name: task_id.package.clone(),
                    }
                });
            }
        }
    }

    // Phase 2: Propagate through the task dependency graph via BFS.
    // If task B depends on task A and A is affected, B is also affected.
    let mut visited: HashSet<QueryTaskId> = affected.keys().cloned().collect();
    let mut queue: VecDeque<QueryTaskId> = affected.keys().cloned().collect();

    while let Some(cause_id) = queue.pop_front() {
        for dependent_id in run.task_dependents(&cause_id) {
            if !visited.insert(dependent_id.clone()) {
                continue;
            }
            queue.push_back(dependent_id.clone());
            affected.insert(
                dependent_id,
                TaskChangeReason::DependencyTaskChanged {
                    task_name: cause_id.task.clone(),
                    package_name: cause_id.package.clone(),
                },
            );
        }
    }

    Ok(affected
        .into_iter()
        .map(|(task_id, reason)| AffectedTask { task_id, reason })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
    use turborepo_engine::Building;
    use turborepo_microfrontends_config::UnifiedTurboJsonLoader;
    use turborepo_query_api::{AffectedPackagesError, BoundariesFuture};
    use turborepo_repository::{
        change_mapper::PackageInclusionReason,
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_graph::{PackageGraph, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_run_context::RepoContext;
    use turborepo_scm::SCM;
    use turborepo_task_id::TaskId;
    use turborepo_turbo_json::TurboJson;
    use turborepo_types::{TaskDefinition, TaskInputs};
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::QueryRun;

    struct MockDiscovery;

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<DiscoveryResponse, turborepo_repository::discovery::Error> {
            self.discover_packages().await
        }
    }

    async fn make_pkg_graph(repo_root: &AbsoluteSystemPath, packages: &[&str]) -> PackageGraph {
        let mut pkgs = HashMap::new();
        for name in packages {
            let path = repo_root.join_components(&["packages", name, "package.json"]);
            let pkg = PackageJson {
                name: Some(turborepo_errors::Spanned::new(name.to_string())),
                ..Default::default()
            };
            pkgs.insert(path, pkg);
        }
        PackageGraph::builder(repo_root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(pkgs))
            .build()
            .await
            .unwrap()
    }

    fn make_engine(
        tasks: &[(TaskId<'static>, TaskDefinition)],
    ) -> turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition> {
        make_engine_with_edges(tasks, &[])
    }

    fn make_engine_with_edges(
        tasks: &[(TaskId<'static>, TaskDefinition)],
        edges: &[(TaskId<'static>, TaskId<'static>)],
    ) -> turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition> {
        let mut engine: turborepo_engine::Engine<Building, TaskDefinition> =
            turborepo_engine::Engine::new();
        for (task_id, def) in tasks {
            engine.get_index(task_id);
            engine.add_definition(task_id.clone(), def.clone());
        }
        for (from, to) in edges {
            let from_idx = engine.get_index(from);
            let to_idx = engine.get_index(to);
            engine.task_graph_mut().add_edge(from_idx, to_idx, ());
        }
        engine.seal()
    }

    fn make_repo_context(
        repo_root: &AbsoluteSystemPath,
        pkg_dep_graph: PackageGraph,
        root_turbo_json: TurboJson,
    ) -> RepoContext {
        let turbo_json_loader = UnifiedTurboJsonLoader::noop(HashMap::from([(
            PackageName::Root,
            root_turbo_json.clone(),
        )]));
        RepoContext {
            repo_root: repo_root.to_owned(),
            color_config: ColorConfig::new(true),
            version: "test",
            scm: SCM::new(repo_root),
            pkg_dep_graph: Arc::new(pkg_dep_graph),
            turbo_json_loader,
            root_turbo_json,
        }
    }

    fn query_task_id(task_id: &TaskId) -> QueryTaskId {
        QueryTaskId::new(task_id.package(), task_id.task())
    }

    fn engine_task_id(task_id: &QueryTaskId) -> TaskId<'static> {
        TaskId::from_static(task_id.package.clone(), task_id.task.clone())
    }

    fn query_task_nodes<'a>(
        nodes: impl IntoIterator<Item = &'a turborepo_engine::TaskNode>,
    ) -> Vec<QueryTaskId> {
        nodes
            .into_iter()
            .filter_map(|node| match node {
                turborepo_engine::TaskNode::Root => None,
                turborepo_engine::TaskNode::Task(task_id) => Some(query_task_id(task_id)),
            })
            .collect()
    }

    struct MockQueryRun {
        engine: turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition>,
        repo_context: RepoContext,
        affected_packages: HashMap<PackageName, PackageInclusionReason>,
        changed_files: HashSet<AnchoredSystemPathBuf>,
        recorded_calls: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl QueryRun for MockQueryRun {
        fn repo_context(&self) -> &RepoContext {
            &self.repo_context
        }

        fn task_ids(&self) -> Vec<QueryTaskId> {
            self.engine.task_ids().map(query_task_id).collect()
        }

        fn task_ids_for_package(&self, package: &str) -> Vec<QueryTaskId> {
            self.recorded_calls
                .lock()
                .unwrap()
                .push(format!("task_ids_for_package:{package}"));
            self.engine
                .task_ids_for_packages(&HashSet::from([PackageName::from(package)]))
                .iter()
                .map(query_task_id)
                .collect()
        }

        fn task_definition(&self, task_id: &QueryTaskId) -> Option<&TaskDefinition> {
            self.engine.task_definition(&engine_task_id(task_id))
        }

        fn task_dependencies(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId> {
            query_task_nodes(
                self.engine
                    .dependencies(&engine_task_id(task_id))
                    .into_iter()
                    .flatten(),
            )
        }

        fn task_dependents(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId> {
            query_task_nodes(
                self.engine
                    .dependents(&engine_task_id(task_id))
                    .into_iter()
                    .flatten(),
            )
        }

        fn transitive_task_dependencies(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId> {
            query_task_nodes(
                self.engine
                    .transitive_dependencies(&engine_task_id(task_id)),
            )
        }

        fn transitive_task_dependents(&self, task_id: &QueryTaskId) -> Vec<QueryTaskId> {
            query_task_nodes(self.engine.transitive_dependents(&engine_task_id(task_id)))
        }

        fn collect_task_dependencies(
            &self,
            task_ids: &HashSet<QueryTaskId>,
        ) -> HashSet<QueryTaskId> {
            let task_ids = task_ids.iter().map(engine_task_id).collect();
            self.engine
                .collect_task_dependencies(&task_ids)
                .iter()
                .map(query_task_id)
                .collect()
        }

        fn calculate_affected_packages(
            &self,
            _base: Option<String>,
            _head: Option<String>,
        ) -> Result<HashMap<PackageName, PackageInclusionReason>, AffectedPackagesError> {
            Ok(self.affected_packages.clone())
        }

        fn changed_files(
            &self,
            _base: Option<&str>,
            _head: Option<&str>,
        ) -> Result<HashSet<AnchoredSystemPathBuf>, AffectedPackagesError> {
            Ok(self.changed_files.clone())
        }

        fn match_tasks_against_changed_files(
            &self,
            changed_files: &HashSet<AnchoredSystemPathBuf>,
        ) -> Result<HashMap<QueryTaskId, String>, AffectedPackagesError> {
            turborepo_engine::match_tasks_against_changed_files(
                &self.engine,
                self.repo_context.pkg_dep_graph(),
                changed_files,
            )
            .map(|matched| {
                matched
                    .into_iter()
                    .map(|(task_id, file)| (query_task_id(&task_id), file))
                    .collect()
            })
            .map_err(|error| AffectedPackagesError::Other(Box::new(error)))
        }

        fn check_boundaries(&self, _show_progress: bool) -> BoundariesFuture<'_> {
            unimplemented!("not needed for affected_tasks tests")
        }
    }

    // These packages intentionally have no manifest dependency edges. Only the
    // explicit task edges connect app-a to lib-a and the unchanged lib-b.
    async fn affected_packages_query_run(
        root: &AbsoluteSystemPath,
        affected_using_task_inputs: bool,
        filter_using_tasks: bool,
        files: &[&str],
    ) -> Arc<MockQueryRun> {
        let pkg_dep_graph =
            make_pkg_graph(root, &["app-a", "lib-a", "lib-b", "ignored", "no-tasks"]).await;
        let source_task = TaskDefinition {
            inputs: TaskInputs {
                globs: vec!["src/**".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = make_engine_with_edges(
            &[
                (TaskId::new("lib-a", "build"), source_task.clone()),
                (TaskId::new("lib-a", "test"), source_task.clone()),
                (TaskId::new("lib-b", "build"), source_task.clone()),
                (TaskId::new("app-a", "build"), source_task.clone()),
                (TaskId::new("app-a", "test"), source_task.clone()),
                (TaskId::new("ignored", "build"), source_task),
                (
                    TaskId::new("//", "build"),
                    TaskDefinition {
                        inputs: TaskInputs {
                            globs: vec!["root.txt".to_string()],
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                ),
            ],
            &[
                (TaskId::new("app-a", "build"), TaskId::new("lib-a", "build")),
                (TaskId::new("app-a", "test"), TaskId::new("lib-a", "build")),
                (TaskId::new("app-a", "build"), TaskId::new("lib-b", "build")),
            ],
        );
        let affected_packages = files
            .iter()
            .map(|file| {
                let package = file
                    .strip_prefix("packages/")
                    .and_then(|file| file.split('/').next())
                    .unwrap_or("//");
                (
                    PackageName::from(package),
                    PackageInclusionReason::FileChanged {
                        file: AnchoredSystemPathBuf::from_raw(file).unwrap(),
                    },
                )
            })
            .collect();
        let mut root_turbo_json = TurboJson::default();
        root_turbo_json.future_flags.affected_using_task_inputs = affected_using_task_inputs;
        root_turbo_json.future_flags.filter_using_tasks = filter_using_tasks;
        Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, pkg_dep_graph, root_turbo_json),
            affected_packages,
            changed_files: files
                .iter()
                .map(|file| AnchoredSystemPathBuf::from_raw(file).unwrap())
                .collect(),
        })
    }

    async fn query_data(run: Arc<dyn QueryRun>, query: &str) -> serde_json::Value {
        let result = crate::execute_query(run, query, None).await.unwrap();
        let result: serde_json::Value = serde_json::from_str(&result.result_json).unwrap();
        assert!(result.get("errors").is_none(), "{result}");
        result["data"].clone()
    }

    #[derive(Default)]
    struct RecordingQueryServer {
        calls: std::sync::Mutex<Vec<(String, Option<String>)>>,
    }

    impl turborepo_query_api::QueryServer for RecordingQueryServer {
        fn execute_query<'a>(
            &'a self,
            run: Arc<dyn QueryRun>,
            query: &'a str,
            variables_json: Option<&'a str>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            turborepo_query_api::QueryResult,
                            turborepo_query_api::Error,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push((query.to_string(), variables_json.map(str::to_string)));
                crate::execute_query(run, query, variables_json)
                    .await
                    .map_err(Into::into)
            })
        }

        fn run_query_server(
            &self,
            _run: Arc<dyn QueryRun>,
            _signal: turborepo_signals::SignalHandler,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<(), turborepo_query_api::Error>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { unreachable!("recording fake never starts a network server") })
        }
    }

    #[tokio::test]
    async fn injected_manifest_graph_projects_tasks_through_recording_query_contracts() {
        use turborepo_query_api::QueryServer;
        use turborepo_repository::discovery::WorkspaceData;

        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let app_manifest = root.join_components(&["packages", "app", "package.json"]);
        let lib_manifest = root.join_components(&["packages", "lib", "package.json"]);
        let response = DiscoveryResponse {
            package_manager: PackageManager::Npm,
            workspaces: [app_manifest.clone(), lib_manifest.clone()]
                .into_iter()
                .map(|path| WorkspaceData::new(path, None).unwrap())
                .collect(),
        };
        let manifests = HashMap::from([
            (
                app_manifest,
                PackageJson::from_value(serde_json::json!({
                    "name": "app", "scripts": {"build": "echo app"},
                    "dependencies": {"lib": "*"}
                }))
                .unwrap(),
            ),
            (
                lib_manifest,
                PackageJson::from_value(serde_json::json!({
                    "name": "lib", "scripts": {"build": "echo lib"}
                }))
                .unwrap(),
            ),
        ]);
        let discovery_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let loader_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let graph = PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery({
                let calls = discovery_calls.clone();
                move || {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let response = response.clone();
                    async move { Ok(response) }
                }
            })
            .with_package_json_loader({
                let calls = loader_calls.clone();
                move |path: &AbsoluteSystemPath| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    manifests.get(path).cloned().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "missing injected manifest",
                        )
                        .into()
                    })
                }
            })
            .without_external_dependencies()
            .build()
            .await
            .unwrap();
        assert_eq!(discovery_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(loader_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            graph
                .filtering_relationships()
                .transitive_dependencies(&PackageName::from("app"))
                .unwrap(),
            [PackageName::from("lib")],
            "the manifest loader must produce the real package edge"
        );
        let engine = make_engine_with_edges(
            &[
                (TaskId::new("app", "build"), TaskDefinition::default()),
                (TaskId::new("lib", "build"), TaskDefinition::default()),
            ],
            &[(TaskId::new("app", "build"), TaskId::new("lib", "build"))],
        );
        let run = Arc::new(MockQueryRun {
            engine,
            repo_context: make_repo_context(root, graph, TurboJson::default()),
            affected_packages: HashMap::new(),
            changed_files: HashSet::new(),
            recorded_calls: Default::default(),
        });
        let server = RecordingQueryServer::default();
        let query = r#"query($name: String!) { package(name: $name) { name tasks { items {
            name fullName script directDependencies { items { fullName } }
        } } } }"#;
        let result = server
            .execute_query(run.clone(), query, Some(r#"{"name":"app"}"#))
            .await
            .unwrap();
        assert!(result.errors.is_empty(), "{}", result.result_json);
        let data: serde_json::Value = serde_json::from_str(&result.result_json).unwrap();
        assert_eq!(
            data["data"]["package"]["tasks"]["items"],
            serde_json::json!([{
                "name": "build", "fullName": "app#build", "script": "echo app",
                "directDependencies": {"items": [{"fullName": "lib#build"}]}
            }])
        );
        assert_eq!(
            *server.calls.lock().unwrap(),
            [(query.to_string(), Some(r#"{"name":"app"}"#.to_string()))]
        );
        assert!(run
            .recorded_calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == "task_ids_for_package:app"));
    }

    // Exercise the query projection with full Cargo task observations, but no
    // cargo metadata process, CLI invocation, or on-disk Cargo workspace.
    struct QueryCargoContributor {
        root: turbopath::AbsoluteSystemPathBuf,
    }

    impl turborepo_repository::toolchain::RepositoryContributor for QueryCargoContributor {
        fn id(&self) -> turborepo_repository::toolchain::ToolchainId {
            turborepo_repository::toolchain::ToolchainId::RUST
        }

        fn discover_package_scopes(
            &self,
        ) -> turborepo_repository::toolchain::DiscoverPackageScopesFuture<'_> {
            Box::pin(async move {
                let packages = self.discover_packages().await?;
                Ok(
                    turborepo_repository::toolchain::DiscoveredPackageScopes::from_full_observation(
                        packages.packages(),
                        packages.workspace_roots(),
                    ),
                )
            })
        }

        fn discover_packages(&self) -> turborepo_repository::toolchain::DiscoverPackagesFuture<'_> {
            use turborepo_repository::{
                cargo::{
                    native_tasks_for_package, CargoPackageDetails, CargoPackageKind, Deliverable,
                    DeliverableKind,
                },
                toolchain::{DiscoveredPackage, DiscoveredPackages, WorkspaceRoot},
            };

            Box::pin(async move {
                let packages = [
                    ("app", CargoPackageKind::Entrypoint, "crates/app/Cargo.toml"),
                    (
                        "lib-a",
                        CargoPackageKind::Library,
                        "crates/lib-a/Cargo.toml",
                    ),
                    ("acme", CargoPackageKind::Workspace, "Cargo.toml"),
                ]
                .into_iter()
                .map(|(name, kind, path)| {
                    let details = CargoPackageDetails {
                        kind,
                        deliverables: (kind == CargoPackageKind::Entrypoint)
                            .then(|| Deliverable {
                                name: name.to_string(),
                                kind: DeliverableKind::Bin,
                            })
                            .into_iter()
                            .collect(),
                        manifest_alters_output_layout: false,
                    };
                    let manifest = self
                        .root
                        .join_components(&path.split('/').collect::<Vec<_>>());
                    let package = if kind == CargoPackageKind::Workspace {
                        DiscoveredPackage::aggregate(
                            name.to_string(),
                            PackageJson::default(),
                            manifest,
                        )
                    } else {
                        DiscoveredPackage::package(
                            Some(name.to_string()),
                            PackageJson::default(),
                            manifest,
                        )
                    };
                    package
                        .with_native_relationships(Vec::new())
                        .with_native_tasks(native_tasks_for_package(&details, name))
                })
                .collect();
                Ok(DiscoveredPackages::new(
                    packages,
                    vec![WorkspaceRoot::new("cargo", self.root.clone())],
                ))
            })
        }
    }

    async fn cargo_query_run(root: &AbsoluteSystemPath, javascript: bool) -> Arc<MockQueryRun> {
        let mut builder = if javascript {
            PackageGraph::builder_optional(root, Some(PackageJson::default()))
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(HashMap::from([(
                    root.join_components(&["packages", "web", "package.json"]),
                    PackageJson::from_value(serde_json::json!({
                        "name": "web", "scripts": {"build": "echo web", "doc": "echo docs"}
                    }))
                    .unwrap(),
                )])))
        } else {
            PackageGraph::builder_optional(root, None)
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(HashMap::new()))
        };
        builder = builder.with_contributor(Arc::new(QueryCargoContributor {
            root: root.to_owned(),
        }));
        let graph = builder.build().await.unwrap();
        let tasks = ["app", "lib-a", "acme"]
            .into_iter()
            .flat_map(|name| {
                graph
                    .package_task_context(&PackageName::from(name))
                    .unwrap()
                    .native_tasks()
                    .registered_names()
                    .into_iter()
                    .map(move |task| {
                        (
                            TaskId::from_static(name.to_string(), task.to_string()),
                            TaskDefinition::default(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        Arc::new(MockQueryRun {
            engine: make_engine(&tasks),
            repo_context: make_repo_context(root, graph, TurboJson::default()),
            affected_packages: HashMap::new(),
            changed_files: HashSet::new(),
            recorded_calls: Default::default(),
        })
    }

    async fn queried_tasks(run: Arc<MockQueryRun>, name: &str) -> serde_json::Value {
        let data = query_data(
            run,
            &format!(
                r#"{{ package(name: "{name}") {{ tasks {{ items {{ name script command }} }} }} }}"#
            ),
        )
        .await;
        let items = data["package"]["tasks"]["items"].as_array().unwrap();
        items
            .iter()
            .map(|task| (task["name"].as_str().unwrap().to_string(), task.clone()))
            .collect::<serde_json::Map<_, _>>()
            .into()
    }

    #[tokio::test]
    async fn cargo_package_and_aggregate_tasks_query_native_commands_without_aliases() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let run = cargo_query_run(root, false).await;
        assert!(!run.repo_context.pkg_dep_graph().has_root_javascript_scope());

        for (package, expected) in [
            (
                "app",
                vec![
                    ("build", "cargo build --package=app --locked"),
                    ("run", "cargo run --package=app --locked"),
                    ("dev", "cargo run --package=app --locked"),
                    ("test", "cargo test --package=app --locked"),
                    ("check", "cargo check --package=app --locked"),
                    ("lint", "cargo clippy --package=app --locked"),
                    ("format", "cargo fmt --package=app"),
                ],
            ),
            (
                "lib-a",
                vec![
                    ("build", "cargo build --package=lib-a --locked"),
                    ("test", "cargo test --package=lib-a --locked"),
                    ("check", "cargo check --package=lib-a --locked"),
                    ("lint", "cargo clippy --package=lib-a --locked"),
                    ("format", "cargo fmt --package=lib-a"),
                ],
            ),
            (
                "acme",
                vec![
                    ("test", "cargo test --workspace --locked"),
                    ("check", "cargo check --workspace --locked"),
                    ("lint", "cargo clippy --workspace --locked"),
                    ("format", "cargo fmt --all"),
                ],
            ),
        ] {
            let tasks = queried_tasks(run.clone(), package).await;
            let names: HashSet<_> = tasks
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                names,
                expected.iter().map(|(name, _)| *name).collect(),
                "{package}"
            );
            for (name, command) in expected {
                assert_eq!(tasks[name]["command"], command, "{package}#{name}");
                assert!(tasks[name]["script"].is_null(), "{package}#{name}");
            }
            for alias in ["doc", "docs", "clippy", "bench"] {
                assert!(tasks.get(alias).is_none(), "{package}#{alias}");
            }
        }
    }

    #[tokio::test]
    async fn mixed_cargo_and_javascript_tasks_keep_javascript_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let run = cargo_query_run(root, true).await;
        let web = queried_tasks(run.clone(), "web").await;
        assert_eq!(
            web["build"],
            serde_json::json!({
                "name": "build", "script": "echo web", "command": "echo web"
            })
        );
        assert_eq!(web["doc"]["script"], "echo docs");
        assert!(web.get("lint").is_none());
        let rust = queried_tasks(run, "lib-a").await;
        assert_eq!(
            rust["lint"]["command"],
            "cargo clippy --package=lib-a --locked"
        );
        assert!(rust.get("doc").is_none());
    }

    #[tokio::test]
    async fn affected_packages_projects_raw_task_owners_and_preserves_predicates() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let run =
            affected_packages_query_run(root, true, false, &["packages/lib-a/src/index.ts"]).await;
        // The existing dependency-count predicate counts the queryable root
        // node, even though app-a has no manifest dependencies on either lib.
        let data = query_data(
            run,
            r#"{
                affectedPackages {
                    length
                    items {
                        name
                        reason {
                            __typename
                            ... on FileChanged { filePath }
                            ... on DependencyChanged { dependencyName }
                        }
                    }
                }
                filtered: affectedPackages(filter: {and: [
                    {equal: {field: NAME, value: "app-a"}},
                    {equal: {field: DIRECT_DEPENDENCY_COUNT, value: 1}}
                ]}) { length items { name } }
                prerequisite: affectedPackages(filter: {equal: {field: NAME, value: "lib-b"}}) {
                    length items { name }
                }
            }"#,
        )
        .await;
        assert_eq!(
            data,
            serde_json::json!({
                "affectedPackages": {
                    "length": 2,
                    "items": [
                        {"name": "app-a", "reason": {
                            "__typename": "DependencyChanged", "dependencyName": "lib-a"
                        }},
                        {"name": "lib-a", "reason": {
                            "__typename": "FileChanged", "filePath": "packages/lib-a/src/index.ts"
                        }}
                    ]
                },
                "filtered": {"length": 1, "items": [{"name": "app-a"}]},
                "prerequisite": {"length": 0, "items": []}
            })
        );
    }

    #[tokio::test]
    async fn affected_packages_preserves_lockfile_reasons_and_maps_upstream_task_changes() {
        use serde_json::json;

        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        for (reason, expected) in [
            (
                PackageInclusionReason::LockfileChanged {
                    added: vec![turborepo_lockfiles::Package {
                        key: "new-dep".to_string(),
                        version: "2.0.0".to_string(),
                    }],
                    removed: vec![turborepo_lockfiles::Package {
                        key: "old-dep".to_string(),
                        version: "1.0.0".to_string(),
                    }],
                },
                json!({
                    "__typename": "LockfileChanged", "empty": false,
                    "added": {"length": 1, "items": [{"name": "new-dep"}]},
                    "removed": {"length": 1, "items": [{"name": "old-dep"}]}
                }),
            ),
            (
                PackageInclusionReason::ConservativeRootLockfileChanged,
                json!({"__typename": "ConservativeRootLockfileChanged", "empty": false}),
            ),
        ] {
            let mut run = affected_packages_query_run(root, true, false, &[]).await;
            Arc::get_mut(&mut run).unwrap().affected_packages = HashMap::from([
                (PackageName::from("lib-a"), reason),
                // Legacy package propagation must not replace the task graph's
                // explanation for app-a with an unrelated package dependency.
                (
                    PackageName::from("app-a"),
                    PackageInclusionReason::DependencyChanged {
                        dependency: PackageName::from("lib-b"),
                    },
                ),
            ]);
            let data = query_data(
                run,
                "{ affectedPackages { length items { name reason {
                    __typename
                    ... on LockfileChanged { empty added { length items { name } }
                        removed { length items { name } } }
                    ... on ConservativeRootLockfileChanged { empty }
                    ... on DependencyChanged { dependencyName }
                } } } }",
            )
            .await;
            assert_eq!(
                data["affectedPackages"],
                json!({"length": 2, "items": [
                    {"name": "app-a", "reason": {
                        "__typename": "DependencyChanged", "dependencyName": "lib-a"
                    }},
                    {"name": "lib-a", "reason": expected}
                ]})
            );
        }
    }

    #[tokio::test]
    async fn affected_packages_preserves_structured_global_reasons_for_raw_task_owners() {
        use serde_json::json;

        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        for (reason, expected) in [
            (
                AllPackageChangeReason::GitRefNotFound {
                    from_ref: Some("missing-base".to_string()),
                    to_ref: Some("missing-head".to_string()),
                },
                json!({"__typename": "GitRefNotFound", "fromRef": "missing-base", "toRef": "missing-head"}),
            ),
            (
                AllPackageChangeReason::RootInternalDepChanged {
                    root_internal_dep: PackageName::from("lib-a"),
                },
                json!({"__typename": "RootInternalDepChanged", "rootInternalDep": "lib-a"}),
            ),
            (
                AllPackageChangeReason::ScmError {
                    error: "git failed".to_string(),
                },
                json!({"__typename": "ScmError", "error": "git failed"}),
            ),
            (
                AllPackageChangeReason::LockfileChangeDetectionFailed,
                json!({"__typename": "LockfileChangeDetectionFailed", "empty": false}),
            ),
            (
                AllPackageChangeReason::LockfileChangedWithoutDetails,
                json!({"__typename": "LockfileChangedWithoutDetails", "empty": false}),
            ),
            (
                AllPackageChangeReason::ConservativeFallback,
                json!({"__typename": "AllPackagesChanged", "empty": false}),
            ),
        ] {
            let mut run = affected_packages_query_run(root, true, false, &[]).await;
            // Only the taskless package appears in the legacy map. Its global
            // reason applies to raw task owners, but it must not join the result.
            Arc::get_mut(&mut run).unwrap().affected_packages = HashMap::from([(
                PackageName::from("no-tasks"),
                PackageInclusionReason::All(reason),
            )]);
            let data = query_data(
                run,
                "{ affectedPackages { length items { name reason {
                    __typename
                    ... on GitRefNotFound { fromRef toRef }
                    ... on RootInternalDepChanged { rootInternalDep }
                    ... on ScmError { error }
                    ... on LockfileChangeDetectionFailed { empty }
                    ... on LockfileChangedWithoutDetails { empty }
                    ... on AllPackagesChanged { empty }
                } } } }",
            )
            .await;
            let items: Vec<_> = ["//", "app-a", "ignored", "lib-a", "lib-b"]
                .into_iter()
                .map(|name| json!({"name": name, "reason": expected}))
                .collect();
            assert_eq!(
                data["affectedPackages"],
                json!({"length": 5, "items": items})
            );
        }
    }

    #[tokio::test]
    async fn affected_packages_flag_off_keeps_legacy_owners_even_with_filter_using_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        for filter_using_tasks in [false, true] {
            let run = affected_packages_query_run(
                root,
                false,
                filter_using_tasks,
                &["packages/lib-a/src/index.ts"],
            )
            .await;
            let data = query_data(
                run,
                "{ affectedPackages { length items { name reason { __typename ... on FileChanged \
                 { filePath } } } } }",
            )
            .await;
            assert_eq!(
                data["affectedPackages"],
                serde_json::json!({"length": 1, "items": [{
                    "name": "lib-a",
                    "reason": {"__typename": "FileChanged", "filePath": "packages/lib-a/src/index.ts"}
                }]})
            );
        }
    }

    #[tokio::test]
    async fn affected_packages_ignores_non_inputs_and_packages_without_tasks_only_with_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        for enabled in [false, true] {
            let run = affected_packages_query_run(
                root,
                enabled,
                false,
                &[
                    "packages/ignored/README.md",
                    "packages/no-tasks/src/index.ts",
                ],
            )
            .await;
            let data = query_data(run, "{ affectedPackages { length items { name } } }").await;
            let expected = if enabled {
                serde_json::json!({"length": 0, "items": []})
            } else {
                serde_json::json!({"length": 2, "items": [{"name": "ignored"}, {"name": "no-tasks"}]})
            };
            assert_eq!(data["affectedPackages"], expected);
        }
    }

    #[tokio::test]
    async fn affected_packages_preserves_root_package() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        for enabled in [false, true] {
            let run = affected_packages_query_run(root, enabled, false, &["root.txt"]).await;
            let data = query_data(run, "{ affectedPackages { length items { name } } }").await;
            assert_eq!(
                data["affectedPackages"],
                serde_json::json!({"length": 1, "items": [{"name": "//"}]})
            );
        }
    }

    /// Regression test: a task in a non-affected package that has
    /// $TURBO_ROOT$ inputs (resolved to ../../ paths) pointing to a changed
    /// root file should be detected as affected.
    ///
    /// The `turbo run --affected` path (task_change_detector.rs) iterates
    /// ALL engine tasks and catches this. The query path must do the same.
    #[tokio::test]
    async fn turbo_root_input_in_non_affected_package_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");

        let engine = make_engine(&[
            // lib-a: default inputs (matches files in packages/lib-a/**)
            (a_build.clone(), TaskDefinition::default()),
            // lib-b: has a $TURBO_ROOT$ input that resolved to ../../config.txt
            (
                b_build.clone(),
                TaskDefinition {
                    inputs: TaskInputs {
                        globs: vec!["../../config.txt".to_string()],
                        default: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ),
        ]);

        // Only lib-a is in the affected packages set (a source file changed).
        // lib-b is NOT affected at the package level.
        let mut affected_packages = HashMap::new();
        affected_packages.insert(
            PackageName::from("lib-a"),
            PackageInclusionReason::FileChanged {
                file: AnchoredSystemPathBuf::from_raw("packages/lib-a/src/index.ts").unwrap(),
            },
        );

        // Changed files: a root-level config file AND a file in lib-a.
        let changed_files: HashSet<AnchoredSystemPathBuf> =
            ["config.txt", "packages/lib-a/src/index.ts"]
                .iter()
                .map(|f| AnchoredSystemPathBuf::from_raw(f).unwrap())
                .collect();

        let mock: Arc<dyn QueryRun> = Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, pkg_graph, TurboJson::default()),
            affected_packages,
            changed_files,
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();

        let affected_ids: HashSet<_> = result.iter().map(|at| at.task_id.clone()).collect();

        assert!(
            affected_ids.contains(&query_task_id(&a_build)),
            "lib-a#build should be affected (source file changed)"
        );
        assert!(
            affected_ids.contains(&query_task_id(&b_build)),
            "lib-b#build should be affected ($TURBO_ROOT$ input config.txt changed), but the \
             query path only visited tasks in affected packages and missed it"
        );
    }

    #[tokio::test]
    async fn lockfile_changed_package_seeds_own_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let a_typecheck = TaskId::new("lib-a", "typecheck");
        let b_typecheck = TaskId::new("lib-b", "typecheck");

        let engine = make_engine(&[
            (a_typecheck.clone(), TaskDefinition::default()),
            (b_typecheck.clone(), TaskDefinition::default()),
        ]);

        let mut affected_packages = HashMap::new();
        affected_packages.insert(
            PackageName::from("lib-b"),
            PackageInclusionReason::LockfileChanged {
                added: Vec::new(),
                removed: Vec::new(),
            },
        );

        let changed_files =
            HashSet::from([AnchoredSystemPathBuf::from_raw("pnpm-lock.yaml").unwrap()]);

        let mock: Arc<dyn QueryRun> = Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, pkg_graph, TurboJson::default()),
            affected_packages,
            changed_files,
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();
        let affected_ids: HashSet<_> = result.iter().map(|at| at.task_id.clone()).collect();

        assert!(
            !affected_ids.contains(&query_task_id(&a_typecheck)),
            "lib-a should not be affected by lib-b's lockfile closure change"
        );
        assert!(
            affected_ids.contains(&query_task_id(&b_typecheck)),
            "lib-b#typecheck should be affected when lib-b's lockfile closure changes"
        );

        let b_task = result
            .iter()
            .find(|at| at.task_id == query_task_id(&b_typecheck))
            .unwrap();
        assert!(
            matches!(
                &b_task.reason,
                TaskChangeReason::PackageDependencyChanged { package_name }
                    if package_name == "lib-b"
            ),
            "expected package dependency reason, got {:?}",
            b_task.reason
        );
    }

    #[tokio::test]
    async fn invalid_input_glob_conservatively_changes_all_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;
        let task_id = TaskId::new("lib-a", "build");
        let unaffected_id = TaskId::new("lib-a", "test");
        let engine = make_engine(&[
            (
                task_id.clone(),
                TaskDefinition {
                    inputs: TaskInputs {
                        globs: vec!["[invalid".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ),
            (unaffected_id.clone(), TaskDefinition::default()),
        ]);
        let mock: Arc<dyn QueryRun> = Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, pkg_graph, TurboJson::default()),
            affected_packages: HashMap::new(),
            changed_files: HashSet::new(),
        });

        let affected = calculate_affected_tasks(&mock, None, None).unwrap();
        let affected: HashMap<_, _> = affected
            .into_iter()
            .map(|task| (task.task_id, task.reason))
            .collect();
        assert_eq!(affected.len(), 2);
        for task_id in [task_id, unaffected_id] {
            assert!(matches!(
                affected.get(&query_task_id(&task_id)),
                Some(TaskChangeReason::AllTasksChanged { description })
                    if description == "conservative affectedness fallback"
            ));
        }
    }

    // Query projection coverage uses the same graph and engine seams as the
    // affected-task tests, but supplies native Python facts without running uv.
    struct QueryPythonContributor {
        root: turbopath::AbsoluteSystemPathBuf,
    }

    impl turborepo_repository::toolchain::RepositoryContributor for QueryPythonContributor {
        fn id(&self) -> turborepo_repository::toolchain::ToolchainId {
            turborepo_repository::toolchain::ToolchainId::PYTHON
        }

        fn discover_packages(&self) -> turborepo_repository::toolchain::DiscoverPackagesFuture<'_> {
            use turborepo_repository::{
                native_tasks::{
                    NativeCommandArguments, NativeCommandProgram, NativeTask,
                    WorkingDirectoryPolicy,
                },
                toolchain::{DiscoveredPackage, DiscoveredPackages, WorkspaceRoot},
            };

            fn command(name: &str, arguments: &str) -> NativeTask {
                NativeTask::command_task(
                    name,
                    format!("uv {arguments}"),
                    NativeCommandProgram::Tool("uv".into()),
                    NativeCommandArguments::new(
                        arguments.split_whitespace().map(str::to_string).collect(),
                    ),
                    None,
                    WorkingDirectoryPolicy::RepositoryRoot,
                )
            }

            Box::pin(async move {
                let root = DiscoveredPackage::aggregate(
                    "acme".into(),
                    PackageJson::default(),
                    self.root.join_component("pyproject.toml"),
                )
                .with_native_relationships(vec![])
                .with_native_tasks(vec![
                    command("test", "run --active --frozen --all-packages pytest"),
                    NativeTask::aggregate("lint", ["lint:ruff"]),
                    command(
                        "lint:ruff",
                        "run --active --frozen ruff check packages/py-app packages/py-lib",
                    ),
                    NativeTask::aggregate("check", ["check:mypy"]),
                    command(
                        "check:mypy",
                        "run --active --frozen mypy packages/py-app packages/py-lib",
                    ),
                    command(
                        "format",
                        "run --active --frozen ruff format packages/py-app packages/py-lib",
                    ),
                    command(
                        "format:ruff",
                        "run --active --frozen ruff format packages/py-app packages/py-lib",
                    ),
                ]);
                let app = DiscoveredPackage::package(
                    Some("py-app".into()),
                    PackageJson::default(),
                    self.root
                        .join_components(&["packages", "py-app", "pyproject.toml"]),
                )
                .with_native_relationships(vec![])
                .with_native_tasks(vec![
                    command(
                        "test",
                        "run --active --frozen --package py-app pytest packages/py-app",
                    ),
                    NativeTask::aggregate("lint", ["lint:ruff"]),
                    command(
                        "lint:ruff",
                        "run --active --frozen --package py-app ruff check packages/py-app",
                    ),
                    NativeTask::aggregate("check", ["check:mypy"]),
                    command(
                        "check:mypy",
                        "run --active --frozen --package py-app mypy packages/py-app",
                    ),
                    command(
                        "format",
                        "run --active --frozen --package py-app ruff format packages/py-app",
                    ),
                    command(
                        "format:ruff",
                        "run --active --frozen --package py-app ruff format packages/py-app",
                    ),
                ]);
                let lib = DiscoveredPackage::package(
                    Some("py-lib".into()),
                    PackageJson::default(),
                    self.root
                        .join_components(&["packages", "py-lib", "pyproject.toml"]),
                )
                .with_native_relationships(vec![])
                .with_native_tasks(vec![command("format", "format -- packages/py-lib")]);
                Ok(DiscoveredPackages::new(
                    vec![root, app, lib],
                    vec![WorkspaceRoot::new("python", self.root.clone())],
                ))
            })
        }

        fn discover_package_scopes(
            &self,
        ) -> turborepo_repository::toolchain::DiscoverPackageScopesFuture<'_> {
            Box::pin(async move {
                let observation = self.discover_packages().await?;
                Ok(
                    turborepo_repository::toolchain::DiscoveredPackageScopes::from_full_observation(
                        observation.packages(),
                        observation.workspace_roots(),
                    ),
                )
            })
        }
    }

    #[tokio::test]
    async fn uv_native_query_projects_root_member_and_mixed_js_tasks() {
        use serde_json::{json, Value};

        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let js_path = root.join_components(&["packages", "web", "package.json"]);
        let js = PackageJson {
            name: Some(turborepo_errors::Spanned::new("web".into())),
            scripts: [(
                "lint".into(),
                turborepo_errors::Spanned::new("eslint .".into()),
            )]
            .into(),
            ..Default::default()
        };
        let graph = PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::from([(js_path, js)])))
            .with_contributor(Arc::new(QueryPythonContributor {
                root: root.to_owned(),
            }))
            .build()
            .await
            .unwrap();
        let native_tasks: Vec<_> = ["acme", "py-app"]
            .into_iter()
            .flat_map(|package| {
                [
                    "test",
                    "lint",
                    "lint:ruff",
                    "check",
                    "check:mypy",
                    "format",
                    "format:ruff",
                ]
                .into_iter()
                .map(move |task| (TaskId::new(package, task), TaskDefinition::default()))
            })
            .chain([(TaskId::new("py-lib", "format"), TaskDefinition::default())])
            .collect();
        let edges: Vec<_> = ["acme", "py-app"]
            .into_iter()
            .flat_map(|package| {
                [("lint", "lint:ruff"), ("check", "check:mypy")]
                    .into_iter()
                    .map(move |(parent, child)| {
                        (TaskId::new(package, parent), TaskId::new(package, child))
                    })
            })
            .collect();
        let engine = make_engine_with_edges(&native_tasks, &edges);
        let run: Arc<dyn QueryRun> = Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, graph, TurboJson::default()),
            affected_packages: HashMap::new(),
            changed_files: HashSet::new(),
        });

        for (name, expected) in [
            (
                "acme",
                json!({
                    "test": "uv run --active --frozen --all-packages pytest",
                    "lint": null,
                    "lint:ruff": "uv run --active --frozen ruff check packages/py-app packages/py-lib",
                    "check": null,
                    "check:mypy": "uv run --active --frozen mypy packages/py-app packages/py-lib",
                    "format": "uv run --active --frozen ruff format packages/py-app packages/py-lib",
                    "format:ruff": "uv run --active --frozen ruff format packages/py-app packages/py-lib"
                }),
            ),
            (
                "py-app",
                json!({
                    "test": "uv run --active --frozen --package py-app pytest packages/py-app",
                    "lint": null,
                    "lint:ruff": "uv run --active --frozen --package py-app ruff check packages/py-app",
                    "check": null,
                    "check:mypy": "uv run --active --frozen --package py-app mypy packages/py-app",
                    "format": "uv run --active --frozen --package py-app ruff format packages/py-app",
                    "format:ruff": "uv run --active --frozen --package py-app ruff format packages/py-app"
                }),
            ),
            ("py-lib", json!({"format": "uv format -- packages/py-lib"})),
            ("web", json!({"lint": "eslint ."})),
        ] {
            let data = query_data(
                run.clone(),
                &format!(
                    "{{ package(name: \"{name}\") {{ tasks {{ items {{ name command script \
                     directDependencies {{ items {{ fullName }} }} }} }} }} }}"
                ),
            )
            .await;
            let tasks = data["package"]["tasks"]["items"].as_array().unwrap();
            let commands: Value = tasks
                .iter()
                .map(|task| {
                    (
                        task["name"].as_str().unwrap().to_string(),
                        task["command"].clone(),
                    )
                })
                .collect::<serde_json::Map<_, _>>()
                .into();
            assert_eq!(commands, expected, "package: {name}");
            for task in tasks {
                let task_name = task["name"].as_str().unwrap();
                if name == "web" && task_name == "lint" {
                    assert_eq!(task["script"], "eslint .");
                } else {
                    assert!(task["script"].is_null(), "{name}#{task_name}");
                }
                let dependencies = &task["directDependencies"]["items"];
                let expected_child = match task_name {
                    "lint" if name != "web" => Some(format!("{name}#lint:ruff")),
                    "check" => Some(format!("{name}#check:mypy")),
                    _ => None,
                };
                assert_eq!(
                    dependencies,
                    &json!(expected_child
                        .into_iter()
                        .map(|full_name| json!({"fullName": full_name}))
                        .collect::<Vec<_>>()),
                    "{name}#{task_name}"
                );
            }
        }
    }

    #[tokio::test]
    async fn directly_affected_task_propagates_to_task_dependents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "app-a"]).await;

        let lib_build = TaskId::new("lib-a", "build");
        let app_test = TaskId::new("app-a", "test");
        let app_lint = TaskId::new("app-a", "lint");

        let engine = make_engine_with_edges(
            &[
                (lib_build.clone(), TaskDefinition::default()),
                (app_test.clone(), TaskDefinition::default()),
                (app_lint.clone(), TaskDefinition::default()),
            ],
            &[(app_test.clone(), lib_build.clone())],
        );

        let mut affected_packages = HashMap::new();
        affected_packages.insert(
            PackageName::from("lib-a"),
            PackageInclusionReason::FileChanged {
                file: AnchoredSystemPathBuf::from_raw("packages/lib-a/index.ts").unwrap(),
            },
        );
        let changed_files =
            HashSet::from([AnchoredSystemPathBuf::from_raw("packages/lib-a/index.ts").unwrap()]);

        let mock: Arc<dyn QueryRun> = Arc::new(MockQueryRun {
            recorded_calls: Default::default(),
            engine,
            repo_context: make_repo_context(root, pkg_graph, TurboJson::default()),
            affected_packages,
            changed_files,
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();
        let reasons: HashMap<_, _> = result
            .iter()
            .map(|task| (task.task_id.clone(), &task.reason))
            .collect();

        assert!(matches!(
            reasons.get(&query_task_id(&lib_build)),
            Some(TaskChangeReason::FileChanged { file_path })
                if file_path == "packages/lib-a/index.ts"
        ));
        assert!(matches!(
            reasons.get(&query_task_id(&app_test)),
            Some(TaskChangeReason::DependencyTaskChanged {
                task_name,
                package_name,
            }) if task_name == "build" && package_name == "lib-a"
        ));
        assert!(
            !reasons.contains_key(&query_task_id(&app_lint)),
            "unrelated app task should not be affected: {reasons:?}"
        );
    }
}
