//! Engine builder for constructing task graphs from turbo.json configurations.
//!
//! This module provides `EngineBuilder` which constructs task graphs by:
//! - Loading turbo.json configurations via the `TurboJsonLoader` trait
//! - Resolving task dependencies through the extends chain
//! - Validating task definitions and dependencies
//! - Building the final execution engine
//!
//! The engine builder is the sole layer that validates the task graph for
//! cycles and self-dependencies. Package graph cycles are intentionally allowed
//! — only task graph cycles (e.g. from topological `^` dependencies through a
//! package cycle) prevent execution.

use std::collections::{HashMap, HashSet, VecDeque};

use itertools::Itertools;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_errors::Spanned;
use turborepo_graph_utils as graph;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, PackageNode, ROOT_PKG_NAME},
    toolchain::{TaskIOEnvironment, ToolchainId},
};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{FutureFlags, TurboJson, Validator};
use turborepo_types::TaskDefinition;

use crate::{
    BuilderError, Building, Built, Engine, MissingPackageFromTaskError,
    MissingRootTaskInTurboJsonError, MissingTaskError, TaskNode, TurboJsonLoader,
    validate_task_name,
};

mod definitions;
mod inheritance;

pub use inheritance::{TaskInheritanceResolver, ValidationMode};

/// Builder for constructing a task execution engine.
///
/// The `EngineBuilder` is generic over `L: TurboJsonLoader` to allow different
/// implementations of configuration loading (filesystem, in-memory for tests,
/// etc.)
pub struct EngineBuilder<'a, L: TurboJsonLoader> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    turbo_json_loader: Option<&'a L>,
    is_single: bool,
    workspaces: Vec<PackageName>,
    tasks: Vec<Spanned<TaskName<'static>>>,
    root_enabled_tasks: HashSet<TaskName<'static>>,
    tasks_only: bool,
    add_all_tasks: bool,
    should_validate_engine: bool,
    validator: Validator,
    future_flags: FutureFlags,
    /// When `futureFlags.globalConfiguration` is enabled, these globs are
    /// prepended to every task's inputs instead of being included in the
    /// global hash.
    global_deps: Vec<String>,
    pass_through_args: Vec<String>,
    requested_tasks: Vec<String>,
    /// Each toolchain receives only the startup environment keys it declared.
    environments: HashMap<ToolchainId, TaskIOEnvironment>,
}

