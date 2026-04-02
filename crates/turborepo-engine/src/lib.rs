//! turborepo-engine: Task execution engine for Turborepo
//!
//! This crate provides the core engine for executing tasks in a Turborepo
//! monorepo. It handles task graph construction, dependency resolution, and
//! parallel execution.

// Allow large error types - boxing would be a significant refactor and these
// errors are already established patterns in the codebase
#![allow(clippy::result_large_err)]

pub mod affected;
mod builder;
mod builder_error;
mod builder_errors;
mod dot;
mod execute;
mod graph_visualizer;
mod loader;
mod mermaid;
mod task_definition;
mod validate;

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

pub use affected::match_tasks_against_changed_files;
pub use builder::{EngineBuilder, TaskInheritanceResolver, ValidationMode};
pub use builder_error::Error as BuilderError;
pub use builder_errors::{
    CyclicExtends, InvalidTaskNameError, MissingPackageFromTaskError, MissingPackageTaskError,
    MissingRootTaskInTurboJsonError, MissingTaskError, MissingTurboJsonExtends,
};
pub use execute::{ExecuteError, ExecutionOptions, Message, StopExecution};
pub use graph_visualizer::{
    ChildProcess, ChildSpawner, Error as GraphVisualizerError, GraphvizWarningFn, NoOpChild,
    NoOpSpawner, write_graph,
};
pub use loader::TurboJsonLoader;
use petgraph::{
    Graph,
    visit::{DfsEvent, Reversed, depth_first_search},
};
pub use task_definition::TaskDefinitionFromProcessed;
use thiserror::Error;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::PackageName;
use turborepo_task_id::TaskId;
use turborepo_types::{EngineInfo, TaskDefinition};
pub use validate::{TaskDefinitionResult, validate_task_name};

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
    /// Creates an engine containing only tasks reachable from the given
    /// packages: their direct tasks, transitive dependents, and all
    /// transitive dependencies needed for execution. Persistent
    /// non-interruptible tasks are excluded (they can't be restarted in
    /// watch mode). Used by watch mode to scope rebuilds to the changed
    /// portion of the task graph.
    ///
    /// Transitive dependencies are included because the executor needs them
    /// to produce outputs that downstream tasks consume — on a cold cache
    /// or first run, those outputs won't already be on disk.
    pub fn create_engine_for_subgraph(self, changed_packages: &HashSet<PackageName>) -> Self {
        let entrypoint_indices: Vec<_> = changed_packages
            .iter()
            .filter_map(|pkg| self.package_tasks.get(pkg))
            .flatten()
            .copied()
            .collect();

        let reachable = self.reachable_closure(entrypoint_indices);
        self.prune_to_reachable(&reachable, true)
    }

    /// Returns a new engine containing only the given directly affected tasks,
    /// their transitive dependents, and all transitive dependencies required
    /// for execution. Used for task-level `--affected` detection: the caller
    /// determines which tasks' `inputs` match the changed files, then this
    /// method expands that set to include downstream tasks and their full
    /// dependency chains, pruning everything else.
    ///
    /// Dependencies of affected tasks are included because the executor
    /// needs them in the graph to restore cached outputs before running
    /// the affected tasks that consume them.
    ///
    /// This consumes and returns a new engine rather than mutating in place,
    /// following the sealed typestate contract. Must be called during build,
    /// before the engine is shared via `Arc` or handed to the executor.
    pub fn retain_affected_tasks(self, affected_tasks: &HashSet<TaskId>) -> Self {
        let entrypoint_indices: Vec<_> = affected_tasks
            .iter()
            .filter_map(|task_id| self.task_lookup.get(task_id))
            .copied()
            .collect();

        let original_task_count = self.task_graph.node_count().saturating_sub(1);
        let reachable = self.reachable_closure(entrypoint_indices);
        let retained_task_count = reachable.len().saturating_sub(1);

        tracing::info!(
            directly_affected = affected_tasks.len(),
            retained = retained_task_count,
            pruned = original_task_count.saturating_sub(retained_task_count),
            "task graph pruned for --affected"
        );

        self.prune_to_reachable(&reachable, false)
    }

    /// Prunes the engine to only the given tasks and their transitive
    /// dependencies (upstream tasks needed for execution).
    ///
    /// Unlike `retain_affected_tasks`, this does NOT expand to dependents.
    /// Use this for `--filter` where the user explicitly scoped the task set
    /// and dependent expansion has already been handled by selector resolution.
    pub fn retain_filtered_tasks(self, filtered_tasks: &HashSet<TaskId>) -> Self {
        let entrypoint_indices: Vec<_> = filtered_tasks
            .iter()
            .filter_map(|task_id| self.task_lookup.get(task_id))
            .copied()
            .collect();

        let original_task_count = self.task_graph.node_count().saturating_sub(1);

        // Forward DFS only: find the filtered tasks + transitive dependencies.
        let mut reachable = HashSet::new();
        reachable.insert(self.root_index);
        depth_first_search(&self.task_graph, entrypoint_indices, |event| {
            if let DfsEvent::Discover(n, _) = event {
                reachable.insert(n);
            }
        });

        let retained_task_count = reachable.len().saturating_sub(1);
        tracing::info!(
            directly_filtered = filtered_tasks.len(),
            retained = retained_task_count,
            pruned = original_task_count.saturating_sub(retained_task_count),
            "task graph pruned for --filter"
        );

        self.prune_to_reachable(&reachable, false)
    }

    /// Computes the full reachable set from seed nodes: reverse DFS for
    /// transitive dependents, then forward DFS for transitive dependencies.
    /// Root is always included so `prune_to_reachable` can recover it.
    fn reachable_closure(
        &self,
        entrypoint_indices: Vec<petgraph::graph::NodeIndex>,
    ) -> HashSet<petgraph::graph::NodeIndex> {
        // Reverse DFS: find transitive dependents (downstream consumers).
        let mut reachable = HashSet::new();
        reachable.insert(self.root_index);
        depth_first_search(Reversed(&self.task_graph), entrypoint_indices, |event| {
            if let DfsEvent::Discover(n, _) = event {
                reachable.insert(n);
            }
        });

        // Forward DFS: find transitive dependencies (upstream tasks needed as
        // cache hits). Root is excluded as a seed since it has no outgoing
        // edges in the forward direction.
        let forward_seeds: Vec<_> = reachable
            .iter()
            .copied()
            .filter(|&n| n != self.root_index)
            .collect();
        depth_first_search(&self.task_graph, forward_seeds, |event| {
            if let DfsEvent::Discover(n, _) = event {
                reachable.insert(n);
            }
        });

        reachable
    }

    /// Prunes the engine graph to only nodes in `reachable` and rebuilds all
    /// metadata (`task_lookup`, `root_index`, `task_definitions`,
    /// `task_locations`, `package_tasks`, `has_non_interruptible_tasks`).
    ///
    /// When `exclude_non_interruptible_persistent` is true, persistent
    /// non-interruptible tasks are also filtered out even if reachable (used
    /// by watch mode).
    fn prune_to_reachable(
        mut self,
        reachable: &HashSet<petgraph::graph::NodeIndex>,
        exclude_non_interruptible_persistent: bool,
    ) -> Self {
        self.task_graph = self.task_graph.filter_map(
            |node_idx, node| {
                if !reachable.contains(&node_idx) {
                    return None;
                }
                if exclude_non_interruptible_persistent && let TaskNode::Task(task) = node {
                    let def = self
                        .task_definitions
                        .get(task)
                        .expect("task should have definition");
                    if def.persistent() && !def.interruptible() {
                        return None;
                    }
                }
                Some(node.clone())
            },
            |_, _| Some(()),
        );

        // Rebuild all metadata from the pruned graph. root_index is recovered
        // during the task_lookup rebuild to avoid a separate linear scan.
        let mut new_root_index = None;
        self.task_lookup = self
            .task_graph
            .node_indices()
            .filter_map(|index| {
                match self
                    .task_graph
                    .node_weight(index)
                    .expect("node index should be present")
                {
                    TaskNode::Root => {
                        new_root_index = Some(index);
                        None
                    }
                    TaskNode::Task(task) => Some((task.clone(), index)),
                }
            })
            .collect();
        self.root_index = new_root_index.expect("root node should be present");

        self.task_definitions
            .retain(|id, _| self.task_lookup.contains_key(id));
        self.task_locations
            .retain(|id, _| self.task_lookup.contains_key(id));

        self.package_tasks =
            self.task_lookup
                .iter()
                .fold(HashMap::new(), |mut acc, (task_id, &idx)| {
                    acc.entry(PackageName::from(task_id.package()))
                        .or_default()
                        .push(idx);
                    acc
                });

        self.has_non_interruptible_tasks = self
            .task_definitions
            .values()
            .any(|def| def.persistent() && !def.interruptible());

        self
    }

    pub fn dependencies(&self, task_id: &TaskId) -> Option<Vec<&TaskNode>> {
        self.neighbors(task_id, petgraph::Direction::Outgoing)
    }

    pub fn dependents(&self, task_id: &TaskId) -> Option<Vec<&TaskNode>> {
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

    /// Returns all task IDs belonging to the given packages.
    pub fn task_ids_for_packages(
        &self,
        packages: &HashSet<PackageName>,
    ) -> HashSet<TaskId<'static>> {
        packages
            .iter()
            .filter_map(|pkg| self.package_tasks.get(pkg))
            .flatten()
            .filter_map(|&idx| match self.task_graph.node_weight(idx)? {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect()
    }

    /// Returns the transitive task-graph dependencies of the given task set.
    /// Forward DFS: all tasks that must run before any task in `task_ids`.
    pub fn collect_task_dependencies(
        &self,
        task_ids: &HashSet<TaskId<'static>>,
    ) -> HashSet<TaskId<'static>> {
        let indices = task_ids
            .iter()
            .filter_map(|id| self.task_lookup.get(id))
            .copied();
        let nodes = turborepo_graph_utils::transitive_closure(
            &self.task_graph,
            indices,
            petgraph::Direction::Outgoing,
        );
        nodes
            .into_iter()
            .filter_map(|node| match node {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect()
    }

    /// Returns the transitive task-graph dependents of the given task set.
    /// Reverse DFS: all tasks that depend on any task in `task_ids`.
    pub fn collect_task_dependents(
        &self,
        task_ids: &HashSet<TaskId<'static>>,
    ) -> HashSet<TaskId<'static>> {
        let indices = task_ids
            .iter()
            .filter_map(|id| self.task_lookup.get(id))
            .copied();
        let nodes = turborepo_graph_utils::transitive_closure(
            &self.task_graph,
            indices,
            petgraph::Direction::Incoming,
        );
        nodes
            .into_iter()
            .filter_map(|node| match node {
                TaskNode::Task(id) => Some(id.clone()),
                TaskNode::Root => None,
            })
            .collect()
    }

    fn neighbors(
        &self,
        task_id: &TaskId,
        direction: petgraph::Direction,
    ) -> Option<Vec<&TaskNode>> {
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
        std::vec::IntoIter<&'a TaskNode>,
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

// Warning that comes from the execution of the task
#[derive(Debug, Clone)]
pub struct TaskWarning {
    task_id: String,
    missing_platform_env: Vec<String>,
}

// Error that comes from the execution of the task
#[derive(Debug, Error, Clone)]
#[error("{task_id}: {cause}")]
pub struct TaskError {
    task_id: String,
    cause: TaskErrorCause,
}

#[derive(Debug, Error, Clone)]
pub enum TaskErrorCause {
    #[error("unable to spawn child process: {msg}")]
    // We eagerly serialize this in order to allow us to implement clone
    Spawn { msg: String },
    #[error("command {command} exited ({exit_code})")]
    Exit { command: String, exit_code: i32 },
    #[error("turbo has internal error processing task")]
    Internal,
}

impl TaskWarning {
    /// Construct a new warning for a given task with the
    /// Returns `None` if there are no missing platform environment variables
    pub fn new(task_id: &str, missing_platform_env: Vec<String>) -> Option<Self> {
        if missing_platform_env.is_empty() {
            return None;
        }
        Some(Self {
            task_id: task_id.to_owned(),
            missing_platform_env,
        })
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// All missing platform environment variables.
    /// Guaranteed to have at least length 1 due to constructor validation.
    pub fn missing_platform_env(&self) -> &[String] {
        &self.missing_platform_env
    }
}

impl TaskError {
    pub fn new(task_id: String, cause: TaskErrorCause) -> Self {
        Self { task_id, cause }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self.cause {
            TaskErrorCause::Exit { exit_code, .. } => Some(exit_code),
            _ => None,
        }
    }

    pub fn from_spawn(task_id: String, err: std::io::Error) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Spawn {
                msg: err.to_string(),
            },
        }
    }

    pub fn from_execution(task_id: String, command: String, exit_code: i32) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Exit { command, exit_code },
        }
    }
}

