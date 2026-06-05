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
pub use execute::{ExecuteError, ExecutionOptions, Message};
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
use turborepo_types::{EngineInfo, ShardSpec, ShardSummary, ShardingSummary, TaskDefinition};
pub use validate::{TaskDefinitionResult, validate_task_name};

/// Trait for types that provide task definition information needed by the
/// engine.
///
/// This allows the engine to be decoupled from the full TaskDefinition type
/// while still having access to the fields it needs for execution decisions.
pub trait TaskDefinitionInfo {
    /// Returns true if this task can restore outputs from cache.
    fn cache(&self) -> bool {
        true
    }
    /// Returns true if this task is persistent (long-running)
    fn persistent(&self) -> bool;
    /// Returns true if this task can be interrupted and restarted
    fn interruptible(&self) -> bool;
    /// Returns true if this task requires interactive input
    fn interactive(&self) -> bool;
}

impl TaskDefinitionInfo for turborepo_types::TaskDefinition {
    fn cache(&self) -> bool {
        self.cache
    }

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

/// The result of dividing the task graph into shards.
///
/// Carries both the serializable [`ShardingSummary`] (surfaced in run summaries
/// / `--dry=json`) and the per-shard entry task sets needed to prune the engine
/// down to a single selected shard.
#[derive(Debug, Clone)]
pub struct ShardingPlan {
    /// Human/machine-readable summary of the sharding decision.
    pub summary: ShardingSummary,
    /// For each shard (indexed 0-based; shard `n` in the summary is
    /// `shard_entry_tasks[n - 1]`), the set of entry (top-level requested)
    /// tasks assigned to it. Pruning the engine to a shard means retaining
    /// these tasks and their transitive dependencies.
    pub shard_entry_tasks: Vec<HashSet<TaskId<'static>>>,
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
    /// packages: their direct tasks, transitive dependents, and cacheable
    /// transitive dependencies needed for execution. Persistent
    /// non-interruptible tasks are excluded (they can't be restarted in
    /// watch mode). Used by watch mode to scope rebuilds to the changed
    /// portion of the task graph.
    ///
    /// Cacheable transitive dependencies are included because the executor
    /// needs them to restore outputs that downstream tasks consume.
    /// Non-cacheable dependencies are excluded because retaining them would
    /// force execution even when they did not change.
    pub fn create_engine_for_subgraph(self, changed_packages: &HashSet<PackageName>) -> Self {
        let entrypoint_indices: Vec<_> = changed_packages
            .iter()
            .filter_map(|pkg| self.package_tasks.get(pkg))
            .flatten()
            .copied()
            .collect();

        let reachable = self.watch_reachable_closure(entrypoint_indices);
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

    /// Computes the watch-mode reachable set from changed package task nodes:
    /// reverse DFS for transitive dependents, then forward traversal for only
    /// cacheable transitive dependencies.
    fn watch_reachable_closure(
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

        let mut stack: Vec<_> = reachable
            .iter()
            .copied()
            .filter(|&n| n != self.root_index)
            .collect();

        while let Some(node) = stack.pop() {
            for dependency in self
                .task_graph
                .neighbors_directed(node, petgraph::Direction::Outgoing)
            {
                if dependency == self.root_index || !self.is_cacheable_task_node(dependency) {
                    continue;
                }

                if reachable.insert(dependency) {
                    stack.push(dependency);
                }
            }
        }

        reachable
    }

    fn is_cacheable_task_node(&self, node: petgraph::graph::NodeIndex) -> bool {
        let Some(TaskNode::Task(task)) = self.task_graph.node_weight(node) else {
            return false;
        };

        self.task_definitions
            .get(task)
            .is_some_and(|def| def.cache())
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
        let pruned_graph = self.task_graph.filter_map(
            |node_idx, node| {
                if !reachable.contains(&node_idx) {
                    return None;
                }
                if exclude_non_interruptible_persistent
                    && let TaskNode::Task(task) = node
                    && self
                        .task_definitions
                        .get(task)
                        .is_some_and(|def| def.persistent() && !def.interruptible())
                {
                    return None;
                }
                Some(node.clone())
            },
            |_, _| Some(()),
        );

        // Rebuild all metadata from the pruned graph. root_index is recovered
        // during the task_lookup rebuild to avoid a separate linear scan.
        let mut new_root_index = None;
        let task_lookup = pruned_graph
            .node_indices()
            .filter_map(|index| match pruned_graph.node_weight(index)? {
                TaskNode::Root => {
                    new_root_index = Some(index);
                    None
                }
                TaskNode::Task(task) => Some((task.clone(), index)),
            })
            .collect();
        let Some(root_index) = new_root_index else {
            tracing::debug!("skipping task graph prune because the root node was not retained");
            return self;
        };

        self.task_graph = pruned_graph;
        self.task_lookup = task_lookup;
        self.root_index = root_index;

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
                .filter_map(|index| self.task_graph.node_weight(index))
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

    /// The "entry" tasks: the top-level tasks that nothing else depends on,
    /// i.e. the sinks of the dependency DAG (the final outputs the user asked
    /// for). Edges point from a dependent to its dependency, so an entry task
    /// is a task node with no incoming edges. These are the units the sharder
    /// distributes across shards; every other task is pulled into a shard as a
    /// transitive dependency of one of these.
    ///
    /// (Note: the synthetic root is connected *from* the leaf tasks that have
    /// no dependencies, so it is not an entry task and is skipped here.)
    fn entrypoint_indices(&self) -> Vec<petgraph::graph::NodeIndex> {
        self.task_graph
            .node_indices()
            .filter(|&idx| {
                idx != self.root_index
                    && matches!(self.task_graph.node_weight(idx), Some(TaskNode::Task(_)))
                    && self
                        .task_graph
                        .neighbors_directed(idx, petgraph::Direction::Incoming)
                        .next()
                        .is_none()
            })
            .collect()
    }

    /// Forward (dependency) closure of a single task node: the node itself plus
    /// every task it transitively depends on. Root is excluded. This is exactly
    /// the set of tasks that must be present for the seed task to run.
    fn dependency_closure(
        &self,
        seed: petgraph::graph::NodeIndex,
    ) -> HashSet<petgraph::graph::NodeIndex> {
        let mut set = HashSet::new();
        depth_first_search(&self.task_graph, Some(seed), |event| {
            if let DfsEvent::Discover(n, _) = event
                && n != self.root_index
            {
                set.insert(n);
            }
        });
        set
    }

    fn node_to_task_id(&self, idx: petgraph::graph::NodeIndex) -> Option<TaskId<'static>> {
        match self.task_graph.node_weight(idx)? {
            TaskNode::Task(id) => Some(id.clone()),
            TaskNode::Root => None,
        }
    }

    fn sorted_task_ids(
        &self,
        indices: impl IntoIterator<Item = petgraph::graph::NodeIndex>,
    ) -> Vec<String> {
        let mut ids: Vec<String> = indices
            .into_iter()
            .filter_map(|idx| self.node_to_task_id(idx).map(|id| id.to_string()))
            .collect();
        ids.sort();
        ids
    }

    /// Divides the task graph into shards according to `spec`.
    ///
    /// The unit of distribution is the set of *entry* tasks (task nodes with no
    /// dependents). Each entry's transitive dependencies are pulled into its
    /// shard so every shard is independently runnable. Because the graph is a
    /// dependency graph, entries share large parts of their dependency closures
    /// (e.g. when `test` depends on `^test`, sibling packages pull in the same
    /// lower layers), so a naive split duplicates those shared layers into
    /// every shard.
    ///
    /// To reduce that duplication, assignment is *overlap-aware*: entries whose
    /// closures overlap are co-located so a shared dependency is duplicated
    /// across as few shards as possible. Concretely we minimize, greedily, the
    /// number of *new* nodes each entry adds to the shard it is placed in.
    ///
    /// - `MaxShards(k)`: at most `k` shards (capped by the entry count). Shards
    ///   are seeded with `k` entries chosen farthest-first (each seed maximizes
    ///   the new nodes it adds versus already-chosen seeds) so the shards start
    ///   in different regions of the graph and none stays empty. Remaining
    ///   entries, heaviest first, go to the shard they overlap most with,
    ///   capped at `ceil(entries / k)` entries per shard for balance.
    /// - `MaxNodesPerShard(p)`: as many shards as needed so each holds at most
    ///   `p` *distinct* task nodes. Entries (heaviest first) go to the existing
    ///   shard they overlap most with that still fits under `p`; otherwise a
    ///   new shard is opened. A single entry whose own closure exceeds `p` gets
    ///   its own (over-sized) shard.
    ///
    /// Overlap is measured on the true, de-duplicated shard node sets, so the
    /// reported `task_count`/`total_task_instances` reflect actual duplication.
    pub fn compute_sharding(&self, spec: ShardSpec) -> ShardingPlan {
        // Entries, sorted deterministically by task id so sharding is stable
        // across runs.
        let mut entries: Vec<(TaskId<'static>, petgraph::graph::NodeIndex)> = self
            .entrypoint_indices()
            .into_iter()
            .filter_map(|idx| self.node_to_task_id(idx).map(|id| (id, idx)))
            .collect();
        entries.sort_by_key(|(id, _)| id.to_string());

        let closures: Vec<HashSet<petgraph::graph::NodeIndex>> = entries
            .iter()
            .map(|(_, idx)| self.dependency_closure(*idx))
            .collect();
        let num_entries = entries.len();

        // Deterministic processing order: heaviest closures first, ties by id.
        // Placing big entries first lets smaller, overlapping entries slot into
        // the shard that already contains their dependencies.
        let mut order: Vec<usize> = (0..num_entries).collect();
        order.sort_by(|&a, &b| {
            closures[b]
                .len()
                .cmp(&closures[a].len())
                .then_with(|| entries[a].0.to_string().cmp(&entries[b].0.to_string()))
        });

        // Number of nodes in `closures[entry]` not already in `shard`.
        let added_nodes = |shard: &HashSet<petgraph::graph::NodeIndex>, entry: usize| -> usize {
            closures[entry]
                .iter()
                .filter(|n| !shard.contains(n))
                .count()
        };

        let mut shard_nodes: Vec<HashSet<petgraph::graph::NodeIndex>> = Vec::new();
        let mut shard_entries: Vec<Vec<usize>> = Vec::new();

        let limit = match spec {
            ShardSpec::MaxShards(k) => {
                let k = k.max(1);
                let num_shards = if num_entries == 0 {
                    0
                } else {
                    k.min(num_entries)
                };

                if num_shards > 0 {
                    // Farthest-first seeding: first seed is the largest closure,
                    // each subsequent seed maximizes new coverage versus the
                    // union of chosen seeds. Guarantees `num_shards` non-empty
                    // shards spread across the graph.
                    let mut chosen = vec![false; num_entries];
                    let mut seed_union: HashSet<petgraph::graph::NodeIndex> = HashSet::new();
                    let mut seeds: Vec<usize> = Vec::with_capacity(num_shards);
                    while seeds.len() < num_shards {
                        // Pick the not-yet-chosen entry (iterating in `order` for
                        // determinism) that adds the most new nodes.
                        let mut best: Option<usize> = None;
                        let mut best_added = 0usize;
                        for &entry in &order {
                            if chosen[entry] {
                                continue;
                            }
                            let added = added_nodes(&seed_union, entry);
                            if best.is_none() || added > best_added {
                                best = Some(entry);
                                best_added = added;
                            }
                        }
                        let seed = best.unwrap_or(order[seeds.len()]);
                        chosen[seed] = true;
                        seed_union.extend(closures[seed].iter().copied());
                        seeds.push(seed);
                    }

                    shard_nodes = seeds.iter().map(|&e| closures[e].clone()).collect();
                    shard_entries = seeds.iter().map(|&e| vec![e]).collect();

                    // Balance: cap entries per shard.
                    let cap = num_entries.div_ceil(num_shards);
                    for &entry in &order {
                        if chosen[entry] {
                            continue;
                        }
                        // Place in the under-cap shard the entry overlaps most
                        // with (fewest new nodes); ties to the smaller shard.
                        let mut best: Option<usize> = None;
                        let mut best_key = (usize::MAX, usize::MAX);
                        for s in 0..shard_nodes.len() {
                            if shard_entries[s].len() >= cap {
                                continue;
                            }
                            let key = (added_nodes(&shard_nodes[s], entry), shard_nodes[s].len());
                            if key < best_key {
                                best_key = key;
                                best = Some(s);
                            }
                        }
                        let s = best.unwrap_or(0);
                        shard_nodes[s].extend(closures[entry].iter().copied());
                        shard_entries[s].push(entry);
                    }
                }
                k
            }
            ShardSpec::MaxNodesPerShard(p) => {
                let p = p.max(1);
                for &entry in &order {
                    // Best-overlap existing shard that still fits under `p`.
                    let mut best: Option<usize> = None;
                    let mut best_added = usize::MAX;
                    for (s, shard) in shard_nodes.iter().enumerate() {
                        let added = added_nodes(shard, entry);
                        if shard.len() + added > p {
                            continue;
                        }
                        if added < best_added {
                            best_added = added;
                            best = Some(s);
                        }
                    }
                    match best {
                        Some(s) => {
                            shard_nodes[s].extend(closures[entry].iter().copied());
                            shard_entries[s].push(entry);
                        }
                        None => {
                            // Doesn't fit anywhere (or graph is empty of shards):
                            // open a new shard, even if this single entry's
                            // closure already exceeds `p`.
                            shard_nodes.push(closures[entry].clone());
                            shard_entries.push(vec![entry]);
                        }
                    }
                }
                p
            }
        };

        let num_shards = shard_nodes.len();
        let shard_entry_indices: Vec<Vec<petgraph::graph::NodeIndex>> = shard_entries
            .iter()
            .map(|es| es.iter().map(|&e| entries[e].1).collect())
            .collect();
        let shard_entry_tasks: Vec<HashSet<TaskId<'static>>> = shard_entries
            .iter()
            .map(|es| es.iter().map(|&e| entries[e].0.clone()).collect())
            .collect();

        // Count how many shards each node appears in to find shared deps.
        let mut shard_count: HashMap<petgraph::graph::NodeIndex, usize> = HashMap::new();
        for nodes in &shard_nodes {
            for &node in nodes {
                *shard_count.entry(node).or_default() += 1;
            }
        }
        let is_shared = |idx: petgraph::graph::NodeIndex| -> bool {
            shard_count.get(&idx).is_some_and(|&c| c > 1)
        };

        let shared_indices: Vec<petgraph::graph::NodeIndex> = shard_count
            .iter()
            .filter_map(|(&n, &c)| (c > 1).then_some(n))
            .collect();
        let shared_tasks = self.sorted_task_ids(shared_indices);

        let shards: Vec<ShardSummary> = (0..num_shards)
            .map(|s| {
                let tasks = self.sorted_task_ids(shard_nodes[s].iter().copied());
                let shared =
                    self.sorted_task_ids(shard_nodes[s].iter().copied().filter(|&n| is_shared(n)));
                ShardSummary {
                    index: s + 1,
                    entry_tasks: self.sorted_task_ids(shard_entry_indices[s].iter().copied()),
                    task_count: tasks.len(),
                    tasks,
                    shared_tasks: shared,
                }
            })
            .collect();

        // total_tasks: distinct tasks across all shards (the work that must run
        // once). total_task_instances: sum of shard sizes (with duplication).
        let total_tasks = shard_count.len();
        let total_task_instances: usize = shard_nodes.iter().map(|nodes| nodes.len()).sum();

        ShardingPlan {
            summary: ShardingSummary {
                strategy: spec.strategy(),
                limit,
                total_shards: num_shards,
                total_tasks,
                total_task_instances,
                selected_shard: None,
                shards,
                shared_tasks,
            },
            shard_entry_tasks,
        }
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(TaskError::from_spawn(task_id, error));
    }

    fn push_execution_error(&self, task_id: String, command: String, exit_code: i32) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(warning);
        }
    }
}

#[cfg(test)]
mod sharding_tests {
    use turborepo_types::ShardSpec;