impl<'a, L: TurboJsonLoader> EngineBuilder<'a, L> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        turbo_json_loader: &'a L,
        is_single: bool,
    ) -> Self {
        Self {
            repo_root,
            package_graph,
            turbo_json_loader: Some(turbo_json_loader),
            is_single,
            workspaces: Vec::new(),
            tasks: Vec::new(),
            root_enabled_tasks: HashSet::new(),
            tasks_only: false,
            add_all_tasks: false,
            should_validate_engine: true,
            validator: Validator::new(),
            future_flags: FutureFlags::default(),
            global_deps: Vec::new(),
            pass_through_args: Vec::new(),
            requested_tasks: Vec::new(),
            environments: HashMap::new(),
        }
    }

    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.validator = self.validator.with_future_flags(future_flags);
        self.future_flags = future_flags;
        self
    }

    pub fn with_global_deps(mut self, global_deps: Vec<String>) -> Self {
        self.global_deps = global_deps;
        self
    }

    pub fn with_task_io_context(
        mut self,
        pass_through_args: Vec<String>,
        requested_tasks: Vec<String>,
        environments: HashMap<ToolchainId, TaskIOEnvironment>,
    ) -> Self {
        self.pass_through_args = pass_through_args;
        self.requested_tasks = requested_tasks;
        self.environments = environments;
        self
    }

    pub fn with_tasks_only(mut self, tasks_only: bool) -> Self {
        self.tasks_only = tasks_only;
        self
    }

    pub fn with_root_tasks<I: IntoIterator<Item = TaskName<'static>>>(mut self, tasks: I) -> Self {
        self.root_enabled_tasks = tasks
            .into_iter()
            .filter(|name| name.package() == Some(ROOT_PKG_NAME))
            .map(|name| name.into_non_workspace_task())
            .collect();
        self
    }

    pub fn with_workspaces(mut self, workspaces: Vec<PackageName>) -> Self {
        self.workspaces = workspaces;
        self
    }

    pub fn with_tasks<I: IntoIterator<Item = Spanned<TaskName<'static>>>>(
        mut self,
        tasks: I,
    ) -> Self {
        self.tasks = tasks.into_iter().collect();
        self
    }

    /// If set, we will include all tasks in the graph, even if they are not
    /// specified
    pub fn add_all_tasks(mut self) -> Self {
        self.add_all_tasks = true;
        self
    }

    pub fn do_not_validate_engine(mut self) -> Self {
        self.should_validate_engine = false;
        self
    }

    // Returns the set of allowed tasks that can be run if --only is used
    // The set is exactly the product of the packages in filter and tasks specified
    // by CLI
    fn allowed_tasks(&self) -> Option<HashSet<TaskId<'static>>> {
        if self.tasks_only {
            Some(
                self.workspaces
                    .iter()
                    .cartesian_product(self.tasks.iter())
                    .map(|(package, task_name)| {
                        task_name
                            .task_id()
                            .unwrap_or(TaskId::new(package.as_ref(), task_name.task()))
                            .into_owned()
                    })
                    .collect(),
            )
        } else {
            None
        }
    }

    pub fn build(mut self) -> Result<Engine<crate::Built, TaskDefinition>, BuilderError> {
        let turbo_json_loader = self
            .turbo_json_loader
            .take()
            .ok_or(BuilderError::MissingTurboJsonLoader)?;
        let mut missing_tasks: HashMap<&TaskName<'_>, Spanned<()>> =
            HashMap::from_iter(self.tasks.iter().map(|spanned| spanned.as_ref().split()));
        let mut traversal_queue = VecDeque::with_capacity(1);
        let tasks: Vec<Spanned<TaskName<'static>>> = if self.add_all_tasks {
            let mut tasks_set = HashSet::new();

            // Collect tasks from root and its extends chain
            let root_tasks =
                TaskInheritanceResolver::new(turbo_json_loader).resolve(&PackageName::Root)?;
            tasks_set.extend(root_tasks);

            // Collect tasks from each workspace and its extends chain
            for workspace in self.workspaces.iter() {
                let workspace_tasks =
                    TaskInheritanceResolver::new(turbo_json_loader).resolve(workspace)?;
                tasks_set.extend(workspace_tasks);
            }

            tasks_set.into_iter().map(Spanned::new).collect()
        } else {
            self.tasks.clone()
        };

        let entrypoints_span = tracing::info_span!("engine_entrypoints").entered();
        for (workspace, task) in self.workspaces.iter().cartesian_product(tasks.iter()) {
            // When a task uses package#task syntax (e.g. "web#build"), the task_id
            // always resolves to that specific package regardless of which workspace
            // we're iterating over. Skip workspaces that don't match to avoid
            // unnecessary turbo.json lookups across every package in the monorepo.
            if let Some(task_pkg) = task.package()
                && workspace != &PackageName::from(task_pkg)
            {
                continue;
            }

            let task_id = task
                .task_id()
                .unwrap_or_else(|| TaskId::new(workspace.as_ref(), task.task()));

            if Self::has_task_definition_in_run(turbo_json_loader, workspace, task, &task_id)? {
                missing_tasks.remove(task.as_inner());

                // Even if a task definition was found, we _only_ want to add it as an entry
                // point to the task graph (i.e. the traversalQueue), if
                // it's:
                // - A task from the non-root workspace (i.e. tasks from every other workspace)
                // - A task that we *know* is rootEnabled task (in which case, the root
                //   workspace is acceptable)
                if !matches!(workspace, PackageName::Root)
                    || self
                        .root_enabled_tasks
                        .contains(&TaskName::from(task.task()))
                {
                    let task_id = task.to(task_id);
                    traversal_queue.push_back(task_id);
                }
            }
        }

        drop(entrypoints_span);

        {
            let _span = tracing::info_span!("engine_missing_tasks").entered();
            // We can encounter IO errors trying to load turbo.jsons which prevents using
            // `retain` in the standard way. Instead we store the possible error
            // outside of the loop and short circuit checks if we've encountered an error.
            let mut error = None;
            missing_tasks.retain(|task_name, _| {
                // If we've already encountered an error skip checking the rest.
                if error.is_some() {
                    return true;
                }
                match Self::has_task_definition_in_repo(
                    turbo_json_loader,
                    self.package_graph,
                    task_name,
                ) {
                    Ok(has_defn) => !has_defn,
                    Err(e) => {
                        error.get_or_insert(e);
                        true
                    }
                }
            });
            if let Some(err) = error {
                return Err(err);
            }
        }

        if !missing_tasks.is_empty() {
            let missing_pkgs: HashMap<_, _> = missing_tasks
                .keys()
                .filter_map(|task| {
                    let pkg = task.package()?;
                    let missing_pkg = self
                        .package_graph
                        .package_info(&PackageName::from(pkg))
                        .is_none();
                    missing_pkg.then(|| (task.to_string(), pkg.to_string()))
                })
                .collect();
            let mut missing_tasks = missing_tasks
                .into_iter()
                .map(|(task_name, span)| (task_name.to_string(), span))
                .collect::<Vec<_>>();
            // We sort the tasks mostly to keep it deterministic for our tests
            missing_tasks.sort_by(|a, b| a.0.cmp(&b.0));
            let errors = missing_tasks
                .into_iter()
                .map(|(name, span)| {
                    if let Some(pkg) = missing_pkgs.get(&name) {
                        MissingTaskError::MissingPackage { name: pkg.clone() }
                    } else {
                        let (span, text) = span.span_and_text("turbo.json");
                        MissingTaskError::MissingTaskDefinition { name, span, text }
                    }
                })
                .collect();

            return Err(BuilderError::MissingTasks(errors));
        }

        let allowed_tasks = self.allowed_tasks();

        let traversal_span = tracing::info_span!("engine_traversal").entered();
        let mut visited = HashSet::new();
        let mut engine: Engine<Building, TaskDefinition> = Engine::default();
        // Entry points are a floor for the final task count; doubling covers
        // dependency tasks in the common case so the maps allocate once.
        engine.reserve(traversal_queue.len() * 2);
        let mut turbo_json_chain_cache: HashMap<PackageName, Vec<&TurboJson>> = HashMap::new();
        let mut task_def_memo: HashMap<definitions::TaskDefMemoKey, TaskDefinition> =
            HashMap::new();

        while let Some(task_id) = traversal_queue.pop_front() {
            {
                let (task_id, span) = task_id.clone().split();
                engine.add_task_location(task_id.into_owned(), span);
            }

            // Skip before doing expensive work if we've already processed this task.
            if visited.contains(task_id.as_inner()) {
                continue;
            }

            // For root tasks, verify they are either explicitly enabled OR (when using
            // add_all_tasks mode like devtools) have a definition in root turbo.json.
            // Tasks defined without the //#  prefix (like "transit") in root turbo.json
            // are valid root tasks when referenced as dependencies in add_all_tasks mode.
            if task_id.package() == ROOT_PKG_NAME
                && !self
                    .root_enabled_tasks
                    .contains(&task_id.as_non_workspace_task_name())
            {
                // In add_all_tasks mode (devtools), allow root tasks that have a definition
                // in turbo.json even if not explicitly in root_enabled_tasks
                let should_allow = if self.add_all_tasks {
                    let task_name: TaskName<'static> =
                        TaskName::from(task_id.task().to_string()).into_owned();
                    let task_id_owned = task_id.as_inner().clone().into_owned();
                    Self::has_task_definition_in_run(
                        turbo_json_loader,
                        &PackageName::Root,
                        &task_name,
                        &task_id_owned,
                    )?
                } else {
                    false
                };

                if !should_allow {
                    let (span, text) = task_id.span_and_text("turbo.json");
                    return Err(BuilderError::MissingRootTaskInTurboJson(Box::new(
                        MissingRootTaskInTurboJsonError {
                            span,
                            text,
                            task_id: task_id.to_string(),
                        },
                    )));
                }
            }

            validate_task_name(task_id.to(task_id.task()))?;

            if task_id.package() != ROOT_PKG_NAME
                && self
                    .package_graph
                    .package_json(&PackageName::from(task_id.package()))
                    .is_none()
            {
                // If we have a pkg it should be in PackageGraph.
                // If we're hitting this error something has gone wrong earlier when building
                // PackageGraph or the package really doesn't exist and
                // turbo.json is misconfigured.
                let (span, text) = task_id.span_and_text("turbo.json");
                return Err(BuilderError::MissingPackageFromTask(Box::new(
                    MissingPackageFromTaskError {
                        span,
                        text,
                        package: task_id.package().to_string(),
                        task_id: task_id.to_string(),
                    },
                )));
            }

            let task_definition = self.task_definition_cached(
                turbo_json_loader,
                &task_id,
                &task_id.as_non_workspace_task_name(),
                &mut turbo_json_chain_cache,
                &mut task_def_memo,
            )?;

            visited.insert(task_id.as_inner().clone());

            // Note that the Go code has a whole if/else statement for putting stuff into
            // deps or calling e.AddDep the bool is cannot be true so we skip to
            // just doing deps
            let deps = task_definition
                .task_dependencies
                .iter()
                .map(|spanned| spanned.as_ref().split())
                .collect::<HashMap<_, _>>();
            let topo_deps = task_definition
                .topological_dependencies
                .iter()
                .map(|spanned| spanned.as_ref().split())
                .collect::<HashMap<_, _>>();

            // Don't ask why, but for some reason we refer to the source as "to"
            // and the target node as "from"
            let to_task_id = task_id.as_inner().clone().into_owned();
            let to_task_index = engine.get_index(&to_task_id);

            let dep_pkgs = self
                .package_graph
                .immediate_dependencies_iter(&PackageNode::Workspace(to_task_id.package().into()));

            let mut has_deps = false;
            let mut has_topo_deps = false;

            topo_deps
                .iter()
                .cartesian_product(dep_pkgs.into_iter().flatten())
                .for_each(|((from, span), dependency_workspace)| {
                    // We don't need to add an edge from the root node if we're in this branch
                    if let PackageNode::Workspace(dependency_workspace) = dependency_workspace {
                        let from_task_id = TaskId::from_graph(dependency_workspace, from);
                        if let Some(allowed_tasks) = &allowed_tasks
                            && !allowed_tasks.contains(&from_task_id)
                        {
                            return;
                        }
                        let from_task_index = engine.get_index(&from_task_id);
                        has_topo_deps = true;
                        engine
                            .task_graph_mut()
                            .add_edge(to_task_index, from_task_index, ());
                        let from_task_id = span.to(from_task_id);
                        traversal_queue.push_back(from_task_id);
                    }
                });

            for (sibling, span) in task_definition
                .with
                .iter()
                .flatten()
                .map(|s| s.as_ref().split())
            {
                let sibling_task_id = sibling
                    .task_id()
                    .unwrap_or_else(|| TaskId::new(to_task_id.package(), sibling.task()))
                    .into_owned();
                traversal_queue.push_back(span.to(sibling_task_id));
            }

            for (dep, span) in deps {
                let from_task_id = dep
                    .task_id()
                    .unwrap_or_else(|| TaskId::new(to_task_id.package(), dep.task()))
                    .into_owned();
                if let Some(allowed_tasks) = &allowed_tasks
                    && !allowed_tasks.contains(&from_task_id)
                {
                    continue;
                }
                has_deps = true;
                let from_task_index = engine.get_index(&from_task_id);
                engine
                    .task_graph_mut()
                    .add_edge(to_task_index, from_task_index, ());
                let from_task_id = span.to(from_task_id);
                traversal_queue.push_back(from_task_id);
            }

            engine.add_definition(task_id.as_inner().clone().into_owned(), task_definition);
            if !has_deps && !has_topo_deps {
                engine.connect_to_root(&to_task_id);
            }
        }

        drop(traversal_span);

        let _span = tracing::info_span!("engine_validate").entered();
        // This is the sole cycle/self-dependency check in the pipeline. Package
        // graph cycles are intentionally allowed; only task graph cycles prevent
        // execution. See #2559.
        graph::validate_graph(engine.task_graph_mut())?;

        let engine = engine.seal();
        validate_dependency_outputs_inputs(&engine)?;
        Ok(engine)
    }

    /// Returns the path from a task's package directory to the repo root
    pub fn path_to_root(&self, task_id: &TaskId) -> Result<RelativeUnixPathBuf, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let pkg_path = self
            .package_graph
            .package_dir(&package_name)
            .ok_or_else(|| BuilderError::MissingPackageJson {
                workspace: package_name,
            })?;
        Ok(AnchoredSystemPathBuf::relative_path_between(
            &self.repo_root.resolve(pkg_path),
            self.repo_root,
        )
        .to_unix())
    }
}

