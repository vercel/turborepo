use std::collections::BTreeMap;

use anyhow::Result;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    config::TurboJson,
    run::pipeline::{Pipeline, TaskDefinition},
};

pub struct CompleteGraph {
    // TODO: This should actually be an acyclic graph type
    // Expresses the dependencies between packages
    workspace_graph: petgraph::Graph<String, String>,
    // Config from turbo.json
    pipeline: Pipeline,
    // Stores the package.json contents by package name
    workspace_infos: WorkspaceCatalog,
    // Hash of all global dependencies
    global_hash: Option<String>,

    task_definitions: BTreeMap<String, TaskDefinition>,
    repo_root: AbsoluteSystemPathBuf,

    task_hash_tracker: TaskHashTracker,
}

impl CompleteGraph {
    pub fn new(
        workspace_graph: &petgraph::Graph<String, String>,
        workspace_infos: &WorkspaceCatalog,
        repo_root: AbsoluteSystemPathBuf,
    ) -> Self {
        Self {
            workspace_graph,
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
