//! turborepo-engine: Task execution engine for Turborepo
//!
//! This crate provides the core engine for executing tasks in a Turborepo
//! monorepo. It handles task graph construction, dependency resolution, and
//! parallel execution.

mod dot;
mod execute;
mod mermaid;

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

pub use execute::{ExecuteError, ExecutionOptions, Message, StopExecution};
use petgraph::Graph;
use thiserror::Error;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::PackageName;
use turborepo_run_summary::EngineInfo;
use turborepo_task_id::TaskId;
use turborepo_types::TaskDefinition;

/// Trait for types that provide task definition information needed by the
/// engine.
///
/// This allows the engine to be decoupled from the full TaskDefinition type
/// while still having access to the fields it needs for execution decisions.
pub trait TaskDefinitionInfo {
    /// Returns true if this task is persistent (long-running)
    fn persistent(&self) -> bool;
    /// Returns true if this task can be interrupted and restarted
    fn interruptible(&self) -> bool;
    /// Returns true if this task requires interactive input
    fn interactive(&self) -> bool;
}

impl TaskDefinitionInfo for turborepo_types::TaskDefinition {
    fn persistent(&self) -> bool {
        self.persistent
    }
    fn interruptible(&self) -> bool {
        self.interruptible
    }
    fn interactive(&self) -> bool {
        self.interactive
    }
}

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

#[derive(Debug, Error)]
pub enum Error {
    #[error("Expected a task node, but got workspace root.")]
    Root,
}

impl TryFrom<TaskNode> for TaskId<'static> {
    type Error = Error;

    fn try_from(node: TaskNode) -> Result<Self, Self::Error> {
        match node {
            TaskNode::Root => Err(Error::Root),
            TaskNode::Task(id) => Ok(id),
        }
    }
}

#[derive(Debug, Default)]
pub struct Building;
#[derive(Debug, Default)]
pub struct Built;

#[derive(Debug)]
pub struct Engine<S = Built, T: TaskDefinitionInfo = TaskInfo> {
    marker: std::marker::PhantomData<S>,
    task_graph: Graph<TaskNode, ()>,
    root_index: petgraph::graph::NodeIndex,
    task_lookup: HashMap<TaskId<'static>, petgraph::graph::NodeIndex>,
    task_definitions: HashMap<TaskId<'static>, T>,
    task_locations: HashMap<TaskId<'static>, Spanned<()>>,
    package_tasks: HashMap<PackageName, Vec<petgraph::graph::NodeIndex>>,
    pub has_non_interruptible_tasks: bool,
}

/// Simple struct containing just the task definition fields needed by the
/// engine. This is the default task definition info type used when no custom
/// type is provided.
#[derive(Debug, Clone, Default)]
pub struct TaskInfo {
    pub persistent: bool,
    pub interruptible: bool,
    pub interactive: bool,
}

impl TaskDefinitionInfo for TaskInfo {
    fn persistent(&self) -> bool {
        self.persistent
    }
    fn interruptible(&self) -> bool {
        self.interruptible
    }
    fn interactive(&self) -> bool {
        self.interactive
    }
}

impl<T: TaskDefinitionInfo + Default + Clone> Engine<Building, T> {
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
            has_non_interruptible_tasks: false,
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

    pub fn add_definition(&mut self, task_id: TaskId<'static>, definition: T) -> Option<T> {
        if definition.persistent() && !definition.interruptible() {
            self.has_non_interruptible_tasks = true;
        }
        self.task_definitions.insert(task_id, definition)
    }

    pub fn add_task_location(&mut self, task_id: TaskId<'static>, location: Spanned<()>) {
        // If we don't have the location stored,
        // or if the location stored is empty, we add it to the map.
        let has_location = self
            .task_locations
            .get(&task_id)
            .is_some_and(|existing| existing.range.is_some());

        if !has_location {
            self.task_locations.insert(task_id, location);
        }
    }

    // Seals the task graph from being mutated
    pub fn seal(self) -> Engine<Built, T> {
        let Engine {
            task_graph,
            task_lookup,
            root_index,
            task_definitions,
            task_locations,
            package_tasks,
            has_non_interruptible_tasks,
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
            has_non_interruptible_tasks,
        }
    }

    /// Provides mutable access to the task graph for direct edge manipulation.
    /// Use with care - prefer using the builder methods when possible.
    pub fn task_graph_mut(&mut self) -> &mut Graph<TaskNode, ()> {
        &mut self.task_graph
    }
}

impl<T: TaskDefinitionInfo + Default + Clone> Default for Engine<Building, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: TaskDefinitionInfo + Clone> Engine<Built, T> {
    /// Creates an instance of `Engine` that only contains tasks that depend on
    /// tasks from a given package. This is useful for watch mode, where we
    /// need to re-run only a portion of the task graph.
    pub fn create_engine_for_subgraph(&self, changed_packages: &HashSet<PackageName>) -> Self {
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

                    if def.persistent() && !def.interruptible() {
                        return None;
                    }
                }
                // If the node is reachable from any of the entrypoint tasks, we include it
                entrypoint_indices
                    .iter()
                    .any(|idx| {
                        node_distances
                            .get(&(**idx, node_idx))
                            .is_some_and(|dist| *dist != i32::MAX)
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
            has_non_interruptible_tasks: false,
        }
    }

