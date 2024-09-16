mod builder;
mod execute;

mod dot;
mod mermaid;

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

pub use builder::{EngineBuilder, Error as BuilderError};
pub use execute::{ExecuteError, ExecutionOptions, Message, StopExecution};
use miette::{Diagnostic, NamedSource, SourceSpan};
use petgraph::Graph;
use thiserror::Error;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::{PackageGraph, PackageName};

use crate::{run::task_id::TaskId, task_graph::TaskDefinition, turbo_json::UIMode};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TaskNode {
    Root,
    Task(TaskId<'static>),
}

impl From<TaskId<'static>> for TaskNode {
    fn from(value: TaskId<'static>) -> Self {
        Self::Task(value)
    }
}

#[derive(Debug, Default)]
pub struct Building;
#[derive(Debug, Default)]
pub struct Built;

#[derive(Debug)]
pub struct Engine<S = Built> {
    marker: std::marker::PhantomData<S>,
    task_graph: Graph<TaskNode, ()>,
    root_index: petgraph::graph::NodeIndex,
    task_lookup: HashMap<TaskId<'static>, petgraph::graph::NodeIndex>,
    task_definitions: HashMap<TaskId<'static>, TaskDefinition>,
    task_locations: HashMap<TaskId<'static>, Spanned<()>>,
    package_tasks: HashMap<PackageName, Vec<petgraph::graph::NodeIndex>>,
    pub(crate) has_persistent_tasks: bool,
}

impl Engine<Building> {
    pub fn new() -> Self {
        let mut task_graph = Graph::default();
        let root_index = task_graph.add_node(TaskNode::Root);
        Self {
            marker: std::marker::PhantomData,
            task_graph,
            root_index,
            task_lookup: HashMap::default(),
            task_definitions: HashMap::default(),
            task_locations: HashMap::default(),
            package_tasks: HashMap::default(),
            has_persistent_tasks: false,
        }
    }

    pub fn get_index(&mut self, task_id: &TaskId<'static>) -> petgraph::graph::NodeIndex {
        self.task_lookup.get(task_id).copied().unwrap_or_else(|| {
            let index = self.task_graph.add_node(TaskNode::Task(task_id.clone()));
            self.task_lookup.insert(task_id.clone(), index);
            self.package_tasks
                .entry(PackageName::from(task_id.package()))
                .or_default()
                .push(index);

            index
        })
    }

    pub fn connect_to_root(&mut self, task_id: &TaskId<'static>) {
        let source = self.get_index(task_id);
        self.task_graph.add_edge(source, self.root_index, ());
    }

    pub fn add_definition(
        &mut self,
        task_id: TaskId<'static>,
        definition: TaskDefinition,
    ) -> Option<TaskDefinition> {
        if definition.persistent {
            self.has_persistent_tasks = true;
        }
        self.task_definitions.insert(task_id, definition)
    }

    pub fn add_task_location(&mut self, task_id: TaskId<'static>, location: Spanned<()>) {
        // If we don't have the location stored,
        // or if the location stored is empty, we add it to the map.
        let has_location = self
            .task_locations
            .get(&task_id)
            .map_or(false, |existing| existing.range.is_some());

        if !has_location {
            self.task_locations.insert(task_id, location);
        }
    }

    // Seals the task graph from being mutated
    pub fn seal(self) -> Engine<Built> {
        let Engine {
            task_graph,
            task_lookup,
            root_index,
            task_definitions,
            task_locations,
            package_tasks,
            has_persistent_tasks: has_persistent_task,
            ..
        } = self;
        Engine {
            marker: std::marker::PhantomData,
            task_graph,
            task_lookup,
            root_index,
            task_definitions,
            task_locations,
            package_tasks,
            has_persistent_tasks: has_persistent_task,
        }
    }
}

