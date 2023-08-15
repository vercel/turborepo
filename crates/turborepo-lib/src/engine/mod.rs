mod builder;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub use builder::EngineBuilder;
use petgraph::Graph;

use crate::{
    package_graph::{PackageGraph, WorkspaceName},
    run::task_id::TaskId,
    task_graph::TaskDefinition,
};

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
    task_definitions: HashMap<TaskId<'static>, Rc<TaskDefinition>>,
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
        definition: Rc<TaskDefinition>,
    ) -> Option<Rc<TaskDefinition>> {
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
    #[allow(dead_code)]
    pub fn dependencies(&self, task_id: &TaskId) -> Option<HashSet<&TaskNode>> {
        let index = self.task_lookup.get(task_id)?;
        Some(
            self.task_graph
                .neighbors_directed(*index, petgraph::Direction::Outgoing)
                .map(|index| {
                    self.task_graph
                        .node_weight(index)
                        .expect("node index should be present")
                })
                .collect(),
        )
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
                let is_persistent = self
                    .task_definitions
                    .get(task_id)
                    .map_or(false, |task_def| task_def.persistent);

                for dep_index in self
                    .task_graph
                    .neighbors_directed(node_index, petgraph::Direction::Outgoing)
                {
                    let TaskNode::Task(dep_id) = self
                        .task_graph
                        .node_weight(dep_index)
                        .expect("index comes from iterating the graph and must be present")
                    else {
                        panic!("{task_id} depends on root task node");
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

                Ok(is_persistent)
            })
            .fold((0, Vec::new()), |(mut count, mut errs), result| {
                match result {
                    Ok(true) => count += 1,
                    Ok(false) => (),
                    Err(e) => errs.push(e),
                }
                (count, errs)
            });

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

#[derive(Debug, thiserror::Error)]
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
        "You have {persistent_count} persistent tasks, but `turbo` is configured for concurrency \
         of {concurrency}. Set --concurrency to at least {persistent_count}"
    )]
    PersistentTasksExceedConcurrency {
        persistent_count: u32,
        concurrency: u32,
    },
}