    use super::*;

    /// Builds a fan-in graph with four independent `test` sinks, each
    /// depending on its own `#build` plus one `shared#build` dependency:
    ///
    ///   {a,b,c,d}#test ── depend on ──> {a,b,c,d}#build
    ///                  └─ depend on ──> shared#build
    ///
    /// Entry tasks (no dependents) are the four `#test` tasks; each has a
    /// dependency closure of size 3 (`X#test`, `X#build`, `shared#build`).
    fn build_fan_in_engine() -> Engine {
        let mut engine: Engine<Building> = Engine::new();
        let shared = TaskId::new("shared", "build");
        let shared_idx = engine.get_index(&shared);
        engine.add_definition(shared.clone(), TaskInfo::default());
        engine.connect_to_root(&shared);

        for pkg in ["a", "b", "c", "d"] {
            let test = TaskId::new(pkg, "test");
            let build = TaskId::new(pkg, "build");
            let test_idx = engine.get_index(&test);
            let build_idx = engine.get_index(&build);
            engine.add_definition(test.clone(), TaskInfo::default());
            engine.add_definition(build.clone(), TaskInfo::default());
            // test depends on its own build and the shared build.
            engine.task_graph_mut().add_edge(test_idx, build_idx, ());
            engine.task_graph_mut().add_edge(test_idx, shared_idx, ());
            // build has no deps, so it anchors to root.
            engine.connect_to_root(&build);
        }

        engine.seal()
    }