impl Default for Engine<Building> {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine<Built> {
    /// Creates an instance of `Engine` that only contains tasks that depend on
    /// tasks from a given package. This is useful for watch mode, where we
    /// need to re-run only a portion of the task graph.
    pub fn create_engine_for_subgraph(
        &self,
        changed_packages: &HashSet<PackageName>,
    ) -> Engine<Built> {
        let entrypoint_indices: Vec<_> = changed_packages
            .iter()
            .flat_map(|pkg| self.package_tasks.get(pkg))
            .flatten()
            .collect();

        // We reverse the graph because we want the *dependents* of entrypoint tasks
        let mut reversed_graph = self.task_graph.clone();
        reversed_graph.reverse();

        // This is `O(V^3)`, so in theory a bottleneck. Running dijkstra's
        // algorithm for each entrypoint task could potentially be faster.
        let node_distances = petgraph::algo::floyd_warshall::floyd_warshall(&reversed_graph, |_| 1)
            .expect("no negative cycles");

        let new_graph = self.task_graph.filter_map(
            |node_idx, node| {
                if let TaskNode::Task(task) = &self.task_graph[node_idx] {
                    // We only want to include tasks that are not persistent
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");

                    if def.persistent {
                        return None;
                    }
                }
                // If the node is reachable from any of the entrypoint tasks, we include it
                entrypoint_indices
                    .iter()
                    .any(|idx| {
                        node_distances
                            .get(&(**idx, node_idx))
                            .map_or(false, |dist| *dist != i32::MAX)
                    })
                    .then_some(node.clone())
            },
            |_, _| Some(()),
        );

        let task_lookup: HashMap<_, _> = new_graph
            .node_indices()
            .filter_map(|index| {
                let task = new_graph
                    .node_weight(index)
                    .expect("node index should be present");
                match task {
                    TaskNode::Root => None,
                    TaskNode::Task(task) => Some((task.clone(), index)),
                }
            })
            .collect();

        Engine {
            marker: std::marker::PhantomData,
            root_index: self.root_index,
            task_graph: new_graph,
            task_lookup,
            task_definitions: self.task_definitions.clone(),
            task_locations: self.task_locations.clone(),
            package_tasks: self.package_tasks.clone(),
            // We've filtered out persistent tasks
            has_persistent_tasks: false,
        }
    }

    /// Creates an `Engine` with persistent tasks filtered out. Used in watch
    /// mode to re-run the non-persistent tasks.
    pub fn create_engine_without_persistent_tasks(&self) -> Engine<Built> {
        let new_graph = self.task_graph.filter_map(
            |node_idx, node| match &self.task_graph[node_idx] {
                TaskNode::Task(task) => {
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");

                    if !def.persistent {
                        Some(node.clone())
                    } else {
                        None
                    }
                }
                TaskNode::Root => Some(node.clone()),
            },
            |_, _| Some(()),
        );

        let root_index = new_graph
            .node_indices()
            .find(|index| new_graph[*index] == TaskNode::Root)
            .expect("root node should be present");

        let task_lookup: HashMap<_, _> = new_graph
            .node_indices()
            .filter_map(|index| {
                let task = new_graph
                    .node_weight(index)
                    .expect("node index should be present");
                match task {
                    TaskNode::Root => None,
                    TaskNode::Task(task) => Some((task.clone(), index)),
                }
            })
            .collect();

        Engine {
            marker: std::marker::PhantomData,
            root_index,
            task_graph: new_graph,
            task_lookup,
            task_definitions: self.task_definitions.clone(),
            task_locations: self.task_locations.clone(),
            package_tasks: self.package_tasks.clone(),
            has_persistent_tasks: false,
        }
    }

    /// Creates an `Engine` that is only the persistent tasks.
    pub fn create_engine_for_persistent_tasks(&self) -> Engine<Built> {
        let mut new_graph = self.task_graph.filter_map(
            |node_idx, node| match &self.task_graph[node_idx] {
                TaskNode::Task(task) => {
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");

                    if def.persistent {
                        Some(node.clone())
                    } else {
                        None
                    }
                }
                TaskNode::Root => Some(node.clone()),
            },
            |_, _| Some(()),
        );

        let root_index = new_graph
            .node_indices()
            .find(|index| new_graph[*index] == TaskNode::Root)
            .expect("root node should be present");

        // Connect persistent tasks to root
        for index in new_graph.node_indices() {
            if new_graph[index] == TaskNode::Root {
                continue;
            }

            new_graph.add_edge(index, root_index, ());
        }

        let task_lookup: HashMap<_, _> = new_graph
            .node_indices()
            .filter_map(|index| {
                let task = new_graph
                    .node_weight(index)
                    .expect("node index should be present");
                match task {
                    TaskNode::Root => None,
                    TaskNode::Task(task) => Some((task.clone(), index)),
                }
            })
            .collect();

        Engine {
            marker: std::marker::PhantomData,
            root_index,
            task_graph: new_graph,
            task_lookup,
            task_definitions: self.task_definitions.clone(),
            task_locations: self.task_locations.clone(),
            package_tasks: self.package_tasks.clone(),
            has_persistent_tasks: true,
        }
    }

