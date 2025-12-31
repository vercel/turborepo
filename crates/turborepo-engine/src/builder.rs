//! Engine builder for constructing task graphs from turbo.json configurations.
//!
//! This module provides `EngineBuilder` which constructs task graphs by:
//! - Loading turbo.json configurations via the `TurboJsonLoader` trait
//! - Resolving task dependencies through the extends chain
//! - Validating task definitions and dependencies
//! - Building the final execution engine

use std::collections::{HashMap, HashSet, VecDeque};

use itertools::Itertools;
use miette::{NamedSource, SourceSpan};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_errors::Spanned;
use turborepo_graph_utils as graph;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode, ROOT_PKG_NAME};
use turborepo_task_id::{TaskId, TaskName};
use turborepo_turbo_json::{
    FutureFlags, HasConfigBeyondExtends, ProcessedTaskDefinition, RawTaskDefinition, TurboJson,
    Validator,
};
use turborepo_types::TaskDefinition;

use crate::{
    BuilderError, Building, CyclicExtends, Engine, MissingPackageFromTaskError,
    MissingPackageTaskError, MissingRootTaskInTurboJsonError, MissingTaskError,
    MissingTurboJsonExtends, TaskDefinitionFromProcessed, TaskDefinitionResult, TurboJsonLoader,
    validate_task_name,
};

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
        }
    }

    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.validator = self.validator.with_future_flags(future_flags);
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
        // If there are no affected packages, we don't need to go through all this work
        // we can just exit early.
        // TODO(mehulkar): but we still need to validate bad task names?
        if self.workspaces.is_empty() {
            return Ok(Engine::default().seal());
        }

        let turbo_json_loader = self
            .turbo_json_loader
            .take()
            .expect("engine builder cannot be constructed without TurboJsonLoader");
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

        for (workspace, task) in self.workspaces.iter().cartesian_product(tasks.iter()) {
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
                if !matches!(workspace, PackageName::Root) || self.root_enabled_tasks.contains(task)
                {
                    let task_id = task.to(task_id);
                    traversal_queue.push_back(task_id);
                }
            }
        }

        {
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
                .iter()
                .filter_map(|(task, _)| {
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

        let mut visited = HashSet::new();
        let mut engine: Engine<Building, TaskDefinition> = Engine::default();

        while let Some(task_id) = traversal_queue.pop_front() {
            {
                let (task_id, span) = task_id.clone().split();
                engine.add_task_location(task_id.into_owned(), span);
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

            let task_definition = self.task_definition(
                turbo_json_loader,
                &task_id,
                &task_id.as_non_workspace_task_name(),
            )?;

            // Skip this iteration of the loop if we've already seen this taskID
            if visited.contains(task_id.as_inner()) {
                continue;
            }

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
                .immediate_dependencies(&PackageNode::Workspace(to_task_id.package().into()));

            let mut has_deps = false;
            let mut has_topo_deps = false;

            topo_deps
                .iter()
                .cartesian_product(dep_pkgs.iter().flatten())
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

        graph::validate_graph(engine.task_graph_mut())?;

        Ok(engine.seal())
    }

    // Helper methods used when building the engine
    /// Checks if there's a task definition somewhere in the repository
    pub fn has_task_definition_in_repo(
        loader: &L,
        package_graph: &PackageGraph,
        task_name: &TaskName<'static>,
    ) -> Result<bool, BuilderError> {
        for (package, _) in package_graph.packages() {
            let task_id = task_name
                .task_id()
                .unwrap_or_else(|| TaskId::new(package.as_str(), task_name.task()));
            if Self::has_task_definition_in_run(loader, package, task_name, &task_id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Checks if there's a task definition in the current run
    pub fn has_task_definition_in_run(
        loader: &L,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
    ) -> Result<bool, BuilderError> {
        let result = Self::has_task_definition_in_run_inner(
            loader,
            workspace,
            task_name,
            task_id,
            &mut HashSet::new(),
        )?;
        Ok(result.has_definition())
    }

    fn has_task_definition_in_run_inner(
        loader: &L,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
        visited: &mut HashSet<PackageName>,
    ) -> Result<TaskDefinitionResult, BuilderError> {
        // Avoid infinite loops from cyclic extends
        if visited.contains(workspace) {
            return Ok(TaskDefinitionResult::not_found());
        }
        visited.insert(workspace.clone());

        let turbo_json = loader.load(workspace).map_or_else(
            |err| {
                if err.is_no_turbo_json() && !matches!(workspace, PackageName::Root) {
                    Ok(None)
                } else {
                    Err(err)
                }
            },
            |turbo_json| Ok(Some(turbo_json)),
        )?;

        let Some(turbo_json) = turbo_json else {
            // If there was no turbo.json in the workspace, fallback to the root turbo.json
            return Self::has_task_definition_in_run_inner(
                loader,
                &PackageName::Root,
                task_name,
                task_id,
                visited,
            );
        };

        let task_id_as_name = task_id.as_task_name();

        // Helper to check task definition status based on extends configuration
        let check_task_def = |task_def: &RawTaskDefinition| -> TaskDefinitionResult {
            let has_extends_false = task_def
                .extends
                .as_ref()
                .map(|e| !*e.as_inner())
                .unwrap_or(false);

            if has_extends_false && !task_def.has_config_beyond_extends() {
                // Task is explicitly excluded via `extends: false` with no config
                TaskDefinitionResult::excluded()
            } else {
                // Task exists (either with `extends: false` + config, or normal definition)
                TaskDefinitionResult::found()
            }
        };

        // Check if this package's turbo.json has the task defined under various key
        // formats
        let base_task_name = TaskName::from(task_name.task());
        let check_base_task = matches!(workspace, PackageName::Root)
            || workspace == &PackageName::from(task_id.package());

        // Try task keys in order of specificity: task_id, task_name, base_task_name
        let task_def = turbo_json
            .tasks
            .get(&task_id_as_name)
            .or_else(|| turbo_json.tasks.get(task_name))
            .or_else(|| {
                if check_base_task {
                    turbo_json.tasks.get(&base_task_name)
                } else {
                    None
                }
            });

        if let Some(task_def) = task_def {
            return Ok(check_task_def(task_def));
        }

        // Check the extends chain for the task definition
        // Track if any package in the chain excluded this task
        for extend in turbo_json.extends.as_inner().iter() {
            let extend_package = PackageName::from(extend.as_str());
            let result = Self::has_task_definition_in_run_inner(
                loader,
                &extend_package,
                task_name,
                task_id,
                visited,
            )?;
            // If any package in the chain excluded this task, propagate that exclusion
            if result.is_excluded() {
                return Ok(TaskDefinitionResult::excluded());
            }
            if result.has_definition() {
                return Ok(TaskDefinitionResult::found());
            }
        }

        // This fallback only applies when there's no explicit `extends` field.
        // If `extends` is present (even if it only contains non-root packages),
        // we don't implicitly fall back to root since the validator ensures
        // the extends chain will eventually reach root.
        if turbo_json.extends.is_empty() && !matches!(workspace, PackageName::Root) {
            return Self::has_task_definition_in_run_inner(
                loader,
                &PackageName::Root,
                task_name,
                task_id,
                visited,
            );
        }

        Ok(TaskDefinitionResult::not_found())
    }

    fn task_definition(
        &self,
        turbo_json_loader: &L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<TaskDefinition, BuilderError> {
        let processed_task_definition = ProcessedTaskDefinition::from_iter(
            self.task_definition_chain(turbo_json_loader, task_id, task_name)?,
        );
        let path_to_root = self.path_to_root(task_id.as_inner())?;
        TaskDefinition::from_processed(processed_task_definition, &path_to_root)
    }

    pub fn task_definition_chain(
        &self,
        turbo_json_loader: &L,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<Vec<ProcessedTaskDefinition>, BuilderError> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
        let mut task_definitions = Vec::new();

        // Find the first package in the chain (iterating in reverse from leaf to root)
        // that has `extends: false` for this task. This stops inheritance from earlier
        // packages.
        let mut extends_false_index: Option<usize> = None;
        for (index, turbo_json) in turbo_json_chain.iter().enumerate().rev() {
            if let Some(task_def) = turbo_json.tasks.get(task_name)
                && task_def
                    .extends
                    .as_ref()
                    .map(|e| !*e.as_inner())
                    .unwrap_or(false)
            {
                // Found `extends: false` for this task in this package
                extends_false_index = Some(index);
                break;
            }
        }

        // If we found extends: false, only process from that point onwards
        if let Some(index) = extends_false_index {
            if let Some(turbo_json) = turbo_json_chain.get(index)
                && let Some(local_def) = turbo_json.task(task_id, task_name)?
                && local_def.has_config_beyond_extends()
            {
                task_definitions.push(local_def);
            }
            // Process any packages after this one (towards the leaf)
            for turbo_json in turbo_json_chain.iter().skip(index + 1) {
                if let Some(workspace_def) = turbo_json.task(task_id, task_name)? {
                    task_definitions.push(workspace_def);
                }
            }
            return Ok(task_definitions);
        }

        // Normal inheritance path
        let mut turbo_json_chain = turbo_json_chain.into_iter();

        if let Some(root_definition) = turbo_json_chain
            .next()
            .expect("root turbo.json is always in chain")
            .task(task_id, task_name)?
        {
            task_definitions.push(root_definition)
        }

        if self.is_single {
            return match task_definitions.is_empty() {
                true => {
                    let (span, text) = task_id.span_and_text("turbo.json");
                    Err(BuilderError::MissingRootTaskInTurboJson(Box::new(
                        MissingRootTaskInTurboJsonError {
                            span,
                            text,
                            task_id: task_id.to_string(),
                        },
                    )))
                }
                false => Ok(task_definitions),
            };
        }

        for turbo_json in turbo_json_chain {
            if let Some(workspace_def) = turbo_json.task(task_id, task_name)? {
                task_definitions.push(workspace_def);
            }
        }

        if task_definitions.is_empty() && self.should_validate_engine {
            let (span, text) = task_id.span_and_text("turbo.json");
            return Err(BuilderError::MissingPackageTask(Box::new(
                MissingPackageTaskError {
                    span,
                    text,
                    task_id: task_id.to_string(),
                    task_name: task_name.to_string(),
                },
            )));
        }

        Ok(task_definitions)
    }

    // Provide the chain of turbo.json's to load to fully resolve all extends for a
    // package turbo.json.
    fn turbo_json_chain<'b>(
        &self,
        turbo_json_loader: &'b L,
        package_name: &PackageName,
    ) -> Result<Vec<&'b TurboJson>, BuilderError> {
        let validator = &self.validator;
        let mut turbo_jsons = Vec::with_capacity(2);

        enum ReadReq {
            // An inferred check we perform for each package to see if there is a package specific
            // turbo.json
            Infer(PackageName),
            // A specifically requested read from a package name being present in `extends`
            Request(Spanned<PackageName>),
        }

        impl ReadReq {
            fn package_name(&self) -> &PackageName {
                match self {
                    ReadReq::Infer(package_name) => package_name,
                    ReadReq::Request(package_name) => package_name.as_inner(),
                }
            }

            fn required(&self) -> Option<(Option<SourceSpan>, NamedSource<String>)> {
                match self {
                    ReadReq::Infer(_) => None,
                    ReadReq::Request(spanned) => Some(spanned.span_and_text("turbo.json")),
                }
            }
        }

        let mut read_stack = vec![(ReadReq::Infer(package_name.clone()), vec![])];
        let mut visited = std::collections::HashSet::new();

        while let Some((read_req, mut path)) = read_stack.pop() {
            let package_name = read_req.package_name();

            // Check for cycle by seeing if this package is already in the current path
            if let Some(cycle_index) = path.iter().position(|p: &PackageName| p == package_name) {
                // Found a cycle - build the cycle portion for error
                let mut cycle = path[cycle_index..]
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>();
                cycle.push(package_name.to_string());

                let (span, text) = read_req
                    .required()
                    .unwrap_or_else(|| (None, NamedSource::new("turbo.json", String::new())));

                return Err(BuilderError::CyclicExtends(Box::new(CyclicExtends {
                    cycle,
                    span,
                    text,
                })));
            }

            // Skip if we've already fully processed this package
            if visited.contains(package_name) {
                continue;
            }

            let turbo_json = turbo_json_loader
                .load(package_name)
                .map(Some)
                .or_else(|err| {
                    if let Some((span, text)) = read_req.required() {
                        if err.is_no_turbo_json() {
                            Err(BuilderError::MissingTurboJsonExtends(Box::new(
                                MissingTurboJsonExtends {
                                    package_name: read_req.package_name().to_string(),
                                    span,
                                    text,
                                },
                            )))
                        } else {
                            Err(err)
                        }
                    } else if err.is_no_turbo_json() {
                        Ok(None)
                    } else {
                        Err(err)
                    }
                })?;
            if let Some(turbo_json) = turbo_json {
                BuilderError::from_validation(
                    validator
                        .validate_turbo_json(package_name, turbo_json)
                        .into_iter()
                        .map(turborepo_config::Error::from)
                        .collect(),
                )?;
                turbo_jsons.push(turbo_json);
                visited.insert(package_name.clone());

                // Add current package to path for cycle detection
                path.push(package_name.clone());

                // Add the new turbo.json we are extending from
                let (extends, span) = turbo_json.extends.clone().split();
                for extend_package in extends {
                    let extend_package_name = PackageName::from(extend_package);
                    read_stack.push((
                        ReadReq::Request(span.clone().to(extend_package_name)),
                        path.clone(),
                    ));
                }
            } else if turbo_jsons.is_empty() {
                // If there is no package turbo.json extend from root by default
                read_stack.push((ReadReq::Infer(PackageName::Root), path));
            }
        }

        Ok(turbo_jsons.into_iter().rev().collect())
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

/// Controls whether validation is performed during task inheritance resolution.
///
/// This enum replaces a boolean flag to make the code's intent clearer at call
/// sites. Validation checks that tasks referenced with `extends: false`
/// actually exist in the extends chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// Validate that `extends: false` references existing tasks.
    /// Used at the entry point of resolution.
    Validate,
    /// Skip validation. Used in recursive calls where validation
    /// has already been performed at the entry point.
    Skip,
}

/// Resolves task inheritance through the extends chain.
///
/// This struct encapsulates the logic for collecting tasks from a turbo.json
/// and its extends chain, handling task-level `extends: false` which can:
/// - Exclude a task entirely (when no other config is provided)
/// - Create a fresh task definition (when other config is provided)
///
/// Task exclusions propagate through the extends chain. If package B
/// excludes a task from package C, and package A extends B, then A will
/// not see that task from C (unless A explicitly re-adds it).
pub struct TaskInheritanceResolver<'a, L: TurboJsonLoader> {
    loader: &'a L,
    /// Controls validation of `extends: false` usage.
    /// Set to `Validate` at entry point, `Skip` in recursive calls.
    validation_mode: ValidationMode,
}

/// Internal state for recursive resolution.
/// Separated from TaskInheritanceResolver to allow sharing the visited set
/// across the entire resolution without cloning.
struct ResolutionState {
    /// Tasks collected from the inheritance chain
    tasks: HashSet<TaskName<'static>>,
    /// Tasks that have been excluded via `extends: false`
    excluded_tasks: HashSet<TaskName<'static>>,
    /// Packages that have been visited to prevent infinite loops.
    /// This is shared across all recursive calls to avoid O(n²) cloning.
    visited: HashSet<PackageName>,
}

impl<'a, L: TurboJsonLoader> TaskInheritanceResolver<'a, L> {
    /// Creates a new resolver for collecting tasks from a workspace.
    pub fn new(loader: &'a L) -> Self {
        Self {
            loader,
            validation_mode: ValidationMode::Validate,
        }
    }

    /// Resolves all tasks from the given workspace and its extends chain.
    pub fn resolve(
        self,
        workspace: &PackageName,
    ) -> Result<HashSet<TaskName<'static>>, BuilderError> {
        let mut state = ResolutionState {
            tasks: HashSet::new(),
            excluded_tasks: HashSet::new(),
            visited: HashSet::new(),
        };
        self.collect_from_workspace(workspace, &mut state)?;
        Ok(state.tasks)
    }

    /// Internal recursive collection that tracks exclusions.
    /// Uses a shared mutable state to avoid cloning the visited set on each
    /// iteration.
    fn collect_from_workspace(
        &self,
        workspace: &PackageName,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        // Avoid infinite loops from cyclic extends
        if state.visited.contains(workspace) {
            return Ok(());
        }
        state.visited.insert(workspace.clone());

        let turbo_json = match self.loader.load(workspace) {
            Ok(json) => json,
            Err(err) if err.is_no_turbo_json() && !matches!(workspace, PackageName::Root) => {
                // If no turbo.json for this workspace, check root
                return self.collect_from_workspace(&PackageName::Root, state);
            }
            Err(err) => return Err(err),
        };

        // Collect inherited tasks from the extends chain
        let (inherited_tasks, chain_exclusions) =
            self.collect_from_extends_chain(turbo_json, state)?;

        // Process tasks from this turbo.json
        self.process_local_tasks(turbo_json, &inherited_tasks, state)?;

        // Add inherited tasks that aren't excluded
        Self::merge_inherited_tasks(inherited_tasks, &chain_exclusions, state);

        // Merge chain exclusions into our exclusions (they propagate up)
        state.excluded_tasks.extend(chain_exclusions);

        Ok(())
    }

    /// Collects tasks from the extends chain of a turbo.json.
    /// Uses the shared visited set from state to avoid O(n²) cloning for deep
    /// chains.
    fn collect_from_extends_chain(
        &self,
        turbo_json: &TurboJson,
        state: &mut ResolutionState,
    ) -> Result<(HashSet<TaskName<'static>>, HashSet<TaskName<'static>>), BuilderError> {
        let mut inherited_tasks = HashSet::new();
        let mut chain_exclusions = HashSet::new();

        for extend in turbo_json.extends.as_inner().iter() {
            let extend_package = PackageName::from(extend.as_str());

            // Skip if already visited (cycle detection without cloning)
            if state.visited.contains(&extend_package) {
                continue;
            }

            // Create a child resolver that skips validation (only validate at entry point)
            let child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                validation_mode: ValidationMode::Skip,
            };

            // Use separate state for child to collect its tasks/exclusions,
            // but share the visited set to avoid cloning
            let mut child_state = ResolutionState {
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                // Take ownership of visited temporarily to avoid cloning
                visited: std::mem::take(&mut state.visited),
            };

            child_resolver.collect_from_workspace(&extend_package, &mut child_state)?;

            // Restore visited set (now includes all packages visited by child)
            state.visited = child_state.visited;

            inherited_tasks.extend(child_state.tasks);
            chain_exclusions.extend(child_state.excluded_tasks);
        }

        // Fallback to root if no explicit extends and not already at root
        if turbo_json.extends.is_empty() && !state.visited.contains(&PackageName::Root) {
            let child_resolver = TaskInheritanceResolver {
                loader: self.loader,
                validation_mode: ValidationMode::Skip,
            };

            // Use separate state for child, sharing visited set
            let mut child_state = ResolutionState {
                tasks: HashSet::new(),
                excluded_tasks: HashSet::new(),
                visited: std::mem::take(&mut state.visited),
            };

            child_resolver.collect_from_workspace(&PackageName::Root, &mut child_state)?;

            // Restore visited set
            state.visited = child_state.visited;

            inherited_tasks.extend(child_state.tasks);
            chain_exclusions.extend(child_state.excluded_tasks);
        }

        Ok((inherited_tasks, chain_exclusions))
    }

    /// Processes tasks defined in the local turbo.json.
    fn process_local_tasks(
        &self,
        turbo_json: &TurboJson,
        inherited_tasks: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        for (task_name, task_def) in turbo_json.tasks.iter() {
            match task_def.extends.as_ref().map(|s| *s.as_inner()) {
                Some(false) => {
                    self.handle_excluded_task(
                        turbo_json,
                        task_name,
                        task_def,
                        inherited_tasks,
                        state,
                    )?;
                }
                _ => {
                    // Normal task or explicit `extends: true` - add it
                    state.tasks.insert(task_name.clone());
                }
            }
        }
        Ok(())
    }

    /// Handles a task with `extends: false`.
    fn handle_excluded_task(
        &self,
        turbo_json: &TurboJson,
        task_name: &TaskName<'static>,
        task_def: &RawTaskDefinition,
        inherited_tasks: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) -> Result<(), BuilderError> {
        // Validate that the task exists in the extends chain (only at entry point)
        if self.validation_mode == ValidationMode::Validate && !inherited_tasks.contains(task_name)
        {
            let (span, text) = task_def
                .extends
                .as_ref()
                .unwrap()
                .span_and_text("turbo.json");
            let extends_chain = Self::format_extends_chain(turbo_json, inherited_tasks);
            return Err(BuilderError::TurboJson(
                turborepo_turbo_json::Error::TaskNotInExtendsChain {
                    task_name: task_name.to_string(),
                    extends_chain,
                    span,
                    text,
                },
            ));
        }

        if task_def.has_config_beyond_extends() {
            // Has other config - this is a fresh definition, add it
            state.tasks.insert(task_name.clone());
        }
        // Track as excluded (propagates to parent packages)
        state.excluded_tasks.insert(task_name.clone());
        Ok(())
    }

    /// Merges inherited tasks that aren't excluded.
    fn merge_inherited_tasks(
        inherited_tasks: HashSet<TaskName<'static>>,
        chain_exclusions: &HashSet<TaskName<'static>>,
        state: &mut ResolutionState,
    ) {
        for task in inherited_tasks {
            if !state.excluded_tasks.contains(&task) && !chain_exclusions.contains(&task) {
                state.tasks.insert(task);
            }
        }
    }

    /// Formats the extends chain for error messages.
    fn format_extends_chain(
        turbo_json: &TurboJson,
        available_tasks: &HashSet<TaskName<'static>>,
    ) -> String {
        let mut result = String::new();
        result.push_str("The extends chain includes:\n");

        let extends = turbo_json.extends.as_inner();
        if extends.is_empty() {
            result.push_str("  → // (root)\n");
        } else {
            for extend in extends {
                result.push_str(&format!("  → {}\n", extend));
            }
        }

        result.push_str("\nTasks available from extends chain:\n");
        if available_tasks.is_empty() {
            result.push_str("  (none)\n");
        } else {
            let mut sorted_tasks: Vec<_> = available_tasks.iter().collect();
            sorted_tasks.sort();
            for task in sorted_tasks {
                result.push_str(&format!("  • {}\n", task));
            }
        }

        result
    }
}
