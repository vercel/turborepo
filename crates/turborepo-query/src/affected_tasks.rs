use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use petgraph::Direction;
use turborepo_engine::TaskNode;
use turborepo_repository::{
    change_mapper::{AllPackageChangeReason, PackageInclusionReason},
    package_graph::PackageName,
};
use turborepo_task_id::TaskId;

use crate::{Error, QueryRun};

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
    pub task_id: TaskId<'static>,
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

    let engine = run.engine();

    if let Some(all_reason) = all_packages_reason {
        // Every task in the engine is affected
        let description = match &all_reason {
            AllPackageChangeReason::GlobalDepsChanged { file } => {
                return Ok(engine
                    .task_ids()
                    .map(|task_id| AffectedTask {
                        task_id: task_id.clone(),
                        reason: TaskChangeReason::GlobalDepsChanged {
                            file_path: file.to_string(),
                        },
                    })
                    .collect());
            }
            AllPackageChangeReason::DefaultGlobalFileChanged { file } => {
                return Ok(engine
                    .task_ids()
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

        return Ok(engine
            .task_ids()
            .map(|task_id| AffectedTask {
                task_id: task_id.clone(),
                reason: TaskChangeReason::AllTasksChanged {
                    description: description.clone(),
                },
            })
            .collect());
    }

    // Get the raw changed files for input-level matching
    let changed_files = run.changed_files(base.as_deref(), head.as_deref())?;

    let pkg_dep_graph = run.pkg_dep_graph();

    // Phase 1: Direct task affectedness — check each task's inputs against
    // changed files. Uses the shared matching function that iterates ALL
    // engine tasks regardless of package, so tasks with $TURBO_ROOT$ inputs
    // in non-affected packages are correctly detected.
    let matched = match turborepo_engine::match_tasks_against_changed_files(
        engine,
        pkg_dep_graph,
        &changed_files,
    ) {
        Ok(matched) => matched,
        Err(error) => {
            tracing::error!("failed to determine affected tasks: {error}");
            return Ok(engine
                .task_ids()
                .map(|task_id| AffectedTask {
                    task_id: task_id.clone(),
                    reason: TaskChangeReason::AllTasksChanged {
                        description: "conservative affectedness fallback".to_string(),
                    },
                })
                .collect());
        }
    };
    let mut affected: HashMap<TaskId<'static>, TaskChangeReason> = matched
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
        for task_id in engine.task_ids() {
            if lockfile_changed_packages.contains(task_id.package()) {
                affected.entry(task_id.clone()).or_insert_with(|| {
                    TaskChangeReason::PackageDependencyChanged {
                        package_name: task_id.package().to_string(),
                    }
                });
            }
        }
    }

    // Phase 2: Propagate through the task dependency graph via BFS.
    // If task B depends on task A and A is affected, B is also affected.
    // Single-pass BFS from seed tasks in the Incoming direction is O(V + E).
    let task_graph = engine.task_graph();
    let task_lookup = engine.task_lookup();

    let mut affected_indices: HashSet<petgraph::graph::NodeIndex> =
        HashSet::with_capacity(affected.len());
    let mut queue: VecDeque<petgraph::graph::NodeIndex> = VecDeque::with_capacity(affected.len());

    for task_id in affected.keys() {
        if let Some(&idx) = task_lookup.get(task_id) {
            affected_indices.insert(idx);
            queue.push_back(idx);
        }
    }

    while let Some(idx) = queue.pop_front() {
        // Incoming neighbors = tasks that depend on this task
        for dependent_idx in task_graph.neighbors_directed(idx, Direction::Incoming) {
            if !affected_indices.insert(dependent_idx) {
                continue;
            }
            queue.push_back(dependent_idx);

            if let (Some(TaskNode::Task(dependent_id)), Some(TaskNode::Task(cause_id))) = (
                task_graph.node_weight(dependent_idx),
                task_graph.node_weight(idx),
            ) {
                affected.insert(
                    dependent_id.clone(),
                    TaskChangeReason::DependencyTaskChanged {
                        task_name: cause_id.task().to_string(),
                        package_name: cause_id.package().to_string(),
                    },
                );
            }
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

    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_engine::Building;
    use turborepo_query_api::{AffectedPackagesError, BoundariesFuture};
    use turborepo_repository::{
        change_mapper::PackageInclusionReason,
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_graph::{PackageGraph, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_scm::SCM;
    use turborepo_task_id::TaskId;
    use turborepo_turbo_json::TurboJson;
    use turborepo_types::{TaskDefinition, TaskInputs};

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

    struct MockQueryRun {
        engine: turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition>,
        pkg_dep_graph: PackageGraph,
        affected_packages: HashMap<PackageName, PackageInclusionReason>,
        changed_files: HashSet<AnchoredSystemPathBuf>,
        repo_root: AbsoluteSystemPathBuf,
        root_turbo_json: TurboJson,
    }

    impl QueryRun for MockQueryRun {
        fn version(&self) -> &'static str {
            "test"
        }

        fn repo_root(&self) -> &AbsoluteSystemPath {
            &self.repo_root
        }

        fn pkg_dep_graph(&self) -> &PackageGraph {
            &self.pkg_dep_graph
        }

        fn engine(&self) -> &turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition> {
            &self.engine
        }

        fn scm(&self) -> &SCM {
            unimplemented!("not needed for affected_tasks tests")
        }

        fn root_turbo_json(&self) -> &TurboJson {
            &self.root_turbo_json
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
            engine,
            pkg_dep_graph,
            affected_packages,
            changed_files: files
                .iter()
                .map(|file| AnchoredSystemPathBuf::from_raw(file).unwrap())
                .collect(),
            repo_root: root.to_owned(),
            root_turbo_json,
        })
    }

    async fn query_data(run: Arc<dyn QueryRun>, query: &str) -> serde_json::Value {
        let result = crate::execute_query(run, query, None).await.unwrap();
        let result: serde_json::Value = serde_json::from_str(&result.result_json).unwrap();
        assert!(result.get("errors").is_none(), "{result}");
        result["data"].clone()
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
            engine,
            pkg_dep_graph: pkg_graph,
            affected_packages,
            changed_files,
            repo_root: root.to_owned(),
            root_turbo_json: TurboJson::default(),
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();

        let affected_ids: HashSet<_> = result.iter().map(|at| at.task_id.clone()).collect();

        assert!(
            affected_ids.contains(&a_build),
            "lib-a#build should be affected (source file changed)"
        );
        assert!(
            affected_ids.contains(&b_build),
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
            engine,
            pkg_dep_graph: pkg_graph,
            affected_packages,
            changed_files,
            repo_root: root.to_owned(),
            root_turbo_json: TurboJson::default(),
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();
        let affected_ids: HashSet<_> = result.iter().map(|at| at.task_id.clone()).collect();

        assert!(
            !affected_ids.contains(&a_typecheck),
            "lib-a should not be affected by lib-b's lockfile closure change"
        );
        assert!(
            affected_ids.contains(&b_typecheck),
            "lib-b#typecheck should be affected when lib-b's lockfile closure changes"
        );

        let b_task = result.iter().find(|at| at.task_id == b_typecheck).unwrap();
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
            engine,
            pkg_dep_graph: pkg_graph,
            affected_packages: HashMap::new(),
            changed_files: HashSet::new(),
            repo_root: root.to_owned(),
            root_turbo_json: TurboJson::default(),
        });

        let affected = calculate_affected_tasks(&mock, None, None).unwrap();
        let affected: HashMap<_, _> = affected
            .into_iter()
            .map(|task| (task.task_id, task.reason))
            .collect();
        assert_eq!(affected.len(), 2);
        for task_id in [task_id, unaffected_id] {
            assert!(matches!(
                affected.get(&task_id),
                Some(TaskChangeReason::AllTasksChanged { description })
                    if description == "conservative affectedness fallback"
            ));
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
            engine,
            pkg_dep_graph: pkg_graph,
            affected_packages,
            changed_files,
            repo_root: root.to_owned(),
            root_turbo_json: TurboJson::default(),
        });

        let result = calculate_affected_tasks(&mock, None, None).unwrap();
        let reasons: HashMap<_, _> = result
            .iter()
            .map(|task| (task.task_id.clone(), &task.reason))
            .collect();

        assert!(matches!(
            reasons.get(&lib_build),
            Some(TaskChangeReason::FileChanged { file_path })
                if file_path == "packages/lib-a/index.ts"
        ));
        assert!(matches!(
            reasons.get(&app_test),
            Some(TaskChangeReason::DependencyTaskChanged {
                task_name,
                package_name,
            }) if task_name == "build" && package_name == "lib-a"
        ));
        assert!(
            !reasons.contains_key(&app_lint),
            "unrelated app task should not be affected: {reasons:?}"
        );
    }
}