    #[test]
    fn entrypoints_are_the_sinks() {
        let engine = build_fan_in_engine();
        let mut entries: Vec<String> = engine
            .entrypoint_indices()
            .into_iter()
            .filter_map(|idx| engine.node_to_task_id(idx).map(|id| id.to_string()))
            .collect();
        entries.sort();
        assert_eq!(
            entries,
            vec!["a#test", "b#test", "c#test", "d#test"],
            "only the test tasks (no dependents) are entries"
        );
    }

    #[test]
    fn max_shards_balances_and_reports_shared() {
        let engine = build_fan_in_engine();
        let plan = engine.compute_sharding(ShardSpec::MaxShards(2));
        let summary = &plan.summary;

        assert_eq!(summary.strategy, "maxShards");
        assert_eq!(summary.limit, 2);
        assert_eq!(summary.total_shards, 2);
        assert_eq!(summary.selected_shard, None);
        // 4 entries split evenly across 2 shards.
        assert_eq!(summary.shards.len(), 2);
        for shard in &summary.shards {
            assert_eq!(shard.entry_tasks.len(), 2);
            // {X#test, X#build, Y#test, Y#build, shared#build} == 5
            assert_eq!(shard.task_count, 5);
            assert!(shard.tasks.contains(&"shared#build".to_string()));
            assert!(shard.shared_tasks.contains(&"shared#build".to_string()));
        }
        // shared#build is the only task in more than one shard.
        assert_eq!(summary.shared_tasks, vec!["shared#build".to_string()]);

        // Every entry is assigned to exactly one shard.
        let total_entries: usize = plan.shard_entry_tasks.iter().map(|s| s.len()).sum();
        assert_eq!(total_entries, 4);
    }

