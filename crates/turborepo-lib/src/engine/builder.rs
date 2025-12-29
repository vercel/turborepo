use std::collections::{HashMap, HashSet, VecDeque};

use convert_case::{Case, Casing};
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, SourceSpan};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_errors::{Spanned, TURBO_SITE};
use turborepo_graph_utils as graph;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode, ROOT_PKG_NAME};
use turborepo_task_id::{TaskId, TaskName};

use super::{task_inheritance::TaskInheritanceResolver, Engine};
use crate::{
    config,
    task_graph::TaskDefinition,
    turbo_json::{
        validator::Validator, FutureFlags, HasConfigBeyondExtends, ProcessedTaskDefinition,
        RawTaskDefinition, TaskDefinitionFromProcessed, TurboJson, TurboJsonLoader,
    },
};

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum MissingTaskError {
    #[error("Could not find task `{name}` in project")]
    MissingTaskDefinition {
        name: String,
        #[label]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Could not find package `{name}` in project")]
    MissingPackage { name: String },
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Could not find \"{task_id}\" in root turbo.json or \"{task_name}\" in package")]
pub struct MissingPackageTaskError {
    #[label]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
    pub task_id: String,
    pub task_name: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Could not find package \"{package}\" referenced by task \"{task_id}\" in project")]
