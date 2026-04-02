//! Task-level filter resolution for `--filter` with the `filterUsingTasks`
//! future flag.
//!
//! When active, `--filter` patterns are resolved against the task graph
//! rather than the package graph:
//!
//! - Git-range selectors (`[main]`) match changed files against each task's
//!   `inputs` globs, catching out-of-package inputs like `$TURBO_ROOT$`.
//! - The `...` dependency/dependent syntax traverses the task graph, picking up
//!   cross-package task dependencies (e.g. `web#build -> schema#gen` where
//!   `web` has no package-level dependency on `schema`).
//!
//! The core matching logic reuses
//! `turborepo_engine::match_tasks_against_changed_files`
//! and `crate::task_change_detector::affected_task_ids`, sharing code with
//! `--affected` + `affectedUsingTaskInputs`.

use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_scm::SCM;
use turborepo_scope::{target_selector::GitRange, TargetSelector};
use turborepo_task_id::TaskId;
use wax::Program;

use crate::engine::Engine;

/// Filters an engine down to only the tasks matching the given selectors.
///
/// Each include selector contributes a set of tasks (unioned together).
/// Exclude selectors remove tasks from the result. After all selectors
/// are processed, the engine is pruned to the surviving tasks, their
/// transitive dependents, and all transitive dependencies needed for
/// execution.
pub fn filter_engine_to_tasks(
    engine: Engine,
    selectors: &[TargetSelector],
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<Engine, crate::run::error::Error> {
    let (include, exclude): (Vec<_>, Vec<_>) = selectors.iter().partition(|s| !s.exclude);

    let mut included_tasks: HashSet<TaskId<'static>> = HashSet::new();

    for selector in &include {
        let matched = resolve_selector_to_tasks(
            &engine,
            selector,
            pkg_dep_graph,
            scm,
            repo_root,
            global_deps,
        )?;
        included_tasks.extend(matched);
    }

    // If there were no include selectors (only excludes), start with all tasks.
    if include.is_empty() {
        included_tasks = engine.task_ids().cloned().collect();
    }

    for selector in &exclude {
        let to_exclude = resolve_selector_to_tasks(
            &engine,
            selector,
            pkg_dep_graph,
            scm,
            repo_root,
            global_deps,
        )?;
        included_tasks.retain(|t| !to_exclude.contains(t));
    }

    if included_tasks.is_empty() {
        return Ok(engine.retain_filtered_tasks(&included_tasks));
    }

    // retain_filtered_tasks includes transitive dependencies for
    // execution and prunes the rest. Dependent expansion was already
    // handled during selector resolution via `include_dependents`.
    Ok(engine.retain_filtered_tasks(&included_tasks))
}

/// Resolves a single selector to the set of matching task IDs.
///
/// Steps:
/// 1. Find the "base" task set from name/directory/git-range
/// 2. Expand via `...` (dependencies/dependents) in the task graph
fn resolve_selector_to_tasks(
    engine: &Engine,
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<HashSet<TaskId<'static>>, crate::run::error::Error> {
    if selector.match_dependencies {
        return resolve_match_dependencies(
            engine,
            selector,
            pkg_dep_graph,
            scm,
            repo_root,
            global_deps,
        );
    }

    let base_tasks =
        resolve_base_tasks(engine, selector, pkg_dep_graph, scm, repo_root, global_deps)?;

    let mut result = HashSet::new();

    if selector.include_dependencies {
        let deps = engine.collect_task_dependencies(&base_tasks);
        result.extend(deps);
    }

    if selector.include_dependents {
        let dependents = engine.collect_task_dependents(&base_tasks);
        result.extend(dependents);
    }

    if selector.include_dependencies || selector.include_dependents {
        if selector.exclude_self {
            // Remove the originally matched tasks, keeping only the
            // traversed deps/dependents.
            for t in &base_tasks {
                result.remove(t);
            }
        } else {
            result.extend(base_tasks);
        }
    } else {
        result.extend(base_tasks);
    }

    Ok(result)
}

/// The base task set before `...` expansion.
///
/// Combines name/directory matching (package-level) with git-range matching
/// (task-level via inputs). When both are present, the result is their
/// intersection (same semantics as the package-level filter).
fn resolve_base_tasks(
    engine: &Engine,
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<HashSet<TaskId<'static>>, crate::run::error::Error> {
    let tasks_from_packages = resolve_name_and_dir(engine, selector, pkg_dep_graph);
    let tasks_from_git_range =
        resolve_git_range(engine, selector, pkg_dep_graph, scm, repo_root, global_deps)?;

    match (tasks_from_packages, tasks_from_git_range) {
        (Some(pkg_tasks), Some(git_tasks)) => {
            // Intersection: task must match both name/dir AND git range.
            Ok(pkg_tasks.intersection(&git_tasks).cloned().collect())
        }
        (Some(tasks), None) | (None, Some(tasks)) => Ok(tasks),
        (None, None) => Ok(HashSet::new()),
    }
}

/// Matches tasks by package name pattern and/or directory.
/// Returns None if the selector has no name/directory constraints.
fn resolve_name_and_dir(
    engine: &Engine,
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
) -> Option<HashSet<TaskId<'static>>> {
    let has_name = !selector.name_pattern.is_empty();
    let has_dir = selector.parent_dir.is_some();

    if !has_name && !has_dir {
        return None;
    }

    let matching_packages = find_matching_packages(selector, pkg_dep_graph);
    Some(engine.task_ids_for_packages(&matching_packages))
}

