use std::{collections::BTreeMap, rc::Rc};

use anyhow::Result;
use turbopath::AbsoluteSystemPath;

use super::workspace_catalog::WorkspaceCatalog;
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
pub struct TaskHashTracker {}
