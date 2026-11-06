//! Task-level affected detection for `--affected` with the
//! `affectedUsingTaskInputs` future flag and `turbo watch` with
//! `watchUsingTaskInputs`.
//!
//! The core matching logic lives in `turborepo_engine::affected` and is
//! shared with `turbo query { affectedTasks }`. This module adds the
//! global-change fast path (root config files, global deps, root internal
//! dependencies) before delegating to the shared function, and seeds the tasks
//! of packages whose lockfile resolution changed.

use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{
        ChangeMapper, DefaultPackageChangeMapper, LockfileContents, PackageChanges,
        is_default_global_file,
    },
    global_deps::GlobalDepsMatcher,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_task_id::TaskId;

use crate::Engine;

/// Result of resolving which tasks are affected by file changes in watch mode
/// with `watchUsingTaskInputs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchTaskFilterResult {
    /// Changed files that still exist on disk.
    pub existing_files: HashSet<AnchoredSystemPathBuf>,
    /// Tasks whose `inputs` directly match the changed files.
    pub directly_affected: HashSet<TaskId<'static>>,
    /// Tasks that would execute in watch mode, including dependents and
    /// cacheable dependencies. Persistent non-interruptible tasks are omitted.
    /// Matches what `Engine::retain_watch_affected_tasks` keeps.
    pub execution_tasks: HashSet<TaskId<'static>>,
}

/// Resolves which tasks watch mode should stop and/or re-run when
/// `watchUsingTaskInputs` is enabled.
///
/// Both `stop_impacted_tasks` and `RunBuilder` engine pruning must call this
/// function so they stay aligned.
pub fn resolve_watch_task_filter(
    engine: &Engine,
    pkg_dep_graph: &PackageGraph,
    repo_root: &AbsoluteSystemPath,
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    global_deps: &[String],
) -> WatchTaskFilterResult {
    let existing_files = filter_existing_changed_files(repo_root, changed_files);
    let directly_affected = affected_task_ids(engine, pkg_dep_graph, &existing_files, global_deps);
    let execution_tasks = engine.watch_execution_closure_for_affected(&directly_affected);

    WatchTaskFilterResult {
        existing_files,
        directly_affected,
        execution_tasks,
    }
}

/// Filters changed files to those that still exist on disk.
///
/// Editor temp files (vim 4913, *~ backups, etc.) are created and deleted
/// within the same watcher batch. The hash algorithm only sees files that
/// exist, so input matching should too.
pub fn filter_existing_changed_files(
    repo_root: &AbsoluteSystemPath,
    changed_files: &HashSet<AnchoredSystemPathBuf>,
) -> HashSet<AnchoredSystemPathBuf> {
    changed_files
        .iter()
        .filter(|f| repo_root.resolve(f).exists())
        .cloned()
        .collect()
}

/// Determines which tasks are directly affected by the given set of changed
/// files. Does NOT expand to transitive dependents or dependencies. Callers
/// must use the closure appropriate to their mode: `retain_affected_tasks` for
/// `--affected`, or `retain_watch_affected_tasks` for watch mode.
///
/// Checks all tasks against all changed files regardless of package boundaries.
/// This is what makes cross-package inputs (`$TURBO_ROOT$/schema/api.json`)
/// work correctly.
///
/// Returns an empty set when no files have changed.
///
/// # Global changes
///
/// If any changed file is a global dependency (root config files,
/// user-configured `globalDependencies`, or a package the root package
/// depends on), all tasks are returned.
///
/// # Lockfile changes
///
/// A package manager lockfile change selects the tasks of the packages whose
/// resolved dependencies changed, as package-level affected detection does.
/// When the change cannot be resolved (watch mode, missing history, parse
/// failures) every task is selected.
///
/// # Error handling
///
/// Invalid glob patterns in task `inputs` are logged at `warn` level and
/// skipped. If the SCM range is invalid, the caller should handle the
/// fallback (typically running all tasks).
#[tracing::instrument(skip(engine, pkg_dep_graph, changed_files), fields(
    file_count = changed_files.len(),
    global_deps = global_deps.len(),
))]
pub fn affected_task_ids(
    engine: &Engine,
    pkg_dep_graph: &PackageGraph,
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    global_deps: &[String],
) -> HashSet<TaskId<'static>> {
    let root = pkg_dep_graph.repo_root();
    let tracked = match turborepo_scm::SCM::new(root).tracked_changed_files(root, changed_files) {
        Ok(tracked) => tracked,
        Err(error) => {
            tracing::warn!(%error, "unable to classify task input changes; selecting all tasks");
            return engine.task_ids().cloned().collect();
        }
    };
    let changes = turborepo_types::TaskInputChanges {
        files: changed_files.clone(),
        tracked,
    };
    // Watch batches have no base snapshot to compare the lockfile against.
    affected_task_ids_with_managed_setup(
        engine,
        pkg_dep_graph,
        &changes,
        global_deps,
        false,
        || LockfileContents::UnknownChange,
    )
}

