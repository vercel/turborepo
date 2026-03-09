use std::{collections::HashMap, sync::Arc};

use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_engine::TaskNode;
use turborepo_repository::change_mapper::{AllPackageChangeReason, PackageInclusionReason};
use turborepo_task_id::TaskId;
use wax::Program;

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
/// This walks the task graph (not just the package graph) and checks each
/// task's specific `inputs` configuration against the changed files. A task
/// is only reported as affected if its inputs actually changed, or if one
/// of its upstream task dependencies is affected.
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

    // Phase 1: Direct task affectedness — check each task's inputs against changed
    // files
    let mut affected: HashMap<TaskId<'static>, TaskChangeReason> = HashMap::new();

    for (pkg_name, reason) in &affected_packages {
        let Some(pkg_info) = pkg_dep_graph.package_info(pkg_name) else {
            continue;
        };
        let pkg_path = pkg_info.package_path();
        let pkg_unix_path = pkg_path.to_unix();

        // Get files that changed within this package's directory
        let pkg_changed_files: Vec<&AnchoredSystemPathBuf> = changed_files
            .iter()
            .filter(|f| {
                let file_str = f.to_unix().to_string();
                let pkg_str = pkg_unix_path.to_string();
                if pkg_str.is_empty() {
                    // Root package — all non-package files are relevant.
                    // The engine only includes tasks actually in the graph,
                    // so this is safe.
                    true
                } else {
                    file_str.starts_with(&format!("{pkg_str}/"))
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

            let task_def = engine.task_definition(task_id);

            match reason {
                PackageInclusionReason::FileChanged { file } => {
                    if let Some(def) = task_def {
                        if file_matches_task_inputs(file, &pkg_unix_path, &def.inputs) {
                            affected.insert(
                                task_id.clone(),
                                TaskChangeReason::FileChanged {
                                    file_path: file.to_string(),
                                },
                            );
                        }
                    } else {
                        // No task definition means we can't check inputs; assume affected
                        affected.insert(
                            task_id.clone(),
                            TaskChangeReason::FileChanged {
                                file_path: file.to_string(),
                            },
                        );
                    }
                }
                PackageInclusionReason::DependencyChanged { .. }
                | PackageInclusionReason::DependentChanged { .. }
                | PackageInclusionReason::LockfileChanged { .. }
                | PackageInclusionReason::ConservativeRootLockfileChanged
                | PackageInclusionReason::InFilteredDirectory { .. }
                | PackageInclusionReason::IncludedByFilter { .. }
                | PackageInclusionReason::RootTask { .. } => {
                    // These reasons mean the package is affected but we need to check
                    // if this specific task's inputs actually changed by looking at
                    // all changed files for this package.
                    if let Some(def) = task_def {
                        for file in &pkg_changed_files {
                            if file_matches_task_inputs(file, &pkg_unix_path, &def.inputs) {
                                affected
                                    .insert(task_id.clone(), reason_from_package_reason(reason));
                                break;
                            }
                        }
                    } else {
                        // No task definition — conservatively mark as affected
                        affected.insert(task_id.clone(), reason_from_package_reason(reason));
                    }
                }
                PackageInclusionReason::All(_) => {
                    unreachable!("all-packages case handled above")
                }
            }
        }
    }

    // Phase 2: Propagate through the task dependency graph.
    // If task A depends on task B and B is affected, A is also affected.
    // We do a reverse topological walk so dependencies are processed before
    // dependents.
    let mut propagated: HashMap<TaskId<'static>, TaskChangeReason> = HashMap::new();
    propagated.extend(affected.iter().map(|(k, v)| (k.clone(), v.clone())));

    // Keep propagating until stable
    let mut changed = true;
    while changed {
        changed = false;
        for task_id in engine.task_ids() {
            if propagated.contains_key(task_id) {
                continue;
            }

            // Check if any dependency of this task is affected
            if let Some(deps) = engine.dependencies(task_id) {
                for dep in deps {
                    if let TaskNode::Task(dep_id) = dep {
                        if propagated.contains_key(dep_id) {
                            propagated.insert(
                                task_id.clone(),
                                TaskChangeReason::DependencyTaskChanged {
                                    task_name: dep_id.task().to_string(),
                                    package_name: dep_id.package().to_string(),
                                },
                            );
                            changed = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(propagated
        .into_iter()
        .map(|(task_id, reason)| AffectedTask { task_id, reason })
        .collect())
}

/// Checks whether a changed file matches a task's input configuration.
///
/// The file path is repo-root-relative (an `AnchoredSystemPathBuf`).
/// The task inputs have globs that are package-relative, with `$TURBO_ROOT$`
/// references already resolved to relative paths like `../../jest.config.js`.
fn file_matches_task_inputs(
    file: &AnchoredSystemPathBuf,
    package_unix_path: &RelativeUnixPathBuf,
    inputs: &turborepo_types::TaskInputs,
) -> bool {
    let file_unix = file.to_unix().to_string();
    let pkg_prefix = package_unix_path.to_string();

    // Make file relative to the package directory for matching
    let file_relative_to_pkg = if pkg_prefix.is_empty() {
        file_unix.clone()
    } else if let Some(stripped) = file_unix.strip_prefix(&format!("{pkg_prefix}/")) {
        stripped.to_string()
    } else {
        // File is not inside this package directory. It could still match
        // a $TURBO_ROOT$ glob that traverses up (e.g., "../../jest.config.js").
        // We'll check against the full package-relative path interpretation.
        // For $TURBO_ROOT$ globs, the resolved path is relative to the package dir
        // (e.g., "../../jest.config.js"). The file is relative to repo root.
        // We need to check if any glob matches the file when interpreted from
        // the package's perspective.
        //
        // Construct the relative path from the package to this file:
        // If package is at "services/my-svc" and file is "jest.config.js",
        // then relative path from package to file is "../../jest.config.js".
        // This matches "$TURBO_ROOT$/jest.config.js" which resolves to
        // "../../jest.config.js" for a package at depth 2.
        //
        // For simplicity and correctness, we check all globs against both
        // the package-relative path and a constructed relative path.
        String::new()
    };

    let (inclusions, exclusions) = partition_globs(&inputs.globs);

    // For files outside the package dir, check if any glob with path
    // traversal (../) could match
    if file_relative_to_pkg.is_empty() && !pkg_prefix.is_empty() {
        // Build relative path from package to the file
        let depth = pkg_prefix.matches('/').count() + 1;
        let mut relative = String::new();
        for _ in 0..depth {
            relative.push_str("../");
        }
        relative.push_str(&file_unix);

        return check_file_against_globs(&relative, &inclusions, &exclusions, inputs.default);
    }

    check_file_against_globs(
        &file_relative_to_pkg,
        &inclusions,
        &exclusions,
        inputs.default,
    )
}

fn check_file_against_globs(
    file_path: &str,
    inclusions: &[&str],
    exclusions: &[&str],
    default: bool,
) -> bool {
    // Check exclusions first — if file matches any exclusion, it's not an input
    for pattern in exclusions {
        if glob_matches(pattern, file_path) {
            return false;
        }
    }

    if default {
        // $TURBO_DEFAULT$ means "all git-tracked files" are included.
        // If we got past exclusions, the file is an input.
        return true;
    }

    // No $TURBO_DEFAULT$ — but if there are no globs at all, the task has
    // no inputs configuration, which means "all files are inputs" (the
    // default behavior when no `inputs` key is in turbo.json).
    if inclusions.is_empty() && exclusions.is_empty() {
        return true;
    }

    // Explicit inclusion globs — file must match one of them
    for pattern in inclusions {
        if glob_matches(pattern, file_path) {
            return true;
        }
    }

    false
}

fn partition_globs<'a>(globs: &'a [String]) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut inclusions = Vec::new();
    let mut exclusions = Vec::new();
    for glob in globs {
        if let Some(stripped) = glob.strip_prefix('!') {
            exclusions.push(stripped);
        } else {
            inclusions.push(glob.as_str());
        }
    }
    (inclusions, exclusions)
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    // Use wax for glob matching. If the pattern fails to compile,
    // conservatively return false (don't match).
    wax::Glob::new(pattern)
        .map(|glob| glob.is_match(path))
        .unwrap_or(false)
}

fn reason_from_package_reason(reason: &PackageInclusionReason) -> TaskChangeReason {
    match reason {
        PackageInclusionReason::DependencyChanged { dependency } => {
            TaskChangeReason::DependencyTaskChanged {
                task_name: String::new(),
                package_name: dependency.to_string(),
            }
        }
        PackageInclusionReason::DependentChanged { dependent } => {
            TaskChangeReason::DependencyTaskChanged {
                task_name: String::new(),
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