    pub fn dependencies(&self, task_id: &TaskId) -> Option<HashSet<&TaskNode>> {
        self.neighbors(task_id, petgraph::Direction::Outgoing)
    }

    pub fn dependents(&self, task_id: &TaskId) -> Option<HashSet<&TaskNode>> {
        self.neighbors(task_id, petgraph::Direction::Incoming)
    }

    fn neighbors(
        &self,
        task_id: &TaskId,
        direction: petgraph::Direction,
    ) -> Option<HashSet<&TaskNode>> {
        let index = self.task_lookup.get(task_id)?;
        Some(
            self.task_graph
                .neighbors_directed(*index, direction)
                .map(|index| {
                    self.task_graph
                        .node_weight(index)
                        .expect("node index should be present")
                })
                .collect(),
        )
    }

    // TODO get rid of static lifetime and figure out right way to tell compiler the
    // lifetime of the return ref
    pub fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition> {
        self.task_definitions.get(task_id)
    }

    pub fn tasks(&self) -> impl Iterator<Item = &TaskNode> {
        self.task_graph.node_weights()
    }

    /// Return all tasks that have a command to be run
    pub fn tasks_with_command(&self, pkg_graph: &PackageGraph) -> Vec<String> {
        self.tasks()
            .filter_map(|node| match node {
                TaskNode::Root => None,
                TaskNode::Task(task) => Some(task),
            })
            .filter_map(|task| {
                let pkg_name = PackageName::from(task.package());
                let json = pkg_graph.package_json(&pkg_name)?;
                json.command(task.task()).map(|_| task.to_string())
            })
            .collect()
    }

    pub fn task_definitions(&self) -> &HashMap<TaskId<'static>, TaskDefinition> {
        &self.task_definitions
    }

    pub fn validate(
        &self,
        package_graph: &PackageGraph,
        concurrency: u32,
        ui_mode: UIMode,
    ) -> Result<(), Vec<ValidateError>> {
        // TODO(olszewski) once this is hooked up to a real run, we should
        // see if using rayon to parallelize would provide a speedup
        let (persistent_count, mut validation_errors) = self
            .task_graph
            .node_indices()
            .map(|node_index| {
                let TaskNode::Task(task_id) = self
                    .task_graph
                    .node_weight(node_index)
                    .expect("graph should contain weight for node index")
                else {
                    // No need to check the root node if that's where we are.
                    return Ok(false);
                };

                for dep_index in self
                    .task_graph
                    .neighbors_directed(node_index, petgraph::Direction::Outgoing)
                {
                    let TaskNode::Task(dep_id) = self
                        .task_graph
                        .node_weight(dep_index)
                        .expect("index comes from iterating the graph and must be present")
                    else {
                        // No need to check the root node
                        continue;
                    };

                    let task_definition = self.task_definitions.get(dep_id).ok_or_else(|| {
                        ValidateError::MissingTask {
                            task_id: dep_id.to_string(),
                            package_name: dep_id.package().to_string(),
                        }
                    })?;

                    let package_json = package_graph
                        .package_json(&PackageName::from(dep_id.package()))
                        .ok_or_else(|| ValidateError::MissingPackageJson {
                            package: dep_id.package().to_string(),
                        })?;
                    if task_definition.persistent
                        && package_json.scripts.contains_key(dep_id.task())
                    {
                        let (span, text) = self
                            .task_locations
                            .get(dep_id)
                            .map(|spanned| spanned.span_and_text("turbo.json"))
                            .unwrap_or((None, NamedSource::new("", "")));

                        return Err(ValidateError::DependencyOnPersistentTask {
                            span,
                            text,
                            persistent_task: dep_id.to_string(),
                            dependant: task_id.to_string(),
                        });
                    }
                }

                // check if the package for the task has that task in its package.json
                let info = package_graph
                    .package_info(&PackageName::from(task_id.package().to_string()))
                    .expect("package graph should contain workspace info for task package");

                let package_has_task = info
                    .package_json
                    .scripts
                    .get(task_id.task())
                    // handle legacy behaviour from go where an empty string may appear
                    .map_or(false, |script| !script.is_empty());

                let task_is_persistent = self
                    .task_definitions
                    .get(task_id)
                    .map_or(false, |task_def| task_def.persistent);

                Ok(task_is_persistent && package_has_task)
            })
            .fold((0, Vec::new()), |(mut count, mut errs), result| {
                match result {
                    Ok(true) => count += 1,
                    Ok(false) => (),
                    Err(e) => errs.push(e),
                }
                (count, errs)
            });

        // there must always be at least one concurrency 'slot' available for
        // non-persistent tasks otherwise we get race conditions
        if persistent_count >= concurrency {
            validation_errors.push(ValidateError::PersistentTasksExceedConcurrency {
                persistent_count,
                concurrency,
            })
        }

        validation_errors.extend(self.validate_interactive(ui_mode));

        match validation_errors.is_empty() {
            true => Ok(()),
            false => Err(validation_errors),
        }
    }

