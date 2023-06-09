use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

use anyhow::Result;
use daggy::{stable_dag::StableDag, NodeIndex};
use turbopath::AbsoluteSystemPath;

use super::task_id::TaskId;
use crate::{
    config::TurboJson,
    run::pipeline::{Pipeline, TaskDefinition},
};

pub struct CompleteGraph<'run> {
    // TODO: This should actually be an acyclic graph type
    // Expresses the dependencies between packages
    workspace_graph: Rc<petgraph::Graph<String, String>>,
    // Config from turbo.json
    pipeline: Pipeline,
    // Stores the package.json contents by package name
    workspace_infos: Rc<WorkspaceCatalog>,
    // Hash of all global dependencies
    global_hash: Option<String>,

    task_definitions: BTreeMap<String, TaskDefinition>,
    repo_root: &'run AbsoluteSystemPath,

    task_hash_tracker: TaskHashTracker,
}

impl<'run> CompleteGraph<'run> {
    pub fn new(
        workspace_graph: Rc<petgraph::Graph<String, String>>,
        workspace_infos: Rc<WorkspaceCatalog>,
        repo_root: &'run AbsoluteSystemPath,
    ) -> Self {
        Self {
            workspace_graph,
            pipeline: Pipeline::default(),
            workspace_infos,
            repo_root,
            global_hash: None,
            task_definitions: BTreeMap::new(),
            task_hash_tracker: TaskHashTracker::default(),
        }
    }

    pub fn get_turbo_config_from_workspace(
        &self,
        _workspace_name: &str,
        _is_single_package: bool,
    ) -> Result<TurboJson> {
        // TODO
        Ok(TurboJson::default())
    }
}

#[derive(Default)]
pub struct WorkspaceCatalog {}

#[derive(Default)]
pub struct TaskHashTracker {}

pub enum Error {
    MissingTask(TaskId),
}

pub enum Dependency {
    Internal,
    Task,
    Topological,
}

pub enum Node {
    Root,
    Task(TaskId),
}

struct TaskGraph {
    graph: StableDag<Node, Dependency>,
    root_index: NodeIndex,
    tasks: HashMap<TaskId, NodeIndex>,
}

impl TaskGraph {
    pub fn new() -> Self {
        let mut graph = StableDag::new();
        let root_index = graph.add_node(Node::Root);
        Self {
            graph,
            root_index,
            tasks: HashMap::new(),
        }
    }

    pub fn add_root_task(&mut self, task_id: TaskId) {
        let (_, idx) = self.graph.add_child(
            self.root_index,
            Dependency::Internal,
            Node::Task(task_id.clone()),
        );
        self.tasks.insert(task_id, idx);
    }

    pub fn add_task_dep(&mut self, parent: &TaskId, child: TaskId) -> Result<(), Error> {
        let parent_idx = self
            .tasks
            .get(parent)
            .ok_or_else(|| Error::MissingTask(parent.clone()))?;
        let (_, child_idx) =
            self.graph
                .add_child(*parent_idx, Dependency::Task, Node::Task(child.clone()));
        self.tasks.insert(child, child_idx);
        Ok(())
    }

    pub fn execute<Visitor>(&self, visitor: Visitor) -> Vec<Error>
    where
        Visitor: Fn(&TaskId) -> Result<(), Error>,
    {
        let mut walker = petgraph::visit::Topo::new(self.graph.graph());
        while let Some(idx) = walker.next(self.graph.graph()) {
            match self.graph.node_weight(idx) {
                None => println!("no node here"),
                Some(Node::Root) => println!("root node"),
                Some(Node::Task(task_id)) => {
                    visitor(task_id);
                }
            }
        }
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::TaskGraph;
    use crate::run::task_id::TaskId;

    #[test]
    fn test_iterate() {
        let mut g = TaskGraph::new();
        g.add_root_task(TaskId::Root("test".to_string()));
        g.add_root_task(TaskId::Root("lint".to_string()));
        g.add_root_task(TaskId::Root("build".to_string()));
        g.add_task_dep(
            &TaskId::Root("test".to_string()),
            TaskId::Package {
                package: "some-pkg".to_string(),
                task: "generate".to_string(),
            },
        );
        let errors = g.execute(|task_id| {
            println!("{:?}", task_id);
            Ok(())
        });
        assert!(errors.is_empty());
    }
}