    #[test]
    fn max_shards_is_capped_by_entry_count() {
        let engine = build_fan_in_engine();
        // Asking for more shards than entries yields one shard per entry.
        let plan = engine.compute_sharding(ShardSpec::MaxShards(100));
        assert_eq!(plan.summary.total_shards, 4);
        for shard in &plan.summary.shards {
            assert_eq!(shard.entry_tasks.len(), 1);
        }
    }

    #[test]
    fn max_nodes_per_shard_packs_entries() {
        let engine = build_fan_in_engine();
        // Each entry's closure weight is 3, so a limit of 6 packs two entries
        // per shard (2 shards total).
        let plan = engine.compute_sharding(ShardSpec::MaxNodesPerShard(6));
        assert_eq!(plan.summary.strategy, "maxNodesPerShard");
        assert_eq!(plan.summary.limit, 6);
        assert_eq!(plan.summary.total_shards, 2);
        for shard in &plan.summary.shards {
            assert_eq!(shard.entry_tasks.len(), 2);
        }
    }

    #[test]
    fn max_nodes_per_shard_isolates_oversized_entries() {
        let engine = build_fan_in_engine();
        // A limit smaller than a single entry's closure (3) forces each entry
        // into its own shard.
        let plan = engine.compute_sharding(ShardSpec::MaxNodesPerShard(1));
        assert_eq!(plan.summary.total_shards, 4);
        for shard in &plan.summary.shards {
            assert_eq!(shard.entry_tasks.len(), 1);
        }
    }