fn validate_dependency_outputs_inputs(
    engine: &Engine<Built, TaskDefinition>,
) -> Result<(), BuilderError> {
    for task_id in engine.task_ids() {
        let Some(task_definition) = engine.task_definition(task_id) else {
            continue;
        };
        let Some(dependency_outputs) = task_definition.inputs.dependency_outputs.as_ref() else {
            continue;
        };

        let selected = if let Some(from) = dependency_outputs.from.as_ref() {
            let mut selected = Vec::new();
            for selector in from {
                let matches = engine.dependency_output_producers_for_selector(task_id, selector);
                if matches.is_empty()
                    && !selector_allows_empty_match(engine, task_id, task_definition, selector)
                {
                    return Err(structured_input_error(format!(
                        "does not match any eligible dependency task node for this task: \
                         \"dependencyOutputs.from\" contains \"{selector}\".\n\nAdd it to \
                         dependsOn or remove it from dependencyOutputs.from."
                    )));
                }
                selected.extend(matches);
            }
            selected.sort();
            selected.dedup();
            selected
        } else {
            let selected = engine.dependency_output_producers(task_id, None);
            if selected.is_empty() && !has_configured_dependencies(task_definition) {
                return Err(structured_input_error(format!(
                    "dependencyOutputs mode was used for task \"{}\", but this task has no \
                     dependency tasks to select.",
                    task_id.task()
                )));
            }
            selected
        };

        for selected_task in &selected {
            let Some(selected_definition) = engine.task_definition(selected_task) else {
                continue;
            };
            if selected_definition.outputs.inclusions.is_empty()
                && selected_definition.outputs.exclusions.is_empty()
            {
                return Err(structured_input_error(format!(
                    "Selected dependency task \"{}\" does not declare outputs, so it cannot \
                     contribute dependency output inputs.\n\nAdd outputs to \"{}\" or remove it \
                     from dependencyOutputs.from.",
                    selected_task.task(),
                    selected_task.task()
                )));
            }
        }

        if !selected.is_empty() && !dependency_outputs.globs.is_empty() {
            validate_dependency_outputs_globs(&selected, engine, &dependency_outputs.globs)?;
        }
    }

    Ok(())
}