impl TaskErrorCause {
    pub fn from_spawn(err: std::io::Error) -> Self {
        TaskErrorCause::Spawn {
            msg: err.to_string(),
        }
    }

    pub fn from_execution(command: String, exit_code: i32) -> Self {
        TaskErrorCause::Exit { command, exit_code }
    }
}

#[cfg(test)]
mod task_error_tests {
    use super::*;

    #[test]
    fn test_warning_no_vars() {
        let no_warning = TaskWarning::new("a-task", vec![]);
        assert!(no_warning.is_none());
    }

    #[test]
    fn test_warning_some_var() {
        let warning = TaskWarning::new("a-task", vec!["MY_VAR".into()]);
        assert!(warning.is_some());
        let warning = warning.unwrap();
        assert_eq!(warning.task_id(), "a-task");
        assert_eq!(warning.missing_platform_env(), &["MY_VAR".to_owned()]);
    }
}

#[cfg(test)]
mod affected_tasks_tests {
    use std::collections::HashSet;

    use super::*;

    /// Builds a linear chain: a#build ← b#build ← c#build
    /// (b depends on a, c depends on b)
    fn build_linear_engine() -> Engine {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "build");
        let c = TaskId::new("c", "build");

        let a_idx = engine.get_index(&a);
        let b_idx = engine.get_index(&b);
        let c_idx = engine.get_index(&c);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(b.clone(), TaskInfo::default());
        engine.add_definition(c.clone(), TaskInfo::default());

