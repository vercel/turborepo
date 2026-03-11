use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use petgraph::Direction;
use turbopath::AnchoredSystemPathBuf;
use turborepo_engine::TaskNode;
use turborepo_repository::change_mapper::{AllPackageChangeReason, PackageInclusionReason};
use turborepo_task_id::TaskId;
use turborepo_types::task_input_matching::{compile_globs, file_matches_compiled_inputs};

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
    // changed files.
    let mut affected: HashMap<TaskId<'static>, TaskChangeReason> = HashMap::new();

    for (pkg_name, reason) in &affected_packages {
        let Some(pkg_info) = pkg_dep_graph.package_info(pkg_name) else {
            continue;
        };
        let pkg_path = pkg_info.package_path();
        let pkg_unix_path = pkg_path.to_unix();
        let pkg_prefix = pkg_unix_path.to_string();

        // TODO: This pre-filters changed files to only those within the package
        // directory, which means cross-package inputs from $TURBO_ROOT$ expansion
        // (e.g. "../../jest.config.js") won't match files outside the package.
        // The `turbo run --affected` path in task_change_detector.rs correctly
        // checks all changed files against all tasks. These two paths should be
        // unified so `turbo query { affectedTasks }` and `turbo run --affected`
        // produce consistent results.
        let pkg_changed_files: Vec<&AnchoredSystemPathBuf> = changed_files
            .iter()
            .filter(|f| {
                let file_str = f.to_unix().to_string();
                if pkg_prefix.is_empty() {
                    // Root package — all files are potentially relevant.
                    // The task's input globs will filter further.
                    true
                } else {
                    file_str.starts_with(&format!("{pkg_prefix}/"))
                }
            })
            .collect();

        // For each task this package has in the engine
        for task_id in engine.task_ids() {
            if task_id.package() != pkg_name.as_str() {
                continue;
            }

            if affected.contains_key(task_id) {
                continue;
            }

            // Check all changed files in this package against the task's input
            // globs. This handles all PackageInclusionReason variants uniformly:
            // regardless of why the package was included, a task is only
            // directly affected if its inputs actually changed.
            if let Some(def) = engine.task_definition(task_id) {
                let compiled = compile_globs(&def.inputs);
                for file in &pkg_changed_files {
                    if file_matches_compiled_inputs(file, &pkg_unix_path, &compiled) {
                        affected.insert(
                            task_id.clone(),
                            TaskChangeReason::FileChanged {
                                file_path: file.to_string(),
                            },
                        );
                        break;
                    }
                }
            } else {
                // No task definition — conservatively mark as affected
                affected.insert(task_id.clone(), reason_from_package_reason(reason));
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

fn reason_from_package_reason(reason: &PackageInclusionReason) -> TaskChangeReason {
    match reason {
        PackageInclusionReason::DependencyChanged { dependency } => {
            TaskChangeReason::PackageDependencyChanged {
                package_name: dependency.to_string(),
            }
        }
        PackageInclusionReason::DependentChanged { dependent } => {
            TaskChangeReason::PackageDependencyChanged {
                package_name: dependent.to_string(),
            }
        }
        PackageInclusionReason::LockfileChanged { .. } => TaskChangeReason::AllTasksChanged {
            description: "lockfile changed".to_string(),
        },
        PackageInclusionReason::ConservativeRootLockfileChanged => {
            TaskChangeReason::AllTasksChanged {
                description: "root lockfile changed".to_string(),
            }
        }
        PackageInclusionReason::FileChanged { file } => TaskChangeReason::FileChanged {
            file_path: file.to_string(),
        },
        PackageInclusionReason::InFilteredDirectory { directory } => {
            TaskChangeReason::AllTasksChanged {
                description: format!("in filtered directory: {directory}"),
            }
        }
        PackageInclusionReason::IncludedByFilter { filters } => TaskChangeReason::AllTasksChanged {
            description: format!("included by filter: {}", filters.join(", ")),
        },
        PackageInclusionReason::RootTask { task } => TaskChangeReason::AllTasksChanged {
            description: format!("root task: {task}"),
        },
        PackageInclusionReason::All(_) => {
            unreachable!("All case handled separately")
        }
    }
}
