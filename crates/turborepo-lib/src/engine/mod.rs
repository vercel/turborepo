mod builder;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub use builder::EngineBuilder;
use petgraph::Graph;

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
}