    /// Two groups of entries; entries within a group share a deep subtree but
    /// the groups share nothing:
    ///
    ///   a1#test, a2#test ──> libA#typecheck ──> libA#build
    ///   b1#test, b2#test ──> libB#typecheck ──> libB#build
    fn build_two_cluster_engine() -> Engine {
        let mut engine: Engine<Building> = Engine::new();
        for (lib, entries) in [("libA", ["a1", "a2"]), ("libB", ["b1", "b2"])] {
            let lib_build = TaskId::new(lib, "build").into_owned();
            let lib_check = TaskId::new(lib, "typecheck").into_owned();
            let lib_build_idx = engine.get_index(&lib_build);
            let lib_check_idx = engine.get_index(&lib_check);
            engine.add_definition(lib_build.clone(), TaskInfo::default());
            engine.add_definition(lib_check.clone(), TaskInfo::default());
            engine
                .task_graph_mut()
                .add_edge(lib_check_idx, lib_build_idx, ());
            engine.connect_to_root(&lib_build);

            for pkg in entries {
                let test = TaskId::new(pkg, "test").into_owned();
                let test_idx = engine.get_index(&test);
                engine.add_definition(test.clone(), TaskInfo::default());
                engine
                    .task_graph_mut()
                    .add_edge(test_idx, lib_check_idx, ());
            }
        }
        engine.seal()
    }

