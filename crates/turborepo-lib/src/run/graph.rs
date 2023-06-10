use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

use anyhow::Result;
use daggy::{stable_dag::StableDag, NodeIndex, Walker};
use petgraph::{stable_graph::DefaultIx, visit::Topo};
use thiserror::Error;
use tokio::sync::{
    mpsc::Receiver as MpscReceiver,
    watch::{channel as watch_channel, Receiver as WatchReceiver, Sender as WatchSender},
};
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

#[derive(Error, Debug)]
pub enum Error {
    #[error("Cannot find task {0}")]
    MissingTask(TaskId),
    #[error("Internal error: bad node index {0:?}")]
    BadIndex(NodeIndex),
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

#[derive(Debug, PartialEq, Eq)]
enum Upstream {
    Pending,
    Done,
}

trait Visitor: Clone + Send {
    fn visit(&self, task_id: TaskId);
}

type TraverseItem = (TaskId, WatchSender<Upstream>);

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
        let (_, idx) = self.graph.add_parent(
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
                .add_parent(*parent_idx, Dependency::Task, Node::Task(child.clone()));
        self.tasks.insert(child, child_idx);
        Ok(())
    }

    pub fn execute(&self) -> Result<MpscReceiver<TraverseItem>, Error> {
        let (send_ready, ready_ch) = tokio::sync::mpsc::channel::<TraverseItem>(1);
        let mut walker = Topo::new(self.graph.graph());
        let mut receivers: HashMap<NodeIndex, WatchReceiver<Upstream>> = HashMap::new();
        while let Some(idx) = walker.next(self.graph.graph()) {
            match self.graph.node_weight(idx) {
                None => return Err(Error::BadIndex(idx)),
                Some(Node::Root) => {}
                Some(Node::Task(task_id)) => {
                    let task_id = task_id.clone();
                    let dep_channels: Vec<WatchReceiver<Upstream>> = self
                        .graph
                        .parents(idx)
                        .iter(&self.graph)
                        .filter_map(|(_, parent_idx)| {
                            if parent_idx == self.root_index {
                                None
                            } else {
                                Some(parent_idx)
                            }
                        })
                        .map(|idx| match receivers.get(&idx) {
                            Some(dep_ch) => Ok(dep_ch.clone()),
                            None => Err(Error::BadIndex(idx)),
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    let (task_done, downstream) = watch_channel::<Upstream>(Upstream::Pending);
                    let ready = send_ready.clone();
                    tokio::spawn(async move {
                        // We're going to drop this at the end of the closure
                        let task_done = task_done;
                        for mut dep in dep_channels {
                            // TODO: handle upstream errors here
                            match dep.wait_for(|upstream| *upstream == Upstream::Done).await {
                                Ok(_) => continue,
                                Err(e) => panic!("hmm {}", e),
                            }
                        }
                        ready.send((task_id, task_done)).await
                    });
                    receivers.insert(idx, downstream);
                }
            }
        }
        Ok(ready_ch)
    }
}

#[cfg(test)]
mod tests {
    use super::TaskGraph;
    use crate::run::{graph::Upstream, task_id::TaskId};

    #[tokio::test]
    async fn test_traverse() {
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
        )
        .unwrap();
        let mut tasks = g.execute().unwrap();
        while let Some((task, tx)) = tasks.recv().await {
            tokio::task::spawn_blocking(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                println!("got {}", task);
                if !tx.is_closed() {
                    tx.send(Upstream::Done).unwrap();
                }
            });
        }
    }
}