/// Finds packages matching a selector's name pattern and/or directory.
fn find_matching_packages(
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
) -> HashSet<PackageName> {
    let mut packages: HashSet<PackageName> = HashSet::new();

    // Directory matching
    if let Some(parent_dir) = &selector.parent_dir {
        let parent_dir_unix = parent_dir.to_unix();
        if let Ok(globber) = wax::Glob::new(parent_dir_unix.as_str()) {
            let root_anchor = AnchoredSystemPathBuf::from_raw(".").expect("valid anchored");
            if parent_dir == &root_anchor {
                packages.insert(PackageName::Root);
            } else {
                for (name, info) in pkg_dep_graph.packages() {
                    if globber.is_match(info.package_path().as_path()) {
                        packages.insert(name.clone());
                    }
                }
            }
        }
    } else {
        // Start with all packages when only name pattern is used
        packages = pkg_dep_graph
            .packages()
            .map(|(name, _)| name.clone())
            .collect();
    }

    // Name pattern matching
    if !selector.name_pattern.is_empty() {
        if let Ok(matcher) = turborepo_scope::simple_glob::SimpleGlob::new(&selector.name_pattern) {
            use turborepo_scope::simple_glob::Match;
            packages.retain(|name| matcher.is_match(name.as_ref()));
        }
    }

    packages
}

/// Matches tasks by git range using task-level input matching.
/// Returns None if the selector has no git range.
fn resolve_git_range(
    engine: &Engine,
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<Option<HashSet<TaskId<'static>>>, crate::run::error::Error> {
    let git_range = match &selector.git_range {
        Some(range) => range,
        None => return Ok(None),
    };

    let changed_files = get_changed_files(scm, repo_root, git_range)?;

    match changed_files {
        Ok(files) => {
            let affected = crate::task_change_detector::affected_task_ids(
                engine,
                pkg_dep_graph,
                &files,
                global_deps,
            );
            Ok(Some(affected))
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                "SCM returned invalid change set for filter git range; including all tasks"
            );
            // If we can't determine changes, include all tasks (safe fallback).
            Ok(Some(engine.task_ids().cloned().collect()))
        }
    }
}

/// Resolves `match_dependencies` selectors (e.g. `web...[main]`).
///
/// Finds tasks in the named packages (or their task-graph dependencies)
/// whose inputs match the changed files.
fn resolve_match_dependencies(
    engine: &Engine,
    selector: &TargetSelector,
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<HashSet<TaskId<'static>>, crate::run::error::Error> {
    let git_range = match &selector.git_range {
        Some(range) => range,
        None => return Ok(HashSet::new()),
    };

    // Find all tasks in the named packages
    let matching_packages = find_matching_packages(selector, pkg_dep_graph);
    let package_tasks = engine.task_ids_for_packages(&matching_packages);

    // Expand to include all task-graph dependencies
    let mut candidate_tasks = engine.collect_task_dependencies(&package_tasks);
    if !selector.exclude_self {
        candidate_tasks.extend(package_tasks);
    }

    // Now check which of these candidates are affected by the git range
    let changed_files = get_changed_files(scm, repo_root, git_range)?;

    match changed_files {
        Ok(files) => {
            let affected = crate::task_change_detector::affected_task_ids(
                engine,
                pkg_dep_graph,
                &files,
                global_deps,
            );
            // Intersection: task must be in the candidate set AND affected
            Ok(candidate_tasks.intersection(&affected).cloned().collect())
        }
        Err(_) => {
            // Can't determine changes → return all candidates
            Ok(candidate_tasks)
        }
    }
}

fn get_changed_files(
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    git_range: &GitRange,
) -> Result<
    Result<HashSet<AnchoredSystemPathBuf>, turborepo_scm::git::InvalidRange>,
    crate::run::error::Error,