/// Like `affected_task_ids`, with committed managed toolchain resolution
/// treated as a default global file, independently of task inputs and
/// configured globs.
///
/// `lockfile_contents` describes the lockfile change for the compared range,
/// typically from `ChangedFilesDetector::lockfile_contents`. It is only
/// invoked when the package manager lockfile is among the changed files.
pub fn affected_task_ids_with_managed_setup(
    engine: &Engine,
    pkg_dep_graph: &PackageGraph,
    changed_files: &turborepo_types::TaskInputChanges,
    global_deps: &[String],
    managed_setup: bool,
    lockfile_contents: impl FnOnce() -> LockfileContents,
) -> HashSet<TaskId<'static>> {
    if is_global_change(
        &changed_files.files,
        global_deps,
        pkg_dep_graph,
        managed_setup,
    ) {
        return engine.task_ids().cloned().collect();
    }

    let mut affected: HashSet<TaskId<'static>> =
        match turborepo_engine::match_tasks_against_changed_files(
            engine,
            pkg_dep_graph,
            changed_files,
        ) {
            Ok(matched) => matched.into_keys().collect(),
            Err(error) => {
                tracing::warn!(%error, "unable to project task affectedness; selecting all tasks");
                return engine.task_ids().cloned().collect();
            }
        };

    if package_manager_lockfile_changed(&changed_files.files, pkg_dep_graph) {
        match lockfile_changed_packages(pkg_dep_graph, lockfile_contents()) {
            Some(packages) => affected.extend(engine.task_ids_for_packages(&packages)),
            None => return engine.task_ids().cloned().collect(),
        }
    }

    affected
}

fn package_manager_lockfile_changed(
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    pkg_dep_graph: &PackageGraph,
) -> bool {
    pkg_dep_graph
        .package_manager()
        .map(|pm| pm.lockfile_name())
        .is_some_and(|lockfile| changed_files.iter().any(|file| file.as_str() == lockfile))
}

/// Resolves the packages invalidated by a lockfile change with the
/// package-level change mapper, so lockfile scoping (resolution comparison,
/// root dependency changes invalidating every package, relationship
/// projections) matches `ScopeChangeDetector` exactly. Changed files are
/// deliberately not passed: they are matched against task inputs instead.
///
/// Returns `None` when every package must be treated as changed.
fn lockfile_changed_packages(
    pkg_dep_graph: &PackageGraph,
    lockfile_contents: LockfileContents,
) -> Option<HashSet<PackageName>> {
    // The caller already knows the lockfile changed, so a reader that saw no
    // change is not evidence that nothing was affected.
    let lockfile_contents = match lockfile_contents {
        LockfileContents::Unchanged => LockfileContents::UnknownChange,
        contents => contents,
    };
    let mapper = ChangeMapper::new(
        pkg_dep_graph,
        Vec::new(),
        DefaultPackageChangeMapper::new(pkg_dep_graph),
    );
    match mapper.changed_packages(HashSet::new(), lockfile_contents) {
        Ok(PackageChanges::Some(packages)) => {
            tracing::debug!(
                packages = packages.len(),
                "lockfile change scoped to packages"
            );
            Some(packages.into_keys().map(|package| package.name).collect())
        }
        Ok(PackageChanges::All(reason)) => {
            tracing::debug!(?reason, "lockfile change affects all tasks");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "unable to scope lockfile change; selecting all tasks");
            None
        }
    }
}