pub struct MissingPackageFromTaskError {
    #[label]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
    pub package: String,
    pub task_id: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Invalid task name: {reason}")]
pub struct InvalidTaskNameError {
    #[label]
    span: Option<SourceSpan>,
    #[source_code]
    text: NamedSource<String>,
    task_name: String,
    reason: String,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error(
    "{task_id} requires an entry in turbo.json before it can be depended on because it is a task \
     declared in the root package.json"
)]
#[diagnostic(
    code(missing_root_task_in_turbo_json),
    url(
            "{}/messages/{}",
            TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab)
    )
)]
pub struct MissingRootTaskInTurboJsonError {
    task_id: String,
    #[label("Add an entry in turbo.json for this task")]
    span: Option<SourceSpan>,
    #[source_code]
    text: NamedSource<String>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Cannot extend from '{package_name}' without a package 'turbo.json'.")]
pub struct MissingTurboJsonExtends {
    package_name: String,
    #[label("Extended from here")]
    span: Option<SourceSpan>,
    #[source_code]
    text: NamedSource<String>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("Cyclic \"extends\" detected: {}", cycle.join(" -> "))]
pub struct CyclicExtends {
    cycle: Vec<String>,
    #[label("Cycle detected here")]
    span: Option<SourceSpan>,
    #[source_code]
    text: NamedSource<String>,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("Missing tasks in project")]
    MissingTasks(#[related] Vec<MissingTaskError>),
    #[error("No package.json found for {workspace}")]
    MissingPackageJson { workspace: PackageName },
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingRootTaskInTurboJson(Box<MissingRootTaskInTurboJsonError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingPackageFromTask(Box<MissingPackageFromTaskError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingPackageTask(Box<MissingPackageTaskError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingTurboJsonExtends(Box<MissingTurboJsonExtends>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CyclicExtends(Box<CyclicExtends>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] crate::config::Error),
    #[error("Invalid turbo.json configuration")]
    Validation {
        #[related]
        errors: Vec<config::Error>,
    },
    #[error(transparent)]
    Graph(#[from] graph::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidTaskName(Box<InvalidTaskNameError>),
}

/// Result of checking if a task has a definition in the current run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskDefinitionResult {
    /// True if the task has a valid definition.
    has_definition: bool,
    /// True if the task was excluded via `extends: false` somewhere in the
    /// chain.
    is_excluded: bool,
}

impl TaskDefinitionResult {
    fn new(has_definition: bool, is_excluded: bool) -> Self {
        Self {
            has_definition,
            is_excluded,
        }
    }

    /// Creates a result indicating no definition was found.
    fn not_found() -> Self {
        Self::new(false, false)
    }

    /// Creates a result indicating the task was explicitly excluded.
    fn excluded() -> Self {
        Self::new(false, true)
    }

    /// Creates a result indicating a definition was found.
    fn found() -> Self {
        Self::new(true, false)
    }
}

pub struct EngineBuilder<'a> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    turbo_json_loader: Option<&'a TurboJsonLoader>,
    is_single: bool,
    workspaces: Vec<PackageName>,
    tasks: Vec<Spanned<TaskName<'static>>>,
    root_enabled_tasks: HashSet<TaskName<'static>>,
    tasks_only: bool,
    add_all_tasks: bool,
    should_validate_engine: bool,
    validator: Validator,
}

impl<'a> EngineBuilder<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        turbo_json_loader: &'a TurboJsonLoader,
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

    pub fn build(mut self) -> Result<super::Engine, Error> {
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

            return Err(Error::MissingTasks(errors));
        }

        let allowed_tasks = self.allowed_tasks();

        let mut visited = HashSet::new();
        let mut engine = Engine::default();

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
                    return Err(Error::MissingRootTaskInTurboJson(Box::new(
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
                return Err(Error::MissingPackageFromTask(Box::new(
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
                        if let Some(allowed_tasks) = &allowed_tasks {
                            if !allowed_tasks.contains(&from_task_id) {
                                return;
                            }
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
                if let Some(allowed_tasks) = &allowed_tasks {
                    if !allowed_tasks.contains(&from_task_id) {
                        continue;
                    }
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
    fn has_task_definition_in_repo(
        loader: &TurboJsonLoader,
        package_graph: &PackageGraph,
        task_name: &TaskName<'static>,
    ) -> Result<bool, Error> {
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
    fn has_task_definition_in_run(
        loader: &TurboJsonLoader,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
    ) -> Result<bool, Error> {
        let result = Self::has_task_definition_in_run_inner(
            loader,
            workspace,
            task_name,
            task_id,
            &mut HashSet::new(),
        )?;
        Ok(result.has_definition)
    }

    fn has_task_definition_in_run_inner(
        loader: &TurboJsonLoader,
        workspace: &PackageName,
        task_name: &TaskName<'static>,
        task_id: &TaskId,
        visited: &mut HashSet<PackageName>,
    ) -> Result<TaskDefinitionResult, Error> {
        // Avoid infinite loops from cyclic extends
        if visited.contains(workspace) {
            return Ok(TaskDefinitionResult::not_found());
        }
        visited.insert(workspace.clone());

        let turbo_json = loader.load(workspace).map_or_else(
            |err| {
                if matches!(err, config::Error::NoTurboJSON)
                    && !matches!(workspace, PackageName::Root)
                {
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
            if result.is_excluded {
                return Ok(TaskDefinitionResult::excluded());
            }
            if result.has_definition {
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

    /// Collects all task names from a turbo.json and its extends chain.
    ///
    /// This is a convenience wrapper around `TaskInheritanceResolver` that
    /// maintains the original API for compatibility with existing code and
    /// tests.
    #[cfg(test)]
    fn collect_tasks_from_extends_chain(
        loader: &TurboJsonLoader,
        workspace: &PackageName,
        tasks: &mut HashSet<TaskName<'static>>,
        _visited: &mut HashSet<PackageName>,
    ) -> Result<(), Error> {
        let resolved_tasks = TaskInheritanceResolver::new(loader).resolve(workspace)?;
        tasks.extend(resolved_tasks);
        Ok(())
    }

    fn task_definition(
        &self,
        turbo_json_loader: &TurboJsonLoader,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<TaskDefinition, Error> {
        let processed_task_definition = ProcessedTaskDefinition::from_iter(
            self.task_definition_chain(turbo_json_loader, task_id, task_name)?,
        );
        let path_to_root = self.path_to_root(task_id.as_inner())?;
        Ok(TaskDefinition::from_processed(
            processed_task_definition,
            &path_to_root,
        )?)
    }

    fn task_definition_chain(
        &self,
        turbo_json_loader: &TurboJsonLoader,
        task_id: &Spanned<TaskId>,
        task_name: &TaskName,
    ) -> Result<Vec<ProcessedTaskDefinition>, Error> {
        let package_name = PackageName::from(task_id.package());
        let turbo_json_chain = self.turbo_json_chain(turbo_json_loader, &package_name)?;
        let mut task_definitions = Vec::new();

        // Find the first package in the chain (iterating in reverse from leaf to root)
        // that has `extends: false` for this task. This stops inheritance from earlier
        // packages.
        let mut extends_false_index: Option<usize> = None;
        for (index, turbo_json) in turbo_json_chain.iter().enumerate().rev() {
            if let Some(task_def) = turbo_json.tasks.get(task_name) {
                if task_def
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
        }

        // If we found extends: false, only process from that point onwards
        if let Some(index) = extends_false_index {
            if let Some(turbo_json) = turbo_json_chain.get(index) {
                if let Some(local_def) = turbo_json.task(task_id, task_name)? {
                    if local_def.has_config_beyond_extends() {
                        task_definitions.push(local_def);
                    }
                }
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
                    Err(Error::MissingRootTaskInTurboJson(Box::new(
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
            return Err(Error::MissingPackageTask(Box::new(
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
        turbo_json_loader: &'b TurboJsonLoader,
        package_name: &PackageName,
    ) -> Result<Vec<&'b TurboJson>, Error> {
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

                return Err(Error::CyclicExtends(Box::new(CyclicExtends {
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
                        if matches!(err, config::Error::NoTurboJSON) {
                            Err(Error::MissingTurboJsonExtends(Box::new(
                                MissingTurboJsonExtends {
                                    package_name: read_req.package_name().to_string(),
                                    span,
                                    text,
                                },
                            )))
                        } else {
                            Err(err.into())
                        }
                    } else if matches!(err, config::Error::NoTurboJSON) {
                        Ok(None)
                    } else {
                        Err(err.into())
                    }
                })?;
            if let Some(turbo_json) = turbo_json {
                Error::from_validation(validator.validate_turbo_json(package_name, turbo_json))?;
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

    // Returns that path from a task's package directory to the repo root
    fn path_to_root(&self, task_id: &TaskId) -> Result<RelativeUnixPathBuf, Error> {
        let package_name = PackageName::from(task_id.package());
        let pkg_path = self
            .package_graph
            .package_dir(&package_name)
            .ok_or_else(|| Error::MissingPackageJson {
                workspace: package_name,
            })?;
        Ok(AnchoredSystemPathBuf::relative_path_between(
            &self.repo_root.resolve(pkg_path),
            self.repo_root,
        )
        .to_unix())
    }
}

impl Error {
    fn is_missing_turbo_json(&self) -> bool {
        matches!(self, Self::Config(crate::config::Error::NoTurboJSON))
    }

    fn from_validation(errors: Vec<config::Error>) -> Result<(), Self> {
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Validation { errors })
        }
    }
}

// If/when we decide to be stricter about task names,
// we can expand the patterns here.
const INVALID_TOKENS: &[&str] = &["$colon$"];

fn validate_task_name(task: Spanned<&str>) -> Result<(), Error> {
    INVALID_TOKENS
        .iter()
        .find(|token| task.contains(**token))
        .map(|found_token| {
            let (span, text) = task.span_and_text("turbo.json");
            Err(Error::InvalidTaskName(Box::new(InvalidTaskNameError {
                span,
                text,
                task_name: task.to_string(),
                reason: format!("task contains invalid string '{found_token}'"),
            })))
        })
        .unwrap_or(Ok(()))
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use insta::{assert_json_snapshot, assert_snapshot};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::TempDir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_lockfiles::Lockfile;
    use turborepo_repository::{
        discovery::PackageDiscovery, package_json::PackageJson, package_manager::PackageManager,
    };

    use super::*;
    use crate::{
        engine::TaskNode,
        turbo_json::{RawPackageTurboJson, RawRootTurboJson, RawTurboJson, TurboJson},
    };

    // Only used to prevent package graph construction from attempting to read
    // lockfile from disk
    #[derive(Debug)]
    struct MockLockfile;
    impl Lockfile for MockLockfile {
        fn resolve_package(
            &self,
            _workspace_path: &str,
            _name: &str,
            _version: &str,
        ) -> Result<Option<turborepo_lockfiles::Package>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn all_dependencies(
            &self,
            _key: &str,
        ) -> Result<Option<HashMap<String, String>>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn subgraph(
            &self,
            _workspace_packages: &[String],
            _packages: &[String],
        ) -> Result<Box<dyn Lockfile>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn encode(&self) -> Result<Vec<u8>, turborepo_lockfiles::Error> {
            unreachable!()
        }

        fn global_change(&self, _other: &dyn Lockfile) -> bool {
            unreachable!()
        }

        fn turbo_version(&self) -> Option<String> {
            None
        }
    }

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(turborepo_repository::discovery::DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![], // we don't care about this
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            self.discover_packages().await
        }
    }

    macro_rules! package_jsons {
        {$root:expr, $($name:expr => $deps:expr),+} => {
            {
                let mut _map = HashMap::new();
                $(
                    let path = $root.join_components(&["packages", $name, "package.json"]);
                    let dependencies = Some($deps.iter().map(|dep: &&str| (dep.to_string(), "workspace:*".to_string())).collect());
                    let package_json = PackageJson { name: Some(Spanned::new($name.to_string())), dependencies, ..Default::default() };
                    _map.insert(path, package_json);
                )+
                _map
            }
        };
    }

    fn mock_package_graph(
        repo_root: &AbsoluteSystemPath,
        jsons: HashMap<AbsoluteSystemPathBuf, PackageJson>,
    ) -> PackageGraph {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(
            PackageGraph::builder(repo_root, PackageJson::default())
                .with_package_discovery(MockDiscovery)
                .with_lockfile(Some(Box::new(MockLockfile)))
                .with_package_jsons(Some(jsons))
                .build(),
        )
        .unwrap()
    }

    fn turbo_json(value: serde_json::Value) -> TurboJson {
        let is_package = value.as_object().unwrap().contains_key("extends");
        let json_text = serde_json::to_string(&value).unwrap();
        let raw: RawTurboJson = if is_package {
            RawPackageTurboJson::parse(&json_text, "").unwrap().into()
        } else {
            RawRootTurboJson::parse(&json_text, "").unwrap().into()
        };
        TurboJson::try_from(raw).unwrap()
    }

    #[test_case(PackageName::Root, "build", "//#build", true ; "root task")]
    #[test_case(PackageName::from("a"), "build", "a#build", true ; "workspace task in root")]
    #[test_case(PackageName::from("b"), "build", "b#build", true ; "workspace task in workspace")]
    #[test_case(PackageName::from("b"), "test", "b#test", true ; "task missing from workspace")]
    #[test_case(PackageName::from("c"), "missing", "c#missing", false ; "task missing")]
    #[test_case(PackageName::from("c"), "c#curse", "c#curse", true ; "root defined task")]
    #[test_case(PackageName::from("b"), "c#curse", "c#curse", true ; "non-workspace root defined task")]
    #[test_case(PackageName::from("b"), "b#special", "b#special", true ; "workspace defined task")]
    #[test_case(PackageName::from("c"), "b#special", "b#special", false ; "non-workspace defined task")]
    fn test_task_definition(
        workspace: PackageName,
        task_name: &'static str,
        task_id: &'static str,
        expected: bool,
    ) {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "test": { "inputs": ["testing"] },
                        "build": { "inputs": ["primary"] },
                        "a#build": { "inputs": ["special"] },
                        "c#curse": {},
                    }
                })),
            ),
            (
                PackageName::from("b"),
                turbo_json(json!({
                    "tasks": {
                        "build": { "inputs": ["outer"]},
                        "special": {},
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let task_name = TaskName::from(task_name);
        let task_id = TaskId::try_from(task_id).unwrap();

        let has_def =
            EngineBuilder::has_task_definition_in_run(&loader, &workspace, &task_name, &task_id)
                .unwrap();
        assert_eq!(has_def, expected);
    }

    macro_rules! deps {
        {} => {
            HashMap::new()
        };
        {$($key:expr => $value:expr),*} => {
            {
                let mut _map = HashMap::new();
                $(
                let key = TaskId::try_from($key).unwrap();
                let value = $value.iter().copied().map(|x| {
                    if x == "___ROOT___" {
                        TaskNode::Root
                    } else {
                        TaskNode::Task(TaskId::try_from(x).unwrap())
                    }
                }).collect::<HashSet<_>>();
                _map.insert(key, value);
                )*
                _map
            }
        };
    }

    fn all_dependencies(engine: &Engine) -> HashMap<TaskId<'static>, HashSet<TaskNode>> {
        engine
            .task_lookup()
            .keys()
            .filter_map(|task_id| {
                let deps = engine.dependencies(task_id)?;
                Some((task_id.clone(), deps.into_iter().cloned().collect()))
            })
            .collect()
    }

    #[test]
    fn test_default_engine() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => [],
                "c" => ["a", "b"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "test": { "dependsOn": ["^build", "prepare"] },
                    "build": { "dependsOn": ["^build", "prepare"] },
                    "prepare": {},
                    "side-quest": { "dependsOn": ["prepare"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![
                PackageName::from("a"),
                PackageName::from("b"),
                PackageName::from("c"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "a#test" => ["a#prepare"],
            "a#build" => ["a#prepare"],
            "a#prepare" => ["___ROOT___"],
            "b#test" => ["b#prepare"],
            "b#build" => ["b#prepare"],
            "b#prepare" => ["___ROOT___"],
            "c#prepare" => ["___ROOT___"],
            "c#test" => ["a#build", "b#build", "c#prepare"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_dependencies_on_unspecified_packages() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        // app1 -> libA
        //              \
        //                > libB -> libD
        //              /
        //       app2 <
        //              \ libC
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "app2" => ["libB", "libC"],
                "libA" => ["libB"],
                "libB" => ["libD"],
                "libC" => [],
                "libD" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "test": { "dependsOn": ["^build"] },
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![PackageName::from("app2")])
            .build()
            .unwrap();

        let expected = deps! {
            "app2#test" => ["libB#build", "libC#build"],
            "libB#build" => ["libD#build"],
            "libC#build" => ["___ROOT___"],
            "libD#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_run_package_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "app1#special": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("special"))))
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#special" => ["libA#build"],
            "libA#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_include_root_tasks() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "test": { "dependsOn": ["^build"] },
                    "//#test": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![
                Spanned::new(TaskName::from("build")),
                Spanned::new(TaskName::from("test")),
            ])
            .with_workspaces(vec![
                PackageName::Root,
                PackageName::from("app1"),
                PackageName::from("libA"),
            ])
            .with_root_tasks(vec![
                TaskName::from("//#test"),
                TaskName::from("build"),
                TaskName::from("test"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "//#test" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "app1#test" => ["libA#build"],
            "libA#build" => ["___ROOT___"],
            "libA#test" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depend_on_root_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "libA#build": { "dependsOn": ["//#root-task"] },
                    "//#root-task": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("//#root-task"),
                TaskName::from("libA#build"),
                TaskName::from("build"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "//#root-task" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "libA#build" => ["//#root-task"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depend_on_missing_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "libA#build": { "dependsOn": ["//#root-task"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![TaskName::from("libA#build"), TaskName::from("build")])
            .build();

        assert_matches!(engine, Err(Error::MissingRootTaskInTurboJson(_)));
    }

    #[test]
    fn test_depend_on_multiple_package_tasks() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "libA#build": { "dependsOn": ["app1#compile", "app1#test"] },
                    "build": { "dependsOn": ["^build"] },
                    "compile": {},
                    "test": {}
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("libA#build"),
                TaskName::from("build"),
                TaskName::from("compile"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#compile" => ["___ROOT___"],
            "app1#test" => ["___ROOT___"],
            "app1#build" => ["libA#build"],
            "libA#build" => ["app1#compile", "app1#test"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_depends_on_disabled_root_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                    "foo": {},
                    "libA#build": { "dependsOn": ["//#foo"] }
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .with_root_tasks(vec![
                TaskName::from("libA#build"),
                TaskName::from("build"),
                TaskName::from("foo"),
            ])
            .build();

        assert_matches!(engine, Err(Error::MissingRootTaskInTurboJson(_)));
    }

    #[test]
    fn test_engine_tasks_only() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => [],
                "c" => ["a", "b"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build", "prepare"] },
                    "test": { "dependsOn": ["^build", "prepare"] },
                    "prepare": {},
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("test"))))
            .with_workspaces(vec![
                PackageName::from("a"),
                PackageName::from("b"),
                PackageName::from("c"),
            ])
            .with_root_tasks(vec![
                TaskName::from("build"),
                TaskName::from("test"),
                TaskName::from("prepare"),
            ])
            .build()
            .unwrap();

        let expected = deps! {
            "a#test" => ["___ROOT___"],
            "b#test" => ["___ROOT___"],
            "c#test" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_engine_tasks_only_package_deps() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => ["a"]
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("b")])
            .with_root_tasks(vec![TaskName::from("build")])
            .build()
            .unwrap();

        // With task only we shouldn't do package tasks dependencies either
        let expected = deps! {
            "b#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_engine_tasks_only_task_dep() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "a" => [],
                "b" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "a#build": { },
                    "b#build": { "dependsOn": ["a#build"] }
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks_only(true)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("b")])
            .with_root_tasks(vec![TaskName::from("build")])
            .build()
            .unwrap();

        // With task only we shouldn't do package tasks dependencies either
        let expected = deps! {
            "b#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[allow(clippy::duplicated_attributes)]
    #[test_case("build", None)]
    #[test_case("build:prod", None)]
    #[test_case("build$colon$prod", Some("task contains invalid string '$colon$'"))]
    fn test_validate_task_name(task_name: &str, reason: Option<&str>) {
        let result = validate_task_name(Spanned::new(task_name))
            .map_err(|e| {
                if let Error::InvalidTaskName(box InvalidTaskNameError { reason, .. }) = e {
                    reason
                } else {
                    panic!("invalid error encountered {e:?}")
                }
            })
            .err();
        assert_eq!(result.as_deref(), reason);
    }

    #[test]
    fn test_run_package_task_exact() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "app2" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                        "special": { "dependsOn": ["^build"] },
                    }
                })),
            ),
            (
                PackageName::from("app2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "another": { "dependsOn": ["^build"] },
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![
                Spanned::new(TaskName::from("app1#special")),
                Spanned::new(TaskName::from("app2#another")),
            ])
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("app2")])
            .build()
            .unwrap();

        let expected = deps! {
            "app1#special" => ["libA#build"],
            "app2#another" => ["libA#build"],
            "libA#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_with_task() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "web" => [],
                "api" => []
            },
        );
        let turbo_jsons = vec![(PackageName::Root, {
            turbo_json(json!({
                "tasks": {
                    "web#dev": { "persistent": true, "with": ["api#serve"] },
                    "api#serve": { "persistent": true }
                }
            }))
        })]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("dev"))))
            .with_workspaces(vec![PackageName::from("web")])
            .build()
            .unwrap();

        let expected = deps! {
            "web#dev" => ["___ROOT___"],
            "api#serve" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    #[test]
    fn test_run_package_task_exact_error() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "another": { "dependsOn": ["^build"] },
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![Spanned::new(TaskName::from("app1#special"))])
            .with_workspaces(vec![PackageName::from("app1")])
            .build();
        assert!(engine.is_err());
        let report = miette::Report::new(engine.unwrap_err());
        let mut msg = String::new();
        miette::JSONReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_json_snapshot!(msg);

        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![Spanned::new(TaskName::from("app1#another"))])
            .with_workspaces(vec![PackageName::from("libA")])
            .build()
            .unwrap();
        assert_eq!(engine.tasks().collect::<Vec<_>>(), &[&TaskNode::Root]);
    }

    #[test]
    fn test_run_package_task_invalid_package() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![Spanned::new(TaskName::from("app2#bad-task"))])
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("libA")])
            .build();
        assert!(engine.is_err());
        let report = miette::Report::new(engine.unwrap_err());
        let mut msg = String::new();
        miette::NarratableReportHandler::new()
            .render_report(&mut msg, report.as_ref())
            .unwrap();
        assert_snapshot!(msg);
    }

    #[test]
    fn test_filter_removes_task_def() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "tasks": {
                        "app1-only": {},
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(vec![Spanned::new(TaskName::from("app1-only"))])
            .with_workspaces(vec![PackageName::from("libA")])
            .build()
            .unwrap();
        assert_eq!(
            engine.tasks().collect::<Vec<_>>(),
            &[&TaskNode::Root],
            "only the root task node should be present"
        );
    }

    #[test]
    fn test_path_to_root() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => ["libA"],
                "libA" => []
            },
        );
        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "build": { "dependsOn": ["^build"] },
                }
            })),
        )]
        .into_iter()
        .collect();
        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false);
        assert_eq!(
            engine
                .path_to_root(&TaskId::new("//", "build"))
                .unwrap()
                .as_str(),
            "."
        );
        // libA is located at packages/libA
        assert_eq!(
            engine
                .path_to_root(&TaskId::new("libA", "build"))
                .unwrap()
                .as_str(),
            "../.."
        );
    }

    #[test]
    fn test_cyclic_extends() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => [],
                "app2" => []
            },
        );

        // Create a self-referencing cycle: Root extends itself
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "extends": ["//"],  // Root extending itself creates a cycle
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {}
                })),
            ),
            (
                PackageName::from("app2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine_result = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1")])
            .build();

        assert!(engine_result.is_err());
        if let Err(Error::CyclicExtends(box CyclicExtends { cycle, .. })) = engine_result {
            // The cycle should contain root (//) since it's a self-reference
            assert!(cycle.contains(&"//".to_string()));
            // Should have at least 2 entries to show the cycle (// -> //)
            assert!(cycle.len() >= 2);
        } else {
            panic!("Expected CyclicExtends error, got {:?}", engine_result);
        }
    }

    // Test that tasks are inherited from non-root extends even when child has no
    // tasks key
    #[test]
    fn test_extends_inherits_tasks_from_non_root_package() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "shared-config" => [],
                "app" => []
            },
        );

        // Setup:
        // - shared-config defines a "build" task
        // - app extends from root and shared-config but has NO tasks key
        // - app should still be able to run the "build" task inherited from
        //   shared-config
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "inputs": ["src/**"] }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                // app extends from root and shared-config but has NO tasks defined
                turbo_json(json!({
                    "extends": ["//", "shared-config"]
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Verify that "app" can find the "build" task inherited from "shared-config"
        let task_name = TaskName::from("build");
        let task_id = TaskId::try_from("app#build").unwrap();
        let has_def = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("app"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(
            has_def,
            "app should inherit 'build' task from shared-config via extends"
        );

        // Also verify the engine can be built with this task
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app")])
            .build()
            .unwrap();

        // The engine should contain the app#build task
        let expected = deps! {
            "app#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test that tasks are discovered from non-root extends when using add_all_tasks
    #[test]
    fn test_add_all_tasks_discovers_extended_tasks() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "shared-config" => [],
                "app" => []
            },
        );

        // Setup:
        // - root has "test" task
        // - shared-config has "build" task
        // - app extends from shared-config but has no tasks
        // - When using add_all_tasks, "build" should be discovered for app
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "shared-config"]
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Test collect_tasks_from_extends_chain
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // app should have discovered "build" from shared-config and "test" from root
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should discover 'build' task from shared-config"
        );
        assert!(
            tasks.contains(&TaskName::from("test")),
            "Should discover 'test' task from root"
        );
    }

    // Test A→B→A cycle handling (gracefully handled via visited set)
    #[test]
    fn test_cyclic_extends_between_packages_graceful() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => []
            },
        );

        // Create a cycle: pkg-a extends pkg-b, pkg-b extends pkg-a
        // Note: Both extend root first to satisfy validation
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "root-task": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        "task-a": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-a"],
                    "tasks": {
                        "task-b": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // The cycle is handled gracefully via the visited set - it doesn't error,
        // it just stops recursion when it encounters a visited package.
        // This test verifies that the cycle doesn't cause infinite recursion
        // and that we still collect all reachable tasks.
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have collected tasks from pkg-a, pkg-b, and root (despite the cycle)
        assert!(
            tasks.contains(&TaskName::from("task-a")),
            "Should have task-a"
        );
        assert!(
            tasks.contains(&TaskName::from("task-b")),
            "Should have task-b"
        );
        assert!(
            tasks.contains(&TaskName::from("root-task")),
            "Should have root-task"
        );

        // Also verify has_task_definition_in_run handles the cycle gracefully
        let task_name = TaskName::from("task-b");
        let task_id = TaskId::try_from("pkg-a#task-b").unwrap();
        let has_def = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("pkg-a"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(has_def, "Should find task-b via extends chain");
    }

    // Test deep extends chain: A extends B extends C extends D extends root
    #[test]
    fn test_deep_extends_chain() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => [],
                "pkg-c" => [],
                "pkg-d" => []
            },
        );

        // Create a deep chain: pkg-a -> pkg-b -> pkg-c -> pkg-d -> root
        // Each level adds a unique task
        // Note: Each package must extend root first to satisfy validation
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "root-task": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-d"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "task-d": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-c"),
                turbo_json(json!({
                    "extends": ["//", "pkg-d"],
                    "tasks": {
                        "task-c": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-c"],
                    "tasks": {
                        "task-b": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        "task-a": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Test that pkg-a can discover all tasks from the entire chain
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // pkg-a should have all tasks from the chain
        assert!(
            tasks.contains(&TaskName::from("task-a")),
            "Should have task-a"
        );
        assert!(
            tasks.contains(&TaskName::from("task-b")),
            "Should have task-b from pkg-b"
        );
        assert!(
            tasks.contains(&TaskName::from("task-c")),
            "Should have task-c from pkg-c"
        );
        assert!(
            tasks.contains(&TaskName::from("task-d")),
            "Should have task-d from pkg-d"
        );
        assert!(
            tasks.contains(&TaskName::from("root-task")),
            "Should have root-task from root"
        );

        // Also verify has_task_definition_in_run works for deep chain
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(Some(Spanned::new(TaskName::from("task-d"))))
            .with_workspaces(vec![PackageName::from("pkg-a")])
            .build()
            .unwrap();

        let expected = deps! {
            "pkg-a#task-d" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test diamond inheritance: app extends [base1, base2], both base1 and base2
    // extend root
    #[test]
    fn test_diamond_inheritance_deduplication() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "base1" => [],
                "base2" => [],
                "app" => []
            },
        );

        // Diamond pattern:
        //        app
        //       /   \
        //    base1  base2
        //       \   /
        //        root
        // Both base1 and base2 define "build" task, app should only get it once
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "root-task": {},
                        "build": {}  // Also defined in root
                    }
                })),
            ),
            (
                PackageName::from("base1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {},  // Same task name as base2
                        "base1-only": {}
                    }
                })),
            ),
            (
                PackageName::from("base2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {},  // Same task name as base1
                        "base2-only": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "base1", "base2"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Test that tasks are deduplicated
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have all unique tasks
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build"
        );
        assert!(
            tasks.contains(&TaskName::from("root-task")),
            "Should have root-task"
        );
        assert!(
            tasks.contains(&TaskName::from("base1-only")),
            "Should have base1-only"
        );
        assert!(
            tasks.contains(&TaskName::from("base2-only")),
            "Should have base2-only"
        );

        // Verify count - build should only appear once due to HashSet deduplication
        assert_eq!(tasks.len(), 4, "Should have exactly 4 unique tasks");

        // Also verify the engine builds successfully
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app")])
            .build()
            .unwrap();

        let expected = deps! {
            "app#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test that workspace without turbo.json falls back to root
    #[test]
    fn test_missing_workspace_turbo_json_fallback() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app-with-config" => [],
                "app-without-config" => []
            },
        );

        // Only root and app-with-config have turbo.json
        // app-without-config has no turbo.json and should fall back to root
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("app-with-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "custom": {}
                    }
                })),
            ),
            // Note: app-without-config has NO turbo.json entry
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Test collect_tasks_from_extends_chain for workspace without turbo.json
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app-without-config"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should fall back to root and get root's tasks
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build from root fallback"
        );
        assert!(
            tasks.contains(&TaskName::from("test")),
            "Should have test from root fallback"
        );

        // Test has_task_definition_in_run for workspace without turbo.json
        let task_name = TaskName::from("build");
        let task_id = TaskId::try_from("app-without-config#build").unwrap();
        let has_def = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("app-without-config"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(
            has_def,
            "app-without-config should find 'build' task via root fallback"
        );

        // Verify engine builds correctly for workspace without turbo.json
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app-without-config")])
            .build()
            .unwrap();

        let expected = deps! {
            "app-without-config#build" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test task-level extends: false to opt out of inherited tasks
    #[test]
    fn test_task_extends_false_excludes_task() {
        // shared-config defines build and lint tasks
        // app extends shared-config but opts out of lint with extends: false
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "outputs": ["dist/**"] },
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "shared-config"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Collect tasks for app
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // app should have build (inherited) and test (from root) but NOT lint
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build from shared-config"
        );
        assert!(
            tasks.contains(&TaskName::from("test")),
            "Should have test from root"
        );
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint - excluded with extends: false"
        );
    }

    // Test task-level extends: false with local config creates fresh definition
    #[test]
    fn test_task_extends_false_with_config_creates_fresh_task() {
        // app has extends: false on build but provides its own config
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {}
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "outputs": ["dist/**"], "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "shared-config"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "outputs": ["custom-dist/**"],
                            "cache": false
                        }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Collect tasks for app - should still have build (as fresh definition)
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build as a fresh definition"
        );
    }

    // Test error when extends: false is used on a task not in the extends chain
    #[test]
    fn test_task_extends_false_on_nonexistent_task_errors() {
        // app tries to opt out of "nonexistent" task that doesn't exist in chain
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "nonexistent": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Should error because "nonexistent" is not in the extends chain
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        let result = EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        );

        assert!(
            result.is_err(),
            "Should error when extends: false is used on non-inherited task"
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("nonexistent"),
            "Error should mention the task name"
        );
    }

    // Test that extends: true is a no-op (same as omitting the field)
    #[test]
    fn test_task_extends_true_is_noop() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "extends": true }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Should have build task (inherited normally)
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build task with extends: true"
        );
    }

    // Test that has_task_definition_in_run returns false for tasks excluded via
    // extends: false
    #[test]
    fn test_has_task_definition_returns_false_for_excluded_tasks() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // build should still be found (inherited from root)
        let task_name = TaskName::from("build");
        let task_id = TaskId::try_from("app#build").unwrap();
        let has_build = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("app"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(has_build, "build should be found via extends chain");

        // lint should NOT be found (excluded via extends: false)
        let task_name = TaskName::from("lint");
        let task_id = TaskId::try_from("app#lint").unwrap();
        let has_lint = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("app"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(
            !has_lint,
            "lint should NOT be found - excluded with extends: false"
        );
    }

    // Test that has_task_definition_in_run returns true for extends: false WITH
    // config
    #[test]
    fn test_has_task_definition_returns_true_for_excluded_tasks_with_config() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "cache": false
                        }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // build should be found (has extends: false but also has config)
        let task_name = TaskName::from("build");
        let task_id = TaskId::try_from("app#build").unwrap();
        let has_build = EngineBuilder::has_task_definition_in_run(
            &loader,
            &PackageName::from("app"),
            &task_name,
            &task_id,
        )
        .unwrap();
        assert!(
            has_build,
            "build should be found - extends: false with config creates fresh definition"
        );
    }

    // ==================== Additional Test Coverage ====================
    // The following tests cover gaps identified in the test coverage review

    // Test multi-level task-level extends: A extends B, B has extends: false on
    // task from C NOTE: The current implementation behavior is that `extends:
    // false` only applies to the package where it's defined. If pkg-a extends
    // pkg-b, and pkg-b excludes a task from pkg-c, pkg-a will still see the
    // task because it collects from the full chain. This is intentional:
    // exclusions are package-local, not propagated through the chain.
    #[test]
    fn test_multi_level_task_extends_false() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => [],
                "pkg-c" => []
            },
        );

        // Chain: pkg-a extends pkg-b extends pkg-c extends root
        // pkg-c defines "lint" task
        // pkg-b excludes "lint" task via extends: false
        // pkg-a extends pkg-b
        //
        // Correct behavior: pkg-a should NOT see "lint" because exclusions propagate
        // through the extends chain. When pkg-b excludes "lint", all packages that
        // extend pkg-b (like pkg-a) will also not see "lint".
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-c"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-c"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // pkg-a should NOT see lint because exclusions propagate through extends chain
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build from root"
        );
        // lint is NOT visible to pkg-a because pkg-b's exclusion propagates
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint - exclusions propagate through extends chain"
        );

        // pkg-b itself should also NOT see lint
        let mut tasks_b = HashSet::new();
        let mut visited_b = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-b"),
            &mut tasks_b,
            &mut visited_b,
        )
        .unwrap();
        assert!(
            !tasks_b.contains(&TaskName::from("lint")),
            "pkg-b should NOT have lint - excluded locally"
        );
    }

    // Test that pkg-a can re-add an excluded task by defining it explicitly
    #[test]
    fn test_multi_level_task_extends_false_re_add() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => [],
                "pkg-c" => []
            },
        );

        // Even though pkg-b excludes lint, pkg-a can re-add it by defining it
        // explicitly
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-c"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-c"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        // Explicitly define lint to re-add it (overrides pkg-b's exclusion)
                        "lint": { "cache": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // pkg-a should see lint because it explicitly defines it
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build from root"
        );
        assert!(
            tasks.contains(&TaskName::from("lint")),
            "Should have lint - pkg-a re-added it explicitly"
        );
    }

    // Test multiple tasks excluded with extends: false in the same package
    #[test]
    fn test_multiple_tasks_excluded_with_extends_false() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {},
                        "test": {},
                        "deploy": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false },
                        "test": { "extends": false },
                        "custom": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have build and deploy from root, custom from app
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build"
        );
        assert!(
            tasks.contains(&TaskName::from("deploy")),
            "Should have deploy"
        );
        assert!(
            tasks.contains(&TaskName::from("custom")),
            "Should have custom"
        );
        // lint and test should be excluded
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint"
        );
        assert!(
            !tasks.contains(&TaskName::from("test")),
            "Should NOT have test"
        );
    }

    // Test extends: false on the same task in multiple packages in the chain
    #[test]
    fn test_extends_false_same_task_multiple_packages() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => []
            },
        );

        // Both pkg-a and pkg-b exclude "lint" task
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build"
        );
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint - excluded in both packages"
        );
    }

    // Test empty tasks objects in intermediate packages
    #[test]
    fn test_empty_tasks_in_intermediate_packages() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => [],
                "pkg-c" => []
            },
        );

        // pkg-a extends pkg-b extends pkg-c extends root
        // pkg-b has empty tasks object
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "root-task": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-c"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "c-task": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-c"],
                    "tasks": {}
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        "a-task": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have all tasks despite empty tasks in pkg-b
        assert!(
            tasks.contains(&TaskName::from("root-task")),
            "Should have root-task from root"
        );
        assert!(
            tasks.contains(&TaskName::from("c-task")),
            "Should have c-task from pkg-c"
        );
        assert!(
            tasks.contains(&TaskName::from("a-task")),
            "Should have a-task from pkg-a"
        );
    }

    // Test extends: false with different config types (inputs, outputs, env)
    #[test]
    fn test_extends_false_with_various_configs() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app" => []
            },
        );

        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "outputs": ["dist/**"], "cache": true },
                        "lint": {},
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "outputs": ["custom-dist/**"],
                            "inputs": ["src/**"],
                            "env": ["NODE_ENV"]
                        },
                        "lint": {
                            "extends": false,
                            "persistent": true
                        },
                        "test": {
                            "extends": false,
                            "dependsOn": ["build"]
                        }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // All tasks should be found since they have config beyond extends
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(vec![
                Spanned::new(TaskName::from("build")),
                Spanned::new(TaskName::from("lint")),
                Spanned::new(TaskName::from("test")),
            ])
            .with_workspaces(vec![PackageName::from("app")])
            .build()
            .unwrap();

        // All tasks should be in the engine
        let expected = deps! {
            "app#build" => ["___ROOT___"],
            "app#lint" => ["___ROOT___"],
            "app#test" => ["app#build"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test add_all_tasks with excluded tasks - full engine build
    #[test]
    fn test_add_all_tasks_with_excluded_tasks_full_build() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app" => []
            },
        );

        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {},
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Use add_all_tasks mode
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .add_all_tasks()
            .with_workspaces(vec![PackageName::from("app")])
            .build()
            .unwrap();

        // Should have build and test, but NOT lint
        let task_ids: HashSet<_> = engine
            .task_lookup()
            .keys()
            .map(|id| id.to_string())
            .collect();

        assert!(task_ids.contains("app#build"), "Should have app#build");
        assert!(task_ids.contains("app#test"), "Should have app#test");
        assert!(
            !task_ids.contains("app#lint"),
            "Should NOT have app#lint - excluded"
        );
    }

    // Test that transit node pattern works with add_all_tasks (GitHub issue #11301)
    // This tests the case where a root task like "transit" is defined without the
    // //# prefix in turbo.json, but is used as a dependency from other tasks.
    //
    // The scenario: User has a turbo.json with:
    //   "type-check": { "dependsOn": ["transit"] }
    //   "transit": { "dependsOn": ["^transit"] }
    //
    // And a root package.json with a "type-check" script (but NOT a "transit"
    // script). When devtools runs:
    // 1. //#type-check is enabled as a root task (from package.json script)
    // 2. Processing //#type-check adds //#transit as a dependency
    // 3. //#transit should be allowed because it has a definition in turbo.json
    #[test]
    fn test_add_all_tasks_with_transit_node() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app" => ["lib"],
                "lib" => []
            },
        );

        let turbo_jsons = vec![(
            PackageName::Root,
            turbo_json(json!({
                "tasks": {
                    "type-check": { "dependsOn": ["transit"] },
                    "transit": { "dependsOn": ["^transit"] }
                }
            })),
        )]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Simulate what devtools does:
        // - Collect task keys from turbo.json (type-check, transit - both without //#)
        // - Also add //#type-check because it's in root package.json scripts
        // The key is that //#type-check is enabled but //#transit is NOT explicitly
        // enabled - it only has a definition in turbo.json.
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_root_tasks(vec![
                // This simulates having a "type-check" script in root package.json
                // Devtools adds it with //#  prefix
                TaskName::from("//#type-check"),
            ])
            .add_all_tasks()
            .with_workspaces(vec![
                PackageName::Root,
                PackageName::from("app"),
                PackageName::from("lib"),
            ])
            .build()
            .expect("Engine build should succeed with transit node pattern");

        let task_ids: HashSet<_> = engine
            .task_lookup()
            .keys()
            .map(|id| id.to_string())
            .collect();

        // Should have root tasks
        assert!(
            task_ids.contains("//#type-check"),
            "Should have //#type-check"
        );
        assert!(task_ids.contains("//#transit"), "Should have //#transit");

        // Should have workspace transit tasks from ^transit dependency
        assert!(task_ids.contains("app#transit"), "Should have app#transit");
        assert!(task_ids.contains("lib#transit"), "Should have lib#transit");

        // Verify the dependency graph structure
        let deps = all_dependencies(&engine);

        // //#type-check should depend on //#transit
        let type_check_deps = deps.get(&TaskId::try_from("//#type-check").unwrap());
        assert!(
            type_check_deps.is_some(),
            "//#type-check should have dependencies"
        );
        assert!(
            type_check_deps
                .unwrap()
                .contains(&TaskNode::Task(TaskId::try_from("//#transit").unwrap())),
            "//#type-check should depend on //#transit"
        );
    }

    // Test interaction with dependsOn and topological dependencies
    #[test]
    fn test_extends_false_with_dependson_topo() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app" => ["lib"],
                "lib" => []
            },
        );

        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "dependsOn": ["^build"] },
                        "lint": { "dependsOn": ["build"] }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "dependsOn": ["^build", "prepare"]
                        },
                        "prepare": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app")])
            .build()
            .unwrap();

        // app#build should depend on lib#build (^build) and app#prepare (fresh
        // definition)
        let expected = deps! {
            "app#build" => ["lib#build", "app#prepare"],
            "lib#build" => ["___ROOT___"],
            "app#prepare" => ["___ROOT___"]
        };
        assert_eq!(all_dependencies(&engine), expected);
    }

    // Test order of extends array affecting task resolution
    #[test]
    fn test_extends_order_affects_resolution() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app" => [],
                "config-a" => [],
                "config-b" => []
            },
        );

        // config-a and config-b both define same task with different configs
        // Order in extends should determine which is used
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {}
                })),
            ),
            (
                PackageName::from("config-a"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "outputs": ["dist-a/**"] }
                    }
                })),
            ),
            (
                PackageName::from("config-b"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "outputs": ["dist-b/**"] }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "config-a", "config-b"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Tasks should be discovered from both - deduplication happens by task name
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have build task (deduplicated)
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build task"
        );
        assert_eq!(tasks.len(), 1, "Should only have one unique task");
    }

    // Test that extends: false requires the task to exist in the chain (error case
    // verification)
    #[test]
    fn test_extends_false_error_message_quality() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "test": {}
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "nonexistent-task": { "extends": false }
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        let result = EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = err.to_string();

        // Error should mention the task name
        assert!(
            err_string.contains("nonexistent-task"),
            "Error should mention the task name: {}",
            err_string
        );
    }

    // Test extends: true mixed with extends: false in same package
    #[test]
    fn test_extends_true_and_false_mixed() {
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "cache": true },
                        "lint": { "cache": true },
                        "test": { "cache": true }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "extends": true },
                        "lint": { "extends": false },
                        "test": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // build should be inherited (extends: true)
        // lint should be excluded (extends: false)
        // test should be inherited (no extends field = normal inheritance)
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build"
        );
        assert!(tasks.contains(&TaskName::from("test")), "Should have test");
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint"
        );
    }

    // Test task discovery when workspace has no turbo.json but extends from a
    // package that DOES have one
    #[test]
    fn test_workspace_without_turbo_json_with_extends_in_root() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "shared-config" => [],
                "app-with-config" => [],
                "app-without-config" => []
            },
        );

        // shared-config defines tasks
        // app-with-config extends shared-config
        // app-without-config has NO turbo.json (should fallback to root)
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "root-build": {}
                    }
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "shared-build": {}
                    }
                })),
            ),
            (
                PackageName::from("app-with-config"),
                turbo_json(json!({
                    "extends": ["//", "shared-config"],
                    "tasks": {}
                })),
            ),
            // Note: app-without-config has NO turbo.json entry
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // app-with-config should have both root-build and shared-build
        let mut tasks1 = HashSet::new();
        let mut visited1 = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app-with-config"),
            &mut tasks1,
            &mut visited1,
        )
        .unwrap();

        assert!(tasks1.contains(&TaskName::from("root-build")));
        assert!(tasks1.contains(&TaskName::from("shared-build")));

        // app-without-config should fallback to root (only root-build)
        let mut tasks2 = HashSet::new();
        let mut visited2 = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app-without-config"),
            &mut tasks2,
            &mut visited2,
        )
        .unwrap();

        assert!(tasks2.contains(&TaskName::from("root-build")));
        // Should NOT have shared-build since app-without-config doesn't extend
        // shared-config
        assert!(!tasks2.contains(&TaskName::from("shared-build")));
    }

    // Test partial exclusion - only exclude task for specific package via extends:
    // false
    #[test]
    fn test_partial_exclusion_specific_package() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => [],
                "app2" => []
            },
        );

        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("app2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // app1 should NOT have lint
        let mut tasks1 = HashSet::new();
        let mut visited1 = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app1"),
            &mut tasks1,
            &mut visited1,
        )
        .unwrap();
        assert!(tasks1.contains(&TaskName::from("build")));
        assert!(!tasks1.contains(&TaskName::from("lint")));

        // app2 SHOULD have lint (exclusion is package-specific)
        let mut tasks2 = HashSet::new();
        let mut visited2 = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("app2"),
            &mut tasks2,
            &mut visited2,
        )
        .unwrap();
        assert!(tasks2.contains(&TaskName::from("build")));
        assert!(tasks2.contains(&TaskName::from("lint")));
    }

    // Test that engine building with multiple workspaces handles exclusions
    // correctly
    #[test]
    fn test_engine_multiple_workspaces_with_exclusions() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "app1" => [],
                "app2" => []
            },
        );

        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {}
                    }
                })),
            ),
            (
                PackageName::from("app1"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("app2"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Build with both workspaces
        let engine = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default())
            .with_tasks(Some(Spanned::new(TaskName::from("build"))))
            .with_workspaces(vec![PackageName::from("app1"), PackageName::from("app2")])
            .build()
            .unwrap();

        // Only app2#build should be in the engine (app1 excluded it)
        let task_ids: HashSet<_> = engine
            .task_lookup()
            .keys()
            .map(|id| id.to_string())
            .collect();

        assert!(
            !task_ids.contains("app1#build"),
            "Should NOT have app1#build - excluded"
        );
        assert!(task_ids.contains("app2#build"), "Should have app2#build");
    }

    // Test cyclic extends with task exclusion doesn't cause issues
    #[test]
    fn test_cyclic_extends_with_task_exclusion() {
        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let _package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "pkg-a" => [],
                "pkg-b" => []
            },
        );

        // Create a cycle with task exclusions
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": {},
                        "lint": {}
                    }
                })),
            ),
            (
                PackageName::from("pkg-a"),
                turbo_json(json!({
                    "extends": ["//", "pkg-b"],
                    "tasks": {
                        "lint": { "extends": false }
                    }
                })),
            ),
            (
                PackageName::from("pkg-b"),
                turbo_json(json!({
                    "extends": ["//", "pkg-a"],
                    "tasks": {
                        "custom-b": {}
                    }
                })),
            ),
        ]
        .into_iter()
        .collect();

        let loader = TurboJsonLoader::noop(turbo_jsons);

        // Should handle cycle gracefully even with task exclusion
        let mut tasks = HashSet::new();
        let mut visited = HashSet::new();
        EngineBuilder::collect_tasks_from_extends_chain(
            &loader,
            &PackageName::from("pkg-a"),
            &mut tasks,
            &mut visited,
        )
        .unwrap();

        // Should have build and custom-b, but NOT lint
        assert!(
            tasks.contains(&TaskName::from("build")),
            "Should have build"
        );
        assert!(
            tasks.contains(&TaskName::from("custom-b")),
            "Should have custom-b from pkg-b"
        );
        assert!(
            !tasks.contains(&TaskName::from("lint")),
            "Should NOT have lint - excluded"
        );
    }

    // Test that task_definition_chain correctly handles extends: false in
    // intermediate packages. This ensures that when a shared-config package
    // uses `extends: false` for a task, packages extending from it will
    // use the shared-config's definition, not the root's.
    #[test]
    fn test_task_definition_chain_with_extends_false_in_intermediate() {
        // Scenario:
        // - Root turbo.json: defines build: { cache: true, outputs: ["dist/**"] }
        // - shared-config/turbo.json: extends root, defines build: { extends: false,
        //   cache: false }
        // - app/turbo.json: extends shared-config, empty tasks
        //
        // Expected: app#build should use shared-config's cache: false, NOT root's
        // cache: true
        let turbo_jsons = vec![
            (
                PackageName::Root,
                turbo_json(json!({
                    "tasks": {
                        "build": { "cache": true, "outputs": ["dist/**"] }
                    }
                })),
            ),
            (
                PackageName::from("shared-config"),
                turbo_json(json!({
                    "extends": ["//"],
                    "tasks": {
                        "build": {
                            "extends": false,
                            "cache": false
                        }
                    }
                })),
            ),
            (
                PackageName::from("app"),
                turbo_json(json!({
                    "extends": ["//", "shared-config"],
                    "tasks": {}
                })),
            ),
        ]
        .into_iter()
        .collect();

        let repo_root_dir = TempDir::with_prefix("repo").unwrap();
        let repo_root = AbsoluteSystemPathBuf::new(repo_root_dir.path().to_str().unwrap()).unwrap();
        let package_graph = mock_package_graph(
            &repo_root,
            package_jsons! {
                repo_root,
                "shared-config" => [],
                "app" => []
            },
        );

        let loader = TurboJsonLoader::noop(turbo_jsons);
        let engine_builder = EngineBuilder::new(&repo_root, &package_graph, &loader, false)
            .with_future_flags(FutureFlags::default());

        // Verify task_definition_chain gets definitions from shared-config, not root
        let task_name = TaskName::from("build");
        let task_id = TaskId::try_from("app#build").unwrap();
        let task_id_spanned = Spanned::new(task_id);
        let definitions = engine_builder
            .task_definition_chain(&loader, &task_id_spanned, &task_name)
            .unwrap();

        assert!(
            !definitions.is_empty(),
            "task_definition_chain should return definitions for app#build"
        );

        // Should use shared-config's cache: false (not root's cache: true)
        // The first definition in the chain should be from shared-config
        if let Some(first_def) = definitions.first() {
            assert_eq!(
                first_def.cache.as_ref().map(|c| *c.as_inner()),
                Some(false),
                "Should use shared-config cache: false, not root cache: true"
            );
        }

        // There should only be one definition (shared-config's), not two
        assert_eq!(
            definitions.len(),
            1,
            "Should only have one definition from shared-config, not root + shared-config"
        );
    }
}
