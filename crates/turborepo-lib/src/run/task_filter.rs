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

/// Resolves an `--affected` range to the set of task IDs that are affected
/// (changed + dependents). Used by the builder to compute an intersection
/// constraint when both `--affected` and `--filter` are active.
pub fn resolve_affected_tasks(
    engine: &Engine,
    affected_range: &(Option<String>, Option<String>),
    pkg_dep_graph: &PackageGraph,
    scm: &SCM,
    repo_root: &AbsoluteSystemPath,
    global_deps: &[String],
) -> Result<HashSet<TaskId<'static>>, crate::run::error::Error> {
    let selector = TargetSelector {
        git_range: Some(GitRange {
            from_ref: affected_range.0.clone(),
            to_ref: affected_range.1.clone(),
            include_uncommitted: true,
            allow_unknown_objects: true,
            merge_base: true,
        }),
        include_dependents: true,
        ..Default::default()
    };
    resolve_selector_to_tasks(
        engine,
        &selector,
        pkg_dep_graph,
        scm,
        repo_root,
        global_deps,
    )
}

/// Filters an engine down to only the tasks matching the given selectors.
///
/// Each include selector contributes a set of tasks (unioned together).
/// Exclude selectors remove tasks from the result. When
/// `affected_constraint` is provided, the included tasks are intersected
/// with it before excludes are applied.
pub fn filter_engine_to_tasks(
    engine: Engine,
    selectors: &[TargetSelector],
    affected_constraint: Option<&HashSet<TaskId<'static>>>,
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

    if let Some(affected) = affected_constraint {
        included_tasks.retain(|t| affected.contains(t));
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

    // `with` relationships (used by microfrontends to co-schedule proxy
    // tasks alongside dev tasks) create no graph edges, so
    // retain_filtered_tasks' forward DFS would miss them. Expand the
    // included set to cover `with` siblings before pruning.
    let included_tasks = expand_with_siblings(&engine, included_tasks);

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

/// Expands a task set to include all `with` siblings, transitively.
///
/// The `with` field on a task definition means "co-schedule these sibling
/// tasks alongside me." Unlike `dependsOn`, `with` creates no graph edges
/// in the task graph — siblings are simply pushed into the engine builder's
/// traversal queue during construction.
///
/// When `retain_filtered_tasks` prunes the engine via forward DFS, edge-less
/// `with` siblings are unreachable and get dropped. This function closes that
/// gap by expanding the retained set before pruning.
fn expand_with_siblings(
    engine: &Engine,
    tasks: HashSet<TaskId<'static>>,
) -> HashSet<TaskId<'static>> {
    let mut result = tasks;
    let mut frontier: Vec<TaskId<'static>> = result.iter().cloned().collect();

    while let Some(task_id) = frontier.pop() {
        let Some(definition) = engine.task_definition(&task_id) else {
            continue;
        };
        let Some(with) = &definition.with else {
            continue;
        };
        for spanned_name in with {
            let name = spanned_name.as_inner();
            let sibling_id = name
                .task_id()
                .map(|id| id.into_owned())
                .unwrap_or_else(|| TaskId::new(task_id.package(), name.task()).into_owned());

            if result.insert(sibling_id.clone()) {
                frontier.push(sibling_id);
            }
        }
    }

    let added = result.len().saturating_sub(frontier.capacity());
    if added > 0 {
        tracing::debug!(
            expanded = result.len(),
            "expanded task filter set with `with` siblings"
        );
    }

    result
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
            super::filter_engine_to_tasks(engine, &[selector], None, &pkg_graph, &scm, root, &[])
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

    /// Include selector + affected constraint → only tasks matching both.
    ///
    /// Engine: ui#build, app#build (app depends on ui).
    /// Selector: name=ui (selects ui#build, engine retains ui#build).
    /// Affected: {app#build} (only app is affected).
    /// Result: empty — ui#build is selected by filter but not affected.
    #[tokio::test]
    async fn affected_constraint_intersects_with_include() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["ui", "app"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let ui_build = TaskId::new("ui", "build");
        let app_build = TaskId::new("app", "build");

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

        let affected: HashSet<TaskId<'static>> = [app_build.clone()].into_iter().collect();

        let result = super::filter_engine_to_tasks(
            engine,
            &[selector],
            Some(&affected),
            &pkg_graph,
            &scm,
            root,
            &[],
        )
        .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            !remaining.contains(&ui_build),
            "ui#build is not affected and should be excluded: {remaining:?}"
        );
    }

    /// Exclude selector + affected constraint → affected minus excluded.
    ///
    /// Engine: ui#build, app#build, lib#build (no edges between them).
    /// Selector: exclude ui.
    /// Affected: {ui#build, app#build}.
    /// Result: {app#build} — ui is excluded, lib is not affected.
    #[tokio::test]
    async fn affected_constraint_with_exclude_selector() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["ui", "app", "lib"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let ui_build = TaskId::new("ui", "build");
        let app_build = TaskId::new("app", "build");
        let lib_build = TaskId::new("lib", "build");

        // No dependency edges — tasks are independent
        let engine = make_engine(
            &[
                (ui_build.clone(), TaskDefinition::default()),
                (app_build.clone(), TaskDefinition::default()),
                (lib_build.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let exclude_selector = turborepo_scope::TargetSelector {
            name_pattern: "ui".to_string(),
            exclude: true,
            ..Default::default()
        };

        let affected: HashSet<TaskId<'static>> =
            [ui_build.clone(), app_build.clone()].into_iter().collect();

        let result = super::filter_engine_to_tasks(
            engine,
            &[exclude_selector],
            Some(&affected),
            &pkg_graph,
            &scm,
            root,
            &[],
        )
        .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&app_build),
            "app#build is affected and not excluded: {remaining:?}"
        );
        assert!(
            !remaining.contains(&ui_build),
            "ui#build should be excluded: {remaining:?}"
        );
        assert!(
            !remaining.contains(&lib_build),
            "lib#build is not affected: {remaining:?}"
        );
    }

    /// Empty selectors + affected constraint → only affected tasks survive.
    ///
    /// Engine: ui#build, app#build, lib#build.
    /// Selectors: none.
    /// Affected: {app#build}.
    /// Result: {app#build} (plus ui#build as a transitive dep for execution).
    #[tokio::test]
    async fn affected_constraint_with_empty_selectors() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["ui", "app", "lib"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let ui_build = TaskId::new("ui", "build");
        let app_build = TaskId::new("app", "build");
        let lib_build = TaskId::new("lib", "build");

        // app depends on ui
        let engine = make_engine(
            &[
                (ui_build.clone(), TaskDefinition::default()),
                (app_build.clone(), TaskDefinition::default()),
                (lib_build.clone(), TaskDefinition::default()),
            ],
            &[(app_build.clone(), ui_build.clone())],
        );

        let affected: HashSet<TaskId<'static>> = [app_build.clone()].into_iter().collect();

        let result = super::filter_engine_to_tasks(
            engine,
            &[],
            Some(&affected),
            &pkg_graph,
            &scm,
            root,
            &[],
        )
        .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&app_build),
            "app#build is affected: {remaining:?}"
        );
        assert!(
            remaining.contains(&ui_build),
            "ui#build should be retained as a transitive dependency of app#build: {remaining:?}"
        );
        assert!(
            !remaining.contains(&lib_build),
            "lib#build is not affected and not a dependency: {remaining:?}"
        );
    }

    /// A task with $TURBO_ROOT$ inputs should survive filtering when
    /// the affected constraint includes it (simulating a root-level file
    /// change detected by git-range resolution).
    ///
    /// This tests the integration path: affected_constraint intersection
    /// with name selector → retain_filtered_tasks preserves the task +
    /// its transitive dependencies.
    #[tokio::test]
    async fn turbo_root_task_survives_filter_with_affected_constraint() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");

        // lib-a has $TURBO_ROOT$ inputs (expanded to ../../config.txt).
        // lib-b has default inputs.
        let engine = make_engine(
            &[
                (
                    a_build.clone(),
                    def_with_inputs(&["../../config.txt"], true),
                ),
                (b_build.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        // Simulate git-range resolution finding that only lib-a#build
        // is affected (its $TURBO_ROOT$ input matched a changed root file).
        let affected: HashSet<TaskId<'static>> = [a_build.clone()].into_iter().collect();

        // Filter to lib-a + affected constraint.
        let selector = turborepo_scope::TargetSelector {
            name_pattern: "lib-a".to_string(),
            ..Default::default()
        };

        let result = super::filter_engine_to_tasks(
            engine,
            &[selector],
            Some(&affected),
            &pkg_graph,
            &scm,
            root,
            &[],
        )
        .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&a_build),
            "lib-a#build should survive: matched by name AND affected via $TURBO_ROOT$ input: \
             {remaining:?}"
        );
        assert!(
            !remaining.contains(&b_build),
            "lib-b#build should be excluded: not in filter scope: {remaining:?}"
        );
    }

    /// When affected_constraint excludes a task whose $TURBO_ROOT$ input
    /// didn't match, the filter should respect that even if the name
    /// selector would include it.
    #[tokio::test]
    async fn turbo_root_task_excluded_when_not_affected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["lib-a", "lib-b"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let a_build = TaskId::new("lib-a", "build");
        let b_build = TaskId::new("lib-b", "build");

        let engine = make_engine(
            &[
                (
                    a_build.clone(),
                    def_with_inputs(&["../../config.txt"], true),
                ),
                (b_build.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        // Simulate: only lib-b is affected (a source file in lib-b changed).
        // lib-a's $TURBO_ROOT$ input did NOT match.
        let affected: HashSet<TaskId<'static>> = [b_build.clone()].into_iter().collect();

        // Filter to lib-a, but affected says lib-a isn't affected.
        let selector = turborepo_scope::TargetSelector {
            name_pattern: "lib-a".to_string(),
            ..Default::default()
        };

        let result = super::filter_engine_to_tasks(
            engine,
            &[selector],
            Some(&affected),
            &pkg_graph,
            &scm,
            root,
            &[],
        )
        .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.is_empty(),
            "no tasks should survive: lib-a matches name but not affected: {remaining:?}"
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

    fn def_with_siblings(siblings: &[&str]) -> TaskDefinition {
        use turborepo_errors::Spanned;
        use turborepo_task_id::TaskName;

        TaskDefinition {
            with: Some(
                siblings
                    .iter()
                    .map(|s| Spanned::new(TaskName::from(*s).into_owned()))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    /// expand_with_siblings should include cross-package `with` siblings.
    ///
    /// This is the core microfrontends scenario: docs#dev has
    /// `with: ["web#proxy"]` but no graph edge to web#proxy.
    #[test]
    fn expand_with_siblings_cross_package() {
        let docs_dev = TaskId::new("docs", "dev");
        let web_proxy = TaskId::new("web", "proxy");

        let engine = make_engine(
            &[
                (docs_dev.clone(), def_with_siblings(&["web#proxy"])),
                (web_proxy.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let initial: HashSet<_> = [docs_dev.clone()].into_iter().collect();
        let expanded = super::expand_with_siblings(&engine, initial);

        assert!(
            expanded.contains(&docs_dev),
            "original task should survive: {expanded:?}"
        );
        assert!(
            expanded.contains(&web_proxy),
            "with sibling web#proxy should be included: {expanded:?}"
        );
    }

    /// expand_with_siblings should handle same-package siblings
    /// (no package prefix in the `with` value).
    #[test]
    fn expand_with_siblings_same_package() {
        let web_dev = TaskId::new("web", "dev");
        let web_proxy = TaskId::new("web", "proxy");

        let engine = make_engine(
            &[
                (web_dev.clone(), def_with_siblings(&["proxy"])),
                (web_proxy.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let initial: HashSet<_> = [web_dev.clone()].into_iter().collect();
        let expanded = super::expand_with_siblings(&engine, initial);

        assert!(expanded.contains(&web_dev));
        assert!(
            expanded.contains(&web_proxy),
            "same-package with sibling should be included: {expanded:?}"
        );
    }

    /// expand_with_siblings should follow chains transitively.
    /// a has with:[b], b has with:[c] → filtering to a includes b and c.
    #[test]
    fn expand_with_siblings_transitive() {
        let a = TaskId::new("a", "dev");
        let b = TaskId::new("b", "dev");
        let c = TaskId::new("c", "dev");

        let engine = make_engine(
            &[
                (a.clone(), def_with_siblings(&["b#dev"])),
                (b.clone(), def_with_siblings(&["c#dev"])),
                (c.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let initial: HashSet<_> = [a.clone()].into_iter().collect();
        let expanded = super::expand_with_siblings(&engine, initial);

        assert_eq!(
            expanded.len(),
            3,
            "all three should be included: {expanded:?}"
        );
        assert!(expanded.contains(&a));
        assert!(expanded.contains(&b));
        assert!(expanded.contains(&c));
    }

    /// Full microfrontends scenario: filtering to a child MFE package
    /// should retain the proxy task from the parent package.
    ///
    /// Setup:
    /// - web owns microfrontends.json, has web#dev (with: web#proxy)
    /// - docs is a child MFE app, has docs#dev (with: web#proxy)
    /// - web#proxy has a dependency on mfe-pkg#build
    /// - --filter=docs should produce: docs#dev, web#proxy, mfe-pkg#build
    #[tokio::test]
    async fn filter_retains_mfe_proxy_for_child_package() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["web", "docs", "mfe-pkg"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_dev = TaskId::new("web", "dev");
        let docs_dev = TaskId::new("docs", "dev");
        let web_proxy = TaskId::new("web", "proxy");
        let mfe_build = TaskId::new("mfe-pkg", "build");

        let engine = make_engine(
            &[
                (web_dev.clone(), def_with_siblings(&["web#proxy"])),
                (docs_dev.clone(), def_with_siblings(&["web#proxy"])),
                (web_proxy.clone(), TaskDefinition::default()),
                (mfe_build.clone(), TaskDefinition::default()),
            ],
            // web#proxy depends on mfe-pkg#build (like @vercel/microfrontends#build)
            &[(web_proxy.clone(), mfe_build.clone())],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "docs".to_string(),
            ..Default::default()
        };

        let result =
            super::filter_engine_to_tasks(engine, &[selector], None, &pkg_graph, &scm, root, &[])
                .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&docs_dev),
            "docs#dev should be retained: {remaining:?}"
        );
        assert!(
            remaining.contains(&web_proxy),
            "web#proxy should be retained via `with` expansion: {remaining:?}"
        );
        assert!(
            remaining.contains(&mfe_build),
            "mfe-pkg#build should be retained as a dependency of web#proxy: {remaining:?}"
        );
        assert!(
            !remaining.contains(&web_dev),
            "web#dev should NOT be retained (not in filter): {remaining:?}"
        );
    }

    /// Filtering to the parent MFE package should also retain the proxy.
    #[tokio::test]
    async fn filter_retains_mfe_proxy_for_parent_package() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmp.path()).unwrap();
        let pkg_graph = make_pkg_graph(root, &["web", "docs"]).await;
        let scm = turborepo_scm::SCM::new(root);

        let web_dev = TaskId::new("web", "dev");
        let docs_dev = TaskId::new("docs", "dev");
        let web_proxy = TaskId::new("web", "proxy");

        let engine = make_engine(
            &[
                (web_dev.clone(), def_with_siblings(&["web#proxy"])),
                (docs_dev.clone(), def_with_siblings(&["web#proxy"])),
                (web_proxy.clone(), TaskDefinition::default()),
            ],
            &[],
        );

        let selector = turborepo_scope::TargetSelector {
            name_pattern: "web".to_string(),
            ..Default::default()
        };

        let result =
            super::filter_engine_to_tasks(engine, &[selector], None, &pkg_graph, &scm, root, &[])
                .unwrap();

        let remaining: HashSet<_> = result.task_ids().cloned().collect();
        assert!(
            remaining.contains(&web_dev),
            "web#dev should be retained: {remaining:?}"
        );
        assert!(
            remaining.contains(&web_proxy),
            "web#proxy should be retained via `with` expansion: {remaining:?}"
        );
        assert!(
            !remaining.contains(&docs_dev),
            "docs#dev should NOT be retained (not in filter): {remaining:?}"
        );
    }
}