/// Returns `true` if any changed file is a global dependency, meaning all
/// tasks should be considered affected regardless of their individual inputs.
///
/// Global changes include:
/// - Root config files: `turbo.json`, `turbo.jsonc`
/// - Files matching user-configured `globalDependencies` globs
/// - Files inside a package the root package depends on, directly or
///   transitively. Those files feed `hashOfInternalDependencies`, which is part
///   of the global hash, so they change every task's hash. This mirrors
///   `AllPackageChangeReason::RootInternalDepChanged` in the package-level
///   change mapper.
fn is_global_change(
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    global_deps: &[String],
    pkg_dep_graph: &PackageGraph,
    managed_setup: bool,
) -> bool {
    let global_deps_matcher = GlobalDepsMatcher::new_ignoring_invalid(
        global_deps.iter().map(String::as_str),
        |glob, error| {
            tracing::warn!(
                %glob,
                %error,
                "invalid globalDependency glob; ignoring for affected detection"
            );
        },
    );
    // The global hash walks these directories wholesale, so a prefix match
    // covers exactly the files that feed `hashOfInternalDependencies`,
    // nested packages included.
    let root_internal_dep_dirs = pkg_dep_graph.root_internal_package_dependencies_paths();

    for file in changed_files {
        let file_str = file.as_str();

        if is_default_global_file(file_str, managed_setup) {
            return true;
        }

        // A matcher compilation failure is conservative: select all tasks.
        if global_deps_matcher
            .as_ref()
            .map_or(true, |matcher| matcher.is_match(file_str))
        {
            return true;
        }

        if root_internal_dep_dirs
            .iter()
            .any(|dir| file.as_path().starts_with(dir.as_path()))
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, WorkspaceData},
        package_graph::PackageGraph,
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_task_id::TaskId;
    use turborepo_types::{TaskDefinition, TaskInputs};

    use super::*;
    use crate::Building;

    async fn make_pkg_graph(repo_root: &AbsoluteSystemPath, packages: &[&str]) -> PackageGraph {
        make_pkg_graph_with_root(repo_root, packages, PackageJson::default()).await
    }

    async fn make_pkg_graph_with_root(
        repo_root: &AbsoluteSystemPath,
        packages: &[&str],
        root_package_json: PackageJson,
    ) -> PackageGraph {
        let manifests: HashMap<_, _> = packages
            .iter()
            .map(|name| {
                (
                    repo_root.join_components(&["packages", name, "package.json"]),
                    PackageJson {
                        name: Some(Spanned::new(name.to_string())),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let workspaces = manifests
            .keys()
            .cloned()
            .map(|path| WorkspaceData::new(path, None).unwrap())
            .collect();
        let response = DiscoveryResponse {
            workspaces,
            package_manager: PackageManager::Npm,
        };

        PackageGraph::builder(repo_root, root_package_json)
            .with_package_discovery(move || {
                let response = response.clone();
                async move { Ok(response) }
            })
            .with_package_json_loader(move |path: &AbsoluteSystemPath| {
                manifests.get(path).cloned().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("missing test manifest at {path}"),
                    )
                    .into()
                })
            })
            .without_external_dependencies()
            .build()
            .await
            .unwrap()
    }

    fn make_engine(
        tasks: &[(TaskId<'static>, TaskDefinition)],
        edges: &[(TaskId<'static>, TaskId<'static>)],
    ) -> Engine {
        let mut engine: Engine<Building> = Engine::new();

        for (task_id, def) in tasks {
            engine.get_index(task_id);
            engine.add_definition(task_id.clone(), def.clone());
        }

        for (dependent, dependency) in edges {
            let dep_idx = engine.get_index(dependent);
            let dependency_idx = engine.get_index(dependency);
            engine
                .task_graph_mut()
                .add_edge(dep_idx, dependency_idx, ());
        }

        engine.seal()
    }

    fn changed(files: &[&str]) -> HashSet<AnchoredSystemPathBuf> {
        files
            .iter()
            .map(|f| AnchoredSystemPathBuf::from_raw(f).unwrap())
            .collect()
    }

    fn default_def() -> TaskDefinition {
        TaskDefinition::default()
    }

    #[tokio::test]
    async fn managed_setup_lock_is_global_independently_of_inputs_and_globs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let graph = make_pkg_graph(root, &["a", "b"]).await;
        let tasks = [
            (TaskId::new("a", "build"), default_def()),
            (TaskId::new("b", "build"), default_def()),
        ];
        let engine = make_engine(&tasks, &[]);
        let global_deps = vec!["!turbo.lock".to_string()];
        for managed_setup in [false, true] {
            let affected = affected_task_ids_with_managed_setup(
                &engine,
                &graph,
                &turborepo_types::TaskInputChanges::from_tracked(changed(&["turbo.lock"])),
                &global_deps,
                managed_setup,
                || unreachable!("turbo.lock is not the package manager lockfile"),
            );
            assert_eq!(affected.len(), if managed_setup { 2 } else { 0 });
            assert!(
                affected_task_ids_with_managed_setup(
                    &engine,
                    &graph,
                    &turborepo_types::TaskInputChanges::from_tracked(changed(&[
                        "packages/a/turbo.lock"
                    ])),
                    &global_deps,
                    managed_setup,
                    || unreachable!("turbo.lock is not the package manager lockfile"),
                )
                .iter()
                .all(|task| task.package() != "b")
            );
        }
    }

    #[tokio::test]
    async fn root_package_json_change_is_not_global() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let a_test = TaskId::new("lib-a", "test");
        let engine = make_engine(
            &[
                (a_build.clone(), default_def()),
                (a_test.clone(), default_def()),
            ],
            &[],
        );

        // root package.json is not in the global hash (when a lockfile exists),
        // so changing it should not affect all tasks.
        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["package.json"]), &[]);
        assert!(
            result.is_empty(),
            "root package.json should not globally affect tasks: {result:?}"
        );
    }

    #[tokio::test]
    async fn global_turbo_json_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["turbo.json"]), &[]);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    fn lockfile_changed_for(package: &str) -> LockfileContents {
        LockfileContents::Resolved(vec![
            turborepo_repository::package_graph::ExternalDependencyChange {
                package: turborepo_repository::package_graph::WorkspacePackage {
                    name: PackageName::from(package),
                    path: AnchoredSystemPathBuf::from_raw(format!("packages/{package}")).unwrap(),
                },
                added: Vec::new(),
                removed: Vec::new(),
            },
        ])
    }

    #[tokio::test]
    async fn resolved_lockfile_change_selects_only_changed_package_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");
        let b_test = TaskId::new("lib-b", "test");
        let engine = make_engine(
            &[
                (a_build.clone(), default_def()),
                (b_build.clone(), default_def()),
                (b_test.clone(), default_def()),
            ],
            &[],
        );

        let result = affected_task_ids_with_managed_setup(
            &engine,
            &pkg_graph,
            &turborepo_types::TaskInputChanges::from_tracked(changed(&["package-lock.json"])),
            &[],
            false,
            || lockfile_changed_for("lib-b"),
        );
        assert_eq!(result, HashSet::from([b_build, b_test]));
    }

    #[tokio::test]
    async fn resolved_lockfile_change_unions_with_input_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b", "lib-c"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");
        let c_build = TaskId::new("lib-c", "build");
        let engine = make_engine(
            &[
                (a_build.clone(), default_def()),
                (b_build.clone(), default_def()),
                (c_build.clone(), default_def()),
            ],
            &[],
        );

        let result = affected_task_ids_with_managed_setup(
            &engine,
            &pkg_graph,
            &turborepo_types::TaskInputChanges::from_tracked(changed(&[
                "package-lock.json",
                "packages/lib-a/src/index.ts",
            ])),
            &[],
            false,
            || lockfile_changed_for("lib-b"),
        );
        assert_eq!(result, HashSet::from([a_build, b_build]));
    }

    #[tokio::test]
    async fn unresolvable_lockfile_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let engine = make_engine(
            &[
                (TaskId::new("lib-a", "build"), default_def()),
                (TaskId::new("lib-b", "build"), default_def()),
            ],
            &[],
        );

        let lockfile =
            || turborepo_types::TaskInputChanges::from_tracked(changed(&["package-lock.json"]));
        // Previous lockfile content that cannot be parsed, a change without
        // history, and a reader that missed the change all fall back to
        // selecting every task.
        let cases: [fn() -> LockfileContents; 3] = [
            || LockfileContents::Changed {
                path: AnchoredSystemPathBuf::from_raw("package-lock.json").unwrap(),
                previous_contents: b"not a lockfile".to_vec(),
            },
            || LockfileContents::UnknownChange,
            || LockfileContents::Unchanged,
        ];
        for contents in cases {
            let result = affected_task_ids_with_managed_setup(
                &engine,
                &pkg_graph,
                &lockfile(),
                &[],
                false,
                contents,
            );
            assert_eq!(result.len(), 2, "{result:?}");
        }
    }

    #[tokio::test]
    async fn lockfile_contents_are_not_read_without_lockfile_change() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;
        let engine = make_engine(&[(TaskId::new("lib-a", "build"), default_def())], &[]);

        let result = affected_task_ids_with_managed_setup(
            &engine,
            &pkg_graph,
            &turborepo_types::TaskInputChanges::from_tracked(changed(&["README.md"])),
            &[],
            false,
            || unreachable!("lockfile did not change"),
        );
        assert!(result.is_empty(), "{result:?}");
    }

    #[tokio::test]
    async fn lockfile_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        // MockDiscovery returns PackageManager::Npm → lockfile is package-lock.json.
        // Watch mode has no base snapshot, so the change cannot be scoped.
        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["package-lock.json"]), &[]);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    #[tokio::test]
    async fn custom_global_deps_triggers_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        let global_deps = vec!["config/*.yaml".to_string()];
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["config/ci.yaml"]),
            &global_deps,
        );
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    #[tokio::test]
    async fn negated_global_deps_do_not_affect_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;
        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);
        let global_deps = vec!["ci/**".to_string(), "!ci/test/**".to_string()];

        for file in ["ci/test/plan.test.ts", "docs/notes.md"] {
            let result = affected_task_ids(&engine, &pkg_graph, &changed(&[file]), &global_deps);
            assert!(
                result.is_empty(),
                "{file} should not affect tasks: {result:?}"
            );
        }
        let result =
            affected_task_ids(&engine, &pkg_graph, &changed(&["ci/plan.ts"]), &global_deps);
        assert_eq!(result, HashSet::from([a_build]));
    }

    #[tokio::test]
    async fn global_turbo_jsonc_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["turbo.jsonc"]), &[]);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    #[tokio::test]
    async fn invalid_global_dep_glob_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        // Invalid glob is skipped, valid glob still works.
        let global_deps = vec!["[invalid".to_string(), "config/*.yaml".to_string()];
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["config/ci.yaml"]),
            &global_deps,
        );
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    /// A change inside a package the root depends on goes into
    /// `hashOfInternalDependencies`, so it changes every task's hash.
    /// Task-level affected detection has to treat it as a global change, the
    /// way `ChangeMapper` does with `RootInternalDepChanged`.
    #[tokio::test]
    async fn root_internal_dependency_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let root_package_json = PackageJson {
            dependencies: Some([("lib-a".to_string(), "workspace:*".to_string())].into()),
            ..Default::default()
        };
        // `lib-ab` shares a name prefix with the root dependency `lib-a`.
        let pkg_graph =
            make_pkg_graph_with_root(root, &["lib-a", "lib-ab"], root_package_json).await;

        let a_build = TaskId::new("lib-a", "build");
        let ab_build = TaskId::new("lib-ab", "build");
        let engine = make_engine(
            &[
                (a_build.clone(), default_def()),
                (ab_build.clone(), default_def()),
            ],
            &[],
        );

        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/src/x.txt"]),
            &[],
        );
        assert_eq!(
            result,
            HashSet::from([a_build, ab_build.clone()]),
            "a root internal dependency change affects every task"
        );

        // A package the root does not depend on stays local, even when its
        // directory shares a name prefix with the root dependency.
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-ab/src/x.txt"]),
            &[],
        );
        assert_eq!(result, HashSet::from([ab_build]));
    }

    #[tokio::test]
    async fn unknown_task_package_conservatively_selects_all_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &[]).await;
        let unknown = TaskId::new("missing", "build");
        let engine = make_engine(&[(unknown.clone(), default_def())], &[]);

        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["file.txt"]), &[]);
        assert_eq!(result, HashSet::from([unknown]));
    }

    #[tokio::test]
    async fn invalid_task_glob_conservatively_selects_all_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;
        let task = TaskId::new("lib-a", "build");
        let unaffected = TaskId::new("lib-a", "test");
        let definition = TaskDefinition {
            inputs: TaskInputs {
                globs: vec!["[invalid".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = make_engine(
            &[
                (task.clone(), definition),
                (unaffected.clone(), default_def()),
            ],
            &[],
        );

        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["file.txt"]), &[]);
        assert_eq!(result, HashSet::from([task, unaffected]));
    }
}
