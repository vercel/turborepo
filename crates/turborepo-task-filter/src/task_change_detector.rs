//! Task-level affected detection for `--affected` with the
//! `affectedUsingTaskInputs` future flag and `turbo watch` with
//! `watchUsingTaskInputs`.
//!
//! The core matching logic lives in `turborepo_engine::affected` and is
//! shared with `turbo query { affectedTasks }`. This module adds the
//! global-change fast path (root config files, lockfile, global deps, root
//! internal dependencies) before delegating to the shared function.

use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{global_deps::GlobalDepsMatcher, package_graph::PackageGraph};
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

/// Root-level files that always trigger a full rebuild when changed.
///
/// - `package.json`: workspace topology or root dependency changes
/// - `turbo.json`/`turbo.jsonc`: task definitions, global deps, pipelines
///
/// Lockfile changes are detected separately via the package manager.
const DEFAULT_GLOBAL_DEPS: &[&str] = &["turbo.json", "turbo.jsonc"];

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
/// If any changed file is a global dependency (root config files, lockfile,
/// user-configured `globalDependencies`, or a package the root package
/// depends on), all tasks are returned.
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
    if is_global_change(changed_files, global_deps, pkg_dep_graph) {
        return engine.task_ids().cloned().collect();
    }

    match turborepo_engine::match_tasks_against_changed_files(engine, pkg_dep_graph, changed_files)
    {
        Ok(matched) => matched.into_keys().collect(),
        Err(error) => {
            tracing::warn!(%error, "unable to project task affectedness; selecting all tasks");
            engine.task_ids().cloned().collect()
        }
    }
}

/// Returns `true` if any changed file is a global dependency, meaning all
/// tasks should be considered affected regardless of their individual inputs.
///
/// Global changes include:
/// - Root config files: `package.json`, `turbo.json`, `turbo.jsonc`
/// - The package manager's lockfile
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
) -> bool {
    let lockfile_name = pkg_dep_graph.package_manager().map(|pm| pm.lockfile_name());
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

        if DEFAULT_GLOBAL_DEPS.contains(&file_str) {
            return true;
        }

        if Some(file_str) == lockfile_name {
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
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
    use turborepo_errors::Spanned;
    use turborepo_repository::{
        discovery::WorkspaceData,
        package_graph::PackageGraph,
        package_json::PackageJson,
        package_manager::PackageManager,
        test_util::{MockPackageDiscovery, MockPackageJsonLoader, PackageGraphFixture},
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
        let mut fixture =
            PackageGraphFixture::new(repo_root).with_root_package_json(root_package_json);
        for name in packages {
            fixture = fixture.with_package(name, &format!("packages/{name}"));
        }
        fixture.build().await.unwrap()
    }

    #[tokio::test]
    async fn affected_tasks_use_injected_repository_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(dir.path()).unwrap();
        let manifest_path = root.join_components(&["packages", "lib-a", "package.json"]);
        let graph = PackageGraph::builder(root, PackageJson::default())
            .with_package_discovery(
                MockPackageDiscovery::new(PackageManager::Npm).with_workspaces(vec![
                    WorkspaceData::new(manifest_path.clone(), None).unwrap(),
                ]),
            )
            .with_package_json_loader(Arc::new(MockPackageJsonLoader::new(HashMap::from([(
                manifest_path,
                PackageJson {
                    name: Some(Spanned::new("lib-a".to_string())),
                    ..Default::default()
                },
            )]))))
            .without_external_dependencies()
            .build()
            .await
            .unwrap();
        let build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(build.clone(), TaskDefinition::default())], &[]);

        assert_eq!(
            affected_task_ids(
                &engine,
                &graph,
                &changed(&["packages/lib-a/src/index.ts"]),
                &[]
            ),
            HashSet::from([build])
        );
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

    #[tokio::test]
    async fn lockfile_change_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        // MockDiscovery returns PackageManager::Npm → lockfile is package-lock.json
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