fn selector_allows_empty_match(
    engine: &Engine<Built, TaskDefinition>,
    task_id: &TaskId,
    task_definition: &TaskDefinition,
    selector: &str,
) -> bool {
    let has_dependency_tasks = engine
        .dependencies(task_id)
        .is_some_and(|deps| deps.iter().any(|node| matches!(node, TaskNode::Task(_))));

    if has_dependency_tasks {
        return false;
    }

    let Some(task) = selector.strip_prefix('^') else {
        return false;
    };

    task_definition
        .topological_dependencies
        .iter()
        .any(|dependency| dependency.task() == task)
}

fn has_configured_dependencies(task_definition: &TaskDefinition) -> bool {
    !task_definition.topological_dependencies.is_empty()
        || !task_definition.task_dependencies.is_empty()
}

fn validate_dependency_outputs_globs(
    selected_tasks: &[TaskId<'static>],
    engine: &Engine<Built, TaskDefinition>,
    requested_globs: &[String],
) -> Result<(), BuilderError> {
    for requested_glob in requested_globs {
        if requested_glob.starts_with('!') {
            continue;
        }

        let requested_base = glob_base(requested_glob);
        let matches_selected_outputs = selected_tasks.iter().any(|selected_task| {
            let Some(selected_definition) = engine.task_definition(selected_task) else {
                return false;
            };
            selected_definition
                .outputs
                .inclusions
                .iter()
                .any(|output_glob| glob_base(output_glob).is_prefix_of(&requested_base))
        });

        if !matches_selected_outputs {
            return Err(structured_input_error(format!(
                "dependencyOutputs.globs contains \"{requested_glob}\", but it is not covered by \
                 any selected dependency task outputs.\n\nSelect files from declared outputs or \
                 update the dependency task outputs."
            )));
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct GlobBase<'a>(&'a str);

impl<'a> GlobBase<'a> {
    fn is_prefix_of(&self, other: &GlobBase<'_>) -> bool {
        self.0.is_empty()
            || other.0 == self.0
            || other
                .0
                .strip_prefix(self.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

fn glob_base(glob: &str) -> GlobBase<'_> {
    let glob = glob.strip_prefix('!').unwrap_or(glob);
    let wildcard = glob.find(['*', '?', '[', '{']).unwrap_or(glob.len());
    if wildcard == glob.len() {
        return GlobBase(glob.trim_end_matches('/'));
    }
    let base = glob[..wildcard]
        .rsplit_once('/')
        .map_or("", |(base, _)| base)
        .trim_end_matches('/');
    GlobBase(base)
}

fn structured_input_error(message: String) -> BuilderError {
    BuilderError::TurboJson(turborepo_turbo_json::Error::StructuredInput {
        message,
        span: None,
        text: miette::NamedSource::new("turbo.json", String::new()),
    })
}

#[cfg(test)]
mod test;
