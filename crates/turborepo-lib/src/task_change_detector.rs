//! Task-level affected detection for `--affected` with the
//! `affectedUsingTaskInputs` future flag.
//!
//! The core matching logic lives in `turborepo_engine::affected` and is
//! shared with `turbo query { affectedTasks }`. This module adds the
//! global-change fast path (root config files, lockfile, global deps)
//! before delegating to the shared function.

use std::collections::HashSet;

use turbopath::AnchoredSystemPathBuf;
use turborepo_repository::package_graph::PackageGraph;
use turborepo_task_id::TaskId;
use wax::Program;

use crate::engine::Engine;

/// Root-level files that always trigger a full rebuild when changed.
///
/// - `package.json`: workspace topology or root dependency changes
/// - `turbo.json`/`turbo.jsonc`: task definitions, global deps, pipelines
///
/// Lockfile changes are detected separately via the package manager.
const DEFAULT_GLOBAL_DEPS: &[&str] = &["package.json", "turbo.json", "turbo.jsonc"];

/// Determines which tasks are directly affected by the given set of changed
/// files. Does NOT include transitive dependents — use
/// `Engine::retain_affected_tasks` afterward for that.
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
/// or user-configured `globalDependencies`), all tasks are returned.
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

    turborepo_engine::match_tasks_against_changed_files(engine, pkg_dep_graph, changed_files)
        .into_keys()
        .collect()
}

/// Returns `true` if any changed file is a global dependency, meaning all
/// tasks should be considered affected regardless of their individual inputs.
///
/// Global changes include:
/// - Root config files: `package.json`, `turbo.json`, `turbo.jsonc`
/// - The package manager's lockfile
/// - Files matching user-configured `globalDependencies` globs
fn is_global_change(
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    global_deps: &[String],
    pkg_dep_graph: &PackageGraph,
) -> bool {
    let lockfile_name = pkg_dep_graph.package_manager().lockfile_name();
    let global_globs: Vec<_> = global_deps
        .iter()
        .filter_map(|g| match wax::Glob::new(g) {
            Ok(glob) => Some(glob),
            Err(e) => {
                tracing::warn!(
                    glob = %g,
                    error = %e,
                    "invalid globalDependency glob; ignoring for affected detection"
                );
                None
            }
        })
        .collect();
    let global_deps_matcher = wax::any(global_globs).ok();

    for file in changed_files {
        let file_str = file.as_str();

        if DEFAULT_GLOBAL_DEPS.contains(&file_str) {
            return true;
        }

        if file_str == lockfile_name {
            return true;
        }

        if let Some(ref matcher) = global_deps_matcher {
            if matcher.is_match(file_str) {
                return true;
            }
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
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_graph::PackageGraph,
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_task_id::TaskId;
    use turborepo_types::TaskDefinition;

    use super::*;
    use crate::engine::Building;

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
                name: Some(Spanned::new(name.to_string())),
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
    async fn global_package_json_change_returns_all() {
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

        let result = affected_task_ids(&engine, &pkg_graph, &changed(&["package.json"]), &[]);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&a_build));
        assert!(result.contains(&a_test));
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
}