    /// Creates an `Engine` with only interruptible tasks, i.e. non-persistent
    /// tasks and persistent tasks that are allowed to be interrupted
    pub fn create_engine_for_interruptible_tasks(&self) -> Self {
        let new_graph = self.task_graph.filter_map(
            |node_idx, node| match &self.task_graph[node_idx] {
                TaskNode::Task(task) => {
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");

                    if !def.persistent() || def.interruptible() {
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
            has_non_interruptible_tasks: false,
        }
    }

    /// Creates an `Engine` that is only the tasks that are not interruptible,
    /// i.e. persistent and not allowed to be restarted
    pub fn create_engine_for_non_interruptible_tasks(&self) -> Self {
        let mut new_graph = self.task_graph.filter_map(
            |node_idx, node| match &self.task_graph[node_idx] {
                TaskNode::Task(task) => {
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");

                    if def.persistent() && !def.interruptible() {
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
            has_non_interruptible_tasks: true,
        }
    }

    pub fn dependencies(&self, task_id: &TaskId) -> Option<HashSet<&TaskNode>> {
        self.neighbors(task_id, petgraph::Direction::Outgoing)
    }

    pub fn dependents(&self, task_id: &TaskId) -> Option<HashSet<&TaskNode>> {
        self.neighbors(task_id, petgraph::Direction::Incoming)
    }

    pub fn transitive_dependents(&self, task_id: &TaskId<'static>) -> HashSet<&TaskNode> {
        turborepo_graph_utils::transitive_closure(
            &self.task_graph,
            self.task_lookup.get(task_id).cloned(),
            petgraph::Direction::Incoming,
        )
    }

    pub fn transitive_dependencies(&self, task_id: &TaskId<'static>) -> HashSet<&TaskNode> {
        turborepo_graph_utils::transitive_closure(
            &self.task_graph,
            self.task_lookup.get(task_id).cloned(),
            petgraph::Direction::Outgoing,
        )
    }

    /// Returns all tasks belonging to the given packages, plus all tasks that
    /// transitively depend on them. This performs a single batched graph
    /// traversal, which is more efficient than calling `transitive_dependents`
    /// for each task individually.
    pub fn tasks_impacted_by_packages(
        &self,
        packages: &HashSet<PackageName>,
    ) -> HashSet<&TaskNode> {
        // Collect all task indices belonging to the changed packages
        let starting_indices = packages
            .iter()
            .filter_map(|pkg| self.package_tasks.get(pkg))
            .flatten()
            .copied();

        // Single batched DFS traversal to find all transitive dependents
        turborepo_graph_utils::transitive_closure(
            &self.task_graph,
            starting_indices,
            petgraph::Direction::Incoming,
        )
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

    pub fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&T> {
        self.task_definitions.get(task_id)
    }

    pub fn tasks(&self) -> impl Iterator<Item = &TaskNode> {
        self.task_graph.node_weights()
    }

    pub fn task_ids(&self) -> impl Iterator<Item = &TaskId<'static>> {
        self.tasks().filter_map(|task| match task {
            TaskNode::Task(task_id) => Some(task_id),
            TaskNode::Root => None,
        })
    }

    pub fn task_definitions(&self) -> &HashMap<TaskId<'static>, T> {
        &self.task_definitions
    }

    pub fn task_locations(&self) -> &HashMap<TaskId<'static>, Spanned<()>> {
        &self.task_locations
    }

    /// Provides access to the underlying task graph
    pub fn task_graph(&self) -> &Graph<TaskNode, ()> {
        &self.task_graph
    }

    /// Provides access to the task lookup map (task_id -> node index)
    pub fn task_lookup(&self) -> &HashMap<TaskId<'static>, petgraph::graph::NodeIndex> {
        &self.task_lookup
    }
}

// Implement EngineInfo for Engine<Built, TaskDefinition> to allow use with
// turborepo-run-summary. This implementation provides access to task
// definitions and dependency information needed for run summaries.
impl EngineInfo for Engine<Built, TaskDefinition> {
    type TaskIter<'a> = std::iter::FilterMap<
        std::collections::hash_set::IntoIter<&'a TaskNode>,
        fn(&'a TaskNode) -> Option<&'a TaskId<'static>>,
    >;

    fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition> {
        Engine::task_definition(self, task_id)
    }

    fn dependencies(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
        Engine::dependencies(self, task_id).map(|deps| {
            deps.into_iter().filter_map(
                (|node| match node {
                    TaskNode::Task(id) => Some(id),
                    TaskNode::Root => None,
                }) as fn(&TaskNode) -> Option<&TaskId<'static>>,
            )
        })
    }

    fn dependents(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> {
        Engine::dependents(self, task_id).map(|deps| {
            deps.into_iter().filter_map(
                (|node| match node {
                    TaskNode::Task(id) => Some(id),
                    TaskNode::Root => None,
                }) as fn(&TaskNode) -> Option<&TaskId<'static>>,
            )
        })
    }
}

impl fmt::Display for TaskNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskNode::Root => f.write_str("___ROOT___"),
            TaskNode::Task(task) => task.fmt(f),
        }
    }
}