        // b depends on a, c depends on b
        engine.task_graph_mut().add_edge(b_idx, a_idx, ());
        engine.task_graph_mut().add_edge(c_idx, b_idx, ());

        engine.connect_to_root(&a);

        engine.seal()
    }

    fn task_ids_set(engine: &Engine) -> HashSet<TaskId<'static>> {
        engine.task_ids().cloned().collect()
    }

    #[test]
    fn empty_affected_returns_root_only() {
        let engine = build_linear_engine();
        let engine = engine.retain_affected_tasks(&HashSet::new());

        assert!(
            task_ids_set(&engine).is_empty(),
            "empty affected set should produce an engine with no tasks"
        );
        assert!(
            engine.tasks().any(|n| *n == TaskNode::Root),
            "root node must always be present"
        );
    }

    #[test]
    fn affected_leaf_includes_all_dependents() {
        let engine = build_linear_engine();
        let affected: HashSet<_> = [TaskId::new("a", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        let ids = task_ids_set(&engine);
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(ids.contains(&TaskId::new("c", "build")));
    }

    #[test]
    fn affected_terminal_includes_dependency_chain() {
        let engine = build_linear_engine();
        let affected: HashSet<_> = [TaskId::new("c", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        // c is affected, but c depends on b which depends on a.
        // All three must survive so the executor can run the full
        // dependency chain (a and b will be cache hits).
        let ids = task_ids_set(&engine);
        assert_eq!(
            ids.len(),
            3,
            "full dependency chain should survive: {ids:?}"
        );
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(ids.contains(&TaskId::new("c", "build")));
    }

    #[test]
    fn affected_middle_includes_deps_and_dependents() {
        let engine = build_linear_engine();
        let affected: HashSet<_> = [TaskId::new("b", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        // b is affected. c depends on b (dependent, needs re-run).
        // b depends on a (dependency, needed for execution as a cache hit).
        let ids = task_ids_set(&engine);
        assert_eq!(ids.len(), 3, "deps + dependents should survive: {ids:?}");
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(ids.contains(&TaskId::new("c", "build")));
    }

    #[test]
    fn metadata_pruned_to_surviving_tasks() {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "build");
        let x = TaskId::new("x", "build");

        let a_idx = engine.get_index(&a);
        let b_idx = engine.get_index(&b);
        engine.get_index(&x);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(b.clone(), TaskInfo::default());
        engine.add_definition(x.clone(), TaskInfo::default());

        engine.task_graph_mut().add_edge(b_idx, a_idx, ());
        engine.connect_to_root(&a);
        engine.connect_to_root(&x);

        let engine = engine.seal();
        let affected: HashSet<_> = [TaskId::new("a", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        // a and b survive (a affected, b depends on a). x is unrelated and pruned.
        assert!(engine.task_definition(&TaskId::new("a", "build")).is_some());
        assert!(engine.task_definition(&TaskId::new("b", "build")).is_some());
        assert!(
            engine.task_definition(&TaskId::new("x", "build")).is_none(),
            "unrelated task metadata should be pruned"
        );
    }

    #[test]
    fn unrelated_task_excluded() {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "build");
        let x = TaskId::new("x", "build");

        let a_idx = engine.get_index(&a);
        let b_idx = engine.get_index(&b);
        engine.get_index(&x);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(b.clone(), TaskInfo::default());
        engine.add_definition(x.clone(), TaskInfo::default());

        engine.task_graph_mut().add_edge(b_idx, a_idx, ());
        engine.connect_to_root(&a);
        engine.connect_to_root(&x);

        let engine = engine.seal();
        let affected: HashSet<_> = [TaskId::new("a", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        let ids = task_ids_set(&engine);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(!ids.contains(&TaskId::new("x", "build")));
    }

    #[test]
    fn nonexistent_task_id_ignored() {
        let engine = build_linear_engine();
        let affected: HashSet<_> = [TaskId::new("nonexistent", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        assert!(
            task_ids_set(&engine).is_empty(),
            "nonexistent task should produce empty engine"
        );
    }

    #[test]
    fn has_non_interruptible_tracks_pruned_state() {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "dev");

        engine.get_index(&a);
        engine.get_index(&b);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(
            b.clone(),
            TaskInfo {
                persistent: true,
                interruptible: false,
                ..Default::default()
            },
        );

        engine.connect_to_root(&a);
        engine.connect_to_root(&b);

        let engine = engine.seal();
        assert!(engine.has_non_interruptible_tasks);

        let affected: HashSet<_> = [TaskId::new("a", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);
        assert!(
            !engine.has_non_interruptible_tasks,
            "filtered engine should not have non-interruptible tasks"
        );
    }

    /// Diamond graph: a → b, a → c, b → d, c → d
    /// Tests that multi-path reachability is handled correctly.
    #[test]
    fn diamond_graph_affected_at_root() {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "build");
        let c = TaskId::new("c", "build");
        let d = TaskId::new("d", "build");

        let a_idx = engine.get_index(&a);
        let b_idx = engine.get_index(&b);
        let c_idx = engine.get_index(&c);
        let d_idx = engine.get_index(&d);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(b.clone(), TaskInfo::default());
        engine.add_definition(c.clone(), TaskInfo::default());
        engine.add_definition(d.clone(), TaskInfo::default());

        // b depends on a, c depends on a, d depends on b and c
        engine.task_graph_mut().add_edge(b_idx, a_idx, ());
        engine.task_graph_mut().add_edge(c_idx, a_idx, ());
        engine.task_graph_mut().add_edge(d_idx, b_idx, ());
        engine.task_graph_mut().add_edge(d_idx, c_idx, ());
        engine.connect_to_root(&a);

        let engine = engine.seal();
        let affected: HashSet<_> = [TaskId::new("a", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        let ids = task_ids_set(&engine);
        assert_eq!(ids.len(), 4, "all diamond tasks should survive: {ids:?}");
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(ids.contains(&TaskId::new("c", "build")));
        assert!(ids.contains(&TaskId::new("d", "build")));
    }

    #[test]
    fn diamond_graph_affected_at_branch() {
        let mut engine: Engine<Building> = Engine::new();

        let a = TaskId::new("a", "build");
        let b = TaskId::new("b", "build");
        let c = TaskId::new("c", "build");
        let d = TaskId::new("d", "build");

        let a_idx = engine.get_index(&a);
        let b_idx = engine.get_index(&b);
        let c_idx = engine.get_index(&c);
        let d_idx = engine.get_index(&d);

        engine.add_definition(a.clone(), TaskInfo::default());
        engine.add_definition(b.clone(), TaskInfo::default());
        engine.add_definition(c.clone(), TaskInfo::default());
        engine.add_definition(d.clone(), TaskInfo::default());

        engine.task_graph_mut().add_edge(b_idx, a_idx, ());
        engine.task_graph_mut().add_edge(c_idx, a_idx, ());
        engine.task_graph_mut().add_edge(d_idx, b_idx, ());
        engine.task_graph_mut().add_edge(d_idx, c_idx, ());
        engine.connect_to_root(&a);

        let engine = engine.seal();
        // b is affected. d depends on b (dependent). b depends on a (dependency).
        // d also depends on c which depends on a. All four must survive so d's
        // full dependency diamond can execute.
        let affected: HashSet<_> = [TaskId::new("b", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        let ids = task_ids_set(&engine);
        assert_eq!(ids.len(), 4, "full diamond should survive: {ids:?}");
        assert!(ids.contains(&TaskId::new("a", "build")));
        assert!(ids.contains(&TaskId::new("b", "build")));
        assert!(ids.contains(&TaskId::new("c", "build")));
        assert!(ids.contains(&TaskId::new("d", "build")));
    }

    #[test]
    fn retain_is_idempotent() {
        let engine = build_linear_engine();
        let affected: HashSet<_> = [TaskId::new("b", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);
        let ids_first = task_ids_set(&engine);

        // Second call with same affected set should be idempotent.
        let engine = engine.retain_affected_tasks(&affected);
        let ids_second = task_ids_set(&engine);

        assert_eq!(ids_first, ids_second, "retain should be idempotent");
    }

    /// Regression test for https://github.com/vercel/turborepo/issues/12512
    ///
    /// When an app's source changes and it has a ^build dependency on a lib,
    /// the lib's build task must remain in the graph even though the lib's
    /// inputs didn't change. Without it, the executor can't restore the
    /// lib's cached output before the app tries to consume it.
    #[test]
    fn affected_app_retains_lib_dependency() {
        let mut engine: Engine<Building> = Engine::new();

        let lib_build = TaskId::new("common", "build");
        let app_build = TaskId::new("web", "build");

        let lib_idx = engine.get_index(&lib_build);
        let app_idx = engine.get_index(&app_build);

        engine.add_definition(lib_build.clone(), TaskInfo::default());
        engine.add_definition(app_build.clone(), TaskInfo::default());

        // web#build depends on common#build (^build)
        engine.task_graph_mut().add_edge(app_idx, lib_idx, ());
        engine.connect_to_root(&lib_build);

        let engine = engine.seal();

        // Only web's source changed — common is not directly affected.
        let affected: HashSet<_> = [TaskId::new("web", "build")].into_iter().collect();
        let engine = engine.retain_affected_tasks(&affected);

        let ids = task_ids_set(&engine);
        assert!(
            ids.contains(&TaskId::new("web", "build")),
            "affected app task should survive"
        );
        assert!(
            ids.contains(&TaskId::new("common", "build")),
            "^build dependency must survive for the app to execute"
        );
    }
}

use std::sync::{Arc, Mutex};

/// A wrapper around `Arc<Mutex<Vec<TaskError>>>` that implements
/// `TaskErrorCollector`.
#[derive(Clone)]
pub struct TaskErrorCollectorWrapper(pub Arc<Mutex<Vec<TaskError>>>);

impl TaskErrorCollectorWrapper {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    pub fn from_arc(arc: Arc<Mutex<Vec<TaskError>>>) -> Self {
        Self(arc)
    }

    pub fn into_inner(self) -> Arc<Mutex<Vec<TaskError>>> {
        self.0
    }
}

impl Default for TaskErrorCollectorWrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl turborepo_task_executor::TaskErrorCollector for TaskErrorCollectorWrapper {
    fn push_spawn_error(&self, task_id: String, error: std::io::Error) {
        self.0
            .lock()
            .expect("lock poisoned")
            .push(TaskError::from_spawn(task_id, error));
    }

    fn push_execution_error(&self, task_id: String, command: String, exit_code: i32) {
        self.0
            .lock()
            .expect("lock poisoned")
            .push(TaskError::from_execution(task_id, command, exit_code));
    }
}

/// A wrapper around `Arc<Mutex<Vec<TaskWarning>>>` that implements
/// `TaskWarningCollector`.
#[derive(Clone)]
pub struct TaskWarningCollectorWrapper(pub Arc<Mutex<Vec<TaskWarning>>>);

impl TaskWarningCollectorWrapper {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }

    pub fn from_arc(arc: Arc<Mutex<Vec<TaskWarning>>>) -> Self {
        Self(arc)
    }

    pub fn into_inner(self) -> Arc<Mutex<Vec<TaskWarning>>> {
        self.0
    }
}

impl Default for TaskWarningCollectorWrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl turborepo_task_executor::TaskWarningCollector for TaskWarningCollectorWrapper {
    fn push_platform_env_warning(&self, task_id: &str, missing_vars: Vec<String>) {
        if let Some(warning) = TaskWarning::new(task_id, missing_vars) {
            self.0.lock().expect("lock poisoned").push(warning);
        }
    }
}
