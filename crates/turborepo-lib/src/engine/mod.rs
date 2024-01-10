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
use miette::Diagnostic;
use petgraph::Graph;
use thiserror::Error;
use turborepo_repository::package_graph::{PackageGraph, WorkspaceName};

use crate::{run::task_id::TaskId, task_graph::TaskDefinition};

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
        }
    }

    pub fn get_index(&mut self, task_id: &TaskId<'static>) -> petgraph::graph::NodeIndex {
        self.task_lookup.get(task_id).copied().unwrap_or_else(|| {
            let index = self.task_graph.add_node(TaskNode::Task(task_id.clone()));
            self.task_lookup.insert(task_id.clone(), index);
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
        self.task_definitions.insert(task_id, definition)
    }

    // Seals the task graph from being mutated
    pub fn seal(self) -> Engine<Built> {
        let Engine {
            task_graph,
            task_lookup,
            root_index,
            task_definitions,
            ..
        } = self;
        Engine {
            marker: std::marker::PhantomData,
            task_graph,
            task_lookup,
            root_index,
            task_definitions,
        }
    }
}

impl Default for Engine<Building> {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine<Built> {
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

    pub fn task_definitions(&self) -> &HashMap<TaskId<'static>, TaskDefinition> {
        &self.task_definitions
    }

    pub fn validate(
        &self,
        package_graph: &PackageGraph,
        concurrency: u32,
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
                        .package_json(&WorkspaceName::from(dep_id.package()))
                        .ok_or_else(|| ValidateError::MissingPackageJson {
                            package: dep_id.package().to_string(),
                        })?;
                    if task_definition.persistent
                        && package_json.scripts.contains_key(dep_id.task())
                    {
                        return Err(ValidateError::DependencyOnPersistentTask {
                            persistent_task: dep_id.to_string(),
                            dependant: task_id.to_string(),
                        });
                    }
                }

                // check if the package for the task has that task in its package.json
                let info = package_graph
                    .workspace_info(&WorkspaceName::from(task_id.package().to_string()))
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

        match validation_errors.is_empty() {
            true => Ok(()),
            false => Err(validation_errors),
        }
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

    use tempdir::TempDir;
    use turbopath::AbsoluteSystemPath;
    use turborepo_repository::{
        discovery::{DiscoveryResponse, PackageDiscovery, WorkspaceData},
        package_json::PackageJson,
    };

    use super::*;

    struct DummyDiscovery<'a>(&'a TempDir);

    impl<'a> PackageDiscovery for DummyDiscovery<'a> {
        async fn discover_packages(
            &mut self,
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
                            [("build".to_string(), "echo built!".to_string())].into_iter(),
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
    }

    #[tokio::test]
    async fn issue_4291() {
        // we had an issue where our engine validation would reject running persistent
        // tasks if the number of _total packages_ exceeded the concurrency limit,
        // rather than the number of package that had that task. in this test, we
        // set up a workspace with three packages, two of which have a persistent build
        // task. we expect concurrency limit 1 to fail, but 2 and 3 to pass.

        let tmp = tempdir::TempDir::new("issue_4291").unwrap();

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
        engine.validate(&graph, 1).expect_err("not enough");

        // if our limit is less than, it should fail
        engine.validate(&graph, 2).expect_err("not enough");

        // we have two persistent tasks, and a slot for all other tasks, so this should
        // pass
        engine.validate(&graph, 3).expect("ok");

        // if our limit is greater, then it should pass
        engine.validate(&graph, 4).expect("ok");
    }
}
