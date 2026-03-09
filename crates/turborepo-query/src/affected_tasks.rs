use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use petgraph::Direction;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_engine::TaskNode;
use turborepo_repository::change_mapper::{AllPackageChangeReason, PackageInclusionReason};
use turborepo_task_id::TaskId;
// Program trait provides is_match() on wax::Glob
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

/// Pre-compiled glob patterns for efficient matching against many files.
struct CompiledGlobs {
    inclusions: Vec<wax::Glob<'static>>,
    exclusions: Vec<wax::Glob<'static>>,
    default: bool,
    has_traversal_globs: bool,
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

        // Get files that changed within this package's directory
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

/// Pre-compiles a task's input globs for efficient matching against many files.
fn compile_globs(inputs: &turborepo_types::TaskInputs) -> CompiledGlobs {
    let mut inclusions = Vec::new();
    let mut exclusions = Vec::new();
    let mut has_traversal_globs = false;

    for glob_str in &inputs.globs {
        if let Some(stripped) = glob_str.strip_prefix('!') {
            if stripped.starts_with("../") {
                has_traversal_globs = true;
            }
            if let Ok(glob) = wax::Glob::new(stripped) {
                exclusions.push(glob.into_owned());
            }
        } else {
            if glob_str.starts_with("../") {
                has_traversal_globs = true;
            }
            if let Ok(glob) = wax::Glob::new(glob_str) {
                inclusions.push(glob.into_owned());
            }
        }
    }

    CompiledGlobs {
        inclusions,
        exclusions,
        default: inputs.default,
        has_traversal_globs,
    }
}

/// Checks whether a changed file matches pre-compiled task input globs.
fn file_matches_compiled_inputs(
    file: &AnchoredSystemPathBuf,
    package_unix_path: &RelativeUnixPathBuf,
    compiled: &CompiledGlobs,
) -> bool {
    let file_unix = file.to_unix().to_string();
    let pkg_prefix = package_unix_path.to_string();

    let file_relative_to_pkg = if pkg_prefix.is_empty() {
        file_unix.clone()
    } else if let Some(stripped) = file_unix.strip_prefix(&format!("{pkg_prefix}/")) {
        stripped.to_string()
    } else {
        String::new()
    };

    // For files outside the package dir, only check if there are globs
    // with path traversal (../) that could reach outside the package.
    // These come from $TURBO_ROOT$ references resolved to relative paths
    // like "../../jest.config.js". Without such globs, a file in a sibling
    // package should never match.
    if file_relative_to_pkg.is_empty() && !pkg_prefix.is_empty() {
        if !compiled.has_traversal_globs {
            return false;
        }

        let depth = pkg_prefix.matches('/').count() + 1;
        let mut relative = String::new();
        for _ in 0..depth {
            relative.push_str("../");
        }
        relative.push_str(&file_unix);

        return check_compiled_globs(
            &relative,
            &compiled.inclusions,
            &compiled.exclusions,
            compiled.default,
        );
    }

    check_compiled_globs(
        &file_relative_to_pkg,
        &compiled.inclusions,
        &compiled.exclusions,
        compiled.default,
    )
}

fn check_compiled_globs(
    file_path: &str,
    inclusions: &[wax::Glob<'static>],
    exclusions: &[wax::Glob<'static>],
    default: bool,
) -> bool {
    for pattern in exclusions {
        if pattern.is_match(file_path) {
            return false;
        }
    }

    if default {
        return true;
    }

    if inclusions.is_empty() && exclusions.is_empty() {
        return true;
    }

    for pattern in inclusions {
        if pattern.is_match(file_path) {
            return true;
        }
    }

    false
}

/// Checks whether a changed file matches a task's input configuration.
///
/// This compiles globs on each call — use `file_matches_compiled_inputs` with
/// `compile_globs` in hot loops to avoid repeated compilation.
///
/// Note: This is intentionally a separate implementation from the input
/// matching in `turborepo-task-hash`. That system walks the filesystem for
/// cache hashing. Here we check a pre-computed set of changed files from SCM,
/// which only needs to know whether *any* changed file matches.
#[cfg(test)]
fn file_matches_task_inputs(
    file: &AnchoredSystemPathBuf,
    package_unix_path: &RelativeUnixPathBuf,
    inputs: &turborepo_types::TaskInputs,
) -> bool {
    let compiled = compile_globs(inputs);
    file_matches_compiled_inputs(file, package_unix_path, &compiled)
}

#[cfg(test)]
fn check_file_against_globs(
    file_path: &str,
    inclusions: &[&str],
    exclusions: &[&str],
    default: bool,
) -> bool {
    for pattern in exclusions {
        if glob_matches(pattern, file_path) {
            return false;
        }
    }

    if default {
        return true;
    }

    if inclusions.is_empty() && exclusions.is_empty() {
        return true;
    }

    for pattern in inclusions {
        if glob_matches(pattern, file_path) {
            return true;
        }
    }

    false
}

#[cfg(test)]
fn partition_globs(globs: &[String]) -> (Vec<&str>, Vec<&str>) {
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

#[cfg(test)]
fn glob_matches(pattern: &str, path: &str) -> bool {
    wax::Glob::new(pattern)
        .map(|glob| glob.is_match(path))
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
    use turborepo_types::TaskInputs;

    use super::{
        check_file_against_globs, file_matches_task_inputs, glob_matches, partition_globs,
    };

    // ── glob_matches ──

    #[test]
    fn glob_matches_simple_extension() {
        assert!(glob_matches("**/*.ts", "src/index.ts"));
        assert!(glob_matches("**/*.ts", "deeply/nested/file.ts"));
        assert!(!glob_matches("**/*.ts", "src/index.js"));
    }

    #[test]
    fn glob_matches_exact_file() {
        assert!(glob_matches("README.md", "README.md"));
        assert!(!glob_matches("README.md", "src/README.md"));
    }

    #[test]
    fn glob_matches_double_star() {
        assert!(glob_matches("**/*.md", "README.md"));
        assert!(glob_matches("**/*.md", "docs/guide.md"));
        assert!(!glob_matches("**/*.md", "src/index.ts"));
    }

    #[test]
    fn glob_matches_invalid_pattern_returns_false() {
        // Invalid glob patterns should not panic, just return false
        assert!(!glob_matches("[invalid", "anything"));
    }

    // ── partition_globs ──

    #[test]
    fn partition_splits_inclusions_and_exclusions() {
        let globs = vec![
            "**/*.ts".to_string(),
            "!**/*.test.ts".to_string(),
            "!**/*.md".to_string(),
            "src/**".to_string(),
        ];
        let (inc, exc) = partition_globs(&globs);
        assert_eq!(inc, vec!["**/*.ts", "src/**"]);
        assert_eq!(exc, vec!["**/*.test.ts", "**/*.md"]);
    }

    #[test]
    fn partition_empty_input() {
        let globs: Vec<String> = vec![];
        let (inc, exc) = partition_globs(&globs);
        assert!(inc.is_empty());
        assert!(exc.is_empty());
    }

    // ── check_file_against_globs ──

    #[test]
    fn default_true_includes_all_files() {
        assert!(check_file_against_globs("src/index.ts", &[], &[], true));
        assert!(check_file_against_globs("README.md", &[], &[], true));
    }

    #[test]
    fn default_true_respects_exclusions() {
        assert!(!check_file_against_globs(
            "README.md",
            &[],
            &["**/*.md"],
            true
        ));
        assert!(check_file_against_globs(
            "src/index.ts",
            &[],
            &["**/*.md"],
            true
        ));
    }

    #[test]
    fn default_true_with_multiple_exclusions() {
        let exc = &["**/*.md", "**/*.test.ts"];
        assert!(!check_file_against_globs("README.md", &[], exc, true));
        assert!(!check_file_against_globs("src/foo.test.ts", &[], exc, true));
        assert!(check_file_against_globs("src/foo.ts", &[], exc, true));
    }

    #[test]
    fn no_default_no_globs_means_all_inputs() {
        // No $TURBO_DEFAULT$ and no explicit globs means the task had no inputs
        // configuration at all — treat everything as an input.
        assert!(check_file_against_globs("anything.ts", &[], &[], false));
    }

    #[test]
    fn no_default_with_inclusions() {
        let inc = &["src/**/*.ts"];
        assert!(check_file_against_globs("src/index.ts", inc, &[], false));
        assert!(!check_file_against_globs("README.md", inc, &[], false));
    }

    #[test]
    fn no_default_inclusion_with_exclusion() {
        let inc = &["**/*.ts"];
        let exc = &["**/*.test.ts"];
        assert!(check_file_against_globs("src/index.ts", inc, exc, false));
        assert!(!check_file_against_globs(
            "src/index.test.ts",
            inc,
            exc,
            false
        ));
    }

    // ── file_matches_task_inputs ──

    fn anchored(s: &str) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf::from_raw(s).unwrap()
    }

    fn pkg_path(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s.to_string()).unwrap()
    }

    #[test]
    fn file_in_package_with_default_inputs() {
        let file = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        assert!(file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn file_in_package_excluded_by_glob() {
        let file = anchored("packages/lib-a/README.md");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string()],
            default: true,
        };
        assert!(!file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn file_in_package_not_excluded() {
        let file = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string()],
            default: true,
        };
        assert!(file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn file_outside_package_no_match() {
        // A file in a sibling package should not match unless there's a
        // $TURBO_ROOT$ glob that traverses up
        let file = anchored("packages/lib-b/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        // File is not inside packages/lib-a, so it doesn't match
        assert!(!file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn turbo_root_glob_matches_root_file() {
        // When turbo.json has $TURBO_ROOT$/jest.config.js as a task input,
        // it gets resolved to ../../jest.config.js for a package at depth 2.
        let file = anchored("jest.config.js");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["../../jest.config.js".to_string()],
            default: true,
        };
        assert!(file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn no_inputs_config_matches_everything() {
        // Default TaskInputs (no inputs key in turbo.json)
        let file = anchored("packages/lib-a/anything.txt");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs::default();
        assert!(file_matches_task_inputs(&file, &pkg, &inputs));
    }

    #[test]
    fn multiple_exclusions_all_respected() {
        let file_md = anchored("packages/lib-a/README.md");
        let file_test = anchored("packages/lib-a/foo.test.ts");
        let file_src = anchored("packages/lib-a/src/index.ts");
        let pkg = pkg_path("packages/lib-a");
        let inputs = TaskInputs {
            globs: vec!["!**/*.md".to_string(), "!**/*.test.ts".to_string()],
            default: true,
        };
        assert!(!file_matches_task_inputs(&file_md, &pkg, &inputs));
        assert!(!file_matches_task_inputs(&file_test, &pkg, &inputs));
        assert!(file_matches_task_inputs(&file_src, &pkg, &inputs));
    }

    #[test]
    fn root_package_file_matches() {
        let file = anchored("scripts/check.sh");
        let pkg = pkg_path("");
        let inputs = TaskInputs {
            globs: vec![],
            default: true,
        };
        assert!(file_matches_task_inputs(&file, &pkg, &inputs));
    }
}
