//! Shared task-level affected detection for both `turbo run --affected`
//! and `turbo query { affectedTasks }`.
//!
//! Iterates **all** engine tasks and checks each task's `inputs` globs
//! against a set of changed files. This is the canonical implementation
//! that both code paths must use to avoid divergence on which tasks are
//! considered (e.g. tasks with `$TURBO_ROOT$` inputs in non-affected
//! packages).

use std::collections::{HashMap, HashSet};

use turbopath::AnchoredSystemPathBuf;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{
    TaskDefinition, TaskInputs,
    task_input_matching::{compile_globs, file_matches_compiled_inputs},
};

use crate::{Built, Engine};

/// Fallback inputs when a task has no definition in the engine.
/// `default: true` means all files in the package directory are considered
/// inputs, matching turbo's default hashing behavior.
static DEFAULT_TASK_INPUTS: TaskInputs = TaskInputs {
    globs: Vec::new(),
    default: true,
};

/// Returns the set of tasks whose input globs match at least one changed file.
///
/// Iterates every task in the engine regardless of which packages are
/// "affected" at the package level. This is critical for correctness:
/// a task in a non-affected package may declare `$TURBO_ROOT$` inputs
/// (resolved to `../../` globs) that reference changed root-level files.
///
/// The returned map associates each affected task with the repo-relative
/// path of the first file that matched its inputs. Callers that don't
/// need the file path can discard the values.
///
/// Does NOT include transitive dependents. Callers should propagate
/// through the task graph separately if needed.
pub fn match_tasks_against_changed_files(
    engine: &Engine<Built, TaskDefinition>,
    pkg_dep_graph: &PackageGraph,
    changed_files: &HashSet<AnchoredSystemPathBuf>,
) -> HashMap<TaskId<'static>, String> {
    let mut matched = HashMap::new();

    // Pre-convert all file paths to Unix strings once, avoiding repeated
    // allocation in the O(tasks × files) inner loop.
    let changed_unix: Vec<String> = changed_files
        .iter()
        .map(|f| f.to_unix().to_string())
        .collect();

    // Compile globs once per unique (package_path, inputs) pair.
    let mut compiled_cache: HashMap<(String, TaskInputs), _> = HashMap::new();

    for task_id in engine.task_ids() {
        let pkg_name = PackageName::from(task_id.package());
        let Some(pkg_dir) = pkg_dep_graph.package_dir(&pkg_name) else {
            continue;
        };
        let pkg_unix = pkg_dir.to_unix();
        let pkg_str = pkg_unix.to_string();

        let inputs = engine
            .task_definition(task_id)
            .map(|def| &def.inputs)
            .unwrap_or(&DEFAULT_TASK_INPUTS);

        let cache_key = (pkg_str.clone(), inputs.clone());
        let compiled = compiled_cache
            .entry(cache_key)
            .or_insert_with(|| compile_globs(inputs));

        let pkg_prefix_slash = if pkg_str.is_empty() {
            String::new()
        } else {
            format!("{pkg_str}/")
        };

        for file_unix in &changed_unix {
            if file_matches_compiled_inputs(file_unix, &pkg_str, &pkg_prefix_slash, compiled) {
                matched.insert(task_id.clone(), file_unix.clone());
                break;
            }
        }
    }

    matched
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
    use crate::Building;

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
    ) -> Engine<crate::Built, TaskDefinition> {
        let mut engine: Engine<Building, TaskDefinition> = Engine::new();
        for (task_id, def) in tasks {
            engine.get_index(task_id);
            engine.add_definition(task_id.clone(), def.clone());
        }
        engine.seal()
    }

    fn changed(files: &[&str]) -> HashSet<AnchoredSystemPathBuf> {
        files
            .iter()
            .map(|f| AnchoredSystemPathBuf::from_raw(f).unwrap())
            .collect()
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

    // Single-file matching (default inputs, exclusions, explicit globs,
    // $TURBO_ROOT$ traversal) is thoroughly tested in
    // `turborepo_types::task_input_matching::tests`. The tests below only
    // cover behavior unique to `match_tasks_against_changed_files`:
    // multi-task iteration, the DEFAULT_TASK_INPUTS fallback, and the
    // cross-package $TURBO_ROOT$ scenario that motivated this module.

    #[tokio::test]
    async fn multiple_tasks_selective_matching() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let a_test = TaskId::new("lib-a", "test");
        let a_typecheck = TaskId::new("lib-a", "typecheck");

        let engine = make_engine(&[
            (a_build.clone(), TaskDefinition::default()),
            (a_test.clone(), def_with_inputs(&["!**/*.md"], true)),
            (
                a_typecheck.clone(),
                def_with_inputs(&["!**/*.md", "!**/*.test.ts"], true),
            ),
        ]);

        // .md change: only build is affected
        let result = match_tasks_against_changed_files(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/README.md"]),
        );
        assert_eq!(result.len(), 1, "only build should match .md: {result:?}");
        assert!(result.contains_key(&a_build));

        // .test.ts change: build and test are affected, typecheck is not
        let result = match_tasks_against_changed_files(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/foo.test.ts"]),
        );
        assert_eq!(
            result.len(),
            2,
            "build+test should match .test.ts: {result:?}"
        );
        assert!(result.contains_key(&a_build));
        assert!(result.contains_key(&a_test));
        assert!(!result.contains_key(&a_typecheck));
    }

    #[tokio::test]
    async fn task_without_definition_uses_default_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        // Register task in the graph but do NOT add a definition.
        let mut engine: Engine<Building, TaskDefinition> = Engine::new();
        let a_build = TaskId::new("lib-a", "build");
        engine.get_index(&a_build);
        let engine = engine.seal();

        let result = match_tasks_against_changed_files(
            &engine,
            &pkg_graph,
            &changed(&["packages/lib-a/src/index.ts"]),
        );
        assert_eq!(
            result.len(),
            1,
            "task with no definition should use default inputs: {result:?}"
        );
    }

    /// The key scenario: a task in package B has $TURBO_ROOT$ inputs
    /// but package B itself has no source file changes. The function
    /// must still detect it because it iterates ALL engine tasks.
    #[tokio::test]
    async fn turbo_root_input_in_non_affected_package() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");

        let engine = make_engine(&[
            (a_build.clone(), TaskDefinition::default()),
            (
                b_build.clone(),
                def_with_inputs(&["../../config.txt"], true),
            ),
        ]);

        // Only a root-level file changed. lib-a has default inputs so its
        // source dir didn't change. lib-b's $TURBO_ROOT$ input DID change.
        let result =
            match_tasks_against_changed_files(&engine, &pkg_graph, &changed(&["config.txt"]));
        assert!(
            result.contains_key(&b_build),
            "lib-b#build should match via $TURBO_ROOT$ input: {result:?}"
        );
        // lib-a's default inputs only match files within packages/lib-a/
        assert!(
            !result.contains_key(&a_build),
            "lib-a#build should not match a root file with default inputs: {result:?}"
        );
    }
}