    #[test]
    fn overlap_aware_clustering_avoids_duplication() {
        let engine = build_two_cluster_engine();
        let plan = engine.compute_sharding(ShardSpec::MaxShards(2));
        let summary = &plan.summary;

        assert_eq!(summary.total_shards, 2);
        // 6 distinct tasks: a1,a2,b1,b2 tests + libA{build,typecheck} +
        // libB{build,typecheck} = 4 + 2 + 2 = 8.
        assert_eq!(summary.total_tasks, 8);
        // Clustering the two groups onto separate shards duplicates nothing, so
        // instances == distinct tasks. A naive size-balanced split (e.g.
        // a1+b1 / a2+b2) would duplicate both lib subtrees, giving 12.
        assert_eq!(
            summary.total_task_instances, 8,
            "overlap-aware clustering should add no duplicated work"
        );
        assert!(
            summary.shared_tasks.is_empty(),
            "no task should span both shards: {:?}",
            summary.shared_tasks
        );

        // Each group's two entries must land together.
        let shard_of = |task: &str| -> usize {
            summary
                .shards
                .iter()
                .position(|s| s.entry_tasks.iter().any(|t| t == task))
                .expect("entry assigned to a shard")
        };
        assert_eq!(shard_of("a1#test"), shard_of("a2#test"));
        assert_eq!(shard_of("b1#test"), shard_of("b2#test"));
        assert_ne!(shard_of("a1#test"), shard_of("b1#test"));
    }
}