> {
    let result = scm.changed_files(
        repo_root,
        git_range.from_ref.as_deref(),
        git_range.to_ref.as_deref(),
        git_range.include_uncommitted,
        git_range.merge_base,
        git_range.allow_unknown_objects,
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery},
        package_graph::{PackageGraph, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_task_id::TaskId;
    use turborepo_types::{TaskDefinition, TaskInputs};

    use crate::engine::{Building, Engine};

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
        edges: &[(TaskId<'static>, TaskId<'static>)],
    ) -> Engine {
        let mut engine: Engine<Building> = Engine::new();
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

    fn def_with_inputs(globs: &[&str], default: bool) -> TaskDefinition {
        TaskDefinition {
            inputs: TaskInputs {
                globs: globs.iter().map(|s| s.to_string()).collect(),
                default,
            },
            ..Default::default()
        }
    }

    /// `--filter=web...` should pick up cross-package task deps.
    ///
    /// web#build -> schema#gen is a task dependency but web has no
    /// package-level dep on schema. The filter should include schema#gen.
    #[tokio::test]
    async fn include_dependencies_traverses_task_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let _pkg_graph = make_pkg_graph(root, &["web", "schema"]).await;

        let web_build = TaskId::new("web", "build");
        let schema_gen = TaskId::new("schema", "gen");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (schema_gen.clone(), TaskDefinition::default()),
            ],
            // web#build depends on schema#gen (task dep, not package dep)
            &[(web_build.clone(), schema_gen.clone())],
        );

        let matching_packages: HashSet<_> = ["web"].iter().map(|s| PackageName::from(*s)).collect();
        let web_tasks = engine.task_ids_for_packages(&matching_packages);
        assert!(web_tasks.contains(&web_build));
        assert!(!web_tasks.contains(&schema_gen));

        // include_dependencies should traverse the task graph
        let mut all_tasks = engine.collect_task_dependencies(&web_tasks);
        all_tasks.extend(web_tasks);
        assert!(all_tasks.contains(&web_build));
        assert!(all_tasks.contains(&schema_gen));
    }

    /// `--filter=...schema` should pick up task-graph dependents.
    #[tokio::test]
    async fn include_dependents_traverses_task_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let _pkg_graph = make_pkg_graph(root, &["web", "schema"]).await;

        let web_build = TaskId::new("web", "build");
        let schema_gen = TaskId::new("schema", "gen");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (schema_gen.clone(), TaskDefinition::default()),
            ],
            &[(web_build.clone(), schema_gen.clone())],
        );

        let matching_packages: HashSet<_> =
            ["schema"].iter().map(|s| PackageName::from(*s)).collect();
        let schema_tasks = engine.task_ids_for_packages(&matching_packages);

        let mut all_tasks = engine.collect_task_dependents(&schema_tasks);
        all_tasks.extend(schema_tasks);
        assert!(all_tasks.contains(&web_build));
        assert!(all_tasks.contains(&schema_gen));
    }

    /// Task input matching: a task with $TURBO_ROOT$ inputs should be
    /// detectable through engine-level matching.
    #[tokio::test]
    async fn task_inputs_matching_via_engine() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a"]).await;

        let a_build = TaskId::new("lib-a", "build");
        // ../../config.txt is the $TURBO_ROOT$ expansion for packages/lib-a
        let engine = make_engine(
            &[(
                a_build.clone(),
                def_with_inputs(&["../../config.txt"], true),
            )],
            &[],
        );

        let changed: HashSet<AnchoredSystemPathBuf> = ["config.txt"]
            .iter()
            .map(|f| AnchoredSystemPathBuf::from_raw(f).unwrap())
            .collect();

        let affected =
            turborepo_engine::match_tasks_against_changed_files(&engine, &pkg_graph, &changed);
        assert!(
            affected.contains_key(&a_build),
            "task with $TURBO_ROOT$ input should match root file change"
        );
    }

    /// resolve_selector_to_tasks with a name-only selector returns
    /// only the matching package's tasks.
    #[tokio::test]
    async fn selector_name_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["web", "api"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_build = TaskId::new("web", "build");
        let api_build = TaskId::new("api", "build");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (api_build.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "web".to_string(),
            ..Default::default()
        };

        let tasks =
            super::resolve_selector_to_tasks(&engine, &selector, &pkg_graph, &scm, root, &[])
                .unwrap();

        assert!(tasks.contains(&web_build));
        assert!(!tasks.contains(&api_build));
    }

    /// resolve_selector_to_tasks with include_dependencies traverses
    /// the task graph, not the package graph.
    #[tokio::test]
    async fn selector_include_dependencies_uses_task_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        // web has no package dep on schema
        let pkg_graph = make_pkg_graph(root, &["web", "schema"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_build = TaskId::new("web", "build");
        let schema_gen = TaskId::new("schema", "gen");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (schema_gen.clone(), TaskDefinition::default()),
            ],
            // Task dep: web#build -> schema#gen
            &[(web_build.clone(), schema_gen.clone())],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "web".to_string(),
            include_dependencies: true,
            ..Default::default()
        };

        let tasks =
            super::resolve_selector_to_tasks(&engine, &selector, &pkg_graph, &scm, root, &[])
                .unwrap();

        assert!(
            tasks.contains(&web_build),
            "web#build should be included: {tasks:?}"
        );
        assert!(
            tasks.contains(&schema_gen),
            "schema#gen should be included via task graph traversal: {tasks:?}"
        );
    }

    /// resolve_selector_to_tasks with include_dependents traverses
    /// the task graph backwards.
    #[tokio::test]
    async fn selector_include_dependents_uses_task_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["web", "schema"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_build = TaskId::new("web", "build");
        let schema_gen = TaskId::new("schema", "gen");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (schema_gen.clone(), TaskDefinition::default()),
            ],
            &[(web_build.clone(), schema_gen.clone())],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "schema".to_string(),
            include_dependents: true,
            ..Default::default()
        };

        let tasks =
            super::resolve_selector_to_tasks(&engine, &selector, &pkg_graph, &scm, root, &[])
                .unwrap();

        assert!(
            tasks.contains(&schema_gen),
            "schema#gen should be included: {tasks:?}"
        );
        assert!(
            tasks.contains(&web_build),
            "web#build should be included as a task-level dependent: {tasks:?}"
        );
    }

    /// Regression: filtering to a specific package must NOT pull in
    /// dependent tasks after selector resolution. The engine pruning
    /// should only expand to transitive dependencies (upstream), not
    /// dependents (downstream).
    #[tokio::test]
    async fn filter_does_not_include_dependents() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["ui", "app"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let ui_build = TaskId::new("ui", "build");
        let app_build = TaskId::new("app", "build");

        // app#build depends on ui#build
        let engine = make_engine(
            &[
                (ui_build.clone(), TaskDefinition::default()),
                (app_build.clone(), TaskDefinition::default()),
            ],
            &[(app_build.clone(), ui_build.clone())],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "ui".to_string(),
            ..Default::default()
        };

        let result =
            super::filter_engine_to_tasks(engine, &[selector], &pkg_graph, &scm, root, &[])
                .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&ui_build),
            "ui#build should be retained: {remaining:?}"
        );
        assert!(
            !remaining.contains(&app_build),
            "app#build should NOT be pulled in as a dependent: {remaining:?}"
        );
    }

    /// exclude_self with include_dependencies should include deps but
    /// not the matched package's own tasks.
    #[tokio::test]
    async fn selector_exclude_self_with_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["web", "schema"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_build = TaskId::new("web", "build");
        let schema_gen = TaskId::new("schema", "gen");

        let engine = make_engine(
            &[
                (web_build.clone(), TaskDefinition::default()),
                (schema_gen.clone(), TaskDefinition::default()),
            ],
            &[(web_build.clone(), schema_gen.clone())],
        );

        // ^web... — web's deps but not web itself
        let selector = turborepo_scope::TargetSelector {
            name_pattern: "web".to_string(),
            include_dependencies: true,
            exclude_self: true,
            ..Default::default()
        };

        let tasks =
            super::resolve_selector_to_tasks(&engine, &selector, &pkg_graph, &scm, root, &[])
                .unwrap();

        assert!(
            !tasks.contains(&web_build),
            "web#build should be excluded by exclude_self: {tasks:?}"
        );
        assert!(
            tasks.contains(&schema_gen),
            "schema#gen should still be included as a dep: {tasks:?}"
        );
    }

    /// Glob name pattern should match multiple packages.
    #[tokio::test]
    async fn selector_glob_name_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b", "app"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");
        let app_build = TaskId::new("app", "build");

        let engine = make_engine(
            &[
                (a_build.clone(), TaskDefinition::default()),
                (b_build.clone(), TaskDefinition::default()),
                (app_build.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "lib-*".to_string(),
            ..Default::default()
        };

        let tasks =
            super::resolve_selector_to_tasks(&engine, &selector, &pkg_graph, &scm, root, &[])
                .unwrap();

        assert!(
            tasks.contains(&a_build),
            "lib-a#build should match: {tasks:?}"
        );
        assert!(
            tasks.contains(&b_build),
            "lib-b#build should match: {tasks:?}"
        );
        assert!(
            !tasks.contains(&app_build),
            "app#build should not match lib-* glob: {tasks:?}"
        );
    }
}
