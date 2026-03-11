//! Task-level affected detection for `--affected` with the
//! `affectedUsingTaskInputs` future flag.
//!
//! Glob matching logic is shared with `turborepo-query/src/affected_tasks.rs`
//! via `turborepo_types::task_input_matching`. Changes to matching semantics
//! apply to both `turbo run --affected` and `turbo query { affectedTasks }`.

use std::collections::{HashMap, HashSet};

use turbopath::AnchoredSystemPathBuf;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{
    task_input_matching::{compile_globs, file_matches_compiled_inputs},
    TaskInputs,
};
use wax::Program;

use crate::engine::Engine;

// Root-level files that always trigger a full rebuild when changed.
// Lockfile detection is handled separately via the package manager.
const DEFAULT_GLOBAL_DEPS: &[&str] = &["package.json", "turbo.json", "turbo.jsonc"];

/// Determines which tasks are directly affected by the given set of changed
/// files. Does NOT include transitive dependents — use
/// `Engine::retain_affected_tasks` afterward for that.
///
/// Checks all tasks against all changed files regardless of package boundaries.
/// This is what makes cross-package inputs (`$TURBO_ROOT$/schema/api.json`)
/// work correctly.
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
pub fn affected_task_ids(
    engine: &Engine,
    pkg_dep_graph: &PackageGraph,
    changed_files: &HashSet<AnchoredSystemPathBuf>,
    global_deps: &[String],
) -> HashSet<TaskId<'static>> {
    if is_global_change(changed_files, global_deps, pkg_dep_graph) {
        return engine.task_ids().cloned().collect();
    }

    let mut affected = HashSet::new();

    // Compile globs once per unique (package_path, inputs) pair.
    let mut compiled_cache: HashMap<(String, TaskInputsKey), _> = HashMap::new();

    for task_id in engine.task_ids() {
        let pkg_name = PackageName::from(task_id.package());
        let Some(pkg_dir) = pkg_dep_graph.package_dir(&pkg_name) else {
            continue;
        };
        let pkg_unix = pkg_dir.to_unix();

        let default_inputs = DEFAULT_TASK_INPUTS;
        let inputs = engine
            .task_definition(task_id)
            .map(|def| &def.inputs)
            .unwrap_or(&default_inputs);

        let cache_key = (pkg_unix.to_string(), TaskInputsKey::from(inputs));
        let compiled = compiled_cache
            .entry(cache_key)
            .or_insert_with(|| compile_globs(inputs));

        for file in changed_files {
            if file_matches_compiled_inputs(file, &pkg_unix, compiled) {
                affected.insert(task_id.clone());
                break;
            }
        }
    }

    affected
}

/// Fallback inputs when a task has no definition in the engine.
/// `default: true` means all files in the package directory are considered
/// inputs, matching turbo's default hashing behavior.
const DEFAULT_TASK_INPUTS: TaskInputs = TaskInputs {
    globs: Vec::new(),
    default: true,
};

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

#[derive(Hash, Eq, PartialEq)]
struct TaskInputsKey {
    globs: Vec<String>,
    default: bool,
}

impl From<&TaskInputs> for TaskInputsKey {
    fn from(inputs: &TaskInputs) -> Self {
        Self {
            globs: inputs.globs.clone(),
            default: inputs.default,
        }
    }
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
    use turborepo_types::{TaskDefinition, TaskInputs};

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

    fn def_with_inputs(globs: &[&str], default: bool) -> TaskDefinition {
        TaskDefinition {
            inputs: TaskInputs {
                globs: globs.iter().map(|s| s.to_string()).collect(),
                default,
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn no_changed_files_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let engine = make_engine(&[(TaskId::new("lib-a", "build"), default_def())], &[]);

        let result = affected_task_ids(&engine, &pkg_graph, &HashSet::new(), &[]);
        assert!(result.is_empty());
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
    async fn source_file_matches_default_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/src/index.ts"]),
            &[],
        );
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a_build));
    }

    #[tokio::test]
    async fn excluded_file_not_matched() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        // Task excludes .md files from its inputs
        let a_test = TaskId::new("lib-a", "test");
        let engine = make_engine(
            &[(a_test.clone(), def_with_inputs(&["!**/*.md"], true))],
            &[],
        );

        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/README.md"]),
            &[],
        );
        assert!(
            result.is_empty(),
            ".md change should not affect task that excludes *.md"
        );
    }

    #[tokio::test]
    async fn file_outside_package_not_matched() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(&[(a_build.clone(), default_def())], &[]);

        // Changing a file in lib-b should not affect lib-a's build
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-b/src/index.ts"]),
            &[],
        );
        assert!(
            result.is_empty(),
            "file in sibling package should not match: {result:?}"
        );
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
    async fn non_matching_file_not_affected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        // Task only includes .ts files explicitly (no $TURBO_DEFAULT$)
        let a_build = TaskId::new("lib-a", "build");
        let engine = make_engine(
            &[(a_build.clone(), def_with_inputs(&["src/**/*.ts"], false))],
            &[],
        );

        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/README.md"]),
            &[],
        );
        assert!(
            result.is_empty(),
            ".md file should not match src/**/*.ts inputs"
        );
    }

    #[tokio::test]
    async fn multiple_tasks_selective_matching() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let a_test = TaskId::new("lib-a", "test");
        let a_typecheck = TaskId::new("lib-a", "typecheck");

        let engine = make_engine(
            &[
                // build: default inputs (matches everything in package)
                (a_build.clone(), default_def()),
                // test: excludes .md
                (a_test.clone(), def_with_inputs(&["!**/*.md"], true)),
                // typecheck: excludes .md and .test.ts
                (
                    a_typecheck.clone(),
                    def_with_inputs(&["!**/*.md", "!**/*.test.ts"], true),
                ),
            ],
            &[],
        );

        // .md change: only build is affected
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/README.md"]),
            &[],
        );
        assert_eq!(result.len(), 1, "only build should match .md: {result:?}");
        assert!(result.contains(&a_build));

        // .test.ts change: build and test are affected, typecheck is not
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/foo.test.ts"]),
            &[],
        );
        assert_eq!(
            result.len(),
            2,
            "build+test should match .test.ts: {result:?}"
        );
        assert!(result.contains(&a_build));
        assert!(result.contains(&a_test));
        assert!(!result.contains(&a_typecheck));

        // .ts source change: all three are affected
        let result = affected_task_ids(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/src/index.ts"]),
            &[],
        );
        assert_eq!(
            result.len(),
            3,
            "all tasks should match .ts source: {result:?}"
        );
    }
}
