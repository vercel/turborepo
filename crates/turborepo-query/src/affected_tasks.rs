use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use petgraph::Direction;
use turborepo_engine::TaskNode;
use turborepo_repository::change_mapper::{AllPackageChangeReason, PackageInclusionReason};
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
    /// A package-level dependency changed. Unlike `DependencyTaskChanged`,
    /// there is no specific upstream task — the package graph edge triggered
    /// this.
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
/// 2. **Direct input matching**: For each affected package, each task's
///    `inputs` globs are checked against the changed files. Only tasks whose
///    inputs actually match a changed file are marked affected. Tasks without a
///    definition are conservatively included.
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

    if affected_packages.is_empty() {
        return Ok(Vec::new());
    }

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
    let matched =
        turborepo_engine::match_tasks_against_changed_files(engine, pkg_dep_graph, &changed_files);
    let mut affected: HashMap<TaskId<'static>, TaskChangeReason> = matched
        .into_iter()
        .map(|(task_id, file_path)| (task_id, TaskChangeReason::FileChanged { file_path }))
        .collect();

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
        let mut engine: turborepo_engine::Engine<Building, TaskDefinition> =
            turborepo_engine::Engine::new();
        for (task_id, def) in tasks {
            engine.get_index(task_id);
            engine.add_definition(task_id.clone(), def.clone());
        }
        engine.seal()
    }

    struct MockQueryRun {
        engine: turborepo_engine::Engine<turborepo_engine::Built, TaskDefinition>,
        pkg_dep_graph: PackageGraph,
        affected_packages: HashMap<PackageName, PackageInclusionReason>,
        changed_files: HashSet<AnchoredSystemPathBuf>,
        #[allow(dead_code)]
        repo_root: AbsoluteSystemPathBuf,
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
            unimplemented!("not needed for affected_tasks tests")
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
}