    // Validates that UI is setup if any interactive tasks will be executed
    fn validate_interactive(&self, ui_mode: UIMode) -> Vec<ValidateError> {
        // If experimental_ui is being used, then we don't need check for interactive
        // tasks
        if matches!(ui_mode, UIMode::Tui) {
            return Vec::new();
        }
        self.task_definitions
            .iter()
            .filter_map(|(task, definition)| {
                if definition.interactive {
                    Some(ValidateError::InteractiveNeedsUI {
                        task: task.to_string(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum ValidateError {
    #[error("Cannot find task definition for {task_id} in package {package_name}")]
    MissingTask {
        task_id: String,
        package_name: String,
    },
    #[error("Cannot find package {package}")]
    MissingPackageJson { package: String },
    #[error("\"{persistent_task}\" is a persistent task, \"{dependant}\" cannot depend on it")]
    DependencyOnPersistentTask {
        #[label("persistent task")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
        persistent_task: String,
        dependant: String,
    },
    #[error(
        "You have {persistent_count} persistent tasks but `turbo` is configured for concurrency \
         of {concurrency}. Set --concurrency to at least {}", persistent_count+1
    )]
    PersistentTasksExceedConcurrency {
        persistent_count: u32,
        concurrency: u32,
    },
    #[error(
        "Cannot run interactive task \"{task}\" without experimental UI. Set `\"experimentalUI\": \
         true` in `turbo.json` or `TURBO_EXPERIMENTAL_UI=true` as an environment variable"
    )]
    InteractiveNeedsUI { task: String },
}

impl fmt::Display for TaskNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskNode::Root => f.write_str("___ROOT___"),
            TaskNode::Task(task) => task.fmt(f),
        }
    }
}

#[cfg(test)]
mod test {

    use std::collections::BTreeMap;

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPath;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery, WorkspaceData},
        package_json::PackageJson,
    };

    use super::*;
    use crate::run::task_id::TaskName;

    struct DummyDiscovery<'a>(&'a TempDir);

    impl<'a> PackageDiscovery for DummyDiscovery<'a> {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            // our workspace has three packages, two of which have a build script
            let workspaces = [("a", true), ("b", true), ("c", false)]
                .into_iter()
                .map(|(name, had_build)| {
                    let path = AbsoluteSystemPath::from_std_path(self.0.path()).unwrap();
                    let package_json = path.join_component(&format!("{}.json", name));

                    let scripts = if had_build {
                        BTreeMap::from_iter(
                            [
                                ("build".to_string(), Spanned::new("echo built!".to_string())),
                                (
                                    "dev".to_string(),
                                    Spanned::new("echo running dev!".to_string()),
                                ),
                            ]
                            .into_iter(),
                        )
                    } else {
                        BTreeMap::default()
                    };

                    let package = PackageJson {
                        name: Some(name.to_string()),
                        scripts,
                        ..Default::default()
                    };

                    let file = std::fs::File::create(package_json.as_std_path()).unwrap();
                    serde_json::to_writer(file, &package).unwrap();

                    WorkspaceData {
                        package_json,
                        turbo_json: None,
                    }
                })
                .collect();

            Ok(DiscoveryResponse {
                package_manager: turborepo_repository::package_manager::PackageManager::Pnpm,
                workspaces,
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

    #[tokio::test]
    async fn issue_4291() {
        // we had an issue where our engine validation would reject running persistent
        // tasks if the number of _total packages_ exceeded the concurrency limit,
        // rather than the number of package that had that task. in this test, we
        // set up a workspace with three packages, two of which have a persistent build
        // task. we expect concurrency limit 1 to fail, but 2 and 3 to pass.

        let tmp = tempfile::TempDir::with_prefix("issue_4291").unwrap();

        let mut engine = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let task_id = TaskId::new(package, "build");
            engine.get_index(&task_id);
            engine.add_definition(
                task_id,
                TaskDefinition {
                    persistent: true,
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let graph_builder = PackageGraph::builder(
            AbsoluteSystemPath::from_std_path(tmp.path()).unwrap(),
            PackageJson::default(),
        )
        .with_package_discovery(DummyDiscovery(&tmp));

        let graph = graph_builder.build().await.unwrap();

        // if our limit is less than, it should fail
        engine
            .validate(&graph, 1, UIMode::Stream)
            .expect_err("not enough");

        // if our limit is less than, it should fail
        engine
            .validate(&graph, 2, UIMode::Stream)
            .expect_err("not enough");

        // we have two persistent tasks, and a slot for all other tasks, so this should
        // pass
        engine.validate(&graph, 3, UIMode::Stream).expect("ok");

        // if our limit is greater, then it should pass
        engine.validate(&graph, 4, UIMode::Stream).expect("ok");
    }

    #[tokio::test]
    async fn test_prune_persistent_tasks() {
        // Verifies that we can prune the `Engine` to include only the persistent tasks
        // or only the non-persistent tasks.

        let mut engine = Engine::new();

        // add two packages with a persistent build task
        for package in ["a", "b"] {
            let build_task_id = TaskId::new(package, "build");
            let dev_task_id = TaskId::new(package, "dev");

            engine.get_index(&build_task_id);
            engine.add_definition(build_task_id.clone(), TaskDefinition::default());

            engine.get_index(&dev_task_id);
            engine.add_definition(
                dev_task_id,
                TaskDefinition {
                    persistent: true,
                    task_dependencies: vec![Spanned::new(TaskName::from(build_task_id))],
                    ..Default::default()
                },
            );
        }

        let engine = engine.seal();

        let persistent_tasks_engine = engine.create_engine_for_persistent_tasks();
        for node in persistent_tasks_engine.tasks() {
            if let TaskNode::Task(task_id) = node {
                let def = persistent_tasks_engine
                    .task_definition(task_id)
                    .expect("task should have definition");
                assert!(def.persistent, "task should be persistent");
            }
        }

        let non_persistent_tasks_engine = engine.create_engine_without_persistent_tasks();
        for node in non_persistent_tasks_engine.tasks() {
            if let TaskNode::Task(task_id) = node {
                let def = non_persistent_tasks_engine
                    .task_definition(task_id)
                    .expect("task should have definition");
                assert!(!def.persistent, "task should not be persistent");
            }
        }
    }

    #[tokio::test]
    async fn test_get_subgraph_for_package() {
        // Verifies that we can prune the `Engine` to include only the persistent tasks
        // or only the non-persistent tasks.

        let mut engine = Engine::new();

        // Add two tasks in package `a`
        let a_build_task_id = TaskId::new("a", "build");
        let a_dev_task_id = TaskId::new("a", "dev");

        let a_build_idx = engine.get_index(&a_build_task_id);
        engine.add_definition(a_build_task_id.clone(), TaskDefinition::default());

        engine.get_index(&a_dev_task_id);
        engine.add_definition(a_dev_task_id.clone(), TaskDefinition::default());

        // Add two tasks in package `b` where the `build` task depends
        // on the `build` task from package `a`
        let b_build_task_id = TaskId::new("b", "build");
        let b_dev_task_id = TaskId::new("b", "dev");

        let b_build_idx = engine.get_index(&b_build_task_id);
        engine.add_definition(
            b_build_task_id.clone(),
            TaskDefinition {
                task_dependencies: vec![Spanned::new(TaskName::from(a_build_task_id.clone()))],
                ..Default::default()
            },
        );

        engine.get_index(&b_dev_task_id);
        engine.add_definition(b_dev_task_id.clone(), TaskDefinition::default());
        engine.task_graph.add_edge(b_build_idx, a_build_idx, ());

        let engine = engine.seal();
        let subgraph =
            engine.create_engine_for_subgraph(&[PackageName::from("a")].into_iter().collect());

        // Verify that the subgraph only contains tasks from package `a` and the `build`
        // task from package `b`
        let tasks: Vec<_> = subgraph.tasks().collect();
        assert_eq!(tasks.len(), 3);
        assert!(tasks.contains(&&TaskNode::Task(a_build_task_id)));
        assert!(tasks.contains(&&TaskNode::Task(a_dev_task_id)));
        assert!(tasks.contains(&&TaskNode::Task(b_build_task_id)));
    }
}
